//! Sparse snapshot pasting with complete-axis defaults and merged ranges.
use crate::{
    patch::{apply, attribute, escape, local, namespace_prefix, Splice},
    CellEdit, NewValue, Result, SheetEdits, WriteError,
};
use jade_opc::OpcPackage;
use jade_xlsx_reader::{
    model::{MAX_COLUMN_COUNT, MAX_ROW_COUNT},
    Cell, CellRange, CellRef, CellValue, Sheet,
};
use quick_xml::{events::Event, name::QName, Reader};
use std::collections::{BTreeMap, BTreeSet};

pub const CLIPBOARD_CELL_LIMIT: u64 = 1_000_000;
pub struct PasteRequest<'a> {
    pub source_sheet: &'a str,
    pub source: CellRange,
    pub target_sheet: &'a str,
    pub target: CellRef,
    pub cut: bool,
    pub keep_dimensions: bool,
}
#[derive(Clone, Copy, PartialEq)]
struct Dimension {
    size: f32,
    custom: bool,
    hidden: bool,
}

/// Both archives are history states of the same workbook. A cut is invalidated
/// by intervening writes so clearing its source cannot discard newer content.
pub fn paste_snapshot(
    original: &[u8],
    snapshot: &[u8],
    request: &PasteRequest<'_>,
) -> Result<(Vec<u8>, CellRange)> {
    if request.cut && original != snapshot {
        return Err(WriteError::Clipboard(
            "the cut was invalidated by a workbook edit; cut the cells again".into(),
        ));
    }
    let source_book = jade_xlsx_reader::read_xlsx(snapshot)?;
    let target_book = jade_xlsx_reader::read_xlsx(original)?;
    let source = source_book
        .sheets
        .iter()
        .find(|sheet| sheet.name.eq_ignore_ascii_case(request.source_sheet))
        .ok_or_else(|| WriteError::NoSuchSheet(request.source_sheet.into()))?;
    let target = target_book
        .sheets
        .iter()
        .find(|sheet| sheet.name.eq_ignore_ascii_case(request.target_sheet))
        .ok_or_else(|| WriteError::NoSuchSheet(request.target_sheet.into()))?;
    let range = request.source;
    if range.start.column > range.end.column
        || range.start.row > range.end.row
        || range.end.column >= MAX_COLUMN_COUNT
        || range.end.row >= MAX_ROW_COUNT
    {
        return Err(WriteError::Clipboard("invalid source range".into()));
    }
    let width = range.end.column - range.start.column + 1;
    let height = range.end.row - range.start.row + 1;
    let end = CellRef::new(
        request.target.column.saturating_add(width - 1),
        request.target.row.saturating_add(height - 1),
    );
    if end.column >= MAX_COLUMN_COUNT || end.row >= MAX_ROW_COUNT {
        return Err(WriteError::CellOutOfBounds {
            column: end.column,
            row: end.row,
        });
    }
    let destination = CellRange {
        start: request.target,
        end,
    };
    let rows = width == MAX_COLUMN_COUNT;
    let columns = height == MAX_ROW_COUNT;
    let whole_sheet = rows && columns;
    let source_merges = checked_merges(source, range)?;
    let _ = checked_merges(target, destination)?;
    if source_merges.iter().any(|merge| {
        let moved = CellRange {
            start: CellRef::new(
                request.target.column + merge.start.column - range.start.column,
                request.target.row + merge.start.row - range.start.row,
            ),
            end: CellRef::new(
                request.target.column + merge.end.column - range.start.column,
                request.target.row + merge.end.row - range.start.row,
            ),
        };
        target
            .tables
            .iter()
            .any(|table| intersects(table.reference, moved))
    }) {
        return Err(WriteError::Clipboard(
            "merged cells cannot be pasted into an Excel table".into(),
        ));
    }
    let stored = source
        .rows
        .iter()
        .flat_map(|row| &row.cells)
        .filter(|cell| range.contains(cell.reference))
        .count()
        + target
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .filter(|cell| destination.contains(cell.reference))
            .count();
    let custom = source
        .rows
        .iter()
        .filter(|row| row.custom_format)
        .count()
        .max(target.rows.iter().filter(|row| row.custom_format).count());
    let minimum = if whole_sheet {
        u64::from(MAX_COLUMN_COUNT) + source.rows.len() as u64 + target.rows.len() as u64
    } else if rows {
        u64::from(height)
    } else if columns {
        u64::from(width) * (custom as u64 + 1)
    } else {
        u64::from(width) * u64::from(height) * if request.cut { 2 } else { 1 }
    };
    if stored as u64 + minimum > CLIPBOARD_CELL_LIMIT {
        return Err(WriteError::Clipboard(format!(
            "paste exceeds the {CLIPBOARD_CELL_LIMIT} metadata/cell limit"
        )));
    }
    let source_cells: BTreeMap<_, _> = source
        .rows
        .iter()
        .flat_map(|row| &row.cells)
        .filter(|cell| range.contains(cell.reference))
        .map(|cell| (cell.reference, cell))
        .collect();
    let target_cells: Vec<_> = target
        .rows
        .iter()
        .flat_map(|row| &row.cells)
        .filter(|cell| destination.contains(cell.reference))
        .collect();
    let source_custom_rows: BTreeSet<_> = source
        .rows
        .iter()
        .filter(|row| row.custom_format && row.style_index.is_some())
        .map(|row| row.index)
        .collect();
    let target_custom_rows: BTreeSet<_> = target
        .rows
        .iter()
        .filter(|row| row.custom_format && row.style_index.is_some())
        .map(|row| row.index)
        .collect();
    let source_column_styles: Vec<_> = (0..MAX_COLUMN_COUNT)
        .map(|column| {
            source
                .column_span(column)
                .and_then(|span| span.style_index)
                .unwrap_or(0)
        })
        .collect();
    let styled_columns: Vec<_> = source_column_styles
        .iter()
        .enumerate()
        .filter(|(_, style)| **style != 0)
        .map(|(column, _)| column as u32)
        .collect();
    let intersections = if whole_sheet {
        0
    } else if rows {
        u64::from(height) * styled_columns.len() as u64
    } else if columns {
        u64::from(width) * source_custom_rows.union(&target_custom_rows).count() as u64
    } else {
        u64::from(width) * u64::from(height) * if request.cut { 2 } else { 1 }
    };
    let metadata = if whole_sheet {
        u64::from(MAX_COLUMN_COUNT) + source.rows.len() as u64 + target.rows.len() as u64
    } else if rows {
        u64::from(height)
    } else if columns {
        u64::from(width)
    } else {
        0
    };
    let count = intersections + metadata + source_cells.len() as u64 + target_cells.len() as u64;
    if count > CLIPBOARD_CELL_LIMIT {
        return Err(WriteError::Clipboard(format!(
            "paste exceeds the {CLIPBOARD_CELL_LIMIT} metadata/cell limit"
        )));
    }
    let (styles_part, mut styles) = crate::styles_part(original)?;
    let (_, snapshot_styles) = crate::styles_part(snapshot)?;
    let mut imported = BTreeMap::new();
    let mut import = |index: u32| -> Result<u32> {
        if let Some(index) = imported.get(&index) {
            return Ok(*index);
        }
        let (xml, mapped) = crate::styles::import_cell_format(&styles, &snapshot_styles, index)?;
        styles = xml;
        imported.insert(index, mapped);
        Ok(mapped)
    };
    let at_target = |at: CellRef| {
        CellRef::new(
            request.target.column + at.column - range.start.column,
            request.target.row + at.row - range.start.row,
        )
    };
    let mut edits: BTreeMap<CellRef, CellEdit> = target_cells
        .iter()
        .map(|cell| {
            (
                cell.reference,
                CellEdit {
                    reference: cell.reference,
                    value: NewValue::Remove,
                    style_index: None,
                },
            )
        })
        .collect();
    for (at, cell) in &source_cells {
        let to = at_target(*at);
        let style = source
            .effective_style_index(*at, cell.style_index)
            .unwrap_or(0);
        edits.insert(
            to,
            CellEdit {
                reference: to,
                value: copied_value(cell, to, request),
                style_index: Some(import(style)?),
            },
        );
    }
    let mut row_defaults = BTreeMap::new();
    let mut column_defaults = BTreeMap::new();
    let mut row_dimensions = BTreeMap::new();
    let mut column_dimensions = BTreeMap::new();
    if rows && !whole_sheet {
        for row in range.start.row..=range.end.row {
            let source_row = source.rows.iter().find(|candidate| candidate.index == row);
            let style = source_row
                .filter(|row| row.custom_format)
                .and_then(|row| row.style_index)
                .unwrap_or(0);
            let to = at_target(CellRef::new(0, row)).row;
            row_defaults.insert(to, import(style)?);
            if request.keep_dimensions {
                row_dimensions.insert(to, row_dimension(source, row));
            }
            if source_row
                .filter(|row| row.custom_format)
                .and_then(|row| row.style_index)
                .is_none()
            {
                for column in &styled_columns {
                    let from = CellRef::new(*column, row);
                    let to = at_target(from);
                    if !source_cells.contains_key(&from) {
                        edits.insert(
                            to,
                            styled_blank(to, import(source_column_styles[*column as usize])?),
                        );
                    }
                }
            }
        }
    } else if columns {
        for column in range.start.column..=range.end.column {
            let to = at_target(CellRef::new(column, 0)).column;
            column_defaults.insert(to, import(source_column_styles[column as usize])?);
            if request.keep_dimensions {
                column_dimensions.insert(to, column_dimension(source, column));
            }
        }
        if whole_sheet {
            for row in source_custom_rows.union(&target_custom_rows) {
                let style = source
                    .effective_style_index(CellRef::new(0, *row), None)
                    .unwrap_or(0);
                // A source row without a custom format must inherit the copied
                // columns; remove target row overrides rather than masking them.
                row_defaults.insert(
                    *row,
                    if source_custom_rows.contains(row) {
                        import(style)?
                    } else {
                        u32::MAX
                    },
                );
            }
            if request.keep_dimensions {
                for row in source
                    .rows
                    .iter()
                    .map(|row| row.index)
                    .chain(target.rows.iter().map(|row| row.index))
                {
                    row_dimensions.insert(row, row_dimension(source, row));
                }
            }
        } else {
            for row in source_custom_rows.union(&target_custom_rows) {
                for column in range.start.column..=range.end.column {
                    let from = CellRef::new(column, *row);
                    let to = at_target(from);
                    if !source_cells.contains_key(&from) {
                        edits.insert(
                            to,
                            styled_blank(
                                to,
                                import(source.effective_style_index(from, None).unwrap_or(0))?,
                            ),
                        );
                    }
                }
            }
        }
    } else {
        for row in range.start.row..=range.end.row {
            for column in range.start.column..=range.end.column {
                let from = CellRef::new(column, row);
                if !source_cells.contains_key(&from) {
                    let to = at_target(from);
                    edits.insert(
                        to,
                        styled_blank(
                            to,
                            import(source.effective_style_index(from, None).unwrap_or(0))?,
                        ),
                    );
                }
            }
        }
    }
    let same_sheet = source.part_name == target.part_name;
    let mut clear_edits = BTreeMap::new();
    let mut clear_rows = BTreeMap::new();
    let mut clear_columns = BTreeMap::new();
    if request.cut {
        for row in &source.rows {
            for cell in row
                .cells
                .iter()
                .filter(|cell| range.contains(cell.reference))
            {
                if same_sheet && destination.contains(cell.reference) {
                    continue;
                }
                clear_edits.insert(
                    cell.reference,
                    CellEdit {
                        reference: cell.reference,
                        value: NewValue::Remove,
                        style_index: None,
                    },
                );
            }
        }
        if rows {
            let row_indices: Vec<_> = if whole_sheet {
                source.rows.iter().map(|row| row.index).collect()
            } else {
                (range.start.row..=range.end.row).collect()
            };
            for row in row_indices {
                if !same_sheet || row < destination.start.row || row > destination.end.row {
                    clear_rows.insert(row, 0);
                }
            }
        }
        if columns {
            for column in range.start.column..=range.end.column {
                if !same_sheet
                    || column < destination.start.column
                    || column > destination.end.column
                {
                    clear_columns.insert(column, 0);
                    if !whole_sheet {
                        for row in &source_custom_rows {
                            let at = CellRef::new(column, *row);
                            clear_edits.insert(at, styled_blank(at, 0));
                        }
                    }
                }
            }
        }
        if !rows && !columns {
            for row in range.start.row..=range.end.row {
                for column in range.start.column..=range.end.column {
                    let at = CellRef::new(column, row);
                    if same_sheet && destination.contains(at) {
                        continue;
                    }
                    clear_edits.insert(at, styled_blank(at, 0));
                }
            }
        }
    }

    if same_sheet {
        edits.append(&mut clear_edits);
        row_defaults.append(&mut clear_rows);
        column_defaults.append(&mut clear_columns);
    }
    let target_edits = SheetEdits {
        part_name: target.part_name.clone(),
        cells: edits.into_values().collect(),
    };
    let mut sheets = vec![target_edits];
    if request.cut && !same_sheet {
        sheets.push(SheetEdits {
            part_name: source.part_name.clone(),
            cells: clear_edits.into_values().collect(),
        });
    }
    if request.cut {
        let mut remaining = CLIPBOARD_CELL_LIMIT.saturating_sub(
            metadata
                + sheets
                    .iter()
                    .map(|sheet| sheet.cells.len() as u64)
                    .sum::<u64>(),
        );
        for sheet in &target_book.sheets {
            let mut changed = Vec::new();
            for cell in sheet.rows.iter().flat_map(|row| &row.cells) {
                if (sheet.part_name == source.part_name && range.contains(cell.reference))
                    || (sheet.part_name == target.part_name && destination.contains(cell.reference))
                {
                    continue;
                }
                let Some(formula) = &cell.formula else {
                    continue;
                };
                let moved = jade_xlsx_reader::move_formula(
                    formula,
                    &sheet.name,
                    &sheet.name,
                    request.source_sheet,
                    range,
                    request.target_sheet,
                    request.target,
                );
                if moved != *formula {
                    if remaining == 0 {
                        return Err(WriteError::Clipboard(format!(
                            "paste exceeds the {CLIPBOARD_CELL_LIMIT} metadata/cell limit"
                        )));
                    }
                    remaining -= 1;
                    changed.push(CellEdit {
                        reference: cell.reference,
                        value: NewValue::FormulaCached {
                            formula: moved,
                            cached: cell.value.clone(),
                        },
                        style_index: None,
                    });
                }
            }
            if !changed.is_empty() {
                if let Some(edits) = sheets
                    .iter_mut()
                    .find(|edits| edits.part_name == sheet.part_name)
                {
                    edits.cells.extend(changed);
                } else {
                    sheets.push(SheetEdits {
                        part_name: sheet.part_name.clone(),
                        cells: changed,
                    });
                }
            }
        }
    }
    if sheets
        .iter()
        .map(|sheet| sheet.cells.len() as u64)
        .sum::<u64>()
        + metadata
        > CLIPBOARD_CELL_LIMIT
    {
        return Err(WriteError::Clipboard(format!(
            "paste exceeds the {CLIPBOARD_CELL_LIMIT} metadata/cell limit"
        )));
    }
    let parts = crate::table::follow_table_headers(original, &target_book, &mut sheets)?;
    let mut package = crate::prepare_xlsx_with_parts(
        original,
        Some((&styles_part, &styles)),
        &sheets,
        &parts,
        false,
    )?;
    let xml = package
        .part_str(&target.part_name)?
        .ok_or_else(|| WriteError::MissingPart(target.part_name.clone()))?;
    let xml = defaults_and_dimensions(
        &xml,
        row_defaults,
        column_defaults,
        &row_dimensions,
        &column_dimensions,
    )?;
    let xml = if whole_sheet && request.keep_dimensions {
        patch_sheet_defaults(&xml, source)?
    } else {
        xml
    };
    let mut merges: Vec<_> = target
        .merges
        .iter()
        .copied()
        .filter(|merge| {
            !(intersects(*merge, destination)
                || (request.cut && same_sheet && intersects(*merge, range)))
        })
        .collect();
    merges.extend(source_merges.into_iter().map(|merge| CellRange {
        start: at_target(merge.start),
        end: at_target(merge.end),
    }));
    let xml = patch_merges(&xml, &merges)?;
    package.replace(&target.part_name, xml.into_bytes());
    if request.cut && !same_sheet {
        let xml = package
            .part_str(&source.part_name)?
            .ok_or_else(|| WriteError::MissingPart(source.part_name.clone()))?;
        let xml = defaults_and_dimensions(
            &xml,
            clear_rows,
            clear_columns,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )?;
        let merges: Vec<_> = source
            .merges
            .iter()
            .copied()
            .filter(|merge| !intersects(*merge, range))
            .collect();
        package.replace(&source.part_name, patch_merges(&xml, &merges)?.into_bytes());
    }
    if request.cut {
        follow_cut_references(&mut package, &target_book, request)?;
    }
    let workbook_part = crate::workbook_part_name(&package)?;
    crate::shift::drop_calc_chain(&mut package, &workbook_part)?;
    crate::chart_cache::refresh_package(&mut package, &crate::VerifiedChartCells::new())?;
    Ok((package.write()?, destination))
}

fn copied_value(cell: &Cell, to: CellRef, request: &PasteRequest<'_>) -> NewValue {
    if let Some(formula) = &cell.formula {
        return NewValue::FormulaCached {
            formula: if request.cut {
                jade_xlsx_reader::move_formula(
                    formula,
                    request.source_sheet,
                    request.target_sheet,
                    request.source_sheet,
                    request.source,
                    request.target_sheet,
                    request.target,
                )
            } else {
                jade_xlsx_reader::translate_formula(formula, cell.reference, to)
            },
            cached: cell.value.clone(),
        };
    }
    match &cell.value {
        CellValue::Empty => NewValue::Blank,
        CellValue::Number(value) => NewValue::Number(*value),
        CellValue::Text(value) => NewValue::Text(value.clone()),
        CellValue::Bool(value) => NewValue::Bool(*value),
        CellValue::Error(value) => NewValue::Error(value.clone()),
    }
}

fn follow_cut_references(
    package: &mut OpcPackage,
    workbook: &jade_xlsx_reader::Workbook,
    request: &PasteRequest<'_>,
) -> Result<()> {
    let workbook_part = crate::workbook_part_name(package)?;
    let xml = package
        .part_str(&workbook_part)?
        .ok_or_else(|| WriteError::MissingPart(workbook_part.clone()))?;
    let xml = map_reference_elements(&xml, b"definedName", workbook, request)?;
    package.replace(&workbook_part, xml.into_bytes());
    let mut charts = BTreeSet::new();
    for sheet in &workbook.sheets {
        let Some(rels) = package.part_str(&jade_opc::rels_part_name(&sheet.part_name))? else {
            continue;
        };
        for drawing in jade_opc::parse_relationships(&rels)?.by_type_suffix("/drawing") {
            let part = jade_opc::resolve_target(&sheet.part_name, &drawing.target);
            let Some(rels) = package.part_str(&jade_opc::rels_part_name(&part))? else {
                continue;
            };
            let rels = jade_opc::parse_relationships(&rels)?;
            for kind in ["/chart", "/chartEx"] {
                for chart in rels.by_type_suffix(kind) {
                    charts.insert(jade_opc::resolve_target(&part, &chart.target));
                }
            }
        }
    }
    for part in charts {
        if let Some(xml) = package.part_str(&part)? {
            let patched = map_reference_elements(&xml, b"f", workbook, request)?;
            package.replace(&part, patched.into_bytes());
        }
    }
    Ok(())
}

fn map_reference_elements(
    xml: &str,
    tag: &[u8],
    workbook: &jade_xlsx_reader::Workbook,
    request: &PasteRequest<'_>,
) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    let mut splices = Vec::new();
    loop {
        let event = reader.read_event()?;
        if let Event::Start(element) = event {
            let name = element.name().as_ref().to_vec();
            if local(&name) != tag {
                continue;
            }
            let context = attribute(&element, b"localSheetId")?
                .and_then(|index| index.parse::<usize>().ok())
                .and_then(|index| workbook.sheets.get(index))
                .map(|sheet| sheet.name.as_str())
                .unwrap_or("");
            let start = reader.buffer_position() as usize;
            reader.read_to_end(QName(&name))?;
            let end = reader.buffer_position() as usize;
            let closing = xml[start..end]
                .rfind("</")
                .map(|at| start + at)
                .ok_or_else(|| WriteError::Clipboard("formula has no closing element".into()))?;
            let mut content = Reader::from_str(&xml[start..closing]);
            let mut formula = String::new();
            loop {
                match content.read_event()? {
                    Event::Text(text) => formula.push_str(&text.decode()?),
                    Event::CData(text) => formula.push_str(&text.decode()?),
                    Event::GeneralRef(entity) => {
                        if let Some(character) = entity.resolve_char_ref()? {
                            formula.push(character);
                        } else if let Some(value) =
                            quick_xml::escape::resolve_predefined_entity(&entity.decode()?)
                        {
                            formula.push_str(value);
                        }
                    }
                    Event::Eof => break,
                    _ => {}
                }
            }
            let moved = jade_xlsx_reader::move_formula(
                &formula,
                context,
                context,
                request.source_sheet,
                request.source,
                request.target_sheet,
                request.target,
            );
            if moved != formula {
                splices.push(Splice {
                    start,
                    end: closing,
                    text: escape(&moved),
                });
            }
        } else if matches!(event, Event::Eof) {
            break;
        }
    }
    Ok(apply(xml, splices))
}
fn styled_blank(reference: CellRef, style: u32) -> CellEdit {
    CellEdit {
        reference,
        value: NewValue::Blank,
        style_index: Some(style),
    }
}
fn intersects(a: CellRange, b: CellRange) -> bool {
    a.start.column <= b.end.column
        && b.start.column <= a.end.column
        && a.start.row <= b.end.row
        && b.start.row <= a.end.row
}
fn checked_merges(sheet: &Sheet, range: CellRange) -> Result<Vec<CellRange>> {
    let mut merges = Vec::new();
    for merge in sheet
        .merges
        .iter()
        .filter(|merge| intersects(**merge, range))
    {
        if !range.contains(merge.start) || !range.contains(merge.end) {
            return Err(WriteError::Clipboard(
                "cannot paste or copy only part of a merged cell".into(),
            ));
        }
        merges.push(*merge);
    }
    Ok(merges)
}
fn row_dimension(sheet: &Sheet, index: u32) -> Dimension {
    let row = sheet.rows.iter().find(|row| row.index == index);
    Dimension {
        size: row
            .and_then(|row| row.height_points)
            .or(sheet.default_row_height_points)
            .unwrap_or(15.0),
        custom: row
            .map(|row| row.custom_height)
            .unwrap_or(sheet.default_row_custom_height),
        hidden: row
            .map(|row| row.hidden)
            .unwrap_or(sheet.default_rows_hidden),
    }
}
fn column_dimension(sheet: &Sheet, index: u32) -> Dimension {
    let column = sheet.column_span(index);
    Dimension {
        size: column
            .and_then(|column| column.width_chars)
            .or(sheet.default_column_width_chars)
            .unwrap_or(jade_xlsx_layout::units::DEFAULT_COLUMN_WIDTH_CHARS),
        custom: column.is_some_and(|column| column.custom_width),
        hidden: column.is_some_and(|column| column.hidden),
    }
}

fn defaults_and_dimensions(
    xml: &str,
    mut rows: BTreeMap<u32, u32>,
    columns: BTreeMap<u32, u32>,
    row_dimensions: &BTreeMap<u32, Dimension>,
    column_dimensions: &BTreeMap<u32, Dimension>,
) -> Result<String> {
    let cleared: BTreeSet<_> = rows
        .iter()
        .filter(|(_, style)| **style == u32::MAX)
        .map(|(row, _)| *row)
        .collect();
    rows.retain(|_, style| *style != u32::MAX);
    // Dimensions also need an explicit row, even when its style is inherited.
    for row in row_dimensions.keys() {
        rows.entry(*row).or_insert(u32::MAX);
    }
    let mut inherit = cleared;
    inherit.extend(
        rows.iter()
            .filter(|(_, style)| **style == u32::MAX)
            .map(|(row, _)| *row),
    );
    for style in rows.values_mut().filter(|style| **style == u32::MAX) {
        *style = 0;
    }
    let xml = crate::format_axes::patch_defaults(xml, rows, columns)?;
    let mut reader = Reader::from_str(&xml);
    let mut splices = Vec::new();
    let mut next_row = 0;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        let empty = matches!(event, Event::Empty(_));
        if let Event::Start(element) | Event::Empty(element) = event {
            let name = element.name().as_ref().to_vec();
            let end = reader.buffer_position() as usize;
            if local(&name) == b"row" {
                let index = attribute(&element, b"r")?
                    .and_then(|value| value.parse::<u32>().ok())
                    .and_then(|row| row.checked_sub(1))
                    .unwrap_or(next_row);
                next_row = index + 1;
                if inherit.contains(&index) || row_dimensions.contains_key(&index) {
                    let attrs = dimension_attributes(row_dimensions.get(&index), true);
                    splices.push(Splice {
                        start,
                        end,
                        text: rewrite_element(
                            &element,
                            &attrs,
                            if inherit.contains(&index) {
                                &["s", "customFormat"]
                            } else {
                                &[]
                            },
                            empty,
                        )?,
                    });
                }
                if !empty {
                    reader.read_to_end(QName(&name))?;
                }
            } else if local(&name) == b"col" && !column_dimensions.is_empty() {
                let first = attribute(&element, b"min")?
                    .and_then(|value| value.parse::<u32>().ok())
                    .unwrap_or(1)
                    .saturating_sub(1);
                let last = attribute(&element, b"max")?
                    .and_then(|value| value.parse::<u32>().ok())
                    .unwrap_or(first + 1)
                    .saturating_sub(1);
                let mut out = String::new();
                let mut at = first;
                while at <= last {
                    let dimension = column_dimensions.get(&at);
                    let mut to = at;
                    while to < last && column_dimensions.get(&(to + 1)) == dimension {
                        to += 1;
                    }
                    let mut attrs = dimension_attributes(dimension, false);
                    attrs.push(("min", (at + 1).to_string()));
                    attrs.push(("max", (to + 1).to_string()));
                    out.push_str(&rewrite_element(&element, &attrs, &[], true)?);
                    at = to + 1;
                }
                if !empty {
                    reader.read_to_end(QName(&name))?;
                }
                splices.push(Splice {
                    start,
                    end: reader.buffer_position() as usize,
                    text: out,
                });
            }
        } else if matches!(event, Event::Eof) {
            break;
        }
    }
    Ok(apply(&xml, splices))
}
fn dimension_attributes(dimension: Option<&Dimension>, row: bool) -> Vec<(&'static str, String)> {
    dimension
        .map(|dimension| {
            vec![
                (
                    if row { "ht" } else { "width" },
                    if row {
                        dimension.size.to_string()
                    } else {
                        f64::from(dimension.size).to_string()
                    },
                ),
                (
                    if row { "customHeight" } else { "customWidth" },
                    u8::from(dimension.custom).to_string(),
                ),
                ("hidden", u8::from(dimension.hidden).to_string()),
            ]
        })
        .unwrap_or_default()
}

fn patch_sheet_defaults(xml: &str, source: &Sheet) -> Result<String> {
    let values = [
        (
            "customHeight",
            u8::from(source.default_row_custom_height).to_string(),
        ),
        (
            "zeroHeight",
            u8::from(source.default_rows_hidden).to_string(),
        ),
        (
            "defaultRowHeight",
            source.default_row_height_points.unwrap_or(15.0).to_string(),
        ),
        (
            "defaultColWidth",
            f64::from(
                source
                    .default_column_width_chars
                    .unwrap_or(jade_xlsx_layout::units::DEFAULT_COLUMN_WIDTH_CHARS),
            )
            .to_string(),
        ),
    ];
    let mut reader = Reader::from_str(xml);
    let mut prefix = String::new();
    let mut insert = None;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(element) | Event::Empty(element) => match local(element.name().as_ref()) {
                b"worksheet" => prefix = namespace_prefix(element.name().as_ref()),
                b"sheetFormatPr" => {
                    return Ok(apply(
                        xml,
                        vec![Splice {
                            start,
                            end: reader.buffer_position() as usize,
                            text: rewrite_element(&element, &values, &[], empty)?,
                        }],
                    ))
                }
                b"cols" | b"sheetData" => {
                    insert.get_or_insert(start);
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    let at = insert.ok_or_else(|| WriteError::Clipboard("worksheet has no sheetData".into()))?;
    Ok(apply(
        xml,
        vec![Splice {
            start: at,
            end: at,
            text: format!(
                "<{prefix}sheetFormatPr customHeight=\"{}\" zeroHeight=\"{}\" defaultRowHeight=\"{}\" defaultColWidth=\"{}\"/>",
                values[0].1, values[1].1, values[2].1, values[3].1
            ),
        }],
    ))
}
fn rewrite_element(
    element: &quick_xml::events::BytesStart<'_>,
    values: &[(&str, String)],
    remove: &[&str],
    empty: bool,
) -> Result<String> {
    let mut out = format!("<{}", String::from_utf8_lossy(element.name().as_ref()));
    for attr in element.attributes() {
        let attr = attr?;
        let key = String::from_utf8_lossy(attr.key.as_ref());
        if remove.contains(&key.as_ref()) || values.iter().any(|(name, _)| *name == key) {
            continue;
        }
        out.push_str(&format!(" {key}=\"{}\"", escape(&attr.unescape_value()?)));
    }
    for (key, value) in values {
        out.push_str(&format!(" {key}=\"{}\"", escape(value)));
    }
    out.push_str(if empty { "/>" } else { ">" });
    Ok(out)
}

fn patch_merges(xml: &str, merges: &[CellRange]) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    let mut prefix = String::new();
    let mut existing = None;
    let mut insert = None;
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event()? {
            Event::Start(element) | Event::Empty(element) => {
                let name = element.name().as_ref().to_vec();
                match local(&name) {
                    b"worksheet" => prefix = namespace_prefix(&name),
                    b"mergeCells" => {
                        let head_end = reader.buffer_position() as usize;
                        if !xml[start..head_end].trim_end().ends_with("/>") {
                            reader.read_to_end(QName(&name))?;
                        }
                        existing = Some((start, reader.buffer_position() as usize));
                    }
                    b"phoneticPr"
                    | b"conditionalFormatting"
                    | b"dataValidations"
                    | b"hyperlinks"
                    | b"printOptions"
                    | b"pageMargins"
                    | b"pageSetup"
                    | b"headerFooter"
                    | b"rowBreaks"
                    | b"colBreaks"
                    | b"drawing"
                    | b"legacyDrawing"
                    | b"tableParts"
                    | b"extLst" => {
                        insert.get_or_insert(start);
                    }
                    _ => {}
                }
            }
            Event::End(element) if local(element.name().as_ref()) == b"worksheet" => {
                insert.get_or_insert(start);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let mut text = String::new();
    if !merges.is_empty() {
        text.push_str(&format!("<{prefix}mergeCells count=\"{}\">", merges.len()));
        for merge in merges {
            let name = |cell: CellRef| {
                format!(
                    "{}{}",
                    jade_xlsx_layout::column_label(cell.column),
                    cell.row + 1
                )
            };
            text.push_str(&format!(
                "<{prefix}mergeCell ref=\"{}:{}\"/>",
                name(merge.start),
                name(merge.end)
            ));
        }
        text.push_str(&format!("</{prefix}mergeCells>"));
    }
    let (start, end) = existing
        .or_else(|| insert.map(|at| (at, at)))
        .ok_or_else(|| WriteError::Clipboard("worksheet has no root".into()))?;
    Ok(apply(xml, vec![Splice { start, end, text }]))
}
