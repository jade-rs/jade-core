//! Keep filter criteria attached to their original columns during grid edits.

use jade_xlsx_reader::{Axis, CellRange, StructureChange};
use quick_xml::{
    events::{BytesStart, Event},
    name::QName,
    Reader,
};

use crate::patch::{apply, escape, local, Splice};
use crate::shift::range_text;
use crate::Result;

pub(crate) fn shift_auto_filter(
    xml: &str,
    change: &StructureChange,
    mapped: CellRange,
) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    let mut splices = Vec::new();
    let mut depth = 0usize;
    let mut original = None;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let name = element.name().as_ref().to_vec();
                let head_end = reader.buffer_position() as usize;
                if depth == 0 {
                    original = attribute(element, b"ref")?
                        .and_then(|reference| CellRange::parse(&reference).ok());
                    splices.push(Splice {
                        start,
                        end: head_end,
                        text: rewrite_attribute(element, "ref", &range_text(mapped), empty)?,
                    });
                } else if depth == 1 && local(&name) == b"filterColumn" {
                    if let (Some(original), Some(id)) = (
                        original,
                        attribute(element, b"colId")?.and_then(|id| id.parse::<u32>().ok()),
                    ) {
                        // colId is relative to the filter range, not the worksheet.
                        let column = original.start.column.checked_add(id);
                        let column = column.filter(|column| *column <= original.end.column);
                        let moved = column.and_then(|column| match change.axis {
                            Axis::Columns => change.map_index(column),
                            Axis::Rows => Some(column),
                        });
                        let moved = moved.filter(|column| {
                            *column >= mapped.start.column && *column <= mapped.end.column
                        });
                        match moved {
                            Some(column) => splices.push(Splice {
                                start,
                                end: head_end,
                                text: rewrite_attribute(
                                    element,
                                    "colId",
                                    &(column - mapped.start.column).to_string(),
                                    empty,
                                )?,
                            }),
                            None => {
                                if !empty {
                                    reader.read_to_end(QName(&name))?;
                                }
                                splices.push(Splice {
                                    start,
                                    end: reader.buffer_position() as usize,
                                    text: String::new(),
                                });
                                continue;
                            }
                        }
                    }
                } else if depth == 1 && local(&name) == b"sortState" {
                    if !empty {
                        reader.read_to_end(QName(&name))?;
                    }
                    let end = reader.buffer_position() as usize;
                    splices.push(Splice {
                        start,
                        end,
                        text: shift_sort_state(&xml[start..end], change)?.unwrap_or_default(),
                    });
                    continue;
                }
                depth += usize::from(!empty);
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(apply(xml, splices))
}

pub(crate) fn shift_sort_state(xml: &str, change: &StructureChange) -> Result<Option<String>> {
    let mut reader = Reader::from_str(xml);
    let mut splices = Vec::new();
    let mut depth = 0usize;
    let mut conditions = 0usize;
    let mut kept = 0usize;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let name = element.name().as_ref().to_vec();
                let head_end = reader.buffer_position() as usize;
                if depth == 0 || (depth == 1 && local(&name) == b"sortCondition") {
                    let condition = depth == 1;
                    conditions += usize::from(condition);
                    if let Some(range) = attribute(element, b"ref")?
                        .and_then(|reference| CellRange::parse(&reference).ok())
                    {
                        match change.map_range(range) {
                            Some(mapped) => splices.push(Splice {
                                start,
                                end: head_end,
                                text: rewrite_attribute(
                                    element,
                                    "ref",
                                    &range_text(mapped),
                                    empty,
                                )?,
                            }),
                            None if !condition => return Ok(None),
                            None => {
                                if !empty {
                                    reader.read_to_end(QName(&name))?;
                                }
                                splices.push(Splice {
                                    start,
                                    end: reader.buffer_position() as usize,
                                    text: String::new(),
                                });
                                continue;
                            }
                        }
                    }
                    kept += usize::from(condition);
                }
                depth += usize::from(!empty);
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Eof => break,
            _ => {}
        }
    }
    // A sort with no surviving keys must not claim that a new column was sorted.
    Ok((conditions == 0 || kept > 0).then(|| apply(xml, splices)))
}

// SpreadsheetML's reference/index attributes are unqualified. Extension
// attributes with the same local name must retain their own values.
pub(crate) fn attribute(element: &BytesStart<'_>, key: &[u8]) -> Result<Option<String>> {
    for attribute in element.attributes() {
        let attribute = attribute?;
        if attribute.key.as_ref() == key {
            return Ok(Some(attribute.unescape_value()?.into_owned()));
        }
    }
    Ok(None)
}

fn rewrite_attribute(
    element: &BytesStart<'_>,
    key: &str,
    value: &str,
    empty: bool,
) -> Result<String> {
    let mut head = format!("<{}", String::from_utf8_lossy(element.name().as_ref()));
    let mut seen = false;
    for attribute in element.attributes() {
        let attribute = attribute?;
        let changed = attribute.key.as_ref() == key.as_bytes();
        seen |= changed;
        let original = attribute.unescape_value()?;
        head.push_str(&format!(
            " {}=\"{}\"",
            String::from_utf8_lossy(attribute.key.as_ref()),
            escape(if changed { value } else { &original })
        ));
    }
    if !seen {
        head.push_str(&format!(" {key}=\"{}\"", escape(value)));
    }
    head.push_str(if empty { "/>" } else { ">" });
    Ok(head)
}
