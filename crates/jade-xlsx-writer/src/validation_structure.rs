//! Whole-axis edits of interpreted input rules. Coverage and formulas share
//! an original source anchor; opaque XML is never a source of references.
use std::collections::BTreeMap;

use jade_xlsx_reader::{
    data_validations::{validation_records, ValidationRecord},
    shift_formula, translate_formula, Axis, CellRange, CellRef, StructureChange,
};

use crate::{
    patch::{apply, Splice},
    shift::range_text,
    xml_edit, xml_edit_ns, Result,
};

/// Replacements keyed by the original container offset. The worksheet walker
/// consumes the entire original container, excluding its descendants from
/// the generic cell/formula pass, including unknown validation lookalikes.
pub(crate) fn containers(
    xml: &str,
    formula_sheet: &str,
    changed: bool,
    change: &StructureChange,
) -> Result<BTreeMap<usize, String>> {
    let mut groups: BTreeMap<usize, Vec<ValidationRecord>> = BTreeMap::new();
    for record in validation_records(xml)? {
        groups
            .entry(record.container.start)
            .or_default()
            .push(record);
    }
    let mut result = BTreeMap::new();
    for (start, records) in groups {
        let container = xml_edit_ns::fragment(xml, records[0].container.clone())?;
        let entries = xml_edit_ns::children(&container.text, b"dataValidation")?;
        let mut edits = Vec::new();
        let mut removed = 0;
        for record in records {
            let Some((start, end, entry)) = entries.get(record.index) else {
                continue;
            };
            let base = record.rule.ranges[0].start;
            let (ranges, source) = if changed {
                let mapped: Vec<_> = record
                    .rule
                    .ranges
                    .iter()
                    .filter_map(|r| {
                        map_range(*r, change).map(|target| (surviving_start(*r, change), target))
                    })
                    .collect();
                let Some((source, _)) = mapped.first() else {
                    removed += 1;
                    edits.push(Splice {
                        start: *start,
                        end: *end,
                        text: String::new(),
                    });
                    continue;
                };
                (
                    mapped.iter().map(|(_, target)| *target).collect::<Vec<_>>(),
                    *source,
                )
            } else {
                (record.rule.ranges.clone(), base)
            };
            let mut moved = crate::validation_sources::map_formulas(
                &entry.text,
                record.extended,
                &|formula| {
                    shift_formula(
                        &translate_formula(formula, base, source),
                        formula_sheet,
                        change,
                    )
                },
            )?;
            if record.extended && changed {
                moved = crate::validation_sources::map_extended_ranges(&moved, &|text| {
                    let Ok(ranges) = text
                        .split_ascii_whitespace()
                        .map(CellRange::parse)
                        .collect::<std::result::Result<Vec<_>, _>>()
                    else {
                        return text.into();
                    };
                    let mapped = ranges
                        .iter()
                        .filter_map(|r| map_range(*r, change))
                        .collect::<Vec<_>>();
                    if mapped == ranges {
                        text.into()
                    } else {
                        mapped
                            .into_iter()
                            .map(range_text)
                            .collect::<Vec<_>>()
                            .join(" ")
                    }
                })?;
            } else if ranges != record.rule.ranges {
                let sqref = ranges
                    .into_iter()
                    .map(range_text)
                    .collect::<Vec<_>>()
                    .join(" ");
                moved = xml_edit::root_attributes(&moved, &[("sqref", Some(sqref))])?;
            }
            if moved != entry.text {
                edits.push(Splice {
                    start: *start,
                    end: *end,
                    text: entry.finish(&moved)?,
                });
            }
        }
        let mut moved = apply(&container.text, edits);
        if removed != 0 {
            moved = xml_edit::root_attributes(
                &moved,
                &[("count", Some((entries.len() - removed).to_string()))],
            )?;
            // A foreign child or an unmodeled rule is still owned by this
            // container. Retain it even after all interpreted coverage goes.
            if entries.len() == removed && !has_content(&moved)? {
                moved.clear();
            }
        }
        if moved == container.text {
            continue;
        }
        let replacement = if moved.is_empty() {
            moved
        } else {
            container.finish(&moved)?
        };
        result.insert(start, replacement);
    }
    Ok(result)
}

fn surviving_start(range: CellRange, change: &StructureChange) -> CellRef {
    let mut source = range.start;
    if !change.inserted {
        let index = match change.axis {
            Axis::Rows => &mut source.row,
            Axis::Columns => &mut source.column,
        };
        if *index >= change.at && *index < change.at.saturating_add(change.count) {
            *index = change.at.saturating_add(change.count);
        }
    }
    source
}

// Insertion can push the far end beyond the grid without discarding every
// covered cell. Retain the intersection instead of dropping the whole rule.
pub(crate) fn map_range(range: CellRange, change: &StructureChange) -> Option<CellRange> {
    if !change.inserted {
        return change.map_range(range);
    }
    let start = change.map_cell(range.start)?;
    let mut end = range.end;
    match change.axis {
        Axis::Rows => {
            end.row = change
                .map_index(end.row)
                .unwrap_or(jade_xlsx_reader::model::MAX_ROW_COUNT - 1)
        }
        Axis::Columns => {
            end.column = change
                .map_index(end.column)
                .unwrap_or(jade_xlsx_reader::model::MAX_COLUMN_COUNT - 1)
        }
    }
    Some(CellRange { start, end })
}

fn has_content(xml: &str) -> Result<bool> {
    use quick_xml::{events::Event, Reader};
    let mut reader = Reader::from_str(xml);
    let mut root = true;
    loop {
        match reader.read_event()? {
            Event::Start(_) | Event::Empty(_) if root => root = false,
            Event::Start(_)
            | Event::Empty(_)
            | Event::Comment(_)
            | Event::PI(_)
            | Event::CData(_) => return Ok(true),
            Event::Text(e) if !e.decode()?.trim().is_empty() => return Ok(true),
            Event::GeneralRef(_) => return Ok(true),
            Event::Eof => return Ok(false),
            _ => {}
        }
    }
}
