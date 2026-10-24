//! Keep filter criteria attached to their original columns during grid edits.

use jade_xlsx_reader::{Axis, CellRange, StructureChange};
use quick_xml::{
    events::{BytesStart, Event},
    Reader,
};

use crate::shift::range_text;
use crate::Result;

pub(crate) fn shift_auto_filter(
    xml: &str,
    change: &StructureChange,
    mapped: CellRange,
) -> Result<String> {
    let original = root_range(xml)?;
    let moved = crate::xml_edit::root_attributes(xml, &[("ref", Some(range_text(mapped)))])?;
    let moved = crate::cell_shift::children_map(&moved, b"filterColumn", |column| {
        if original.is_none() {
            return Ok(column.into());
        }
        let Some(id) = root_attribute(column, b"colId")?.and_then(|s| s.parse::<u32>().ok()) else {
            return Ok(column.into());
        };
        let old = original.and_then(|r| {
            r.start
                .column
                .checked_add(id)
                .filter(|c| *c <= r.end.column)
        });
        let next = old
            .and_then(|c| match change.axis {
                Axis::Columns => change.map_index(c),
                Axis::Rows => Some(c),
            })
            .filter(|c| *c >= mapped.start.column && *c <= mapped.end.column);
        match next {
            Some(c) => crate::xml_edit::root_attributes(
                column,
                &[("colId", Some((c - mapped.start.column).to_string()))],
            ),
            None => Ok(String::new()),
        }
    })?;
    crate::cell_shift::children_map(&moved, b"sortState", |s| {
        Ok(shift_sort_state(s, change)?.unwrap_or_default())
    })
}
pub(crate) fn shift_sort_state(xml: &str, change: &StructureChange) -> Result<Option<String>> {
    let mut moved = xml.to_owned();
    if let Some(old) = root_range(xml)? {
        let Some(mapped) = change.map_range(old) else {
            return Ok(None);
        };
        moved = crate::xml_edit::root_attributes(&moved, &[("ref", Some(range_text(mapped)))])?;
    }
    let had = !crate::xml_edit_ns::children(&moved, b"sortCondition")?.is_empty();
    moved = crate::cell_shift::children_map(&moved, b"sortCondition", |condition| {
        let Some(old) = root_range(condition)? else {
            return Ok(condition.into());
        };
        match change.map_range(old) {
            Some(mapped) => {
                crate::xml_edit::root_attributes(condition, &[("ref", Some(range_text(mapped)))])
            }
            None => Ok(String::new()),
        }
    })?;
    Ok(
        (!had || !crate::xml_edit_ns::children(&moved, b"sortCondition")?.is_empty())
            .then_some(moved),
    )
}
fn root_range(xml: &str) -> Result<Option<CellRange>> {
    Ok(root_attribute(xml, b"ref")?.and_then(|s| CellRange::parse(&s).ok()))
}
fn root_attribute(xml: &str, key: &[u8]) -> Result<Option<String>> {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => return attribute(&e, key),
            Event::Eof => return Ok(None),
            _ => {}
        }
    }
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
