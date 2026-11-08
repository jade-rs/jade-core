//! Remove newly emptied owned x14 conditional lists without dropping opaque data.
use crate::{
    patch::{apply, Splice},
    xml_edit_ns, Result,
};
use jade_xlsx_reader::worksheet_extensions::{self, Kind};
use quick_xml::{events::Event, NsReader};
use std::ops::Range;
fn lists(xml: &str) -> Result<Vec<Range<usize>>> {
    let sources = worksheet_extensions::sources(xml)?;
    let mut reader = NsReader::from_str(xml);
    let mut spans = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event()? {
            Event::Start(e) if sources.owns_payload_at(start, Kind::ConditionalFormats) => {
                reader.read_to_end(e.name())?;
                spans.push(start..reader.buffer_position() as usize);
            }
            Event::Empty(_) if sources.owns_payload_at(start, Kind::ConditionalFormats) => {
                spans.push(start..reader.buffer_position() as usize);
            }
            Event::Eof => return Ok(spans),
            _ => {}
        }
    }
}
pub(crate) fn after_structure(before: &str, after: &str) -> Result<String> {
    if !before.contains("conditionalFormattings") {
        return Ok(after.into());
    }
    let original = lists(before)?;
    let moved = lists(after)?;
    // Geometry edits preserve these list roots. Do not guess correspondence if
    // a future operation changes that invariant.
    if original.len() != moved.len() {
        return Ok(after.into());
    }
    let mut edits = Vec::new();
    for (old, new) in original.iter().zip(&moved) {
        if !xml_edit_ns::plain_empty(&before[old.clone()], &[])?
            && xml_edit_ns::plain_empty(&after[new.clone()], &[])?
        {
            edits.push(Splice {
                start: new.start,
                end: new.end,
                text: String::new(),
            });
        }
    }
    if edits.is_empty() {
        return Ok(after.into());
    }
    let xml = apply(after, edits);
    let mut edits = Vec::new();
    for (start, end, list) in xml_edit_ns::children(&xml, b"extLst")? {
        let mut removed = Vec::new();
        for (a, b, extension) in xml_edit_ns::children(&list.text, b"ext")? {
            let mut reader = quick_xml::Reader::from_str(&extension.text);
            let mut known = false;
            if let Event::Start(e) | Event::Empty(e) = reader.read_event()? {
                for attr in e.attributes() {
                    let attr = attr?;
                    if attr.key.as_ref() == b"uri" {
                        known = Kind::from_uri(&attr.unescape_value()?)
                            == Some(Kind::ConditionalFormats);
                    }
                }
            }
            if known && xml_edit_ns::plain_empty(&extension.text, &[b"uri"])? {
                removed.push(Splice {
                    start: a,
                    end: b,
                    text: String::new(),
                });
            }
        }
        if removed.is_empty() {
            continue;
        }
        let updated = apply(&list.text, removed);
        let text = if xml_edit_ns::plain_empty(&updated, &[])? {
            String::new()
        } else {
            list.finish(&updated)?
        };
        edits.push(Splice { start, end, text });
    }
    Ok(apply(&xml, edits))
}
