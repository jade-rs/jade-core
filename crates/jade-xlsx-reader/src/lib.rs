//! Reads `.xlsx` into a model that keeps what the file said.
//!
//! The parse is our own rather than a spreadsheet library's for the same reason
//! the DOCX side owns its parse: jade edits the original file and writes it
//! back, so a reader that keeps only values and drops style indices, merges and
//! formulas would make round-tripping impossible from the start.
//!
//! What is read today: the sheet list, shared strings, the font/fill/border and
//! `cellXfs` tables, column widths, row heights, cells (values, style index, and
//! the formula with the value Excel cached for it), merged ranges, and global
//! or sheet-scoped defined names, sheet visibility, saved active tab,
//! recalculation-on-load state, frozen-pane metadata, and
//! external or workbook-internal cell hyperlinks, legacy cell comments, and
//! standard data-validation rules.
//!
//! Also read: Excel tables (range, header and totals rows, column names),
//! the page header and footer, page breaks, and the sheet's drawing — the
//! shapes, text boxes, pictures and charts by anchor and text.
//!
//! Standard conditional rule ranges, priorities and differential styles are
//! retained for shared layout and compatibility reporting.
//!
//! Classic chart series, caches and references are retained for shared layout.
//! What is not read yet: extended charts, extension-based data validation, pivot tables, and
//! any recalculation of formulas —
//! rendering uses the cached values the file carries.

pub mod auto_filter;
pub mod chart;
pub mod comments;
pub mod conditional_format;
pub mod data_validations;
pub mod drawing;
pub mod error;
pub mod filter_state;
pub mod formula;
pub mod model;
pub mod phonetic;
pub mod rich_text;
pub mod shared_strings;
pub mod sheet;
pub mod sparkline;
pub mod styles;
pub mod table;
mod theme;
pub mod workbook_part;
pub mod xstring;

pub use error::{Result, XlsxError};
pub use formula::{
    collapse_ref_errors, delete_sheet_in_formula, move_formula, quote_sheet_name,
    rename_sheet_in_formula, shift_formula, Axis, StructureChange,
};
pub use jade_opc::OpcPackage;
pub use model::{
    builtin_number_format, Alignment, ArrayFormula, Border, BorderEdge, CalculationMode, Cell,
    CellComment, CellFormat, CellRange, CellRef, CellValue, Color, ColumnSpan, DataValidation,
    DateSystem, DefinedName, DrawingColor, Fill, Font, FreezePane, HeaderFooter,
    HorizontalAlignment, Hyperlink, PageBreak, PageMargins, PageOrientation, PageSetup, Row, Shape,
    ShapeKind, ShapePaint, ShapeStyle, Sheet, SheetProtection, SheetVisibility, StyleTable, Table,
    TableStyleDefinition, TableStyleElement, VerticalAlignment, Workbook, EMU_PER_PX,
};
pub use sheet::{translate_formula, transpose_formula};
pub use workbook_part::{SheetEntry, WorkbookPart};

/// The relationship type of the package's main part, whose last segment is all
/// that is stable between the strict and transitional conventions.
const OFFICE_DOCUMENT: &str = "/officeDocument";
const COMMENTS: &str = "/comments";
const DRAWING: &str = "/drawing";
const TABLE: &str = "/table";
const SHARED_STRINGS: &str = "/sharedStrings";
const STYLES: &str = "/styles";
const THEME: &str = "/theme";
const WORKSHEET: &str = "/worksheet";

/// Reads a whole workbook from `.xlsx` bytes.
/// The readings of `sheet`'s text cells that carry furigana. A guide the
/// file got wrong is left out rather than failing the read.
fn furigana(
    package: &OpcPackage,
    part: &str,
    sheet: &Sheet,
) -> std::collections::BTreeMap<CellRef, String> {
    let text_cells: Vec<CellRef> = sheet
        .rows
        .iter()
        .flat_map(|row| &row.cells)
        .filter(|cell| matches!(cell.value, CellValue::Text(_)))
        .map(|cell| cell.reference)
        .collect();
    let (Some(first), Some(last)) = (text_cells.first(), text_cells.last()) else {
        return Default::default();
    };
    let range = CellRange {
        start: CellRef::new(
            text_cells.iter().map(|at| at.column).min().unwrap_or(0),
            first.row,
        ),
        end: CellRef::new(
            text_cells.iter().map(|at| at.column).max().unwrap_or(0),
            last.row,
        ),
    };
    let columns = text_cells.iter().map(|at| at.column).collect();
    phonetic::package_readings(package, part, range, &columns)
        .map(|readings| {
            readings
                .into_iter()
                .filter_map(|(at, reading)| reading.key.map(|key| (at, key)))
                .collect()
        })
        .unwrap_or_default()
}

pub fn read_xlsx(bytes: &[u8]) -> Result<Workbook> {
    let package = OpcPackage::open(bytes)?;
    read_package(&package)
}

/// Reads an already-open package, including in-memory part replacements.
pub fn read_package(package: &OpcPackage) -> Result<Workbook> {
    let workbook_part = match package.part_str("_rels/.rels")? {
        Some(xml) => jade_opc::parse_relationships(&xml)?
            .target_by_type_suffix(OFFICE_DOCUMENT)
            .map(|target| jade_opc::resolve_target("", target))
            .unwrap_or_else(|| "xl/workbook.xml".to_string()),
        None => "xl/workbook.xml".to_string(),
    };

    let workbook_xml = package.required_part_str(&workbook_part)?;
    let workbook_rels = match package.part_str(&jade_opc::rels_part_name(&workbook_part))? {
        Some(xml) => jade_opc::parse_relationships(&xml)?,
        None => Default::default(),
    };

    let mut shared_furigana = false;
    let mut shared_runs = Vec::new();
    let shared_strings = match workbook_rels.target_by_type_suffix(SHARED_STRINGS) {
        Some(target) => {
            let part = jade_opc::resolve_target(&workbook_part, target);
            match package.part_str(&part)? {
                Some(xml) => {
                    shared_furigana = xml.contains("rPh");
                    if xml.contains("<r>") || xml.contains(":r>") {
                        shared_runs = rich_text::parse_shared_runs(&xml)?;
                    }
                    shared_strings::parse_shared_strings(&xml)?
                }
                None => Vec::new(),
            }
        }
        None => Vec::new(),
    };

    let theme = match workbook_rels.target_by_type_suffix(THEME) {
        Some(target) => {
            let part = jade_opc::resolve_target(&workbook_part, target);
            match package.part_str(&part)? {
                Some(xml) => Some(theme::parse_theme(&xml)?),
                None => None,
            }
        }
        None => None,
    };

    let mut styles = match workbook_rels.target_by_type_suffix(STYLES) {
        Some(target) => {
            let part = jade_opc::resolve_target(&workbook_part, target);
            match package.part_str(&part)? {
                Some(xml) => styles::parse_styles(&xml)?,
                None => StyleTable::default(),
            }
        }
        None => StyleTable::default(),
    };
    if let Some(theme) = &theme {
        styles::apply_theme_colors(&mut styles, theme);
    }

    let part = workbook_part::parse_workbook(&workbook_xml)?;

    let mut sheets = Vec::new();
    let mut media_cache = std::collections::HashMap::new();
    for (position, entry) in part.sheets.into_iter().enumerate() {
        let target = entry
            .relationship_id
            .as_deref()
            .and_then(|id| workbook_rels.target(id))
            // A workbook without a usable r:id still names its sheets in order,
            // and Excel's own numbering is 1-based and positional.
            .map(|target| jade_opc::resolve_target(&workbook_part, target))
            .unwrap_or_else(|| format!("xl/worksheets/sheet{}.xml", position + 1));
        let Some(xml) = package.part_str(&target)? else {
            continue;
        };
        let relationships = match package.part_str(&jade_opc::rels_part_name(&target))? {
            Some(xml) => jade_opc::parse_relationships(&xml)?,
            None => Default::default(),
        };
        let mut sheet = sheet::parse_sheet_with_relationships(
            &entry.name,
            &target,
            &xml,
            &shared_strings,
            &relationships,
        )?;
        read_sheet_parts(
            package,
            &target,
            &xml,
            &relationships,
            &mut sheet,
            &mut media_cache,
        )?;
        sheet.filter_row_state = filter_state::read(&xml, &sheet)?;
        if shared_furigana || xml.contains("rPh") {
            sheet.phonetic_readings = furigana(package, &target, &sheet);
        }
        sheet.rich_text = rich_text::sheet_runs(&xml, &shared_runs)?;
        sheet.visibility = entry.visibility;
        read_print_titles(&part.defined_names, position, &mut sheet.page_setup);
        sheets.push(sheet);
    }

    // A workbook whose sheet list is unreadable still has worksheet parts; fall
    // back to them so a damaged `workbook.xml` degrades to unnamed sheets
    // rather than an empty document.
    if sheets.is_empty() {
        if let Some(target) = workbook_rels.target_by_type_suffix(WORKSHEET) {
            let part = jade_opc::resolve_target(&workbook_part, target);
            if let Some(xml) = package.part_str(&part)? {
                let relationships = match package.part_str(&jade_opc::rels_part_name(&part))? {
                    Some(xml) => jade_opc::parse_relationships(&xml)?,
                    None => Default::default(),
                };
                let mut sheet = sheet::parse_sheet_with_relationships(
                    "Sheet1",
                    &part,
                    &xml,
                    &shared_strings,
                    &relationships,
                )?;
                read_sheet_parts(
                    package,
                    &part,
                    &xml,
                    &relationships,
                    &mut sheet,
                    &mut media_cache,
                )?;
                sheet.filter_row_state = filter_state::read(&xml, &sheet)?;
                sheets.push(sheet);
            }
        }
    }

    if let Some(theme) = &theme {
        for sheet in &mut sheets {
            conditional_format::apply_theme(&mut sheet.conditional_formats, theme);
            let colors = sheet
                .rich_text
                .values_mut()
                .flatten()
                .map(|run| &mut run.font.color)
                .chain(std::iter::once(&mut sheet.tab_color))
                .chain(sheet.sparkline_groups.iter_mut().flat_map(|group| {
                    [
                        &mut group.color_series,
                        &mut group.color_negative,
                        &mut group.color_axis,
                        &mut group.color_markers,
                        &mut group.color_first,
                        &mut group.color_last,
                        &mut group.color_high,
                        &mut group.color_low,
                    ]
                }));
            for color in colors {
                if let Some(Color::Theme {
                    index, resolved, ..
                }) = color.as_mut()
                {
                    *resolved = theme.color(*index);
                }
            }
            for chart in sheet.shapes.iter_mut().filter_map(|s| s.chart.as_mut()) {
                chart::apply_theme(chart, theme);
            }
        }
    }
    Ok(Workbook {
        sheets,
        styles,
        defined_names: part.defined_names,
        date_system: part.date_system,
        active_sheet_index: part.active_sheet_index,
        full_calculation_on_load: part.full_calculation_on_load,
        calculation_mode: part.calculation_mode,
        calculate_on_save: part.calculate_on_save.unwrap_or(true),
    })
}

/// The rows and columns the sheet at tab `position` repeats on every
/// printed page, from its `_xlnm.Print_Titles`, into `setup`: `Sheet1!$1:$2`
/// rows 0 to 1, `Sheet1!$A:$A` column 0, or both joined by a comma.
fn read_print_titles(names: &[DefinedName], position: usize, setup: &mut PageSetup) {
    let Some(name) = names.iter().find(|name| {
        name.local_sheet_index == Some(position)
            && name.name.eq_ignore_ascii_case("_xlnm.Print_Titles")
    }) else {
        return;
    };
    let mut quoted = false;
    for part in name.formula.split(|c| {
        if c == '\'' {
            quoted = !quoted;
        }
        c == ',' && !quoted
    }) {
        let reference = part
            .rsplit_once('!')
            .map_or(part, |(_, reference)| reference);
        let reference = reference.trim().replace('$', "");
        let Some((first, last)) = reference.split_once(':') else {
            continue;
        };
        if let (Ok(first), Ok(last)) = (first.parse::<u32>(), last.parse::<u32>()) {
            if first > 0 && last > 0 {
                setup.title_rows = Some((first.min(last) - 1, first.max(last) - 1));
            }
        } else if let (Ok(first), Ok(last)) = (
            CellRef::parse(&format!("{first}1")),
            CellRef::parse(&format!("{last}1")),
        ) {
            setup.title_columns =
                Some((first.column.min(last.column), first.column.max(last.column)));
        }
    }
}

/// The parts a worksheet points at through its relationships — comments,
/// tables, the drawing — read into `sheet`, along with the validations that
/// live in the sheet part itself.
fn read_sheet_parts(
    package: &OpcPackage,
    sheet_part: &str,
    sheet_xml: &str,
    relationships: &jade_opc::Relationships,
    sheet: &mut Sheet,
    media_cache: &mut std::collections::HashMap<String, model::MediaPart>,
) -> Result<()> {
    sheet.data_validations = data_validations::parse_data_validations(sheet_xml)?;
    if let Some(comments_target) = relationships.target_by_type_suffix(COMMENTS) {
        let comments_part = jade_opc::resolve_target(sheet_part, comments_target);
        if let Some(xml) = package.part_str(&comments_part)? {
            sheet.comments = comments::parse_comments(&xml)?;
        }
    }
    for relationship in relationships.by_type_suffix(TABLE) {
        let table_part = jade_opc::resolve_target(sheet_part, &relationship.target);
        if let Some(xml) = package.part_str(&table_part)? {
            if let Some(table) = table::parse_table(&table_part, &xml)? {
                sheet.tables.push(table);
            }
        }
    }
    if let Some(drawing_target) = relationships.target_by_type_suffix(DRAWING) {
        let drawing_part = jade_opc::resolve_target(sheet_part, drawing_target);
        if let Some(xml) = package.part_str(&drawing_part)? {
            sheet.shapes = drawing::parse_drawing(&xml)?;
            let drawing_rels = match package.part_str(&jade_opc::rels_part_name(&drawing_part))? {
                Some(xml) => jade_opc::parse_relationships(&xml)?,
                None => Default::default(),
            };
            for picture in sheet
                .shapes
                .iter_mut()
                .filter_map(|shape| shape.picture.as_mut())
            {
                if let Some(relationship) = picture
                    .relationship_id
                    .as_deref()
                    .and_then(|id| drawing_rels.relationship(id))
                {
                    if !relationship.relationship_type.ends_with("/image") {
                        continue;
                    }
                    if relationship.external {
                        picture.external_target = Some(relationship.target.clone());
                        continue;
                    }
                    let part_name = jade_opc::resolve_target(&drawing_part, &relationship.target);
                    if let Some(media) = media_cache.get(&part_name) {
                        picture.media = Some(media.clone());
                        continue;
                    }
                    if let Some(bytes) = package.part(&part_name) {
                        use std::hash::{Hash, Hasher};
                        let mut hash = std::collections::hash_map::DefaultHasher::new();
                        bytes.hash(&mut hash);
                        let cache_key = format!("{part_name}:{:016x}", hash.finish());
                        let extension = part_name
                            .rsplit('.')
                            .next()
                            .unwrap_or("")
                            .to_ascii_lowercase();
                        let fallback_type = match extension.as_str() {
                            "png" => "image/png",
                            "jpg" | "jpeg" => "image/jpeg",
                            "gif" => "image/gif",
                            "svg" => "image/svg+xml",
                            "bmp" => "image/bmp",
                            "webp" => "image/webp",
                            "tif" | "tiff" => "image/tiff",
                            _ => "application/octet-stream",
                        }
                        .to_string();
                        let content_type =
                            media_content_type(package, &part_name)?.unwrap_or(fallback_type);
                        let media = model::MediaPart {
                            part_name,
                            cache_key,
                            content_type,
                            bytes: std::rc::Rc::new(bytes.to_vec()),
                        };
                        media_cache.insert(media.part_name.clone(), media.clone());
                        picture.media = Some(media);
                    }
                }
            }
            for chart in sheet
                .shapes
                .iter_mut()
                .filter_map(|shape| shape.chart.as_mut())
            {
                if let Some(relationship) = chart
                    .relationship_id
                    .as_deref()
                    .and_then(|id| drawing_rels.relationship(id))
                {
                    if relationship.external {
                        chart
                            .unsupported_properties
                            .push("external chart relationship".into());
                        continue;
                    }
                    let part_name = jade_opc::resolve_target(&drawing_part, &relationship.target);
                    if relationship.relationship_type.ends_with("/chart") {
                        if let Some(xml) = package.part_str(&part_name)? {
                            let mut parsed = chart::parse_chart(&xml)?;
                            parsed.relationship_id = chart.relationship_id.clone();
                            parsed.part_name = Some(part_name);
                            *chart = parsed;
                            continue;
                        }
                    }
                }
                chart
                    .unsupported_properties
                    .push("missing chart relationship or part".into());
            }
        }
    }
    Ok(())
}

fn media_content_type(package: &OpcPackage, part: &str) -> Result<Option<String>> {
    use quick_xml::{events::Event, Reader};
    let Some(xml) = package.part_str("[Content_Types].xml")? else {
        return Ok(None);
    };
    let extension = part.rsplit('.').next().unwrap_or("");
    let mut reader = Reader::from_str(&xml);
    let mut default = None;
    loop {
        match reader.read_event()? {
            Event::Start(element) | Event::Empty(element) => {
                let name = jade_opc::local_name(element.name().as_ref()).to_vec();
                if name != b"Override" && name != b"Default" {
                    continue;
                }
                let mut target = None;
                let mut content_type = None;
                for attribute in element.attributes() {
                    let attribute = attribute?;
                    match attribute.key.as_ref() {
                        b"PartName" | b"Extension" => {
                            target = Some(attribute.unescape_value()?.into_owned())
                        }
                        b"ContentType" => {
                            content_type = Some(attribute.unescape_value()?.into_owned())
                        }
                        _ => {}
                    }
                }
                if name == b"Override"
                    && target
                        .as_deref()
                        .is_some_and(|value| value.trim_start_matches('/') == part)
                {
                    return Ok(content_type);
                }
                if name == b"Default"
                    && target
                        .as_deref()
                        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
                {
                    default = content_type;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(default)
}
