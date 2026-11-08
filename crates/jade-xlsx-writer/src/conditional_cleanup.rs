//! Finish newly emptied owned extension features without dropping opaque data.
use crate::{
    patch::{apply, Splice},
    xml_edit_ns, Result,
};
use jade_xlsx_reader::worksheet_extensions::{self, Kind};
use quick_xml::{events::Event, NsReader};
use std::ops::Range;
fn lists(xml: &str) -> Result<Vec<(Range<usize>, Kind)>> {
    let sources = worksheet_extensions::sources(xml)?;
    let mut reader = NsReader::from_str(xml);
    let mut spans = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event()? {
            event @ (Event::Start(_) | Event::Empty(_)) => {
                let kind = [Kind::ConditionalFormats, Kind::Sparklines]
                    .into_iter()
                    .find(|kind| sources.owns_payload_at(start, *kind));
                if let Some(kind) = kind {
                    if let Event::Start(e) = event {
                        reader.read_to_end(e.name())?;
                    }
                    spans.push((start..reader.buffer_position() as usize, kind));
                }
            }
            Event::Eof => return Ok(spans),
            _ => {}
        }
    }
}

fn changed_children(
    before: &str,
    after: &str,
    name: &[u8],
    mut finish: impl FnMut(&xml_edit_ns::ContextXml, &xml_edit_ns::ContextXml) -> Result<String>,
) -> Result<String> {
    let old = xml_edit_ns::children(before, name)?;
    let new = xml_edit_ns::children(after, name)?;
    if old.len() != new.len() {
        return Ok(after.into());
    }
    let mut edits = Vec::new();
    for ((a, b, original), (start, end, moved)) in old.iter().zip(&new) {
        if before[*a..*b] != after[*start..*end] {
            let text = finish(original, moved)?;
            if text != after[*start..*end] {
                edits.push(Splice {
                    start: *start,
                    end: *end,
                    text,
                });
            }
        }
    }
    Ok(apply(after, edits))
}

fn sparkline_groups(before: &str, after: &str) -> Result<String> {
    changed_children(before, after, b"sparklineGroup", |old, new| {
        let moved = changed_children(&old.text, &new.text, b"sparklines", |_, list| {
            crate::empty_feature::finish(list, &list.text, b"sparkline")
        })?;
        crate::empty_feature::finish(new, &moved, b"sparklines")
    })
}

pub(crate) fn after_structure(before: &str, after: &str) -> Result<String> {
    if !before.contains("conditionalFormattings") && !before.contains("sparklineGroups") {
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
    for ((old, original_kind), (new, kind)) in original.iter().zip(&moved) {
        if original_kind != kind {
            return Ok(after.into());
        }
        if before[old.clone()] != after[new.clone()] {
            let context = xml_edit_ns::fragment(after, new.clone())?;
            let text = if *kind == Kind::Sparklines {
                let original = xml_edit_ns::fragment(before, old.clone())?;
                let moved = sparkline_groups(&original.text, &context.text)?;
                crate::empty_feature::finish(&context, &moved, b"sparklineGroup")?
            } else {
                crate::empty_feature::finish(&context, &context.text, b"conditionalFormatting")?
            };
            if text != after[new.clone()] {
                edits.push(Splice {
                    start: new.start,
                    end: new.end,
                    text,
                });
            }
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
                        known = matches!(
                            Kind::from_uri(&attr.unescape_value()?),
                            Some(Kind::ConditionalFormats | Kind::Sparklines)
                        );
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
