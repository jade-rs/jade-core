//! Inserting and deleting rows and columns.
//!
//! A row that comes or goes moves everything below it: the rows and cells
//! themselves, the merges, hyperlinks, validations, conditional formats,
//! protected ranges and page breaks that cover them, the sheet's dimension,
//! and every formula anywhere in the workbook that points at the sheet —
//! including the defined names. The parts hung off the sheet follow too:
//! its tables ([`crate::table`]), the drawing with its shapes and text
//! boxes, the comments and the VML that places them, and every chart whose
//! series point at the sheet ([`crate::drawing`]). As with every other
//! write here, the parts are spliced as text, so what the change does not
//! touch comes back exactly as it was.
//!
//! Shared formulas survive an insertion (their children derive from the
//! master by position, which still holds); a deletion could take the master
//! away, so on the changed sheet a deletion writes every shared formula out
//! as the plain formula the reader already expanded it to. The calculation
//! chain, which lists formula cells by address, is dropped — Excel rebuilds
//! it — rather than rewritten.

use quick_xml::events::Event;
use quick_xml::name::QName;
use quick_xml::Reader;

use jade_opc::OpcPackage;
use jade_xlsx_reader::{shift_formula, Axis, CellRange, CellRef, Sheet, StructureChange, Workbook};

use crate::patch::{apply, attribute, escape, local, Splice};
use crate::{Result, WriteError};

/// Applies `change` to the whole workbook and returns the new `.xlsx` bytes.
///
/// `workbook` is the model read from `original`; it supplies the sheet
/// names, part names and expanded shared formulas the rewrite needs.
pub fn restructure(
    original: &[u8],
    workbook: &Workbook,
    change: &StructureChange,
) -> Result<Vec<u8>> {
    if change.count == 0 {
        return Err(WriteError::EmptyStructureChange);
    }
    let limit = match change.axis {
        Axis::Rows => jade_xlsx_reader::model::MAX_ROW_COUNT,
        Axis::Columns => jade_xlsx_reader::model::MAX_COLUMN_COUNT,
    };
    if change.at >= limit || change.count > limit.saturating_sub(change.at) {
        return Err(WriteError::StructureOutOfBounds {
            at: change.at,
            count: change.count,
        });
    }
    if !workbook
        .sheets
        .iter()
        .any(|sheet| sheet.name.eq_ignore_ascii_case(&change.sheet))
    {
        return Err(WriteError::NoSuchSheet(change.sheet.clone()));
    }

    let mut package = OpcPackage::open(original)?;
    crate::pivots::after_structure(&mut package, workbook, change)?;
    if change.inserted {
        let sheet = workbook
            .sheets
            .iter()
            .find(|sheet| sheet.name.eq_ignore_ascii_case(&change.sheet))
            .expect("sheet checked above");
        if sheet
            .rows
            .iter()
            .flat_map(|row| row.cells.iter())
            .any(|cell| {
                let index = match change.axis {
                    Axis::Rows => cell.reference.row,
                    Axis::Columns => cell.reference.column,
                };
                index >= change.at
                    && change.map_index(index).is_none()
                    && (!cell.value.is_empty()
                        || cell.formula.is_some()
                        || cell.style_index.is_some_and(|s| s != 0))
            })
        {
            return Err(WriteError::InsertionWouldDiscardCells);
        }
    }
    for sheet in &workbook.sheets {
        let xml = package
            .part_str(&sheet.part_name)?
            .ok_or_else(|| WriteError::MissingPart(sheet.part_name.clone()))?;
        let changed = sheet.name.eq_ignore_ascii_case(&change.sheet);
        let mut patched = shift_sheet(&xml, sheet, changed, change)?;
        if changed {
            patched = follow_sheet_parts(&mut package, sheet, patched, change)?;
        }
        if patched != xml {
            package.replace(&sheet.part_name, patched.into_bytes());
        }
    }

    let workbook_part = crate::workbook_part_name(&package)?;
    shift_charts(&mut package, &workbook_part, change)?;
    let workbook_xml = package
        .part_str(&workbook_part)?
        .ok_or_else(|| WriteError::MissingPart(workbook_part.clone()))?;
    let patched = shift_defined_names(&workbook_xml, change)?;
    let patched = crate::request_full_recalculation(&patched);
    package.replace(&workbook_part, patched.into_bytes());

    drop_calc_chain(&mut package, &workbook_part)?;
    crate::table_calculation::after_structure(&mut package, workbook, change)?;
    crate::chart_cache::refresh_package(&mut package, &crate::VerifiedChartCells::new())?;
    Ok(package.write()?)
}

/// A worksheet part with `change` applied. `changed` says whether this is
/// the sheet whose grid moves; on any other sheet only formulas that point
/// at it are rewritten.
pub fn shift_sheet(
    xml: &str,
    sheet: &Sheet,
    changed: bool,
    change: &StructureChange,
) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut splices: Vec<Splice> = Vec::new();
    let mut validations =
        crate::validation_structure::containers(xml, &sheet.name, changed, change)?;
    let mut protected_ranges =
        crate::protected_ranges::structure_containers(xml, changed, &|range| {
            crate::validation_structure::map_range(range, change)
                .into_iter()
                .collect()
        })?;

    // Where we are.
    let mut in_sheet_data = false;
    let mut next_row_index = 0u32;
    // The row being walked: its original index, where it lands, and the
    // column the next reference-less cell would take.
    let mut row: Option<(u32, u32, u32)> = None;
    // The cell being walked: its original reference.
    let mut cell: Option<CellRef> = None;
    // Inside `<f>`, `<formula>` (conditional formatting) or `<formula1>` /
    // `<formula2>` (data validation): where the element's content starts.
    // The content is taken whole at the closing tag, since an entity such
    // as `&amp;` arrives as an event of its own.
    let mut formula_content: Option<usize> = None;
    // Elements whose count attribute has to follow the entries kept.
    let mut list: Option<(usize, usize, usize, Vec<u8>)> = None; // start, head end, kept, name
                                                                 // Inside `<rowBreaks>` / `<colBreaks>`: which axis, and how many breaks
                                                                 // were kept, manual ones counted separately.
    let mut breaks: Option<(Axis, usize, usize, usize, usize)> = None; // axis, start, head end, kept, manual
                                                                       // Inside `<extLst>`: the x14 conditional formats, validations and
                                                                       // sparklines, whose `xm:sqref` is an element rather than an attribute.
                                                                       // Each entry is (start, local name, doomed): an element whose every
                                                                       // range went is removed whole.
    let mut ext_containers: Vec<(usize, Vec<u8>, bool)> = Vec::new();
    let mut sqref_content: Option<usize> = None;

    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let name = element.name().as_ref().to_vec();
                let head_end = reader.buffer_position() as usize;
                let whole_end = |reader: &mut Reader<&[u8]>| -> Result<usize> {
                    if !empty {
                        reader.read_to_end_into(QName(&name), &mut Vec::new())?;
                    }
                    Ok(reader.buffer_position() as usize)
                };
                if matches!(local(&name), b"protectedRanges" | b"protectedRange") {
                    let end = whole_end(&mut reader)?;
                    if let Some(text) = protected_ranges.remove(&start) {
                        if text != xml[start..end] {
                            splices.push(Splice { start, end, text });
                        }
                    }
                    continue;
                }
                if matches!(local(&name), b"dataValidations" | b"dataValidation") {
                    let end = whole_end(&mut reader)?;
                    if let Some(text) = validations.remove(&start) {
                        if text != xml[start..end] {
                            splices.push(Splice { start, end, text });
                        }
                    }
                    // Preserve unrecognized containers/records whole. Their
                    // descendants must not enter the generic formula pass.
                    continue;
                }
                match local(&name) {
                    b"sheetData" => in_sheet_data = !empty,
                    b"dimension" if changed => {
                        if let Some(range) = attribute(element, b"ref")?
                            .and_then(|value| CellRange::parse(&value).ok())
                        {
                            let mapped = change
                                .map_range(range)
                                .map(range_text)
                                .unwrap_or_else(|| "A1".to_string());
                            splices.push(Splice {
                                start,
                                end: head_end,
                                text: rewrite(element, &[("ref", Some(mapped))], empty)?,
                            });
                        }
                    }
                    b"col" if changed && change.axis == Axis::Columns => {
                        let min = attribute(element, b"min")?
                            .and_then(|value| value.parse::<u32>().ok())
                            .filter(|value| *value > 0);
                        let max = attribute(element, b"max")?
                            .and_then(|value| value.parse::<u32>().ok())
                            .filter(|value| *value > 0);
                        if let (Some(min), Some(max)) = (min, max) {
                            let end = whole_end(&mut reader)?;
                            match change.map_span(min - 1, max - 1) {
                                Some((first, last)) => splices.push(Splice {
                                    start,
                                    end,
                                    text: rewrite(
                                        element,
                                        &[
                                            ("min", Some((first + 1).to_string())),
                                            ("max", Some((last + 1).to_string())),
                                        ],
                                        true,
                                    )?,
                                }),
                                None => splices.push(Splice {
                                    start,
                                    end,
                                    text: String::new(),
                                }),
                            }
                        }
                    }
                    b"row" if in_sheet_data => {
                        let index = attribute(element, b"r")?
                            .and_then(|value| value.parse::<u32>().ok())
                            .filter(|value| *value > 0)
                            .map(|value| value - 1)
                            .unwrap_or(next_row_index);
                        next_row_index = index.saturating_add(1);
                        if changed && change.axis == Axis::Rows {
                            match change.map_index(index) {
                                None => {
                                    let end = whole_end(&mut reader)?;
                                    splices.push(Splice {
                                        start,
                                        end,
                                        text: String::new(),
                                    });
                                    continue;
                                }
                                Some(mapped) => {
                                    splices.push(Splice {
                                        start,
                                        end: head_end,
                                        text: rewrite(
                                            element,
                                            &[("r", Some((mapped + 1).to_string()))],
                                            empty,
                                        )?,
                                    });
                                    if !empty {
                                        row = Some((index, mapped, 0));
                                    }
                                }
                            }
                        } else {
                            if changed && change.axis == Axis::Columns {
                                // The column span hint no longer fits; it is
                                // optional, so drop it rather than guess.
                                if attribute(element, b"spans")?.is_some() {
                                    splices.push(Splice {
                                        start,
                                        end: head_end,
                                        text: rewrite(element, &[("spans", None)], empty)?,
                                    });
                                }
                            }
                            if !empty {
                                row = Some((index, index, 0));
                            }
                        }
                    }
                    b"c" if row.is_some() => {
                        let (row_index, mapped_row, next_column) = row.expect("a row");
                        let reference = match attribute(element, b"r")? {
                            Some(value) => CellRef::parse(&value)
                                .unwrap_or_else(|_| CellRef::new(next_column, row_index)),
                            None => CellRef::new(next_column, row_index),
                        };
                        row = Some((row_index, mapped_row, reference.column.saturating_add(1)));
                        if changed {
                            let mapped = match change.axis {
                                Axis::Rows => Some(CellRef::new(reference.column, mapped_row)),
                                Axis::Columns => change.map_cell(reference),
                            };
                            match mapped {
                                None => {
                                    let end = whole_end(&mut reader)?;
                                    splices.push(Splice {
                                        start,
                                        end,
                                        text: String::new(),
                                    });
                                    continue;
                                }
                                Some(mapped) => splices.push(Splice {
                                    start,
                                    end: head_end,
                                    text: rewrite(
                                        element,
                                        &[("r", Some(reference_text(mapped)))],
                                        empty,
                                    )?,
                                }),
                            }
                        }
                        if !empty {
                            cell = Some(reference);
                        }
                    }
                    b"brk" if changed => {
                        if let Some((axis, _, _, kept, manual)) = breaks.as_mut() {
                            let mine = *axis == change.axis;
                            let before = attribute(element, b"id")?
                                .and_then(|value| value.parse::<u32>().ok());
                            let is_manual = attribute(element, b"man")?
                                .is_some_and(|value| value != "0" && value != "false");
                            match (mine, before) {
                                (true, Some(before)) => match change.map_index(before) {
                                    Some(mapped) => {
                                        *kept += 1;
                                        *manual += usize::from(is_manual);
                                        if mapped != before {
                                            splices.push(Splice {
                                                start,
                                                end: head_end,
                                                text: rewrite(
                                                    element,
                                                    &[("id", Some(mapped.to_string()))],
                                                    empty,
                                                )?,
                                            });
                                        }
                                    }
                                    None => {
                                        let end = whole_end(&mut reader)?;
                                        splices.push(Splice {
                                            start,
                                            end,
                                            text: String::new(),
                                        });
                                        continue;
                                    }
                                },
                                _ => {
                                    *kept += 1;
                                    *manual += usize::from(is_manual);
                                }
                            }
                        }
                    }
                    b"rowBreaks" | b"colBreaks" if changed && !empty => {
                        let axis = if local(&name) == b"rowBreaks" {
                            Axis::Rows
                        } else {
                            Axis::Columns
                        };
                        breaks = Some((axis, start, head_end, 0, 0));
                    }
                    b"sparkline" if changed && !empty => {
                        ext_containers.push((start, local(&name).to_vec(), false));
                    }
                    b"sqref" if changed && !empty => {
                        sqref_content = Some(head_end);
                    }
                    b"f" if cell.is_some() => {
                        let shared = attribute(element, b"t")?.as_deref() == Some("shared");
                        if changed && !change.inserted && shared {
                            // Write the formula out in full: its master may
                            // be going away.
                            let end = whole_end(&mut reader)?;
                            let at = cell.expect("a cell");
                            if let Some(formula) = model_formula(sheet, at) {
                                let prefix = namespace_prefix(&name);
                                splices.push(Splice {
                                    start,
                                    end,
                                    text: format!(
                                        "<{prefix}f>{}</{prefix}f>",
                                        escape(&shift_formula(formula, &sheet.name, change))
                                    ),
                                });
                            }
                            continue;
                        }
                        if changed {
                            if let Some(range) = attribute(element, b"ref")?
                                .and_then(|value| CellRange::parse(&value).ok())
                            {
                                let mapped = change.map_range(range).map(range_text);
                                splices.push(Splice {
                                    start,
                                    end: head_end,
                                    text: rewrite(element, &[("ref", mapped)], empty)?,
                                });
                            }
                        }
                        if !empty {
                            formula_content = Some(head_end);
                        }
                    }
                    // `xm:f` in an extension: a sparkline's data, an x14
                    // rule's or validation's formula.
                    b"f" if !empty => {
                        formula_content = Some(head_end);
                    }
                    b"mergeCell" if changed => {
                        if let Some(range) = attribute(element, b"ref")?
                            .and_then(|value| CellRange::parse(&value).ok())
                        {
                            let end = whole_end(&mut reader)?;
                            match change
                                .map_range(range)
                                .filter(|range| range.start != range.end)
                            {
                                Some(mapped) => {
                                    splices.push(Splice {
                                        start,
                                        end,
                                        text: rewrite(
                                            element,
                                            &[("ref", Some(range_text(mapped)))],
                                            true,
                                        )?,
                                    });
                                    if let Some(list) = list.as_mut() {
                                        list.2 += 1;
                                    }
                                }
                                None => splices.push(Splice {
                                    start,
                                    end,
                                    text: String::new(),
                                }),
                            }
                        }
                    }
                    b"hyperlink" if changed => {
                        if let Some(range) = attribute(element, b"ref")?
                            .and_then(|value| CellRange::parse(&value).ok())
                        {
                            let end = whole_end(&mut reader)?;
                            match change.map_range(range) {
                                Some(mapped) => {
                                    splices.push(Splice {
                                        start,
                                        end,
                                        text: rewrite(
                                            element,
                                            &[("ref", Some(range_text(mapped)))],
                                            true,
                                        )?,
                                    });
                                    if let Some(list) = list.as_mut() {
                                        list.2 += 1;
                                    }
                                }
                                None => splices.push(Splice {
                                    start,
                                    end,
                                    text: String::new(),
                                }),
                            }
                        }
                    }
                    b"autoFilter" if changed => {
                        if let Some(range) = crate::filter::attribute(element, b"ref")?
                            .and_then(|value| CellRange::parse(&value).ok())
                        {
                            let end = whole_end(&mut reader)?;
                            match change.map_range(range) {
                                Some(mapped) => splices.push(Splice {
                                    start,
                                    end,
                                    text: {
                                        let context =
                                            crate::xml_edit_ns::fragment(xml, start..end)?;
                                        context.finish(&crate::filter::shift_auto_filter(
                                            &context.text,
                                            change,
                                            mapped,
                                        )?)?
                                    },
                                }),
                                None => splices.push(Splice {
                                    start,
                                    end,
                                    text: String::new(),
                                }),
                            }
                        }
                    }
                    b"sortState" if changed => {
                        let end = whole_end(&mut reader)?;
                        splices.push(Splice {
                            start,
                            end,
                            text: {
                                let context = crate::xml_edit_ns::fragment(xml, start..end)?;
                                match crate::filter::shift_sort_state(&context.text, change)? {
                                    Some(moved) => context.finish(&moved)?,
                                    None => String::new(),
                                }
                            },
                        });
                    }
                    b"conditionalFormatting" if changed => {
                        if attribute(element, b"sqref")?.is_none() && !empty {
                            // The x14 form: its ranges come as an `xm:sqref`
                            // child, mapped when it is reached.
                            ext_containers.push((start, local(&name).to_vec(), false));
                        }
                        if let Some(sqref) = attribute(element, b"sqref")? {
                            let kept = sqref
                                .split_whitespace()
                                .filter_map(|text| CellRange::parse(text).ok())
                                .filter_map(|range| change.map_range(range))
                                .map(range_text)
                                .collect::<Vec<_>>();
                            if kept.is_empty() {
                                let end = whole_end(&mut reader)?;
                                splices.push(Splice {
                                    start,
                                    end,
                                    text: String::new(),
                                });
                                continue;
                            }
                            splices.push(Splice {
                                start,
                                end: head_end,
                                text: rewrite(element, &[("sqref", Some(kept.join(" ")))], empty)?,
                            });
                            if let Some(list) = list.as_mut() {
                                list.2 += 1;
                            }
                        }
                    }
                    b"mergeCells" | b"hyperlinks" if changed && !empty => {
                        list = Some((start, head_end, 0, name.clone()));
                    }
                    b"formula" | b"formula1" | b"formula2" => {
                        if !empty {
                            formula_content = Some(head_end);
                        }
                    }
                    b"selection" if changed => {
                        let active = attribute(element, b"activeCell")?
                            .and_then(|value| CellRef::parse(&value).ok())
                            .map(|at| {
                                change
                                    .map_cell(at)
                                    .map(reference_text)
                                    .unwrap_or_else(|| "A1".to_string())
                            });
                        let sqref = attribute(element, b"sqref")?.map(|value| {
                            let kept = value
                                .split_whitespace()
                                .filter_map(|text| CellRange::parse(text).ok())
                                .filter_map(|range| change.map_range(range))
                                .map(range_text)
                                .collect::<Vec<_>>();
                            if kept.is_empty() {
                                "A1".to_string()
                            } else {
                                kept.join(" ")
                            }
                        });
                        if active.is_some() || sqref.is_some() {
                            let mut updates = Vec::new();
                            if let Some(active) = active {
                                updates.push(("activeCell", Some(active)));
                            }
                            if let Some(sqref) = sqref {
                                updates.push(("sqref", Some(sqref)));
                            }
                            splices.push(Splice {
                                start,
                                end: head_end,
                                text: rewrite(element, &updates, empty)?,
                            });
                        }
                    }
                    b"pane" if changed => {
                        if let Some(at) = attribute(element, b"topLeftCell")?
                            .and_then(|value| CellRef::parse(&value).ok())
                        {
                            let mapped = change
                                .map_cell(at)
                                .map(reference_text)
                                .unwrap_or_else(|| "A1".to_string());
                            splices.push(Splice {
                                start,
                                end: head_end,
                                text: rewrite(element, &[("topLeftCell", Some(mapped))], empty)?,
                            });
                        }
                    }
                    _ => {}
                }
            }
            Event::End(ref element) => match local(element.name().as_ref()) {
                b"sheetData" => in_sheet_data = false,
                b"row" => row = None,
                b"c" => cell = None,
                b"f" | b"formula" | b"formula1" | b"formula2" => {
                    if let Some(content_start) = formula_content.take() {
                        if let Some(splice) =
                            shifted_content(xml, content_start, start, &sheet.name, change)?
                        {
                            splices.push(splice);
                        }
                    }
                }
                b"sqref" => {
                    if let Some(content_start) = sqref_content.take() {
                        let kept = xml[content_start..start]
                            .split_whitespace()
                            .filter_map(|text| CellRange::parse(text).ok())
                            .filter_map(|range| change.map_range(range))
                            .map(range_text)
                            .collect::<Vec<_>>();
                        if kept.is_empty() {
                            if let Some(container) = ext_containers.last_mut() {
                                container.2 = true;
                            }
                        } else {
                            let joined = kept.join(" ");
                            if joined != xml[content_start..start] {
                                splices.push(Splice {
                                    start: content_start,
                                    end: start,
                                    text: joined,
                                });
                            }
                        }
                    }
                }
                b"conditionalFormatting" | b"sparkline" => {
                    let is_top = ext_containers.last().is_some_and(|(_, name, _)| {
                        name.as_slice() == local(element.name().as_ref())
                    });
                    if is_top {
                        if let Some((container_start, _, doomed)) = ext_containers.pop() {
                            if doomed {
                                let end = reader.buffer_position() as usize;
                                splices.push(Splice {
                                    start: container_start,
                                    end,
                                    text: String::new(),
                                });
                            }
                        }
                    }
                }
                b"rowBreaks" | b"colBreaks" => {
                    if let Some((axis, breaks_start, head_end, kept, manual)) = breaks.take() {
                        if axis == change.axis {
                            let end = reader.buffer_position() as usize;
                            if kept == 0 {
                                splices.push(Splice {
                                    start: breaks_start,
                                    end,
                                    text: String::new(),
                                });
                            } else {
                                let head = &xml[breaks_start..head_end];
                                let mut rewritten = head.to_string();
                                if head.contains("count=") {
                                    rewritten =
                                        rewrite_head_text(&rewritten, "count", &kept.to_string())?;
                                }
                                if head.contains("manualBreakCount=") {
                                    rewritten = rewrite_head_text(
                                        &rewritten,
                                        "manualBreakCount",
                                        &manual.to_string(),
                                    )?;
                                }
                                if rewritten != head {
                                    splices.push(Splice {
                                        start: breaks_start,
                                        end: head_end,
                                        text: rewritten,
                                    });
                                }
                            }
                        }
                    }
                }
                b"mergeCells" | b"hyperlinks" => {
                    if let Some((list_start, head_end, kept, name)) = list.take() {
                        let end = reader.buffer_position() as usize;
                        if kept == 0 {
                            splices.push(Splice {
                                start: list_start,
                                end,
                                text: String::new(),
                            });
                        } else {
                            let head = &xml[list_start..head_end];
                            if head.contains("count=") {
                                let rewritten =
                                    rewrite_head_text(head, "count", &kept.to_string())?;
                                let _ = name;
                                splices.push(Splice {
                                    start: list_start,
                                    end: head_end,
                                    text: rewritten,
                                });
                            }
                        }
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    let moved = apply(xml, splices);
    if changed {
        crate::protected_ranges::cleanup(&moved)
    } else {
        Ok(moved)
    }
}

/// The parts hung off the changed sheet — tables, the drawing, comments and
/// their VML — moved with it. Returns the sheet part, which changes when a
/// table went (its `<tablePart>` goes too) or a table needs header cells
/// written.
fn follow_sheet_parts(
    package: &mut OpcPackage,
    sheet: &Sheet,
    mut sheet_xml: String,
    change: &StructureChange,
) -> Result<String> {
    let rels_name = jade_opc::rels_part_name(&sheet.part_name);
    let Some(rels_xml) = package.part_str(&rels_name)? else {
        return Ok(sheet_xml);
    };
    let relationships = jade_opc::parse_relationships(&rels_xml)?;

    let mut header_cells = Vec::new();
    let mut removed_table_ids = Vec::new();
    let mut shifted_tables = std::collections::BTreeSet::new();
    let mut removed_tables = std::collections::BTreeSet::new();
    for relationship in relationships.by_type_suffix("/table") {
        let part = jade_opc::resolve_target(&sheet.part_name, &relationship.target);
        if relationship.external || !sheet.tables.iter().any(|t| t.part_name == part) {
            continue;
        }
        if removed_tables.contains(&part) {
            drop_part(package, &part, &rels_name, &relationship.id)?;
            removed_table_ids.push(relationship.id.clone());
            continue;
        }
        if !shifted_tables.insert(part.clone()) {
            continue;
        }

        let Some(xml) = package.part_str(&part)? else {
            continue;
        };
        let shifted = crate::table::shift_table(&xml, &sheet.name, change)?;
        header_cells.extend(shifted.header_cells);
        match shifted.xml {
            Some(patched) => {
                if patched != xml {
                    package.replace(&part, patched.into_bytes());
                }
            }
            None => {
                removed_tables.insert(part.clone());
                drop_part(package, &part, &rels_name, &relationship.id)?;
                removed_table_ids.push(relationship.id.clone());
            }
        }
    }
    if !removed_table_ids.is_empty() {
        sheet_xml = crate::table::remove_table_parts(&sheet_xml, &removed_table_ids)?;
    }
    if !header_cells.is_empty() {
        sheet_xml = crate::patch_sheet(&sheet_xml, &header_cells)?;
    }

    for relationship in relationships.by_type_suffix("/comments") {
        let part = jade_opc::resolve_target(&sheet.part_name, &relationship.target);
        let Some(xml) = package.part_str(&part)? else {
            continue;
        };
        match crate::drawing::shift_comments(&xml, change)? {
            Some(patched) => {
                if patched != xml {
                    package.replace(&part, patched.into_bytes());
                }
            }
            None => drop_part(package, &part, &rels_name, &relationship.id)?,
        }
    }
    for relationship in relationships.by_type_suffix("/vmlDrawing") {
        let part = jade_opc::resolve_target(&sheet.part_name, &relationship.target);
        let Some(xml) = package.part_str(&part)? else {
            continue;
        };
        let patched = crate::drawing::shift_vml(&xml, change);
        if patched != xml {
            package.replace(&part, patched.into_bytes());
        }
    }
    for relationship in relationships.by_type_suffix("/drawing") {
        let part = jade_opc::resolve_target(&sheet.part_name, &relationship.target);
        let Some(xml) = package.part_str(&part)? else {
            continue;
        };
        let patched = crate::drawing::shift_drawing(&xml, change)?;
        if patched != xml {
            package.replace(&part, patched.into_bytes());
        }
    }
    Ok(sheet_xml)
}

/// Every chart in the workbook — reached through each sheet's drawing —
/// with its series references into the changed sheet moved. A chart's
/// references are sheet-qualified, so this cannot depend on which sheet
/// the chart is drawn on.
fn shift_charts(
    package: &mut OpcPackage,
    workbook_part: &str,
    change: &StructureChange,
) -> Result<()> {
    let Some(workbook_rels) = package.part_str(&jade_opc::rels_part_name(workbook_part))? else {
        return Ok(());
    };
    let workbook_rels = jade_opc::parse_relationships(&workbook_rels)?;
    let mut charts: Vec<String> = Vec::new();
    for suffix in ["/worksheet", "/chartsheet"] {
        for sheet in workbook_rels.by_type_suffix(suffix) {
            let sheet_part = jade_opc::resolve_target(workbook_part, &sheet.target);
            let Some(sheet_rels) = package.part_str(&jade_opc::rels_part_name(&sheet_part))? else {
                continue;
            };
            let sheet_rels = jade_opc::parse_relationships(&sheet_rels)?;
            for drawing in sheet_rels.by_type_suffix("/drawing") {
                let drawing_part = jade_opc::resolve_target(&sheet_part, &drawing.target);
                let Some(drawing_rels) =
                    package.part_str(&jade_opc::rels_part_name(&drawing_part))?
                else {
                    continue;
                };
                let drawing_rels = jade_opc::parse_relationships(&drawing_rels)?;
                for chart_suffix in ["/chart", "/chartEx"] {
                    for chart in drawing_rels.by_type_suffix(chart_suffix) {
                        let part = jade_opc::resolve_target(&drawing_part, &chart.target);
                        if !charts.contains(&part) {
                            charts.push(part);
                        }
                    }
                }
            }
        }
    }
    for part in charts {
        let Some(xml) = package.part_str(&part)? else {
            continue;
        };
        let patched = crate::drawing::shift_chart(&xml, change)?;
        if patched != xml {
            package.replace(&part, patched.into_bytes());
        }
    }
    Ok(())
}

/// Removes `part` — with its own `.rels`, the relationship `rel_id` in
/// `owner_rels_name` that pointed at it, and its content type override.
pub(crate) fn drop_part(
    package: &mut OpcPackage,
    part: &str,
    owner_rels_name: &str,
    rel_id: &str,
) -> Result<()> {
    package.remove(part);
    package.remove(&jade_opc::rels_part_name(part));
    if let Some(rels_xml) = package.part_str(owner_rels_name)? {
        let rels_xml = remove_elements(&rels_xml, |element| {
            attribute(element, b"Id")
                .ok()
                .flatten()
                .is_some_and(|id| id == rel_id)
        })?;
        package.replace(owner_rels_name, rels_xml.into_bytes());
    }
    if let Some(types) = package.part_str("[Content_Types].xml")? {
        let wanted = format!("/{part}");
        let types = remove_elements(&types, |element| {
            local(element.name().as_ref()) == b"Override"
                && attribute(element, b"PartName")
                    .ok()
                    .flatten()
                    .is_some_and(|name| name == wanted)
        })?;
        package.replace("[Content_Types].xml", types.into_bytes());
    }
    Ok(())
}

/// `workbook.xml` with every defined name that points at the changed sheet
/// moved.
pub fn shift_defined_names(xml: &str, change: &StructureChange) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut splices = Vec::new();
    let mut content: Option<usize> = None;
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event_into(&mut buf)? {
            Event::Start(ref element) if local(element.name().as_ref()) == b"definedName" => {
                content = Some(reader.buffer_position() as usize);
            }
            Event::End(ref element) if local(element.name().as_ref()) == b"definedName" => {
                if let Some(content_start) = content.take() {
                    // A name has no sheet of its own: only qualified
                    // references can point at the changed sheet.
                    if let Some(splice) = shifted_content(xml, content_start, start, "", change)? {
                        splices.push(splice);
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(apply(xml, splices))
}

/// The formula written between `content_start` and `content_end` moved, as
/// a splice — or nothing when it does not change.
fn shifted_content(
    xml: &str,
    content_start: usize,
    content_end: usize,
    formula_sheet: &str,
    change: &StructureChange,
) -> Result<Option<Splice>> {
    let raw = &xml[content_start..content_end];
    let formula = quick_xml::escape::unescape(raw).map_err(quick_xml::Error::from)?;
    let shifted = shift_formula(&formula, formula_sheet, change);
    if shifted == formula {
        return Ok(None);
    }
    Ok(Some(Splice {
        start: content_start,
        end: content_end,
        text: escape(&shifted),
    }))
}

/// Removes the calculation chain — the part, its relationship and its
/// content type — when the workbook has one.
pub(crate) fn drop_calc_chain(package: &mut OpcPackage, workbook_part: &str) -> Result<()> {
    let rels_name = jade_opc::rels_part_name(workbook_part);
    let Some(rels_xml) = package.part_str(&rels_name)? else {
        return Ok(());
    };
    let relationships = jade_opc::parse_relationships(&rels_xml)?;
    let Some(target) = relationships.target_by_type_suffix("/calcChain") else {
        return Ok(());
    };
    let part = jade_opc::resolve_target(workbook_part, target);
    package.remove(&part);
    let rels_xml = remove_elements(&rels_xml, |element| {
        attribute(element, b"Type")
            .ok()
            .flatten()
            .is_some_and(|kind| kind.ends_with("/calcChain"))
    })?;
    package.replace(&rels_name, rels_xml.into_bytes());
    if let Some(types) = package.part_str("[Content_Types].xml")? {
        let wanted = format!("/{part}");
        let types = remove_elements(&types, |element| {
            local(element.name().as_ref()) == b"Override"
                && attribute(element, b"PartName")
                    .ok()
                    .flatten()
                    .is_some_and(|name| name == wanted)
        })?;
        package.replace("[Content_Types].xml", types.into_bytes());
    }
    Ok(())
}

/// `xml` without the elements `matches` picks (each removed whole).
pub(crate) fn remove_elements(
    xml: &str,
    matches: impl Fn(&quick_xml::events::BytesStart<'_>) -> bool,
) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut splices = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) if matches(element) => {
                let name = element.name().as_ref().to_vec();
                if matches!(event, Event::Start(_)) {
                    reader.read_to_end_into(QName(&name), &mut Vec::new())?;
                }
                let end = reader.buffer_position() as usize;
                splices.push(Splice {
                    start,
                    end,
                    text: String::new(),
                });
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(apply(xml, splices))
}

fn model_formula(sheet: &Sheet, at: CellRef) -> Option<&str> {
    sheet
        .rows
        .iter()
        .find(|row| row.index == at.row)
        .and_then(|row| row.cells.iter().find(|cell| cell.reference == at))
        .and_then(|cell| cell.formula.as_deref())
}

/// The opening tag of `element` with `updates` applied: `Some` sets an
/// attribute (replacing it where it exists), `None` removes it.
pub(crate) fn rewrite(
    element: &quick_xml::events::BytesStart<'_>,
    updates: &[(&str, Option<String>)],
    empty: bool,
) -> Result<String> {
    let mut head = String::from("<");
    head.push_str(&String::from_utf8_lossy(element.name().as_ref()));
    let mut seen = Vec::new();
    for attribute in element.attributes() {
        let attribute = attribute?;
        let key = local(attribute.key.as_ref()).to_vec();
        let update = updates
            .iter()
            .find(|(name, _)| name.as_bytes() == key.as_slice());
        match update {
            Some((name, None)) => {
                seen.push(*name);
                continue;
            }
            Some((name, Some(value))) => {
                seen.push(*name);
                head.push(' ');
                head.push_str(&String::from_utf8_lossy(attribute.key.as_ref()));
                head.push_str("=\"");
                head.push_str(&escape(value));
                head.push('"');
            }
            None => {
                head.push(' ');
                head.push_str(&String::from_utf8_lossy(attribute.key.as_ref()));
                head.push_str("=\"");
                head.push_str(&escape(&attribute.unescape_value()?));
                head.push('"');
            }
        }
    }
    for (name, value) in updates {
        if let Some(value) = value {
            if !seen.contains(name) {
                head.push(' ');
                head.push_str(name);
                head.push_str("=\"");
                head.push_str(&escape(value));
                head.push('"');
            }
        }
    }
    head.push_str(if empty { "/>" } else { ">" });
    Ok(head)
}

/// `head` — an opening tag as text — with one attribute set.
pub(crate) fn rewrite_head_text(head: &str, key: &str, value: &str) -> Result<String> {
    let mut reader = Reader::from_str(head);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(ref element) => {
                return rewrite(element, &[(key, Some(value.to_string()))], false);
            }
            Event::Empty(ref element) => {
                return rewrite(element, &[(key, Some(value.to_string()))], true);
            }
            Event::Eof => return Ok(head.to_string()),
            _ => {}
        }
    }
}

fn namespace_prefix(name: &[u8]) -> String {
    match name.iter().position(|byte| *byte == b':') {
        Some(index) => String::from_utf8_lossy(&name[..=index]).into_owned(),
        None => String::new(),
    }
}

pub(crate) fn reference_text(at: CellRef) -> String {
    format!("{}{}", column_label(at.column), at.row + 1)
}

pub(crate) fn range_text(range: CellRange) -> String {
    if range.start == range.end {
        reference_text(range.start)
    } else {
        format!(
            "{}:{}",
            reference_text(range.start),
            reference_text(range.end)
        )
    }
}

fn column_label(index: u32) -> String {
    let mut label = String::new();
    let mut value = index + 1;
    while value > 0 {
        let remainder = (value - 1) % 26;
        label.insert(0, (b'A' + remainder as u8) as char);
        value = (value - 1) / 26;
    }
    label
}
