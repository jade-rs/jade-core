//! Sparse cell insert/delete, retaining raw cell XML and row/column defaults.
use crate::{
    patch::{apply, attribute, escape, local, namespace_prefix, Splice},
    shift::{range_text, reference_text},
    xml_edit, xml_edit_ns, Result, WriteError,
};
use jade_opc::OpcPackage;
use jade_xlsx_reader::{shift_cells_formula, Axis, CellRange, CellRef, CellShift, Sheet, Workbook};
use quick_xml::{events::Event, name::ResolveResult, NsReader, Reader};
use std::collections::{BTreeMap, BTreeSet};

pub const CELL_SHIFT_LIMIT: usize = 1_000_000;

fn failure(message: &str) -> WriteError {
    WriteError::CellShift(message.into())
}
pub(crate) fn head_attr(xml: &str, key: &[u8]) -> Result<Option<String>> {
    let mut r = Reader::from_str(xml);
    loop {
        match r.read_event()? {
            Event::Start(e) | Event::Empty(e) => return attribute(&e, key),
            Event::Eof => return Err(WriteError::NoRoot("XML")),
            _ => {}
        }
    }
}
fn prefix(xml: &str) -> Result<String> {
    let mut r = Reader::from_str(xml);
    loop {
        match r.read_event()? {
            Event::Start(e) | Event::Empty(e) => return Ok(namespace_prefix(e.name().as_ref())),
            Event::Eof => return Err(WriteError::NoRoot("XML")),
            _ => {}
        }
    }
}
pub(crate) fn children_map(
    xml: &str,
    name: &[u8],
    mut map: impl FnMut(&str) -> Result<String>,
) -> Result<String> {
    let mut edits = Vec::new();
    for (start, end, context) in xml_edit_ns::children(xml, name)? {
        let changed = map(&context.text)?;
        if changed == context.text {
            continue;
        }
        edits.push(Splice {
            start,
            end,
            text: if changed.is_empty() {
                changed
            } else {
                context.finish(&changed)?
            },
        });
    }
    Ok(apply(xml, edits))
}
fn child_edit(xml: &str, name: &[u8], map: impl FnMut(&str) -> Result<String>) -> Result<String> {
    children_map(xml, name, map)
}
pub(crate) fn text_content(xml: &str, text: &str) -> Result<String> {
    let mut r = Reader::from_str(xml);
    loop {
        let event = r.read_event()?;
        match event {
            Event::Empty(_) => return xml_edit::append_child(xml, &escape(text)),
            Event::Start(e) => {
                let begin = r.buffer_position() as usize;
                let name = e.name().as_ref().to_vec();
                let span = r.read_to_end(quick_xml::name::QName(&name))?;
                return Ok(apply(
                    xml,
                    vec![Splice {
                        start: begin,
                        end: span.end as usize,
                        text: escape(text),
                    }],
                ));
            }
            Event::Eof => return Err(WriteError::NoRoot("XML")),
            _ => {}
        }
    }
}

pub(crate) fn leaf_text(xml: &str) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    let mut depth = 0usize;
    let mut result = String::new();
    loop {
        match reader.read_event()? {
            Event::Start(_) => depth += 1,
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Text(e) if depth == 1 => result.push_str(&e.decode()?),
            Event::CData(e) if depth == 1 => result.push_str(&e.decode()?),
            Event::GeneralRef(e) if depth == 1 => {
                let entity = format!("&{};", e.decode()?);
                result.push_str(
                    &quick_xml::escape::unescape(&entity).map_err(quick_xml::Error::from)?,
                );
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(result)
}

/// Every package edit is prepared before returning bytes. Callers can record
/// one history state for grid cells, dependent formulas and associated parts.
pub fn shift_cells(original: &[u8], book: &Workbook, change: &CellShift) -> Result<Vec<u8>> {
    if !change.is_valid() {
        return Err(failure("invalid selection"));
    }
    let all = match change.axis {
        Axis::Rows => jade_xlsx_reader::model::MAX_COLUMN_COUNT,
        Axis::Columns => jade_xlsx_reader::model::MAX_ROW_COUNT,
    };
    if change.strip() == (0, all - 1) {
        return crate::restructure(original, book, &change.axis_change());
    }
    let sheet = book
        .sheets
        .iter()
        .find(|s| jade_xlsx_reader::formula::same_sheet_name(&s.name, &change.sheet))
        .ok_or_else(|| WriteError::NoSuchSheet(change.sheet.clone()))?;
    preflight(sheet, change)?;
    let mut package = OpcPackage::open(original)?;
    crate::pivots::after_cells(&mut package, book, change)?;
    // Expanding all shared groups is necessary even on another sheet: the
    // reference strip can cut through a group's relative dependency pattern.
    for source in &book.sheets {
        let xml = package.required_part_str(&source.part_name)?;
        let xml = expand_shared(&xml, source)?;
        let xml = child_edit(&xml, b"hyperlinks", |links| {
            children_map(links, b"hyperlink", |link| {
                let Some(location) = head_attr(link, b"location")? else {
                    return Ok(link.into());
                };
                let at = head_attr(link, b"ref")?.and_then(|s| CellRange::parse(&s).ok());
                if source.hyperlinks.iter().any(|h| {
                    Some(h.reference) == at
                        && h.location.as_deref() == Some(&location)
                        && h.target.is_some()
                }) {
                    return Ok(link.into());
                }
                let changed_location = shift_cells_formula(&location, &source.name, change);
                if changed_location == location {
                    return Ok(link.into());
                }
                xml_edit::root_attributes(link, &[("location", Some(changed_location))])
            })
        })?;
        package.replace(&source.part_name, xml.into_bytes());
    }
    let xml = package.required_part_str(&sheet.part_name)?;
    let xml = move_grid(&xml, sheet, change)?;
    let xml = move_metadata(&xml, change)?;
    package.replace(&sheet.part_name, xml.into_bytes());
    follow_parts(&mut package, sheet, change)?;
    crate::table_formulas::rewrite_grid_package(&mut package, book, &|f, own| {
        shift_cells_formula(f, own, change)
    })?;
    crate::table_calculation::after_cells(&mut package, book, change)?;
    let source = OpcPackage::open(original)?.required_part_str(&sheet.part_name)?;
    let current = package.required_part_str(&sheet.part_name)?;
    package.replace(
        &sheet.part_name,
        move_rules(
            &source,
            &current,
            change,
            &mut crate::cell_shift_extensions::IdPool::new(&package),
        )?
        .into_bytes(),
    );
    let part = crate::workbook_part_name(&package)?;
    let xml = package.required_part_str(&part)?;
    package.replace(&part, crate::request_full_recalculation(&xml).into_bytes());
    crate::shift::drop_calc_chain(&mut package, &part)?;
    crate::chart_cache::refresh_package(&mut package, &crate::VerifiedChartCells::new())?;
    let bytes = crate::tables::follow_column_names(original, &package.write()?)?;
    let after = jade_xlsx_reader::read_xlsx(&bytes)?;
    let deleted = book
        .sheets
        .iter()
        .flat_map(|s| &s.tables)
        .filter(|t| {
            !after
                .sheets
                .iter()
                .flat_map(|s| &s.tables)
                .any(|n| n.part_name == t.part_name)
        })
        .map(|t| t.name.to_lowercase())
        .collect::<BTreeSet<_>>();
    if deleted.is_empty() {
        return Ok(bytes);
    }
    let mut package = OpcPackage::open(&bytes)?;
    crate::table_formulas::rewrite_package(&mut package, &after, &|f, _, _| {
        jade_xlsx_reader::rewrite_table_references(f, |name, _| {
            name.filter(|n| deleted.contains(&n.to_lowercase()))
                .map(|_| "#REF!".into())
        })
    })?;
    Ok(package.write()?)
}

fn preflight(sheet: &Sheet, change: &CellShift) -> Result<()> {
    let at = change.axis_change().at;
    for range in &sheet.merges {
        if change.affects(*range) && (!change.covers(*range) || change.along(range.start) < at) {
            return Err(failure("the shift would split a merged range"));
        }
    }
    for array in &sheet.array_formulas {
        if !change.affects(array.range) {
            continue;
        }
        if !change.covers(array.range) {
            return Err(failure("the shift would split an array formula"));
        }
        let mapped = change.map_reference(array.range);
        if let Some(mapped) = mapped {
            if mapped.end.row - mapped.start.row != array.range.end.row - array.range.start.row
                || mapped.end.column - mapped.start.column
                    != array.range.end.column - array.range.start.column
            {
                return Err(failure("the shift would resize an array formula"));
            }
        }
    }
    for table in &sheet.tables {
        if change.affects(table.reference) && !change.covers(table.reference) {
            return Err(failure("the shift would split an Excel table"));
        }
    }
    if change.inserted
        && sheet
            .rows
            .iter()
            .flat_map(|r| &r.cells)
            .any(|c| change.map_cell(c.reference).is_none())
    {
        return Err(WriteError::InsertionWouldDiscardCells);
    }
    if change.inserted {
        if sheet
            .comments
            .iter()
            .any(|c| change.map_cell(c.reference).is_none())
        {
            return Err(WriteError::InsertionWouldDiscardCells);
        }
        for r in sheet
            .merges
            .iter()
            .copied()
            .chain(sheet.hyperlinks.iter().map(|h| h.reference))
        {
            if !change.affects(r) {
                continue;
            }
            let mut last = r.end;
            match change.axis {
                Axis::Rows => last.column = last.column.min(change.strip().1),
                Axis::Columns => last.row = last.row.min(change.strip().1),
            }
            if change.map_cell(last).is_none() {
                return Err(WriteError::InsertionWouldDiscardCells);
            }
        }
    }
    Ok(())
}

fn expand_shared(xml: &str, sheet: &Sheet) -> Result<String> {
    let formulas: BTreeMap<_, _> = sheet
        .rows
        .iter()
        .flat_map(|r| &r.cells)
        .filter_map(|c| c.formula.as_deref().map(|f| (c.reference, f)))
        .collect();
    child_edit(xml, b"sheetData", |data| {
        let mut next_row = 0;
        children_map(data, b"row", |row| {
            let index = head_attr(row, b"r")?
                .and_then(|s| s.parse::<u32>().ok())
                .filter(|r| *r > 0)
                .map(|r| r - 1)
                .unwrap_or(next_row);
            next_row = index + 1;
            let mut column = 0;
            children_map(row, b"c", |cell| {
                let at = head_attr(cell, b"r")?
                    .and_then(|s| CellRef::parse(&s).ok())
                    .unwrap_or(CellRef::new(column, index));
                column = at.column + 1;
                children_map(cell, b"f", |f| {
                    if head_attr(f, b"t")?.as_deref() != Some("shared") {
                        return Ok(f.into());
                    }
                    let Some(formula) = formulas.get(&at) else {
                        return Err(failure("a shared formula could not be expanded"));
                    };
                    let f =
                        xml_edit::root_attributes(f, &[("t", None), ("si", None), ("ref", None)])?;
                    text_content(&f, formula)
                })
            })
        })
    })
}

fn move_grid(xml: &str, sheet: &Sheet, change: &CellShift) -> Result<String> {
    child_edit(xml, b"sheetData", |data| {
        let p = prefix(data)?;
        let mut rows: BTreeMap<u32, String> = BTreeMap::new();
        let mut cells: BTreeMap<CellRef, String> = BTreeMap::new();
        let mut next_row = 0;
        for (_, _, row) in xml_edit_ns::children(data, b"row")? {
            let index = head_attr(&row.text, b"r")?
                .and_then(|s| s.parse::<u32>().ok())
                .filter(|r| *r > 0)
                .map(|r| r - 1)
                .unwrap_or(next_row);
            next_row = index + 1;
            let mut column = 0;
            let entries = xml_edit_ns::children(&row.text, b"c")?;
            for (_, _, cell) in &entries {
                let at = head_attr(&cell.text, b"r")?
                    .and_then(|s| CellRef::parse(&s).ok())
                    .unwrap_or(CellRef::new(column, index));
                column = at.column + 1;
                if cells.insert(at, cell.text.clone()).is_some() {
                    return Err(failure("duplicate cell references"));
                }
            }
            let stripped = apply(
                &row.text,
                entries
                    .into_iter()
                    .map(|(start, end, _)| Splice {
                        start,
                        end,
                        text: String::new(),
                    })
                    .collect(),
            );
            let stripped = xml_edit::root_attributes(
                &stripped,
                &[("r", Some((index + 1).to_string())), ("spans", None)],
            )?;
            rows.insert(index, row.finish(&stripped)?);
        }
        materialize_defaults(&mut cells, sheet, change, &p)?;
        if cells.len() > CELL_SHIFT_LIMIT {
            return Err(failure("too many stored cells for a cell shift"));
        }
        let mut moved: BTreeMap<u32, BTreeMap<u32, String>> = BTreeMap::new();
        let styles: BTreeMap<_, _> = sheet
            .rows
            .iter()
            .flat_map(|r| &r.cells)
            .map(|c| (c.reference, c.style_index))
            .collect();
        for (at, cell) in cells {
            let Some(to) = change.map_cell(at) else {
                if change.inserted {
                    return Err(WriteError::InsertionWouldDiscardCells);
                }
                continue;
            };
            let mut edits = vec![("r", Some(reference_text(to)))];
            if to != at {
                let effective = sheet
                    .effective_style_index(at, styles.get(&at).copied().flatten())
                    .unwrap_or(0);
                edits.push(("s", Some(effective.to_string())));
            }
            let cell = xml_edit::root_attributes(&cell, &edits)?;
            moved.entry(to.row).or_default().insert(to.column, cell);
        }
        for index in moved.keys() {
            rows.entry(*index)
                .or_insert_with(|| format!("<{p}row r=\"{}\"/>", index + 1));
        }
        let mut body = String::new();
        for (index, mut row) in rows {
            if let Some(cells) = moved.remove(&index) {
                let cells = cells.into_values().collect::<String>();
                row = if let Some((start, _, _)) = xml_edit_ns::children(&row, b"extLst")?.first() {
                    apply(
                        &row,
                        vec![Splice {
                            start: *start,
                            end: *start,
                            text: cells,
                        }],
                    )
                } else {
                    xml_edit::append_child(&row, &cells)?
                };
            }
            body.push_str(&row);
        }
        let ranges = xml_edit_ns::children(data, b"row")?;
        let mut splices = ranges
            .iter()
            .map(|(start, end, _)| Splice {
                start: *start,
                end: *end,
                text: String::new(),
            })
            .collect::<Vec<_>>();
        if let Some((start, _, _)) = ranges.first() {
            splices[0].text = body;
            debug_assert_eq!(splices[0].start, *start);
        } else {
            return xml_edit::append_child(data, &body);
        }
        Ok(apply(data, splices))
    })
}

// Only defaults that vary along the moving axis require sparse materialization.
// A vertical move keeps its column default; a horizontal move keeps its row
// default. Destination defaults must not change a moved blank's appearance.
fn materialize_defaults(
    cells: &mut BTreeMap<CellRef, String>,
    sheet: &Sheet,
    change: &CellShift,
    p: &str,
) -> Result<()> {
    let mut positions = BTreeSet::new();
    let axis = change.axis_change();
    let inverse = jade_xlsx_reader::StructureChange {
        inserted: !axis.inserted,
        ..axis.clone()
    };
    let mut add = |at: CellRef| -> Result<()> {
        if change.map_cell(at) != Some(at) {
            positions.insert(at);
        }
        if positions.len() > CELL_SHIFT_LIMIT {
            return Err(failure("too many inherited cell formats for a cell shift"));
        }
        Ok(())
    };
    match change.axis {
        Axis::Rows => {
            for row in sheet
                .rows
                .iter()
                .filter(|r| r.custom_format && r.style_index.is_some())
            {
                if row.index < axis.at {
                    continue;
                }
                for column in change.range.start.column..=change.range.end.column {
                    add(CellRef::new(column, row.index))?;
                    if let Some(source) = inverse.map_index(row.index) {
                        add(CellRef::new(column, source))?;
                    }
                }
            }
        }
        Axis::Columns => {
            for span in sheet.columns.iter().filter(|c| c.style_index.is_some()) {
                for column in span.first.max(axis.at)..=span.last {
                    for row in change.range.start.row..=change.range.end.row {
                        add(CellRef::new(column, row))?;
                        if let Some(source) = inverse.map_index(column) {
                            add(CellRef::new(source, row))?;
                        }
                    }
                }
            }
        }
    }
    for at in positions {
        let s = sheet.effective_style_index(at, None).unwrap_or(0);
        cells
            .entry(at)
            .or_insert_with(|| format!("<{p}c r=\"{}\" s=\"{s}\"/>", reference_text(at)));
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) struct RulePiece {
    pub source: CellRef,
    pub target: CellRange,
    pub inserted: bool,
}

// Rules with relative predicates need separate anchors wherever the grid's
// translation changes. A single union sqref would incorrectly retarget cells
// outside the moving strip, or on the other side of the insertion boundary.
pub(crate) fn rule_pieces(range: CellRange, change: &CellShift) -> Vec<RulePiece> {
    let (lo, hi) = change.strip();
    let a = change.perpendicular(range.start);
    let b = change.perpendicular(range.end);
    let mut strips = Vec::new();
    if a < lo {
        strips.push((a, b.min(lo - 1), false));
    }
    if b >= lo && a <= hi {
        strips.push((a.max(lo), b.min(hi), true));
    }
    if b > hi {
        strips.push((a.max(hi + 1), b, false));
    }
    let rectangle = |first, last, low, high| match change.axis {
        Axis::Rows => CellRange {
            start: CellRef::new(low, first),
            end: CellRef::new(high, last),
        },
        Axis::Columns => CellRange {
            start: CellRef::new(first, low),
            end: CellRef::new(last, high),
        },
    };
    let axis = change.axis_change();
    let first = change.along(range.start);
    let last = change.along(range.end);
    let limit = match change.axis {
        Axis::Rows => jade_xlsx_reader::model::MAX_ROW_COUNT,
        Axis::Columns => jade_xlsx_reader::model::MAX_COLUMN_COUNT,
    };
    let mut result = Vec::new();
    for (low, high, moving) in strips {
        if !moving || last < axis.at {
            let target = rectangle(first, last, low, high);
            result.push(RulePiece {
                source: target.start,
                target,
                inserted: false,
            });
            continue;
        }
        if first < axis.at {
            let target = rectangle(first, axis.at - 1, low, high);
            result.push(RulePiece {
                source: target.start,
                target,
                inserted: false,
            });
            if axis.inserted {
                let target = rectangle(
                    axis.at,
                    axis.at.saturating_add(axis.count - 1).min(limit - 1),
                    low,
                    high,
                );
                let source = rectangle(axis.at - 1, axis.at - 1, low, high).start;
                result.push(RulePiece {
                    source,
                    target,
                    inserted: true,
                });
            }
        }
        let retained = first.max(if axis.inserted {
            axis.at
        } else {
            axis.at + axis.count
        });
        if retained <= last {
            let source = rectangle(retained, last, low, high);
            let Some(start) = change.map_cell(source.start) else {
                continue;
            };
            let end = change.map_cell(source.end).unwrap_or_else(|| {
                let mut end = source.end;
                match change.axis {
                    Axis::Rows => end.row = limit - 1,
                    Axis::Columns => end.column = limit - 1,
                };
                end
            });
            result.push(RulePiece {
                source: source.start,
                target: CellRange { start, end },
                inserted: false,
            });
        }
    }
    result
}

pub(crate) fn rule_formula(
    formula: &str,
    base: CellRef,
    piece: RulePiece,
    change: &CellShift,
) -> String {
    let at_source = jade_xlsx_reader::translate_formula(formula, base, piece.source);
    let shifted = shift_cells_formula(&at_source, &change.sheet, change);
    if piece.inserted {
        jade_xlsx_reader::translate_formula(&shifted, piece.source, piece.target.start)
    } else {
        shifted
    }
}

fn move_rule(entry: &str, change: &CellShift, conditional: bool) -> Result<Vec<String>> {
    let original = head_attr(entry, b"sqref")?
        .unwrap_or_default()
        .split_ascii_whitespace()
        .filter_map(|s| CellRange::parse(s).ok())
        .collect::<Vec<_>>();
    let Some(base) = original.first().map(|r| r.start) else {
        return Ok(vec![entry.into()]);
    };
    let mut result = Vec::new();
    for region in original {
        for piece in rule_pieces(region, change) {
            if result.len() >= CELL_SHIFT_LIMIT {
                return Err(failure("too many split rule regions"));
            }
            let entry =
                xml_edit::root_attributes(entry, &[("sqref", Some(range_text(piece.target)))])?;
            let formulas = |root: &str, names: &[&[u8]]| -> Result<String> {
                let mut root = root.to_string();
                for name in names {
                    root = children_map(&root, name, |f| {
                        let text = leaf_text(f)?;
                        text_content(f, &rule_formula(&text, base, piece, change))
                    })?;
                }
                Ok(root)
            };
            let entry = if conditional {
                children_map(&entry, b"cfRule", |r| formulas(r, &[b"formula"]))?
            } else {
                crate::validation_sources::map_formulas(&entry, false, &|f| {
                    rule_formula(f, base, piece, change)
                })?
            };
            result.push(entry);
        }
    }
    Ok(result)
}

pub(crate) fn aggregate_rule(rule: &str) -> Result<bool> {
    Ok(matches!(
        head_attr(rule, b"type")?.as_deref(),
        Some(
            "dataBar"
                | "colorScale"
                | "iconSet"
                | "top10"
                | "aboveAverage"
                | "uniqueValues"
                | "duplicateValues"
        )
    ))
}

fn move_aggregate(entry: &str, change: &CellShift) -> Result<Option<String>> {
    let targets = regions(entry, b"sqref", change)?;
    if targets.is_empty() {
        return Ok(None);
    }
    let moved = xml_edit::root_attributes(
        entry,
        &[(
            "sqref",
            Some(
                targets
                    .into_iter()
                    .map(range_text)
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
        )],
    )?;
    let moved = children_map(&moved, b"cfRule", |rule| {
        let mut rule = children_map(rule, b"formula", |f| {
            text_content(
                f,
                &shift_cells_formula(&leaf_text(f)?, &change.sheet, change),
            )
        })?;
        for name in [b"dataBar".as_slice(), b"colorScale", b"iconSet"] {
            rule = children_map(&rule, name, |group| {
                children_map(group, b"cfvo", |cfvo| {
                    if head_attr(cfvo, b"type")?.as_deref() != Some("formula") {
                        return Ok(cfvo.into());
                    }
                    let Some(f) = head_attr(cfvo, b"val")? else {
                        return Ok(cfvo.into());
                    };
                    xml_edit::root_attributes(
                        cfvo,
                        &[("val", Some(shift_cells_formula(&f, &change.sheet, change)))],
                    )
                })
            })?;
        }
        Ok(rule)
    })?;
    Ok(Some(moved))
}

fn move_rules(
    original: &str,
    current: &str,
    change: &CellShift,
    ids: &mut crate::cell_shift_extensions::IdPool,
) -> Result<String> {
    let mut cf = Vec::new();
    for (_, _, entry) in xml_edit_ns::children(original, b"conditionalFormatting")? {
        let rules = xml_edit_ns::children(&entry.text, b"cfRule")?;
        // Keep a homogeneous aggregate container intact. A mixed container
        // needs one wrapper per rule, since relative predicates can split
        // while min/max, ranks and frequency still use the entire population.
        if rules
            .iter()
            .map(|(_, _, r)| aggregate_rule(&r.text))
            .collect::<Result<Vec<_>>>()?
            .iter()
            .all(|x| *x)
        {
            if let Some(moved) = move_aggregate(&entry.text, change)? {
                cf.push(entry.finish(&moved)?);
            }
        } else {
            for (index, (_, _, rule)) in rules.iter().enumerate() {
                let mut i = 0;
                let one = children_map(&entry.text, b"cfRule", |r| {
                    let keep = i == index;
                    i += 1;
                    Ok(if keep { r.into() } else { String::new() })
                })?;
                if aggregate_rule(&rule.text)? {
                    if let Some(moved) = move_aggregate(&one, change)? {
                        cf.push(entry.finish(&moved)?);
                    }
                } else {
                    for fragment in move_rule(&one, change, true)? {
                        cf.push(entry.finish(&fragment)?);
                    }
                }
            }
        }
    }
    let spans = xml_edit_ns::children(current, b"conditionalFormatting")?;
    let mut changes = Vec::new();
    let body = cf.concat();
    for (i, (start, end, _)) in spans.into_iter().enumerate() {
        changes.push(Splice {
            start,
            end,
            text: if i == 0 { body.clone() } else { String::new() },
        });
    }
    let mut result = apply(current, changes);
    if let Some((_, _, container)) = xml_edit_ns::children(original, b"dataValidations")?
        .into_iter()
        .next()
    {
        let moved = children_map(&container.text, b"dataValidation", |rule| {
            Ok(move_rule(rule, change, false)?.concat())
        })?;
        let moved = count_children(&moved, b"dataValidation", "count")?;
        let moved = if moved.is_empty() {
            moved
        } else {
            container.finish(&moved)?
        };
        result = xml_edit_ns::set_child(
            &result,
            b"dataValidations",
            |_| moved,
            &[
                b"hyperlinks",
                b"printOptions",
                b"pageMargins",
                b"pageSetup",
                b"headerFooter",
                b"rowBreaks",
                b"colBreaks",
                b"drawing",
                b"legacyDrawing",
                b"tableParts",
                b"extLst",
            ],
        )?;
    }
    crate::cell_shift_extensions::move_extensions(original, &result, change, ids)
}

fn regions(xml: &str, key: &[u8], change: &CellShift) -> Result<Vec<CellRange>> {
    Ok(head_attr(xml, key)?
        .map(|s| {
            s.split_ascii_whitespace()
                .filter_map(|s| CellRange::parse(s).ok())
                .flat_map(|r| change.map_regions(r))
                .collect()
        })
        .unwrap_or_default())
}
fn count_children(xml: &str, child: &[u8], attribute: &str) -> Result<String> {
    let count = xml_edit_ns::children(xml, child)?.len();
    if count == 0 {
        Ok(String::new())
    } else {
        xml_edit::root_attributes(xml, &[(attribute, Some(count.to_string()))])
    }
}
fn move_metadata(xml: &str, change: &CellShift) -> Result<String> {
    let mut xml = child_edit(xml, b"dimension", |_| Ok(String::new()))?;
    xml = child_edit(&xml, b"mergeCells", |container| {
        let container = children_map(container, b"mergeCell", |entry| {
            let Some(old) = head_attr(entry, b"ref")?.and_then(|s| CellRange::parse(&s).ok())
            else {
                return Ok(entry.into());
            };
            match change.map_reference(old).filter(|r| r.start != r.end) {
                Some(r) => xml_edit::root_attributes(entry, &[("ref", Some(range_text(r)))]),
                None => Ok(String::new()),
            }
        })?;
        count_children(&container, b"mergeCell", "count")
    })?;
    xml = child_edit(&xml, b"hyperlinks", |container| {
        children_map(container, b"hyperlink", |entry| {
            let mut out = String::new();
            for range in regions(entry, b"ref", change)? {
                out.push_str(&xml_edit::root_attributes(
                    entry,
                    &[("ref", Some(range_text(range)))],
                )?);
            }
            Ok(out)
        })
    })?;
    xml = child_edit(&xml, b"dataValidations", |container| {
        let container = children_map(container, b"dataValidation", |entry| {
            let ranges = regions(entry, b"sqref", change)?;
            if ranges.is_empty() {
                return Ok(String::new());
            }
            xml_edit::root_attributes(
                entry,
                &[(
                    "sqref",
                    Some(
                        ranges
                            .into_iter()
                            .map(range_text)
                            .collect::<Vec<_>>()
                            .join(" "),
                    ),
                )],
            )
        })?;
        count_children(&container, b"dataValidation", "count")
    })?;
    xml = children_map(&xml, b"conditionalFormatting", |entry| {
        let ranges = regions(entry, b"sqref", change)?;
        if ranges.is_empty() {
            return Ok(String::new());
        }
        xml_edit::root_attributes(
            entry,
            &[(
                "sqref",
                Some(
                    ranges
                        .into_iter()
                        .map(range_text)
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
            )],
        )
    })?;
    xml = crate::protected_ranges::map_sheet(&xml, &|range| change.map_regions(range))?;
    xml = child_edit(&xml, b"autoFilter", |entry| {
        let Some(old) = head_attr(entry, b"ref")?.and_then(|s| CellRange::parse(&s).ok()) else {
            return Ok(entry.into());
        };
        if !change.covers(old) {
            return Ok(entry.into());
        }
        match change.map_reference(old) {
            Some(mapped) => crate::filter::shift_auto_filter(entry, &change.axis_change(), mapped),
            None => Ok(String::new()),
        }
    })?;
    // Array/shared-master rectangles live on the formula element, not the cell.
    xml = child_edit(&xml, b"sheetData", |data| {
        children_map(data, b"row", |row| {
            children_map(row, b"c", |cell| {
                children_map(cell, b"f", |f| {
                    let Some(old) = head_attr(f, b"ref")?.and_then(|s| CellRange::parse(&s).ok())
                    else {
                        return Ok(f.into());
                    };
                    xml_edit::root_attributes(
                        f,
                        &[("ref", change.map_reference(old).map(range_text))],
                    )
                })
            })
        })
    })?;
    Ok(xml)
}

fn follow_parts(package: &mut OpcPackage, sheet: &Sheet, change: &CellShift) -> Result<()> {
    let relpart = jade_opc::rels_part_name(&sheet.part_name);
    let Some(xml) = package.part_str(&relpart)? else {
        return Ok(());
    };
    let rels = jade_opc::parse_relationships(&xml)?;
    let mut headers = Vec::new();
    let mut removed = Vec::new();
    let mut shifted_tables = BTreeSet::new();
    let mut removed_tables = BTreeSet::new();
    for rel in rels.by_type_suffix("/table") {
        let part = jade_opc::resolve_target(&sheet.part_name, &rel.target);
        if rel.external {
            continue;
        }
        if removed_tables.contains(&part) {
            crate::shift::drop_part(package, &part, &relpart, &rel.id)?;
            removed.push(rel.id.clone());
            continue;
        }
        if !shifted_tables.insert(part.clone()) {
            continue;
        }
        let Some(table) = sheet.tables.iter().find(|t| t.part_name == part) else {
            continue;
        };
        if !change.affects(table.reference) {
            continue;
        }
        let xml = package.required_part_str(&part)?;
        let result = crate::table::shift_table_geometry(&xml, &sheet.name, &change.axis_change())?;
        headers.extend(result.header_cells);
        match result.xml {
            Some(xml) => package.replace(&part, xml.into_bytes()),
            None => {
                removed_tables.insert(part.clone());
                crate::shift::drop_part(package, &part, &relpart, &rel.id)?;
                removed.push(rel.id.clone());
            }
        }
    }
    if !removed.is_empty() || !headers.is_empty() {
        let xml = package.required_part_str(&sheet.part_name)?;
        let xml = crate::table::remove_table_parts(&xml, &removed)?;
        package.replace(
            &sheet.part_name,
            crate::patch_sheet(&xml, &headers)?.into_bytes(),
        );
    }
    for rel in rels.by_type_suffix("/comments") {
        let part = jade_opc::resolve_target(&sheet.part_name, &rel.target);
        let xml = package.required_part_str(&part)?;
        let xml = child_edit(&xml, b"commentList", |list| {
            children_map(list, b"comment", |note| {
                let Some(at) = head_attr(note, b"ref")?.and_then(|s| CellRef::parse(&s).ok())
                else {
                    return Ok(note.into());
                };
                match change.map_cell(at) {
                    Some(at) => {
                        xml_edit::root_attributes(note, &[("ref", Some(reference_text(at)))])
                    }
                    None => Ok(String::new()),
                }
            })
        })?;
        package.replace(&part, xml.into_bytes());
    }
    for rel in rels.by_type_suffix("/vmlDrawing") {
        let part = jade_opc::resolve_target(&sheet.part_name, &rel.target);
        let xml = package.required_part_str(&part)?;
        package.replace(&part, move_vml_notes(&xml, change)?.into_bytes());
    }
    for rel in rels.by_type_suffix("/drawing") {
        let part = jade_opc::resolve_target(&sheet.part_name, &rel.target);
        let xml = package.required_part_str(&part)?;
        package.replace(&part, move_drawing(&xml, change)?.into_bytes());
    }
    Ok(())
}

fn coordinate(xml: &str, name: &[u8]) -> Result<u32> {
    let Some((_, _, child)) = xml_edit_ns::children(xml, name)?.into_iter().next() else {
        return Err(failure("incomplete drawing anchor"));
    };
    let mut r = Reader::from_str(&child.text);
    match r.read_event()? {
        Event::Start(e) => r
            .read_text(e.name())?
            .trim()
            .parse()
            .map_err(|_| failure("invalid drawing coordinate")),
        _ => Err(failure("invalid drawing coordinate")),
    }
}
fn marker(xml: &str) -> Result<CellRef> {
    Ok(CellRef::new(
        coordinate(xml, b"col")?,
        coordinate(xml, b"row")?,
    ))
}
fn marker_clamped(change: &CellShift, at: CellRef) -> (CellRef, bool) {
    match change.map_cell(at) {
        Some(to) => (to, false),
        None => {
            let mut to = at;
            let position = if change.inserted {
                match change.axis {
                    Axis::Rows => jade_xlsx_reader::model::MAX_ROW_COUNT - 1,
                    Axis::Columns => jade_xlsx_reader::model::MAX_COLUMN_COUNT - 1,
                }
            } else {
                change.axis_change().at
            };
            match change.axis {
                Axis::Rows => to.row = position,
                Axis::Columns => to.column = position,
            }
            (to, !change.inserted)
        }
    }
}
fn move_drawing(xml: &str, change: &CellShift) -> Result<String> {
    let update = |anchor: &str| -> Result<String> {
        let edit = head_attr(anchor, b"editAs")?;
        if edit.as_deref() == Some("absolute") {
            return Ok(anchor.into());
        }
        let from = xml_edit_ns::children(anchor, b"from")?
            .into_iter()
            .next()
            .map(|(_, _, m)| marker(&m.text))
            .transpose()?;
        let to = xml_edit_ns::children(anchor, b"to")?
            .into_iter()
            .next()
            .map(|(_, _, m)| marker(&m.text))
            .transpose()?;
        let (Some(from), to) = (from, to) else {
            return Ok(anchor.into());
        };
        let (new_from, from_deleted) = marker_clamped(change, from);
        let to_deleted = to.is_some_and(|at| marker_clamped(change, at).1);
        if from_deleted && to_deleted {
            return Ok(String::new());
        }
        let mut result = anchor.to_string();
        for (name, point) in [(b"from".as_slice(), Some(from)), (b"to".as_slice(), to)] {
            let Some(point) = point else {
                continue;
            };
            let (moved, deleted) = if name == b"to" && edit.as_deref() == Some("oneCell") {
                let limit = match change.axis {
                    Axis::Rows => jade_xlsx_reader::model::MAX_ROW_COUNT,
                    Axis::Columns => jade_xlsx_reader::model::MAX_COLUMN_COUNT,
                };
                let value = (i64::from(change.along(point)) + i64::from(change.along(new_from))
                    - i64::from(change.along(from)))
                .clamp(0, i64::from(limit - 1)) as u32;
                let mut mapped = point;
                match change.axis {
                    Axis::Rows => mapped.row = value,
                    Axis::Columns => mapped.column = value,
                }
                (mapped, false)
            } else {
                marker_clamped(change, point)
            };
            if moved == point && !deleted {
                continue;
            }
            result = child_edit(&result, name, |m| {
                let field = match change.axis {
                    Axis::Rows => b"row".as_slice(),
                    Axis::Columns => b"col".as_slice(),
                };
                let mut m = child_edit(m, field, |f| {
                    text_content(f, &change.along(moved).to_string())
                })?;
                if deleted {
                    let offset = match change.axis {
                        Axis::Rows => b"rowOff".as_slice(),
                        Axis::Columns => b"colOff".as_slice(),
                    };
                    m = child_edit(&m, offset, |f| text_content(f, "0"))?;
                }
                Ok(m)
            })?;
        }
        Ok(result)
    };
    let xml = children_map(xml, b"twoCellAnchor", update)?;
    children_map(&xml, b"oneCellAnchor", update)
}

fn move_vml_notes(xml: &str, change: &CellShift) -> Result<String> {
    let mut reader = NsReader::from_str(xml);
    reader.config_mut().check_end_names = false;
    let mut edits = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref e) if e.local_name().as_ref() == b"shape" => {
                let valid = matches!(reader.resolver().resolve_element(e.name()).0,ResolveResult::Bound(ns) if ns.as_ref()==b"urn:schemas-microsoft-com:vml");
                if !valid {
                    continue;
                }
                let name = e.name().as_ref().to_vec();
                reader.read_to_end(quick_xml::name::QName(&name))?;
                let end = reader.buffer_position() as usize;
                let raw = &xml[start..end];
                let mut r = Reader::from_str(raw);
                let mut row = None;
                let mut col = None;
                let mut note = false;
                loop {
                    match r.read_event()? {
                        Event::Start(e) if local(e.name().as_ref()) == b"ClientData" => {
                            note = attribute(&e, b"ObjectType")?.as_deref() == Some("Note");
                        }
                        Event::Start(e)
                            if note && matches!(local(e.name().as_ref()), b"Row" | b"Column") =>
                        {
                            let column = local(e.name().as_ref()) == b"Column";
                            let value = r.read_text(e.name())?.parse::<u32>().ok();
                            if column {
                                col = value;
                            } else {
                                row = value;
                            }
                        }
                        Event::Eof => break,
                        _ => {}
                    }
                }
                if let (Some(column), Some(row)) = (col, row) {
                    let at = CellRef::new(column, row);
                    if change.map_cell(at) != Some(at) {
                        edits.push(Splice {
                            start,
                            end,
                            text: crate::drawing::shift_vml(raw, &change.axis_change()),
                        });
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(apply(xml, edits))
}
