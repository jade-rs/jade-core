//! `xl/worksheets/sheetN.xml` — the grid itself: column widths, rows, cells and
//! merged ranges.

use std::collections::HashMap;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use crate::error::Result;
use crate::model::{
    ArrayFormula, Cell, CellRange, CellRef, CellValue, ColumnSpan, FreezePane, HeaderFooter,
    Hyperlink, PageBreak, PageMargins, PageOrientation, PageSetup, Row, Sheet, SheetVisibility,
};
use jade_opc::{local_name, Relationships};

/// What a cell's `t` attribute says its `<v>` holds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CellType {
    Number,
    SharedString,
    InlineString,
    /// A formula that produced text; `<v>` is the text itself.
    FormulaString,
    Bool,
    Error,
    Date,
}

impl CellType {
    fn parse(value: &str) -> Self {
        match value {
            "s" => CellType::SharedString,
            "inlineStr" => CellType::InlineString,
            "str" => CellType::FormulaString,
            "b" => CellType::Bool,
            "e" => CellType::Error,
            "d" => CellType::Date,
            _ => CellType::Number,
        }
    }
}

/// Parses one worksheet part.
///
/// `shared_strings` is the pool from `xl/sharedStrings.xml`; an index outside it
/// yields empty text rather than an error, because a truncated pool should not
/// make the rest of the sheet unreadable.
pub fn parse_sheet(
    name: &str,
    part_name: &str,
    xml: &str,
    shared_strings: &[String],
) -> Result<Sheet> {
    let relationships = Relationships::default();
    parse_sheet_with_relationships(name, part_name, xml, shared_strings, &relationships)
}

/// Parses a worksheet together with its part relationships, allowing external
/// hyperlinks to resolve without changing the original `parse_sheet` API.
pub fn parse_sheet_with_relationships(
    name: &str,
    part_name: &str,
    xml: &str,
    shared_strings: &[String],
    relationships: &Relationships,
) -> Result<Sheet> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let (auto_filter, filter_mode) = crate::auto_filter::parse(xml, b"worksheet")?;
    let mut sheet = Sheet {
        auto_filter,
        filter_mode,
        filter_row_state: None,
        name: name.to_string(),
        part_name: part_name.to_string(),
        visibility: SheetVisibility::Visible,
        columns: Vec::new(),
        rows: Vec::new(),
        merges: Vec::new(),
        hyperlinks: Vec::new(),
        comments: Vec::new(),
        data_validations: Vec::new(),
        conditional_formats: crate::conditional_format::parse_conditional_formats(xml)?,
        default_column_width_chars: None,
        default_row_height_points: None,
        default_row_custom_height: false,
        default_rows_hidden: false,
        show_grid_lines: true,
        freeze_pane: None,
        tables: Vec::new(),
        header_footer: None,
        row_breaks: Vec::new(),
        column_breaks: Vec::new(),
        page_setup: Default::default(),
        shapes: Vec::new(),
        array_formulas: Vec::new(),
        phonetic_readings: Default::default(),
        rich_text: Default::default(),
        tab_color: None,
        sparkline_groups: crate::sparkline::parse_sparklines(xml)?,
        protection: None,
    };

    let mut row: Option<Row> = None;
    let mut cell: Option<Cell> = None;
    let mut cell_type = CellType::Number;
    let mut raw_value: Option<String> = None;
    let mut in_value = false;
    let mut in_formula = false;
    let mut in_inline_text = false;
    let mut in_phonetic_guide = false;
    let mut next_row_index = 0u32;
    let mut formula = String::new();
    let mut shared_formula_index: Option<u32> = None;
    let mut shared_formula_cells: HashMap<CellRef, u32> = HashMap::new();
    let mut shared_formula_masters: HashMap<u32, (CellRef, String)> = HashMap::new();
    let mut inline_text = String::new();
    // Which `<headerFooter>` child is being read, and its text so far.
    let mut header_footer_slot: Option<&'static str> = None;
    let mut header_footer_text = String::new();
    let mut in_row_breaks = false;
    let mut in_column_breaks = false;
    // A custom view keeps page settings of its own; the sheet's are outside.
    let mut in_custom_views = false;
    let mut buf = Vec::new();

    loop {
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                match local_name(element.name().as_ref()) {
                    b"headerFooter" => {
                        sheet.header_footer = Some(parse_header_footer(element)?);
                    }
                    b"oddHeader" | b"oddFooter" | b"evenHeader" | b"evenFooter"
                    | b"firstHeader" | b"firstFooter"
                        if !empty && sheet.header_footer.is_some() =>
                    {
                        header_footer_slot = Some(match local_name(element.name().as_ref()) {
                            b"oddHeader" => "oddHeader",
                            b"oddFooter" => "oddFooter",
                            b"evenHeader" => "evenHeader",
                            b"evenFooter" => "evenFooter",
                            b"firstHeader" => "firstHeader",
                            _ => "firstFooter",
                        });
                        header_footer_text.clear();
                    }
                    b"rowBreaks" => in_row_breaks = !empty,
                    b"colBreaks" => in_column_breaks = !empty,
                    b"customSheetViews" => in_custom_views = !empty,
                    b"pageSetUpPr" if !in_custom_views => {
                        sheet.page_setup.fit_to_page = attribute_value(element, b"fitToPage")?
                            .is_some_and(|value| value != "0" && value != "false");
                    }
                    b"pageMargins" if !in_custom_views => {
                        sheet.page_setup.margins = Some(parse_page_margins(element)?);
                    }
                    b"pageSetup" if !in_custom_views => {
                        parse_page_setup(element, &mut sheet.page_setup)?;
                    }
                    b"brk" if in_row_breaks || in_column_breaks => {
                        if let Some(brk) = parse_break(element)? {
                            if in_row_breaks {
                                sheet.row_breaks.push(brk);
                            } else {
                                sheet.column_breaks.push(brk);
                            }
                        }
                    }
                    b"sheetProtection" => {
                        sheet.protection = parse_sheet_protection(element)?;
                    }
                    b"tabColor" => {
                        sheet.tab_color = crate::styles::parse_color(element)?;
                    }
                    b"sheetView" => {
                        if let Some(value) = attribute_value(element, b"showGridLines")? {
                            sheet.show_grid_lines = value != "0" && value != "false";
                        }
                    }
                    b"pane" if sheet.freeze_pane.is_none() => {
                        sheet.freeze_pane = parse_freeze_pane(element)?;
                    }
                    b"sheetFormatPr" => {
                        sheet.default_column_width_chars =
                            attribute_value(element, b"defaultColWidth")?
                                .and_then(|value| value.parse().ok());
                        sheet.default_row_height_points =
                            attribute_value(element, b"defaultRowHeight")?
                                .and_then(|value| value.parse().ok());
                        sheet.default_row_custom_height =
                            attribute_value(element, b"customHeight")?
                                .is_some_and(|value| value != "0" && value != "false");
                        sheet.default_rows_hidden = attribute_value(element, b"zeroHeight")?
                            .is_some_and(|value| value != "0" && value != "false");
                    }
                    b"col" => {
                        if let Some(span) = parse_column(element)? {
                            sheet.columns.push(span);
                        }
                    }
                    b"row" => {
                        let parsed = parse_row(element, next_row_index, &sheet)?;
                        next_row_index = parsed.index.saturating_add(1);
                        if empty {
                            sheet.rows.push(parsed);
                        } else {
                            row = Some(parsed);
                        }
                    }
                    b"c" => {
                        let reference = match attribute_value(element, b"r")? {
                            Some(value) => CellRef::parse(&value)?,
                            // A cell may omit `r`, in which case it sits in the
                            // next column of the current row.
                            None => {
                                let column = row
                                    .as_ref()
                                    .and_then(|row| row.cells.last())
                                    .map(|cell| cell.reference.column + 1)
                                    .unwrap_or(0);
                                let row_index = row.as_ref().map(|row| row.index).unwrap_or(0);
                                CellRef::new(column, row_index)
                            }
                        };
                        cell_type = attribute_value(element, b"t")?
                            .map(|value| CellType::parse(&value))
                            .unwrap_or(CellType::Number);
                        raw_value = None;
                        formula.clear();
                        shared_formula_index = None;
                        inline_text.clear();
                        let parsed = Cell {
                            reference,
                            value: CellValue::Empty,
                            style_index: attribute_value(element, b"s")?
                                .and_then(|value| value.parse().ok()),
                            formula: None,
                        };
                        if empty {
                            if let Some(row) = row.as_mut() {
                                row.cells.push(parsed);
                            }
                        } else {
                            cell = Some(parsed);
                        }
                    }
                    b"v" if !empty => {
                        in_value = true;
                        raw_value = Some(String::new());
                    }
                    b"f" => {
                        let kind = attribute_value(element, b"t")?;
                        if kind.as_deref() == Some("array") {
                            if let (Some(anchor), Some(range)) = (
                                cell.as_ref().map(|cell| cell.reference),
                                attribute_value(element, b"ref")?
                                    .and_then(|range| CellRange::parse(&range).ok()),
                            ) {
                                sheet.array_formulas.push(ArrayFormula { anchor, range });
                            }
                        }
                        if kind.as_deref() == Some("shared") {
                            shared_formula_index = attribute_value(element, b"si")?
                                .and_then(|value| value.parse().ok());
                        }
                        if !empty {
                            in_formula = true;
                        }
                    }
                    b"rPh" if !empty && cell.is_some() => in_phonetic_guide = true,
                    b"t" if !empty && cell.is_some() && !in_phonetic_guide => in_inline_text = true,
                    b"mergeCell" => {
                        if let Some(value) = attribute_value(element, b"ref")? {
                            sheet.merges.push(CellRange::parse(&value)?);
                        }
                    }
                    b"hyperlink" => {
                        if let Some(link) = parse_hyperlink(element, relationships)? {
                            sheet.hyperlinks.push(link);
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(ref text) => {
                let decoded = text.decode()?;
                if in_value {
                    if let Some(value) = raw_value.as_mut() {
                        value.push_str(&decoded);
                    }
                } else if in_formula {
                    formula.push_str(&decoded);
                } else if in_inline_text {
                    inline_text.push_str(&decoded);
                } else if header_footer_slot.is_some() {
                    header_footer_text.push_str(&decoded);
                }
            }
            Event::CData(ref text) => {
                let decoded = text.decode()?;
                if in_value {
                    if let Some(value) = raw_value.as_mut() {
                        value.push_str(&decoded);
                    }
                } else if in_formula {
                    formula.push_str(&decoded);
                } else if in_inline_text {
                    inline_text.push_str(&decoded);
                } else if header_footer_slot.is_some() {
                    header_footer_text.push_str(&decoded);
                }
            }
            Event::GeneralRef(ref reference) => {
                let Some(decoded) = decoded_reference(reference) else {
                    continue;
                };
                if in_value {
                    if let Some(value) = raw_value.as_mut() {
                        value.push_str(&decoded);
                    }
                } else if in_formula {
                    formula.push_str(&decoded);
                } else if in_inline_text {
                    inline_text.push_str(&decoded);
                } else if header_footer_slot.is_some() {
                    header_footer_text.push_str(&decoded);
                }
            }
            Event::End(ref element) => match local_name(element.name().as_ref()) {
                b"v" => in_value = false,
                b"f" => in_formula = false,
                b"t" => in_inline_text = false,
                b"rPh" => in_phonetic_guide = false,
                b"rowBreaks" => in_row_breaks = false,
                b"colBreaks" => in_column_breaks = false,
                b"customSheetViews" => in_custom_views = false,
                b"oddHeader" | b"oddFooter" | b"evenHeader" | b"evenFooter" | b"firstHeader"
                | b"firstFooter" => {
                    if let (Some(slot), Some(header_footer)) =
                        (header_footer_slot.take(), sheet.header_footer.as_mut())
                    {
                        let text = Some(header_footer_text.clone());
                        match slot {
                            "oddHeader" => header_footer.odd_header = text,
                            "oddFooter" => header_footer.odd_footer = text,
                            "evenHeader" => header_footer.even_header = text,
                            "evenFooter" => header_footer.even_footer = text,
                            "firstHeader" => header_footer.first_header = text,
                            _ => header_footer.first_footer = text,
                        }
                    }
                }
                b"c" => {
                    if let Some(mut parsed) = cell.take() {
                        parsed.value = resolve_value(
                            cell_type,
                            raw_value.as_deref(),
                            &inline_text,
                            shared_strings,
                        );
                        if !formula.is_empty() {
                            parsed.formula = Some(formula.clone());
                        }
                        if let Some(index) = shared_formula_index {
                            shared_formula_cells.insert(parsed.reference, index);
                            if !formula.is_empty() {
                                shared_formula_masters
                                    .insert(index, (parsed.reference, formula.clone()));
                            }
                        }
                        if let Some(row) = row.as_mut() {
                            row.cells.push(parsed);
                        }
                    }
                }
                b"row" => {
                    if let Some(row) = row.take() {
                        sheet.rows.push(row);
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    sheet.rows.sort_by_key(|row| row.index);
    for row in &mut sheet.rows {
        row.cells.sort_by_key(|cell| cell.reference.column);
        for cell in &mut row.cells {
            if cell.formula.is_some() {
                continue;
            }
            let Some(index) = shared_formula_cells.get(&cell.reference) else {
                continue;
            };
            let Some((origin, formula)) = shared_formula_masters.get(index) else {
                continue;
            };
            cell.formula = Some(translate_formula(formula, *origin, cell.reference));
        }
    }
    Ok(sheet)
}

fn parse_hyperlink(
    element: &BytesStart<'_>,
    relationships: &Relationships,
) -> Result<Option<Hyperlink>> {
    let Some(reference) = attribute_value(element, b"ref")? else {
        return Ok(None);
    };
    let relationship_id = attribute_value(element, b"id")?;
    let target = relationship_id.and_then(|id| {
        relationships.relationship(&id).and_then(|relationship| {
            relationship
                .relationship_type
                .ends_with("/hyperlink")
                .then(|| relationship.target.clone())
        })
    });

    Ok(Some(Hyperlink {
        reference: CellRange::parse(&reference)?,
        target,
        location: attribute_value(element, b"location")?,
        display: attribute_value(element, b"display")?,
        tooltip: attribute_value(element, b"tooltip")?,
    }))
}

fn parse_header_footer(element: &BytesStart<'_>) -> Result<HeaderFooter> {
    let flag = |key: &[u8], default: bool| -> Result<bool> {
        Ok(match attribute_value(element, key)? {
            Some(value) => value != "0" && value != "false",
            None => default,
        })
    };
    Ok(HeaderFooter {
        odd_header: None,
        odd_footer: None,
        even_header: None,
        even_footer: None,
        first_header: None,
        first_footer: None,
        different_odd_even: flag(b"differentOddEven", false)?,
        different_first: flag(b"differentFirst", false)?,
        scale_with_doc: flag(b"scaleWithDoc", true)?,
        align_with_margins: flag(b"alignWithMargins", true)?,
    })
}

/// `<pageSetup>`'s paper, orientation, scale, fit and page order into
/// `setup`.
fn parse_page_setup(element: &BytesStart<'_>, setup: &mut PageSetup) -> Result<()> {
    let number = |key: &[u8]| -> Result<Option<u32>> {
        Ok(attribute_value(element, key)?.and_then(|value| value.parse().ok()))
    };
    setup.paper_size = number(b"paperSize")?;
    setup.scale = number(b"scale")?;
    setup.fit_to_width = number(b"fitToWidth")?;
    setup.fit_to_height = number(b"fitToHeight")?;
    setup.orientation = match attribute_value(element, b"orientation")?.as_deref() {
        Some("landscape") => Some(PageOrientation::Landscape),
        Some("portrait") => Some(PageOrientation::Portrait),
        _ => None,
    };
    setup.over_then_down =
        attribute_value(element, b"pageOrder")?.as_deref() == Some("overThenDown");
    Ok(())
}

/// `<pageMargins>`, each side Excel's Normal one where it is missing.
fn parse_page_margins(element: &BytesStart<'_>) -> Result<PageMargins> {
    let normal = PageMargins::default();
    let inches = |key: &[u8], default: f64| -> Result<f64> {
        Ok(attribute_value(element, key)?
            .and_then(|value| value.parse().ok())
            .unwrap_or(default))
    };
    Ok(PageMargins {
        left: inches(b"left", normal.left)?,
        right: inches(b"right", normal.right)?,
        top: inches(b"top", normal.top)?,
        bottom: inches(b"bottom", normal.bottom)?,
        header: inches(b"header", normal.header)?,
        footer: inches(b"footer", normal.footer)?,
    })
}

fn parse_break(element: &BytesStart<'_>) -> Result<Option<PageBreak>> {
    let Some(before) = attribute_value(element, b"id")?.and_then(|value| value.parse::<u32>().ok())
    else {
        return Ok(None);
    };
    let manual =
        attribute_value(element, b"man")?.is_some_and(|value| value != "0" && value != "false");
    Ok(Some(PageBreak { before, manual }))
}

fn parse_freeze_pane(element: &BytesStart<'_>) -> Result<Option<FreezePane>> {
    let state = attribute_value(element, b"state")?;
    if !matches!(state.as_deref(), Some("frozen" | "frozenSplit")) {
        return Ok(None);
    }

    let frozen_columns = attribute_value(element, b"xSplit")?
        .as_deref()
        .and_then(parse_frozen_count)
        .unwrap_or(0);
    let frozen_rows = attribute_value(element, b"ySplit")?
        .as_deref()
        .and_then(parse_frozen_count)
        .unwrap_or(0);
    let top_left_cell =
        attribute_value(element, b"topLeftCell")?.and_then(|value| CellRef::parse(&value).ok());

    Ok(Some(FreezePane {
        frozen_columns,
        frozen_rows,
        top_left_cell,
    }))
}

fn parse_frozen_count(value: &str) -> Option<u32> {
    let count = value.parse::<f64>().ok()?;
    (count.is_finite() && count >= 0.0 && count.fract() == 0.0 && count <= f64::from(u32::MAX))
        .then_some(count as u32)
}

/// Moves the relative A1 references in a formula from `origin` to `target`.
/// Absolute axes, string literals, and quoted sheet names stay put; a
/// reference pushed off the grid becomes `#REF!`.
///
/// These are the movement rules shared formulas follow, and they are exactly
/// what pasting a copied formula needs — which is why the clipboard uses this
/// rather than a second implementation that would drift.
pub fn translate_formula(formula: &str, origin: CellRef, target: CellRef) -> String {
    let column_delta = i64::from(target.column) - i64::from(origin.column);
    let row_delta = i64::from(target.row) - i64::from(origin.row);
    map_references(formula, |reference| {
        let column = if reference.column_absolute {
            Some(reference.column)
        } else {
            shift_axis(reference.column, column_delta)
        };
        let row = if reference.row_absolute {
            Some(reference.row)
        } else {
            shift_axis(reference.row, row_delta)
        };
        column.zip(row)
    })
}

/// Moves a formula as Paste Special's Transpose does, copied from `origin`
/// and pasted at `target`, the cell `origin` lands on in the transposed
/// range. A relative reference's offset from the formula's cell turns with
/// the range — what was one column to the left is one row up — so
/// references inside the range keep to the same cells, transposed. An
/// absolute axis stays; a reference with one moves as an ordinary paste
/// moves it.
pub fn transpose_formula(formula: &str, origin: CellRef, target: CellRef) -> String {
    let column_delta = i64::from(target.column) - i64::from(origin.column);
    let row_delta = i64::from(target.row) - i64::from(origin.row);
    map_references(formula, |reference| {
        if reference.column_absolute || reference.row_absolute {
            let column = if reference.column_absolute {
                Some(reference.column)
            } else {
                shift_axis(reference.column, column_delta)
            };
            let row = if reference.row_absolute {
                Some(reference.row)
            } else {
                shift_axis(reference.row, row_delta)
            };
            return column.zip(row);
        }
        let across = i64::from(reference.column) - i64::from(origin.column);
        let down = i64::from(reference.row) - i64::from(origin.row);
        shift_axis(target.column, down).zip(shift_axis(target.row, across))
    })
}

/// An A1 reference found in a formula: where it points, and which of its
/// axes carry a `$`.
struct FoundReference {
    column: u32,
    row: u32,
    column_absolute: bool,
    row_absolute: bool,
}

/// `formula` with each A1 reference outside strings and quoted sheet names
/// moved where `place` says (column, row); `None`, or a place off the grid,
/// becomes `#REF!`.
fn map_references(formula: &str, place: impl Fn(&FoundReference) -> Option<(u32, u32)>) -> String {
    let bytes = formula.as_bytes();
    let mut output = String::with_capacity(formula.len());
    let mut index = 0usize;
    let mut in_string = false;
    let mut in_sheet_name = false;

    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if byte == b'"' {
                output.push('"');
                index += 1;
                if bytes.get(index) == Some(&b'"') {
                    output.push('"');
                    index += 1;
                } else {
                    in_string = false;
                }
            } else {
                let character = formula[index..].chars().next().expect("formula character");
                output.push(character);
                index += character.len_utf8();
            }
            continue;
        }
        if in_sheet_name {
            if byte == b'\'' {
                output.push('\'');
                index += 1;
                if bytes.get(index) == Some(&b'\'') {
                    output.push('\'');
                    index += 1;
                } else {
                    in_sheet_name = false;
                }
            } else {
                let character = formula[index..].chars().next().expect("formula character");
                output.push(character);
                index += character.len_utf8();
            }
            continue;
        }
        if byte == b'"' {
            output.push('"');
            index += 1;
            in_string = true;
            continue;
        }
        if byte == b'\'' {
            output.push('\'');
            index += 1;
            in_sheet_name = true;
            continue;
        }
        if let Some((end, translated)) = translated_reference(formula, index, &place) {
            output.push_str(&translated);
            index = end;
            continue;
        }

        let character = formula[index..]
            .chars()
            .next()
            .expect("index is inside formula");
        output.push(character);
        index += character.len_utf8();
    }
    output
}

fn translated_reference(
    formula: &str,
    start: usize,
    place: &impl Fn(&FoundReference) -> Option<(u32, u32)>,
) -> Option<(usize, String)> {
    let bytes = formula.as_bytes();
    let previous = start
        .checked_sub(1)
        .and_then(|index| bytes.get(index))
        .copied();
    if previous.is_some_and(is_name_byte) || previous == Some(b'[') {
        return None;
    }

    let mut index = start;
    let column_absolute = bytes.get(index) == Some(&b'$');
    index += usize::from(column_absolute);
    let column_start = index;
    while bytes.get(index).is_some_and(u8::is_ascii_alphabetic) {
        index += 1;
    }
    if index == column_start {
        return None;
    }

    let row_absolute = bytes.get(index) == Some(&b'$');
    index += usize::from(row_absolute);
    let row_start = index;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    if index == row_start {
        return None;
    }

    let following = bytes.get(index).copied();
    if following.is_some_and(is_name_byte)
        || matches!(following, Some(b'!') | Some(b'[') | Some(b']') | Some(b'('))
    {
        return None;
    }

    let column = column_number(&formula[column_start..row_start - usize::from(row_absolute)])?;
    let row = formula[row_start..index]
        .parse::<u32>()
        .ok()?
        .checked_sub(1)?;
    // XFD1048576 is the edge of an OOXML worksheet. Tokens outside it are
    // names or function identifiers, not cell references.
    if column > 16_383 || row > 1_048_575 {
        return None;
    }
    let Some((column, row)) = place(&FoundReference {
        column,
        row,
        column_absolute,
        row_absolute,
    }) else {
        return Some((index, "#REF!".to_string()));
    };
    if column > 16_383 || row > 1_048_575 {
        return Some((index, "#REF!".to_string()));
    }

    let mut translated = String::new();
    if column_absolute {
        translated.push('$');
    }
    translated.push_str(&column_label(column));
    if row_absolute {
        translated.push('$');
    }
    translated.push_str(&(row + 1).to_string());
    Some((index, translated))
}

fn is_name_byte(byte: u8) -> bool {
    byte >= 0x80 || byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.')
}

fn column_number(letters: &str) -> Option<u32> {
    let mut column = 0u32;
    for byte in letters.bytes() {
        let digit = u32::from(byte.to_ascii_uppercase().checked_sub(b'A')?) + 1;
        column = column.checked_mul(26)?.checked_add(digit)?;
    }
    column.checked_sub(1)
}

fn column_label(index: u32) -> String {
    let mut label = Vec::new();
    let mut remaining = u64::from(index) + 1;
    while remaining > 0 {
        label.push(b'A' + ((remaining - 1) % 26) as u8);
        remaining = (remaining - 1) / 26;
    }
    label.reverse();
    String::from_utf8(label).expect("ASCII column label")
}

fn shift_axis(axis: u32, delta: i64) -> Option<u32> {
    u32::try_from(i64::from(axis) + delta).ok()
}

fn resolve_value(
    cell_type: CellType,
    raw: Option<&str>,
    inline_text: &str,
    shared_strings: &[String],
) -> CellValue {
    match cell_type {
        CellType::InlineString => {
            if inline_text.is_empty() {
                CellValue::Empty
            } else {
                CellValue::Text(crate::xstring::decode_xstring(inline_text))
            }
        }
        CellType::SharedString => match raw.and_then(|value| value.trim().parse::<usize>().ok()) {
            Some(index) => CellValue::Text(shared_strings.get(index).cloned().unwrap_or_default()),
            None => CellValue::Empty,
        },
        CellType::FormulaString => match raw {
            Some(value) => CellValue::Text(crate::xstring::decode_xstring(value)),
            None => CellValue::Empty,
        },
        CellType::Date => match raw {
            Some(value) => CellValue::Text(value.to_string()),
            None => CellValue::Empty,
        },
        CellType::Bool => match raw.map(str::trim) {
            Some("1" | "true") => CellValue::Bool(true),
            Some("0" | "false") => CellValue::Bool(false),
            _ => CellValue::Empty,
        },
        CellType::Error => match raw {
            Some(value) => CellValue::Error(value.to_string()),
            None => CellValue::Empty,
        },
        CellType::Number => match raw.and_then(|value| value.trim().parse::<f64>().ok()) {
            Some(number) => CellValue::Number(number),
            None => CellValue::Empty,
        },
    }
}

fn parse_column(element: &BytesStart<'_>) -> Result<Option<ColumnSpan>> {
    let mut first = None;
    let mut last = None;
    let mut width_chars = None;
    let mut custom_width = false;
    let mut hidden = false;
    let mut style_index = None;
    for attribute in element.attributes() {
        let attribute = attribute?;
        let value = attribute.unescape_value()?;
        match local_name(attribute.key.as_ref()) {
            b"min" => {
                first = value
                    .parse::<u32>()
                    .ok()
                    .map(|index| index.saturating_sub(1))
            }
            b"max" => {
                last = value
                    .parse::<u32>()
                    .ok()
                    .map(|index| index.saturating_sub(1))
            }
            b"width" => width_chars = value.parse().ok(),
            b"customWidth" => custom_width = value != "0" && value != "false",
            b"hidden" => hidden = value != "0" && value != "false",
            b"style" => style_index = value.parse().ok(),
            _ => {}
        }
    }
    let (Some(first), Some(last)) = (first, last) else {
        return Ok(None);
    };
    Ok(Some(ColumnSpan {
        first,
        last,
        width_chars,
        custom_width,
        hidden,
        style_index,
    }))
}

fn parse_row(element: &BytesStart<'_>, fallback_index: u32, sheet: &Sheet) -> Result<Row> {
    let mut index = None;
    let mut height_points = None;
    let mut custom_height = sheet.default_row_custom_height;
    let mut custom_format = false;
    let mut hidden = sheet.default_rows_hidden;
    let mut style_index = None;
    for attribute in element.attributes() {
        let attribute = attribute?;
        let value = attribute.unescape_value()?;
        match local_name(attribute.key.as_ref()) {
            b"r" => {
                index = value
                    .parse::<u32>()
                    .ok()
                    .map(|index| index.saturating_sub(1))
            }
            b"ht" => height_points = value.parse().ok(),
            b"customHeight" => custom_height = value != "0" && value != "false",
            b"customFormat" => custom_format = value != "0" && value != "false",
            b"hidden" => hidden = value != "0" && value != "false",
            b"s" => style_index = value.parse().ok(),
            _ => {}
        }
    }
    Ok(Row {
        index: index.unwrap_or(fallback_index),
        height_points,
        custom_height,
        custom_format,
        hidden,
        style_index,
        cells: Vec::new(),
    })
}

/// A `sheetProtection` that protects the sheet: what it still allows. Each
/// attribute names an operation and is `true` (the default) when the
/// protection forbids it, so an operation is allowed only where the file
/// says `0` (`objects` and `scenarios` default the other way).
fn parse_sheet_protection(
    element: &BytesStart<'_>,
) -> Result<Option<crate::model::SheetProtection>> {
    let flag = |key: &[u8], default: bool| -> Result<bool> {
        Ok(attribute_value(element, key)?
            .map(|value| value == "1" || value == "true")
            .unwrap_or(default))
    };
    if !flag(b"sheet", false)? {
        return Ok(None);
    }
    Ok(Some(crate::model::SheetProtection {
        format_cells: !flag(b"formatCells", true)?,
        format_columns: !flag(b"formatColumns", true)?,
        format_rows: !flag(b"formatRows", true)?,
        insert_columns: !flag(b"insertColumns", true)?,
        insert_rows: !flag(b"insertRows", true)?,
        insert_hyperlinks: !flag(b"insertHyperlinks", true)?,
        delete_columns: !flag(b"deleteColumns", true)?,
        delete_rows: !flag(b"deleteRows", true)?,
        sort: !flag(b"sort", true)?,
        auto_filter: !flag(b"autoFilter", true)?,
        objects: !flag(b"objects", false)?,
    }))
}

fn attribute_value(element: &BytesStart<'_>, key: &[u8]) -> Result<Option<String>> {
    for attribute in element.attributes() {
        let attribute = attribute?;
        if local_name(attribute.key.as_ref()) == key {
            return Ok(Some(attribute.unescape_value()?.into_owned()));
        }
    }
    Ok(None)
}

/// The text an entity reference stands for.
///
/// `quick-xml` reports `&amp;` as its own event rather than as text, so a
/// parser that only handles `Event::Text` silently drops every `&`, `<` and
/// `>` a cell contains.
fn decoded_reference(reference: &quick_xml::events::BytesRef<'_>) -> Option<String> {
    let name = reference.decode().ok()?;
    quick_xml::escape::unescape(&format!("&{name};"))
        .ok()
        .map(|text| text.into_owned())
}

#[cfg(test)]
mod tests {
    use super::translate_formula as translate_shared_formula;
    use crate::CellRef;

    #[test]
    fn shared_formula_translation_respects_formula_tokens() {
        let translated = translate_shared_formula(
            r#""A1"+'売 上'!A1+LOG10(A1)+$A1+A$1+$A$1+日本語A1"#,
            CellRef::new(0, 0),
            CellRef::new(1, 1),
        );
        assert_eq!(
            translated,
            r#""A1"+'売 上'!B2+LOG10(B2)+$A2+B$1+$A$1+日本語A1"#
        );
    }

    #[test]
    fn a_shared_reference_shifted_past_the_grid_becomes_ref_error() {
        assert_eq!(
            translate_shared_formula("A1", CellRef::new(1, 1), CellRef::new(0, 0)),
            "#REF!"
        );
        assert_eq!(
            translate_shared_formula("XFD1048576", CellRef::new(0, 0), CellRef::new(1, 1)),
            "#REF!"
        );
    }
}
