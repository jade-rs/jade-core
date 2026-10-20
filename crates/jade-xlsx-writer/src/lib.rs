//! Writing a workbook back, by patching the parts that changed.
//!
//! The whole point of jade reading `.xlsx` itself rather than through a
//! spreadsheet library is that the file survives a round trip. A writer that
//! regenerated the package would undo that on the first save: every part it
//! does not understand — pivot caches, charts, conditional formatting, the
//! calculation chain — would come back different or not at all.
//!
//! So this changes bytes, not documents. The sheet part is rewritten by
//! splicing replacement cells into the original text at the byte ranges the
//! old ones occupied; every other part goes back exactly as it came in. A save
//! that set one cell shows up as a change to one part.
//!
//! ## What a written cell looks like
//!
//! Text is written inline (`t="inlineStr"`) rather than added to the shared
//! string table. Excel reads both, and the inline form leaves `sharedStrings`
//! and its counts untouched — one less part to get wrong, at the cost of not
//! sharing a string that repeats. The style index of the cell being replaced is
//! kept, so a value written into a formatted cell keeps its format.
//!
//! Setting a value removes the formula that was there, which is what a
//! spreadsheet does when you type over one. A formula edit writes `<f>` with
//! no cached result and asks the opening spreadsheet to calculate it.
//!
//! ## Formulas that depended on what changed
//!
//! Their cached values are now wrong, and nothing here can recalculate them —
//! that needs an engine jade does not have yet. What it can do is say so in the
//! file: every write sets `calcPr/@fullCalcOnLoad`, which is Excel's own way of
//! recording "recalculate everything before showing this". A spreadsheet
//! opening the result gets correct numbers; jade's own reads keep reporting the
//! stale cached value until the engine lands, which is why the headless API
//! reports the formula alongside it.

pub mod auto_filter;
mod calc;
pub mod chart_cache;
pub mod clipboard;
pub mod conditional_formats;
pub mod dimensions;
pub mod drawing;
mod filter;
pub mod filter_state;
pub mod format_axes;
pub mod header_footer;
pub mod merges;
pub mod notes;
mod patch;
mod refresh;
pub mod sheets;
pub mod shift;
pub mod sort;
pub mod spills;
pub mod styles;
pub mod table;

pub use calc::{request_full_recalculation, withdraw_full_recalculation};
pub use conditional_formats::{
    add_conditional_format, clear_conditional_formats, delete_conditional_format, ConditionalStyle,
    NewConditionalRule,
};
pub use drawing::{set_shape_text, shift_chart, shift_comments, shift_drawing, shift_vml};
pub use format_axes::{format_axes, AxisFormatResult};
pub use header_footer::set_header_footer;
pub use merges::set_merges;
pub use notes::{delete_note, set_note};
pub use patch::patch_sheet;
pub use refresh::{refresh_sheet, CachedValue, CachedValueEdit, SheetCachedValues};
pub use shift::{restructure, shift_defined_names, shift_sheet};
pub use styles::{add_cell_format, BorderLine, BorderStyle, FormatChange};
pub use table::{
    fresh_column_name, remove_table_parts, rename_table_column, shift_table, unique_column_name,
    ShiftedTable,
};

pub use chart_cache::{refresh_chart_caches, ChartCacheRefresh, VerifiedChartCells};
use jade_opc::OpcPackage;
use jade_xlsx_reader::CellRef;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WriteError {
    #[error("sort: {0}")]
    Sort(String),
    #[error("filter: {0}")]
    AutoFilter(String),
    #[error("invalid axis size {value}; maximum is {maximum}")]
    InvalidAxisSize { value: f32, maximum: f32 },
    #[error("clipboard: {0}")]
    Clipboard(String),
    #[error("no sheet called `{0}`")]
    NoSuchSheet(String),
    #[error("a structural change needs at least one row or column")]
    EmptyStructureChange,
    #[error("axis formatting exceeds the {limit} metadata/cell limit")]
    AxisFormatLimit { limit: u64 },
    #[error("inserting rows or columns would discard stored cells at the worksheet edge")]
    InsertionWouldDiscardCells,
    #[error("rows or columns {at}+{count} are outside the sheet")]
    StructureOutOfBounds { at: u32, count: u32 },
    /// A `cellXfs`, font or fill index the styles part does not have.
    #[error("the styles part has no entry at index {index}")]
    NoSuchStyle { index: u32 },
    /// A styles part without the `fonts`, `fills` and `cellXfs` lists a
    /// format has to go into.
    #[error("the styles part has no font, fill and cellXfs tables to add to")]
    NoStyleTables,
    #[error("`{value}` is not an RRGGBB colour")]
    BadColor { value: String },
    #[error("font size must be finite and between 1 and 409 points")]
    BadFontSize,
    #[error("package: {0}")]
    Package(#[from] jade_opc::OpcError),
    #[error("spreadsheet: {0}")]
    Reader(#[from] jade_xlsx_reader::XlsxError),
    #[error("xml: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("attribute: {0}")]
    Attribute(#[from] quick_xml::events::attributes::AttrError),
    #[error("text encoding: {0}")]
    TextEncoding(#[from] quick_xml::encoding::EncodingError),
    #[error("the workbook has no part `{0}`")]
    MissingPart(String),
    #[error("`{0}` has no <sheetData> to write into")]
    NoSheetData(String),
    #[error("cell at zero-based column {column}, row {row} is outside XLSX worksheet bounds")]
    CellOutOfBounds { column: u32, row: u32 },
    #[error("cell at zero-based column {column}, row {row} has a non-finite number")]
    NonFiniteNumber { column: u32, row: u32 },
    #[error(
        "cell at zero-based column {column}, row {row} contains U+{codepoint:04X}, which XML 1.0 forbids"
    )]
    InvalidXmlCharacter {
        column: u32,
        row: u32,
        codepoint: u32,
    },
    #[error("cell at zero-based column {column}, row {row} has an empty formula")]
    EmptyFormula { column: u32, row: u32 },
    /// A drawing object the sheet does not have.
    #[error("the sheet has no shape `{0}`")]
    NoSuchShape(String),
    /// A picture, chart or group: nothing to put text into.
    #[error("shape `{0}` has no text body")]
    ShapeHasNoText(String),
    /// A comments part without the `<commentList>` a note goes into.
    #[error("the comments part has no commentList")]
    NoCommentList,
    /// A sheet name Excel would not accept.
    #[error("`{name}` is not a valid sheet name: {reason}")]
    InvalidSheetName { name: String, reason: &'static str },
    /// A sheet name another sheet already has, compared without case.
    #[error("the workbook already has a sheet called `{0}`")]
    DuplicateSheetName(String),
    /// A workbook has to keep at least one visible sheet.
    #[error("`{0}` is the last visible sheet and cannot be deleted")]
    LastVisibleSheet(String),
    /// A sheet position past the end of the tab list.
    #[error("sheet position {index} is outside the {count} sheets")]
    SheetPositionOutOfRange { index: usize, count: usize },
    /// A conditional format past the end of the sheet's rules.
    #[error("the sheet has no conditional format {0}")]
    NoSuchConditionalFormat(usize),
    /// A part with no root element to write into.
    #[error("the {0} part has no root element")]
    NoRoot(&'static str),
}

pub type Result<T> = std::result::Result<T, WriteError>;

/// Persist application calculation preferences without regenerating workbook XML.
pub fn set_calculation_properties(
    original: &[u8],
    mode: jade_xlsx_reader::CalculationMode,
    calculate_on_save: bool,
) -> Result<Vec<u8>> {
    let mut package = OpcPackage::open(original)?;
    let name = workbook_part_name(&package)?;
    let xml = package
        .part_str(&name)?
        .ok_or_else(|| WriteError::MissingPart(name.clone()))?;
    let updated = calc::set_properties(&xml, mode, calculate_on_save)?;
    if updated == xml {
        return Ok(original.to_vec());
    }
    package.replace(&name, updated.into_bytes());
    Ok(package.write()?)
}

/// Keep incomplete evaluations visible to the next spreadsheet opening the file.
pub fn mark_for_recalculation(original: &[u8]) -> Result<Vec<u8>> {
    let mut package = OpcPackage::open(original)?;
    let part = workbook_part_name(&package)?;
    let xml = package
        .part_str(&part)?
        .ok_or_else(|| WriteError::MissingPart(part.clone()))?;
    let updated = request_full_recalculation(&xml);
    if updated == xml {
        return Ok(original.to_vec());
    }
    package.replace(&part, updated.into_bytes());
    Ok(package.write()?)
}

/// What to put in a cell.
#[derive(Debug, Clone, PartialEq)]
pub enum NewValue {
    Number(f64),
    Text(String),
    Bool(bool),
    /// A formula expression without a leading `=`. Its cached value is left
    /// empty; `fullCalcOnLoad` asks the opening spreadsheet to calculate it.
    Formula(String),
    /// A pasted formula keeps its old cached result until recalculation.
    FormulaCached {
        formula: String,
        cached: jade_xlsx_reader::CellValue,
    },
    Error(String),
    /// Removes the cell element so worksheet defaults apply again.
    Remove,
    /// Clears the cell's value, keeping the cell and its style.
    Blank,
    /// Leaves the cell's content — value, formula and cache — exactly as it
    /// is; only the edit's style applies. A cell the sheet does not have
    /// becomes an empty cell with that style, as Excel's format painter
    /// leaves one.
    Unchanged,
}

/// One cell to change.
#[derive(Debug, Clone, PartialEq)]
pub struct CellEdit {
    pub reference: CellRef,
    pub value: NewValue,
    /// The `cellXfs` index to give the cell. `None` keeps the style it has
    /// (a new cell gets none); `Some(0)` is the default style, and so clears
    /// formatting.
    pub style_index: Option<u32>,
}

/// The edits for one worksheet part.
#[derive(Debug, Clone, PartialEq)]
pub struct SheetEdits {
    /// The part name inside the archive, e.g. `xl/worksheets/sheet1.xml`.
    pub part_name: String,
    pub cells: Vec<CellEdit>,
}

/// Applies edits to a workbook and returns the new `.xlsx` bytes.
pub fn write_xlsx(original: &[u8], sheets: &[SheetEdits]) -> Result<Vec<u8>> {
    write_xlsx_with_styles(original, None, sheets)
}

/// The workbook's styles part: its name in the package, and its text.
pub fn styles_part(original: &[u8]) -> Result<(String, String)> {
    let package = OpcPackage::open(original)?;
    let name = styles_part_name(&package)?;
    let xml = match package.part_str(&name)? {
        Some(xml) => xml,
        None => {
            let workbook = workbook_part_name(&package)?;
            let rels = package.part_str(&jade_opc::rels_part_name(&workbook))?;
            if rels
                .as_deref()
                .map(jade_opc::parse_relationships)
                .transpose()?
                .is_some_and(|rels| rels.target_by_type_suffix("/styles").is_some())
            {
                return Err(WriteError::MissingPart(name));
            }
            let strict = package
                .part_str(&workbook)?
                .is_some_and(|xml| xml.contains("http://purl.oclc.org/ooxml/spreadsheetml/main"));
            styles::minimal_styles(strict)
        }
    };
    Ok((name, xml))
}

/// The workbook part's name, from the package relationships.
pub(crate) fn workbook_part_name(package: &OpcPackage) -> Result<String> {
    Ok(match package.part_str("_rels/.rels")? {
        Some(xml) => jade_opc::parse_relationships(&xml)?
            .target_by_type_suffix("/officeDocument")
            .map(|target| jade_opc::resolve_target("", target))
            .unwrap_or_else(|| "xl/workbook.xml".to_string()),
        None => "xl/workbook.xml".to_string(),
    })
}

fn styles_part_name(package: &OpcPackage) -> Result<String> {
    let workbook_part = workbook_part_name(package)?;
    let target = match package.part_str(&jade_opc::rels_part_name(&workbook_part))? {
        Some(xml) => jade_opc::parse_relationships(&xml)?
            .target_by_type_suffix("/styles")
            .map(|target| jade_opc::resolve_target(&workbook_part, target)),
        None => None,
    };
    Ok(target.unwrap_or_else(|| {
        workbook_part.rsplit_once('/').map_or_else(
            || "styles.xml".to_string(),
            |(directory, _)| format!("{directory}/styles.xml"),
        )
    }))
}

/// [`write_xlsx`] with a new styles part as well — the text
/// [`add_cell_format`] returns — so the `xf` indices the edits name exist
/// in the file they land in.
pub fn write_xlsx_with_styles(
    original: &[u8],
    styles: Option<(&str, &str)>,
    sheets: &[SheetEdits],
) -> Result<Vec<u8>> {
    write_xlsx_with_parts(original, styles, sheets, &[])
}

/// [`write_xlsx_with_styles`] with further parts replaced wholesale — a
/// table part whose column was renamed by the header cell edited beside
/// it, a drawing whose text box was rewritten. Each is `(part name, new
/// text)`.
pub fn write_xlsx_with_parts(
    original: &[u8],
    styles: Option<(&str, &str)>,
    sheets: &[SheetEdits],
    parts: &[(String, String)],
) -> Result<Vec<u8>> {
    Ok(prepare_xlsx_with_parts(original, styles, sheets, parts, true)?.write()?)
}

pub(crate) fn prepare_xlsx_with_parts(
    original: &[u8],
    styles: Option<(&str, &str)>,
    sheets: &[SheetEdits],
    parts: &[(String, String)],
    refresh_caches: bool,
) -> Result<OpcPackage> {
    let mut package = OpcPackage::open(original)?;
    let mut changed = false;
    if let Some((name, xml)) = styles {
        styles::install_styles_relationship(&mut package, name)?;
        package.replace(name, xml.as_bytes().to_vec());
    }
    for (name, xml) in parts {
        if package.part(name).is_none() {
            return Err(WriteError::MissingPart(name.clone()));
        }
        package.replace(name, xml.as_bytes().to_vec());
    }
    for sheet in sheets {
        if sheet.cells.is_empty() {
            continue;
        }
        let xml = package
            .part_str(&sheet.part_name)?
            .ok_or_else(|| WriteError::MissingPart(sheet.part_name.clone()))?;
        let patched = patch_sheet(&xml, &sheet.cells)?;
        package.replace(&sheet.part_name, patched.into_bytes());
        changed = true;
    }

    if changed {
        // Dependent formulas now hold stale cached values. Nothing here can
        // recalculate them, so the file says it needs recalculating.
        let name = workbook_part_name(&package)?;
        if let Some(xml) = package.part_str(&name)? {
            package.replace(&name, request_full_recalculation(&xml).into_bytes());
        }
    }
    if refresh_caches && (changed || styles.is_some() || !parts.is_empty()) {
        chart_cache::refresh_package(&mut package, &VerifiedChartCells::new())?;
    }
    Ok(package)
}

/// Refreshes formula cells' cached values and returns the new `.xlsx` bytes.
///
/// With `withdraw_recalculation`, `calcPr/@fullCalcOnLoad` is also turned
/// off — the caller is saying every formula's cache is now current, so a
/// spreadsheet opening the file need not recalculate first.
pub fn write_cached_values(
    original: &[u8],
    sheets: &[SheetCachedValues],
    withdraw_recalculation: bool,
) -> Result<Vec<u8>> {
    write_cached_values_verified(
        original,
        sheets,
        withdraw_recalculation,
        &VerifiedChartCells::new(),
    )
}

/// Also accepts unchanged formula cells verified by the calculator, so chart
/// caches can be refreshed while other unsupported formulas remain pending.
pub fn write_cached_values_verified(
    original: &[u8],
    sheets: &[SheetCachedValues],
    withdraw_recalculation: bool,
    verified: &VerifiedChartCells,
) -> Result<Vec<u8>> {
    let mut package = OpcPackage::open(original)?;
    let mut fresh = verified.clone();
    for sheet in sheets {
        if sheet.cells.is_empty() {
            continue;
        }
        let xml = package
            .part_str(&sheet.part_name)?
            .ok_or_else(|| WriteError::MissingPart(sheet.part_name.clone()))?;
        let refreshed = refresh_sheet(&xml, &sheet.cells)?;
        fresh
            .entry(sheet.part_name.clone())
            .or_default()
            .extend(sheet.cells.iter().map(|cell| cell.reference));
        package.replace(&sheet.part_name, refreshed.into_bytes());
    }
    if withdraw_recalculation {
        let name = workbook_part_name(&package)?;
        if let Some(xml) = package.part_str(&name)? {
            package.replace(&name, withdraw_full_recalculation(&xml).into_bytes());
        }
    }
    let charts = chart_cache::refresh_package(&mut package, &fresh)?;
    if charts.updated_caches == 0
        && !withdraw_recalculation
        && sheets.iter().all(|sheet| sheet.cells.is_empty())
    {
        return Ok(original.to_vec());
    }
    Ok(package.write()?)
}
