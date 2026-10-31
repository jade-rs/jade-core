//! Manual page breaks: a sheet's `rowBreaks` (or `colBreaks`), each `brk`
//! the zero-based row the break sits above, as Excel writes them.

use std::collections::BTreeSet;

use quick_xml::events::Event;
use quick_xml::name::QName;
use quick_xml::Reader;

use jade_xlsx_reader::model::{MAX_COLUMN_COUNT, MAX_ROW_COUNT};
use jade_xlsx_reader::Axis;

use crate::patch::{apply, attribute, local, namespace_prefix, Splice};
use crate::Result;

/// Worksheet children that come after `rowBreaks`, in schema order.
const AFTER_ROW_BREAKS: &[&[u8]] = &[
    b"colBreaks",
    b"customProperties",
    b"cellWatches",
    b"ignoredErrors",
    b"smartTags",
    b"drawing",
    b"legacyDrawing",
    b"legacyDrawingHF",
    b"drawingHF",
    b"picture",
    b"oleObjects",
    b"controls",
    b"webPublishItems",
    b"tableParts",
    b"extLst",
];

/// The sheet part with its manual breaks on `axis` replaced by `breaks`,
/// each the zero-based row (or column) it sits above (or left of). Breaks
/// pagination placed (`man="0"`) are kept; no breaks at all takes the
/// element away.
pub fn set_manual_breaks(xml: &str, axis: Axis, breaks: &BTreeSet<u32>) -> Result<String> {
    let (element, after, max) = match axis {
        Axis::Rows => (
            b"rowBreaks".as_slice(),
            AFTER_ROW_BREAKS,
            MAX_COLUMN_COUNT - 1,
        ),
        Axis::Columns => (
            b"colBreaks".as_slice(),
            &AFTER_ROW_BREAKS[1..],
            MAX_ROW_COUNT - 1,
        ),
    };
    let mut reader = Reader::from_str(xml);
    let mut prefix = String::new();
    let mut existing: Option<(usize, usize)> = None;
    let mut automatic: BTreeSet<u32> = BTreeSet::new();
    let mut insert_at = None;
    let mut buf = Vec::new();
    let mut depth = 0usize;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                let name = e.name().as_ref().to_vec();
                if depth == 0 {
                    prefix = namespace_prefix(&name);
                } else if depth == 1 && local(&name) == element {
                    if !empty {
                        // Keep the breaks pagination placed.
                        let mut inner = Vec::new();
                        loop {
                            match reader.read_event_into(&mut inner)? {
                                Event::Start(ref brk) | Event::Empty(ref brk)
                                    if local(brk.name().as_ref()) == b"brk" =>
                                {
                                    let manual = attribute(brk, b"man")?
                                        .is_some_and(|value| value != "0" && value != "false");
                                    if let Some(id) = attribute(brk, b"id")?
                                        .and_then(|value| value.parse::<u32>().ok())
                                        .filter(|_| !manual)
                                    {
                                        automatic.insert(id);
                                    }
                                }
                                Event::End(ref end) if end.name() == QName(&name) => break,
                                Event::Eof => break,
                                _ => {}
                            }
                            inner.clear();
                        }
                    }
                    existing = Some((start, reader.buffer_position() as usize));
                    buf.clear();
                    continue;
                } else if depth == 1 && insert_at.is_none() && after.contains(&local(&name)) {
                    insert_at = Some(start);
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 && insert_at.is_none() {
                    insert_at = Some(start);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    let name = String::from_utf8_lossy(element);
    let mut all: Vec<(u32, bool)> = automatic.iter().map(|id| (*id, false)).collect();
    all.extend(breaks.iter().map(|id| (*id, true)));
    // A manual break and one pagination placed at the same row: the manual.
    all.sort_unstable_by_key(|(id, manual)| (*id, !manual));
    all.dedup_by_key(|(id, _)| *id);
    let text = if all.is_empty() {
        String::new()
    } else {
        let manual = all.iter().filter(|(_, manual)| *manual).count();
        let mut text = format!(
            "<{prefix}{name} count=\"{}\" manualBreakCount=\"{manual}\">",
            all.len()
        );
        for (id, manual) in all {
            text.push_str(&format!(
                "<{prefix}brk id=\"{id}\" max=\"{max}\"{}/>",
                if manual { " man=\"1\"" } else { "" }
            ));
        }
        text.push_str(&format!("</{prefix}{name}>"));
        text
    };
    let splice = match (existing, insert_at) {
        (Some((start, end)), _) => Splice { start, end, text },
        (None, _) if text.is_empty() => return Ok(xml.to_string()),
        (None, Some(at)) => Splice {
            start: at,
            end: at,
            text,
        },
        (None, None) => return Ok(xml.to_string()),
    };
    Ok(apply(xml, vec![splice]))
}
