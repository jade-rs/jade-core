//! Structure-aware comparison for Excel `.xlsx` workbooks.
//!
//! The crate aligns sheets, rows, and columns before comparing cells so that an
//! insertion does not turn every following cell into a false-positive change.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs::File;
use std::io::{Cursor, Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use calamine::{
    Data, DataRef, DataType, Dimensions, Reader, Xlsx, XlsxError as CalamineXlsxError,
    open_workbook, open_workbook_from_rs,
};
use clap::{Parser, ValueEnum};
use html_escape::encode_text;
use quick_xml::Reader as XmlReader;
use quick_xml::events::{BytesStart as XmlStart, Event as XmlEvent};
use rust_xlsxwriter::{Color, Format, Workbook, XlsxError};
use serde::Serialize;
use unicode_normalization::UnicodeNormalization;
use zip::ZipArchive;

const GAP_PENALTY: f64 = 0.46;
const SINGLE_ITEM_CONTEXT_BONUS: f64 = 0.10;

/// Version of the serialized [`WorkbookDiff`] contract.
pub const DIFF_SCHEMA_VERSION: u32 = 1;

/// Whitespace handling applied while loading string cells.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WhitespaceNormalization {
    /// Preserve cell text exactly as stored in the workbook.
    Preserve,
    /// Collapse whitespace, normalize canonical Unicode, and ignore soft hyphens.
    #[default]
    Collapse,
}

/// Resource limits used when reading and aligning untrusted workbooks.
#[derive(Debug, Clone, Serialize)]
pub struct ResourceLimits {
    pub max_input_bytes: u64,
    pub max_uncompressed_bytes: u64,
    pub max_zip_entries: usize,
    pub max_rows_per_sheet: usize,
    pub max_columns_per_sheet: usize,
    pub max_cells: usize,
    pub max_alignment_pairs: usize,
    /// Maximum rows or columns on either side of a global assignment.
    pub max_assignment_size: usize,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 64 * 1024 * 1024,
            max_uncompressed_bytes: 512 * 1024 * 1024,
            max_zip_entries: 10_000,
            max_rows_per_sheet: 100_000,
            max_columns_per_sheet: 16_384,
            max_cells: 5_000_000,
            max_alignment_pairs: 4_000_000,
            max_assignment_size: 512,
        }
    }
}

/// Policy for comparing values cached alongside formula cells by Excel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FormulaValueComparison {
    /// Ignore cached values when the formulas are structurally equivalent.
    #[default]
    IgnoreWhenFormulasEquivalent,
    /// Report cached value changes even when the formulas are equivalent.
    CompareCached,
}

/// Configuration for loading and comparing workbooks.
#[derive(Debug, Clone, Serialize)]
pub struct DiffOptions {
    pub whitespace: WhitespaceNormalization,
    pub formula_values: FormulaValueComparison,
    pub limits: ResourceLimits,
    pub min_match_score: f64,
    pub move_match_score: f64,
    pub sheet_rename_score: f64,
    pub ambiguity_margin: f64,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            whitespace: WhitespaceNormalization::Collapse,
            formula_values: FormulaValueComparison::IgnoreWhenFormulasEquivalent,
            limits: ResourceLimits::default(),
            min_match_score: 0.38,
            move_match_score: 0.85,
            sheet_rename_score: 0.6,
            ambiguity_margin: 0.03,
        }
    }
}

impl DiffOptions {
    fn validate(&self) -> Result<()> {
        for (name, value) in [
            ("min_match_score", self.min_match_score),
            ("move_match_score", self.move_match_score),
            ("sheet_rename_score", self.sheet_rename_score),
            ("ambiguity_margin", self.ambiguity_margin),
        ] {
            anyhow::ensure!(
                (0.0..=1.0).contains(&value),
                "{name} must be between 0 and 1"
            );
        }
        anyhow::ensure!(
            self.limits.max_alignment_pairs > 0,
            "max_alignment_pairs must be greater than zero"
        );
        anyhow::ensure!(
            self.limits.max_assignment_size > 0,
            "max_assignment_size must be greater than zero"
        );
        Ok(())
    }
}

/// Presentation used by human-readable HTML and XLSX reports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum ReportLayout {
    /// Render one merged stream of change events.
    #[default]
    Merged,
    /// Align the old and new sides in adjacent columns.
    SideBySide,
}

#[derive(Debug, Parser)]
#[command(author, version, about = "Diff xlsx files with row/column alignment")]
struct Cli {
    /// Old xlsx file.
    old: PathBuf,
    /// New xlsx file.
    new: PathBuf,
    /// Write JSON diff to this path. Prints JSON to stdout when no output is specified.
    #[arg(long)]
    json: Option<PathBuf>,
    /// Write HTML report to this path.
    #[arg(long)]
    html: Option<PathBuf>,
    /// Write xlsx report to this path.
    #[arg(long)]
    xlsx: Option<PathBuf>,
    /// Write all report formats into this directory.
    #[arg(long)]
    out_dir: Option<PathBuf>,
    /// Basename used with --out-dir.
    #[arg(long, default_value = "diff")]
    basename: String,
    /// Layout for HTML and XLSX reports.
    #[arg(long, alias = "layout", value_enum, default_value_t = ReportLayout::Merged)]
    report_layout: ReportLayout,
    /// Exit with status 1 when differences are found.
    #[arg(long)]
    check: bool,
    /// Preserve string whitespace exactly instead of collapsing it.
    #[arg(long)]
    preserve_whitespace: bool,
    /// Compare cached formula values even when formulas are structurally equivalent.
    #[arg(long)]
    compare_cached_formula_values: bool,
    /// Maximum compressed input size in bytes.
    #[arg(long, default_value_t = ResourceLimits::default().max_input_bytes)]
    max_input_bytes: u64,
    /// Maximum total uncompressed ZIP size in bytes.
    #[arg(long, default_value_t = ResourceLimits::default().max_uncompressed_bytes)]
    max_uncompressed_bytes: u64,
    /// Maximum number of files in the XLSX ZIP container.
    #[arg(long, default_value_t = ResourceLimits::default().max_zip_entries)]
    max_zip_entries: usize,
    /// Maximum rows in one worksheet used range.
    #[arg(long, default_value_t = ResourceLimits::default().max_rows_per_sheet)]
    max_rows_per_sheet: usize,
    /// Maximum columns in one worksheet used range.
    #[arg(long, default_value_t = ResourceLimits::default().max_columns_per_sheet)]
    max_columns_per_sheet: usize,
    /// Maximum cells loaded across a workbook.
    #[arg(long, default_value_t = ResourceLimits::default().max_cells)]
    max_cells: usize,
    /// Maximum similarity comparisons in one alignment.
    #[arg(long, default_value_t = ResourceLimits::default().max_alignment_pairs)]
    max_alignment_pairs: usize,
    /// Maximum rows or columns on either side of a global assignment.
    #[arg(long, default_value_t = ResourceLimits::default().max_assignment_size)]
    max_assignment_size: usize,
    /// Minimum score for rows or columns to be aligned.
    #[arg(long, default_value_t = DiffOptions::default().min_match_score)]
    min_match_score: f64,
    /// Minimum score for an add/remove pair to be classified as moved.
    #[arg(long, default_value_t = DiffOptions::default().move_match_score)]
    move_match_score: f64,
    /// Minimum content score for a sheet rename.
    #[arg(long, default_value_t = DiffOptions::default().sheet_rename_score)]
    sheet_rename_score: f64,
}

#[derive(Debug, Clone)]
pub struct WorkbookIr {
    pub sheets: Vec<SheetIr>,
    /// Canonical workbook default style used for implicit cells.
    #[doc(hidden)]
    pub default_style_signature: String,
    /// Non-default explicitly styled cells keyed by sheet and one-based position.
    #[doc(hidden)]
    pub explicit_cell_styles: BTreeMap<String, BTreeMap<(usize, usize), String>>,
}

#[derive(Debug, Clone)]
pub struct SheetIr {
    pub name: String,
    /// One-based row of `rows[0][0]` in the source worksheet.
    pub start_row: usize,
    /// One-based column of `rows[0][0]` in the source worksheet.
    pub start_col: usize,
    pub rows: Vec<Vec<CellIr>>,
    pub merged_ranges: Vec<CellRange>,
}

#[derive(Debug, Clone, Default)]
pub struct CellIr {
    pub value: String,
    pub value_kind: CellValueKind,
    pub formula: Option<String>,
    /// Resolved visual style used only for comparison.
    pub style_signature: String,
}

/// Logical type of a normalized cell value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CellValueKind {
    #[default]
    Empty,
    String,
    Number,
    Boolean,
    Error,
    DateTime,
    Duration,
}

#[derive(Debug, Serialize)]
pub struct WorkbookDiff {
    pub schema_version: u32,
    pub coordinate_system: CoordinateSystem,
    pub options: DiffOptions,
    pub old_file: String,
    pub new_file: String,
    pub summary: DiffSummary,
    pub sheets: Vec<SheetDiff>,
}

impl WorkbookDiff {
    /// Returns `true` when any sheet contains a structural or cell change.
    pub fn has_changes(&self) -> bool {
        self.sheets
            .iter()
            .any(|sheet| !matches!(sheet.status, SheetStatus::Unchanged))
    }
}

/// Coordinate convention used by every numeric row, column, and range field.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordinateSystem {
    OneBased,
}

#[derive(Debug, Default, Serialize)]
pub struct DiffSummary {
    pub sheets_added: usize,
    pub sheets_removed: usize,
    pub sheets_renamed: usize,
    pub sheet_order_changes: usize,
    pub table_regions_added: usize,
    pub table_regions_removed: usize,
    pub rows_added: usize,
    pub rows_removed: usize,
    pub rows_moved: usize,
    pub columns_added: usize,
    pub columns_removed: usize,
    pub columns_moved: usize,
    pub cells_changed: usize,
    pub formulas_changed: usize,
    pub cell_styles_changed: usize,
    pub merged_cells_added: usize,
    pub merged_cells_removed: usize,
}

#[derive(Debug, Serialize)]
pub struct SheetDiff {
    pub old_name: Option<String>,
    pub new_name: Option<String>,
    pub status: SheetStatus,
    pub old_position: Option<usize>,
    pub new_position: Option<usize>,
    pub order_changed: bool,
    pub match_confidence: Option<f64>,
    pub alignment: AlignmentDiagnostics,
    pub table_regions: Vec<TableRegionDiff>,
    pub row_changes: Vec<RowChange>,
    pub column_changes: Vec<ColumnChange>,
    pub cell_changes: Vec<CellChange>,
    pub merge_changes: Vec<MergeChange>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SheetStatus {
    Added,
    Removed,
    Modified,
    Renamed,
    Reordered,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct CellRange {
    pub start_row: usize,
    pub start_col: usize,
    pub end_row: usize,
    pub end_col: usize,
}

impl From<&Dimensions> for CellRange {
    fn from(dimensions: &Dimensions) -> Self {
        Self {
            start_row: dimensions.start.0 as usize + 1,
            start_col: dimensions.start.1 as usize + 1,
            end_row: dimensions.end.0 as usize + 1,
            end_col: dimensions.end.1 as usize + 1,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct TableRegionDiff {
    pub old_range: Option<CellRange>,
    pub new_range: Option<CellRange>,
    pub status: RegionStatus,
    pub alignment: AlignmentDiagnostics,
    pub row_changes: Vec<RowChange>,
    pub column_changes: Vec<ColumnChange>,
    pub cell_changes: Vec<CellChange>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionStatus {
    Added,
    Removed,
    Modified,
    Unchanged,
}

#[derive(Debug, Clone, Serialize)]
pub struct RowChange {
    pub kind: ChangeKind,
    pub old_index: Option<usize>,
    pub new_index: Option<usize>,
    pub preview: String,
    pub confidence: Option<f64>,
    pub ambiguous: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ColumnChange {
    pub kind: ChangeKind,
    pub old_index: Option<usize>,
    pub new_index: Option<usize>,
    pub header: String,
    pub confidence: Option<f64>,
    pub ambiguous: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CellChange {
    pub old_address: Option<String>,
    pub new_address: Option<String>,
    pub kind: CellChangeKind,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub old_value_kind: Option<CellValueKind>,
    pub new_value_kind: Option<CellValueKind>,
    pub old_formula: Option<String>,
    pub new_formula: Option<String>,
    pub style_changed: bool,
}

/// Summary of heuristic alignment quality for one comparison area.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AlignmentDiagnostics {
    pub row_ambiguous_signature_groups: usize,
    pub column_ambiguous_signature_groups: usize,
    pub low_confidence_row_matches: usize,
    pub low_confidence_column_matches: usize,
    pub minimum_row_confidence: Option<f64>,
    pub minimum_column_confidence: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct MergeChange {
    pub kind: ChangeKind,
    pub old_range: Option<CellRange>,
    pub new_range: Option<CellRange>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Removed,
    Moved,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CellChangeKind {
    ValueChanged,
    FormulaChanged,
    ValueAndFormulaChanged,
    StyleChanged,
    ValueAdded,
    ValueRemoved,
}

#[derive(Debug, Clone)]
enum MatchOp {
    Match(usize, usize, f64),
    Removed(usize),
    Added(usize),
}

pub fn run_cli() -> Result<()> {
    let cli = Cli::parse();
    let defaults = DiffOptions::default();
    let options = DiffOptions {
        whitespace: if cli.preserve_whitespace {
            WhitespaceNormalization::Preserve
        } else {
            WhitespaceNormalization::Collapse
        },
        formula_values: if cli.compare_cached_formula_values {
            FormulaValueComparison::CompareCached
        } else {
            FormulaValueComparison::IgnoreWhenFormulasEquivalent
        },
        limits: ResourceLimits {
            max_input_bytes: cli.max_input_bytes,
            max_uncompressed_bytes: cli.max_uncompressed_bytes,
            max_zip_entries: cli.max_zip_entries,
            max_rows_per_sheet: cli.max_rows_per_sheet,
            max_columns_per_sheet: cli.max_columns_per_sheet,
            max_cells: cli.max_cells,
            max_alignment_pairs: cli.max_alignment_pairs,
            max_assignment_size: cli.max_assignment_size,
        },
        min_match_score: cli.min_match_score,
        move_match_score: cli.move_match_score,
        sheet_rename_score: cli.sheet_rename_score,
        ambiguity_margin: defaults.ambiguity_margin,
    };
    options.validate()?;
    let (old_book, new_book) = load_workbook_pair_with_options(&cli.old, &cli.new, &options)?;
    let diff = diff_workbooks_with_options(&cli.old, &cli.new, &old_book, &new_book, &options)?;
    let outputs = cli_output_paths(&cli);
    ensure_output_directories(&outputs)?;

    let mut wrote_output = false;
    if let Some(path) = &outputs.json {
        std::fs::write(path, serde_json::to_string_pretty(&diff)?)
            .with_context(|| format!("failed to write JSON report: {}", path.display()))?;
        wrote_output = true;
    }
    if let Some(path) = &outputs.html {
        write_html_report_with_layout(path, &diff, cli.report_layout)?;
        wrote_output = true;
    }
    if let Some(path) = &outputs.xlsx {
        write_xlsx_report_with_layout(path, &diff, cli.report_layout)?;
        wrote_output = true;
    }
    if !wrote_output {
        println!("{}", serde_json::to_string_pretty(&diff)?);
    }

    if cli.check && diff.has_changes() {
        std::process::exit(1);
    }

    Ok(())
}

struct CliOutputPaths {
    json: Option<PathBuf>,
    html: Option<PathBuf>,
    xlsx: Option<PathBuf>,
}

fn cli_output_paths(cli: &Cli) -> CliOutputPaths {
    let out_dir_path = |extension: &str| {
        cli.out_dir
            .as_ref()
            .map(|dir| dir.join(format!("{}.{}", cli.basename, extension)))
    };

    CliOutputPaths {
        json: cli.json.clone().or_else(|| out_dir_path("json")),
        html: cli.html.clone().or_else(|| out_dir_path("html")),
        xlsx: cli.xlsx.clone().or_else(|| out_dir_path("xlsx")),
    }
}

fn ensure_output_directories(outputs: &CliOutputPaths) -> Result<()> {
    for path in [&outputs.json, &outputs.html, &outputs.xlsx]
        .into_iter()
        .flatten()
    {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create output directory: {}", parent.display())
            })?;
        }
    }

    Ok(())
}

pub fn load_workbook(path: &Path) -> Result<WorkbookIr> {
    load_workbook_with_options(path, &DiffOptions::default())
}

/// Loads two independent XLSX workbooks concurrently using default options.
pub fn load_workbook_pair(old_path: &Path, new_path: &Path) -> Result<(WorkbookIr, WorkbookIr)> {
    load_workbook_pair_with_options(old_path, new_path, &DiffOptions::default())
}

/// Loads two independent XLSX workbooks concurrently using explicit options.
pub fn load_workbook_pair_with_options(
    old_path: &Path,
    new_path: &Path,
    options: &DiffOptions,
) -> Result<(WorkbookIr, WorkbookIr)> {
    let (old_result, new_result) = std::thread::scope(|scope| {
        let old_loader = scope.spawn(|| load_workbook_with_options(old_path, options));
        let new_loader = scope.spawn(|| load_workbook_with_options(new_path, options));
        (old_loader.join(), new_loader.join())
    });
    let old = old_result
        .map_err(|_| anyhow::anyhow!("old workbook loader panicked"))?
        .with_context(|| format!("failed to load old workbook {}", old_path.display()))?;
    let new = new_result
        .map_err(|_| anyhow::anyhow!("new workbook loader panicked"))?
        .with_context(|| format!("failed to load new workbook {}", new_path.display()))?;
    Ok((old, new))
}

/// Loads an XLSX workbook using explicit normalization and resource limits.
pub fn load_workbook_with_options(path: &Path, options: &DiffOptions) -> Result<WorkbookIr> {
    options.validate()?;
    let input_size = std::fs::metadata(path)
        .with_context(|| format!("failed to read xlsx metadata: {}", path.display()))?
        .len();
    validate_xlsx_container(
        File::open(path).with_context(|| format!("failed to open xlsx: {}", path.display()))?,
        input_size,
        &options.limits,
    )?;
    let styles = load_xlsx_style_metadata(
        File::open(path).with_context(|| format!("failed to reopen xlsx: {}", path.display()))?,
    )?;
    let mut workbook: Xlsx<_> = open_workbook(path)
        .with_context(|| format!("failed to open xlsx workbook: {}", path.display()))?;
    let mut loaded = load_workbook_from_xlsx(&mut workbook, options)?;
    apply_xlsx_styles(&mut loaded, &styles);
    Ok(loaded)
}

pub fn load_workbook_from_bytes(bytes: &[u8]) -> Result<WorkbookIr> {
    load_workbook_from_bytes_with_options(bytes, &DiffOptions::default())
}

/// Loads an XLSX workbook from memory using explicit resource limits.
pub fn load_workbook_from_bytes_with_options(
    bytes: &[u8],
    options: &DiffOptions,
) -> Result<WorkbookIr> {
    options.validate()?;
    validate_xlsx_container(Cursor::new(bytes), bytes.len() as u64, &options.limits)?;
    let styles = load_xlsx_style_metadata(Cursor::new(bytes))?;
    let reader = Cursor::new(bytes.to_vec());
    let mut workbook: Xlsx<_> =
        open_workbook_from_rs(reader).context("failed to open xlsx workbook from bytes")?;
    let mut loaded = load_workbook_from_xlsx(&mut workbook, options)?;
    apply_xlsx_styles(&mut loaded, &styles);
    Ok(loaded)
}

fn validate_xlsx_container<R>(reader: R, input_size: u64, limits: &ResourceLimits) -> Result<()>
where
    R: Read + Seek,
{
    anyhow::ensure!(
        input_size <= limits.max_input_bytes,
        "xlsx input is {input_size} bytes, exceeding max_input_bytes {}",
        limits.max_input_bytes
    );
    let mut archive = ZipArchive::new(reader).context("failed to inspect xlsx ZIP container")?;
    anyhow::ensure!(
        archive.len() <= limits.max_zip_entries,
        "xlsx ZIP contains {} entries, exceeding max_zip_entries {}",
        archive.len(),
        limits.max_zip_entries
    );
    let mut uncompressed = 0u64;
    for idx in 0..archive.len() {
        let entry = archive
            .by_index(idx)
            .with_context(|| format!("failed to inspect xlsx ZIP entry {idx}"))?;
        uncompressed = uncompressed
            .checked_add(entry.size())
            .context("xlsx uncompressed size overflow")?;
        anyhow::ensure!(
            uncompressed <= limits.max_uncompressed_bytes,
            "xlsx expands to more than max_uncompressed_bytes {}",
            limits.max_uncompressed_bytes
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Default)]
struct XlsxStyleMetadata {
    styles: Vec<String>,
    cells_by_sheet: BTreeMap<String, BTreeMap<(usize, usize), usize>>,
}

#[derive(Debug, Clone, Default)]
struct OoxmlNode {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<OoxmlNode>,
    text: String,
}

impl OoxmlNode {
    fn attr(&self, name: &str) -> Option<&str> {
        if let Some((_, value)) = self.attrs.iter().find(|(key, _)| key == name) {
            return Some(value);
        }
        let wanted = name.rsplit(':').next().unwrap_or(name);
        self.attrs
            .iter()
            .find(|(key, _)| key.rsplit(':').next().unwrap_or(key) == wanted)
            .map(|(_, value)| value.as_str())
    }

    fn child(&self, name: &str) -> Option<&OoxmlNode> {
        self.children.iter().find(|child| child.name == name)
    }

    fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a OoxmlNode> {
        self.children.iter().filter(move |child| child.name == name)
    }
}

#[derive(Debug, Clone)]
struct PackageRelationship {
    target: String,
    kind: String,
}

#[derive(Debug, Clone, Default)]
struct XlsxTheme {
    colors: BTreeMap<usize, String>,
    indexed_colors: Option<BTreeMap<usize, String>>,
}

fn load_xlsx_style_metadata<R: Read + Seek>(reader: R) -> Result<XlsxStyleMetadata> {
    let mut archive = ZipArchive::new(reader).context("failed to inspect xlsx styles")?;
    let root_relationships = read_relationships(&mut archive, "")?;
    let workbook_part = root_relationships
        .values()
        .find(|relationship| relationship.kind.ends_with("/officeDocument"))
        .map(|relationship| relationship.target.clone())
        .unwrap_or_else(|| "xl/workbook.xml".to_string());
    let Some(workbook_xml) = read_zip_part(&mut archive, &workbook_part)? else {
        return Ok(XlsxStyleMetadata::default());
    };
    let workbook = parse_ooxml(&workbook_xml)?;
    let workbook_relationships = read_relationships(&mut archive, &workbook_part)?;

    let styles_part = workbook_relationships
        .values()
        .find(|relationship| relationship.kind.ends_with("/styles"))
        .map(|relationship| relationship.target.clone());
    let theme_part = workbook_relationships
        .values()
        .find(|relationship| relationship.kind.ends_with("/theme"))
        .map(|relationship| relationship.target.clone());
    let theme = match theme_part {
        Some(part) => read_zip_part(&mut archive, &part)?
            .map(|xml| parse_xlsx_theme(&xml))
            .transpose()?,
        None => None,
    };
    let styles = match styles_part {
        Some(part) => read_zip_part(&mut archive, &part)?
            .map(|xml| parse_cell_styles(&xml, theme.as_ref()))
            .transpose()?
            .unwrap_or_default(),
        None => Vec::new(),
    };

    let mut cells_by_sheet = BTreeMap::new();
    if has_non_default_cell_styles(&styles)
        && let Some(sheets) = workbook.child("sheets")
    {
        for sheet in sheets.children_named("sheet") {
            let (Some(name), Some(relationship_id)) = (sheet.attr("name"), sheet.attr("r:id"))
            else {
                continue;
            };
            let Some(relationship) = workbook_relationships.get(relationship_id) else {
                continue;
            };
            if !relationship.kind.ends_with("/worksheet") {
                continue;
            }
            let Some(worksheet_xml) = read_zip_part(&mut archive, &relationship.target)? else {
                continue;
            };
            cells_by_sheet.insert(name.to_string(), parse_worksheet_style_ids(&worksheet_xml)?);
        }
    }

    Ok(XlsxStyleMetadata {
        styles,
        cells_by_sheet,
    })
}

fn has_non_default_cell_styles(styles: &[String]) -> bool {
    let Some(default_style) = styles.first() else {
        return false;
    };
    styles.iter().skip(1).any(|style| style != default_style)
}

fn read_zip_part<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    part: &str,
) -> Result<Option<Vec<u8>>> {
    let mut entry = match archive.by_name(part.trim_start_matches('/')) {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to open xlsx part {part}"));
        }
    };
    let mut bytes = Vec::with_capacity(entry.size().min(usize::MAX as u64) as usize);
    entry
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read xlsx part {part}"))?;
    Ok(Some(bytes))
}

fn read_relationships<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    source_part: &str,
) -> Result<BTreeMap<String, PackageRelationship>> {
    let relationship_part = if source_part.is_empty() {
        "_rels/.rels".to_string()
    } else {
        let (directory, file) = source_part.rsplit_once('/').unwrap_or(("", source_part));
        if directory.is_empty() {
            format!("_rels/{file}.rels")
        } else {
            format!("{directory}/_rels/{file}.rels")
        }
    };
    let Some(xml) = read_zip_part(archive, &relationship_part)? else {
        return Ok(BTreeMap::new());
    };
    let root = parse_ooxml(&xml)?;
    let base_directory = source_part
        .rsplit_once('/')
        .map_or("", |(directory, _)| directory);
    let mut relationships = BTreeMap::new();
    for relationship in root.children_named("Relationship") {
        let (Some(id), Some(target), Some(kind)) = (
            relationship.attr("Id"),
            relationship.attr("Target"),
            relationship.attr("Type"),
        ) else {
            continue;
        };
        if relationship.attr("TargetMode") == Some("External") {
            continue;
        }
        relationships.insert(
            id.to_string(),
            PackageRelationship {
                target: resolve_package_target(base_directory, target),
                kind: kind.to_string(),
            },
        );
    }
    Ok(relationships)
}

fn resolve_package_target(base_directory: &str, target: &str) -> String {
    let mut segments: Vec<&str> = if target.starts_with('/') || base_directory.is_empty() {
        Vec::new()
    } else {
        base_directory.split('/').collect()
    };
    for segment in target.trim_start_matches('/').split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            segment => segments.push(segment),
        }
    }
    segments.join("/")
}

fn parse_ooxml(bytes: &[u8]) -> Result<OoxmlNode> {
    let xml = String::from_utf8_lossy(bytes);
    let mut reader = XmlReader::from_str(&xml);
    reader.config_mut().trim_text(false);
    let mut stack = vec![OoxmlNode::default()];

    loop {
        match reader.read_event().context("failed to parse xlsx XML")? {
            XmlEvent::Start(start) => {
                let mut node = OoxmlNode {
                    name: xml_local_name(start.name().as_ref()),
                    ..Default::default()
                };
                for attribute in start.attributes().flatten() {
                    node.attrs.push((
                        String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
                        attribute.unescape_value().unwrap_or_default().into_owned(),
                    ));
                }
                stack.push(node);
            }
            XmlEvent::Empty(start) => {
                let mut node = OoxmlNode {
                    name: xml_local_name(start.name().as_ref()),
                    ..Default::default()
                };
                for attribute in start.attributes().flatten() {
                    node.attrs.push((
                        String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
                        attribute.unescape_value().unwrap_or_default().into_owned(),
                    ));
                }
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                }
            }
            XmlEvent::End(_) => {
                if stack.len() > 1 {
                    let node = stack.pop().expect("xlsx XML stack depth checked");
                    if let Some(parent) = stack.last_mut() {
                        parent.children.push(node);
                    }
                }
            }
            XmlEvent::Text(text) => {
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&text.xml_content().unwrap_or_default());
                }
            }
            XmlEvent::CData(data) => {
                if let Some(node) = stack.last_mut() {
                    node.text
                        .push_str(&String::from_utf8_lossy(data.into_inner().as_ref()));
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }

    stack
        .remove(0)
        .children
        .into_iter()
        .next()
        .context("xlsx XML document has no root element")
}

fn xml_local_name(raw: &[u8]) -> String {
    let name = String::from_utf8_lossy(raw);
    name.rsplit_once(':')
        .map_or_else(|| name.to_string(), |(_, local)| local.to_string())
}

fn parse_xlsx_theme(xml: &[u8]) -> Result<XlsxTheme> {
    let root = parse_ooxml(xml)?;
    let mut theme = XlsxTheme::default();
    let Some(color_scheme) = root
        .child("themeElements")
        .and_then(|elements| elements.child("clrScheme"))
    else {
        return Ok(theme);
    };
    for color in &color_scheme.children {
        let Some(index) = theme_color_index(&color.name) else {
            continue;
        };
        let Some(value) = color.children.first() else {
            continue;
        };
        theme.colors.insert(index, xlsx_theme_color_value(value));
    }
    Ok(theme)
}

fn xlsx_theme_color_value(color: &OoxmlNode) -> String {
    match color.name.as_str() {
        "srgbClr" => color
            .attr("val")
            .and_then(normalized_argb)
            .unwrap_or_else(|| canonical_ooxml_signature(color)),
        "sysClr" => color
            .attr("lastClr")
            .and_then(normalized_argb)
            .unwrap_or_else(|| canonical_ooxml_signature(color)),
        _ => canonical_ooxml_signature(color),
    }
}

fn normalized_argb(value: &str) -> Option<String> {
    if !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    match value.len() {
        6 => Some(format!("FF{}", value.to_ascii_uppercase())),
        8 => Some(format!("FF{}", value[2..].to_ascii_uppercase())),
        _ => None,
    }
}

const XLSX_DEFAULT_INDEXED_COLORS: [&str; 64] = [
    "FF000000", "FFFFFFFF", "FFFF0000", "FF00FF00", "FF0000FF", "FFFFFF00", "FFFF00FF", "FF00FFFF",
    "FF000000", "FFFFFFFF", "FFFF0000", "FF00FF00", "FF0000FF", "FFFFFF00", "FFFF00FF", "FF00FFFF",
    "FF800000", "FF008000", "FF000080", "FF808000", "FF800080", "FF008080", "FFC0C0C0", "FF808080",
    "FF9999FF", "FF993366", "FFFFFFCC", "FFCCFFFF", "FF660066", "FFFF8080", "FF0066CC", "FFCCCCFF",
    "FF000080", "FFFF00FF", "FFFFFF00", "FF00FFFF", "FF800080", "FF800000", "FF008080", "FF0000FF",
    "FF00CCFF", "FFCCFFFF", "FFCCFFCC", "FFFFFF99", "FF99CCFF", "FFFF99CC", "FFCC99FF", "FFFFCC99",
    "FF3366FF", "FF33CCCC", "FF99CC00", "FFFFCC00", "FFFF9900", "FFFF6600", "FF666699", "FF969696",
    "FF003366", "FF339966", "FF003300", "FF333300", "FF993300", "FF993366", "FF333399", "FF333333",
];

fn xlsx_indexed_color(index: usize, theme: Option<&XlsxTheme>) -> Option<&str> {
    match theme.and_then(|theme| theme.indexed_colors.as_ref()) {
        Some(colors) => colors.get(&index).map(String::as_str),
        None => XLSX_DEFAULT_INDEXED_COLORS.get(index).copied(),
    }
}

fn parse_xlsx_indexed_colors(root: &OoxmlNode) -> Option<BTreeMap<usize, String>> {
    let colors = root.child("colors")?.child("indexedColors")?;
    Some(
        colors
            .children_named("rgbColor")
            .enumerate()
            .filter_map(|(index, color)| Some((index, normalized_argb(color.attr("rgb")?)?)))
            .collect(),
    )
}

fn apply_xlsx_tint(value: &str, tint: f64) -> Option<String> {
    if !(-1.0..=1.0).contains(&tint) {
        return None;
    }
    let argb = normalized_argb(value)?;
    let alpha = &argb[..2];
    let channel = |range: std::ops::Range<usize>| {
        u8::from_str_radix(&argb[range], 16)
            .ok()
            .map(|value| f64::from(value) / 255.0)
    };
    let red = channel(2..4)?;
    let green = channel(4..6)?;
    let blue = channel(6..8)?;
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    let mut luminance = (max + min) / 2.0;
    let (hue, saturation) = if (max - min).abs() < f64::EPSILON {
        (0.0, 0.0)
    } else {
        let delta = max - min;
        let saturation = if luminance > 0.5 {
            delta / (2.0 - max - min)
        } else {
            delta / (max + min)
        };
        let hue = if (max - red).abs() < f64::EPSILON {
            (green - blue) / delta + if green < blue { 6.0 } else { 0.0 }
        } else if (max - green).abs() < f64::EPSILON {
            (blue - red) / delta + 2.0
        } else {
            (red - green) / delta + 4.0
        } / 6.0;
        (hue, saturation)
    };
    luminance = if tint < 0.0 {
        luminance * (1.0 + tint)
    } else {
        luminance * (1.0 - tint) + tint
    };
    let hue_to_rgb = |mut offset: f64| {
        if offset < 0.0 {
            offset += 1.0;
        }
        if offset > 1.0 {
            offset -= 1.0;
        }
        let high = if luminance < 0.5 {
            luminance * (1.0 + saturation)
        } else {
            luminance + saturation - luminance * saturation
        };
        let low = 2.0 * luminance - high;
        let value = if offset < 1.0 / 6.0 {
            low + (high - low) * 6.0 * offset
        } else if offset < 0.5 {
            high
        } else if offset < 2.0 / 3.0 {
            low + (high - low) * (2.0 / 3.0 - offset) * 6.0
        } else {
            low
        };
        (value * 255.0 + 1e-9).round().clamp(0.0, 255.0) as u8
    };
    let (red, green, blue) = if saturation.abs() < f64::EPSILON {
        let value = (luminance * 255.0 + 1e-9).round().clamp(0.0, 255.0) as u8;
        (value, value, value)
    } else {
        (
            hue_to_rgb(hue + 1.0 / 3.0),
            hue_to_rgb(hue),
            hue_to_rgb(hue - 1.0 / 3.0),
        )
    };
    Some(format!("{alpha}{red:02X}{green:02X}{blue:02X}"))
}

fn theme_color_index(name: &str) -> Option<usize> {
    Some(match name {
        "dk1" => 0,
        "lt1" => 1,
        "dk2" => 2,
        "lt2" => 3,
        "accent1" => 4,
        "accent2" => 5,
        "accent3" => 6,
        "accent4" => 7,
        "accent5" => 8,
        "accent6" => 9,
        "hlink" => 10,
        "folHlink" => 11,
        _ => return None,
    })
}

fn parse_cell_styles(xml: &[u8], theme: Option<&XlsxTheme>) -> Result<Vec<String>> {
    let root = parse_ooxml(xml)?;
    let mut color_context = theme.cloned().unwrap_or_default();
    color_context.indexed_colors = parse_xlsx_indexed_colors(&root);
    let color_context = Some(&color_context);
    let fonts = style_components(&root, "fonts", "font", color_context);
    let fills = style_components(&root, "fills", "fill", color_context);
    let borders = style_components(&root, "borders", "border", color_context);
    let number_formats: BTreeMap<String, String> = root
        .child("numFmts")
        .into_iter()
        .flat_map(|formats| formats.children_named("numFmt"))
        .filter_map(|format| {
            Some((
                format.attr("numFmtId")?.to_string(),
                format.attr("formatCode")?.to_string(),
            ))
        })
        .collect();
    let style_xfs: Vec<&OoxmlNode> = root
        .child("cellStyleXfs")
        .into_iter()
        .flat_map(|xfs| xfs.children_named("xf"))
        .collect();
    let mut styles: Vec<String> = root
        .child("cellXfs")
        .into_iter()
        .flat_map(|xfs| xfs.children_named("xf"))
        .map(|xf| canonical_cell_style(xf, &style_xfs, &fonts, &fills, &borders, &number_formats))
        .collect();
    if styles.is_empty() {
        styles.push(String::new());
    }
    Ok(styles)
}

fn style_components(
    root: &OoxmlNode,
    container: &str,
    child: &str,
    theme: Option<&XlsxTheme>,
) -> Vec<String> {
    root.child(container)
        .into_iter()
        .flat_map(|container| container.children_named(child))
        .map(|component| canonical_ooxml_signature_with_theme(component, theme))
        .collect()
}

fn canonical_cell_style(
    xf: &OoxmlNode,
    style_xfs: &[&OoxmlNode],
    fonts: &[String],
    fills: &[String],
    borders: &[String],
    number_formats: &BTreeMap<String, String>,
) -> String {
    let base = xf
        .attr("xfId")
        .and_then(|id| id.parse::<usize>().ok())
        .and_then(|id| style_xfs.get(id).copied());
    let font_id = effective_style_id(xf, base, "fontId", "applyFont");
    let fill_id = effective_style_id(xf, base, "fillId", "applyFill");
    let border_id = effective_style_id(xf, base, "borderId", "applyBorder");
    let number_format_id = effective_style_id(xf, base, "numFmtId", "applyNumberFormat");
    let mut parts = vec![
        format!("font={}", style_component(fonts, font_id)),
        format!("fill={}", style_component(fills, fill_id)),
        format!("border={}", style_component(borders, border_id)),
        format!(
            "number_format={}",
            number_formats
                .get(&number_format_id.to_string())
                .map_or_else(
                    || format!("builtin:{number_format_id}"),
                    |code| { format!("custom:{code}") }
                )
        ),
    ];

    for child_name in ["alignment", "protection"] {
        let child = if style_property_applied(xf, child_name) {
            xf.child(child_name)
                .or_else(|| base.and_then(|base| base.child(child_name)))
        } else {
            base.and_then(|base| base.child(child_name))
        };
        if let Some(child) = child
            && let Some(signature) = canonical_cell_style_property(child_name, child)
        {
            parts.push(signature);
        }
    }
    for attribute_name in ["quotePrefix", "pivotButton"] {
        let value = xf
            .attr(attribute_name)
            .or_else(|| base.and_then(|base| base.attr(attribute_name)));
        if value.is_some_and(|value| !matches!(value, "0" | "false")) {
            parts.push(format!("{attribute_name}=1"));
        }
    }
    parts.join("|")
}

fn canonical_cell_style_property(name: &str, node: &OoxmlNode) -> Option<String> {
    let mut normalized = node.clone();
    normalized.attrs = normalized
        .attrs
        .into_iter()
        .filter_map(|(attribute, mut value)| {
            let local_attribute = xml_local_name(attribute.as_bytes());
            if let Some(default) = cell_style_boolean_default(name, &local_attribute)
                && let Some(enabled) = xml_boolean_value(&value)
            {
                if enabled == default {
                    return None;
                }
                value = if enabled { "1" } else { "0" }.to_string();
            }
            if name == "alignment" {
                if matches!(
                    (local_attribute.as_str(), value.as_str()),
                    ("horizontal", "general") | ("vertical", "bottom")
                ) {
                    return None;
                }
                if matches!(
                    local_attribute.as_str(),
                    "textRotation" | "indent" | "relativeIndent" | "readingOrder"
                ) && let Some(integer) = canonical_xml_integer(&value)
                {
                    if integer == "0" {
                        return None;
                    }
                    value = integer;
                }
            }
            Some((attribute, value))
        })
        .collect();
    if normalized.attrs.is_empty()
        && normalized.children.is_empty()
        && normalized.text.trim().is_empty()
    {
        None
    } else {
        Some(canonical_ooxml_signature(&normalized))
    }
}

fn cell_style_boolean_default(container: &str, attribute: &str) -> Option<bool> {
    match (container, attribute) {
        ("alignment", "wrapText" | "shrinkToFit" | "justifyLastLine" | "mergeCell")
        | ("protection", "hidden") => Some(false),
        ("protection", "locked") => Some(true),
        _ => None,
    }
}

fn xml_boolean_value(value: &str) -> Option<bool> {
    match value {
        "1" | "true" => Some(true),
        "0" | "false" => Some(false),
        _ => None,
    }
}

fn canonical_xml_integer(value: &str) -> Option<String> {
    value.parse::<i64>().ok().map(|value| value.to_string())
}

fn canonical_xml_double(value: &str) -> Option<String> {
    let value = value.parse::<f64>().ok()?;
    value.is_finite().then(|| {
        if value == 0.0 {
            "0".to_string()
        } else {
            value.to_string()
        }
    })
}

fn effective_style_id(
    xf: &OoxmlNode,
    base: Option<&OoxmlNode>,
    id_attribute: &str,
    apply_attribute: &str,
) -> usize {
    let local_applied = !matches!(xf.attr(apply_attribute), Some("0" | "false"));
    let value = if local_applied {
        xf.attr(id_attribute)
            .or_else(|| base.and_then(|base| base.attr(id_attribute)))
    } else {
        base.and_then(|base| base.attr(id_attribute))
    };
    value.and_then(|value| value.parse().ok()).unwrap_or(0)
}

fn style_property_applied(xf: &OoxmlNode, child_name: &str) -> bool {
    let apply_attribute = match child_name {
        "alignment" => "applyAlignment",
        "protection" => "applyProtection",
        _ => return true,
    };
    !matches!(xf.attr(apply_attribute), Some("0" | "false"))
}

fn style_component(components: &[String], index: usize) -> String {
    components
        .get(index)
        .cloned()
        .unwrap_or_else(|| format!("missing:{index}"))
}

fn canonical_ooxml_signature(node: &OoxmlNode) -> String {
    canonical_ooxml_signature_with_theme(node, None)
}

fn spreadsheet_font_boolean_state(node: &OoxmlNode) -> Option<bool> {
    if !matches!(
        node.name.as_str(),
        "b" | "i" | "strike" | "outline" | "shadow" | "condense" | "extend"
    ) {
        return None;
    }
    node.attr("val").map_or(Some(true), xml_boolean_value)
}

fn canonical_ooxml_signature_with_theme(node: &OoxmlNode, theme: Option<&XlsxTheme>) -> String {
    let spreadsheet_font_boolean = spreadsheet_font_boolean_state(node);
    let is_color = matches!(node.name.as_str(), "color" | "fgColor" | "bgColor");
    let resolved_theme_color = is_color
        .then(|| node.attr("theme"))
        .flatten()
        .and_then(|value| value.parse::<usize>().ok())
        .and_then(|index| theme.and_then(|theme| theme.colors.get(&index)));
    let resolved_indexed_color = is_color
        .then(|| node.attr("indexed"))
        .flatten()
        .and_then(|value| value.parse::<usize>().ok())
        .and_then(|index| xlsx_indexed_color(index, theme));
    let resolved_color = resolved_theme_color
        .map(String::as_str)
        .or(resolved_indexed_color);
    let tint = node
        .attr("tint")
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite());
    let tinted_color = tint.and_then(|tint| {
        resolved_color
            .or_else(|| node.attr("rgb"))
            .and_then(|color| apply_xlsx_tint(color, tint))
    });
    let mut attributes: Vec<(String, String)> = node
        .attrs
        .iter()
        .filter(|(name, _)| name != "xmlns" && !name.starts_with("xmlns:") && name != "count")
        .filter_map(|(name, value)| {
            let name = xml_local_name(name.as_bytes());
            if name == "val" && spreadsheet_font_boolean == Some(true) {
                return None;
            }
            if name == "theme" && resolved_theme_color.is_some() {
                return None;
            }
            if name == "indexed" && resolved_indexed_color.is_some() {
                return None;
            }
            if tinted_color.is_some() && matches!(name.as_str(), "rgb" | "tint") {
                return None;
            }
            if name == "tint"
                && let Some(value) = canonical_xml_double(value)
            {
                if value == "0" {
                    return None;
                }
                return Some((name, value));
            }
            let value = if node.name == "sz" && name == "val" {
                canonical_xml_double(value).unwrap_or_else(|| value.clone())
            } else if name == "rgb" {
                normalized_argb(value).unwrap_or_else(|| value.to_ascii_uppercase())
            } else {
                value.clone()
            };
            Some((name, value))
        })
        .collect();
    if let Some(color) = tinted_color.as_deref().or(resolved_color) {
        let (name, value) = normalized_argb(color).map_or_else(
            || ("resolved".to_string(), color.to_string()),
            |color| ("rgb".to_string(), color),
        );
        attributes.push((name, value));
    }
    attributes.sort();
    let mut signature = node.name.clone();
    for (name, value) in attributes {
        signature.push('|');
        signature.push_str(&name);
        signature.push('=');
        signature.push_str(&value);
    }
    for child in &node.children {
        if spreadsheet_font_boolean_state(child) == Some(false) {
            continue;
        }
        let child_signature = canonical_ooxml_signature_with_theme(child, theme);
        signature.push('{');
        signature.push_str(&child_signature.len().to_string());
        signature.push(':');
        signature.push_str(&child_signature);
        signature.push('}');
    }
    let text = node.text.trim();
    if !text.is_empty() {
        signature.push('[');
        signature.push_str(text);
        signature.push(']');
    }
    signature
}

fn parse_worksheet_style_ids(xml: &[u8]) -> Result<BTreeMap<(usize, usize), usize>> {
    let mut reader = XmlReader::from_reader(xml);
    let mut styles = BTreeMap::new();
    loop {
        match reader
            .read_event()
            .context("failed to parse xlsx worksheet styles")?
        {
            XmlEvent::Start(start) | XmlEvent::Empty(start) => {
                let Some((position, style)) = worksheet_cell_style(&start) else {
                    continue;
                };
                if style != 0 {
                    styles.insert(position, style);
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }
    Ok(styles)
}

fn worksheet_cell_style(start: &XmlStart<'_>) -> Option<((usize, usize), usize)> {
    if xml_local_name(start.name().as_ref()) != "c" {
        return None;
    }
    let mut address = None;
    let mut style = None;
    for attribute in start.attributes().flatten() {
        match xml_local_name(attribute.key.as_ref()).as_str() {
            "r" => {
                address = attribute
                    .unescape_value()
                    .ok()
                    .map(|value| value.into_owned())
            }
            "s" => {
                style = attribute
                    .unescape_value()
                    .ok()
                    .and_then(|value| value.parse().ok())
            }
            _ => {}
        }
        if address.is_some() && style.is_some() {
            break;
        }
    }
    match (address, style) {
        (Some(address), Some(style)) => {
            let position = parse_cell_position(&address)?;
            Some((position, style))
        }
        _ => None,
    }
}

fn parse_cell_position(address: &str) -> Option<(usize, usize)> {
    let split = address.find(|character: char| character.is_ascii_digit())?;
    let column = column_index(&address[..split])?;
    let row = address[split..].parse::<usize>().ok()?;
    (row > 0 && column > 0).then_some((row, column))
}

fn apply_xlsx_styles(workbook: &mut WorkbookIr, metadata: &XlsxStyleMetadata) {
    let default_style = metadata.styles.first().cloned().unwrap_or_default();
    workbook.default_style_signature = default_style.clone();
    workbook.explicit_cell_styles.clear();
    for sheet in &mut workbook.sheets {
        let explicit = metadata.cells_by_sheet.get(&sheet.name);
        if let Some(explicit) = explicit {
            let resolved: BTreeMap<(usize, usize), String> = explicit
                .iter()
                .filter_map(|(position, style)| {
                    let signature = metadata.styles.get(*style)?;
                    (signature != &default_style).then(|| (*position, signature.clone()))
                })
                .collect();
            if !resolved.is_empty() {
                workbook
                    .explicit_cell_styles
                    .insert(sheet.name.clone(), resolved);
            }
        }
        for (row_offset, row) in sheet.rows.iter_mut().enumerate() {
            for (column_offset, cell) in row.iter_mut().enumerate() {
                let position = (
                    sheet.start_row + row_offset,
                    sheet.start_col + column_offset,
                );
                cell.style_signature = explicit
                    .and_then(|styles| styles.get(&position))
                    .and_then(|style| metadata.styles.get(*style))
                    .filter(|signature| *signature != &default_style)
                    .cloned()
                    .unwrap_or_default();
            }
        }
    }
}

fn load_workbook_from_xlsx<RS>(workbook: &mut Xlsx<RS>, options: &DiffOptions) -> Result<WorkbookIr>
where
    RS: Read + Seek,
{
    let mut sheets = Vec::new();
    let mut total_cells = 0usize;

    for name in workbook.sheet_names().to_owned() {
        let merged_ranges = workbook
            .merge_cells_by_sheet_name(&name)
            .with_context(|| format!("failed to read merged regions from worksheet `{name}`"))?
            .iter()
            .map(CellRange::from)
            .collect();
        let mut cell_reader = match workbook.worksheet_cells_reader(&name) {
            Ok(reader) => reader,
            Err(CalamineXlsxError::NotAWorksheet(_)) => {
                sheets.push(SheetIr {
                    name,
                    start_row: 1,
                    start_col: 1,
                    rows: Vec::new(),
                    merged_ranges,
                });
                continue;
            }
            Err(error) => {
                return Err(error).with_context(|| format!("failed to read worksheet `{name}`"));
            }
        };
        let mut dense_rows = DenseRowsBuilder::new(
            cell_reader.dimensions(),
            options.limits.max_rows_per_sheet,
            options.limits.max_columns_per_sheet,
            options.limits.max_cells.saturating_sub(total_cells),
        );
        while let Some(record) = cell_reader
            .next_cell_with_formula()
            .with_context(|| format!("failed to read worksheet `{name}`"))?
        {
            let formula = normalize_formula(record.formula);
            if record.value.is_empty() && formula.is_none() {
                continue;
            }
            let position = record.pos;
            let mut cell = cell_ir_from_data_ref(record.value, options.whitespace);
            cell.formula = formula;
            dense_rows
                .push(position, cell)
                .with_context(|| format!("failed to load worksheet `{name}`"))?;
        }
        drop(cell_reader);

        let Some((mut rows, ((start_row, start_col), (end_row, end_col)))) = dense_rows.finish()
        else {
            sheets.push(SheetIr {
                name,
                start_row: 1,
                start_col: 1,
                rows: Vec::new(),
                merged_ranges,
            });
            continue;
        };
        let height = (end_row - start_row + 1) as usize;
        let width = (end_col - start_col + 1) as usize;
        anyhow::ensure!(
            height <= options.limits.max_rows_per_sheet,
            "worksheet `{name}` has {height} rows, exceeding max_rows_per_sheet {}",
            options.limits.max_rows_per_sheet
        );
        anyhow::ensure!(
            width <= options.limits.max_columns_per_sheet,
            "worksheet `{name}` has {width} columns, exceeding max_columns_per_sheet {}",
            options.limits.max_columns_per_sheet
        );
        let sheet_cells = height
            .checked_mul(width)
            .context("worksheet cell count overflow")?;
        total_cells = total_cells
            .checked_add(sheet_cells)
            .context("workbook cell count overflow")?;
        anyhow::ensure!(
            total_cells <= options.limits.max_cells,
            "workbook has more than max_cells {}",
            options.limits.max_cells
        );
        let mut absolute_start_row = start_row as usize + 1;
        let mut absolute_start_col = start_col as usize + 1;
        trim_empty_edges(&mut rows, &mut absolute_start_row, &mut absolute_start_col);
        sheets.push(SheetIr {
            name,
            start_row: absolute_start_row,
            start_col: absolute_start_col,
            rows,
            merged_ranges,
        });
    }

    Ok(WorkbookIr {
        sheets,
        default_style_signature: String::new(),
        explicit_cell_styles: BTreeMap::new(),
    })
}

type CellBounds = ((u32, u32), (u32, u32));

struct DenseRowsBuilder {
    rows: Vec<Vec<CellIr>>,
    start: Option<(u32, u32)>,
    end: Option<(u32, u32)>,
    row_capacity_hint: usize,
    width_hint: usize,
    max_rows: usize,
    max_columns: usize,
    max_cells: usize,
}

impl DenseRowsBuilder {
    fn new(dimensions: Dimensions, max_rows: usize, max_columns: usize, max_cells: usize) -> Self {
        let height = dimensions
            .end
            .0
            .checked_sub(dimensions.start.0)
            .and_then(|height| height.checked_add(1))
            .and_then(|height| usize::try_from(height).ok());
        let width = dimensions
            .end
            .1
            .checked_sub(dimensions.start.1)
            .and_then(|width| width.checked_add(1))
            .and_then(|width| usize::try_from(width).ok());
        let (row_capacity_hint, width_hint) = match (height, width) {
            (Some(height), Some(width))
                if height <= max_rows
                    && width <= max_columns
                    && height
                        .checked_mul(width)
                        .is_some_and(|cells| cells <= max_cells) =>
            {
                (height, width)
            }
            _ => (0, 0),
        };
        Self {
            rows: Vec::new(),
            start: None,
            end: None,
            row_capacity_hint,
            width_hint,
            max_rows,
            max_columns,
            max_cells,
        }
    }

    fn push(&mut self, position: (u32, u32), cell: CellIr) -> Result<()> {
        let candidate_start = self.start.map_or(position, |start| {
            (start.0.min(position.0), start.1.min(position.1))
        });
        let candidate_end = self.end.map_or(position, |end| {
            (end.0.max(position.0), end.1.max(position.1))
        });
        let height = candidate_end
            .0
            .checked_sub(candidate_start.0)
            .and_then(|height| height.checked_add(1))
            .and_then(|height| usize::try_from(height).ok())
            .context("worksheet row count overflow")?;
        let width = candidate_end
            .1
            .checked_sub(candidate_start.1)
            .and_then(|width| width.checked_add(1))
            .and_then(|width| usize::try_from(width).ok())
            .context("worksheet column count overflow")?;
        anyhow::ensure!(
            height <= self.max_rows,
            "worksheet has {height} rows, exceeding max_rows_per_sheet {}",
            self.max_rows
        );
        anyhow::ensure!(
            width <= self.max_columns,
            "worksheet has {width} columns, exceeding max_columns_per_sheet {}",
            self.max_columns
        );
        let cells = height
            .checked_mul(width)
            .context("worksheet cell count overflow")?;
        anyhow::ensure!(
            cells <= self.max_cells,
            "workbook has more than max_cells {}",
            self.max_cells
        );

        let (mut start_row, mut start_col) = match self.start {
            Some(start) => start,
            None => {
                self.rows = Vec::with_capacity(self.row_capacity_hint.max(1));
                self.start = Some(position);
                self.end = Some(position);
                position
            }
        };

        if position.0 < start_row {
            let additional_rows = (start_row - position.0) as usize;
            let mut expanded = Vec::with_capacity(self.rows.len() + additional_rows);
            expanded.resize_with(additional_rows, Vec::new);
            expanded.append(&mut self.rows);
            self.rows = expanded;
            start_row = position.0;
        }
        if position.1 < start_col {
            let additional_columns = (start_col - position.1) as usize;
            for row in &mut self.rows {
                let mut expanded = Vec::with_capacity(row.len() + additional_columns);
                expanded.resize_with(additional_columns, CellIr::default);
                expanded.append(row);
                *row = expanded;
            }
            start_col = position.1;
        }
        self.start = Some((start_row, start_col));

        let row_offset = (position.0 - start_row) as usize;
        while self.rows.len() <= row_offset {
            self.rows.push(Vec::with_capacity(self.width_hint));
        }
        let column_offset = (position.1 - start_col) as usize;
        let row = &mut self.rows[row_offset];
        if row.len() <= column_offset {
            row.resize_with(column_offset, CellIr::default);
            row.push(cell);
        } else {
            row[column_offset] = cell;
        }

        let end = self.end.get_or_insert(position);
        end.0 = end.0.max(position.0);
        end.1 = end.1.max(position.1);
        Ok(())
    }

    fn finish(mut self) -> Option<(Vec<Vec<CellIr>>, CellBounds)> {
        let start = self.start?;
        let end = self.end?;
        let height = (end.0 - start.0 + 1) as usize;
        let width = (end.1 - start.1 + 1) as usize;
        self.rows.resize_with(height, Vec::new);
        for row in &mut self.rows {
            row.resize_with(width, CellIr::default);
        }
        Some((self.rows, (start, end)))
    }
}

fn normalize_formula(formula: Option<String>) -> Option<String> {
    formula.and_then(|formula| {
        let normalized = formula.trim();
        if normalized.is_empty() {
            None
        } else if normalized.starts_with('=') {
            Some(normalized.to_string())
        } else {
            Some(format!("={normalized}"))
        }
    })
}

fn cell_ir_from_data_ref(value: DataRef<'_>, whitespace: WhitespaceNormalization) -> CellIr {
    let (value, value_kind) = match value {
        DataRef::Empty => (String::new(), CellValueKind::Empty),
        DataRef::String(value) => (normalize_text(&value, whitespace), CellValueKind::String),
        DataRef::SharedString(value) => (normalize_text(value, whitespace), CellValueKind::String),
        DataRef::Float(value) => (normalize_number(value), CellValueKind::Number),
        DataRef::Int(value) => (value.to_string(), CellValueKind::Number),
        DataRef::Bool(value) => (value.to_string(), CellValueKind::Boolean),
        DataRef::Error(value) => (format!("{value:?}"), CellValueKind::Error),
        DataRef::DateTime(value) if value.is_duration() => (
            normalize_excel_duration(value.as_f64()),
            CellValueKind::Duration,
        ),
        DataRef::DateTime(value) => (normalize_excel_datetime(&value), CellValueKind::DateTime),
        DataRef::DateTimeIso(value) => {
            let value = Data::DateTimeIso(value);
            (
                normalize_iso_datetime(&value).unwrap_or_else(|| {
                    normalize_text(value.get_datetime_iso().unwrap_or(""), whitespace)
                }),
                CellValueKind::DateTime,
            )
        }
        DataRef::DurationIso(value) => {
            let value = Data::DurationIso(value);
            (
                normalize_iso_duration(&value).unwrap_or_else(|| {
                    normalize_text(value.get_duration_iso().unwrap_or(""), whitespace)
                }),
                CellValueKind::Duration,
            )
        }
    };
    CellIr {
        value,
        value_kind,
        formula: None,
        style_signature: String::new(),
    }
}

pub fn diff_workbooks(
    old_path: &Path,
    new_path: &Path,
    old_book: &WorkbookIr,
    new_book: &WorkbookIr,
) -> Result<WorkbookDiff> {
    diff_workbooks_with_options(
        old_path,
        new_path,
        old_book,
        new_book,
        &DiffOptions::default(),
    )
}

/// Compares two loaded workbooks using explicit alignment options.
pub fn diff_workbooks_with_options(
    old_path: &Path,
    new_path: &Path,
    old_book: &WorkbookIr,
    new_book: &WorkbookIr,
    options: &DiffOptions,
) -> Result<WorkbookDiff> {
    options.validate()?;
    validate_workbook_ir(old_book, &options.limits, "old")?;
    validate_workbook_ir(new_book, &options.limits, "new")?;
    let sheet_matches = match_sheets(old_book, new_book, options)?;
    let mut workbook_mapper = WorkbookCoordinateMapper::default();
    let mut preliminary_areas = HashMap::new();
    for op in &sheet_matches {
        if let MatchOp::Match(old_idx, new_idx, _) = *op {
            let old_sheet = &old_book.sheets[old_idx];
            let new_sheet = &new_book.sheets[new_idx];
            let preliminary = align_area(
                &old_sheet.rows,
                &new_sheet.rows,
                (old_sheet.start_row - 1, old_sheet.start_col - 1),
                (new_sheet.start_row - 1, new_sheet.start_col - 1),
                options,
            )?;
            workbook_mapper.sheets.insert(
                normalize_sheet_name(&old_sheet.name),
                SheetCoordinateMapping {
                    new_name: new_sheet.name.clone(),
                    mapper: preliminary.coordinate_mapper.clone(),
                },
            );
            preliminary_areas.insert((old_idx, new_idx), preliminary);
        }
    }
    let reordered = reordered_sheet_matches(&sheet_matches);
    let mut summary = DiffSummary::default();
    let mut sheets = Vec::new();

    for op in sheet_matches {
        match op {
            MatchOp::Match(old_idx, new_idx, confidence) => {
                let old_sheet = &old_book.sheets[old_idx];
                let new_sheet = &new_book.sheets[new_idx];
                let mut sheet_area = preliminary_areas
                    .remove(&(old_idx, new_idx))
                    .context("missing preliminary worksheet alignment")?;
                sheet_area.cell_changes = compare_mapped_cells(
                    &old_sheet.rows,
                    &new_sheet.rows,
                    options,
                    &sheet_area.coordinate_mapper,
                    AreaContext {
                        old_offset: (old_sheet.start_row - 1, old_sheet.start_col - 1),
                        new_offset: (new_sheet.start_row - 1, new_sheet.start_col - 1),
                        workbook_mapper: Some(&workbook_mapper),
                        style_defaults: StyleDefaults {
                            old: &old_book.default_style_signature,
                            new: &new_book.default_style_signature,
                        },
                    },
                );
                let mut diff = build_sheet_diff_from_area(
                    old_sheet,
                    new_sheet,
                    options,
                    Some(&workbook_mapper),
                    StyleDefaults {
                        old: &old_book.default_style_signature,
                        new: &new_book.default_style_signature,
                    },
                    sheet_area,
                )?;
                append_unmaterialized_style_changes(
                    &mut diff,
                    old_sheet,
                    new_sheet,
                    old_book.explicit_cell_styles.get(&old_sheet.name),
                    new_book.explicit_cell_styles.get(&new_sheet.name),
                    &old_book.default_style_signature,
                    &new_book.default_style_signature,
                );
                diff.old_position = Some(old_idx + 1);
                diff.new_position = Some(new_idx + 1);
                diff.order_changed = reordered.contains(&(old_idx, new_idx));
                diff.match_confidence = Some(confidence);
                if diff.order_changed {
                    summary.sheet_order_changes += 1;
                }
                if !sheet_names_equivalent(&old_sheet.name, &new_sheet.name) {
                    diff.status = SheetStatus::Renamed;
                    summary.sheets_renamed += 1;
                } else if diff.order_changed && matches!(diff.status, SheetStatus::Unchanged) {
                    diff.status = SheetStatus::Reordered;
                }
                accumulate_summary(&mut summary, &diff);
                sheets.push(diff);
            }
            MatchOp::Removed(old_idx) => {
                summary.sheets_removed += 1;
                let sheet_diff = removed_sheet_diff(&old_book.sheets[old_idx], old_idx + 1);
                accumulate_summary(&mut summary, &sheet_diff);
                sheets.push(sheet_diff);
            }
            MatchOp::Added(new_idx) => {
                summary.sheets_added += 1;
                let sheet_diff = added_sheet_diff(&new_book.sheets[new_idx], new_idx + 1);
                accumulate_summary(&mut summary, &sheet_diff);
                sheets.push(sheet_diff);
            }
        }
    }

    Ok(WorkbookDiff {
        schema_version: DIFF_SCHEMA_VERSION,
        coordinate_system: CoordinateSystem::OneBased,
        options: options.clone(),
        old_file: old_path.display().to_string(),
        new_file: new_path.display().to_string(),
        summary,
        sheets,
    })
}

fn validate_workbook_ir(workbook: &WorkbookIr, limits: &ResourceLimits, label: &str) -> Result<()> {
    let mut total_cells = 0usize;
    for sheet in &workbook.sheets {
        anyhow::ensure!(
            sheet.start_row > 0 && sheet.start_col > 0,
            "{label} worksheet `{}` must use one-based start coordinates",
            sheet.name
        );
        anyhow::ensure!(
            sheet.rows.len() <= limits.max_rows_per_sheet,
            "{label} worksheet `{}` exceeds max_rows_per_sheet {}",
            sheet.name,
            limits.max_rows_per_sheet
        );
        let width = sheet.rows.iter().map(Vec::len).max().unwrap_or(0);
        anyhow::ensure!(
            width <= limits.max_columns_per_sheet,
            "{label} worksheet `{}` exceeds max_columns_per_sheet {}",
            sheet.name,
            limits.max_columns_per_sheet
        );
        total_cells = total_cells
            .checked_add(
                sheet
                    .rows
                    .len()
                    .checked_mul(width)
                    .context("worksheet cell count overflow")?,
            )
            .context("workbook cell count overflow")?;
        for range in &sheet.merged_ranges {
            anyhow::ensure!(
                range.start_row > 0
                    && range.start_col > 0
                    && range.start_row <= range.end_row
                    && range.start_col <= range.end_col,
                "{label} worksheet `{}` contains an invalid merged range",
                sheet.name
            );
        }
    }
    anyhow::ensure!(
        total_cells <= limits.max_cells,
        "{label} workbook exceeds max_cells {}",
        limits.max_cells
    );
    Ok(())
}

pub fn diff_workbook_bytes(
    old_name: &str,
    new_name: &str,
    old_bytes: &[u8],
    new_bytes: &[u8],
) -> Result<WorkbookDiff> {
    diff_workbook_bytes_with_options(
        old_name,
        new_name,
        old_bytes,
        new_bytes,
        &DiffOptions::default(),
    )
}

/// Loads and compares two in-memory workbooks with explicit options.
pub fn diff_workbook_bytes_with_options(
    old_name: &str,
    new_name: &str,
    old_bytes: &[u8],
    new_bytes: &[u8],
    options: &DiffOptions,
) -> Result<WorkbookDiff> {
    let old_book = load_workbook_from_bytes_with_options(old_bytes, options)
        .with_context(|| format!("failed to load old workbook `{old_name}`"))?;
    let new_book = load_workbook_from_bytes_with_options(new_bytes, options)
        .with_context(|| format!("failed to load new workbook `{new_name}`"))?;
    diff_workbooks_with_options(
        Path::new(old_name),
        Path::new(new_name),
        &old_book,
        &new_book,
        options,
    )
}

fn match_sheets(
    old_book: &WorkbookIr,
    new_book: &WorkbookIr,
    options: &DiffOptions,
) -> Result<Vec<MatchOp>> {
    let mut ops = Vec::new();
    let mut matched_old = BTreeSet::new();
    let mut matched_new = BTreeSet::new();
    let new_by_name: HashMap<&str, usize> = new_book
        .sheets
        .iter()
        .enumerate()
        .map(|(idx, sheet)| (sheet.name.as_str(), idx))
        .collect();

    for (old_idx, old_sheet) in old_book.sheets.iter().enumerate() {
        if let Some(&new_idx) = new_by_name.get(old_sheet.name.as_str()) {
            matched_old.insert(old_idx);
            matched_new.insert(new_idx);
            ops.push(MatchOp::Match(old_idx, new_idx, 1.0));
        }
    }

    // OOXML producers can serialize a visually identical sheet name in NFC or NFD.
    // Exact matches stay first; canonical matching is used only for a unique
    // remaining candidate on both sides, avoiding normalization collisions.
    let mut remaining_old_counts: HashMap<String, usize> = HashMap::new();
    for (old_idx, sheet) in old_book.sheets.iter().enumerate() {
        if !matched_old.contains(&old_idx) {
            *remaining_old_counts
                .entry(canonical_sheet_identity(&sheet.name))
                .or_default() += 1;
        }
    }
    for (old_idx, old_sheet) in old_book.sheets.iter().enumerate() {
        if matched_old.contains(&old_idx) {
            continue;
        }
        let identity = canonical_sheet_identity(&old_sheet.name);
        if remaining_old_counts.get(&identity) != Some(&1) {
            continue;
        }
        let candidates: Vec<usize> = new_book
            .sheets
            .iter()
            .enumerate()
            .filter(|(new_idx, sheet)| {
                !matched_new.contains(new_idx) && canonical_sheet_identity(&sheet.name) == identity
            })
            .map(|(new_idx, _)| new_idx)
            .collect();
        if let [new_idx] = candidates.as_slice() {
            matched_old.insert(old_idx);
            matched_new.insert(*new_idx);
            ops.push(MatchOp::Match(old_idx, *new_idx, 1.0));
        }
    }

    let unmatched_old: Vec<_> = (0..old_book.sheets.len())
        .filter(|idx| !matched_old.contains(idx))
        .collect();
    let unmatched_new: Vec<_> = (0..new_book.sheets.len())
        .filter(|idx| !matched_new.contains(idx))
        .collect();
    let rename_pair_count = unmatched_old
        .len()
        .checked_mul(unmatched_new.len())
        .context("sheet rename pairing size overflow")?;
    anyhow::ensure!(
        rename_pair_count <= options.limits.max_alignment_pairs,
        "sheet rename pairing {rename_pair_count} exceeds max_alignment_pairs {}",
        options.limits.max_alignment_pairs
    );
    let scores: Vec<Vec<_>> = unmatched_old
        .iter()
        .map(|old_idx| {
            unmatched_new
                .iter()
                .map(|new_idx| {
                    sheet_similarity(&old_book.sheets[*old_idx], &new_book.sheets[*new_idx])
                })
                .collect()
        })
        .collect();
    for (old_local, matched) in maximum_weight_assignment(
        &scores,
        options.sheet_rename_score,
        options.limits.max_assignment_size,
    )?
    .into_iter()
    .enumerate()
    {
        if let Some((new_local, score)) = matched {
            let old_idx = unmatched_old[old_local];
            let new_idx = unmatched_new[new_local];
            matched_old.insert(old_idx);
            matched_new.insert(new_idx);
            ops.push(MatchOp::Match(old_idx, new_idx, score));
        }
    }

    for old_idx in 0..old_book.sheets.len() {
        if !matched_old.contains(&old_idx) {
            ops.push(MatchOp::Removed(old_idx));
        }
    }
    for new_idx in 0..new_book.sheets.len() {
        if !matched_new.contains(&new_idx) {
            ops.push(MatchOp::Added(new_idx));
        }
    }

    Ok(ops)
}

fn reordered_sheet_matches(ops: &[MatchOp]) -> BTreeSet<(usize, usize)> {
    let mut pairs: Vec<_> = ops
        .iter()
        .filter_map(|op| match *op {
            MatchOp::Match(old_idx, new_idx, _) => Some((old_idx, new_idx)),
            _ => None,
        })
        .collect();
    pairs.sort_by_key(|(old_idx, _)| *old_idx);
    let kept: BTreeSet<_> = increasing_anchor_pairs(&pairs).into_iter().collect();
    pairs
        .into_iter()
        .filter(|pair| !kept.contains(pair))
        .collect()
}

#[cfg(test)]
fn diff_sheets(
    old_sheet: &SheetIr,
    new_sheet: &SheetIr,
    options: &DiffOptions,
    workbook_mapper: Option<&WorkbookCoordinateMapper>,
) -> Result<SheetDiff> {
    let sheet_area = diff_area(
        &old_sheet.rows,
        &new_sheet.rows,
        (old_sheet.start_row - 1, old_sheet.start_col - 1),
        (new_sheet.start_row - 1, new_sheet.start_col - 1),
        options,
        workbook_mapper,
    )?;
    build_sheet_diff_from_area(
        old_sheet,
        new_sheet,
        options,
        workbook_mapper,
        StyleDefaults::default(),
        sheet_area,
    )
}

fn build_sheet_diff_from_area(
    old_sheet: &SheetIr,
    new_sheet: &SheetIr,
    options: &DiffOptions,
    workbook_mapper: Option<&WorkbookCoordinateMapper>,
    style_defaults: StyleDefaults<'_>,
    mut sheet_area: AreaDiff,
) -> Result<SheetDiff> {
    if !old_sheet.rows.is_empty() && !new_sheet.rows.is_empty() {
        sheet_area.row_changes.splice(
            0..0,
            leading_row_offset_changes(old_sheet.start_row, new_sheet.start_row),
        );
        sheet_area.column_changes.splice(
            0..0,
            leading_column_offset_changes(old_sheet.start_col, new_sheet.start_col),
        );
    }
    let table_regions = diff_table_regions(
        old_sheet,
        new_sheet,
        options,
        workbook_mapper,
        style_defaults,
        Some(&sheet_area),
    )?;
    let merge_changes = diff_merge_ranges(
        &old_sheet.merged_ranges,
        &new_sheet.merged_ranges,
        &sheet_area.coordinate_mapper,
    );

    let status = if sheet_area.row_changes.is_empty()
        && sheet_area.column_changes.is_empty()
        && sheet_area.cell_changes.is_empty()
        && merge_changes.is_empty()
    {
        SheetStatus::Unchanged
    } else {
        SheetStatus::Modified
    };

    Ok(SheetDiff {
        old_name: Some(old_sheet.name.clone()),
        new_name: Some(new_sheet.name.clone()),
        status,
        old_position: None,
        new_position: None,
        order_changed: false,
        match_confidence: None,
        alignment: sheet_area.alignment,
        table_regions,
        row_changes: sheet_area.row_changes,
        column_changes: sheet_area.column_changes,
        cell_changes: sheet_area.cell_changes,
        merge_changes,
    })
}

fn append_unmaterialized_style_changes(
    diff: &mut SheetDiff,
    old_sheet: &SheetIr,
    new_sheet: &SheetIr,
    old_styles: Option<&BTreeMap<(usize, usize), String>>,
    new_styles: Option<&BTreeMap<(usize, usize), String>>,
    old_default: &str,
    new_default: &str,
) {
    let positions: BTreeSet<(usize, usize)> = old_styles
        .into_iter()
        .flat_map(BTreeMap::keys)
        .chain(new_styles.into_iter().flat_map(BTreeMap::keys))
        .copied()
        .collect();

    for (row, column) in positions {
        // Materialized cells are already compared through the structural row/column
        // mapping. Comparing their absolute addresses again would turn a moved styled
        // cell into two false style changes at its old and new coordinates.
        if sheet_contains_position(old_sheet, row, column)
            || sheet_contains_position(new_sheet, row, column)
        {
            continue;
        }
        let old_style = old_styles
            .and_then(|styles| styles.get(&(row, column)))
            .map_or(old_default, String::as_str);
        let new_style = new_styles
            .and_then(|styles| styles.get(&(row, column)))
            .map_or(new_default, String::as_str);
        if old_style == new_style {
            continue;
        }
        let cell_address = one_based_address(row, column);
        if diff.cell_changes.iter_mut().any(|change| {
            let same_cell = change.old_address.as_deref() == Some(&cell_address)
                && change.new_address.as_deref() == Some(&cell_address);
            if same_cell {
                change.style_changed = true;
            }
            same_cell
        }) {
            continue;
        }
        diff.cell_changes.push(CellChange {
            old_address: Some(cell_address.clone()),
            new_address: Some(cell_address),
            kind: CellChangeKind::StyleChanged,
            old_value: Some(String::new()),
            new_value: Some(String::new()),
            old_value_kind: Some(CellValueKind::Empty),
            new_value_kind: Some(CellValueKind::Empty),
            old_formula: None,
            new_formula: None,
            style_changed: true,
        });
    }
    if !diff.cell_changes.is_empty() && matches!(diff.status, SheetStatus::Unchanged) {
        diff.status = SheetStatus::Modified;
    }
}

fn sheet_contains_position(sheet: &SheetIr, row: usize, column: usize) -> bool {
    let width = sheet.rows.iter().map(Vec::len).max().unwrap_or(0);
    !sheet.rows.is_empty()
        && width > 0
        && row >= sheet.start_row
        && row < sheet.start_row + sheet.rows.len()
        && column >= sheet.start_col
        && column < sheet.start_col + width
}

fn leading_row_offset_changes(old_start: usize, new_start: usize) -> Vec<RowChange> {
    match old_start.cmp(&new_start) {
        Ordering::Less => (old_start..new_start)
            .map(|index| RowChange {
                kind: ChangeKind::Added,
                old_index: None,
                new_index: Some(index),
                preview: String::new(),
                confidence: None,
                ambiguous: false,
            })
            .collect(),
        Ordering::Greater => (new_start..old_start)
            .map(|index| RowChange {
                kind: ChangeKind::Removed,
                old_index: Some(index),
                new_index: None,
                preview: String::new(),
                confidence: None,
                ambiguous: false,
            })
            .collect(),
        Ordering::Equal => Vec::new(),
    }
}

fn leading_column_offset_changes(old_start: usize, new_start: usize) -> Vec<ColumnChange> {
    match old_start.cmp(&new_start) {
        Ordering::Less => (old_start..new_start)
            .map(|index| ColumnChange {
                kind: ChangeKind::Added,
                old_index: None,
                new_index: Some(index),
                header: String::new(),
                confidence: None,
                ambiguous: false,
            })
            .collect(),
        Ordering::Greater => (new_start..old_start)
            .map(|index| ColumnChange {
                kind: ChangeKind::Removed,
                old_index: Some(index),
                new_index: None,
                header: String::new(),
                confidence: None,
                ambiguous: false,
            })
            .collect(),
        Ordering::Equal => Vec::new(),
    }
}

fn removed_sheet_diff(sheet: &SheetIr, position: usize) -> SheetDiff {
    SheetDiff {
        old_name: Some(sheet.name.clone()),
        new_name: None,
        status: SheetStatus::Removed,
        old_position: Some(position),
        new_position: None,
        order_changed: false,
        match_confidence: None,
        alignment: AlignmentDiagnostics::default(),
        table_regions: detect_table_regions(&sheet.rows)
            .into_iter()
            .map(|range| TableRegionDiff {
                old_range: Some(absolute_range(sheet, &range)),
                new_range: None,
                status: RegionStatus::Removed,
                alignment: AlignmentDiagnostics::default(),
                row_changes: Vec::new(),
                column_changes: Vec::new(),
                cell_changes: Vec::new(),
            })
            .collect(),
        row_changes: Vec::new(),
        column_changes: Vec::new(),
        cell_changes: Vec::new(),
        merge_changes: sheet
            .merged_ranges
            .iter()
            .cloned()
            .map(|range| MergeChange {
                kind: ChangeKind::Removed,
                old_range: Some(range),
                new_range: None,
            })
            .collect(),
    }
}

fn added_sheet_diff(sheet: &SheetIr, position: usize) -> SheetDiff {
    SheetDiff {
        old_name: None,
        new_name: Some(sheet.name.clone()),
        status: SheetStatus::Added,
        old_position: None,
        new_position: Some(position),
        order_changed: false,
        match_confidence: None,
        alignment: AlignmentDiagnostics::default(),
        table_regions: detect_table_regions(&sheet.rows)
            .into_iter()
            .map(|range| TableRegionDiff {
                old_range: None,
                new_range: Some(absolute_range(sheet, &range)),
                status: RegionStatus::Added,
                alignment: AlignmentDiagnostics::default(),
                row_changes: Vec::new(),
                column_changes: Vec::new(),
                cell_changes: Vec::new(),
            })
            .collect(),
        row_changes: Vec::new(),
        column_changes: Vec::new(),
        cell_changes: Vec::new(),
        merge_changes: sheet
            .merged_ranges
            .iter()
            .cloned()
            .map(|range| MergeChange {
                kind: ChangeKind::Added,
                old_range: None,
                new_range: Some(range),
            })
            .collect(),
    }
}

struct AreaDiff {
    row_changes: Vec<RowChange>,
    column_changes: Vec<ColumnChange>,
    cell_changes: Vec<CellChange>,
    alignment: AlignmentDiagnostics,
    coordinate_mapper: CoordinateMapper,
}

#[derive(Clone, Copy, Default)]
struct StyleDefaults<'a> {
    old: &'a str,
    new: &'a str,
}

#[derive(Clone, Copy, Default)]
struct AreaContext<'a> {
    old_offset: (usize, usize),
    new_offset: (usize, usize),
    workbook_mapper: Option<&'a WorkbookCoordinateMapper>,
    style_defaults: StyleDefaults<'a>,
}

#[cfg(test)]
fn diff_area(
    old_rows: &[Vec<CellIr>],
    new_rows: &[Vec<CellIr>],
    old_offset: (usize, usize),
    new_offset: (usize, usize),
    options: &DiffOptions,
    workbook_mapper: Option<&WorkbookCoordinateMapper>,
) -> Result<AreaDiff> {
    diff_area_internal(
        old_rows,
        new_rows,
        options,
        AreaContext {
            old_offset,
            new_offset,
            workbook_mapper,
            style_defaults: StyleDefaults::default(),
        },
        true,
    )
}

fn align_area(
    old_rows: &[Vec<CellIr>],
    new_rows: &[Vec<CellIr>],
    old_offset: (usize, usize),
    new_offset: (usize, usize),
    options: &DiffOptions,
) -> Result<AreaDiff> {
    diff_area_internal(
        old_rows,
        new_rows,
        options,
        AreaContext {
            old_offset,
            new_offset,
            ..Default::default()
        },
        false,
    )
}

fn diff_area_internal(
    old_rows: &[Vec<CellIr>],
    new_rows: &[Vec<CellIr>],
    options: &DiffOptions,
    context: AreaContext<'_>,
    compare_cells: bool,
) -> Result<AreaDiff> {
    let old_offset = context.old_offset;
    let new_offset = context.new_offset;
    let old_profiles: Vec<_> = old_rows.iter().map(|row| RowProfile::new(row)).collect();
    let new_profiles: Vec<_> = new_rows.iter().map(|row| RowProfile::new(row)).collect();
    let row_alignment = align_sequences(
        &old_profiles,
        &new_profiles,
        |old, new| old.similarity(new),
        |row| row.signature.as_deref(),
        options,
    )?;
    let row_ambiguous_signature_groups = row_alignment.ambiguous_signature_groups;
    let row_ops = row_alignment.ops;
    let old_cols = columns_from_rows(old_rows);
    let new_cols = columns_from_rows(new_rows);
    let column_alignment = align_sequences(
        &old_cols,
        &new_cols,
        |old_col, new_col| column_similarity(old_col, new_col),
        |column| column_alignment_signature(column),
        options,
    )?;
    let column_ambiguous_signature_groups = column_alignment.ambiguous_signature_groups;
    let col_ops = column_alignment.ops;
    let MovePairing {
        moved: moved_rows,
        removed: removed_rows,
        added: added_rows,
    } = pair_moves(
        &row_ops,
        &old_profiles,
        &new_profiles,
        |old, new| old.similarity(new),
        options,
    )?;
    let MovePairing {
        moved: moved_cols,
        removed: removed_cols,
        added: added_cols,
    } = pair_moves(
        &col_ops,
        &old_cols,
        &new_cols,
        |old_col, new_col| column_similarity(old_col, new_col),
        options,
    )?;

    let mut row_changes = Vec::new();
    for old_idx in removed_rows {
        row_changes.push(RowChange {
            kind: ChangeKind::Removed,
            old_index: Some(old_offset.0 + old_idx + 1),
            new_index: None,
            preview: row_preview(&old_rows[old_idx]),
            confidence: None,
            ambiguous: false,
        });
    }
    for new_idx in added_rows {
        row_changes.push(RowChange {
            kind: ChangeKind::Added,
            old_index: None,
            new_index: Some(new_offset.0 + new_idx + 1),
            preview: row_preview(&new_rows[new_idx]),
            confidence: None,
            ambiguous: false,
        });
    }
    for moved in &moved_rows {
        row_changes.push(RowChange {
            kind: ChangeKind::Moved,
            old_index: Some(old_offset.0 + moved.old_index + 1),
            new_index: Some(new_offset.0 + moved.new_index + 1),
            preview: row_preview(&old_rows[moved.old_index]),
            confidence: Some(moved.confidence),
            ambiguous: moved.ambiguous,
        });
    }

    let mut column_changes = Vec::new();
    for old_idx in removed_cols {
        column_changes.push(ColumnChange {
            kind: ChangeKind::Removed,
            old_index: Some(old_offset.1 + old_idx + 1),
            new_index: None,
            header: column_header(&old_cols[old_idx]),
            confidence: None,
            ambiguous: false,
        });
    }
    for new_idx in added_cols {
        column_changes.push(ColumnChange {
            kind: ChangeKind::Added,
            old_index: None,
            new_index: Some(new_offset.1 + new_idx + 1),
            header: column_header(&new_cols[new_idx]),
            confidence: None,
            ambiguous: false,
        });
    }
    for moved in &moved_cols {
        column_changes.push(ColumnChange {
            kind: ChangeKind::Moved,
            old_index: Some(old_offset.1 + moved.old_index + 1),
            new_index: Some(new_offset.1 + moved.new_index + 1),
            header: column_header(&old_cols[moved.old_index]),
            confidence: Some(moved.confidence),
            ambiguous: moved.ambiguous,
        });
    }

    let mut matched_rows: Vec<(usize, usize, f64)> = row_ops
        .iter()
        .filter_map(|op| match *op {
            MatchOp::Match(old_idx, new_idx, score) => Some((old_idx, new_idx, score)),
            _ => None,
        })
        .collect();
    matched_rows.extend(
        moved_rows
            .iter()
            .map(|moved| (moved.old_index, moved.new_index, moved.confidence)),
    );
    let mut matched_cols: Vec<(usize, usize, f64)> = col_ops
        .iter()
        .filter_map(|op| match *op {
            MatchOp::Match(old_idx, new_idx, score) => Some((old_idx, new_idx, score)),
            _ => None,
        })
        .collect();
    matched_cols.extend(
        moved_cols
            .iter()
            .map(|moved| (moved.old_index, moved.new_index, moved.confidence)),
    );

    let row_scores: Vec<_> = matched_rows.iter().map(|(_, _, score)| *score).collect();
    let column_scores: Vec<_> = matched_cols.iter().map(|(_, _, score)| *score).collect();
    let alignment = AlignmentDiagnostics {
        row_ambiguous_signature_groups,
        column_ambiguous_signature_groups,
        low_confidence_row_matches: row_scores
            .iter()
            .filter(|score| **score < options.move_match_score)
            .count(),
        low_confidence_column_matches: column_scores
            .iter()
            .filter(|score| **score < options.move_match_score)
            .count(),
        minimum_row_confidence: row_scores.into_iter().reduce(f64::min),
        minimum_column_confidence: column_scores.into_iter().reduce(f64::min),
    };
    let coordinate_mapper = CoordinateMapper {
        rows: matched_rows
            .iter()
            .map(|(old_row, new_row, _)| (old_offset.0 + old_row + 1, new_offset.0 + new_row + 1))
            .collect(),
        columns: matched_cols
            .iter()
            .map(|(old_col, new_col, _)| (old_offset.1 + old_col + 1, new_offset.1 + new_col + 1))
            .collect(),
    };

    let cell_changes = if compare_cells {
        compare_mapped_cells(old_rows, new_rows, options, &coordinate_mapper, context)
    } else {
        Vec::new()
    };

    Ok(AreaDiff {
        row_changes,
        column_changes,
        cell_changes,
        alignment,
        coordinate_mapper,
    })
}

fn compare_mapped_cells(
    old_rows: &[Vec<CellIr>],
    new_rows: &[Vec<CellIr>],
    options: &DiffOptions,
    coordinate_mapper: &CoordinateMapper,
    context: AreaContext<'_>,
) -> Vec<CellChange> {
    let old_offset = context.old_offset;
    let new_offset = context.new_offset;
    let mut cell_changes = Vec::new();
    for (&old_absolute_row, &new_absolute_row) in &coordinate_mapper.rows {
        let old_row = old_absolute_row - old_offset.0 - 1;
        let new_row = new_absolute_row - new_offset.0 - 1;
        for (&old_absolute_col, &new_absolute_col) in &coordinate_mapper.columns {
            let old_col = old_absolute_col - old_offset.1 - 1;
            let new_col = new_absolute_col - new_offset.1 - 1;
            let old_cell = get_cell(old_rows, old_row, old_col);
            let new_cell = get_cell(new_rows, new_row, new_col);
            let formulas_equal = formulas_equivalent_after_structure_change(
                old_cell.formula.as_deref(),
                new_cell.formula.as_deref(),
                coordinate_mapper,
                context.workbook_mapper,
            );
            let cached_formula_values_ignored = matches!(
                options.formula_values,
                FormulaValueComparison::IgnoreWhenFormulasEquivalent
            ) && old_cell.formula.is_some()
                && new_cell.formula.is_some()
                && formulas_equal;
            let values_equal =
                cell_values_equal(old_cell, new_cell) || cached_formula_values_ignored;
            let old_style = if old_cell.style_signature.is_empty() {
                context.style_defaults.old
            } else {
                &old_cell.style_signature
            };
            let new_style = if new_cell.style_signature.is_empty() {
                context.style_defaults.new
            } else {
                &new_cell.style_signature
            };
            let style_changed = old_style != new_style;
            if values_equal && formulas_equal && !style_changed {
                continue;
            }

            let kind = match (
                !values_equal,
                !formulas_equal,
                old_cell.value.is_empty(),
                new_cell.value.is_empty(),
            ) {
                (true, false, true, false) => CellChangeKind::ValueAdded,
                (true, false, false, true) => CellChangeKind::ValueRemoved,
                (true, true, _, _) => CellChangeKind::ValueAndFormulaChanged,
                (true, false, _, _) => CellChangeKind::ValueChanged,
                (false, true, _, _) => CellChangeKind::FormulaChanged,
                (false, false, _, _) => CellChangeKind::StyleChanged,
            };
            cell_changes.push(CellChange {
                old_address: Some(one_based_address(old_absolute_row, old_absolute_col)),
                new_address: Some(one_based_address(new_absolute_row, new_absolute_col)),
                kind,
                old_value: Some(old_cell.value.clone()),
                new_value: Some(new_cell.value.clone()),
                old_value_kind: Some(old_cell.value_kind),
                new_value_kind: Some(new_cell.value_kind),
                old_formula: old_cell.formula.clone(),
                new_formula: new_cell.formula.clone(),
                style_changed,
            });
        }
    }
    cell_changes
}

struct SequenceAlignment {
    ops: Vec<MatchOp>,
    ambiguous_signature_groups: usize,
}

fn align_sequences<'a, T, F, K, S>(
    old_items: &'a [T],
    new_items: &'a [T],
    similarity: F,
    exact_key: K,
    options: &DiffOptions,
) -> Result<SequenceAlignment>
where
    F: Fn(&T, &T) -> f64,
    K: Fn(&'a T) -> Option<S>,
    S: AsRef<str> + PartialEq,
{
    let old_keys: Vec<_> = old_items.iter().map(&exact_key).collect();
    let new_keys: Vec<_> = new_items.iter().map(&exact_key).collect();
    let mut old_positions: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut new_positions: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (idx, key) in old_keys.iter().enumerate() {
        if let Some(key) = key.as_ref().map(AsRef::as_ref) {
            old_positions.entry(key).or_default().push(idx);
        }
    }
    for (idx, key) in new_keys.iter().enumerate() {
        if let Some(key) = key.as_ref().map(AsRef::as_ref) {
            new_positions.entry(key).or_default().push(idx);
        }
    }
    let ambiguous_signature_groups = old_positions
        .iter()
        .filter(|(key, old)| {
            new_positions
                .get(**key)
                .is_some_and(|new| old.len() > 1 || new.len() > 1)
        })
        .count();
    let candidates: Vec<_> = old_positions
        .iter()
        .filter_map(|(key, old)| {
            let new = new_positions.get(key)?;
            (old.len() == 1 && new.len() == 1).then_some((old[0], new[0]))
        })
        .collect();
    let anchors = increasing_anchor_pairs(&candidates);

    let mut ops = Vec::new();
    let mut old_start = 0usize;
    let mut new_start = 0usize;
    let mut remaining_pairs = options.limits.max_alignment_pairs;
    for (old_anchor, new_anchor) in anchors {
        ops.extend(align_segment(
            SequenceSegment {
                items: &old_items[old_start..old_anchor],
                keys: &old_keys[old_start..old_anchor],
                offset: old_start,
            },
            SequenceSegment {
                items: &new_items[new_start..new_anchor],
                keys: &new_keys[new_start..new_anchor],
                offset: new_start,
            },
            &similarity,
            options.min_match_score,
            &mut remaining_pairs,
        )?);
        ops.push(MatchOp::Match(old_anchor, new_anchor, 1.0));
        old_start = old_anchor + 1;
        new_start = new_anchor + 1;
    }
    ops.extend(align_segment(
        SequenceSegment {
            items: &old_items[old_start..],
            keys: &old_keys[old_start..],
            offset: old_start,
        },
        SequenceSegment {
            items: &new_items[new_start..],
            keys: &new_keys[new_start..],
            offset: new_start,
        },
        &similarity,
        options.min_match_score,
        &mut remaining_pairs,
    )?);

    Ok(SequenceAlignment {
        ops,
        ambiguous_signature_groups,
    })
}

fn increasing_anchor_pairs(candidates: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut ordered = candidates.to_vec();
    ordered.sort_by_key(|pair| pair.0);
    if ordered.is_empty() {
        return Vec::new();
    }
    let mut tails: Vec<usize> = Vec::new();
    let mut previous = vec![None; ordered.len()];
    for idx in 0..ordered.len() {
        let new_position = ordered[idx].1;
        let slot = tails
            .binary_search_by(|tail_idx| ordered[*tail_idx].1.cmp(&new_position))
            .unwrap_or_else(|slot| slot);
        if slot > 0 {
            previous[idx] = Some(tails[slot - 1]);
        }
        if slot == tails.len() {
            tails.push(idx);
        } else {
            tails[slot] = idx;
        }
    }
    let mut result = Vec::new();
    let mut cursor = tails.last().copied();
    while let Some(idx) = cursor {
        result.push(ordered[idx]);
        cursor = previous[idx];
    }
    result.reverse();
    result
}

struct SequenceSegment<'a, T, S> {
    items: &'a [T],
    keys: &'a [Option<S>],
    offset: usize,
}

fn align_segment<T, F, S>(
    old: SequenceSegment<'_, T, S>,
    new: SequenceSegment<'_, T, S>,
    similarity: &F,
    min_match_score: f64,
    remaining_pairs: &mut usize,
) -> Result<Vec<MatchOp>>
where
    F: Fn(&T, &T) -> f64,
    S: PartialEq,
{
    let n = old.items.len();
    let m = new.items.len();
    if n == 0 {
        return Ok((0..m).map(|idx| MatchOp::Added(new.offset + idx)).collect());
    }
    if m == 0 {
        return Ok((0..n)
            .map(|idx| MatchOp::Removed(old.offset + idx))
            .collect());
    }
    if old.keys == new.keys {
        let scores: Vec<_> = old
            .items
            .iter()
            .zip(new.items)
            .map(|(old, new)| similarity(old, new))
            .collect();
        return Ok(scores
            .into_iter()
            .enumerate()
            .map(|(idx, score)| MatchOp::Match(old.offset + idx, new.offset + idx, score))
            .collect());
    }
    if old.keys.iter().all(|key| key == &old.keys[0]) {
        let exact_new: Vec<_> = new.keys.iter().map(|key| key == &old.keys[0]).collect();
        if exact_new.iter().filter(|is_exact| **is_exact).count() == n {
            let mut old_idx = 0usize;
            return Ok(exact_new
                .into_iter()
                .enumerate()
                .map(|(new_idx, is_exact)| {
                    if is_exact {
                        let op = MatchOp::Match(old.offset + old_idx, new.offset + new_idx, 1.0);
                        old_idx += 1;
                        op
                    } else {
                        MatchOp::Added(new.offset + new_idx)
                    }
                })
                .collect());
        }
    }
    if new.keys.iter().all(|key| key == &new.keys[0]) {
        let exact_old: Vec<_> = old.keys.iter().map(|key| key == &new.keys[0]).collect();
        if exact_old.iter().filter(|is_exact| **is_exact).count() == m {
            let mut new_idx = 0usize;
            return Ok(exact_old
                .into_iter()
                .enumerate()
                .map(|(old_idx, is_exact)| {
                    if is_exact {
                        let op = MatchOp::Match(old.offset + old_idx, new.offset + new_idx, 1.0);
                        new_idx += 1;
                        op
                    } else {
                        MatchOp::Removed(old.offset + old_idx)
                    }
                })
                .collect());
        }
    }
    if n == 1 && m == 1 {
        let score = (similarity(&old.items[0], &new.items[0]) + SINGLE_ITEM_CONTEXT_BONUS).min(1.0);
        if score >= min_match_score {
            // A one-to-one segment is strong positional evidence (and is
            // commonly bounded by exact anchors) that a row or column was
            // edited in place. Preserve that evidence even when every value
            // changed and content similarity alone is just below the general
            // matching threshold.
            return Ok(vec![MatchOp::Match(old.offset, new.offset, score)]);
        }
    }
    let pairs = n.checked_mul(m).context("alignment size overflow")?;
    anyhow::ensure!(
        pairs <= *remaining_pairs,
        "alignment segment {n}x{m} exceeds remaining max_alignment_pairs {}",
        *remaining_pairs
    );
    *remaining_pairs -= pairs;
    // Traceback needs one byte per cell, but the score recurrence only reads the
    // previous row. Keeping all f64 score rows used roughly 8 * n * m bytes on
    // top of traceback (32 MiB at the default four-million-pair limit).
    let stride = m.checked_add(1).context("alignment width overflow")?;
    let back_len = n
        .checked_add(1)
        .and_then(|rows| rows.checked_mul(stride))
        .context("alignment traceback size overflow")?;
    let mut back = vec![0u8; back_len];
    let mut previous = vec![0.0; stride];
    let mut current = vec![0.0; stride];

    for j in 1..=m {
        previous[j] = previous[j - 1] - GAP_PENALTY;
        back[j] = 2;
    }

    for i in 1..=n {
        current[0] = previous[0] - GAP_PENALTY;
        back[i * stride] = 1;
        for j in 1..=m {
            let score = similarity(&old.items[i - 1], &new.items[j - 1]);
            let diag = if score >= min_match_score {
                previous[j - 1] + score
            } else {
                f64::NEG_INFINITY
            };
            let up = previous[j] - GAP_PENALTY;
            let left = current[j - 1] - GAP_PENALTY;
            let back_index = i * stride + j;

            if diag >= up && diag >= left {
                current[j] = diag;
                back[back_index] = 0;
            } else if up >= left {
                current[j] = up;
                back[back_index] = 1;
            } else {
                current[j] = left;
                back[back_index] = 2;
            }
        }
        std::mem::swap(&mut previous, &mut current);
    }

    let mut i = n;
    let mut j = m;
    let mut ops = Vec::new();
    while i > 0 || j > 0 {
        match back[i * stride + j] {
            0 => {
                let score = similarity(&old.items[i - 1], &new.items[j - 1]);
                ops.push(MatchOp::Match(
                    old.offset + i - 1,
                    new.offset + j - 1,
                    score,
                ));
                i -= 1;
                j -= 1;
            }
            1 => {
                ops.push(MatchOp::Removed(old.offset + i - 1));
                i -= 1;
            }
            _ => {
                ops.push(MatchOp::Added(new.offset + j - 1));
                j -= 1;
            }
        }
    }
    ops.reverse();
    Ok(ops)
}

struct MoveMatch {
    old_index: usize,
    new_index: usize,
    confidence: f64,
    ambiguous: bool,
}

struct MovePairing {
    moved: Vec<MoveMatch>,
    removed: Vec<usize>,
    added: Vec<usize>,
}

/// Returns the maximum-total-score one-to-one assignment. Rows may be left
/// unmatched through zero-score dummy columns, and edges below `threshold` are
/// never selected. The Hungarian implementation is deterministic on ties.
fn maximum_weight_assignment(
    scores: &[Vec<f64>],
    threshold: f64,
    max_assignment_size: usize,
) -> Result<Vec<Option<(usize, f64)>>> {
    let row_count = scores.len();
    let column_count = scores.first().map(Vec::len).unwrap_or(0);
    anyhow::ensure!(
        scores.iter().all(|row| row.len() == column_count),
        "assignment score matrix must be rectangular"
    );
    anyhow::ensure!(
        row_count <= max_assignment_size && column_count <= max_assignment_size,
        "global assignment {row_count}x{column_count} exceeds max_assignment_size {max_assignment_size}"
    );
    if row_count == 0 || column_count == 0 {
        return Ok(vec![None; row_count]);
    }

    // One dummy column per row lets every row remain unassigned. With costs
    // `1 - score` and dummy cost 1, minimizing cost maximizes total score.
    let total_columns = column_count + row_count;
    let mut potential_rows = vec![0.0; row_count + 1];
    let mut potential_columns = vec![0.0; total_columns + 1];
    let mut column_row = vec![0usize; total_columns + 1];
    let mut predecessor = vec![0usize; total_columns + 1];

    for row in 1..=row_count {
        column_row[0] = row;
        let mut current_column = 0usize;
        let mut minimum = vec![f64::INFINITY; total_columns + 1];
        let mut used = vec![false; total_columns + 1];
        loop {
            used[current_column] = true;
            let current_row = column_row[current_column];
            let mut delta = f64::INFINITY;
            let mut next_column = 0usize;
            for column in 1..=total_columns {
                if used[column] {
                    continue;
                }
                let raw_cost = if column <= column_count {
                    let score = scores[current_row - 1][column - 1];
                    if score >= threshold { 1.0 - score } else { 2.0 }
                } else {
                    1.0
                };
                let reduced_cost =
                    raw_cost - potential_rows[current_row] - potential_columns[column];
                if reduced_cost < minimum[column] {
                    minimum[column] = reduced_cost;
                    predecessor[column] = current_column;
                }
                if minimum[column] < delta {
                    delta = minimum[column];
                    next_column = column;
                }
            }
            for column in 0..=total_columns {
                if used[column] {
                    potential_rows[column_row[column]] += delta;
                    potential_columns[column] -= delta;
                } else {
                    minimum[column] -= delta;
                }
            }
            current_column = next_column;
            if column_row[current_column] == 0 {
                break;
            }
        }
        loop {
            let previous_column = predecessor[current_column];
            column_row[current_column] = column_row[previous_column];
            current_column = previous_column;
            if current_column == 0 {
                break;
            }
        }
    }

    let mut result = vec![None; row_count];
    for column in 1..=column_count {
        let row = column_row[column];
        if row > 0 {
            let score = scores[row - 1][column - 1];
            if score >= threshold {
                result[row - 1] = Some((column - 1, score));
            }
        }
    }
    Ok(result)
}

fn pair_moves<T, F>(
    ops: &[MatchOp],
    old_items: &[T],
    new_items: &[T],
    similarity: F,
    options: &DiffOptions,
) -> Result<MovePairing>
where
    F: Fn(&T, &T) -> f64,
{
    let removed: Vec<usize> = ops
        .iter()
        .filter_map(|op| match *op {
            MatchOp::Removed(idx) => Some(idx),
            _ => None,
        })
        .collect();
    let added: Vec<usize> = ops
        .iter()
        .filter_map(|op| match *op {
            MatchOp::Added(idx) => Some(idx),
            _ => None,
        })
        .collect();

    let pair_count = removed
        .len()
        .checked_mul(added.len())
        .context("move pairing size overflow")?;
    anyhow::ensure!(
        pair_count <= options.limits.max_alignment_pairs,
        "move pairing {pair_count} exceeds max_alignment_pairs {}",
        options.limits.max_alignment_pairs
    );
    let scores: Vec<Vec<_>> = removed
        .iter()
        .map(|old_idx| {
            added
                .iter()
                .map(|new_idx| similarity(&old_items[*old_idx], &new_items[*new_idx]))
                .collect()
        })
        .collect();
    let assignment = maximum_weight_assignment(
        &scores,
        options.move_match_score,
        options.limits.max_assignment_size,
    )?;
    let mut used_added = BTreeSet::new();
    let mut moved = Vec::new();
    let mut unmatched_removed = Vec::new();

    for (old_local, old_idx) in removed.into_iter().enumerate() {
        if let Some((new_local, score)) = assignment[old_local] {
            let new_idx = added[new_local];
            let mut candidates: Vec<_> = scores[old_local]
                .iter()
                .copied()
                .filter(|candidate| *candidate >= options.move_match_score)
                .collect();
            candidates.sort_by(|a, b| b.partial_cmp(a).unwrap_or(Ordering::Equal));
            let ambiguous = candidates
                .get(1)
                .is_some_and(|second| score - second <= options.ambiguity_margin);
            used_added.insert(new_idx);
            moved.push(MoveMatch {
                old_index: old_idx,
                new_index: new_idx,
                confidence: score,
                ambiguous,
            });
        } else {
            unmatched_removed.push(old_idx);
        }
    }

    let unmatched_added = added
        .into_iter()
        .filter(|new_idx| !used_added.contains(new_idx))
        .collect();

    Ok(MovePairing {
        moved,
        removed: unmatched_removed,
        added: unmatched_added,
    })
}

#[derive(Clone, Default)]
struct CoordinateMapper {
    rows: BTreeMap<usize, usize>,
    columns: BTreeMap<usize, usize>,
}

impl CoordinateMapper {
    fn map_row_confident(&self, row: usize) -> usize {
        map_with_consistent_delta(&self.rows, row).unwrap_or(row)
    }

    fn map_column_confident(&self, column: usize) -> usize {
        map_with_consistent_delta(&self.columns, column).unwrap_or(column)
    }

    fn map_row_start(&self, row: usize) -> usize {
        map_boundary(&self.rows, row, BoundarySide::Start).unwrap_or(row)
    }

    fn map_row_end(&self, row: usize) -> usize {
        map_boundary(&self.rows, row, BoundarySide::End).unwrap_or(row)
    }

    fn map_column_start(&self, column: usize) -> usize {
        map_boundary(&self.columns, column, BoundarySide::Start).unwrap_or(column)
    }

    fn map_column_end(&self, column: usize) -> usize {
        map_boundary(&self.columns, column, BoundarySide::End).unwrap_or(column)
    }
}

#[derive(Default)]
struct WorkbookCoordinateMapper {
    sheets: HashMap<String, SheetCoordinateMapping>,
}

struct SheetCoordinateMapping {
    new_name: String,
    mapper: CoordinateMapper,
}

fn normalize_sheet_name(name: &str) -> String {
    canonical_sheet_identity(name).to_uppercase()
}

fn canonical_sheet_identity(name: &str) -> String {
    name.nfc().collect()
}

fn sheet_names_equivalent(old: &str, new: &str) -> bool {
    old == new || canonical_sheet_identity(old) == canonical_sheet_identity(new)
}

#[derive(Clone, Copy)]
enum BoundarySide {
    Start,
    End,
}

fn mapping_delta(old: usize, new: usize) -> Option<isize> {
    isize::try_from(new)
        .ok()?
        .checked_sub(isize::try_from(old).ok()?)
}

fn apply_mapping_delta(index: usize, delta: isize) -> Option<usize> {
    index.checked_add_signed(delta).filter(|mapped| *mapped > 0)
}

fn neighboring_deltas(
    mapping: &BTreeMap<usize, usize>,
    index: usize,
) -> (Option<isize>, Option<isize>) {
    let lower = mapping
        .range(..index)
        .next_back()
        .and_then(|(old, new)| mapping_delta(*old, *new));
    let upper = mapping
        .range((index + 1)..)
        .next()
        .and_then(|(old, new)| mapping_delta(*old, *new));
    (lower, upper)
}

fn map_with_consistent_delta(mapping: &BTreeMap<usize, usize>, index: usize) -> Option<usize> {
    if let Some(mapped) = mapping.get(&index) {
        return Some(*mapped);
    }
    let (lower, upper) = neighboring_deltas(mapping, index);
    match (lower, upper) {
        (Some(lower), Some(upper)) if lower == upper => apply_mapping_delta(index, lower),
        _ => None,
    }
}

fn map_boundary(
    mapping: &BTreeMap<usize, usize>,
    index: usize,
    side: BoundarySide,
) -> Option<usize> {
    if let Some(mapped) = mapping.get(&index) {
        return Some(*mapped);
    }
    let (lower, upper) = neighboring_deltas(mapping, index);
    let delta = match (lower, upper, side) {
        (Some(lower), Some(upper), _) if lower == upper => lower,
        (Some(lower), _, BoundarySide::Start) => lower,
        (_, Some(upper), BoundarySide::End) => upper,
        (Some(lower), None, _) => lower,
        (None, Some(upper), _) => upper,
        _ => return None,
    };
    apply_mapping_delta(index, delta)
}

fn formulas_equivalent_after_structure_change(
    old_formula: Option<&str>,
    new_formula: Option<&str>,
    mapper: &CoordinateMapper,
    workbook_mapper: Option<&WorkbookCoordinateMapper>,
) -> bool {
    match (old_formula, new_formula) {
        (None, None) => true,
        (Some(old), Some(new)) => {
            canonical_formula_with_workbook(old, Some(mapper), workbook_mapper)
                == canonical_formula_with_workbook(new, None, None)
        }
        _ => false,
    }
}

fn canonical_formula(formula: &str, mapper: Option<&CoordinateMapper>) -> String {
    canonical_formula_with_workbook(formula, mapper, None)
}

fn canonical_formula_with_workbook(
    formula: &str,
    mapper: Option<&CoordinateMapper>,
    workbook_mapper: Option<&WorkbookCoordinateMapper>,
) -> String {
    let chars: Vec<char> = formula.chars().collect();
    let mut output = String::with_capacity(formula.len());
    let mut idx = 0;
    while idx < chars.len() {
        if let Some(qualifier) = parse_three_dimensional_sheet_qualifier(&chars, idx) {
            if let Some(reference) = parse_formula_reference(&chars, qualifier.reference_start) {
                let canonical_first =
                    mapped_sheet_name(&qualifier.first_sheet_name, workbook_mapper);
                let canonical_last = mapped_sheet_name(&qualifier.last_sheet_name, workbook_mapper);
                write_canonical_sheet_range_qualifier(&mut output, canonical_first, canonical_last);
                write_formula_reference(&mut output, &reference, reference.row, reference.column);
                let mut end = reference.end;
                if chars.get(end) == Some(&':')
                    && let Some(range_end) = parse_formula_reference(&chars, end + 1)
                {
                    output.push(':');
                    write_formula_reference(
                        &mut output,
                        &range_end,
                        range_end.row,
                        range_end.column,
                    );
                    end = range_end.end;
                }
                idx = end;
                continue;
            }
            if let Some(range) = parse_formula_axis_range(&chars, qualifier.reference_start) {
                let canonical_first =
                    mapped_sheet_name(&qualifier.first_sheet_name, workbook_mapper);
                let canonical_last = mapped_sheet_name(&qualifier.last_sheet_name, workbook_mapper);
                write_canonical_sheet_range_qualifier(&mut output, canonical_first, canonical_last);
                write_formula_axis_range(&mut output, &range, None);
                idx = range.end;
                continue;
            }
        }
        if let Some(qualified) = parse_sheet_qualified_reference(&chars, idx) {
            let target = workbook_mapper.and_then(|workbook| {
                workbook
                    .sheets
                    .get(&normalize_sheet_name(&qualified.sheet_name))
            });
            let canonical_sheet = target
                .map(|mapping| mapping.new_name.as_str())
                .unwrap_or(&qualified.sheet_name);
            let target_mapper = target.map(|mapping| &mapping.mapper);
            let mapped_row = target_mapper
                .map(|target| target.map_row_confident(qualified.reference.row))
                .unwrap_or(qualified.reference.row);
            let mapped_column = target_mapper
                .map(|target| target.map_column_confident(qualified.reference.column))
                .unwrap_or(qualified.reference.column);
            write_canonical_sheet_qualifier(&mut output, canonical_sheet);
            write_formula_reference(&mut output, &qualified.reference, mapped_row, mapped_column);
            let mut end = qualified.reference.end;
            if chars.get(end) == Some(&':')
                && let Some(range_end) = parse_formula_reference(&chars, end + 1)
            {
                let mapped_end_row = target_mapper
                    .map(|target| target.map_row_confident(range_end.row))
                    .unwrap_or(range_end.row);
                let mapped_end_column = target_mapper
                    .map(|target| target.map_column_confident(range_end.column))
                    .unwrap_or(range_end.column);
                output.push(':');
                write_formula_reference(&mut output, &range_end, mapped_end_row, mapped_end_column);
                end = range_end.end;
            }
            idx = end;
            continue;
        }
        if let Some(qualified) = parse_sheet_qualified_axis_range(&chars, idx) {
            let target = workbook_mapper.and_then(|workbook| {
                workbook
                    .sheets
                    .get(&normalize_sheet_name(&qualified.sheet_name))
            });
            let canonical_sheet = target
                .map(|mapping| mapping.new_name.as_str())
                .unwrap_or(&qualified.sheet_name);
            let target_mapper = target.map(|mapping| &mapping.mapper);
            write_canonical_sheet_qualifier(&mut output, canonical_sheet);
            write_formula_axis_range(&mut output, &qualified.range, target_mapper);
            idx = qualified.range.end;
            continue;
        }
        if let Some((end, segment)) = protected_segment(&chars, idx) {
            if is_structured_reference_start(&chars, idx) {
                for ch in segment.chars() {
                    output.extend(ch.to_uppercase());
                }
            } else {
                output.push_str(&segment);
            }
            idx = end;
            continue;
        }
        if chars[idx].is_whitespace() {
            let whitespace_start = idx;
            while chars.get(idx).is_some_and(|ch| ch.is_whitespace()) {
                idx += 1;
            }
            let previous = chars[..whitespace_start]
                .iter()
                .rev()
                .find(|ch| !ch.is_whitespace())
                .copied();
            let next = chars[idx..].iter().find(|ch| !ch.is_whitespace()).copied();
            if formula_whitespace_is_significant(previous, next) && !output.ends_with(' ') {
                output.push(' ');
            }
            continue;
        }
        if let Some(range) = parse_formula_axis_range(&chars, idx) {
            let sheet_qualified = chars[..idx]
                .iter()
                .rev()
                .find(|ch| !ch.is_whitespace())
                .is_some_and(|ch| *ch == '!');
            write_formula_axis_range(
                &mut output,
                &range,
                if sheet_qualified { None } else { mapper },
            );
            idx = range.end;
            continue;
        }
        if let Some(reference) = parse_formula_reference(&chars, idx) {
            let sheet_qualified = chars[..idx]
                .iter()
                .rev()
                .find(|ch| !ch.is_whitespace())
                .is_some_and(|ch| *ch == '!');
            let mapped_row = if sheet_qualified {
                reference.row
            } else {
                mapper
                    .map(|mapper| mapper.map_row_confident(reference.row))
                    .unwrap_or(reference.row)
            };
            let mapped_col = if sheet_qualified {
                reference.column
            } else {
                mapper
                    .map(|mapper| mapper.map_column_confident(reference.column))
                    .unwrap_or(reference.column)
            };
            write_formula_reference(&mut output, &reference, mapped_row, mapped_col);
            idx = reference.end;
            continue;
        }
        output.extend(chars[idx].to_uppercase());
        idx += 1;
    }
    output
}

fn mapped_sheet_name<'a>(
    sheet_name: &'a str,
    workbook_mapper: Option<&'a WorkbookCoordinateMapper>,
) -> &'a str {
    workbook_mapper
        .and_then(|workbook| workbook.sheets.get(&normalize_sheet_name(sheet_name)))
        .map(|mapping| mapping.new_name.as_str())
        .unwrap_or(sheet_name)
}

fn formula_whitespace_is_significant(previous: Option<char>, next: Option<char>) -> bool {
    let (Some(previous), Some(next)) = (previous, next) else {
        return false;
    };
    let ignorable_after_previous = matches!(
        previous,
        '=' | '+' | '-' | '*' | '/' | '^' | '&' | '<' | '>' | ',' | ';' | ':' | '(' | '{' | '@'
    );
    let ignorable_before_next = matches!(
        next,
        '=' | '+'
            | '-'
            | '*'
            | '/'
            | '^'
            | '&'
            | '<'
            | '>'
            | ','
            | ';'
            | ':'
            | ')'
            | '}'
            | '%'
            | '#'
    );
    !ignorable_after_previous && !ignorable_before_next
}

fn write_canonical_sheet_qualifier(output: &mut String, sheet_name: &str) {
    write_canonical_sheet_name(output, sheet_name);
    output.push('!');
}

fn write_canonical_sheet_range_qualifier(output: &mut String, first: &str, last: &str) {
    write_canonical_sheet_name(output, first);
    output.push(':');
    write_canonical_sheet_name(output, last);
    output.push('!');
}

fn write_canonical_sheet_name(output: &mut String, sheet_name: &str) {
    let normalized_sheet = normalize_sheet_name(sheet_name);
    output.push_str("#SHEET{");
    output.push_str(&normalized_sheet.chars().count().to_string());
    output.push(':');
    output.push_str(&normalized_sheet);
    output.push('}');
}

fn write_formula_reference(
    output: &mut String,
    reference: &FormulaReference,
    row: usize,
    column: usize,
) {
    if reference.absolute_column {
        output.push('$');
    }
    output.push_str(&column_name(column - 1));
    if reference.absolute_row {
        output.push('$');
    }
    output.push_str(&row.to_string());
}

struct FormulaReference {
    end: usize,
    row: usize,
    column: usize,
    absolute_row: bool,
    absolute_column: bool,
}

#[derive(Clone, Copy)]
enum FormulaAxis {
    Row,
    Column,
}

struct FormulaAxisRange {
    end: usize,
    axis: FormulaAxis,
    first: usize,
    second: usize,
    absolute_first: bool,
    absolute_second: bool,
}

struct SheetQualifiedReference {
    sheet_name: String,
    reference: FormulaReference,
}

struct SheetQualifiedAxisRange {
    sheet_name: String,
    range: FormulaAxisRange,
}

struct SheetQualifier {
    sheet_name: String,
    reference_start: usize,
}

struct ThreeDimensionalSheetQualifier {
    first_sheet_name: String,
    last_sheet_name: String,
    reference_start: usize,
}

fn sheet_qualifier_can_start(chars: &[char], idx: usize) -> bool {
    idx == 0
        || !(chars[idx - 1].is_alphanumeric()
            || matches!(chars[idx - 1], '_' | '.' | ':' | ']' | '\''))
}

fn parse_quoted_sheet_name(chars: &[char], idx: usize) -> Option<(String, usize)> {
    if chars.get(idx) != Some(&'\'') {
        return None;
    }
    let mut cursor = idx + 1;
    let mut sheet_name = String::new();
    loop {
        let ch = *chars.get(cursor)?;
        if ch == '\'' {
            if chars.get(cursor + 1) == Some(&'\'') {
                sheet_name.push('\'');
                cursor += 2;
                continue;
            }
            return Some((sheet_name, cursor + 1));
        }
        sheet_name.push(ch);
        cursor += 1;
    }
}

fn parse_sheet_qualifier(chars: &[char], idx: usize) -> Option<SheetQualifier> {
    if !sheet_qualifier_can_start(chars, idx) {
        return None;
    }

    let (sheet_name, bang) = if chars.get(idx) == Some(&'\'') {
        parse_quoted_sheet_name(chars, idx)?
    } else {
        let mut cursor = idx;
        while chars
            .get(cursor)
            .is_some_and(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '.'))
        {
            cursor += 1;
        }
        if cursor == idx {
            return None;
        }
        (chars[idx..cursor].iter().collect(), cursor)
    };
    if chars.get(bang) != Some(&'!') {
        return None;
    }
    Some(SheetQualifier {
        sheet_name,
        reference_start: bang + 1,
    })
}

fn parse_three_dimensional_sheet_qualifier(
    chars: &[char],
    idx: usize,
) -> Option<ThreeDimensionalSheetQualifier> {
    if !sheet_qualifier_can_start(chars, idx) {
        return None;
    }
    let (first_sheet_name, last_sheet_name, bang) = if chars.get(idx) == Some(&'\'') {
        let (sheet_range, bang) = parse_quoted_sheet_name(chars, idx)?;
        let (first, last) = sheet_range.split_once(':')?;
        if first.is_empty() || last.is_empty() || last.contains(':') {
            return None;
        }
        (first.to_string(), last.to_string(), bang)
    } else {
        let mut cursor = idx;
        while chars
            .get(cursor)
            .is_some_and(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '.'))
        {
            cursor += 1;
        }
        if cursor == idx || chars.get(cursor) != Some(&':') {
            return None;
        }
        let first_sheet_name = chars[idx..cursor].iter().collect();
        cursor += 1;
        let last_start = cursor;
        while chars
            .get(cursor)
            .is_some_and(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '.'))
        {
            cursor += 1;
        }
        if cursor == last_start {
            return None;
        }
        let last_sheet_name = chars[last_start..cursor].iter().collect();
        (first_sheet_name, last_sheet_name, cursor)
    };
    if chars.get(bang) != Some(&'!') {
        return None;
    }
    Some(ThreeDimensionalSheetQualifier {
        first_sheet_name,
        last_sheet_name,
        reference_start: bang + 1,
    })
}

fn parse_sheet_qualified_reference(chars: &[char], idx: usize) -> Option<SheetQualifiedReference> {
    let qualifier = parse_sheet_qualifier(chars, idx)?;
    let reference = parse_formula_reference(chars, qualifier.reference_start)?;
    Some(SheetQualifiedReference {
        sheet_name: qualifier.sheet_name,
        reference,
    })
}

fn parse_sheet_qualified_axis_range(chars: &[char], idx: usize) -> Option<SheetQualifiedAxisRange> {
    let qualifier = parse_sheet_qualifier(chars, idx)?;
    let range = parse_formula_axis_range(chars, qualifier.reference_start)?;
    Some(SheetQualifiedAxisRange {
        sheet_name: qualifier.sheet_name,
        range,
    })
}

fn parse_formula_reference(chars: &[char], idx: usize) -> Option<FormulaReference> {
    let first = *chars.get(idx)?;
    if !(first == '$' || first.is_ascii_alphabetic()) {
        return None;
    }
    if idx > 0 && (chars[idx - 1].is_ascii_alphanumeric() || matches!(chars[idx - 1], '_' | '.')) {
        return None;
    }
    let mut cursor = idx;
    let absolute_column = chars[cursor] == '$';
    if absolute_column {
        cursor += 1;
    }
    let column_start = cursor;
    while cursor < chars.len() && chars[cursor].is_ascii_alphabetic() {
        cursor += 1;
    }
    if cursor == column_start {
        return None;
    }
    let absolute_row = cursor < chars.len() && chars[cursor] == '$';
    if absolute_row {
        cursor += 1;
    }
    let row_start = cursor;
    while cursor < chars.len() && chars[cursor].is_ascii_digit() {
        cursor += 1;
    }
    if cursor == row_start
        || chars
            .get(cursor)
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '(' | '!'))
    {
        return None;
    }
    let row = chars[row_start..cursor]
        .iter()
        .collect::<String>()
        .parse::<usize>()
        .ok()?;
    let column_text: String = chars[column_start..row_start]
        .iter()
        .filter(|ch| ch.is_ascii_alphabetic())
        .collect();
    let column = column_index(&column_text)?;
    if row == 0 || row > 1_048_576 || column > 16_384 {
        return None;
    }
    Some(FormulaReference {
        end: cursor,
        row,
        column,
        absolute_row,
        absolute_column,
    })
}

fn parse_formula_axis_range(chars: &[char], idx: usize) -> Option<FormulaAxisRange> {
    if idx > 0 && (chars[idx - 1].is_ascii_alphanumeric() || matches!(chars[idx - 1], '_' | '.')) {
        return None;
    }
    let mut cursor = idx;
    let absolute_first = chars.get(cursor) == Some(&'$');
    if absolute_first {
        cursor += 1;
    }
    let first_start = cursor;
    let axis = match chars.get(cursor)? {
        ch if ch.is_ascii_alphabetic() => {
            while chars.get(cursor).is_some_and(char::is_ascii_alphabetic) {
                cursor += 1;
            }
            FormulaAxis::Column
        }
        ch if ch.is_ascii_digit() => {
            while chars.get(cursor).is_some_and(char::is_ascii_digit) {
                cursor += 1;
            }
            FormulaAxis::Row
        }
        _ => return None,
    };
    if chars.get(cursor) != Some(&':') {
        return None;
    }
    let first_text: String = chars[first_start..cursor].iter().collect();
    cursor += 1;
    let absolute_second = chars.get(cursor) == Some(&'$');
    if absolute_second {
        cursor += 1;
    }
    let second_start = cursor;
    match axis {
        FormulaAxis::Column => {
            while chars.get(cursor).is_some_and(char::is_ascii_alphabetic) {
                cursor += 1;
            }
        }
        FormulaAxis::Row => {
            while chars.get(cursor).is_some_and(char::is_ascii_digit) {
                cursor += 1;
            }
        }
    }
    if cursor == second_start
        || chars
            .get(cursor)
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '!'))
    {
        return None;
    }
    let second_text: String = chars[second_start..cursor].iter().collect();
    let (first, second) = match axis {
        FormulaAxis::Column => (column_index(&first_text)?, column_index(&second_text)?),
        FormulaAxis::Row => (
            first_text.parse::<usize>().ok()?,
            second_text.parse::<usize>().ok()?,
        ),
    };
    let valid = match axis {
        FormulaAxis::Column => first <= 16_384 && second <= 16_384,
        FormulaAxis::Row => first > 0 && first <= 1_048_576 && second > 0 && second <= 1_048_576,
    };
    valid.then_some(FormulaAxisRange {
        end: cursor,
        axis,
        first,
        second,
        absolute_first,
        absolute_second,
    })
}

fn write_formula_axis_range(
    output: &mut String,
    range: &FormulaAxisRange,
    mapper: Option<&CoordinateMapper>,
) {
    let (first, second) = match range.axis {
        FormulaAxis::Column => (
            mapper
                .map(|mapper| mapper.map_column_confident(range.first))
                .unwrap_or(range.first),
            mapper
                .map(|mapper| mapper.map_column_confident(range.second))
                .unwrap_or(range.second),
        ),
        FormulaAxis::Row => (
            mapper
                .map(|mapper| mapper.map_row_confident(range.first))
                .unwrap_or(range.first),
            mapper
                .map(|mapper| mapper.map_row_confident(range.second))
                .unwrap_or(range.second),
        ),
    };
    if range.absolute_first {
        output.push('$');
    }
    match range.axis {
        FormulaAxis::Column => output.push_str(&column_name(first - 1)),
        FormulaAxis::Row => output.push_str(&first.to_string()),
    }
    output.push(':');
    if range.absolute_second {
        output.push('$');
    }
    match range.axis {
        FormulaAxis::Column => output.push_str(&column_name(second - 1)),
        FormulaAxis::Row => output.push_str(&second.to_string()),
    }
}

fn protected_segment(chars: &[char], idx: usize) -> Option<(usize, String)> {
    if chars[idx] == '[' {
        let mut cursor = idx + 1;
        let mut depth = 1usize;
        while cursor < chars.len() {
            match chars[cursor] {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        cursor += 1;
                        break;
                    }
                }
                _ => {}
            }
            cursor += 1;
        }
        return Some((cursor, chars[idx..cursor].iter().collect()));
    }
    let closing = match chars[idx] {
        '\'' => '\'',
        '"' => '"',
        _ => return None,
    };
    let mut cursor = idx + 1;
    while cursor < chars.len() {
        if chars[cursor] == closing {
            if matches!(closing, '\'' | '"') && chars.get(cursor + 1) == Some(&closing) {
                cursor += 2;
                continue;
            }
            cursor += 1;
            break;
        }
        cursor += 1;
    }
    Some((cursor, chars[idx..cursor].iter().collect()))
}

fn is_structured_reference_start(chars: &[char], idx: usize) -> bool {
    if chars.get(idx) != Some(&'[') {
        return false;
    }
    let follows_table_name =
        idx > 0 && (chars[idx - 1].is_alphanumeric() || matches!(chars[idx - 1], '_' | '.' | ']'));
    let has_structured_marker = chars
        .get(idx + 1)
        .is_some_and(|ch| matches!(ch, '@' | '#' | '['));
    follows_table_name || has_structured_marker
}

fn column_index(column: &str) -> Option<usize> {
    let mut index = 0usize;
    for ch in column.chars() {
        if !ch.is_ascii_alphabetic() {
            return None;
        }
        index = index
            .checked_mul(26)?
            .checked_add((ch.to_ascii_uppercase() as u8 - b'A' + 1) as usize)?;
    }
    Some(index)
}

/// Cache row features only when exact anchors cannot settle a pairing. A
/// changed region may compare each row hundreds of times; rebuilding signatures
/// and value-count trees for every candidate dominated alignment time.
struct RowProfile<'a> {
    row: &'a [CellIr],
    signature: Option<String>,
    // Keep exact-anchor rows small; allocate the richer cache only for rows
    // participating in scored comparisons.
    features: OnceLock<Box<RowFeatures<'a>>>,
}

struct RowFeatures<'a> {
    values: Vec<&'a str>,
    counts: BTreeMap<&'a str, usize>,
    key: Option<String>,
}

impl<'a> RowProfile<'a> {
    fn new(row: &'a [CellIr]) -> Self {
        Self {
            row,
            signature: exact_row_signature(row),
            features: OnceLock::new(),
        }
    }

    fn features(&self) -> &RowFeatures<'a> {
        self.features.get_or_init(|| {
            let values: Vec<_> = self.row.iter().map(cell_signature_text).collect();
            Box::new(RowFeatures {
                counts: value_counts(&values),
                values,
                key: key_signature(self.row),
            })
        })
    }

    fn similarity(&self, other: &Self) -> f64 {
        match (&self.signature, &other.signature) {
            (None, None) => return 1.0,
            (None, Some(_)) | (Some(_), None) => return 0.0,
            (Some(old), Some(new)) if old == new => return 1.0,
            _ => {}
        }
        let old = self.features();
        let new = other.features();
        let positional = positional_similarity(&old.values, &new.values);
        let bag = count_similarity(&old.counts, &new.counts);
        let shape = cell_shape_similarity(self.row, other.row);
        let key_bonus = if old.key.is_some() && old.key == new.key {
            0.25
        } else {
            0.0
        };
        (positional * 0.35 + bag * 0.55 + shape * 0.35 + key_bonus).min(1.0)
    }
}

#[cfg(test)]
fn row_similarity(old_row: &[CellIr], new_row: &[CellIr]) -> f64 {
    match (all_empty_cells(old_row), all_empty_cells(new_row)) {
        (true, true) => return 1.0,
        (true, false) | (false, true) => return 0.0,
        (false, false) => {}
    }

    let old_sig = exact_row_signature(old_row);
    let new_sig = exact_row_signature(new_row);
    if old_sig == new_sig && old_sig.is_some() {
        return 1.0;
    }

    let old_values: Vec<_> = old_row.iter().map(cell_signature_text).collect();
    let new_values: Vec<_> = new_row.iter().map(cell_signature_text).collect();
    let positional = positional_similarity(&old_values, &new_values);
    let bag = bag_similarity(&old_values, &new_values);
    let shape = cell_shape_similarity(old_row, new_row);
    let key_bonus =
        if key_signature(old_row) == key_signature(new_row) && key_signature(old_row).is_some() {
            0.25
        } else {
            0.0
        };

    // Bag overlap carries more weight than position so inserting a populated
    // column doesn't make every existing row look unrelated.
    // A shared shape supports content evidence but is intentionally below the
    // minimum match threshold by itself: arbitrary string rows are common and
    // must not hide a genuine reorder.
    (positional * 0.35 + bag * 0.55 + shape * 0.35 + key_bonus).min(1.0)
}

fn column_similarity(old_col: &[&CellIr], new_col: &[&CellIr]) -> f64 {
    match (
        old_col.iter().all(|cell| is_empty_cell(cell)),
        new_col.iter().all(|cell| is_empty_cell(cell)),
    ) {
        (true, true) => return 1.0,
        (true, false) | (false, true) => return 0.0,
        (false, false) => {}
    }

    let old_header = column_header(old_col);
    let new_header = column_header(new_col);
    let old_signature = exact_column_signature(old_col);
    let new_signature = exact_column_signature(new_col);
    if old_signature == new_signature && old_signature.is_some() {
        return 1.0;
    }

    let old_values: Vec<_> = old_col
        .iter()
        .map(|cell| cell_signature_text(cell))
        .collect();
    let new_values: Vec<_> = new_col
        .iter()
        .map(|cell| cell_signature_text(cell))
        .collect();
    let positional = positional_similarity(&old_values, &new_values);
    let bag = bag_similarity(&old_values, &new_values);
    let shape = cell_shape_similarity_refs(old_col, new_col);
    let header_bonus = if !old_header.is_empty() && old_header == new_header {
        0.55
    } else {
        0.0
    };

    // Content overlap keeps a column identifiable even when rows were inserted
    // above it and the first visible value is no longer the same header.
    (positional * 0.20 + bag * 0.65 + shape * 0.35 + header_bonus).min(1.0)
}

fn cell_shape_similarity_refs(old_cells: &[&CellIr], new_cells: &[&CellIr]) -> f64 {
    cell_shape_similarity_by(old_cells, new_cells, |cell| cell)
}

fn cell_shape_similarity(old_cells: &[CellIr], new_cells: &[CellIr]) -> f64 {
    cell_shape_similarity_by(old_cells, new_cells, |cell| cell)
}

fn cell_shape_similarity_by<T>(
    old_cells: &[T],
    new_cells: &[T],
    as_cell: impl Fn(&T) -> &CellIr,
) -> f64 {
    let max_len = old_cells.len().max(new_cells.len());
    let mut active_positions = 0usize;
    let mut matches = 0usize;
    for idx in 0..max_len {
        let old = old_cells.get(idx);
        let new = new_cells.get(idx);
        let old_active = old.is_some_and(|cell| {
            let cell = as_cell(cell);
            cell.value_kind != CellValueKind::Empty || cell.formula.is_some()
        });
        let new_active = new.is_some_and(|cell| {
            let cell = as_cell(cell);
            cell.value_kind != CellValueKind::Empty || cell.formula.is_some()
        });
        if !old_active && !new_active {
            continue;
        }
        active_positions += 1;
        if old.zip(new).is_some_and(|(old, new)| {
            let old = as_cell(old);
            let new = as_cell(new);
            old.value_kind == new.value_kind && old.formula.is_some() == new.formula.is_some()
        }) {
            matches += 1;
        }
    }
    if active_positions == 0 {
        1.0
    } else {
        // This is a per-position ratio. Dividing by the square of the number
        // of active positions made an otherwise identical type/formula shape
        // less similar merely because the row or column was longer. In
        // particular, replacing every value in a two-cell string row scored
        // only 0.2 overall and was misclassified as a remove/add pair.
        matches as f64 / active_positions as f64
    }
}

fn positional_similarity(old_values: &[&str], new_values: &[&str]) -> f64 {
    let max_len = old_values.len().max(new_values.len());
    if max_len == 0 {
        return 1.0;
    }
    let matches = (0..max_len)
        .filter(|idx| {
            let old = old_values.get(*idx).copied().unwrap_or_default();
            let new = new_values.get(*idx).copied().unwrap_or_default();
            !old.is_empty() && old == new
        })
        .count();
    matches as f64 / max_len as f64
}

fn bag_similarity(old_values: &[&str], new_values: &[&str]) -> f64 {
    let old_counts = value_counts(old_values);
    let new_counts = value_counts(new_values);
    count_similarity(&old_counts, &new_counts)
}

fn count_similarity(old_counts: &BTreeMap<&str, usize>, new_counts: &BTreeMap<&str, usize>) -> f64 {
    // Multiset union = total occurrences on both sides - intersection.
    // Avoid allocating a third tree and looking up every union key twice for
    // each candidate row/column pair in the alignment matrix.
    let intersection: usize = old_counts
        .iter()
        .map(|(key, old)| (*old).min(new_counts.get(key).copied().unwrap_or(0)))
        .sum();
    let union =
        old_counts.values().sum::<usize>() + new_counts.values().sum::<usize>() - intersection;
    if union == 0 {
        1.0
    } else {
        intersection as f64 / union as f64
    }
}

fn value_counts<'a>(values: &[&'a str]) -> BTreeMap<&'a str, usize> {
    let mut counts = BTreeMap::new();
    for value in values {
        if !value.is_empty() {
            *counts.entry(*value).or_insert(0) += 1;
        }
    }
    counts
}

fn sheet_similarity(old_sheet: &SheetIr, new_sheet: &SheetIr) -> f64 {
    let old_values = flatten_non_empty(old_sheet);
    let new_values = flatten_non_empty(new_sheet);
    bag_similarity(
        &old_values.iter().map(String::as_str).collect::<Vec<_>>(),
        &new_values.iter().map(String::as_str).collect::<Vec<_>>(),
    )
}

fn flatten_non_empty(sheet: &SheetIr) -> Vec<String> {
    sheet
        .rows
        .iter()
        .flat_map(|row| row.iter())
        .filter(|cell| !is_empty_cell(cell))
        .map(|cell| {
            format!(
                "{:?}:{}:{}",
                cell.value_kind,
                cell.value,
                cell.formula
                    .as_deref()
                    .map(|formula| canonical_formula(formula, None))
                    .unwrap_or_default()
            )
        })
        .collect()
}

fn columns_from_rows(rows: &[Vec<CellIr>]) -> Vec<Vec<&CellIr>> {
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    (0..width)
        .map(|col_idx| {
            rows.iter()
                .map(|row| row.get(col_idx).unwrap_or(&EMPTY_CELL))
                .collect()
        })
        .collect()
}

#[derive(Debug, Clone)]
struct LocalRange {
    start_row: usize,
    start_col: usize,
    end_row: usize,
    end_col: usize,
}

fn absolute_range(sheet: &SheetIr, range: &LocalRange) -> CellRange {
    CellRange {
        start_row: sheet.start_row + range.start_row,
        start_col: sheet.start_col + range.start_col,
        end_row: sheet.start_row + range.end_row,
        end_col: sheet.start_col + range.end_col,
    }
}

fn diff_table_regions(
    old_sheet: &SheetIr,
    new_sheet: &SheetIr,
    options: &DiffOptions,
    workbook_mapper: Option<&WorkbookCoordinateMapper>,
    style_defaults: StyleDefaults<'_>,
    full_sheet_area: Option<&AreaDiff>,
) -> Result<Vec<TableRegionDiff>> {
    let old_regions = detect_table_regions(&old_sheet.rows);
    let new_regions = detect_table_regions(&new_sheet.rows);
    // A dense worksheet is also one table region. Reusing its completed area
    // diff avoids cloning and comparing every cell a second time.
    if let (Some(area), [old_range], [new_range]) = (
        full_sheet_area,
        old_regions.as_slice(),
        new_regions.as_slice(),
    ) && old_sheet.start_row == new_sheet.start_row
        && old_sheet.start_col == new_sheet.start_col
        && range_covers_rows(old_range, &old_sheet.rows)
        && range_covers_rows(new_range, &new_sheet.rows)
    {
        let status = if area.row_changes.is_empty()
            && area.column_changes.is_empty()
            && area.cell_changes.is_empty()
        {
            RegionStatus::Unchanged
        } else {
            RegionStatus::Modified
        };
        return Ok(vec![TableRegionDiff {
            old_range: Some(absolute_range(old_sheet, old_range)),
            new_range: Some(absolute_range(new_sheet, new_range)),
            status,
            alignment: area.alignment.clone(),
            row_changes: area.row_changes.clone(),
            column_changes: area.column_changes.clone(),
            cell_changes: area.cell_changes.clone(),
        }]);
    }
    let old_candidates: Vec<_> = old_regions
        .iter()
        .cloned()
        .map(|range| {
            let signature = region_exact_signature(old_sheet, &range);
            (range, signature)
        })
        .collect();
    let new_candidates: Vec<_> = new_regions
        .iter()
        .cloned()
        .map(|range| {
            let signature = region_exact_signature(new_sheet, &range);
            (range, signature)
        })
        .collect();
    let ops = align_sequences(
        &old_candidates,
        &new_candidates,
        |old_region, new_region| {
            region_similarity(old_sheet, &old_region.0, new_sheet, &new_region.0)
        },
        |region| region.1.clone(),
        options,
    )?
    .ops;

    ops.into_iter()
        .map(|op| -> Result<TableRegionDiff> {
            Ok(match op {
                MatchOp::Match(old_idx, new_idx, _) => {
                    let old_range = old_regions[old_idx].clone();
                    let new_range = new_regions[new_idx].clone();
                    let old_rows = extract_range(&old_sheet.rows, &old_range);
                    let new_rows = extract_range(&new_sheet.rows, &new_range);
                    let area = diff_area_internal(
                        &old_rows,
                        &new_rows,
                        options,
                        AreaContext {
                            old_offset: (
                                old_sheet.start_row - 1 + old_range.start_row,
                                old_sheet.start_col - 1 + old_range.start_col,
                            ),
                            new_offset: (
                                new_sheet.start_row - 1 + new_range.start_row,
                                new_sheet.start_col - 1 + new_range.start_col,
                            ),
                            workbook_mapper,
                            style_defaults,
                        },
                        true,
                    )?;
                    let status = if area.row_changes.is_empty()
                        && area.column_changes.is_empty()
                        && area.cell_changes.is_empty()
                    {
                        RegionStatus::Unchanged
                    } else {
                        RegionStatus::Modified
                    };
                    TableRegionDiff {
                        old_range: Some(absolute_range(old_sheet, &old_range)),
                        new_range: Some(absolute_range(new_sheet, &new_range)),
                        status,
                        alignment: area.alignment,
                        row_changes: area.row_changes,
                        column_changes: area.column_changes,
                        cell_changes: area.cell_changes,
                    }
                }
                MatchOp::Removed(old_idx) => TableRegionDiff {
                    old_range: Some(absolute_range(old_sheet, &old_regions[old_idx])),
                    new_range: None,
                    status: RegionStatus::Removed,
                    alignment: AlignmentDiagnostics::default(),
                    row_changes: Vec::new(),
                    column_changes: Vec::new(),
                    cell_changes: Vec::new(),
                },
                MatchOp::Added(new_idx) => TableRegionDiff {
                    old_range: None,
                    new_range: Some(absolute_range(new_sheet, &new_regions[new_idx])),
                    status: RegionStatus::Added,
                    alignment: AlignmentDiagnostics::default(),
                    row_changes: Vec::new(),
                    column_changes: Vec::new(),
                    cell_changes: Vec::new(),
                },
            })
        })
        .collect()
}

fn range_covers_rows(range: &LocalRange, rows: &[Vec<CellIr>]) -> bool {
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    range.start_row == 0
        && range.start_col == 0
        && range.end_row + 1 == rows.len()
        && range.end_col + 1 == width
}

fn detect_table_regions(rows: &[Vec<CellIr>]) -> Vec<LocalRange> {
    if rows.is_empty() {
        return Vec::new();
    }
    if let Some(region) = dense_full_sheet_region(rows) {
        return vec![region];
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut visited = vec![vec![false; width]; rows.len()];
    let mut regions = Vec::new();
    for start_row in 0..rows.len() {
        for start_col in 0..width {
            if visited[start_row][start_col]
                || rows
                    .get(start_row)
                    .and_then(|row| row.get(start_col))
                    .is_none_or(is_empty_cell)
            {
                continue;
            }

            let mut queue = VecDeque::from([(start_row, start_col)]);
            visited[start_row][start_col] = true;
            let (mut min_row, mut max_row) = (start_row, start_row);
            let (mut min_col, mut max_col) = (start_col, start_col);
            while let Some((row, col)) = queue.pop_front() {
                min_row = min_row.min(row);
                max_row = max_row.max(row);
                min_col = min_col.min(col);
                max_col = max_col.max(col);
                for row_delta in -1isize..=1 {
                    for col_delta in -1isize..=1 {
                        if row_delta == 0 && col_delta == 0 {
                            continue;
                        }
                        let Some(next_row) = row.checked_add_signed(row_delta) else {
                            continue;
                        };
                        let Some(next_col) = col.checked_add_signed(col_delta) else {
                            continue;
                        };
                        if next_row >= rows.len()
                            || next_col >= width
                            || visited[next_row][next_col]
                            || rows
                                .get(next_row)
                                .and_then(|next| next.get(next_col))
                                .is_none_or(is_empty_cell)
                        {
                            continue;
                        }
                        visited[next_row][next_col] = true;
                        queue.push_back((next_row, next_col));
                    }
                }
            }
            regions.push(LocalRange {
                start_row: min_row,
                start_col: min_col,
                end_row: max_row,
                end_col: max_col,
            });
        }
    }
    regions.sort_by_key(|range| {
        (
            range.start_row,
            range.start_col,
            range.end_row,
            range.end_col,
        )
    });
    regions
}

fn dense_full_sheet_region(rows: &[Vec<CellIr>]) -> Option<LocalRange> {
    let width = rows.first()?.len();
    if width == 0
        || rows
            .iter()
            .any(|row| row.len() != width || row.iter().any(is_empty_cell))
    {
        return None;
    }
    Some(LocalRange {
        start_row: 0,
        start_col: 0,
        end_row: rows.len() - 1,
        end_col: width - 1,
    })
}

fn extract_range(rows: &[Vec<CellIr>], range: &LocalRange) -> Vec<Vec<CellIr>> {
    (range.start_row..=range.end_row)
        .map(|row_idx| {
            (range.start_col..=range.end_col)
                .map(|col_idx| get_cell(rows, row_idx, col_idx).clone())
                .collect()
        })
        .collect()
}

fn region_similarity(
    old_sheet: &SheetIr,
    old_range: &LocalRange,
    new_sheet: &SheetIr,
    new_range: &LocalRange,
) -> f64 {
    let old_rows = extract_range(&old_sheet.rows, old_range);
    let new_rows = extract_range(&new_sheet.rows, new_range);
    let old_values = flatten_rows_non_empty(&old_rows);
    let new_values = flatten_rows_non_empty(&new_rows);
    let content = bag_similarity(
        &old_values.iter().map(String::as_str).collect::<Vec<_>>(),
        &new_values.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    let position = if old_range.start_row == new_range.start_row
        || old_range.start_col == new_range.start_col
    {
        0.2
    } else {
        0.0
    };
    (content * 0.8 + position).min(1.0)
}

fn region_exact_signature(sheet: &SheetIr, range: &LocalRange) -> Option<String> {
    let rows = extract_range(&sheet.rows, range);
    if rows.iter().flatten().all(is_empty_cell) {
        return None;
    }
    Some(format!(
        "{}x{}:{}",
        range.end_row - range.start_row + 1,
        range.end_col - range.start_col + 1,
        rows.iter()
            .map(|row| exact_cells_signature(row).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\u{1e}")
    ))
}

fn flatten_rows_non_empty(rows: &[Vec<CellIr>]) -> Vec<String> {
    rows.iter()
        .flat_map(|row| row.iter())
        .map(cell_signature_text)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn diff_merge_ranges(
    old_ranges: &[CellRange],
    new_ranges: &[CellRange],
    mapper: &CoordinateMapper,
) -> Vec<MergeChange> {
    let old_expected: BTreeMap<_, _> = old_ranges
        .iter()
        .map(|range| {
            let expected = CellRange {
                start_row: mapper.map_row_start(range.start_row),
                start_col: mapper.map_column_start(range.start_col),
                end_row: mapper.map_row_end(range.end_row),
                end_col: mapper.map_column_end(range.end_col),
            };
            (expected, range.clone())
        })
        .collect();
    let new_set: BTreeSet<_> = new_ranges.iter().cloned().collect();
    let mut changes = Vec::new();

    for (expected, original) in &old_expected {
        if !new_set.contains(expected) {
            changes.push(MergeChange {
                kind: ChangeKind::Removed,
                old_range: Some(original.clone()),
                new_range: None,
            });
        }
    }
    for range in new_set
        .iter()
        .filter(|range| !old_expected.contains_key(*range))
    {
        changes.push(MergeChange {
            kind: ChangeKind::Added,
            old_range: None,
            new_range: Some(range.clone()),
        });
    }

    changes
}

static EMPTY_CELL: CellIr = CellIr {
    value: String::new(),
    value_kind: CellValueKind::Empty,
    formula: None,
    style_signature: String::new(),
};

fn get_cell(rows: &[Vec<CellIr>], row_idx: usize, col_idx: usize) -> &CellIr {
    rows.get(row_idx)
        .and_then(|row| row.get(col_idx))
        .unwrap_or(&EMPTY_CELL)
}

fn trim_empty_edges(rows: &mut Vec<Vec<CellIr>>, start_row: &mut usize, start_col: &mut usize) {
    let leading_rows = rows
        .iter()
        .take_while(|row| row.iter().all(is_empty_cell))
        .count();
    if leading_rows > 0 {
        rows.drain(..leading_rows);
        *start_row += leading_rows;
    }
    while rows.last().is_some_and(|row| row.iter().all(is_empty_cell)) {
        rows.pop();
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let leading_cols = (0..width)
        .take_while(|col_idx| {
            rows.iter()
                .all(|row| row.get(*col_idx).is_none_or(is_empty_cell))
        })
        .count();
    if leading_cols > 0 {
        for row in rows.iter_mut() {
            row.drain(..leading_cols.min(row.len()));
        }
        *start_col += leading_cols;
    }
    let mut keep_width = width.saturating_sub(leading_cols);
    while keep_width > 0
        && rows
            .iter()
            .all(|row| row.get(keep_width - 1).is_none_or(is_empty_cell))
    {
        keep_width -= 1;
    }
    for row in rows {
        row.truncate(keep_width);
    }
}

fn is_empty_cell(cell: &CellIr) -> bool {
    cell.value.is_empty() && cell.formula.as_deref().unwrap_or_default().is_empty()
}

fn cell_values_equal(old: &CellIr, new: &CellIr) -> bool {
    old.value == new.value && old.value_kind == new.value_kind
}

#[cfg(test)]
fn all_empty_cells(cells: &[CellIr]) -> bool {
    cells.iter().all(is_empty_cell)
}

fn exact_row_signature(row: &[CellIr]) -> Option<String> {
    exact_cells_signature(row)
}

fn exact_column_signature(column: &[&CellIr]) -> Option<String> {
    exact_cell_refs_signature(column)
}

fn column_alignment_signature(column: &[&CellIr]) -> Option<String> {
    let first = column.first()?;
    if !is_empty_cell(first) {
        // Unique headers are stable column identities even when a row insertion
        // changes the full-column signature. Duplicate headers remain ambiguous
        // and therefore fall back to the bounded similarity alignment.
        return Some(format!(
            "header:{:?}:{}:{}",
            first.value_kind,
            cell_signature_text(first),
            first.formula.as_deref().unwrap_or_default()
        ));
    }
    exact_column_signature(column)
}

fn exact_cell_refs_signature(cells: &[&CellIr]) -> Option<String> {
    if cells.iter().all(|cell| is_empty_cell(cell)) {
        return None;
    }
    Some(
        cells
            .iter()
            .map(|cell| exact_cell_signature(cell))
            .collect::<Vec<_>>()
            .join("\u{1f}"),
    )
}

fn exact_cells_signature(cells: &[CellIr]) -> Option<String> {
    if cells.iter().all(is_empty_cell) {
        return None;
    }
    Some(
        cells
            .iter()
            .map(exact_cell_signature)
            .collect::<Vec<_>>()
            .join("\u{1f}"),
    )
}

fn exact_cell_signature(cell: &CellIr) -> String {
    format!(
        "{:?}:{}:{}:{}",
        cell.value_kind,
        cell.value.len(),
        cell.value,
        cell.formula
            .as_deref()
            .map(|formula| canonical_formula(formula, None))
            .unwrap_or_default()
    )
}

fn key_signature(row: &[CellIr]) -> Option<String> {
    row.iter()
        .map(cell_signature_text)
        .find(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn row_preview(row: &[CellIr]) -> String {
    row.iter()
        .map(cell_signature_text)
        .filter(|value| !value.is_empty())
        .take(5)
        .collect::<Vec<_>>()
        .join(" / ")
}

fn column_header(column: &[&CellIr]) -> String {
    column
        .iter()
        .find_map(|cell| {
            let text = cell_signature_text(cell);
            (!text.is_empty()).then(|| text.to_string())
        })
        .unwrap_or_default()
}

fn cell_signature_text(cell: &CellIr) -> &str {
    if !cell.value.is_empty() {
        &cell.value
    } else {
        cell.formula.as_deref().unwrap_or_default()
    }
}

#[cfg(test)]
fn normalize_value(value: &Data) -> String {
    normalize_value_with_policy(value, WhitespaceNormalization::Collapse)
}

#[cfg(test)]
fn normalize_value_with_policy(value: &Data, whitespace: WhitespaceNormalization) -> String {
    match value {
        Data::Empty => String::new(),
        Data::String(value) => normalize_text(value, whitespace),
        Data::Float(value) => normalize_number(*value),
        Data::Int(value) => value.to_string(),
        Data::Bool(value) => value.to_string(),
        Data::Error(value) => format!("{value:?}"),
        Data::DateTime(value) if value.is_duration() => normalize_excel_duration(value.as_f64()),
        Data::DateTime(value) => normalize_excel_datetime(value),
        Data::DateTimeIso(text) => {
            normalize_iso_datetime(value).unwrap_or_else(|| normalize_text(text, whitespace))
        }
        Data::DurationIso(text) => {
            normalize_iso_duration(value).unwrap_or_else(|| normalize_text(text, whitespace))
        }
    }
}

#[cfg(test)]
fn value_kind(value: &Data) -> CellValueKind {
    match value {
        Data::Empty => CellValueKind::Empty,
        Data::String(_) => CellValueKind::String,
        Data::Float(_) | Data::Int(_) => CellValueKind::Number,
        Data::Bool(_) => CellValueKind::Boolean,
        Data::Error(_) => CellValueKind::Error,
        Data::DateTime(value) if value.is_duration() => CellValueKind::Duration,
        Data::DateTime(_) | Data::DateTimeIso(_) => CellValueKind::DateTime,
        Data::DurationIso(_) => CellValueKind::Duration,
    }
}

fn normalize_excel_datetime(value: &calamine::ExcelDateTime) -> String {
    let (year, month, day, hour, minute, second, millisecond) = value.to_ymd_hms_milli();
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millisecond:03}")
}

fn normalize_iso_datetime(value: &Data) -> Option<String> {
    if let Some(datetime) = value.as_datetime() {
        return Some(normalize_naive_datetime_text(&datetime.to_string()));
    }
    value.as_date().map(|date| format!("{date}T00:00:00.000"))
}

fn normalize_naive_datetime_text(value: &str) -> String {
    let value = value.replacen(' ', "T", 1);
    let Some((whole_seconds, fraction)) = value.split_once('.') else {
        return format!("{value}.000");
    };
    if fraction.len() >= 3 {
        value
    } else {
        format!("{whole_seconds}.{fraction:0<3}")
    }
}

fn normalize_excel_duration(serial: f64) -> String {
    let milliseconds = (serial * 86_400_000.0).round() as i64;
    format!("DURATION_MS:{milliseconds}")
}

fn normalize_iso_duration(value: &Data) -> Option<String> {
    let Data::DurationIso(value) = value else {
        return None;
    };
    parse_iso_duration_milliseconds(value).map(|milliseconds| format!("DURATION_MS:{milliseconds}"))
}

fn parse_iso_duration_milliseconds(value: &str) -> Option<i64> {
    let (sign, value) = if let Some(value) = value.strip_prefix('-') {
        (-1i128, value)
    } else {
        (1i128, value.strip_prefix('+').unwrap_or(value))
    };
    let value = value.strip_prefix('P')?;
    let mut in_time = false;
    let mut saw_component = false;
    let mut saw_time_component = false;
    let mut saw_week = false;
    let mut last_date_rank = 0u8;
    let mut last_time_rank = 0u8;
    let mut number = String::new();
    let mut total_milliseconds = 0i128;
    for ch in value.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            number.push(ch);
            continue;
        }
        if ch == 'T' {
            if in_time || !number.is_empty() || saw_week {
                return None;
            }
            in_time = true;
            continue;
        }
        if number.is_empty() {
            return None;
        }
        let (multiplier, rank, is_week) = match (in_time, ch) {
            (false, 'W') => (7 * 86_400_000, 1, true),
            (false, 'D') => (86_400_000, 2, false),
            (true, 'H') => (3_600_000, 1, false),
            (true, 'M') => (60_000, 2, false),
            (true, 'S') => (1_000, 3, false),
            _ => return None,
        };
        if in_time {
            if rank <= last_time_rank {
                return None;
            }
            last_time_rank = rank;
        } else if is_week {
            if saw_component {
                return None;
            }
            saw_week = true;
        } else {
            if saw_week || rank <= last_date_rank {
                return None;
            }
            last_date_rank = rank;
        }
        let component = number.parse::<f64>().ok()?;
        if !component.is_finite() || component < 0.0 {
            return None;
        }
        let component_milliseconds = component * f64::from(multiplier);
        if !component_milliseconds.is_finite() || component_milliseconds > i64::MAX as f64 {
            return None;
        }
        total_milliseconds =
            total_milliseconds.checked_add(component_milliseconds.round() as i128)?;
        number.clear();
        saw_component = true;
        saw_time_component |= in_time;
    }
    if !number.is_empty() || !saw_component || in_time && !saw_time_component {
        return None;
    }
    i64::try_from(total_milliseconds.checked_mul(sign)?).ok()
}

fn normalize_text(value: &str, whitespace: WhitespaceNormalization) -> String {
    match whitespace {
        WhitespaceNormalization::Preserve => value.to_string(),
        WhitespaceNormalization::Collapse => value
            .chars()
            .filter(|character| *character != '\u{ad}')
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .nfc()
            .collect(),
    }
}

fn normalize_number(value: f64) -> String {
    if value == 0.0 {
        "0".to_string()
    } else if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

fn address(row_idx: usize, col_idx: usize) -> String {
    format!("{}{}", column_name(col_idx), row_idx + 1)
}

fn column_name(mut col_idx: usize) -> String {
    let mut name = String::new();
    col_idx += 1;
    while col_idx > 0 {
        let rem = (col_idx - 1) % 26;
        name.insert(0, (b'A' + rem as u8) as char);
        col_idx = (col_idx - 1) / 26;
    }
    name
}

fn accumulate_summary(summary: &mut DiffSummary, sheet: &SheetDiff) {
    summary.rows_added += sheet
        .row_changes
        .iter()
        .filter(|change| matches!(change.kind, ChangeKind::Added))
        .count();
    summary.rows_removed += sheet
        .row_changes
        .iter()
        .filter(|change| matches!(change.kind, ChangeKind::Removed))
        .count();
    summary.rows_moved += sheet
        .row_changes
        .iter()
        .filter(|change| matches!(change.kind, ChangeKind::Moved))
        .count();
    summary.columns_added += sheet
        .column_changes
        .iter()
        .filter(|change| matches!(change.kind, ChangeKind::Added))
        .count();
    summary.columns_removed += sheet
        .column_changes
        .iter()
        .filter(|change| matches!(change.kind, ChangeKind::Removed))
        .count();
    summary.columns_moved += sheet
        .column_changes
        .iter()
        .filter(|change| matches!(change.kind, ChangeKind::Moved))
        .count();
    summary.cells_changed += sheet.cell_changes.len();
    summary.formulas_changed += sheet
        .cell_changes
        .iter()
        .filter(|change| {
            matches!(
                change.kind,
                CellChangeKind::FormulaChanged | CellChangeKind::ValueAndFormulaChanged
            )
        })
        .count();
    summary.cell_styles_changed += sheet
        .cell_changes
        .iter()
        .filter(|change| change.style_changed)
        .count();
    summary.table_regions_added += sheet
        .table_regions
        .iter()
        .filter(|region| matches!(region.status, RegionStatus::Added))
        .count();
    summary.table_regions_removed += sheet
        .table_regions
        .iter()
        .filter(|region| matches!(region.status, RegionStatus::Removed))
        .count();
    summary.merged_cells_added += sheet
        .merge_changes
        .iter()
        .filter(|change| matches!(change.kind, ChangeKind::Added))
        .count();
    summary.merged_cells_removed += sheet
        .merge_changes
        .iter()
        .filter(|change| matches!(change.kind, ChangeKind::Removed))
        .count();
}

/// Write an HTML report using the default merged layout.
pub fn write_html_report(path: &Path, diff: &WorkbookDiff) -> Result<()> {
    write_html_report_with_layout(path, diff, ReportLayout::Merged)
}

/// Write an HTML report using the requested presentation layout.
pub fn write_html_report_with_layout(
    path: &Path,
    diff: &WorkbookDiff,
    layout: ReportLayout,
) -> Result<()> {
    let mut html = String::new();
    html.push_str("<!doctype html><html><head><meta charset=\"utf-8\"><title>xlsx diff</title>");
    html.push_str(
        "<style>body{font-family:system-ui,sans-serif;margin:24px}table{border-collapse:collapse;margin:12px 0 24px;width:100%}th,td{border:1px solid #ddd;padding:6px 8px;text-align:left;vertical-align:top}th{background:#f5f5f5}.added{background:#e9f8ed}.removed{background:#fdeaea}.changed{background:#fff6d6}.sheet{margin-top:28px}.side-by-side-wrap{overflow-x:auto}.side-by-side{min-width:960px;table-layout:fixed}.side-by-side .side-heading{text-align:center}.side-by-side .new-side{border-left:3px solid #aaa}.side-by-side .empty{color:#888}</style>",
    );
    html.push_str("</head><body>");
    html.push_str("<h1>xlsx diff</h1>");
    html.push_str(&format!(
        "<p><strong>Old:</strong> {}<br><strong>New:</strong> {}</p>",
        encode_text(&diff.old_file),
        encode_text(&diff.new_file)
    ));
    html.push_str("<h2>Summary</h2><table><tr><th>metric</th><th>count</th></tr>");
    for (name, value) in summary_rows(&diff.summary) {
        html.push_str(&format!(
            "<tr><td>{}</td><td>{}</td></tr>",
            encode_text(name),
            value
        ));
    }
    html.push_str("</table>");

    for sheet in &diff.sheets {
        html.push_str("<section class=\"sheet\">");
        html.push_str(&format!(
            "<h2>{}</h2><p>Status: {:?}</p>",
            encode_text(&sheet_title(sheet)),
            sheet.status
        ));
        match layout {
            ReportLayout::Merged => {
                write_html_rows(&mut html, sheet);
                write_html_columns(&mut html, sheet);
                write_html_cells(&mut html, sheet);
                write_html_table_regions(&mut html, sheet);
                write_html_merges(&mut html, sheet);
            }
            ReportLayout::SideBySide => write_html_side_by_side(&mut html, sheet),
        }
        html.push_str("</section>");
    }

    html.push_str("</body></html>");
    std::fs::write(path, html)
        .with_context(|| format!("failed to write HTML report: {}", path.display()))
}

fn write_html_side_by_side(html: &mut String, sheet: &SheetDiff) {
    let events = side_by_side_events(sheet);
    if events.is_empty() {
        return;
    }
    html.push_str("<div class=\"side-by-side-wrap\"><table class=\"side-by-side\"><thead><tr><th rowspan=\"2\">category</th><th rowspan=\"2\">kind</th><th class=\"side-heading\" colspan=\"3\">OLD</th><th class=\"side-heading new-side\" colspan=\"3\">NEW</th></tr><tr><th>location</th><th>value / detail</th><th>formula</th><th class=\"new-side\">location</th><th>value / detail</th><th>formula</th></tr></thead><tbody>");
    for event in events {
        let old_class = side_event_class(&event, ReportSide::Old);
        let new_class = side_event_class(&event, ReportSide::New);
        html.push_str(&format!(
            "<tr><td class=\"{}\">{}</td><td class=\"{}\">{}</td><td class=\"{}\">{}</td><td class=\"{}\">{}</td><td class=\"{}\">{}</td><td class=\"new-side {}\">{}</td><td class=\"{}\">{}</td><td class=\"{}\">{}</td></tr>",
            event.class,
            encode_text(event.category),
            event.class,
            encode_text(&event.kind),
            old_class,
            encode_text(&event.old_location),
            old_class,
            encode_text(&event.old_value),
            old_class,
            encode_text(&event.old_formula),
            new_class,
            encode_text(&event.new_location),
            new_class,
            encode_text(&event.new_value),
            new_class,
            encode_text(&event.new_formula),
        ));
    }
    html.push_str("</tbody></table></div>");
}

fn write_html_rows(html: &mut String, sheet: &SheetDiff) {
    if sheet.row_changes.is_empty() {
        return;
    }
    html.push_str("<h3>Rows</h3><table><tr><th>kind</th><th>old row</th><th>new row</th><th>preview</th></tr>");
    for change in &sheet.row_changes {
        let class = change_class(&change.kind);
        html.push_str(&format!(
            "<tr class=\"{}\"><td>{:?}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            class,
            change.kind,
            optional_usize(change.old_index),
            optional_usize(change.new_index),
            encode_text(&change.preview)
        ));
    }
    html.push_str("</table>");
}

fn write_html_columns(html: &mut String, sheet: &SheetDiff) {
    if sheet.column_changes.is_empty() {
        return;
    }
    html.push_str("<h3>Columns</h3><table><tr><th>kind</th><th>old column</th><th>new column</th><th>header</th></tr>");
    for change in &sheet.column_changes {
        let class = change_class(&change.kind);
        html.push_str(&format!(
            "<tr class=\"{}\"><td>{:?}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            class,
            change.kind,
            change
                .old_index
                .map(|idx| column_name(idx - 1))
                .unwrap_or_default(),
            change
                .new_index
                .map(|idx| column_name(idx - 1))
                .unwrap_or_default(),
            encode_text(&change.header)
        ));
    }
    html.push_str("</table>");
}

fn write_html_cells(html: &mut String, sheet: &SheetDiff) {
    if sheet.cell_changes.is_empty() {
        return;
    }
    html.push_str("<h3>Cells</h3><table><tr><th>kind</th><th>old</th><th>new</th><th>old value (type)</th><th>new value (type)</th><th>old formula</th><th>new formula</th></tr>");
    for change in &sheet.cell_changes {
        let old_value = display_cell_value(change.old_value.as_deref(), change.old_value_kind);
        let new_value = display_cell_value(change.new_value.as_deref(), change.new_value_kind);
        html.push_str(&format!(
            "<tr class=\"changed\"><td>{:?}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            change.kind,
            encode_text(change.old_address.as_deref().unwrap_or_default()),
            encode_text(change.new_address.as_deref().unwrap_or_default()),
            encode_text(&old_value),
            encode_text(&new_value),
            encode_text(change.old_formula.as_deref().unwrap_or_default()),
            encode_text(change.new_formula.as_deref().unwrap_or_default())
        ));
    }
    html.push_str("</table>");
}

fn write_html_table_regions(html: &mut String, sheet: &SheetDiff) {
    if sheet.table_regions.is_empty() {
        return;
    }
    html.push_str("<h3>Table Regions</h3><table><tr><th>status</th><th>old range</th><th>new range</th><th>rows</th><th>columns</th><th>cells</th></tr>");
    for region in &sheet.table_regions {
        html.push_str(&format!(
            "<tr><td>{:?}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            region.status,
            encode_text(&optional_range(&region.old_range)),
            encode_text(&optional_range(&region.new_range)),
            region.row_changes.len(),
            region.column_changes.len(),
            region.cell_changes.len(),
        ));
    }
    html.push_str("</table>");
}

fn write_html_merges(html: &mut String, sheet: &SheetDiff) {
    if sheet.merge_changes.is_empty() {
        return;
    }
    html.push_str(
        "<h3>Merged Cells</h3><table><tr><th>kind</th><th>old range</th><th>new range</th></tr>",
    );
    for change in &sheet.merge_changes {
        html.push_str(&format!(
            "<tr class=\"{}\"><td>{:?}</td><td>{}</td><td>{}</td></tr>",
            change_class(&change.kind),
            change.kind,
            encode_text(&optional_range(&change.old_range)),
            encode_text(&optional_range(&change.new_range)),
        ));
    }
    html.push_str("</table>");
}

struct SideBySideEvent {
    category: &'static str,
    kind: String,
    class: &'static str,
    old_location: String,
    old_value: String,
    old_formula: String,
    new_location: String,
    new_value: String,
    new_formula: String,
}

enum ReportSide {
    Old,
    New,
}

fn side_event_class(event: &SideBySideEvent, side: ReportSide) -> &'static str {
    let has_content = match side {
        ReportSide::Old => {
            !event.old_location.is_empty()
                || !event.old_value.is_empty()
                || !event.old_formula.is_empty()
        }
        ReportSide::New => {
            !event.new_location.is_empty()
                || !event.new_value.is_empty()
                || !event.new_formula.is_empty()
        }
    };
    if has_content { event.class } else { "empty" }
}

fn side_by_side_events(sheet: &SheetDiff) -> Vec<SideBySideEvent> {
    let mut events = Vec::new();
    if !matches!(sheet.status, SheetStatus::Modified | SheetStatus::Unchanged) {
        events.push(SideBySideEvent {
            category: "sheet",
            kind: sheet_status_label(&sheet.status).to_string(),
            class: sheet_status_class(&sheet.status),
            old_location: sheet.old_name.clone().unwrap_or_default(),
            old_value: optional_position(sheet.old_position),
            old_formula: String::new(),
            new_location: sheet.new_name.clone().unwrap_or_default(),
            new_value: optional_position(sheet.new_position),
            new_formula: String::new(),
        });
    }
    for change in &sheet.row_changes {
        events.push(SideBySideEvent {
            category: "row",
            kind: change_kind_label(&change.kind).to_string(),
            class: change_class(&change.kind),
            old_location: optional_usize(change.old_index),
            old_value: detail_when_present(change.old_index, &change.preview),
            old_formula: String::new(),
            new_location: optional_usize(change.new_index),
            new_value: detail_when_present(change.new_index, &change.preview),
            new_formula: String::new(),
        });
    }
    for change in &sheet.column_changes {
        events.push(SideBySideEvent {
            category: "column",
            kind: change_kind_label(&change.kind).to_string(),
            class: change_class(&change.kind),
            old_location: change
                .old_index
                .map(|idx| column_name(idx - 1))
                .unwrap_or_default(),
            old_value: detail_when_present(change.old_index, &change.header),
            old_formula: String::new(),
            new_location: change
                .new_index
                .map(|idx| column_name(idx - 1))
                .unwrap_or_default(),
            new_value: detail_when_present(change.new_index, &change.header),
            new_formula: String::new(),
        });
    }
    for change in &sheet.cell_changes {
        events.push(SideBySideEvent {
            category: "cell",
            kind: cell_change_kind_label(&change.kind).to_string(),
            class: "changed",
            old_location: change.old_address.clone().unwrap_or_default(),
            old_value: display_cell_value(change.old_value.as_deref(), change.old_value_kind),
            old_formula: change.old_formula.clone().unwrap_or_default(),
            new_location: change.new_address.clone().unwrap_or_default(),
            new_value: display_cell_value(change.new_value.as_deref(), change.new_value_kind),
            new_formula: change.new_formula.clone().unwrap_or_default(),
        });
    }
    for region in &sheet.table_regions {
        let detail = format!(
            "rows: {}, columns: {}, cells: {}",
            region.row_changes.len(),
            region.column_changes.len(),
            region.cell_changes.len()
        );
        events.push(SideBySideEvent {
            category: "table_region",
            kind: region_status_label(&region.status).to_string(),
            class: region_status_class(&region.status),
            old_location: optional_range(&region.old_range),
            old_value: detail_when_present(region.old_range.as_ref(), &detail),
            old_formula: String::new(),
            new_location: optional_range(&region.new_range),
            new_value: detail_when_present(region.new_range.as_ref(), &detail),
            new_formula: String::new(),
        });
    }
    for change in &sheet.merge_changes {
        events.push(SideBySideEvent {
            category: "merged_cell",
            kind: change_kind_label(&change.kind).to_string(),
            class: change_class(&change.kind),
            old_location: optional_range(&change.old_range),
            old_value: String::new(),
            old_formula: String::new(),
            new_location: optional_range(&change.new_range),
            new_value: String::new(),
            new_formula: String::new(),
        });
    }
    events
}

fn detail_when_present<T>(value: Option<T>, detail: &str) -> String {
    value.map(|_| detail.to_string()).unwrap_or_default()
}

fn optional_position(value: Option<usize>) -> String {
    value
        .map(|position| format!("position {position}"))
        .unwrap_or_default()
}

fn sheet_status_label(status: &SheetStatus) -> &'static str {
    match status {
        SheetStatus::Added => "added",
        SheetStatus::Removed => "removed",
        SheetStatus::Modified => "modified",
        SheetStatus::Renamed => "renamed",
        SheetStatus::Reordered => "reordered",
        SheetStatus::Unchanged => "unchanged",
    }
}

fn sheet_status_class(status: &SheetStatus) -> &'static str {
    match status {
        SheetStatus::Added => "added",
        SheetStatus::Removed => "removed",
        SheetStatus::Modified | SheetStatus::Renamed | SheetStatus::Reordered => "changed",
        SheetStatus::Unchanged => "",
    }
}

fn region_status_class(status: &RegionStatus) -> &'static str {
    match status {
        RegionStatus::Added => "added",
        RegionStatus::Removed => "removed",
        RegionStatus::Modified => "changed",
        RegionStatus::Unchanged => "",
    }
}

/// Write an XLSX report using the default merged layout.
pub fn write_xlsx_report(path: &Path, diff: &WorkbookDiff) -> Result<()> {
    write_xlsx_report_with_layout(path, diff, ReportLayout::Merged)
}

/// Write an XLSX report using the requested presentation layout.
pub fn write_xlsx_report_with_layout(
    path: &Path,
    diff: &WorkbookDiff,
    layout: ReportLayout,
) -> Result<()> {
    let mut workbook = Workbook::new();
    let header_format = Format::new()
        .set_bold()
        .set_background_color(Color::RGB(0xD9EAF7));
    let added_format = Format::new().set_background_color(Color::RGB(0xE2F0D9));
    let removed_format = Format::new().set_background_color(Color::RGB(0xF4CCCC));
    let changed_format = Format::new().set_background_color(Color::RGB(0xFFF2CC));

    {
        let worksheet = workbook.add_worksheet();
        worksheet.set_name("Summary")?;
        write_row(worksheet, 0, &["metric", "count"], Some(&header_format))?;
        for (idx, (name, value)) in summary_rows(&diff.summary).into_iter().enumerate() {
            worksheet.write_string((idx + 1) as u32, 0, name)?;
            worksheet.write_number((idx + 1) as u32, 1, value as f64)?;
        }
    }

    {
        let worksheet = workbook.add_worksheet();
        worksheet.set_name("Changes")?;
        write_row(
            worksheet,
            0,
            &[
                "sheet",
                "category",
                "kind",
                "old",
                "new",
                "old value (type)",
                "new value (type)",
                "old formula",
                "new formula",
            ],
            Some(&header_format),
        )?;
        let mut row = 1u32;
        for sheet in &diff.sheets {
            let sheet_name = sheet_title(sheet);
            if matches!(
                sheet.status,
                SheetStatus::Added
                    | SheetStatus::Removed
                    | SheetStatus::Renamed
                    | SheetStatus::Reordered
            ) {
                let format = if matches!(sheet.status, SheetStatus::Added) {
                    &added_format
                } else if matches!(sheet.status, SheetStatus::Removed) {
                    &removed_format
                } else {
                    &changed_format
                };
                write_change_row(
                    worksheet,
                    row,
                    format,
                    &[
                        &sheet_name,
                        "sheet",
                        &format!("{:?}", sheet.status),
                        "",
                        "",
                        "",
                        "",
                        "",
                        "",
                    ],
                )?;
                row += 1;
            }
            for region in &sheet.table_regions {
                write_change_row(
                    worksheet,
                    row,
                    &changed_format,
                    &[
                        &sheet_name,
                        "table_region",
                        &format!("{:?}", region.status),
                        &optional_range(&region.old_range),
                        &optional_range(&region.new_range),
                        &region.row_changes.len().to_string(),
                        &region.column_changes.len().to_string(),
                        &region.cell_changes.len().to_string(),
                        "",
                    ],
                )?;
                row += 1;
            }
            for change in &sheet.row_changes {
                let format = change_format(
                    &change.kind,
                    &added_format,
                    &removed_format,
                    &changed_format,
                );
                write_change_row(
                    worksheet,
                    row,
                    format,
                    &[
                        &sheet_name,
                        "row",
                        &format!("{:?}", change.kind),
                        &optional_usize(change.old_index),
                        &optional_usize(change.new_index),
                        &change.preview,
                        "",
                        "",
                        "",
                    ],
                )?;
                row += 1;
            }
            for change in &sheet.column_changes {
                let format = change_format(
                    &change.kind,
                    &added_format,
                    &removed_format,
                    &changed_format,
                );
                write_change_row(
                    worksheet,
                    row,
                    format,
                    &[
                        &sheet_name,
                        "column",
                        &format!("{:?}", change.kind),
                        &change
                            .old_index
                            .map(|idx| column_name(idx - 1))
                            .unwrap_or_default(),
                        &change
                            .new_index
                            .map(|idx| column_name(idx - 1))
                            .unwrap_or_default(),
                        &change.header,
                        "",
                        "",
                        "",
                    ],
                )?;
                row += 1;
            }
            for change in &sheet.cell_changes {
                let old_value =
                    display_cell_value(change.old_value.as_deref(), change.old_value_kind);
                let new_value =
                    display_cell_value(change.new_value.as_deref(), change.new_value_kind);
                write_change_row(
                    worksheet,
                    row,
                    &changed_format,
                    &[
                        &sheet_name,
                        "cell",
                        &format!("{:?}", change.kind),
                        change.old_address.as_deref().unwrap_or_default(),
                        change.new_address.as_deref().unwrap_or_default(),
                        &old_value,
                        &new_value,
                        change.old_formula.as_deref().unwrap_or_default(),
                        change.new_formula.as_deref().unwrap_or_default(),
                    ],
                )?;
                row += 1;
            }
            for change in &sheet.merge_changes {
                let format = if matches!(change.kind, ChangeKind::Added) {
                    &added_format
                } else {
                    &removed_format
                };
                write_change_row(
                    worksheet,
                    row,
                    format,
                    &[
                        &sheet_name,
                        "merged_cell",
                        &format!("{:?}", change.kind),
                        &optional_range(&change.old_range),
                        &optional_range(&change.new_range),
                        "",
                        "",
                        "",
                        "",
                    ],
                )?;
                row += 1;
            }
        }
    }

    for (idx, sheet) in diff.sheets.iter().enumerate() {
        match layout {
            ReportLayout::Merged => write_xlsx_grid_sheet(
                &mut workbook,
                sheet,
                idx,
                &header_format,
                &added_format,
                &removed_format,
                &changed_format,
            )?,
            ReportLayout::SideBySide => write_xlsx_side_by_side_sheet(
                &mut workbook,
                sheet,
                idx,
                &header_format,
                &added_format,
                &removed_format,
                &changed_format,
            )?,
        }
    }

    workbook
        .save(path)
        .with_context(|| format!("failed to write xlsx report: {}", path.display()))?;
    Ok(())
}

fn write_xlsx_side_by_side_sheet(
    workbook: &mut Workbook,
    sheet: &SheetDiff,
    idx: usize,
    header_format: &Format,
    added_format: &Format,
    removed_format: &Format,
    changed_format: &Format,
) -> std::result::Result<(), XlsxError> {
    let worksheet = workbook.add_worksheet();
    worksheet.set_name(side_by_side_sheet_name(sheet, idx))?;
    worksheet.merge_range(0, 0, 0, 1, "CHANGE", header_format)?;
    worksheet.merge_range(0, 2, 0, 4, "OLD", header_format)?;
    worksheet.merge_range(0, 5, 0, 7, "NEW", header_format)?;
    for (column, width) in [
        (0, 18.0),
        (1, 28.0),
        (2, 14.0),
        (3, 36.0),
        (4, 28.0),
        (5, 14.0),
        (6, 36.0),
        (7, 28.0),
    ] {
        worksheet.set_column_width(column, width)?;
    }
    write_row(
        worksheet,
        1,
        &[
            "category",
            "kind",
            "location",
            "value / detail",
            "formula",
            "location",
            "value / detail",
            "formula",
        ],
        Some(header_format),
    )?;

    for (idx, event) in side_by_side_events(sheet).into_iter().enumerate() {
        write_side_by_side_change_row(
            worksheet,
            idx as u32 + 2,
            &event,
            header_format,
            added_format,
            removed_format,
            changed_format,
        )?;
    }

    Ok(())
}

fn write_xlsx_grid_sheet(
    workbook: &mut Workbook,
    sheet: &SheetDiff,
    idx: usize,
    header_format: &Format,
    added_format: &Format,
    removed_format: &Format,
    changed_format: &Format,
) -> std::result::Result<(), XlsxError> {
    let worksheet = workbook.add_worksheet();
    worksheet.set_name(grid_sheet_name(sheet, idx))?;
    write_row(
        worksheet,
        0,
        &[
            "category",
            "kind",
            "old",
            "new",
            "old value (type)",
            "new value (type)",
            "old formula",
            "new formula",
        ],
        Some(header_format),
    )?;

    let mut row = 1u32;
    for change in &sheet.row_changes {
        write_change_row(
            worksheet,
            row,
            change_format(&change.kind, added_format, removed_format, changed_format),
            &[
                "row",
                change_kind_label(&change.kind),
                &optional_usize(change.old_index),
                &optional_usize(change.new_index),
                &change.preview,
                "",
                "",
                "",
            ],
        )?;
        row += 1;
    }
    for change in &sheet.column_changes {
        write_change_row(
            worksheet,
            row,
            change_format(&change.kind, added_format, removed_format, changed_format),
            &[
                "column",
                change_kind_label(&change.kind),
                &change
                    .old_index
                    .map(|idx| column_name(idx - 1))
                    .unwrap_or_default(),
                &change
                    .new_index
                    .map(|idx| column_name(idx - 1))
                    .unwrap_or_default(),
                &change.header,
                "",
                "",
                "",
            ],
        )?;
        row += 1;
    }
    for change in &sheet.cell_changes {
        let old_value = display_cell_value(change.old_value.as_deref(), change.old_value_kind);
        let new_value = display_cell_value(change.new_value.as_deref(), change.new_value_kind);
        write_change_row(
            worksheet,
            row,
            changed_format,
            &[
                "cell",
                cell_change_kind_label(&change.kind),
                change.old_address.as_deref().unwrap_or_default(),
                change.new_address.as_deref().unwrap_or_default(),
                &old_value,
                &new_value,
                change.old_formula.as_deref().unwrap_or_default(),
                change.new_formula.as_deref().unwrap_or_default(),
            ],
        )?;
        row += 1;
    }
    for region in &sheet.table_regions {
        write_change_row(
            worksheet,
            row,
            match region.status {
                RegionStatus::Added => added_format,
                RegionStatus::Removed => removed_format,
                RegionStatus::Modified => changed_format,
                RegionStatus::Unchanged => header_format,
            },
            &[
                "table_region",
                region_status_label(&region.status),
                &optional_range(&region.old_range),
                &optional_range(&region.new_range),
                &region.row_changes.len().to_string(),
                &region.column_changes.len().to_string(),
                "",
                "",
            ],
        )?;
        row += 1;
    }
    for change in &sheet.merge_changes {
        write_change_row(
            worksheet,
            row,
            change_format(&change.kind, added_format, removed_format, changed_format),
            &[
                "merged_cell",
                change_kind_label(&change.kind),
                &optional_range(&change.old_range),
                &optional_range(&change.new_range),
                "",
                "",
                "",
                "",
            ],
        )?;
        row += 1;
    }

    Ok(())
}

fn write_row(
    worksheet: &mut rust_xlsxwriter::Worksheet,
    row: u32,
    values: &[&str],
    format: Option<&Format>,
) -> std::result::Result<(), XlsxError> {
    for (col, value) in values.iter().enumerate() {
        if let Some(format) = format {
            worksheet.write_string_with_format(row, col as u16, *value, format)?;
        } else {
            worksheet.write_string(row, col as u16, *value)?;
        }
    }
    Ok(())
}

fn write_change_row(
    worksheet: &mut rust_xlsxwriter::Worksheet,
    row: u32,
    format: &Format,
    values: &[&str],
) -> std::result::Result<(), XlsxError> {
    for (col, value) in values.iter().enumerate() {
        worksheet.write_string_with_format(row, col as u16, *value, format)?;
    }
    Ok(())
}

fn write_side_by_side_change_row(
    worksheet: &mut rust_xlsxwriter::Worksheet,
    row: u32,
    event: &SideBySideEvent,
    header_format: &Format,
    added_format: &Format,
    removed_format: &Format,
    changed_format: &Format,
) -> std::result::Result<(), XlsxError> {
    let event_format = report_event_format(
        event.class,
        header_format,
        added_format,
        removed_format,
        changed_format,
    );
    worksheet.write_string_with_format(row, 0, event.category, event_format)?;
    worksheet.write_string_with_format(row, 1, &event.kind, event_format)?;

    for (offset, value) in [&event.old_location, &event.old_value, &event.old_formula]
        .into_iter()
        .enumerate()
    {
        write_side_value(
            worksheet,
            row,
            offset as u16 + 2,
            value,
            side_event_class(event, ReportSide::Old),
            header_format,
            added_format,
            removed_format,
            changed_format,
        )?;
    }
    for (offset, value) in [&event.new_location, &event.new_value, &event.new_formula]
        .into_iter()
        .enumerate()
    {
        write_side_value(
            worksheet,
            row,
            offset as u16 + 5,
            value,
            side_event_class(event, ReportSide::New),
            header_format,
            added_format,
            removed_format,
            changed_format,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_side_value(
    worksheet: &mut rust_xlsxwriter::Worksheet,
    row: u32,
    column: u16,
    value: &str,
    class: &str,
    header_format: &Format,
    added_format: &Format,
    removed_format: &Format,
    changed_format: &Format,
) -> std::result::Result<(), XlsxError> {
    if class == "empty" {
        worksheet.write_string(row, column, value)?;
    } else {
        worksheet.write_string_with_format(
            row,
            column,
            value,
            report_event_format(
                class,
                header_format,
                added_format,
                removed_format,
                changed_format,
            ),
        )?;
    }
    Ok(())
}

fn report_event_format<'a>(
    class: &str,
    header_format: &'a Format,
    added_format: &'a Format,
    removed_format: &'a Format,
    changed_format: &'a Format,
) -> &'a Format {
    match class {
        "added" => added_format,
        "removed" => removed_format,
        "changed" => changed_format,
        _ => header_format,
    }
}

fn summary_rows(summary: &DiffSummary) -> Vec<(&'static str, usize)> {
    vec![
        ("sheets_added", summary.sheets_added),
        ("sheets_removed", summary.sheets_removed),
        ("sheets_renamed", summary.sheets_renamed),
        ("sheet_order_changes", summary.sheet_order_changes),
        ("table_regions_added", summary.table_regions_added),
        ("table_regions_removed", summary.table_regions_removed),
        ("rows_added", summary.rows_added),
        ("rows_removed", summary.rows_removed),
        ("rows_moved", summary.rows_moved),
        ("columns_added", summary.columns_added),
        ("columns_removed", summary.columns_removed),
        ("columns_moved", summary.columns_moved),
        ("cells_changed", summary.cells_changed),
        ("formulas_changed", summary.formulas_changed),
        ("cell_styles_changed", summary.cell_styles_changed),
        ("merged_cells_added", summary.merged_cells_added),
        ("merged_cells_removed", summary.merged_cells_removed),
    ]
}

fn sheet_title(sheet: &SheetDiff) -> String {
    match (&sheet.old_name, &sheet.new_name) {
        (Some(old), Some(new)) if old != new => format!("{old} -> {new}"),
        (Some(name), _) | (_, Some(name)) => name.clone(),
        _ => "(unknown)".to_string(),
    }
}

fn grid_sheet_name(sheet: &SheetDiff, idx: usize) -> String {
    report_sheet_name("Grid", sheet, idx)
}

fn side_by_side_sheet_name(sheet: &SheetDiff, idx: usize) -> String {
    report_sheet_name("Side", sheet, idx)
}

fn report_sheet_name(prefix: &str, sheet: &SheetDiff, idx: usize) -> String {
    let base = format!("{prefix} - {}", sheet_title(sheet));
    let sanitized = sanitize_xlsx_sheet_name(&base);
    if sanitized.chars().count() <= 31 {
        sanitized
    } else {
        let suffix = format!(" {}", idx + 1);
        let prefix_len = 31usize.saturating_sub(suffix.chars().count());
        format!(
            "{}{}",
            sanitized.chars().take(prefix_len).collect::<String>(),
            suffix
        )
    }
}

fn sanitize_xlsx_sheet_name(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|ch| match ch {
            ':' | '\\' | '/' | '?' | '*' | '[' | ']' => '_',
            _ => ch,
        })
        .collect::<String>();
    if sanitized.trim().is_empty() {
        "Grid".to_string()
    } else {
        sanitized
    }
}

fn optional_usize(value: Option<usize>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn optional_range(range: &Option<CellRange>) -> String {
    range.as_ref().map(range_address).unwrap_or_default()
}

fn range_address(range: &CellRange) -> String {
    if range.start_row == range.end_row && range.start_col == range.end_col {
        one_based_address(range.start_row, range.start_col)
    } else {
        format!(
            "{}:{}",
            one_based_address(range.start_row, range.start_col),
            one_based_address(range.end_row, range.end_col)
        )
    }
}

fn one_based_address(row: usize, column: usize) -> String {
    if row == 0 || column == 0 {
        return "#INVALID".to_string();
    }
    address(row - 1, column - 1)
}

fn change_class(kind: &ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "added",
        ChangeKind::Removed => "removed",
        ChangeKind::Moved => "changed",
    }
}

fn change_kind_label(kind: &ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "added",
        ChangeKind::Removed => "removed",
        ChangeKind::Moved => "moved",
    }
}

fn cell_change_kind_label(kind: &CellChangeKind) -> &'static str {
    match kind {
        CellChangeKind::ValueChanged => "value_changed",
        CellChangeKind::FormulaChanged => "formula_changed",
        CellChangeKind::ValueAndFormulaChanged => "value_and_formula_changed",
        CellChangeKind::StyleChanged => "style_changed",
        CellChangeKind::ValueAdded => "value_added",
        CellChangeKind::ValueRemoved => "value_removed",
    }
}

fn display_cell_value(value: Option<&str>, kind: Option<CellValueKind>) -> String {
    let value = value.unwrap_or_default();
    let Some(kind) = kind else {
        return value.to_string();
    };
    let kind = match kind {
        CellValueKind::Empty => "empty",
        CellValueKind::String => "string",
        CellValueKind::Number => "number",
        CellValueKind::Boolean => "boolean",
        CellValueKind::Error => "error",
        CellValueKind::DateTime => "date_time",
        CellValueKind::Duration => "duration",
    };
    if value.is_empty() {
        format!("[{kind}]")
    } else {
        format!("{value} [{kind}]")
    }
}

fn region_status_label(status: &RegionStatus) -> &'static str {
    match status {
        RegionStatus::Added => "added",
        RegionStatus::Removed => "removed",
        RegionStatus::Modified => "modified",
        RegionStatus::Unchanged => "unchanged",
    }
}

fn change_format<'a>(
    kind: &ChangeKind,
    added_format: &'a Format,
    removed_format: &'a Format,
    changed_format: &'a Format,
) -> &'a Format {
    match kind {
        ChangeKind::Added => added_format,
        ChangeKind::Removed => removed_format,
        ChangeKind::Moved => changed_format,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use rust_xlsxwriter::ExcelDateTime;

    const INDEXED_COLOR_CASES: &[(usize, &str)] = &[
        (0, "000000"),
        (1, "FFFFFF"),
        (2, "FF0000"),
        (3, "00FF00"),
        (4, "0000FF"),
        (5, "FFFF00"),
        (6, "FF00FF"),
        (7, "00FFFF"),
        (16, "800000"),
        (22, "C0C0C0"),
        (24, "9999FF"),
        (31, "CCCCFF"),
        (40, "00CCFF"),
        (47, "FFCC99"),
        (55, "969696"),
        (63, "333333"),
    ];

    fn diff_sheets(old: &SheetIr, new: &SheetIr) -> SheetDiff {
        super::diff_sheets(old, new, &DiffOptions::default(), None).unwrap()
    }

    fn diff_workbooks(
        old_path: &Path,
        new_path: &Path,
        old_book: &WorkbookIr,
        new_book: &WorkbookIr,
    ) -> WorkbookDiff {
        super::diff_workbooks(old_path, new_path, old_book, new_book).unwrap()
    }

    #[test]
    fn detects_inserted_row_column_and_shifted_cell_change() {
        let old = sheet(
            "料金表",
            &[
                &["商品コード", "商品名", "単価", "数量"],
                &["A001", "りんご", "100", "2"],
                &["A002", "みかん", "120", "3"],
            ],
        );
        let new = sheet(
            "料金表",
            &[
                &["商品コード", "商品名", "税区分", "単価", "数量"],
                &["A001", "りんご", "軽減", "100", "2"],
                &["A999", "バナナ", "標準", "150", "1"],
                &["A002", "みかん", "軽減", "130", "3"],
            ],
        );

        let diff = diff_sheets(&old, &new);

        assert_eq!(diff.row_changes.len(), 1);
        assert!(matches!(diff.row_changes[0].kind, ChangeKind::Added));
        assert_eq!(diff.row_changes[0].new_index, Some(3));
        assert_eq!(diff.column_changes.len(), 1);
        assert!(matches!(diff.column_changes[0].kind, ChangeKind::Added));
        assert_eq!(diff.column_changes[0].new_index, Some(3));
        assert_eq!(diff.cell_changes.len(), 1);
        assert_eq!(diff.cell_changes[0].old_address.as_deref(), Some("C3"));
        assert_eq!(diff.cell_changes[0].new_address.as_deref(), Some("D4"));
    }

    #[test]
    fn unchanged_blank_separator_rows_do_not_create_structure_changes() {
        let old = sheet(
            "複数表",
            &[
                &["商品コード", "商品名", "単価"],
                &["A001", "りんご", "100"],
                &["", "", ""],
                &["担当者コード", "担当者名", ""],
                &["U001", "田中", ""],
            ],
        );
        let new = old.clone();

        let diff = diff_sheets(&old, &new);

        assert!(diff.row_changes.is_empty());
        assert!(diff.column_changes.is_empty());
        assert!(diff.cell_changes.is_empty());
        assert!(matches!(diff.status, SheetStatus::Unchanged));
    }

    #[test]
    fn unchanged_blank_separator_columns_do_not_create_structure_changes() {
        let old = sheet(
            "横並び表",
            &[
                &["商品コード", "商品名", "", "担当者コード", "担当者名"],
                &["A001", "りんご", "", "U001", "田中"],
            ],
        );
        let new = old.clone();

        let diff = diff_sheets(&old, &new);

        assert!(diff.row_changes.is_empty());
        assert!(diff.column_changes.is_empty());
        assert!(diff.cell_changes.is_empty());
        assert!(matches!(diff.status, SheetStatus::Unchanged));
    }

    #[test]
    fn detects_formula_only_change() {
        let mut old = sheet("数式", &[&["商品コード", "金額"], &["A001", "200"]]);
        let mut new = old.clone();
        old.rows[1][1].formula = Some("=C2*D2".to_string());
        new.rows[1][1].formula = Some("=ROUND(C2*D2,0)".to_string());

        let diff = diff_sheets(&old, &new);

        assert_eq!(diff.cell_changes.len(), 1);
        assert!(matches!(
            diff.cell_changes[0].kind,
            CellChangeKind::FormulaChanged
        ));
        assert_eq!(diff.cell_changes[0].old_address.as_deref(), Some("B2"));
        assert_eq!(diff.cell_changes[0].new_address.as_deref(), Some("B2"));
        assert_eq!(
            diff.cell_changes[0].old_value,
            diff.cell_changes[0].new_value
        );
        assert_eq!(diff.cell_changes[0].old_formula.as_deref(), Some("=C2*D2"));
        assert_eq!(
            diff.cell_changes[0].new_formula.as_deref(),
            Some("=ROUND(C2*D2,0)")
        );
    }

    #[test]
    fn detects_formula_added_or_removed_when_cached_value_is_unchanged() {
        let plain = sheet("数式化", &[&["合計"], &["200"]]);
        let mut formula = plain.clone();
        formula.rows[1][0].formula = Some("=SUM(B2:C2)".to_string());

        for (old, new) in [(&plain, &formula), (&formula, &plain)] {
            let diff = diff_sheets(old, new);
            assert_eq!(diff.cell_changes.len(), 1);
            assert!(matches!(
                diff.cell_changes[0].kind,
                CellChangeKind::FormulaChanged
            ));
            assert_eq!(
                diff.cell_changes[0].old_value,
                diff.cell_changes[0].new_value
            );
        }
    }

    #[test]
    fn ignores_formula_ascii_case_only_changes() {
        let mut old = sheet("数式大小", &[&["コード", "合計"], &["A001", "300"]]);
        let mut new = old.clone();
        old.rows[1][1].formula = Some("=sum(a2:b2)".to_string());
        new.rows[1][1].formula = Some("=SUM(A2:B2)".to_string());

        let diff = diff_sheets(&old, &new);

        assert!(diff.cell_changes.is_empty());

        let mut formula_only_old = sheet("formula-only", &[&[""]]);
        formula_only_old.rows[0][0].formula = Some("=sum(a2:b2)".to_string());
        let mut formula_only_new = formula_only_old.clone();
        formula_only_new.rows[0][0].formula = Some("=SUM(A2:B2)".to_string());

        let formula_only_diff = diff_sheets(&formula_only_old, &formula_only_new);
        assert!(formula_only_diff.row_changes.is_empty());
        assert!(formula_only_diff.column_changes.is_empty());
        assert!(formula_only_diff.cell_changes.is_empty());
    }

    #[test]
    fn canonical_unicode_forms_load_as_the_same_cell_value() {
        let composed = normalize_value(&Data::String("Caf\u{e9}".to_string()));
        let decomposed = normalize_value(&Data::String("Cafe\u{301}".to_string()));

        assert_eq!(composed, decomposed);
        assert_ne!(
            normalize_value_with_policy(
                &Data::String("Caf\u{e9}".to_string()),
                WhitespaceNormalization::Preserve,
            ),
            normalize_value_with_policy(
                &Data::String("Cafe\u{301}".to_string()),
                WhitespaceNormalization::Preserve,
            ),
        );
    }

    #[test]
    fn default_text_normalization_ignores_discretionary_soft_hyphens() {
        let plain = Data::String("international".to_string());
        let discretionary = Data::String("inter\u{ad}national".to_string());

        assert_eq!(normalize_value(&plain), normalize_value(&discretionary));
        assert_ne!(
            normalize_value_with_policy(&plain, WhitespaceNormalization::Preserve),
            normalize_value_with_policy(&discretionary, WhitespaceNormalization::Preserve),
        );
    }

    #[test]
    fn cli_out_dir_generates_all_report_paths() {
        let cli = Cli {
            old: PathBuf::from("old.xlsx"),
            new: PathBuf::from("new.xlsx"),
            json: None,
            html: None,
            xlsx: None,
            out_dir: Some(PathBuf::from("reports")),
            basename: "料金差分".to_string(),
            report_layout: ReportLayout::Merged,
            check: false,
            preserve_whitespace: false,
            compare_cached_formula_values: false,
            max_input_bytes: ResourceLimits::default().max_input_bytes,
            max_uncompressed_bytes: ResourceLimits::default().max_uncompressed_bytes,
            max_zip_entries: ResourceLimits::default().max_zip_entries,
            max_rows_per_sheet: ResourceLimits::default().max_rows_per_sheet,
            max_columns_per_sheet: ResourceLimits::default().max_columns_per_sheet,
            max_cells: ResourceLimits::default().max_cells,
            max_alignment_pairs: ResourceLimits::default().max_alignment_pairs,
            max_assignment_size: ResourceLimits::default().max_assignment_size,
            min_match_score: DiffOptions::default().min_match_score,
            move_match_score: DiffOptions::default().move_match_score,
            sheet_rename_score: DiffOptions::default().sheet_rename_score,
        };

        let outputs = cli_output_paths(&cli);

        assert_eq!(outputs.json, Some(PathBuf::from("reports/料金差分.json")));
        assert_eq!(outputs.html, Some(PathBuf::from("reports/料金差分.html")));
        assert_eq!(outputs.xlsx, Some(PathBuf::from("reports/料金差分.xlsx")));
    }

    #[test]
    fn cli_accepts_p2_accuracy_controls() {
        let cli = Cli::try_parse_from([
            "xlsx-diff",
            "old.xlsx",
            "new.xlsx",
            "--compare-cached-formula-values",
            "--max-assignment-size",
            "32",
        ])
        .unwrap();

        assert!(cli.compare_cached_formula_values);
        assert_eq!(cli.max_assignment_size, 32);
    }

    #[test]
    fn cli_accepts_side_by_side_report_layout() {
        let cli = Cli::try_parse_from([
            "xlsx-diff",
            "old.xlsx",
            "new.xlsx",
            "--report-layout",
            "side-by-side",
        ])
        .unwrap();

        assert_eq!(cli.report_layout, ReportLayout::SideBySide);
    }

    #[test]
    fn cli_output_directories_are_created_before_writing_reports() {
        let root = std::env::temp_dir().join(format!(
            "xlsx-diff-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let outputs = CliOutputPaths {
            json: Some(root.join("reports/nested/diff.json")),
            html: Some(root.join("reports/nested/diff.html")),
            xlsx: Some(root.join("reports/nested/diff.xlsx")),
        };

        ensure_output_directories(&outputs).unwrap();

        assert!(root.join("reports/nested").is_dir());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn formula_only_rows_and_columns_are_aligned_by_formula_text() {
        let mut old = sheet("数式のみ", &[&["", ""], &["", ""]]);
        old.rows[0][0].formula = Some("=SUM(A2:A10)".to_string());
        old.rows[0][1].formula = Some("=COUNT(A2:A10)".to_string());
        old.rows[1][0].formula = Some("=A1*2".to_string());
        old.rows[1][1].formula = Some("=B1*2".to_string());
        let mut new = old.clone();
        new.rows[1][1].formula = Some("=B1*3".to_string());

        let diff = diff_sheets(&old, &new);

        assert!(diff.row_changes.is_empty());
        assert!(diff.column_changes.is_empty());
        assert_eq!(diff.cell_changes.len(), 1);
        assert!(matches!(
            diff.cell_changes[0].kind,
            CellChangeKind::FormulaChanged
        ));
        assert_eq!(diff.cell_changes[0].old_address.as_deref(), Some("B2"));
        assert_eq!(diff.cell_changes[0].new_address.as_deref(), Some("B2"));
    }

    #[test]
    fn rows_with_same_first_key_stay_aligned_when_multiple_values_change() {
        let old = sheet(
            "キー行",
            &[
                &["コード", "商品名", "単価", "数量"],
                &["A001", "りんご", "100", "2"],
                &["A002", "みかん", "120", "3"],
            ],
        );
        let new = sheet(
            "キー行",
            &[
                &["コード", "商品名", "単価", "数量"],
                &["A001", "りんご", "100", "2"],
                &["A002", "オレンジ", "130", "4"],
            ],
        );

        let diff = diff_sheets(&old, &new);

        assert!(diff.row_changes.is_empty());
        assert_eq!(diff.cell_changes.len(), 3);
        assert!(
            diff.cell_changes
                .iter()
                .any(|change| change.old_address.as_deref() == Some("B3"))
        );
    }

    #[test]
    fn ignores_formula_reference_row_shift_caused_by_inserted_row() {
        let mut old = sheet(
            "数式シフト",
            &[
                &["コード", "単価", "数量", "金額"],
                &["A001", "100", "2", "200"],
                &["A002", "120", "3", "360"],
            ],
        );
        old.rows[1][3].formula = Some("=B2*C2".to_string());
        old.rows[2][3].formula = Some("=B3*C3".to_string());
        let mut new = sheet(
            "数式シフト",
            &[
                &["コード", "単価", "数量", "金額"],
                &["A001", "100", "2", "200"],
                &["A999", "150", "1", "150"],
                &["A002", "120", "3", "360"],
            ],
        );
        new.rows[1][3].formula = Some("=B2*C2".to_string());
        new.rows[2][3].formula = Some("=B3*C3".to_string());
        new.rows[3][3].formula = Some("=B4*C4".to_string());

        let diff = diff_sheets(&old, &new);

        assert_eq!(diff.row_changes.len(), 1);
        assert!(matches!(diff.row_changes[0].kind, ChangeKind::Added));
        assert!(diff.cell_changes.is_empty());
    }

    #[test]
    fn ignores_mixed_shifted_and_unshifted_formula_references() {
        let mut old = sheet(
            "混在参照",
            &[
                &["税率", "0.1", "", "金額"],
                &["A001", "100", "2", "220"],
                &["A002", "120", "3", "396"],
            ],
        );
        old.rows[1][3].formula = Some("=C2*B2*(1+$B$1)".to_string());
        old.rows[2][3].formula = Some("=C3*B3*(1+$B$1)".to_string());
        let mut new = sheet(
            "混在参照",
            &[
                &["税率", "0.1", "", "金額"],
                &["A001", "100", "2", "220"],
                &["A999", "150", "1", "165"],
                &["A002", "120", "3", "396"],
            ],
        );
        new.rows[1][3].formula = Some("=C2*B2*(1+$B$1)".to_string());
        new.rows[2][3].formula = Some("=C3*B3*(1+$B$1)".to_string());
        new.rows[3][3].formula = Some("=C4*B4*(1+$B$1)".to_string());

        let diff = diff_sheets(&old, &new);

        assert_eq!(diff.row_changes.len(), 1);
        assert!(matches!(diff.row_changes[0].kind, ChangeKind::Added));
        assert!(diff.cell_changes.is_empty());
    }

    #[test]
    fn detects_formula_change_when_absolute_reference_changes_after_inserted_row() {
        let mut old = sheet(
            "絶対参照",
            &[
                &["税率", "0.1", "金額"],
                &["A001", "100", "110"],
                &["A002", "120", "132"],
            ],
        );
        old.rows[1][2].formula = Some("=B2*(1+$B$1)".to_string());
        old.rows[2][2].formula = Some("=B3*(1+$B$1)".to_string());
        let mut new = sheet(
            "絶対参照",
            &[
                &["税率", "0.1", "金額"],
                &["A001", "100", "110"],
                &["A999", "150", "165"],
                &["A002", "120", "132"],
            ],
        );
        new.rows[1][2].formula = Some("=B2*(1+$B$1)".to_string());
        new.rows[2][2].formula = Some("=B3*(1+$B$1)".to_string());
        new.rows[3][2].formula = Some("=B4*(1+$B$2)".to_string());

        let diff = diff_sheets(&old, &new);

        assert_eq!(diff.row_changes.len(), 1);
        assert!(
            diff.cell_changes.iter().any(|change| {
                change.old_address.as_deref() == Some("C3")
                    && change.new_address.as_deref() == Some("C4")
                    && matches!(change.kind, CellChangeKind::FormulaChanged)
            }),
            "absolute reference changes must not be hidden by row shift normalization"
        );
    }

    #[test]
    fn ignores_formula_reference_column_shift_caused_by_inserted_column() {
        let mut old = sheet(
            "数式列シフト",
            &[
                &["コード", "単価", "数量", "金額"],
                &["A001", "100", "2", "200"],
            ],
        );
        old.rows[1][3].formula = Some("=B2*C2".to_string());
        let mut new = sheet(
            "数式列シフト",
            &[
                &["コード", "税区分", "単価", "数量", "金額"],
                &["A001", "軽減", "100", "2", "200"],
            ],
        );
        new.rows[1][4].formula = Some("=C2*D2".to_string());

        let diff = diff_sheets(&old, &new);

        assert_eq!(diff.column_changes.len(), 1);
        assert!(matches!(diff.column_changes[0].kind, ChangeKind::Added));
        assert!(diff.cell_changes.is_empty());
    }

    #[test]
    fn does_not_treat_function_name_digits_as_cell_references() {
        let mut old = sheet(
            "関数名",
            &[&["コード", "値"], &["A001", "100"], &["A002", "1000"]],
        );
        old.rows[1][1].formula = Some("=LOG10(A2)".to_string());
        old.rows[2][1].formula = Some("=LOG10(A3)".to_string());
        let mut new = sheet(
            "関数名",
            &[
                &["コード", "値"],
                &["A001", "100"],
                &["A999", "10"],
                &["A002", "1000"],
            ],
        );
        new.rows[1][1].formula = Some("=LOG10(A2)".to_string());
        new.rows[2][1].formula = Some("=LOG10(A3)".to_string());
        new.rows[3][1].formula = Some("=LOG10(A4)".to_string());

        let diff = diff_sheets(&old, &new);

        assert_eq!(diff.row_changes.len(), 1);
        assert!(matches!(diff.row_changes[0].kind, ChangeKind::Added));
        assert!(diff.cell_changes.is_empty());
    }

    #[test]
    fn does_not_shift_cell_like_text_inside_formula_strings() {
        let mut old = sheet(
            "文字列",
            &[&["コード", "説明"], &["A001", "A1"], &["A002", "A2"]],
        );
        old.rows[1][1].formula = Some("=\"A1\"&A2".to_string());
        old.rows[2][1].formula = Some("=\"A2\"&A3".to_string());
        let mut new = sheet(
            "文字列",
            &[
                &["コード", "説明"],
                &["A001", "A1"],
                &["A999", "A999"],
                &["A002", "A2"],
            ],
        );
        new.rows[1][1].formula = Some("=\"A1\"&A2".to_string());
        new.rows[2][1].formula = Some("=\"A999\"&A3".to_string());
        new.rows[3][1].formula = Some("=\"A2\"&A4".to_string());

        let diff = diff_sheets(&old, &new);

        assert_eq!(diff.row_changes.len(), 1);
        assert!(matches!(diff.row_changes[0].kind, ChangeKind::Added));
        assert!(diff.cell_changes.is_empty());
    }

    #[test]
    fn reports_changed_sheet_qualified_reference_without_rewriting_sheet_name() {
        let mut old = sheet(
            "別シート参照",
            &[&["コード", "値"], &["A001", "100"], &["A002", "1000"]],
        );
        old.rows[1][1].formula = Some("=Sheet1!A2".to_string());
        old.rows[2][1].formula = Some("=Sheet1!A3".to_string());
        let mut new = sheet(
            "別シート参照",
            &[
                &["コード", "値"],
                &["A001", "100"],
                &["A999", "10"],
                &["A002", "1000"],
            ],
        );
        new.rows[1][1].formula = Some("=Sheet1!A2".to_string());
        new.rows[2][1].formula = Some("=Sheet1!A3".to_string());
        new.rows[3][1].formula = Some("=Sheet1!A4".to_string());

        let diff = diff_sheets(&old, &new);

        assert_eq!(diff.row_changes.len(), 1);
        assert!(matches!(diff.row_changes[0].kind, ChangeKind::Added));
        assert_eq!(diff.cell_changes.len(), 1);
        assert!(matches!(
            diff.cell_changes[0].kind,
            CellChangeKind::FormulaChanged
        ));
    }

    #[test]
    fn reports_changed_quoted_sheet_reference_without_rewriting_sheet_name() {
        let mut old = sheet(
            "クォート別シート参照",
            &[&["コード", "値"], &["A001", "100"], &["A002", "1000"]],
        );
        old.rows[1][1].formula = Some("='FY2024 Plan'!A2".to_string());
        old.rows[2][1].formula = Some("='FY2024 Plan'!A3".to_string());
        let mut new = sheet(
            "クォート別シート参照",
            &[
                &["コード", "値"],
                &["A001", "100"],
                &["A999", "10"],
                &["A002", "1000"],
            ],
        );
        new.rows[1][1].formula = Some("='FY2024 Plan'!A2".to_string());
        new.rows[2][1].formula = Some("='FY2024 Plan'!A3".to_string());
        new.rows[3][1].formula = Some("='FY2024 Plan'!A4".to_string());

        let diff = diff_sheets(&old, &new);

        assert_eq!(diff.row_changes.len(), 1);
        assert!(matches!(diff.row_changes[0].kind, ChangeKind::Added));
        assert_eq!(diff.cell_changes.len(), 1);
        assert!(matches!(
            diff.cell_changes[0].kind,
            CellChangeKind::FormulaChanged
        ));
    }

    #[test]
    fn reports_changed_escaped_quote_sheet_reference() {
        let mut old = sheet(
            "エスケープクォート参照",
            &[&["コード", "値"], &["A001", "100"], &["A002", "1000"]],
        );
        old.rows[1][1].formula = Some("='Bob''s A1 Plan'!A2".to_string());
        old.rows[2][1].formula = Some("='Bob''s A1 Plan'!A3".to_string());
        let mut new = sheet(
            "エスケープクォート参照",
            &[
                &["コード", "値"],
                &["A001", "100"],
                &["A999", "10"],
                &["A002", "1000"],
            ],
        );
        new.rows[1][1].formula = Some("='Bob''s A1 Plan'!A2".to_string());
        new.rows[2][1].formula = Some("='Bob''s A1 Plan'!A3".to_string());
        new.rows[3][1].formula = Some("='Bob''s A1 Plan'!A4".to_string());

        let diff = diff_sheets(&old, &new);

        assert_eq!(diff.row_changes.len(), 1);
        assert!(matches!(diff.row_changes[0].kind, ChangeKind::Added));
        assert_eq!(diff.cell_changes.len(), 1);
        assert!(matches!(
            diff.cell_changes[0].kind,
            CellChangeKind::FormulaChanged
        ));
    }

    #[test]
    fn reports_changed_external_workbook_reference() {
        let mut old = sheet(
            "外部参照",
            &[&["コード", "値"], &["A001", "100"], &["A002", "1000"]],
        );
        old.rows[1][1].formula = Some("=[Book1.xlsx]Sheet1!A2".to_string());
        old.rows[2][1].formula = Some("=[Book1.xlsx]Sheet1!A3".to_string());
        let mut new = sheet(
            "外部参照",
            &[
                &["コード", "値"],
                &["A001", "100"],
                &["A999", "10"],
                &["A002", "1000"],
            ],
        );
        new.rows[1][1].formula = Some("=[Book1.xlsx]Sheet1!A2".to_string());
        new.rows[2][1].formula = Some("=[Book1.xlsx]Sheet1!A3".to_string());
        new.rows[3][1].formula = Some("=[Book1.xlsx]Sheet1!A4".to_string());

        let diff = diff_sheets(&old, &new);

        assert_eq!(diff.row_changes.len(), 1);
        assert!(matches!(diff.row_changes[0].kind, ChangeKind::Added));
        assert_eq!(diff.cell_changes.len(), 1);
        assert!(matches!(
            diff.cell_changes[0].kind,
            CellChangeKind::FormulaChanged
        ));
    }

    #[test]
    fn does_not_shift_name_like_tokens_outside_excel_cell_range() {
        let mut old = sheet(
            "名前定義",
            &[&["コード", "値"], &["A001", "100"], &["A002", "1000"]],
        );
        old.rows[1][1].formula = Some("=PLAN2024+A2".to_string());
        old.rows[2][1].formula = Some("=PLAN2024+A3".to_string());
        let mut new = sheet(
            "名前定義",
            &[
                &["コード", "値"],
                &["A001", "100"],
                &["A999", "10"],
                &["A002", "1000"],
            ],
        );
        new.rows[1][1].formula = Some("=PLAN2024+A2".to_string());
        new.rows[2][1].formula = Some("=PLAN2024+A3".to_string());
        new.rows[3][1].formula = Some("=PLAN2024+A4".to_string());

        let diff = diff_sheets(&old, &new);

        assert_eq!(diff.row_changes.len(), 1);
        assert!(matches!(diff.row_changes[0].kind, ChangeKind::Added));
        assert!(diff.cell_changes.is_empty());
    }

    #[test]
    fn summarizes_sheet_added_removed_and_renamed() {
        let old_book = workbook(&[
            sheet("料金表", &[&["コード", "金額"], &["A001", "100"]]),
            sheet("削除予定", &[&["x"]]),
        ]);
        let new_book = workbook(&[
            sheet("価格表", &[&["コード", "金額"], &["A001", "100"]]),
            sheet("追加分", &[&["y"]]),
        ]);

        let diff = diff_workbooks(
            Path::new("old.xlsx"),
            Path::new("new.xlsx"),
            &old_book,
            &new_book,
        );

        assert_eq!(diff.summary.sheets_renamed, 1);
        assert_eq!(diff.summary.sheets_removed, 1);
        assert_eq!(diff.summary.sheets_added, 1);
        assert!(
            diff.sheets
                .iter()
                .any(|sheet| matches!(sheet.status, SheetStatus::Renamed))
        );
        assert!(
            diff.sheets
                .iter()
                .any(|sheet| matches!(sheet.status, SheetStatus::Removed))
        );
        assert!(
            diff.sheets
                .iter()
                .any(|sheet| matches!(sheet.status, SheetStatus::Added))
        );
    }

    #[test]
    fn canonical_unicode_sheet_names_are_not_reported_as_renames() {
        let old_book = workbook(&[sheet("Caf\u{e9}", &[&["value"], &["100"]])]);
        let new_book = workbook(&[sheet("Cafe\u{301}", &[&["value"], &["100"]])]);

        let diff = diff_workbooks(
            Path::new("old.xlsx"),
            Path::new("new.xlsx"),
            &old_book,
            &new_book,
        );

        assert!(!diff.has_changes(), "{diff:?}");
        assert_eq!(diff.summary.sheets_renamed, 0);
        assert_eq!(diff.sheets.len(), 1);
        assert!(matches!(diff.sheets[0].status, SheetStatus::Unchanged));
    }

    #[test]
    fn writes_json_html_and_xlsx_reports() -> Result<()> {
        let old_book = workbook(&[sheet(
            "料金表",
            &[
                &["商品コード", "商品名", "単価"],
                &["A001", "りんご", "100"],
            ],
        )]);
        let new_book = workbook(&[sheet(
            "料金表",
            &[
                &["商品コード", "商品名", "税区分", "単価"],
                &["A001", "りんご", "軽減", "120"],
            ],
        )]);
        let diff = diff_workbooks(
            Path::new("old.xlsx"),
            Path::new("new.xlsx"),
            &old_book,
            &new_book,
        );
        let dir = unique_test_dir("reports")?;
        let json_path = dir.join("diff.json");
        let html_path = dir.join("diff.html");
        let xlsx_path = dir.join("diff.xlsx");

        std::fs::write(&json_path, serde_json::to_string_pretty(&diff)?)?;
        write_html_report(&html_path, &diff)?;
        write_xlsx_report(&xlsx_path, &diff)?;

        let json = std::fs::read_to_string(json_path)?;
        let html = std::fs::read_to_string(html_path)?;
        assert!(json.contains("\"columns_added\": 1"));
        assert!(html.contains("xlsx diff"));
        assert!(xlsx_path.exists());
        assert!(std::fs::metadata(xlsx_path)?.len() > 0);
        Ok(())
    }

    #[test]
    fn xlsx_report_contains_per_sheet_grid_view() -> Result<()> {
        let old_book = workbook(&[sheet(
            "料金表",
            &[
                &["商品コード", "商品名", "単価"],
                &["A001", "りんご", "100"],
            ],
        )]);
        let new_book = workbook(&[sheet(
            "料金表",
            &[
                &["商品コード", "商品名", "税区分", "単価"],
                &["A001", "りんご", "軽減", "120"],
            ],
        )]);
        let diff = diff_workbooks(
            Path::new("old.xlsx"),
            Path::new("new.xlsx"),
            &old_book,
            &new_book,
        );
        let dir = unique_test_dir("grid-report")?;
        let xlsx_path = dir.join("diff.xlsx");

        write_xlsx_report(&xlsx_path, &diff)?;

        let mut report: Xlsx<_> = open_workbook(&xlsx_path)?;
        assert!(
            report
                .sheet_names()
                .iter()
                .any(|name| name == "Grid - 料金表")
        );
        let grid = report.worksheet_range("Grid - 料金表")?;
        let values: Vec<String> = grid
            .rows()
            .flat_map(|row| row.iter().map(normalize_value))
            .collect();
        assert!(values.iter().any(|value| value == "D2"));
        assert!(values.iter().any(|value| value == "120 [string]"));
        assert!(values.iter().any(|value| value == "value_changed"));
        Ok(())
    }

    #[test]
    fn html_and_xlsx_reports_support_side_by_side_layout() -> Result<()> {
        let old_book = workbook(&[sheet(
            "料金表",
            &[
                &["商品コード", "商品名", "単価"],
                &["A001", "りんご", "100"],
            ],
        )]);
        let new_book = workbook(&[sheet(
            "料金表",
            &[
                &["商品コード", "商品名", "税区分", "単価"],
                &["A001", "りんご", "軽減", "120"],
            ],
        )]);
        let diff = diff_workbooks(
            Path::new("old.xlsx"),
            Path::new("new.xlsx"),
            &old_book,
            &new_book,
        );
        let dir = unique_test_dir("side-by-side-report")?;
        let html_path = dir.join("diff.html");
        let xlsx_path = dir.join("diff.xlsx");

        write_html_report_with_layout(&html_path, &diff, ReportLayout::SideBySide)?;
        write_xlsx_report_with_layout(&xlsx_path, &diff, ReportLayout::SideBySide)?;

        let html = std::fs::read_to_string(html_path)?;
        assert!(html.contains("class=\"side-by-side\""));
        assert!(html.contains(">OLD<"));
        assert!(html.contains(">NEW<"));
        assert!(html.contains("C2"));
        assert!(html.contains("D2"));

        let mut report: Xlsx<_> = open_workbook(&xlsx_path)?;
        assert!(
            report
                .sheet_names()
                .iter()
                .any(|name| name == "Side - 料金表")
        );
        assert!(
            report
                .sheet_names()
                .iter()
                .all(|name| !name.starts_with("Grid - "))
        );
        let side = report.worksheet_range("Side - 料金表")?;
        let values: Vec<String> = side
            .rows()
            .flat_map(|row| row.iter().map(normalize_value))
            .collect();
        for expected in ["OLD", "NEW", "C2", "D2", "100 [string]", "120 [string]"] {
            assert!(values.iter().any(|value| value == expected));
        }
        Ok(())
    }

    #[test]
    fn html_report_includes_moved_region_and_merge_sections() -> Result<()> {
        let mut diff = diff_workbooks(
            Path::new("old.xlsx"),
            Path::new("new.xlsx"),
            &workbook(&[sheet(
                "並び替え",
                &[&["コード", "単価"], &["A001", "100"], &["A002", "120"]],
            )]),
            &workbook(&[sheet(
                "並び替え",
                &[&["コード", "単価"], &["A002", "120"], &["A001", "100"]],
            )]),
        );
        diff.sheets[0].merge_changes.push(MergeChange {
            kind: ChangeKind::Added,
            old_range: None,
            new_range: Some(CellRange {
                start_row: 1,
                start_col: 1,
                end_row: 1,
                end_col: 2,
            }),
        });
        let dir = unique_test_dir("html-detail")?;
        let html_path = dir.join("diff.html");

        write_html_report(&html_path, &diff)?;

        let html = std::fs::read_to_string(html_path)?;
        assert!(html.contains("Moved"));
        assert!(html.contains("Table Regions"));
        assert!(html.contains("Merged Cells"));
        Ok(())
    }

    #[test]
    fn reports_table_regions_split_by_blank_rows_and_columns() {
        let old = sheet(
            "複数表",
            &[
                &["商品コード", "商品名", "", "担当者コード", "担当者名"],
                &["A001", "りんご", "", "U001", "田中"],
                &["", "", "", "", ""],
                &["条件", "値", "", "", ""],
                &["支払", "月末", "", "", ""],
            ],
        );
        let mut new = old.clone();
        new.rows[1][1].value = "りんご改".to_string();

        let diff = diff_sheets(&old, &new);

        assert_eq!(diff.table_regions.len(), 3);
        assert_eq!(
            diff.table_regions
                .iter()
                .filter(|region| matches!(region.status, RegionStatus::Modified))
                .count(),
            1
        );
        let modified = diff
            .table_regions
            .iter()
            .find(|region| matches!(region.status, RegionStatus::Modified))
            .unwrap();
        assert_eq!(
            modified.old_range.as_ref().map(range_address).as_deref(),
            Some("A1:B2")
        );
        assert_eq!(modified.cell_changes.len(), 1);
        assert_eq!(modified.cell_changes[0].old_address.as_deref(), Some("B2"));
    }

    #[test]
    fn reports_merged_cell_changes() {
        let old = sheet_with_merges(
            "結合",
            &[&["料金表", ""], &["コード", "金額"]],
            &[CellRange {
                start_row: 1,
                start_col: 1,
                end_row: 1,
                end_col: 2,
            }],
        );
        let new = sheet_with_merges(
            "結合",
            &[&["料金表", "", ""], &["コード", "金額", "備考"]],
            &[CellRange {
                start_row: 1,
                start_col: 1,
                end_row: 1,
                end_col: 3,
            }],
        );

        let diff = diff_sheets(&old, &new);

        assert_eq!(diff.merge_changes.len(), 2);
        assert_eq!(
            diff.merge_changes
                .iter()
                .filter(|change| matches!(change.kind, ChangeKind::Removed))
                .count(),
            1
        );
        assert_eq!(
            diff.merge_changes
                .iter()
                .filter(|change| matches!(change.kind, ChangeKind::Added))
                .count(),
            1
        );
    }

    #[test]
    fn shifted_merged_range_is_not_reported_as_content_change() {
        let old = sheet_with_merges(
            "merged",
            &[&["title", ""], &["id", "value"]],
            &[CellRange {
                start_row: 1,
                start_col: 1,
                end_row: 1,
                end_col: 2,
            }],
        );
        let new = sheet_with_merges(
            "merged",
            &[&["notice", ""], &["title", ""], &["id", "value"]],
            &[CellRange {
                start_row: 2,
                start_col: 1,
                end_row: 2,
                end_col: 2,
            }],
        );

        let diff = diff_sheets(&old, &new);

        assert!(diff.merge_changes.is_empty());
    }

    #[test]
    fn loads_merged_cells_from_xlsx_file() -> Result<()> {
        let dir = unique_test_dir("merged-input")?;
        let path = dir.join("merged.xlsx");
        let mut workbook = Workbook::new();
        let worksheet = workbook.add_worksheet();
        worksheet.set_name("結合")?;
        worksheet.merge_range(0, 0, 0, 1, "料金表", &Format::new())?;
        workbook.save(&path)?;

        let loaded = load_workbook(&path)?;

        assert_eq!(loaded.sheets.len(), 1);
        assert_eq!(
            loaded.sheets[0].merged_ranges,
            vec![CellRange {
                start_row: 1,
                start_col: 1,
                end_row: 1,
                end_col: 2,
            }]
        );
        Ok(())
    }

    #[test]
    fn reports_sheet_order_changes() {
        let old_book = workbook(&[
            sheet("A", &[&["a"]]),
            sheet("B", &[&["b"]]),
            sheet("C", &[&["c"]]),
        ]);
        let new_book = workbook(&[
            sheet("B", &[&["b"]]),
            sheet("A", &[&["a"]]),
            sheet("C", &[&["c"]]),
        ]);

        let diff = diff_workbooks(
            Path::new("old.xlsx"),
            Path::new("new.xlsx"),
            &old_book,
            &new_book,
        );

        assert_eq!(diff.summary.sheet_order_changes, 1);
        assert_eq!(
            diff.sheets
                .iter()
                .filter(|sheet| sheet.order_changed)
                .count(),
            1
        );
        assert!(
            diff.sheets
                .iter()
                .any(|sheet| matches!(sheet.status, SheetStatus::Reordered))
        );
    }

    #[test]
    fn reports_row_and_column_moves_without_add_remove_noise() {
        let old = sheet(
            "並び替え",
            &[
                &["コード", "商品名", "単価", "数量"],
                &["A001", "りんご", "100", "2"],
                &["A002", "みかん", "120", "3"],
                &["A003", "バナナ", "150", "1"],
            ],
        );
        let new = sheet(
            "並び替え",
            &[
                &["コード", "商品名", "数量", "単価"],
                &["A003", "バナナ", "1", "150"],
                &["A001", "りんご", "2", "100"],
                &["A002", "みかん", "3", "120"],
            ],
        );

        let diff = diff_sheets(&old, &new);

        assert_eq!(
            diff.row_changes
                .iter()
                .filter(|change| matches!(change.kind, ChangeKind::Moved))
                .count(),
            1
        );
        assert_eq!(
            diff.column_changes
                .iter()
                .filter(|change| matches!(change.kind, ChangeKind::Moved))
                .count(),
            1
        );
        assert!(
            diff.row_changes
                .iter()
                .all(|change| !matches!(change.kind, ChangeKind::Added | ChangeKind::Removed))
        );
        assert!(
            diff.column_changes
                .iter()
                .all(|change| !matches!(change.kind, ChangeKind::Added | ChangeKind::Removed))
        );
        assert!(diff.cell_changes.is_empty());
    }

    #[test]
    fn summarizes_added_and_removed_sheet_contents() {
        let old_book = workbook(&[sheet("削除予定", &[&["コード", "金額"], &["A001", "100"]])]);
        let new_book = workbook(&[sheet("追加分", &[&["コード", "金額"], &["B001", "200"]])]);

        let diff = diff_workbooks(
            Path::new("old.xlsx"),
            Path::new("new.xlsx"),
            &old_book,
            &new_book,
        );

        assert_eq!(diff.summary.table_regions_removed, 1);
        assert_eq!(diff.summary.table_regions_added, 1);
        let removed = diff
            .sheets
            .iter()
            .find(|sheet| matches!(sheet.status, SheetStatus::Removed))
            .unwrap();
        let added = diff
            .sheets
            .iter()
            .find(|sheet| matches!(sheet.status, SheetStatus::Added))
            .unwrap();
        assert_eq!(removed.table_regions.len(), 1);
        assert_eq!(added.table_regions.len(), 1);
        assert!(matches!(
            removed.table_regions[0].status,
            RegionStatus::Removed
        ));
        assert!(matches!(added.table_regions[0].status, RegionStatus::Added));
    }

    #[test]
    fn end_to_end_xlsx_detects_inserted_row_and_column() -> Result<()> {
        let dir = unique_test_dir("e2e-insert")?;
        let old_path = dir.join("old.xlsx");
        let new_path = dir.join("new.xlsx");
        write_xlsx_rows(
            &old_path,
            &[
                &["id", "name", "amount"],
                &["A001", "apple", "100"],
                &["A002", "orange", "120"],
            ],
            0,
            0,
        )?;
        write_xlsx_rows(
            &new_path,
            &[
                &["id", "name", "tax", "amount"],
                &["A001", "apple", "low", "100"],
                &["A999", "banana", "normal", "150"],
                &["A002", "orange", "low", "120"],
            ],
            0,
            0,
        )?;

        let old = load_workbook(&old_path)?;
        let new = load_workbook(&new_path)?;
        let diff = diff_workbooks(&old_path, &new_path, &old, &new);

        assert_eq!(diff.summary.rows_added, 1);
        assert_eq!(diff.summary.columns_added, 1);
        assert_eq!(diff.summary.cells_changed, 0);
        Ok(())
    }

    #[test]
    fn workbook_pair_loader_preserves_both_inputs() -> Result<()> {
        let dir = unique_test_dir("parallel-workbook-pair")?;
        let old_path = dir.join("old.xlsx");
        let new_path = dir.join("new.xlsx");
        write_xlsx_rows(&old_path, &[&["old", "1"]], 0, 0)?;
        write_xlsx_rows(&new_path, &[&["new", "2"]], 0, 0)?;

        let (old, new) = load_workbook_pair(&old_path, &new_path)?;

        assert_eq!(old.sheets[0].rows[0][0].value, "old");
        assert_eq!(new.sheets[0].rows[0][0].value, "new");
        Ok(())
    }

    #[test]
    fn dense_rows_builder_expands_beyond_inaccurate_dimension_hint() -> Result<()> {
        let mut builder = DenseRowsBuilder::new(Dimensions::new((5, 5), (6, 6)), 100, 100, 100);
        builder.push((5, 5), typed_row(&["center"]).pop().unwrap())?;
        builder.push((7, 8), typed_row(&["bottom-right"]).pop().unwrap())?;
        builder.push((4, 3), typed_row(&["top-left"]).pop().unwrap())?;

        let (rows, ((start_row, start_col), (end_row, end_col))) =
            builder.finish().expect("inserted cells produce rows");

        assert_eq!((start_row, start_col), (4, 3));
        assert_eq!((end_row, end_col), (7, 8));
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|row| row.len() == 6));
        assert_eq!(rows[0][0].value, "top-left");
        assert_eq!(rows[1][2].value, "center");
        assert_eq!(rows[3][5].value, "bottom-right");
        Ok(())
    }

    #[test]
    fn dense_rows_builder_rejects_outliers_before_allocating_them() -> Result<()> {
        let mut builder = DenseRowsBuilder::new(Dimensions::new((0, 0), (0, 0)), 2, 2, 4);
        builder.push((0, 0), typed_row(&["first"]).pop().unwrap())?;

        let error = builder
            .push((100_000, 0), typed_row(&["outlier"]).pop().unwrap())
            .expect_err("oversized outlier must be rejected before row expansion");

        assert!(error.to_string().contains("max_rows_per_sheet"));
        Ok(())
    }

    #[test]
    fn sparse_xlsx_preserves_origin_and_formula_coordinate() -> Result<()> {
        let dir = unique_test_dir("sparse-origin")?;
        let path = dir.join("sparse.xlsx");
        let mut workbook = Workbook::new();
        let worksheet = workbook.add_worksheet();
        worksheet.write_string(9, 2, "amount")?;
        worksheet.write_formula(9, 3, "=C10")?;
        workbook.save(&path)?;

        let loaded = load_workbook(&path)?;
        let sheet = &loaded.sheets[0];

        assert_eq!((sheet.start_row, sheet.start_col), (10, 3));
        assert_eq!(sheet.rows[0][0].value, "amount");
        assert_eq!(sheet.rows[0][1].formula.as_deref(), Some("=C10"));
        Ok(())
    }

    #[test]
    fn xlsx_stream_loader_expands_shared_formulas() -> Result<()> {
        let dir = unique_test_dir("shared-formulas")?;
        let source_path = dir.join("source.xlsx");
        let shared_path = dir.join("shared.xlsx");
        let mut workbook = Workbook::new();
        let worksheet = workbook.add_worksheet();
        worksheet.write_formula(0, 0, "=B1")?;
        worksheet.write_formula(1, 0, "=B2")?;
        workbook.save(&source_path)?;
        rewrite_xlsx_part(
            &source_path,
            &shared_path,
            "xl/worksheets/sheet1.xml",
            |xml| {
                xml.replacen(
                    "<f>B1</f>",
                    "<f t=\"shared\" si=\"0\" ref=\"A1:A2\">B1</f>",
                    1,
                )
                .replacen("<f>B2</f>", "<f t=\"shared\" si=\"0\"/>", 1)
            },
        )?;

        let loaded = load_workbook(&shared_path)?;
        let sheet = &loaded.sheets[0];

        assert_eq!(sheet.rows[0][0].formula.as_deref(), Some("=B1"));
        assert_eq!(sheet.rows[1][0].formula.as_deref(), Some("=B2"));
        Ok(())
    }

    #[test]
    fn actual_xlsx_preserves_string_and_number_types() -> Result<()> {
        let dir = unique_test_dir("cell-types")?;
        let old_path = dir.join("old.xlsx");
        let new_path = dir.join("new.xlsx");
        let mut old_workbook = Workbook::new();
        old_workbook.add_worksheet().write_string(0, 0, "1")?;
        old_workbook.save(&old_path)?;
        let mut new_workbook = Workbook::new();
        new_workbook.add_worksheet().write_number(0, 0, 1.0)?;
        new_workbook.save(&new_path)?;

        let old = load_workbook(&old_path)?;
        let new = load_workbook(&new_path)?;
        let diff = diff_workbooks(&old_path, &new_path, &old, &new);

        assert_eq!(diff.summary.cells_changed, 1);
        assert_eq!(
            diff.sheets[0].cell_changes[0].old_value_kind,
            Some(CellValueKind::String)
        );
        assert_eq!(
            diff.sheets[0].cell_changes[0].new_value_kind,
            Some(CellValueKind::Number)
        );
        Ok(())
    }

    #[test]
    fn actual_xlsx_detects_cell_style_only_changes() -> Result<()> {
        let dir = unique_test_dir("cell-style-change")?;
        let old_path = dir.join("old.xlsx");
        let new_path = dir.join("new.xlsx");

        let mut old_workbook = Workbook::new();
        old_workbook.add_worksheet().write_string(0, 0, "same")?;
        old_workbook.save(&old_path)?;

        let changed_format = Format::new().set_background_color(Color::RGB(0xFFCC00));
        let mut new_workbook = Workbook::new();
        new_workbook
            .add_worksheet()
            .write_string_with_format(0, 0, "same", &changed_format)?;
        new_workbook.save(&new_path)?;

        let old = load_workbook(&old_path)?;
        let new = load_workbook(&new_path)?;
        let diff = diff_workbooks(&old_path, &new_path, &old, &new);

        assert!(diff.has_changes(), "{diff:?}");
        assert_eq!(diff.summary.cells_changed, 1, "{diff:?}");
        assert_eq!(diff.summary.cell_styles_changed, 1, "{diff:?}");
        assert!(matches!(
            diff.sheets[0].cell_changes[0].kind,
            CellChangeKind::StyleChanged
        ));
        assert!(diff.sheets[0].cell_changes[0].style_changed);
        Ok(())
    }

    #[test]
    fn actual_xlsx_detects_referenced_theme_color_changes() -> Result<()> {
        let dir = unique_test_dir("theme-color-change")?;
        let old_path = dir.join("old.xlsx");
        let new_path = dir.join("new.xlsx");
        let themed_format = Format::new().set_background_color(Color::Theme(4, 0));

        let mut workbook = Workbook::new();
        workbook
            .add_worksheet()
            .write_string_with_format(0, 0, "same", &themed_format)?;
        workbook.save(&old_path)?;
        rewrite_xlsx_part(&old_path, &new_path, "xl/theme/theme1.xml", |xml| {
            xml.replace("val=\"4F81BD\"", "val=\"AA0000\"")
        })?;

        let old = load_workbook(&old_path)?;
        let new = load_workbook(&new_path)?;
        let diff = diff_workbooks(&old_path, &new_path, &old, &new);

        assert_eq!(diff.summary.cell_styles_changed, 1, "{diff:?}");
        assert!(diff.sheets[0].cell_changes[0].style_changed);
        Ok(())
    }

    #[test]
    fn actual_xlsx_ignores_unreferenced_theme_color_changes() -> Result<()> {
        let dir = unique_test_dir("unused-theme-color-change")?;
        let old_path = dir.join("old.xlsx");
        let new_path = dir.join("new.xlsx");
        let themed_format = Format::new().set_background_color(Color::Theme(4, 0));

        let mut workbook = Workbook::new();
        workbook
            .add_worksheet()
            .write_string_with_format(0, 0, "same", &themed_format)?;
        workbook.save(&old_path)?;
        rewrite_xlsx_part(&old_path, &new_path, "xl/theme/theme1.xml", |xml| {
            xml.replace("val=\"C0504D\"", "val=\"00AA00\"")
        })?;

        let old = load_workbook(&old_path)?;
        let new = load_workbook(&new_path)?;
        let diff = diff_workbooks(&old_path, &new_path, &old, &new);

        assert!(!diff.has_changes(), "{diff:?}");
        Ok(())
    }

    #[test]
    fn actual_xlsx_ignores_equivalent_theme_and_direct_colors() -> Result<()> {
        let dir = unique_test_dir("equivalent-theme-direct-color")?;
        let old_path = dir.join("old.xlsx");
        let new_path = dir.join("new.xlsx");

        let mut old_workbook = Workbook::new();
        old_workbook.add_worksheet().write_string_with_format(
            0,
            0,
            "same",
            &Format::new().set_background_color(Color::Theme(4, 0)),
        )?;
        old_workbook.save(&old_path)?;

        let mut new_workbook = Workbook::new();
        new_workbook.add_worksheet().write_string_with_format(
            0,
            0,
            "same",
            &Format::new().set_background_color(Color::RGB(0x4F81BD)),
        )?;
        new_workbook.save(&new_path)?;

        let old = load_workbook(&old_path)?;
        let new = load_workbook(&new_path)?;
        let diff = diff_workbooks(&old_path, &new_path, &old, &new);

        assert!(!diff.has_changes(), "{diff:?}");
        Ok(())
    }

    #[test]
    fn actual_xlsx_ignores_explicit_default_cell_style() -> Result<()> {
        let dir = unique_test_dir("default-cell-style")?;
        let old_path = dir.join("old.xlsx");
        let new_path = dir.join("new.xlsx");

        let mut old_workbook = Workbook::new();
        old_workbook.add_worksheet().write_string(0, 0, "same")?;
        old_workbook.save(&old_path)?;

        let mut new_workbook = Workbook::new();
        new_workbook
            .add_worksheet()
            .write_string_with_format(0, 0, "same", &Format::new())?;
        new_workbook.save(&new_path)?;

        let old = load_workbook(&old_path)?;
        let new = load_workbook(&new_path)?;
        let diff = diff_workbooks(&old_path, &new_path, &old, &new);

        assert!(!diff.has_changes(), "{diff:?}");
        Ok(())
    }

    #[test]
    fn worksheet_style_parser_streams_cells_and_omits_default_style() -> Result<()> {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
            <worksheet><sheetData><row r="1">
                <c r="A1" s="0"><v>1</v></c>
                <c s="2" r="B1"/>
                <c r="C1" s="3"><v>3</v></c>
            </row></sheetData></worksheet>"#;

        let styles = parse_worksheet_style_ids(xml)?;

        assert_eq!(styles.len(), 2);
        assert_eq!(styles.get(&(1, 2)), Some(&2));
        assert_eq!(styles.get(&(1, 3)), Some(&3));
        assert!(!styles.contains_key(&(1, 1)));
        assert!(!has_non_default_cell_styles(&[]));
        assert!(!has_non_default_cell_styles(&[
            "default".to_string(),
            "default".to_string(),
        ]));
        assert!(has_non_default_cell_styles(&[
            "default".to_string(),
            "changed".to_string(),
        ]));
        Ok(())
    }

    #[test]
    fn actual_xlsx_ignores_explicit_default_alignment_and_protection() -> Result<()> {
        let dir = unique_test_dir("default-alignment-protection")?;
        let old_path = dir.join("old.xlsx");
        let new_path = dir.join("new.xlsx");

        let mut workbook = Workbook::new();
        workbook.add_worksheet().write_string(0, 0, "same")?;
        workbook.save(&old_path)?;
        rewrite_xlsx_part(&old_path, &new_path, "xl/styles.xml", |xml| {
            let original = r#"<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>"#;
            assert!(xml.contains(original), "unexpected styles.xml: {xml}");
            xml.replacen(
                original,
                r#"<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0" applyAlignment="1" applyProtection="1"><alignment horizontal="general" vertical="bottom" textRotation="0" wrapText="0" shrinkToFit="0" indent="0" relativeIndent="0" justifyLastLine="0" readingOrder="0"/><protection locked="1" hidden="0"/></xf>"#,
                1,
            )
        })?;

        let old = load_workbook(&old_path)?;
        let new = load_workbook(&new_path)?;
        let diff = diff_workbooks(&old_path, &new_path, &old, &new);

        assert!(!diff.has_changes(), "{diff:?}");
        Ok(())
    }

    #[test]
    fn actual_xlsx_detects_style_changes_on_empty_cells() -> Result<()> {
        let dir = unique_test_dir("empty-cell-style")?;
        let old_path = dir.join("old.xlsx");
        let new_path = dir.join("new.xlsx");

        let mut old_workbook = Workbook::new();
        old_workbook.add_worksheet().write_string(1, 1, "anchor")?;
        old_workbook.save(&old_path)?;

        let changed_format = Format::new().set_background_color(Color::RGB(0xCCEEFF));
        let mut new_workbook = Workbook::new();
        let worksheet = new_workbook.add_worksheet();
        worksheet.write_blank(0, 0, &changed_format)?;
        worksheet.write_string(1, 1, "anchor")?;
        new_workbook.save(&new_path)?;

        let old = load_workbook(&old_path)?;
        let new = load_workbook(&new_path)?;
        let diff = diff_workbooks(&old_path, &new_path, &old, &new);

        assert_eq!(diff.summary.cell_styles_changed, 1, "{diff:?}");
        assert!(
            diff.sheets[0]
                .cell_changes
                .iter()
                .any(|change| change.new_address.as_deref() == Some("A1") && change.style_changed)
        );
        Ok(())
    }

    #[test]
    fn actual_xlsx_does_not_report_style_change_when_styled_cell_moves_with_row() -> Result<()> {
        let dir = unique_test_dir("styled-cell-row-move")?;
        let old_path = dir.join("old.xlsx");
        let new_path = dir.join("new.xlsx");
        let format = Format::new().set_background_color(Color::RGB(0xDDEEFF));

        let mut old_workbook = Workbook::new();
        let old_sheet = old_workbook.add_worksheet();
        old_sheet.write_string(0, 0, "header")?;
        old_sheet.write_string_with_format(1, 0, "same", &format)?;
        old_workbook.save(&old_path)?;

        let mut new_workbook = Workbook::new();
        let new_sheet = new_workbook.add_worksheet();
        new_sheet.write_string(0, 0, "inserted")?;
        new_sheet.write_string(1, 0, "header")?;
        new_sheet.write_string_with_format(2, 0, "same", &format)?;
        new_workbook.save(&new_path)?;

        let old = load_workbook(&old_path)?;
        let new = load_workbook(&new_path)?;
        let diff = diff_workbooks(&old_path, &new_path, &old, &new);

        assert_eq!(diff.summary.cell_styles_changed, 0, "{diff:?}");
        Ok(())
    }

    #[test]
    fn actual_xlsx_preserves_boolean_and_datetime_types() -> Result<()> {
        let dir = unique_test_dir("boolean-datetime-types")?;
        let path = dir.join("types.xlsx");
        let mut workbook = Workbook::new();
        let worksheet = workbook.add_worksheet();
        worksheet.write_boolean(0, 0, true)?;
        worksheet.write_datetime_with_format(
            0,
            1,
            ExcelDateTime::from_ymd(2026, 7, 20)?,
            &Format::new().set_num_format("yyyy-mm-dd"),
        )?;
        workbook.save(&path)?;

        let loaded = load_workbook(&path)?;

        assert_eq!(
            loaded.sheets[0].rows[0][0].value_kind,
            CellValueKind::Boolean
        );
        assert_eq!(
            loaded.sheets[0].rows[0][1].value_kind,
            CellValueKind::DateTime
        );
        Ok(())
    }

    #[test]
    fn whitespace_normalization_is_configurable() -> Result<()> {
        let dir = unique_test_dir("whitespace")?;
        let old_path = dir.join("old.xlsx");
        let new_path = dir.join("new.xlsx");
        write_xlsx_rows(&old_path, &[&["a  b"]], 0, 0)?;
        write_xlsx_rows(&new_path, &[&["a b"]], 0, 0)?;

        let collapsed = DiffOptions::default();
        let old = load_workbook_with_options(&old_path, &collapsed)?;
        let new = load_workbook_with_options(&new_path, &collapsed)?;
        assert!(
            !diff_workbooks_with_options(&old_path, &new_path, &old, &new, &collapsed,)?
                .has_changes()
        );

        let preserved = DiffOptions {
            whitespace: WhitespaceNormalization::Preserve,
            ..DiffOptions::default()
        };
        let old = load_workbook_with_options(&old_path, &preserved)?;
        let new = load_workbook_with_options(&new_path, &preserved)?;
        assert!(
            diff_workbooks_with_options(&old_path, &new_path, &old, &new, &preserved,)?
                .has_changes()
        );
        Ok(())
    }

    #[test]
    fn sheet_insertion_does_not_report_reorder_noise() {
        let old = workbook(&[sheet("A", &[&["a"]]), sheet("B", &[&["b"]])]);
        let new = workbook(&[
            sheet("X", &[&["x"]]),
            sheet("A", &[&["a"]]),
            sheet("B", &[&["b"]]),
        ]);

        let diff = diff_workbooks(Path::new("old.xlsx"), Path::new("new.xlsx"), &old, &new);

        assert_eq!(diff.summary.sheets_added, 1);
        assert_eq!(diff.summary.sheet_order_changes, 0);
    }

    #[test]
    fn duplicate_rows_are_exposed_as_ambiguous_alignment_groups() {
        let old = sheet("duplicates", &[&["same"], &["same"]]);
        let new = old.clone();

        let diff = diff_sheets(&old, &new);

        assert_eq!(diff.alignment.row_ambiguous_signature_groups, 1);
        assert_eq!(diff.alignment.minimum_row_confidence, Some(1.0));
    }

    #[test]
    fn stale_formula_reference_after_insertion_is_reported() {
        let mut old = sheet("formula", &[&["id", "value"], &["A1", "10"], &["A2", "20"]]);
        old.rows[2][1].formula = Some("=A3".to_string());
        let mut new = sheet(
            "formula",
            &[
                &["id", "value"],
                &["A1", "10"],
                &["NEW", "15"],
                &["A2", "20"],
            ],
        );
        new.rows[3][1].formula = Some("=A3".to_string());

        let diff = diff_sheets(&old, &new);

        assert!(
            diff.cell_changes
                .iter()
                .any(|change| matches!(change.kind, CellChangeKind::FormulaChanged))
        );
    }

    #[test]
    fn input_and_alignment_limits_fail_cleanly() -> Result<()> {
        let dir = unique_test_dir("limits")?;
        let path = dir.join("book.xlsx");
        write_xlsx_rows(&path, &[&["a"]], 0, 0)?;
        let options = DiffOptions {
            limits: ResourceLimits {
                max_input_bytes: 1,
                ..ResourceLimits::default()
            },
            ..DiffOptions::default()
        };
        assert!(load_workbook_with_options(&path, &options).is_err());

        let old = sheet("data", &[&["A"], &["B"]]);
        let new = sheet("data", &[&["X"], &["Y"]]);
        let options = DiffOptions {
            limits: ResourceLimits {
                max_alignment_pairs: 1,
                ..ResourceLimits::default()
            },
            ..DiffOptions::default()
        };
        assert!(super::diff_sheets(&old, &new, &options, None).is_err());

        let old_book = workbook(&[sheet("old-a", &[&["A"]]), sheet("old-b", &[&["B"]])]);
        let new_book = workbook(&[sheet("new-a", &[&["A"]]), sheet("new-b", &[&["B"]])]);
        let options = DiffOptions {
            limits: ResourceLimits {
                max_alignment_pairs: 3,
                ..ResourceLimits::default()
            },
            ..DiffOptions::default()
        };
        assert!(
            diff_workbooks_with_options(
                Path::new("old.xlsx"),
                Path::new("new.xlsx"),
                &old_book,
                &new_book,
                &options,
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn json_contract_is_versioned_and_one_based() {
        let old = workbook(&[sheet("A", &[&["id", "value"], &["A", "old"]])]);
        let new = workbook(&[sheet("A", &[&["id", "value"], &["A", "new"]])]);
        let diff = diff_workbooks(Path::new("old.xlsx"), Path::new("new.xlsx"), &old, &new);
        let value = serde_json::to_value(diff).unwrap();

        assert_eq!(value["schema_version"], DIFF_SCHEMA_VERSION);
        assert_eq!(value["coordinate_system"], "one_based");
        assert_eq!(value["sheets"][0]["cell_changes"][0]["old_address"], "B2");
    }

    #[test]
    fn checked_in_json_schema_matches_contract_version() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../schema/diff-v1.schema.json")).unwrap();

        assert_eq!(
            schema["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
        assert_eq!(
            schema["properties"]["schema_version"]["const"],
            DIFF_SCHEMA_VERSION
        );
        assert_eq!(
            schema["properties"]["coordinate_system"]["const"],
            "one_based"
        );
        assert!(schema["$defs"]["diff_options"]["properties"]["formula_values"].is_object());
        assert!(
            schema["$defs"]["resource_limits"]["properties"]["max_assignment_size"].is_object()
        );
    }

    #[test]
    fn global_assignment_avoids_greedy_trap_and_is_deterministic() -> Result<()> {
        let scores = vec![vec![0.90, 0.80], vec![0.85, 0.10]];

        let assignment = maximum_weight_assignment(&scores, 0.60, 8)?;

        assert_eq!(assignment, vec![Some((1, 0.80)), Some((0, 0.85))]);
        assert_eq!(assignment, maximum_weight_assignment(&scores, 0.60, 8)?);
        assert!(maximum_weight_assignment(&scores, 0.60, 1).is_err());
        Ok(())
    }

    #[test]
    fn global_assignment_accepts_default_dimension_boundary() -> Result<()> {
        let limit = ResourceLimits::default().max_assignment_size;
        let tall = vec![vec![1.0]; limit];
        let wide = vec![vec![1.0; limit]];

        assert_eq!(maximum_weight_assignment(&tall, 0.6, limit)?.len(), limit);
        assert_eq!(maximum_weight_assignment(&wide, 0.6, limit)?.len(), 1);
        assert!(maximum_weight_assignment(&vec![vec![1.0]; limit + 1], 0.6, limit).is_err());
        Ok(())
    }

    #[test]
    fn same_typed_single_cell_change_stays_a_cell_change() {
        let mut old = sheet("Sheet1", &[&["2025-01-01"]]);
        old.rows[0][0].value_kind = CellValueKind::DateTime;
        let mut new = sheet("Sheet1", &[&["2026-01-01"]]);
        new.rows[0][0].value_kind = CellValueKind::DateTime;

        let diff = diff_sheets(&old, &new);

        assert!(diff.row_changes.is_empty());
        assert!(diff.column_changes.is_empty());
        assert_eq!(diff.cell_changes.len(), 1);
        assert!(matches!(
            diff.cell_changes[0].kind,
            CellChangeKind::ValueChanged
        ));
    }

    #[test]
    fn workbook_formula_references_follow_target_sheet_insert_and_rename() {
        let mut old_summary = sheet("Summary", &[&["id", "total"], &["A", "20"]]);
        old_summary.rows[1][1].formula = Some("=Data!B3".to_string());
        let old = workbook(&[
            sheet("Data", &[&["id", "value"], &["A", "10"], &["B", "20"]]),
            old_summary,
        ]);

        let mut new_summary = sheet("Summary", &[&["id", "total"], &["A", "20"]]);
        new_summary.rows[1][1].formula = Some("='Data 2026'!B4".to_string());
        let new = workbook(&[
            sheet(
                "Data 2026",
                &[&["id", "value"], &["A", "10"], &["NEW", "15"], &["B", "20"]],
            ),
            new_summary,
        ]);

        let diff = diff_workbooks(Path::new("old.xlsx"), Path::new("new.xlsx"), &old, &new);

        let summary = diff
            .sheets
            .iter()
            .find(|sheet| sheet.old_name.as_deref() == Some("Summary"))
            .unwrap();
        assert!(summary.cell_changes.is_empty());
        assert_eq!(diff.summary.formulas_changed, 0);
        assert_eq!(diff.summary.sheets_renamed, 1);
    }

    #[test]
    fn cached_formula_value_policy_is_explicit() {
        let mut old = sheet("calc", &[&["id", "total"], &["A", "10"]]);
        old.rows[1][1].formula = Some("=1+1".to_string());
        let mut new = old.clone();
        new.rows[1][1].value = "11".to_string();

        let default_diff = diff_sheets(&old, &new);
        assert!(default_diff.cell_changes.is_empty());

        let options = DiffOptions {
            formula_values: FormulaValueComparison::CompareCached,
            ..DiffOptions::default()
        };
        let strict_diff = super::diff_sheets(&old, &new, &options, None).unwrap();
        assert_eq!(strict_diff.cell_changes.len(), 1);
        assert!(matches!(
            strict_diff.cell_changes[0].kind,
            CellChangeKind::ValueChanged
        ));
    }

    #[test]
    fn dense_full_sheet_region_accepts_formula_only_cells_but_not_gaps() {
        let mut dense = vec![typed_row(&["id", "value"]), typed_row(&["A", ""])];
        dense[1][1].formula = Some("=1+1".to_string());

        let region = dense_full_sheet_region(&dense).expect("formula cell keeps sheet dense");
        assert_eq!(
            (
                region.start_row,
                region.start_col,
                region.end_row,
                region.end_col
            ),
            (0, 0, 1, 1)
        );

        dense[1][1].formula = None;
        assert!(dense_full_sheet_region(&dense).is_none());
        assert!(dense_full_sheet_region(&[typed_row(&["A", "B"]), typed_row(&["C"])]).is_none());
    }

    #[test]
    fn distant_note_does_not_bridge_vertically_separated_tables() {
        let mut rows = vec![
            typed_row(&["id", "value"]),
            typed_row(&["A", "10"]),
            vec![CellIr::default(); 26],
            typed_row(&["id", "value"]),
            typed_row(&["B", "20"]),
        ];
        rows[2][25] = CellIr {
            value: "note".to_string(),
            value_kind: CellValueKind::String,
            formula: None,
            style_signature: String::new(),
        };
        let sheet = SheetIr {
            name: "regions".to_string(),
            start_row: 1,
            start_col: 1,
            rows,
            merged_ranges: Vec::new(),
        };

        let regions = detect_table_regions(&sheet.rows);

        assert_eq!(regions.len(), 3);
        assert_eq!(
            absolute_range(&sheet, &regions[0]),
            CellRange {
                start_row: 1,
                start_col: 1,
                end_row: 2,
                end_col: 2,
            }
        );
        assert_eq!(
            absolute_range(&sheet, &regions[1]),
            CellRange {
                start_row: 3,
                start_col: 26,
                end_row: 3,
                end_col: 26,
            }
        );
        assert_eq!(
            absolute_range(&sheet, &regions[2]),
            CellRange {
                start_row: 4,
                start_col: 1,
                end_row: 5,
                end_col: 2,
            }
        );
    }

    #[test]
    fn merge_boundaries_follow_neighboring_structural_shifts() {
        let old_range = CellRange {
            start_row: 1,
            start_col: 1,
            end_row: 2,
            end_col: 1,
        };
        let shifted_range = CellRange {
            start_row: 2,
            start_col: 1,
            end_row: 3,
            end_col: 1,
        };
        let mapper = CoordinateMapper {
            rows: BTreeMap::from([(1, 2)]),
            columns: BTreeMap::from([(1, 1)]),
        };
        assert!(diff_merge_ranges(&[old_range], &[shifted_range], &mapper).is_empty());

        let mapper = CoordinateMapper {
            rows: BTreeMap::from([(1, 1), (3, 4)]),
            columns: BTreeMap::from([(1, 1)]),
        };
        let expanded = CellRange {
            start_row: 1,
            start_col: 1,
            end_row: 3,
            end_col: 1,
        };
        assert!(
            diff_merge_ranges(
                &[CellRange {
                    start_row: 1,
                    start_col: 1,
                    end_row: 2,
                    end_col: 1,
                }],
                &[expanded],
                &mapper,
            )
            .is_empty()
        );
    }

    #[test]
    fn serialized_diff_is_deterministic() {
        let old = workbook(&[sheet("data", &[&["id", "value"], &["A", "10"]])]);
        let new = workbook(&[sheet(
            "data",
            &[&["id", "extra", "value"], &["A", "x", "11"]],
        )]);
        let render = || {
            serde_json::to_string(&diff_workbooks(
                Path::new("old.xlsx"),
                Path::new("new.xlsx"),
                &old,
                &new,
            ))
            .unwrap()
        };

        assert_eq!(render(), render());
    }

    #[test]
    fn office_theme_tint_equals_its_rendered_rgb() {
        let theme = XlsxTheme {
            colors: BTreeMap::from([(4, "FF4F81BD".to_string())]),
            ..Default::default()
        };
        let themed = OoxmlNode {
            name: "fgColor".to_string(),
            attrs: vec![
                ("theme".to_string(), "4".to_string()),
                ("tint".to_string(), "0.3999755851924192".to_string()),
            ],
            ..Default::default()
        };
        let direct = OoxmlNode {
            name: "fgColor".to_string(),
            attrs: vec![("rgb".to_string(), "FF95B3D7".to_string())],
            ..Default::default()
        };

        assert_eq!(
            canonical_ooxml_signature_with_theme(&themed, Some(&theme)),
            canonical_ooxml_signature_with_theme(&direct, Some(&theme)),
        );
    }

    #[test]
    fn custom_indexed_palette_equals_its_direct_color() {
        let indexed = br#"<styleSheet><fills><fill><patternFill patternType="solid"><fgColor indexed="0"/></patternFill></fill></fills><cellXfs><xf fillId="0"/></cellXfs><colors><indexedColors><rgbColor rgb="00123456"/></indexedColors></colors></styleSheet>"#;
        let direct = br#"<styleSheet><fills><fill><patternFill patternType="solid"><fgColor rgb="FF123456"/></patternFill></fill></fills><cellXfs><xf fillId="0"/></cellXfs></styleSheet>"#;

        assert_eq!(
            parse_cell_styles(indexed, None).unwrap(),
            parse_cell_styles(direct, None).unwrap(),
        );
    }

    proptest! {
        #[test]
        fn cached_row_scores_match_uncached_reference(
            old in prop::collection::vec((
                prop::sample::select(vec!["", "x", "日本語", "a\tb", "e\u{301}"]),
                prop::sample::select(vec![CellValueKind::Empty, CellValueKind::String, CellValueKind::Number, CellValueKind::Boolean, CellValueKind::DateTime]),
                prop::option::of(prop::sample::select(vec!["=A1+1", "=SUM(B2:C2)", ""])),
            ), 0..12),
            new in prop::collection::vec((
                prop::sample::select(vec!["", "x", "日本語", "a\tb", "e\u{301}"]),
                prop::sample::select(vec![CellValueKind::Empty, CellValueKind::String, CellValueKind::Number, CellValueKind::Boolean, CellValueKind::DateTime]),
                prop::option::of(prop::sample::select(vec!["=A1+1", "=SUM(B2:C2)", ""])),
            ), 0..12),
        ) {
            let cells = |row: Vec<(&str, CellValueKind, Option<&str>)>| row.into_iter().map(|(value, value_kind, formula)| CellIr {
                value: value.to_string(), value_kind, formula: formula.map(str::to_string), style_signature: String::new(),
            }).collect::<Vec<_>>();
            let old = cells(old);
            let new = cells(new);
            let before = RowProfile::new(&old);
            let after = RowProfile::new(&new);
            let expected = row_similarity(&old, &new);
            prop_assert_eq!(before.similarity(&after), expected);
            // A second lookup exercises cached features.
            prop_assert_eq!(before.similarity(&after), expected);
            prop_assert_eq!(after.similarity(&before), row_similarity(&new, &old));
        }

        #[test]
        fn bag_similarity_preserves_multiset_overlap(
            old in prop::collection::vec(prop::sample::select(vec!["", "a", "b", "日本語"]), 0..40),
            new in prop::collection::vec(prop::sample::select(vec!["", "a", "b", "日本語"]), 0..40),
        ) {
            let mut intersection = 0usize;
            let mut union = 0usize;
            for value in ["a", "b", "日本語"] {
                let old_count = old.iter().filter(|&&item| item == value).count();
                let new_count = new.iter().filter(|&&item| item == value).count();
                intersection += old_count.min(new_count);
                union += old_count.max(new_count);
            }
            let expected = if union == 0 { 1.0 } else { intersection as f64 / union as f64 };
            prop_assert_eq!(bag_similarity(&old, &new), expected);
        }

        #[test]
        fn tinted_theme_grays_equal_their_rendered_rgb(
            channel in any::<u8>(),
            tint_hundredths in -100i16..=100,
        ) {
            let tint = f64::from(tint_hundredths) / 100.0;
            let rendered = if tint < 0.0 {
                f64::from(channel) * (1.0 + tint)
            } else {
                f64::from(channel) * (1.0 - tint) + 255.0 * tint
            };
            let rendered = (rendered + 1e-9).round() as u8;
            let theme = XlsxTheme {
                colors: BTreeMap::from([(4, format!("FF{channel:02X}{channel:02X}{channel:02X}"))]),
                ..Default::default()
            };
            let themed = OoxmlNode {
                name: "fgColor".to_string(),
                attrs: vec![
                    ("theme".to_string(), "4".to_string()),
                    ("tint".to_string(), tint.to_string()),
                ],
                ..Default::default()
            };
            let direct = OoxmlNode {
                name: "fgColor".to_string(),
                attrs: vec![(
                    "rgb".to_string(),
                    format!("FF{rendered:02X}{rendered:02X}{rendered:02X}"),
                )],
                ..Default::default()
            };

            prop_assert_eq!(
                canonical_ooxml_signature_with_theme(&themed, Some(&theme)),
                canonical_ooxml_signature_with_theme(&direct, Some(&theme)),
            );
        }

        #[test]
        fn equivalent_font_size_number_spellings_have_one_signature(
            size_hundredths in 1u16..=40_900,
        ) {
            let decimal = format!("{:.2}", f64::from(size_hundredths) / 100.0);
            let scientific = format!("{size_hundredths}e-2");
            let with_decimal = OoxmlNode {
                name: "sz".to_string(),
                attrs: vec![("val".to_string(), decimal)],
                ..Default::default()
            };
            let with_scientific = OoxmlNode {
                name: "sz".to_string(),
                attrs: vec![("val".to_string(), scientific)],
                ..Default::default()
            };

            prop_assert_eq!(
                canonical_ooxml_signature(&with_decimal),
                canonical_ooxml_signature(&with_scientific),
            );
        }

        #[test]
        fn equivalent_spreadsheet_font_boolean_spellings_have_one_signature(
            property in prop::sample::select(vec![
                "b", "i", "strike", "outline", "shadow", "condense", "extend",
            ]),
            enabled in any::<bool>(),
            numeric_spelling in any::<bool>(),
        ) {
            let implicit_xml = if enabled {
                format!("<font><{property}/></font>")
            } else {
                "<font/>".to_string()
            };
            let explicit_value = match (enabled, numeric_spelling) {
                (true, true) => "1",
                (true, false) => "true",
                (false, true) => "0",
                (false, false) => "false",
            };
            let explicit_xml =
                format!(r#"<font><{property} val="{explicit_value}"/></font>"#);
            let implicit = parse_ooxml(implicit_xml.as_bytes()).unwrap();
            let explicit = parse_ooxml(explicit_xml.as_bytes()).unwrap();
            let opposite_xml = if enabled {
                "<font/>".to_string()
            } else {
                format!("<font><{property}/></font>")
            };
            let opposite = parse_ooxml(opposite_xml.as_bytes()).unwrap();

            prop_assert_eq!(
                canonical_ooxml_signature(&implicit),
                canonical_ooxml_signature(&explicit),
            );
            prop_assert_ne!(
                canonical_ooxml_signature(&implicit),
                canonical_ooxml_signature(&opposite),
            );
        }

        #[test]
        fn equivalent_cell_style_boolean_spellings_have_one_signature(
            property in prop::sample::select(vec![
                ("alignment", "wrapText", false),
                ("alignment", "shrinkToFit", false),
                ("alignment", "justifyLastLine", false),
                ("alignment", "mergeCell", false),
                ("protection", "locked", true),
                ("protection", "hidden", false),
            ]),
            enabled in any::<bool>(),
        ) {
            let (container, attribute, default) = property;
            let numeric = if enabled { "1" } else { "0" };
            let word = if enabled { "true" } else { "false" };
            let numeric_node = parse_ooxml(
                format!(r#"<{container} {attribute}="{numeric}"/>"#).as_bytes(),
            )
            .unwrap();
            let word_node = parse_ooxml(
                format!(r#"<{container} {attribute}="{word}"/>"#).as_bytes(),
            )
            .unwrap();
            let implicit_node = if enabled == default {
                parse_ooxml(format!("<{container}/>").as_bytes()).unwrap()
            } else {
                numeric_node.clone()
            };
            let opposite = if enabled { "0" } else { "1" };
            let opposite_node = parse_ooxml(
                format!(r#"<{container} {attribute}="{opposite}"/>"#).as_bytes(),
            )
            .unwrap();

            prop_assert_eq!(
                canonical_cell_style_property(container, &numeric_node),
                canonical_cell_style_property(container, &word_node),
            );
            prop_assert_eq!(
                canonical_cell_style_property(container, &numeric_node),
                canonical_cell_style_property(container, &implicit_node),
            );
            prop_assert_ne!(
                canonical_cell_style_property(container, &numeric_node),
                canonical_cell_style_property(container, &opposite_node),
            );
        }

        #[test]
        fn equivalent_tint_number_spellings_have_one_signature(
            tint_hundredths in -100i16..=100,
        ) {
            let decimal = format!("{:.2}", f64::from(tint_hundredths) / 100.0);
            let scientific = format!("{tint_hundredths}e-2");
            let with_decimal = OoxmlNode {
                name: "fgColor".to_string(),
                attrs: vec![
                    ("rgb".to_string(), "FF4F81BD".to_string()),
                    ("tint".to_string(), decimal),
                ],
                ..Default::default()
            };
            let with_scientific = OoxmlNode {
                name: "fgColor".to_string(),
                attrs: vec![
                    ("rgb".to_string(), "FF4F81BD".to_string()),
                    ("tint".to_string(), scientific),
                ],
                ..Default::default()
            };

            prop_assert_eq!(
                canonical_ooxml_signature(&with_decimal),
                canonical_ooxml_signature(&with_scientific),
            );
        }

        #[test]
        fn equivalent_argb_encodings_have_one_signature(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
            lowercase in any::<bool>(),
        ) {
            let rgb = format!("{red:02X}{green:02X}{blue:02X}");
            let encoded = if lowercase { rgb.to_ascii_lowercase() } else { rgb.clone() };
            prop_assert_eq!(normalized_argb(&encoded), normalized_argb(&format!("FF{rgb}")));
        }

        #[test]
        fn xlsx_color_alpha_byte_does_not_change_appearance(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
            alpha in any::<u8>(),
            tint_hundredths in -100i16..=100,
        ) {
            let rgb = format!("{red:02X}{green:02X}{blue:02X}");
            let tint = (f64::from(tint_hundredths) / 100.0).to_string();
            let with_alpha = OoxmlNode {
                name: "fgColor".to_string(),
                attrs: vec![
                    ("rgb".to_string(), format!("{alpha:02X}{rgb}")),
                    ("tint".to_string(), tint.clone()),
                ],
                ..Default::default()
            };
            let opaque = OoxmlNode {
                name: "fgColor".to_string(),
                attrs: vec![
                    ("rgb".to_string(), format!("FF{rgb}")),
                    ("tint".to_string(), tint),
                ],
                ..Default::default()
            };

            prop_assert_eq!(
                canonical_ooxml_signature(&with_alpha),
                canonical_ooxml_signature(&opaque),
            );
        }

        #[test]
        fn theme_and_direct_xlsx_colors_are_equivalent_for_any_rgb(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
        ) {
            let rgb = format!("{red:02X}{green:02X}{blue:02X}");
            let theme = XlsxTheme {
                colors: BTreeMap::from([(4, format!("FF{rgb}"))]),
                ..Default::default()
            };
            let themed = OoxmlNode {
                name: "fgColor".to_string(),
                attrs: vec![("theme".to_string(), "4".to_string())],
                ..Default::default()
            };
            let direct = OoxmlNode {
                name: "fgColor".to_string(),
                attrs: vec![("rgb".to_string(), rgb.to_ascii_lowercase())],
                ..Default::default()
            };

            prop_assert_eq!(
                canonical_ooxml_signature_with_theme(&themed, Some(&theme)),
                canonical_ooxml_signature_with_theme(&direct, Some(&theme)),
            );
        }

        #[test]
        fn indexed_and_direct_xlsx_colors_are_equivalent(
            (index, rgb) in prop::sample::select(INDEXED_COLOR_CASES.to_vec()),
            tint_hundredths in -100i16..=100,
            color_element in 0usize..3,
        ) {
            let name = ["color", "fgColor", "bgColor"][color_element];
            let tint = f64::from(tint_hundredths) / 100.0;
            let indexed = OoxmlNode {
                name: name.to_string(),
                attrs: vec![
                    ("indexed".to_string(), index.to_string()),
                    ("tint".to_string(), tint.to_string()),
                ],
                ..Default::default()
            };
            let direct = OoxmlNode {
                name: name.to_string(),
                attrs: vec![
                    ("rgb".to_string(), format!("FF{rgb}")),
                    ("tint".to_string(), tint.to_string()),
                ],
                ..Default::default()
            };

            prop_assert_eq!(
                canonical_ooxml_signature_with_theme(&indexed, None),
                canonical_ooxml_signature_with_theme(&direct, None),
            );
        }

        #[test]
        fn lexical_zero_alignment_values_are_defaults(
            zero_digits in 1usize..5,
            sign in 0u8..3,
        ) {
            let sign = match sign {
                1 => "+",
                2 => "-",
                _ => "",
            };
            let zero = format!("{sign}{}", "0".repeat(zero_digits));
            let alignment = OoxmlNode {
                name: "alignment".to_string(),
                attrs: vec![
                    ("textRotation".to_string(), zero.clone()),
                    ("indent".to_string(), zero.clone()),
                    ("relativeIndent".to_string(), zero.clone()),
                    ("readingOrder".to_string(), zero),
                ],
                ..Default::default()
            };

            prop_assert_eq!(canonical_cell_style_property("alignment", &alignment), None);
        }

        #[test]
        fn equivalent_alignment_integer_spellings_have_one_signature(
            attribute in prop::sample::select(vec![
                "textRotation", "indent", "relativeIndent", "readingOrder",
            ]),
            magnitude in 1i16..=180,
            negative in any::<bool>(),
            leading_zeros in 1usize..5,
            explicit_plus in any::<bool>(),
        ) {
            let value = if attribute == "relativeIndent" && negative {
                -magnitude
            } else {
                magnitude
            };
            let sign = if value < 0 {
                "-"
            } else if explicit_plus {
                "+"
            } else {
                ""
            };
            let padded = format!(
                "{sign}{}{magnitude}",
                "0".repeat(leading_zeros),
            );
            let canonical = OoxmlNode {
                name: "alignment".to_string(),
                attrs: vec![(attribute.to_string(), value.to_string())],
                ..Default::default()
            };
            let alternate = OoxmlNode {
                name: "alignment".to_string(),
                attrs: vec![(attribute.to_string(), padded)],
                ..Default::default()
            };
            let changed = OoxmlNode {
                name: "alignment".to_string(),
                attrs: vec![(attribute.to_string(), (value + 1).to_string())],
                ..Default::default()
            };

            prop_assert_eq!(
                canonical_cell_style_property("alignment", &canonical),
                canonical_cell_style_property("alignment", &alternate),
            );
            prop_assert_ne!(
                canonical_cell_style_property("alignment", &canonical),
                canonical_cell_style_property("alignment", &changed),
            );
        }

        #[test]
        fn identical_generated_sheets_are_reflexive(
            row_count in 1usize..16,
            column_count in 1usize..10,
            start_row in 1usize..32,
            start_col in 1usize..16,
        ) {
            let rows = (0..row_count)
                .map(|row| {
                    (0..column_count)
                        .map(|column| CellIr {
                            value: format!("R{row}-C{column}"),
                            value_kind: match (row + column) % 4 {
                                0 => CellValueKind::String,
                                1 => CellValueKind::Number,
                                2 => CellValueKind::Boolean,
                                _ => CellValueKind::DateTime,
                            },
                            formula: ((row + column) % 7 == 0)
                                .then(|| format!("=A{}", row + start_row)),
                            style_signature: String::new(),
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let sheet = SheetIr {
                name: "generated".to_string(),
                start_row,
                start_col,
                rows,
                merged_ranges: Vec::new(),
            };

            let diff = diff_sheets(&sheet, &sheet);

            prop_assert!(matches!(diff.status, SheetStatus::Unchanged));
            prop_assert!(diff.row_changes.is_empty());
            prop_assert!(diff.column_changes.is_empty());
            prop_assert!(diff.cell_changes.is_empty());
        }

        #[test]
        fn shifting_sheet_origin_reports_blank_structure_without_cell_noise(
            row_count in 2usize..10,
            column_count in 2usize..8,
            start_row in 1usize..24,
            start_col in 1usize..12,
            row_shift in 1usize..6,
            column_shift in 1usize..6,
            target_row in 0usize..10,
            target_column in 0usize..8,
        ) {
            let target_row = target_row.min(row_count - 1);
            let target_column = target_column.min(column_count - 1);
            let formula_row = row_count - 1;
            let formula_column = column_count - 1;
            let mut old_rows = (0..row_count)
                .map(|row| {
                    (0..column_count)
                        .map(|column| CellIr {
                            value: format!("R{row}-C{column}"),
                            value_kind: CellValueKind::String,
                            formula: None,
                            style_signature: String::new(),
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            old_rows[formula_row][formula_column].formula = Some(format!(
                "=${}${}",
                column_name(start_col + target_column - 1),
                start_row + target_row
            ));
            let mut new_rows = old_rows.clone();
            new_rows[formula_row][formula_column].formula = Some(format!(
                "=${}${}",
                column_name(start_col + column_shift + target_column - 1),
                start_row + row_shift + target_row
            ));
            let old = SheetIr {
                name: "shifted-origin".to_string(),
                start_row,
                start_col,
                rows: old_rows,
                merged_ranges: vec![CellRange {
                    start_row,
                    start_col,
                    end_row: start_row,
                    end_col: start_col + 1,
                }],
            };
            let new = SheetIr {
                start_row: start_row + row_shift,
                start_col: start_col + column_shift,
                rows: new_rows,
                merged_ranges: vec![CellRange {
                    start_row: start_row + row_shift,
                    start_col: start_col + column_shift,
                    end_row: start_row + row_shift,
                    end_col: start_col + column_shift + 1,
                }],
                ..old.clone()
            };

            let forward = diff_sheets(&old, &new);
            let reverse = diff_sheets(&new, &old);

            let forward_added_rows = forward.row_changes.iter().filter_map(|change| {
                matches!(change.kind, ChangeKind::Added).then_some(change.new_index.unwrap())
            }).collect::<Vec<_>>();
            let forward_added_columns = forward.column_changes.iter().filter_map(|change| {
                matches!(change.kind, ChangeKind::Added).then_some(change.new_index.unwrap())
            }).collect::<Vec<_>>();
            prop_assert_eq!(forward_added_rows, (start_row..start_row + row_shift).collect::<Vec<_>>());
            prop_assert_eq!(forward_added_columns, (start_col..start_col + column_shift).collect::<Vec<_>>());
            prop_assert!(forward.cell_changes.is_empty());
            prop_assert!(forward.merge_changes.is_empty());

            let reverse_removed_rows = reverse.row_changes.iter().filter_map(|change| {
                matches!(change.kind, ChangeKind::Removed).then_some(change.old_index.unwrap())
            }).collect::<Vec<_>>();
            let reverse_removed_columns = reverse.column_changes.iter().filter_map(|change| {
                matches!(change.kind, ChangeKind::Removed).then_some(change.old_index.unwrap())
            }).collect::<Vec<_>>();
            prop_assert_eq!(reverse_removed_rows, (start_row..start_row + row_shift).collect::<Vec<_>>());
            prop_assert_eq!(reverse_removed_columns, (start_col..start_col + column_shift).collect::<Vec<_>>());
            prop_assert!(reverse.cell_changes.is_empty());
            prop_assert!(reverse.merge_changes.is_empty());
        }

        #[test]
        fn missing_and_explicit_trailing_empty_cells_are_equivalent(
            row_count in 2usize..16,
            column_count in 2usize..10,
            sparse_row in 0usize..16,
            retained_columns in 1usize..10,
        ) {
            let sparse_row = sparse_row.min(row_count - 1);
            let retained_columns = retained_columns.min(column_count - 1);
            let mut ragged_rows = (0..row_count)
                .map(|row| {
                    (0..column_count)
                        .map(|column| CellIr {
                            value: format!("R{row}-C{column}"),
                            value_kind: CellValueKind::String,
                            formula: None,
                            style_signature: String::new(),
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            ragged_rows[sparse_row].truncate(retained_columns);
            let mut explicit_rows = ragged_rows.clone();
            explicit_rows[sparse_row].resize(column_count, CellIr::default());
            let ragged = SheetIr {
                name: "ragged".to_string(),
                start_row: 1,
                start_col: 1,
                rows: ragged_rows,
                merged_ranges: Vec::new(),
            };
            let explicit = SheetIr {
                rows: explicit_rows,
                ..ragged.clone()
            };

            let forward = diff_sheets(&ragged, &explicit);
            let reverse = diff_sheets(&explicit, &ragged);

            prop_assert!(matches!(forward.status, SheetStatus::Unchanged));
            prop_assert!(forward.row_changes.is_empty());
            prop_assert!(forward.column_changes.is_empty());
            prop_assert!(forward.cell_changes.is_empty());
            prop_assert!(matches!(reverse.status, SheetStatus::Unchanged));
            prop_assert!(reverse.row_changes.is_empty());
            prop_assert!(reverse.column_changes.is_empty());
            prop_assert!(reverse.cell_changes.is_empty());
        }

        #[test]
        fn inserting_one_unique_row_has_no_following_cell_noise(
            row_count in 2usize..30,
            insertion in 0usize..30,
        ) {
            let insertion = insertion.min(row_count);
            let old_rows: Vec<Vec<CellIr>> = (0..row_count)
                .map(|idx| typed_row(&[&format!("ID-{idx}"), &idx.to_string()]))
                .collect();
            let mut new_rows = old_rows.clone();
            new_rows.insert(insertion, typed_row(&["NEW", "999"]));
            let old = SheetIr {
                name: "data".to_string(),
                start_row: 1,
                start_col: 1,
                rows: old_rows,
                merged_ranges: Vec::new(),
            };
            let new = SheetIr {
                rows: new_rows,
                ..old.clone()
            };

            let diff = diff_sheets(&old, &new);

            prop_assert_eq!(
                diff.row_changes.iter().filter(|change| matches!(change.kind, ChangeKind::Added)).count(),
                1
            );
            prop_assert!(diff.cell_changes.is_empty());
        }

        #[test]
        fn inserting_into_duplicate_rows_reports_only_the_inserted_row(
            duplicate_count in 2usize..64,
            insertion in 0usize..64,
            column_count in 1usize..8,
        ) {
            let duplicate = (0..column_count)
                .map(|column| CellIr {
                    value: format!("duplicate-{column}"),
                    value_kind: CellValueKind::String,
                    formula: None,
                    style_signature: String::new(),
                })
                .collect::<Vec<_>>();
            let old_rows = vec![duplicate; duplicate_count];
            let mut new_rows = old_rows.clone();
            let insertion = insertion.min(duplicate_count);
            new_rows.insert(
                insertion,
                (0..column_count)
                    .map(|column| CellIr {
                        value: format!("inserted-{column}"),
                        value_kind: CellValueKind::String,
                        formula: None,
                        style_signature: String::new(),
                    })
                    .collect(),
            );
            let old = SheetIr {
                name: "duplicates".to_string(),
                start_row: 1,
                start_col: 1,
                rows: old_rows,
                merged_ranges: Vec::new(),
            };
            let new = SheetIr {
                rows: new_rows,
                ..old.clone()
            };

            let diff = diff_sheets(&old, &new);

            prop_assert_eq!(diff.row_changes.len(), 1);
            prop_assert!(matches!(diff.row_changes[0].kind, ChangeKind::Added));
            prop_assert_eq!(diff.row_changes[0].new_index, Some(insertion + 1));
            prop_assert!(diff.column_changes.is_empty());
            prop_assert!(diff.cell_changes.is_empty());
        }

        #[test]
        fn compound_unique_row_and_column_insertions_have_no_cell_noise(
            row_count in 3usize..20,
            first_insertion in 0usize..20,
            second_insertion in 0usize..20,
            column_insertion in 0usize..4,
        ) {
            let old_rows: Vec<Vec<CellIr>> = (0..row_count)
                .map(|idx| typed_row(&[
                    &format!("ID-{idx}"),
                    &format!("name-{idx}"),
                    &format!("value-{idx}"),
                ]))
                .collect();
            let mut new_rows = old_rows.clone();
            let first = first_insertion.min(new_rows.len());
            new_rows.insert(first, typed_row(&["NEW-1", "new-one", "101"]));
            let second = second_insertion.min(new_rows.len());
            new_rows.insert(second, typed_row(&["NEW-2", "new-two", "102"]));
            let column = column_insertion.min(3);
            for row in &mut new_rows {
                row.insert(column, CellIr {
                    value: "inserted".to_string(),
                    value_kind: CellValueKind::String,
                    formula: None,
                    style_signature: String::new(),
                });
            }
            let old = SheetIr {
                name: "compound".to_string(),
                start_row: 1,
                start_col: 1,
                rows: old_rows,
                merged_ranges: Vec::new(),
            };
            let new = SheetIr {
                rows: new_rows,
                ..old.clone()
            };

            let diff = diff_sheets(&old, &new);

            prop_assert_eq!(
                diff.row_changes.iter().filter(|change| matches!(change.kind, ChangeKind::Added)).count(),
                2
            );
            prop_assert_eq!(
                diff.column_changes.iter().filter(|change| matches!(change.kind, ChangeKind::Added)).count(),
                1
            );
            prop_assert!(diff.cell_changes.is_empty());
        }

        #[test]
        fn structural_insertions_are_symmetric_and_shift_formulas(
            row_count in 2usize..10,
            column_count in 2usize..8,
            row_insertion in 0usize..10,
            column_insertion in 0usize..8,
            target_row in 0usize..10,
            target_column in 0usize..8,
        ) {
            let row_insertion = row_insertion.min(row_count);
            let column_insertion = column_insertion.min(column_count);
            let target_row = target_row.min(row_count - 1);
            let target_column = target_column.min(column_count - 1);
            let formula_row = row_count - 1;
            let formula_column = column_count - 1;
            let mut old_rows = (0..row_count)
                .map(|row| {
                    (0..column_count)
                        .map(|column| CellIr {
                            value: format!("R{row}-C{column}"),
                            value_kind: CellValueKind::String,
                            formula: None,
                            style_signature: String::new(),
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            old_rows[formula_row][formula_column].formula = Some(format!(
                "=${}${}",
                column_name(target_column),
                target_row + 1
            ));

            let mut new_rows = old_rows.clone();
            new_rows.insert(
                row_insertion,
                (0..column_count)
                    .map(|column| CellIr {
                        value: format!("inserted-row-C{column}"),
                        value_kind: CellValueKind::String,
                        formula: None,
                        style_signature: String::new(),
                    })
                    .collect(),
            );
            for (row, cells) in new_rows.iter_mut().enumerate() {
                cells.insert(
                    column_insertion,
                    CellIr {
                        value: format!("inserted-column-R{row}"),
                        value_kind: CellValueKind::String,
                        formula: None,
                        style_signature: String::new(),
                    },
                );
            }
            let shifted_formula_row = formula_row + usize::from(row_insertion <= formula_row);
            let shifted_formula_column =
                formula_column + usize::from(column_insertion <= formula_column);
            let shifted_target_row = target_row + usize::from(row_insertion <= target_row);
            let shifted_target_column =
                target_column + usize::from(column_insertion <= target_column);
            new_rows[shifted_formula_row][shifted_formula_column].formula = Some(format!(
                "=${}${}",
                column_name(shifted_target_column),
                shifted_target_row + 1
            ));

            let old = SheetIr {
                name: "formula-shift".to_string(),
                start_row: 1,
                start_col: 1,
                rows: old_rows,
                merged_ranges: Vec::new(),
            };
            let new = SheetIr {
                rows: new_rows,
                ..old.clone()
            };

            let forward = diff_sheets(&old, &new);
            let reverse = diff_sheets(&new, &old);

            prop_assert_eq!(forward.row_changes.len(), 1);
            prop_assert!(matches!(forward.row_changes[0].kind, ChangeKind::Added));
            prop_assert_eq!(forward.column_changes.len(), 1);
            prop_assert!(matches!(forward.column_changes[0].kind, ChangeKind::Added));
            prop_assert!(forward.cell_changes.is_empty());

            prop_assert_eq!(reverse.row_changes.len(), 1);
            prop_assert!(matches!(reverse.row_changes[0].kind, ChangeKind::Removed));
            prop_assert_eq!(reverse.column_changes.len(), 1);
            prop_assert!(matches!(reverse.column_changes[0].kind, ChangeKind::Removed));
            prop_assert!(reverse.cell_changes.is_empty());
        }

        #[test]
        fn generated_row_and_column_rotations_report_only_moves(
            row_count in 2usize..14,
            column_count in 2usize..10,
            row_rotation in 0usize..14,
            column_rotation in 0usize..10,
        ) {
            let row_rotation = 1 + row_rotation % (row_count - 1);
            let column_rotation = 1 + column_rotation % (column_count - 1);
            let old_rows = (0..row_count)
                .map(|row| {
                    (0..column_count)
                        .map(|column| CellIr {
                            value: format!("R{row}-C{column}"),
                            value_kind: CellValueKind::String,
                            formula: None,
                            style_signature: String::new(),
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let mut new_rows = old_rows.clone();
            new_rows.rotate_left(row_rotation);
            for row in &mut new_rows {
                row.rotate_left(column_rotation);
            }
            let old = SheetIr {
                name: "rotations".to_string(),
                start_row: 1,
                start_col: 1,
                rows: old_rows,
                merged_ranges: Vec::new(),
            };
            let new = SheetIr {
                rows: new_rows,
                ..old.clone()
            };

            let forward = diff_sheets(&old, &new);
            let reverse = diff_sheets(&new, &old);

            prop_assert!(!forward.row_changes.is_empty());
            prop_assert!(!forward.column_changes.is_empty());
            prop_assert!(forward.row_changes.iter().all(|change| matches!(change.kind, ChangeKind::Moved)));
            prop_assert!(forward.column_changes.iter().all(|change| matches!(change.kind, ChangeKind::Moved)));
            prop_assert!(forward.cell_changes.is_empty());
            prop_assert!(reverse.row_changes.iter().all(|change| matches!(change.kind, ChangeKind::Moved)));
            prop_assert!(reverse.column_changes.iter().all(|change| matches!(change.kind, ChangeKind::Moved)));
            prop_assert!(reverse.cell_changes.is_empty());
        }

        #[test]
        fn formula_ranges_follow_generated_row_and_column_insertions(
            row_count in 2usize..20,
            column_count in 2usize..12,
            row_insertion in 1usize..21,
            column_insertion in 1usize..13,
            first_row in 1usize..21,
            first_column in 1usize..13,
            second_row in 1usize..21,
            second_column in 1usize..13,
        ) {
            let row_insertion = row_insertion.min(row_count + 1);
            let column_insertion = column_insertion.min(column_count + 1);
            let first_row = first_row.min(row_count);
            let first_column = first_column.min(column_count);
            let second_row = second_row.min(row_count);
            let second_column = second_column.min(column_count);
            let mapper = CoordinateMapper {
                rows: (1..=row_count)
                    .map(|row| (row, row + usize::from(row_insertion <= row)))
                    .collect(),
                columns: (1..=column_count)
                    .map(|column| {
                        (
                            column,
                            column + usize::from(column_insertion <= column),
                        )
                    })
                    .collect(),
            };
            let mapped_first_row = mapper.rows[&first_row];
            let mapped_first_column = mapper.columns[&first_column];
            let mapped_second_row = mapper.rows[&second_row];
            let mapped_second_column = mapper.columns[&second_column];
            let old_formula = format!(
                "=SUM(${}${}:{}{})",
                column_name(first_column - 1),
                first_row,
                column_name(second_column - 1),
                second_row
            );
            let new_formula = format!(
                "=SUM(${}${}:{}{})",
                column_name(mapped_first_column - 1),
                mapped_first_row,
                column_name(mapped_second_column - 1),
                mapped_second_row
            );

            prop_assert!(formulas_equivalent_after_structure_change(
                Some(&old_formula),
                Some(&new_formula),
                &mapper,
                None,
            ));
        }

        #[test]
        fn whole_row_and_column_formula_ranges_follow_generated_insertions(
            row_count in 2usize..64,
            column_count in 2usize..32,
            row_insertion in 1usize..65,
            column_insertion in 1usize..33,
            first_row in 1usize..65,
            second_row in 1usize..65,
            first_column in 1usize..33,
            second_column in 1usize..33,
        ) {
            let row_insertion = row_insertion.min(row_count + 1);
            let column_insertion = column_insertion.min(column_count + 1);
            let first_row = first_row.min(row_count);
            let second_row = second_row.min(row_count);
            let first_column = first_column.min(column_count);
            let second_column = second_column.min(column_count);
            let mapper = CoordinateMapper {
                rows: (1..=row_count)
                    .map(|row| (row, row + usize::from(row_insertion <= row)))
                    .collect(),
                columns: (1..=column_count)
                    .map(|column| {
                        (
                            column,
                            column + usize::from(column_insertion <= column),
                        )
                    })
                    .collect(),
            };
            let old_formula = format!(
                "=SUM(${}:{})+SUM(${}:{})",
                column_name(first_column - 1),
                column_name(second_column - 1),
                first_row,
                second_row
            );
            let new_formula = format!(
                "=SUM(${}:{})+SUM(${}:{})",
                column_name(mapper.columns[&first_column] - 1),
                column_name(mapper.columns[&second_column] - 1),
                mapper.rows[&first_row],
                mapper.rows[&second_row]
            );

            prop_assert!(formulas_equivalent_after_structure_change(
                Some(&old_formula),
                Some(&new_formula),
                &mapper,
                None,
            ));
        }

        #[test]
        fn sheet_qualified_formula_ranges_follow_target_sheet_insertions(
            row_count in 2usize..16,
            column_count in 2usize..10,
            row_insertion in 1usize..17,
            column_insertion in 1usize..11,
            first_row in 1usize..17,
            first_column in 1usize..11,
            second_row in 1usize..17,
            second_column in 1usize..11,
        ) {
            let row_insertion = row_insertion.min(row_count + 1);
            let column_insertion = column_insertion.min(column_count + 1);
            let first_row = first_row.min(row_count);
            let first_column = first_column.min(column_count);
            let second_row = second_row.min(row_count);
            let second_column = second_column.min(column_count);
            let target_mapper = CoordinateMapper {
                rows: (1..=row_count)
                    .map(|row| (row, row + usize::from(row_insertion <= row)))
                    .collect(),
                columns: (1..=column_count)
                    .map(|column| {
                        (
                            column,
                            column + usize::from(column_insertion <= column),
                        )
                    })
                    .collect(),
            };
            let mapped_first_row = target_mapper.rows[&first_row];
            let mapped_first_column = target_mapper.columns[&first_column];
            let mapped_second_row = target_mapper.rows[&second_row];
            let mapped_second_column = target_mapper.columns[&second_column];
            let workbook_mapper = WorkbookCoordinateMapper {
                sheets: HashMap::from([(
                    normalize_sheet_name("Data"),
                    SheetCoordinateMapping {
                        new_name: "Data 2026".to_string(),
                        mapper: target_mapper,
                    },
                )]),
            };
            let old_formula = format!(
                "=SUM(Data!${}${}:{}{})",
                column_name(first_column - 1),
                first_row,
                column_name(second_column - 1),
                second_row
            );
            let new_formula = format!(
                "=SUM('Data 2026'!${}${}:{}{})",
                column_name(mapped_first_column - 1),
                mapped_first_row,
                column_name(mapped_second_column - 1),
                mapped_second_row
            );

            prop_assert!(formulas_equivalent_after_structure_change(
                Some(&old_formula),
                Some(&new_formula),
                &CoordinateMapper::default(),
                Some(&workbook_mapper),
            ));
        }

        #[test]
        fn sheet_qualified_whole_axis_ranges_follow_target_sheet_insertions(
            row_count in 2usize..64,
            column_count in 2usize..32,
            row_insertion in 1usize..65,
            column_insertion in 1usize..33,
            first_row in 1usize..65,
            second_row in 1usize..65,
            first_column in 1usize..33,
            second_column in 1usize..33,
        ) {
            let row_insertion = row_insertion.min(row_count + 1);
            let column_insertion = column_insertion.min(column_count + 1);
            let first_row = first_row.min(row_count);
            let second_row = second_row.min(row_count);
            let first_column = first_column.min(column_count);
            let second_column = second_column.min(column_count);
            let target_mapper = CoordinateMapper {
                rows: (1..=row_count)
                    .map(|row| (row, row + usize::from(row_insertion <= row)))
                    .collect(),
                columns: (1..=column_count)
                    .map(|column| {
                        (
                            column,
                            column + usize::from(column_insertion <= column),
                        )
                    })
                    .collect(),
            };
            let workbook_mapper = WorkbookCoordinateMapper {
                sheets: HashMap::from([(
                    normalize_sheet_name("Data"),
                    SheetCoordinateMapping {
                        new_name: "Data 2026".to_string(),
                        mapper: target_mapper,
                    },
                )]),
            };
            let old_formula = format!(
                "=SUM(Data!${}:{})+SUM(Data!${}:{})",
                column_name(first_column - 1),
                column_name(second_column - 1),
                first_row,
                second_row
            );
            let target_mapper = &workbook_mapper.sheets[&normalize_sheet_name("Data")].mapper;
            let new_formula = format!(
                "=SUM('Data 2026'!${}:{})+SUM('Data 2026'!${}:{})",
                column_name(target_mapper.columns[&first_column] - 1),
                column_name(target_mapper.columns[&second_column] - 1),
                target_mapper.rows[&first_row],
                target_mapper.rows[&second_row]
            );

            prop_assert!(formulas_equivalent_after_structure_change(
                Some(&old_formula),
                Some(&new_formula),
                &CoordinateMapper::default(),
                Some(&workbook_mapper),
            ));
        }

        #[test]
        fn three_dimensional_references_follow_endpoint_sheet_renames(
            (old_first, old_last, new_first, new_last) in prop::sample::select(vec![
                ("Jan 2025", "Mar 2025", "Start 2026", "End 2026"),
                ("Bob's Jan", "Bob's Mar", "First's Data", "Last's Data"),
                ("北_1", "北_3", "開始", "終了"),
            ]),
            first_row in 1usize..64,
            second_row in 1usize..64,
            first_column in 1usize..32,
            second_column in 1usize..32,
            range_suffix in 0usize..128,
        ) {
            let quoted_range = |first: &str, last: &str| {
                format!(
                    "'{}:{}'",
                    first.replace('\'', "''"),
                    last.replace('\'', "''")
                )
            };
            let old_range = quoted_range(old_first, old_last);
            let new_range = quoted_range(new_first, new_last);
            let old_formula = format!(
                "=SUM({old_range}!${}${}:{}{})+SUM({old_range}!${}:{})+SUM({old_range}!${}:{})",
                column_name(first_column - 1),
                first_row,
                column_name(second_column - 1),
                second_row,
                column_name(first_column - 1),
                column_name(second_column - 1),
                first_row,
                second_row
            );
            let new_formula = format!(
                "=SUM({new_range}!${}${}:{}{})+SUM({new_range}!${}:{})+SUM({new_range}!${}:{})",
                column_name(first_column - 1),
                first_row,
                column_name(second_column - 1),
                second_row,
                column_name(first_column - 1),
                column_name(second_column - 1),
                first_row,
                second_row
            );
            let workbook_mapper = WorkbookCoordinateMapper {
                sheets: HashMap::from([
                    (
                        normalize_sheet_name(old_first),
                        SheetCoordinateMapping {
                            new_name: new_first.to_string(),
                            mapper: CoordinateMapper::default(),
                        },
                    ),
                    (
                        normalize_sheet_name(old_last),
                        SheetCoordinateMapping {
                            new_name: new_last.to_string(),
                            mapper: CoordinateMapper::default(),
                        },
                    ),
                ]),
            };
            let unrelated_local_mapper = CoordinateMapper {
                rows: BTreeMap::from([(first_row, first_row + 1), (second_row, second_row + 1)]),
                columns: BTreeMap::from([
                    (first_column, first_column + 1),
                    (second_column, second_column + 1),
                ]),
            };

            prop_assert!(formulas_equivalent_after_structure_change(
                Some(&old_formula),
                Some(&new_formula),
                &unrelated_local_mapper,
                Some(&workbook_mapper),
            ));

            let unquoted_old_first = format!("Sheet{range_suffix}A");
            let unquoted_old_last = format!("Sheet{range_suffix}C");
            let unquoted_new_first = format!("Start{range_suffix}");
            let unquoted_new_last = format!("End{range_suffix}");
            let unquoted_old = format!(
                "=SUM({unquoted_old_first}:{unquoted_old_last}!{}{})",
                column_name(first_column - 1),
                first_row
            );
            let unquoted_new = format!(
                "=SUM({unquoted_new_first}:{unquoted_new_last}!{}{})",
                column_name(first_column - 1),
                first_row
            );
            let unquoted_mapper = WorkbookCoordinateMapper {
                sheets: HashMap::from([
                    (
                        normalize_sheet_name(&unquoted_old_first),
                        SheetCoordinateMapping {
                            new_name: unquoted_new_first,
                            mapper: CoordinateMapper::default(),
                        },
                    ),
                    (
                        normalize_sheet_name(&unquoted_old_last),
                        SheetCoordinateMapping {
                            new_name: unquoted_new_last,
                            mapper: CoordinateMapper::default(),
                        },
                    ),
                ]),
            };
            prop_assert!(formulas_equivalent_after_structure_change(
                Some(&unquoted_old),
                Some(&unquoted_new),
                &unrelated_local_mapper,
                Some(&unquoted_mapper),
            ));

            let changed_endpoint = quoted_range(old_first, "Different End");
            prop_assert_ne!(
                canonical_formula(&format!("={old_range}!A1"), None),
                canonical_formula(&format!("={changed_endpoint}!A1"), None)
            );
        }

        #[test]
        fn insignificant_formula_whitespace_does_not_create_changes(
            row in 1usize..128,
            column in 1usize..64,
            whitespace_count in 1usize..6,
        ) {
            let whitespace = " ".repeat(whitespace_count);
            let reference = format!("{}{}", column_name(column - 1), row);
            let compact = format!("=SUM({reference},B2)+C3*2");
            let formatted = format!(
                "={whitespace}SUM({whitespace}{reference}{whitespace},{whitespace}B2{whitespace}){whitespace}+{whitespace}C3{whitespace}*{whitespace}2{whitespace}"
            );

            prop_assert_eq!(
                canonical_formula(&compact, None),
                canonical_formula(&formatted, None)
            );
            let intersection = format!("={reference}{whitespace}B2");
            prop_assert_eq!(
                canonical_formula(&intersection, None),
                canonical_formula(&format!("={reference} B2"), None)
            );
            prop_assert_ne!(
                canonical_formula(&intersection, None),
                canonical_formula(&format!("={reference}B2"), None)
            );
            prop_assert_ne!(
                canonical_formula(&format!("=\"left{whitespace}right\""), None),
                canonical_formula("=\"leftright\"", None)
            );
        }

        #[test]
        fn unicode_defined_names_are_case_insensitive_but_literals_are_not(
            (lower, upper) in prop::sample::select(vec![
                ("taux_été", "TAUX_ÉTÉ"),
                ("κόστος", "ΚΌΣΤΟΣ"),
                ("straße_total", "STRASSE_TOTAL"),
            ]),
            row in 1usize..128,
        ) {
            let lower_formula = format!("={lower}+A{row}");
            let upper_formula = format!("={upper}+A{row}");
            prop_assert_eq!(
                canonical_formula(&lower_formula, None),
                canonical_formula(&upper_formula, None)
            );
            prop_assert_ne!(
                canonical_formula(&format!("=\"{lower}\""), None),
                canonical_formula(&format!("=\"{upper}\""), None)
            );
        }

        #[test]
        fn structured_references_are_case_insensitive_but_external_books_are_not(
            (table_lower, table_upper, column_lower, column_upper) in prop::sample::select(vec![
                ("sales_été", "SALES_ÉTÉ", "amount", "AMOUNT"),
                ("κόστος", "ΚΌΣΤΟΣ", "φόρος", "ΦΌΡΟΣ"),
                ("strasse", "STRASSE", "tax_rate", "TAX_RATE"),
            ]),
            workbook_suffix in 0usize..128,
        ) {
            let lower_formula = format!(
                "=SUM({table_lower}[{column_lower}])+[@{column_lower}]+{table_lower}[[#headers],[{column_lower}]]"
            );
            let upper_formula = format!(
                "=SUM({table_upper}[{column_upper}])+[@{column_upper}]+{table_upper}[[#HEADERS],[{column_upper}]]"
            );
            prop_assert_eq!(
                canonical_formula(&lower_formula, None),
                canonical_formula(&upper_formula, None)
            );
            let mut lower_sheet = sheet("structured", &[&[""]]);
            lower_sheet.rows[0][0].formula = Some(lower_formula);
            let mut upper_sheet = lower_sheet.clone();
            upper_sheet.rows[0][0].formula = Some(upper_formula);
            let structured_diff = diff_sheets(&lower_sheet, &upper_sheet);
            prop_assert!(structured_diff.row_changes.is_empty());
            prop_assert!(structured_diff.column_changes.is_empty());
            prop_assert!(structured_diff.cell_changes.is_empty());

            let first_book = format!("=[Book{workbook_suffix}A.xlsx]Sheet1!A1");
            let second_book = format!("=[Book{workbook_suffix}B.xlsx]Sheet1!A1");
            prop_assert_ne!(
                canonical_formula(&first_book, None),
                canonical_formula(&second_book, None)
            );
            let mixed_case_book = format!("=A1 [Book{workbook_suffix}.xlsx]Sheet1!A1");
            let upper_case_book = format!("=A1 [BOOK{workbook_suffix}.XLSX]Sheet1!A1");
            prop_assert_ne!(
                canonical_formula(&mixed_case_book, None),
                canonical_formula(&upper_case_book, None)
            );
            let mut first_book_sheet = sheet("external", &[&[""]]);
            first_book_sheet.rows[0][0].formula = Some(first_book);
            let mut second_book_sheet = first_book_sheet.clone();
            second_book_sheet.rows[0][0].formula = Some(second_book);
            let external_diff = diff_sheets(&first_book_sheet, &second_book_sheet);
            prop_assert_eq!(external_diff.cell_changes.len(), 1);
            prop_assert!(matches!(
                external_diff.cell_changes[0].kind,
                CellChangeKind::FormulaChanged
            ));
            prop_assert_ne!(
                canonical_formula(&format!("=\"[{column_lower}]\""), None),
                canonical_formula(&format!("=\"[{column_upper}]\""), None)
            );
        }

        #[test]
        fn equivalent_numeric_representations_are_canonical(
            value in -1_000_000i64..1_000_000,
            negative_zero in any::<bool>(),
        ) {
            prop_assert_eq!(
                normalize_value(&Data::Int(value)),
                normalize_value(&Data::Float(value as f64))
            );
            let zero = if negative_zero { -0.0 } else { 0.0 };
            prop_assert_eq!(normalize_value(&Data::Float(zero)), "0");
        }

        #[test]
        fn equivalent_datetime_and_duration_storage_formats_are_canonical(
            day in 61u32..60_000,
            milliseconds in 0u32..86_400_000,
            is_1904 in any::<bool>(),
            duration_hours in 0u32..240,
            duration_minutes in 0u32..60,
            duration_seconds in 0u32..60,
            duration_milliseconds in 0u32..1_000,
        ) {
            let serial = day as f64 + milliseconds as f64 / 86_400_000.0;
            let excel_datetime = calamine::ExcelDateTime::new(
                serial,
                calamine::ExcelDateTimeType::DateTime,
                is_1904,
            );
            let (year, month, day, hour, minute, second, millisecond) =
                excel_datetime.to_ymd_hms_milli();
            let iso_datetime_text = format!(
                "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millisecond:03}"
            );
            let serial_datetime = Data::DateTime(excel_datetime);
            let iso_datetime = Data::DateTimeIso(iso_datetime_text);
            prop_assert_eq!(
                normalize_value(&serial_datetime),
                normalize_value(&iso_datetime)
            );

            let duration_total_milliseconds = ((duration_hours * 60 * 60
                + duration_minutes * 60
                + duration_seconds)
                * 1_000
                + duration_milliseconds) as f64;
            let excel_duration = calamine::ExcelDateTime::new(
                duration_total_milliseconds / 86_400_000.0,
                calamine::ExcelDateTimeType::TimeDelta,
                false,
            );
            let iso_duration_text = format!(
                "PT{duration_hours:02}H{duration_minutes:02}M{duration_seconds:02}.{duration_milliseconds:03}S"
            );
            let serial_duration = Data::DateTime(excel_duration);
            let iso_duration = Data::DurationIso(iso_duration_text);
            prop_assert_eq!(value_kind(&serial_duration), CellValueKind::Duration);
            prop_assert_eq!(
                normalize_value(&serial_duration),
                normalize_value(&iso_duration)
            );

            let normalized_cell = |value: &Data| CellIr {
                value: normalize_value(value),
                value_kind: value_kind(value),
                formula: None,
                style_signature: String::new(),
            };
            let serial_sheet = SheetIr {
                name: "temporal".to_string(),
                start_row: 1,
                start_col: 1,
                rows: vec![vec![
                    normalized_cell(&serial_datetime),
                    normalized_cell(&serial_duration),
                ]],
                merged_ranges: Vec::new(),
            };
            let iso_sheet = SheetIr {
                rows: vec![vec![
                    normalized_cell(&iso_datetime),
                    normalized_cell(&iso_duration),
                ]],
                ..serial_sheet.clone()
            };
            let diff = diff_sheets(&serial_sheet, &iso_sheet);
            prop_assert!(matches!(diff.status, SheetStatus::Unchanged));
            prop_assert!(diff.row_changes.is_empty());
            prop_assert!(diff.column_changes.is_empty());
            prop_assert!(diff.cell_changes.is_empty());
            prop_assert_eq!(
                normalize_value(&Data::DurationIso("PT1S1H".to_string())),
                "PT1S1H"
            );
            prop_assert_eq!(
                normalize_value(&Data::DurationIso("P1W1D".to_string())),
                "P1W1D"
            );
        }

        #[test]
        fn dynamic_array_references_shift_without_rewriting_error_literals(
            row in 1usize..128,
            column in 1usize..64,
            row_shift in 1usize..8,
            column_shift in 1usize..8,
        ) {
            let mapped_row = row + row_shift;
            let mapped_column = column + column_shift;
            let mapper = CoordinateMapper {
                rows: BTreeMap::from([(row, mapped_row)]),
                columns: BTreeMap::from([(column, mapped_column)]),
            };
            let old_formula = format!(
                "=IFERROR(${}${}#,#N/A)",
                column_name(column - 1),
                row
            );
            let new_formula = format!(
                "=IFERROR(${}${}#,#N/A)",
                column_name(mapped_column - 1),
                mapped_row
            );
            prop_assert!(formulas_equivalent_after_structure_change(
                Some(&old_formula),
                Some(&new_formula),
                &mapper,
                None,
            ));
            prop_assert_ne!(
                canonical_formula("=IFERROR(A1#,#N/A)", None),
                canonical_formula("=IFERROR(A1#,#REF!)", None)
            );
        }

        #[test]
        fn canonicalizing_arbitrary_formula_and_duration_text_never_panics(
            formula in ".{0,128}",
            duration in ".{0,128}",
        ) {
            let _ = canonical_formula(&formula, None);
            let _ = normalize_value(&Data::DurationIso(duration));
        }

        #[test]
        fn same_typed_value_edits_do_not_become_structural_changes(
            row_count in 1usize..20,
            column_count in 1usize..10,
            changed_row in 0usize..20,
            changed_column in 0usize..10,
        ) {
            let changed_row = changed_row.min(row_count - 1);
            let changed_column = changed_column.min(column_count - 1);
            let old_rows = (0..row_count)
                .map(|row| {
                    (0..column_count)
                        .map(|column| CellIr {
                            value: format!("R{row}-C{column}"),
                            value_kind: CellValueKind::String,
                            formula: None,
                            style_signature: String::new(),
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let mut new_rows = old_rows.clone();
            new_rows[changed_row][changed_column].value = "P3-CHANGED".to_string();
            let old = SheetIr {
                name: "typed-edit".to_string(),
                start_row: 1,
                start_col: 1,
                rows: old_rows,
                merged_ranges: Vec::new(),
            };
            let new = SheetIr {
                rows: new_rows,
                ..old.clone()
            };

            let diff = diff_sheets(&old, &new);

            prop_assert!(diff.row_changes.is_empty());
            prop_assert!(diff.column_changes.is_empty());
            prop_assert_eq!(diff.cell_changes.len(), 1);
            prop_assert!(matches!(diff.cell_changes[0].kind, CellChangeKind::ValueChanged));
        }

        #[test]
        fn typed_value_addition_and_removal_are_inverse(
            row_count in 3usize..14,
            column_count in 3usize..10,
            changed_row in 0usize..14,
            changed_column in 0usize..10,
            value_kind in 0u8..6,
        ) {
            let changed_row = changed_row.min(row_count - 1);
            let changed_column = changed_column.min(column_count - 1);
            let rows = (0..row_count)
                .map(|row| {
                    (0..column_count)
                        .map(|column| CellIr {
                            value: format!("R{row}-C{column}"),
                            value_kind: CellValueKind::String,
                            formula: None,
                            style_signature: String::new(),
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let mut old_rows = rows.clone();
            old_rows[changed_row][changed_column] = CellIr::default();
            let mut new_rows = old_rows.clone();
            new_rows[changed_row][changed_column] = CellIr {
                value: "PBT-ADDED".to_string(),
                value_kind: generated_value_kind(value_kind),
                formula: None,
                style_signature: String::new(),
            };
            let old = SheetIr {
                name: "typed-add-remove".to_string(),
                start_row: 1,
                start_col: 1,
                rows: old_rows,
                merged_ranges: Vec::new(),
            };
            let new = SheetIr {
                rows: new_rows,
                ..old.clone()
            };

            let forward = diff_sheets(&old, &new);
            let reverse = diff_sheets(&new, &old);

            prop_assert!(forward.row_changes.is_empty());
            prop_assert!(forward.column_changes.is_empty());
            prop_assert_eq!(forward.cell_changes.len(), 1);
            prop_assert!(matches!(forward.cell_changes[0].kind, CellChangeKind::ValueAdded));
            prop_assert!(reverse.row_changes.is_empty());
            prop_assert!(reverse.column_changes.is_empty());
            prop_assert_eq!(reverse.cell_changes.len(), 1);
            prop_assert!(matches!(reverse.cell_changes[0].kind, CellChangeKind::ValueRemoved));
            prop_assert_eq!(
                forward.cell_changes[0].old_address.as_deref(),
                reverse.cell_changes[0].new_address.as_deref()
            );
            prop_assert_eq!(
                forward.cell_changes[0].new_address.as_deref(),
                reverse.cell_changes[0].old_address.as_deref()
            );
        }

        #[test]
        fn same_typed_whole_row_edits_remain_cell_changes(
            row_count in 3usize..16,
            column_count in 2usize..10,
            changed_row in 1usize..15,
            value_kind in 0u8..6,
        ) {
            let changed_row = changed_row.min(row_count - 2);
            let value_kind = generated_value_kind(value_kind);
            let old_rows = (0..row_count)
                .map(|row| {
                    (0..column_count)
                        .map(|column| CellIr {
                            value: format!("old-R{row}-C{column}"),
                            value_kind,
                            formula: None,
                            style_signature: String::new(),
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let mut new_rows = old_rows.clone();
            for (column, cell) in new_rows[changed_row].iter_mut().enumerate() {
                cell.value = format!("new-R{changed_row}-C{column}");
            }
            let old = SheetIr {
                name: "whole-row-edit".to_string(),
                start_row: 1,
                start_col: 1,
                rows: old_rows,
                merged_ranges: Vec::new(),
            };
            let new = SheetIr {
                rows: new_rows,
                ..old.clone()
            };

            let diff = diff_sheets(&old, &new);

            prop_assert!(diff.row_changes.is_empty());
            prop_assert!(diff.column_changes.is_empty());
            prop_assert_eq!(diff.cell_changes.len(), column_count);
            let only_value_changes = diff.cell_changes.iter().all(|change| {
                matches!(change.kind, CellChangeKind::ValueChanged)
            });
            prop_assert!(only_value_changes);
        }

        #[test]
        fn same_typed_whole_column_edits_remain_cell_changes(
            row_count in 2usize..12,
            column_count in 3usize..12,
            changed_column in 1usize..11,
            value_kind in 0u8..6,
        ) {
            let changed_column = changed_column.min(column_count - 2);
            let value_kind = generated_value_kind(value_kind);
            let old_rows = (0..row_count)
                .map(|row| {
                    (0..column_count)
                        .map(|column| CellIr {
                            value: format!("old-R{row}-C{column}"),
                            value_kind,
                            formula: None,
                            style_signature: String::new(),
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let mut new_rows = old_rows.clone();
            for (row, cells) in new_rows.iter_mut().enumerate() {
                cells[changed_column].value = format!("new-R{row}-C{changed_column}");
            }
            let old = SheetIr {
                name: "whole-column-edit".to_string(),
                start_row: 1,
                start_col: 1,
                rows: old_rows,
                merged_ranges: Vec::new(),
            };
            let new = SheetIr {
                rows: new_rows,
                ..old.clone()
            };

            let diff = diff_sheets(&old, &new);

            prop_assert!(diff.row_changes.is_empty());
            prop_assert!(diff.column_changes.is_empty());
            prop_assert_eq!(diff.cell_changes.len(), row_count);
            let only_value_changes = diff.cell_changes.iter().all(|change| {
                matches!(change.kind, CellChangeKind::ValueChanged)
            });
            prop_assert!(only_value_changes);
        }
    }

    fn generated_value_kind(index: u8) -> CellValueKind {
        match index % 6 {
            0 => CellValueKind::String,
            1 => CellValueKind::Number,
            2 => CellValueKind::Boolean,
            3 => CellValueKind::Error,
            4 => CellValueKind::DateTime,
            _ => CellValueKind::Duration,
        }
    }

    fn write_xlsx_rows(
        path: &Path,
        rows: &[&[&str]],
        start_row: u32,
        start_col: u16,
    ) -> Result<()> {
        let mut workbook = Workbook::new();
        let worksheet = workbook.add_worksheet();
        for (row_idx, row) in rows.iter().enumerate() {
            for (col_idx, value) in row.iter().enumerate() {
                worksheet.write_string(
                    start_row + row_idx as u32,
                    start_col + col_idx as u16,
                    *value,
                )?;
            }
        }
        workbook.save(path)?;
        Ok(())
    }

    fn typed_row(values: &[&str]) -> Vec<CellIr> {
        values
            .iter()
            .map(|value| CellIr {
                value: (*value).to_string(),
                value_kind: if value.is_empty() {
                    CellValueKind::Empty
                } else {
                    CellValueKind::String
                },
                formula: None,
                style_signature: String::new(),
            })
            .collect()
    }

    fn sheet(name: &str, rows: &[&[&str]]) -> SheetIr {
        sheet_with_merges(name, rows, &[])
    }

    fn sheet_with_merges(name: &str, rows: &[&[&str]], merged_ranges: &[CellRange]) -> SheetIr {
        SheetIr {
            name: name.to_string(),
            start_row: 1,
            start_col: 1,
            rows: rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|value| CellIr {
                            value: (*value).to_string(),
                            value_kind: if value.is_empty() {
                                CellValueKind::Empty
                            } else {
                                CellValueKind::String
                            },
                            formula: None,
                            style_signature: String::new(),
                        })
                        .collect()
                })
                .collect(),
            merged_ranges: merged_ranges.to_vec(),
        }
    }

    fn workbook(sheets: &[SheetIr]) -> WorkbookIr {
        WorkbookIr {
            sheets: sheets.to_vec(),
            default_style_signature: String::new(),
            explicit_cell_styles: BTreeMap::new(),
        }
    }

    fn unique_test_dir(name: &str) -> Result<PathBuf> {
        let dir = std::env::temp_dir().join(format!(
            "xlsx-diff-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    fn rewrite_xlsx_part(
        source: &Path,
        destination: &Path,
        part_name: &str,
        rewrite: impl FnOnce(String) -> String,
    ) -> Result<()> {
        use std::io::Write;
        use zip::ZipWriter;

        let mut source = ZipArchive::new(File::open(source)?)?;
        let mut destination = ZipWriter::new(File::create(destination)?);
        let mut rewrite = Some(rewrite);
        let mut found = false;
        for index in 0..source.len() {
            let mut entry = source.by_index(index)?;
            let name = entry.name().to_string();
            let options = entry.options();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            destination.start_file(&name, options)?;
            if name == part_name {
                found = true;
                let xml = String::from_utf8(bytes)?;
                let rewritten = rewrite.take().expect("target xlsx part is unique")(xml.clone());
                anyhow::ensure!(
                    rewritten != xml,
                    "xlsx part rewrite did not change {part_name}"
                );
                destination.write_all(rewritten.as_bytes())?;
            } else {
                destination.write_all(&bytes)?;
            }
        }
        anyhow::ensure!(found, "xlsx part not found: {part_name}");
        destination.finish()?;
        Ok(())
    }
}
