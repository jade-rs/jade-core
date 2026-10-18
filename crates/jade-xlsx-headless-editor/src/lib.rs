//! Reading a workbook with no window.
//!
//! The browser and the native shell both go reader → layout → renderer to put
//! a sheet on a surface. A caller with no surface wants the same reader and the
//! same number formats, and none of the geometry: what a cell holds, what it
//! shows, and what formula is behind it.
//!
//! So this depends on `jade-xlsx-layout` only for `number_format` — the display
//! text has to be the text the shells draw, and a second implementation of the
//! format language would drift from it — and never lays anything out. Nothing
//! here measures a font or needs one.
//!
//! Writes patch the original workbook bytes and immediately reread the model,
//! so a headless `set` preserves parts it does not understand and a following
//! read sees exactly what will be exported.

mod auto_filter;
mod current_region;
pub mod fill;
mod merge;
mod sheets;
mod sort;

fn formula_cache_state(cell: &Cell, needs_recalculation: bool) -> Option<FormulaState> {
    cell.formula.as_ref()?;
    Some(if matches!(cell.value, CellValue::Empty) {
        FormulaState::Uncalculated
    } else if needs_recalculation {
        FormulaState::Stale
    } else {
        FormulaState::Cached
    })
}

use std::collections::{BTreeMap, HashSet};

pub use jade_xlsx_calc::formula_text;
use jade_xlsx_headless_api::{
    CellInput, CellKind, CellView, CommentView, DataValidationView, DefinedNameSummary,
    FillSummary, FindHit, FormatSummary, FormulaState, FreezePaneSummary, HeaderFooterChange,
    HeaderFooterView, HyperlinkView, PageBreaksView, RangeView, RecalculationSummary,
    SetFormatSummary, ShapeKindView, ShapeView, SheetNoteView, SheetSummary, SheetText,
    SheetVisibility, SkippedFormula, StructureSummary, TableCellView, TableSummary,
    WorkbookSummary,
};
use jade_xlsx_layout::{column_label, number_format};
use jade_xlsx_reader::{
    Axis, Cell, CellRange, CellRef, CellValue, DateSystem, HeaderFooter, Shape, ShapeKind, Sheet,
    StructureChange, Workbook,
};
pub use jade_xlsx_writer::{BorderLine, BorderStyle, FormatChange};
use jade_xlsx_writer::{
    CachedValue, CachedValueEdit, CellEdit, NewValue, SheetCachedValues, SheetEdits,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HeadlessError {
    #[error("cannot read the workbook: {0}")]
    Read(#[from] jade_xlsx_reader::XlsxError),
    #[error("no sheet called `{0}`")]
    NoSuchSheet(String),
    #[error("`{0}` is not a cell reference or range")]
    BadReference(String),
    #[error("duplicate cell reference `{0}` in one batch")]
    DuplicateReference(String),
    /// A format change that names nothing to change.
    #[error("the format change names nothing to change")]
    EmptyFormatChange,
    /// A fill whose target is not the run grown along one axis. (The run is
    /// not called `source` here: thiserror reads that name as the cause.)
    #[error("`{target}` does not extend `{run}` along one axis: a fill grows its run by rows or by columns, never both, and never shrinks it")]
    FillShape { run: String, target: String },
    /// A range with more cells than a caller is likely to have meant.
    #[error("the range covers {cells} cells, more than the limit of {limit}")]
    RangeTooLarge { cells: u64, limit: u64 },
    #[error("cannot write the workbook: {0}")]
    Write(#[from] jade_xlsx_writer::WriteError),
    #[error("cannot recalculate: {0}")]
    Recalculation(#[from] jade_xlsx_calc::CalcError),
    /// A header/footer change that names nothing to change.
    #[error("the header/footer change names nothing to change")]
    EmptyHeaderFooterChange,
    /// A shape the sheet does not have, by name or id.
    #[error("the sheet has no shape `{0}`")]
    NoSuchShape(String),
    /// A picture, chart or group: nothing to put text into.
    #[error("shape `{0}` has no text body")]
    ShapeHasNoText(String),
    #[error("cannot read the package: {0}")]
    Package(#[from] jade_opc::OpcError),
    #[error("filter: {0}")]
    AutoFilter(String),
    /// A merge of one cell: there is nothing to merge it with.
    #[error("`{0}` is a single cell; a merge needs at least two")]
    MergeTooSmall(String),
    /// A merge that would overlap one the sheet already has.
    #[error("`{range}` overlaps the merged range `{existing}`")]
    MergeOverlaps { range: String, existing: String },
    /// Excel does not merge cells inside a table.
    #[error("`{range}` overlaps the table `{table}`, whose cells cannot be merged")]
    MergeInTable { range: String, table: String },
    /// An unmerge whose range touches no merged cell.
    #[error("`{0}` contains no merged cells")]
    NoSuchMerge(String),
}

impl HeadlessError {
    /// Whether the caller can fix this error by changing its arguments.
    pub fn is_invalid_input(&self) -> bool {
        matches!(
            self,
            HeadlessError::NoSuchSheet(_)
                | HeadlessError::AutoFilter(_)
                | HeadlessError::BadReference(_)
                | HeadlessError::DuplicateReference(_)
                | HeadlessError::FillShape { .. }
                | HeadlessError::EmptyFormatChange
                | HeadlessError::Write(
                    jade_xlsx_writer::WriteError::Sort(_)
                        | jade_xlsx_writer::WriteError::BadColor { .. }
                        | jade_xlsx_writer::WriteError::BadFontSize
                        | jade_xlsx_writer::WriteError::NoSuchStyle { .. }
                        | jade_xlsx_writer::WriteError::NoSuchSheet(_)
                        | jade_xlsx_writer::WriteError::EmptyStructureChange
                        | jade_xlsx_writer::WriteError::StructureOutOfBounds { .. }
                        | jade_xlsx_writer::WriteError::InvalidSheetName { .. }
                        | jade_xlsx_writer::WriteError::DuplicateSheetName(_)
                        | jade_xlsx_writer::WriteError::LastVisibleSheet(_)
                        | jade_xlsx_writer::WriteError::SheetPositionOutOfRange { .. }
                )
                | HeadlessError::RangeTooLarge { .. }
                | HeadlessError::MergeTooSmall(_)
                | HeadlessError::MergeOverlaps { .. }
                | HeadlessError::MergeInTable { .. }
                | HeadlessError::NoSuchMerge(_)
                | HeadlessError::EmptyHeaderFooterChange
                | HeadlessError::NoSuchShape(_)
                | HeadlessError::ShapeHasNoText(_)
                | HeadlessError::Write(
                    jade_xlsx_writer::WriteError::CellOutOfBounds { .. }
                        | jade_xlsx_writer::WriteError::NonFiniteNumber { .. }
                        | jade_xlsx_writer::WriteError::InvalidXmlCharacter { .. }
                        | jade_xlsx_writer::WriteError::EmptyFormula { .. }
                )
        )
    }
}

pub type Result<T> = std::result::Result<T, HeadlessError>;

/// How many cells one `range` call will materialise.
///
/// A caller that asks for `A1:XFD1048576` means the used range, not seventeen
/// billion empty cells; refusing beats spending the memory to find out.
pub const RANGE_CELL_LIMIT: u64 = 1_000_000;

/// A workbook opened for reading, and for writing back.
///
/// The original bytes are kept because a write is a patch of them, not a
/// regeneration: that is what lets a save leave every part it did not touch
/// exactly as it was.
pub struct HeadlessWorkbook {
    clipboard_origin: usize,
    workbook: Workbook,
    source: Vec<u8>,
    automatic_height_base: Option<Vec<u8>>,
    calculation_time_zone: String,
    source_time_zone: String,
    /// The bytes before each write, newest last. A write is a patch of the
    /// whole package, so the package itself is the cheapest faithful undo
    /// record: restoring it brings back every cell, cache and flag exactly.
    history: Vec<HistorySnapshot>,
    /// Writes undone and not yet redone, newest last.
    future: Vec<HistorySnapshot>,
    filter_row_state: BTreeMap<String, auto_filter::FilterRowState>,
}

struct HistorySnapshot {
    bytes: Vec<u8>,
    time_zone: String,
    filter_row_state: BTreeMap<String, auto_filter::FilterRowState>,
}

/// How many writes can be undone. Each step holds a whole package, so this
/// bounds memory at roughly a hundred copies of the file.
pub const UNDO_LIMIT: usize = 100;

impl HeadlessWorkbook {
    /// Uses the caller's shared text measurer to fold height-cache updates into
    /// the pending edit. History restoration never schedules a re-measurement.
    pub fn refresh_automatic_row_heights(
        &mut self,
        options: &jade_xlsx_layout::LayoutOptions,
    ) -> Result<bool> {
        let Some(base) = self.automatic_height_base.as_ref() else {
            return Ok(false);
        };
        let before = jade_xlsx_reader::read_xlsx(base)?;
        let mut updates = Vec::new();
        for sheet in &self.workbook.sheets {
            let Some(old) = before
                .sheets
                .iter()
                .find(|old| old.part_name == sheet.part_name)
            else {
                continue;
            };
            if !sheet.rows.iter().any(|row| !row.custom_height) {
                continue;
            }
            if old.rows == sheet.rows
                && old.columns == sheet.columns
                && old.merges == sheet.merges
                && old.conditional_formats == sheet.conditional_formats
                && old.default_column_width_chars == sheet.default_column_width_chars
                && old.default_row_height_points == sheet.default_row_height_points
                && old.default_row_custom_height == sheet.default_row_custom_height
                && before.styles == self.workbook.styles
            {
                continue;
            }
            let old_fit = jade_xlsx_layout::resize::auto_fit_all_rows(old, &before.styles, options);
            let fit =
                jade_xlsx_layout::resize::auto_fit_all_rows(sheet, &self.workbook.styles, options);
            let mut sizes = BTreeMap::new();
            for row in sheet.rows.iter().filter(|row| !row.custom_height) {
                let needed = fit
                    .overrides
                    .get(&row.index)
                    .copied()
                    .unwrap_or(fit.default_points);
                let previous = old_fit
                    .overrides
                    .get(&row.index)
                    .copied()
                    .unwrap_or(old_fit.default_points);
                let current = row
                    .height_points
                    .or(sheet.default_row_height_points)
                    .unwrap_or(jade_xlsx_layout::units::DEFAULT_ROW_HEIGHT_POINTS);
                if needed != previous && needed != current {
                    sizes.insert(row.index, needed);
                }
            }
            if !sizes.is_empty() {
                updates.push((sheet.name.clone(), sizes));
            }
        }
        let changed = !updates.is_empty();
        let mut bytes = self.source.clone();
        for (sheet, sizes) in updates {
            bytes = jade_xlsx_writer::dimensions::refresh_automatic_row_sizes(
                &bytes,
                &self.workbook,
                &sheet,
                &sizes,
            )?;
        }
        if changed {
            let workbook = jade_xlsx_reader::read_xlsx(&bytes)?;
            self.source = bytes;
            self.workbook = workbook;
        }
        self.automatic_height_base = None;
        Ok(changed)
    }

    /// An environment-dependent view; raw document/history bytes stay intact.
    pub fn workbook_with_refreshed_row_metrics(
        &self,
        options: &jade_xlsx_layout::LayoutOptions,
    ) -> Workbook {
        self.workbook_with_refreshed_row_metrics_for_fonts(options, None)
    }

    pub fn workbook_with_refreshed_row_metrics_for_fonts(
        &self,
        options: &jade_xlsx_layout::LayoutOptions,
        families: Option<&HashSet<String>>,
    ) -> Workbook {
        let mut workbook = self.workbook.clone();
        for sheet in &mut workbook.sheets {
            if !sheet.rows.iter().any(|row| !row.custom_height) {
                continue;
            }
            let fit = jade_xlsx_layout::resize::auto_fit_all_rows(sheet, &workbook.styles, options);
            let matches_font = |name: &str| {
                families.is_none_or(|families| {
                    families
                        .iter()
                        .any(|family| name.eq_ignore_ascii_case(family))
                })
            };
            let normal_changed = matches_font(
                workbook
                    .styles
                    .normal_font()
                    .and_then(|font| font.name.as_deref())
                    .unwrap_or(&options.default_font.family),
            );
            let affected: HashSet<_> = sheet
                .rows
                .iter()
                .filter(|row| {
                    !row.custom_height
                        && (normal_changed
                            || row.cells.iter().any(|cell| {
                                let format = workbook.styles.cell_format(
                                    sheet.effective_style_index(cell.reference, cell.style_index),
                                );
                                let name = format
                                    .and_then(|format| workbook.styles.font(format))
                                    .and_then(|font| font.name.as_deref())
                                    .unwrap_or(&options.default_font.family);
                                matches_font(name)
                            }))
                })
                .map(|row| row.index)
                .collect();
            for row in sheet
                .rows
                .iter_mut()
                .filter(|row| affected.contains(&row.index))
            {
                row.height_points = Some(
                    fit.overrides
                        .get(&row.index)
                        .copied()
                        .unwrap_or(fit.default_points),
                );
            }
        }
        workbook
    }

    /// Synchronizes environment-dependent height caches only in the export.
    pub fn export_bytes_with_row_metrics(
        &self,
        options: &jade_xlsx_layout::LayoutOptions,
    ) -> Result<Vec<u8>> {
        self.export_bytes_with_row_metrics_for_fonts(options, None)
    }

    pub fn export_bytes_with_row_metrics_for_fonts(
        &self,
        options: &jade_xlsx_layout::LayoutOptions,
        families: Option<&HashSet<String>>,
    ) -> Result<Vec<u8>> {
        let measured = self.workbook_with_refreshed_row_metrics_for_fonts(options, families);
        let mut bytes = self.export_bytes()?;
        for (sheet, original) in measured.sheets.iter().zip(&self.workbook.sheets) {
            let sizes: BTreeMap<_, _> = sheet
                .rows
                .iter()
                .zip(&original.rows)
                .filter(|(row, old)| {
                    !row.custom_height
                        && row.height_points
                            != old
                                .height_points
                                .or(original.default_row_height_points)
                                .or(Some(jade_xlsx_layout::units::DEFAULT_ROW_HEIGHT_POINTS))
                })
                .filter_map(|(row, _)| row.height_points.map(|height| (row.index, height)))
                .collect();
            if !sizes.is_empty() {
                bytes = jade_xlsx_writer::dimensions::refresh_automatic_row_sizes(
                    &bytes,
                    &self.workbook,
                    &sheet.name,
                    &sizes,
                )?;
            }
        }
        Ok(bytes)
    }

    pub fn set_axis_sizes(
        &mut self,
        sheet: &str,
        axis: Axis,
        sizes: &BTreeMap<u32, f32>,
    ) -> Result<()> {
        if sizes.is_empty() {
            return Ok(());
        }
        let written = jade_xlsx_writer::dimensions::set_axis_sizes(
            &self.source,
            &self.workbook,
            sheet,
            axis,
            sizes,
        )?;
        let state = (axis == Axis::Rows).then(|| self.filter_state_for_row_sizes(sheet, sizes));
        self.commit_axis_sizes_with_filter_state(written, state)
    }

    fn commit_axis_sizes(&mut self, written: Vec<u8>) -> Result<()> {
        self.commit_axis_sizes_with_filter_state(written, None)
    }

    fn commit_axis_sizes_with_filter_state(
        &mut self,
        mut written: Vec<u8>,
        filter_state: Option<BTreeMap<String, auto_filter::FilterRowState>>,
    ) -> Result<()> {
        let mut workbook = jade_xlsx_reader::read_xlsx(&written)?;
        let visibility_changed =
            self.workbook
                .sheets
                .iter()
                .zip(&workbook.sheets)
                .any(|(before, after)| {
                    if before.default_rows_hidden != after.default_rows_hidden {
                        return true;
                    }
                    let exceptions = |sheet: &Sheet| {
                        sheet
                            .rows
                            .iter()
                            .filter(|row| row.hidden != sheet.default_rows_hidden)
                            .map(|row| row.index)
                            .collect::<HashSet<_>>()
                    };
                    exceptions(before) != exceptions(after)
                });
        if visibility_changed
            && workbook
                .sheets
                .iter()
                .flat_map(|sheet| &sheet.rows)
                .flat_map(|row| &row.cells)
                .any(|cell| cell.formula.is_some())
        {
            written = jade_xlsx_writer::mark_for_recalculation(&written)?;
            workbook = jade_xlsx_reader::read_xlsx(&written)?;
        }
        self.workbook = workbook;
        let before = std::mem::replace(&mut self.source, written);
        self.remember(before);
        if let Some(state) = filter_state {
            self.filter_row_state = state;
            let sheets: Vec<_> = self.filter_row_state.keys().cloned().collect();
            for sheet in sheets {
                self.persist_filter_row_state(&sheet)?;
            }
        }
        if visibility_changed {
            self.recalculate_after_edit();
        }
        Ok(())
    }

    /// Applies an automatic row fit without pinning future wrapped content.
    pub fn set_fitted_row_sizes(&mut self, sheet: &str, sizes: &BTreeMap<u32, f32>) -> Result<()> {
        if sizes.is_empty() {
            return Ok(());
        }
        let written = jade_xlsx_writer::dimensions::set_fitted_row_sizes(
            &self.source,
            &self.workbook,
            sheet,
            sizes,
        )?;
        self.commit_axis_sizes(written)
    }

    /// Applies a default and sparse row exceptions as one undo transaction.
    pub fn set_row_sizes_with_default(
        &mut self,
        sheet: &str,
        default: f32,
        overrides: &BTreeMap<u32, f32>,
        custom_height: bool,
    ) -> Result<()> {
        let written = jade_xlsx_writer::dimensions::set_row_sizes_with_default(
            &self.source,
            &self.workbook,
            sheet,
            default,
            overrides,
            custom_height,
        )?;
        self.commit_axis_sizes(written)
    }

    pub fn set_axis_size(
        &mut self,
        sheet: &str,
        axis: Axis,
        first: u32,
        last: u32,
        size: f32,
    ) -> Result<()> {
        let limit = if axis == Axis::Rows {
            jade_xlsx_reader::model::MAX_ROW_COUNT
        } else {
            jade_xlsx_reader::model::MAX_COLUMN_COUNT
        };
        if first > last || last >= limit {
            return Err(jade_xlsx_writer::WriteError::StructureOutOfBounds {
                at: first,
                count: last.saturating_sub(first).saturating_add(1),
            }
            .into());
        }
        if axis == Axis::Rows && first == 0 && last == limit - 1 {
            let written = jade_xlsx_writer::dimensions::set_all_row_sizes(
                &self.source,
                &self.workbook,
                sheet,
                size,
            )?;
            let state = self.filter_state_for_all_row_sizes(sheet, size);
            return self.commit_axis_sizes_with_filter_state(written, Some(state));
        }
        if (last - first + 1) as usize > jade_xlsx_writer::dimensions::AXIS_SIZE_LIMIT {
            return Err(jade_xlsx_writer::WriteError::AxisFormatLimit {
                limit: jade_xlsx_writer::dimensions::AXIS_SIZE_LIMIT as u64,
            }
            .into());
        }
        self.set_axis_sizes(
            sheet,
            axis,
            &(first..=last).map(|index| (index, size)).collect(),
        )
    }

    pub fn open(bytes: &[u8]) -> Result<Self> {
        Self::open_in_time_zone(bytes, "UTC")
    }

    /// Open for editing, refreshing automatic-mode formulas before use.
    pub fn open_for_editing(bytes: &[u8]) -> Result<Self> {
        Self::open_for_editing_in_time_zone(bytes, "UTC")
    }

    pub fn open_for_editing_in_time_zone(bytes: &[u8], time_zone: &str) -> Result<Self> {
        let mut workbook = Self::open_in_time_zone(bytes, time_zone)?;
        workbook.recalculate_on_open()?;
        Ok(workbook)
    }

    /// Initialize automatic calculation without adding an Undo entry.
    /// Imported calculation-engine IDs cannot establish freshness for our engine.
    pub fn recalculate_on_open(&mut self) -> Result<Option<RecalculationSummary>> {
        if !self.workbook.calculation_mode.calculates_on_edit()
            || !self
                .workbook
                .sheets
                .iter()
                .flat_map(|sheet| &sheet.rows)
                .flat_map(|row| &row.cells)
                .any(|cell| cell.formula.is_some())
        {
            return Ok(None);
        }
        self.recalculate().map(Some)
    }

    /// Open without changing stored caches; the configured zone applies to
    /// subsequent edits and explicit recalculation.
    pub fn open_in_time_zone(bytes: &[u8], time_zone: &str) -> Result<Self> {
        jade_xlsx_calc::validate_time_zone(time_zone)?;
        static ORIGINS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
        let mut opened = Self {
            clipboard_origin: ORIGINS.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            workbook: jade_xlsx_reader::read_xlsx(bytes)?,
            source: bytes.to_vec(),
            automatic_height_base: None,
            calculation_time_zone: time_zone.to_string(),
            source_time_zone: time_zone.to_string(),
            history: Vec::new(),
            future: Vec::new(),
            filter_row_state: BTreeMap::new(),
        };
        opened.initialize_filter_row_state();
        Ok(opened)
    }

    pub fn calculation_time_zone(&self) -> &str {
        &self.calculation_time_zone
    }

    /// Host environment configuration, not a document edit or an Undo step.
    /// Recalculate explicitly to refresh caches before the next edit.
    pub fn set_calculation_time_zone(&mut self, time_zone: &str) -> Result<()> {
        jade_xlsx_calc::validate_time_zone(time_zone)?;
        self.calculation_time_zone = time_zone.to_string();
        Ok(())
    }

    /// Persist calculation preferences without adding a document-edit Undo step.
    pub fn set_calculation_properties(
        &mut self,
        mode: jade_xlsx_reader::CalculationMode,
        calculate_on_save: bool,
    ) -> Result<()> {
        if self.workbook.calculation_mode == mode
            && self.workbook.calculate_on_save == calculate_on_save
        {
            return Ok(());
        }
        let written =
            jade_xlsx_writer::set_calculation_properties(&self.source, mode, calculate_on_save)?;
        let model = jade_xlsx_reader::read_xlsx(&written)?;
        self.source = written;
        self.workbook = model;
        self.recalculate_after_edit();
        Ok(())
    }

    fn recalculate_after_edit(&mut self) {
        if self.workbook.calculation_mode.calculates_on_edit() {
            let _ = self.recalculate();
        }
    }

    /// File-save semantics, including calcOnSave. Snapshot export remains read-only.
    pub fn save_bytes(&mut self) -> Result<Vec<u8>> {
        self.prepare_for_save()?;
        self.export_bytes()
    }

    pub fn prepare_for_save(&mut self) -> Result<()> {
        if self.workbook.calculate_on_save {
            self.recalculate()?;
        }
        Ok(())
    }

    fn restored_snapshot(&self, snapshot: &HistorySnapshot) -> Result<Self> {
        let mut restored = Self::open_in_time_zone(&snapshot.bytes, &self.calculation_time_zone)?;
        restored.filter_row_state = snapshot.filter_row_state.clone();
        restored.source_time_zone = snapshot.time_zone.clone();
        restored.set_calculation_properties(
            self.workbook.calculation_mode,
            self.workbook.calculate_on_save,
        )?;
        if snapshot.time_zone != self.calculation_time_zone
            && self.workbook.calculation_mode.calculates_on_edit()
        {
            restored.recalculate()?;
        }
        Ok(restored)
    }

    /// Whether [`HeadlessWorkbook::undo`] has a write to take back.
    pub fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }

    /// Whether [`HeadlessWorkbook::redo`] has an undone write to reapply.
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// Takes back the last write — the recalculation that followed it
    /// included. A changed host time zone refreshes the restored caches before
    /// applying the snapshot; otherwise the original bytes are restored exactly.
    /// Returns `false`, changing nothing, when there is nothing to undo.
    pub fn undo(&mut self) -> Result<bool> {
        let Some(previous) = self.history.last() else {
            return Ok(false);
        };
        let restored = self.restored_snapshot(previous)?;
        self.history.pop();
        let current = HistorySnapshot {
            bytes: std::mem::replace(&mut self.source, restored.source),
            time_zone: std::mem::replace(&mut self.source_time_zone, restored.source_time_zone),
            filter_row_state: std::mem::replace(
                &mut self.filter_row_state,
                restored.filter_row_state,
            ),
        };
        self.future.push(current);
        self.workbook = restored.workbook;
        self.automatic_height_base = restored.automatic_height_base;
        Ok(true)
    }

    /// Reapplies the last undone write. Returns `false` when there is none.
    pub fn redo(&mut self) -> Result<bool> {
        let Some(next) = self.future.last() else {
            return Ok(false);
        };
        let restored = self.restored_snapshot(next)?;
        self.future.pop();
        let current = HistorySnapshot {
            bytes: std::mem::replace(&mut self.source, restored.source),
            time_zone: std::mem::replace(&mut self.source_time_zone, restored.source_time_zone),
            filter_row_state: std::mem::replace(
                &mut self.filter_row_state,
                restored.filter_row_state,
            ),
        };
        self.history.push(current);
        self.workbook = restored.workbook;
        self.automatic_height_base = restored.automatic_height_base;
        Ok(true)
    }

    /// Records the bytes a write is about to replace. A new write forks
    /// history, so the redo branch is dropped.
    fn remember(&mut self, before: Vec<u8>) {
        if self.automatic_height_base.is_none() {
            self.automatic_height_base = Some(before.clone());
        }
        self.history.push(HistorySnapshot {
            bytes: before,
            time_zone: self.source_time_zone.clone(),
            filter_row_state: self.filter_row_state.clone(),
        });
        self.future.clear();
        if self.history.len() > UNDO_LIMIT {
            self.history.remove(0);
        }
    }

    /// The stored snapshot, including everything written into it. History
    /// restores these exact bytes; use export_bytes for synchronized caches.
    pub fn bytes(&self) -> &[u8] {
        &self.source
    }

    /// Export with current chart caches without mutating the undo snapshot.
    pub fn export_bytes(&self) -> Result<Vec<u8>> {
        if !self
            .workbook
            .sheets
            .iter()
            .any(|sheet| sheet.shapes.iter().any(|shape| shape.chart.is_some()))
        {
            return Ok(self.source.clone());
        }
        Ok(jade_xlsx_writer::refresh_chart_caches(&self.source)?.0)
    }

    /// Puts a value in a cell.
    ///
    /// The write lands in the source bytes immediately and the model is reread
    /// from them, so a `get` after a `set` reports what was written rather than
    /// what was there — which is the behaviour a caller assumes and the one a
    /// buffered edit list would quietly break.
    pub fn set(&mut self, sheet: &str, reference: &str, value: CellInput) -> Result<()> {
        self.set_cells(sheet, &[(reference.to_string(), value)])
    }

    /// Puts values in several cells, patching the part once.
    pub fn set_cells(&mut self, sheet: &str, values: &[(String, CellInput)]) -> Result<()> {
        let values = values
            .iter()
            .map(|(reference, value)| (sheet.to_string(), reference.clone(), value.clone()))
            .collect::<Vec<_>>();
        self.set_workbook_cells(&values)
    }

    /// Puts values in several cells, each with a `cellXfs` style index to
    /// wear — `None` keeps the style the cell has. This is what a paste or a
    /// fill that carries formatting goes through.
    pub fn set_cells_styled(
        &mut self,
        sheet: &str,
        values: &[(String, CellInput, Option<u32>)],
    ) -> Result<()> {
        let values = values
            .iter()
            .map(|(reference, value, style)| {
                (sheet.to_string(), reference.clone(), value.clone(), *style)
            })
            .collect::<Vec<_>>();
        self.set_workbook_cells_styled(&values)
    }

    /// Puts values in several sheets as one atomic workbook update.
    ///
    /// Every selector and reference is validated before `self` changes. The
    /// writer patches a temporary package, so a failure in any sheet leaves all
    /// sheets and the export bytes untouched.
    pub fn set_workbook_cells(&mut self, values: &[(String, String, CellInput)]) -> Result<()> {
        let values = values
            .iter()
            .map(|(sheet, reference, value)| {
                (sheet.clone(), reference.clone(), value.clone(), None)
            })
            .collect::<Vec<_>>();
        self.set_workbook_cells_styled(&values)
    }

    /// [`Self::set_workbook_cells`] with a style index per cell.
    pub fn set_workbook_cells_styled(
        &mut self,
        values: &[(String, String, CellInput, Option<u32>)],
    ) -> Result<()> {
        let mut edits = Vec::with_capacity(values.len());
        for (sheet, reference, value, style_index) in values {
            let index = self.sheet_index(sheet)?;
            let parsed = CellRef::parse(reference)
                .map_err(|_| HeadlessError::BadReference(reference.clone()))?;
            edits.push((
                index,
                CellEdit {
                    reference: parsed,
                    value: match value {
                        CellInput::Number(number) => NewValue::Number(*number),
                        CellInput::Text(text) => NewValue::Text(text.clone()),
                        CellInput::Bool(flag) => NewValue::Bool(*flag),
                        // Stored as Excel stores it: `_xlfn.` before the
                        // newer functions, `_xlpm.` on LET/LAMBDA names.
                        CellInput::Formula(formula) => {
                            NewValue::Formula(jade_xlsx_calc::formula_text::to_file_formula(
                                formula.strip_prefix('=').unwrap_or(formula),
                            ))
                        }
                        CellInput::Blank => NewValue::Blank,
                    },
                    style_index: *style_index,
                },
            ));
        }
        self.write_edits(edits)
    }

    /// One atomic write of edits across sheets: one undo step, then a
    /// best-effort recalculation.
    fn write_edits(&mut self, edits: Vec<(usize, CellEdit)>) -> Result<()> {
        self.write_edits_with_styles(edits, None)
    }

    /// [`Self::write_edits`] with a new styles part to land alongside, so
    /// the `xf` indices the edits name exist in the file they go into.
    fn write_edits_with_styles(
        &mut self,
        edits: Vec<(usize, CellEdit)>,
        styles: Option<(String, String)>,
    ) -> Result<()> {
        self.write_edits_with_parts(edits, styles, Vec::new())
    }

    /// [`Self::write_edits_with_styles`] with further parts replaced
    /// wholesale in the same write — a sheet part whose merge list changed
    /// alongside the cells the merge cleared. The cell edits patch the
    /// replaced text.
    fn write_edits_with_parts(
        &mut self,
        edits: Vec<(usize, CellEdit)>,
        styles: Option<(String, String)>,
        extra_parts: Vec<(String, String)>,
    ) -> Result<()> {
        let mut references = HashSet::with_capacity(edits.len());
        for (index, edit) in &edits {
            if !references.insert((*index, edit.reference)) {
                return Err(HeadlessError::DuplicateReference(format_reference(
                    edit.reference,
                )));
            }
        }
        // A table's header cells are its column names: typing into one
        // renames the column, as in Excel, and the name has to stay unique
        // and non-empty for Excel to open the file without repairing it.

        let mut grouped: BTreeMap<usize, Vec<CellEdit>> = BTreeMap::new();
        for (index, edit) in edits {
            grouped.entry(index).or_default().push(edit);
        }

        let mut sheets = grouped
            .into_iter()
            .map(|(index, cells)| SheetEdits {
                part_name: self.workbook.sheets[index].part_name.clone(),
                cells,
            })
            .collect::<Vec<_>>();
        let mut parts = extra_parts;
        parts.extend(jade_xlsx_writer::table::follow_table_headers(
            &self.source,
            &self.workbook,
            &mut sheets,
        )?);
        let written = jade_xlsx_writer::write_xlsx_with_parts(
            &self.source,
            styles
                .as_ref()
                .map(|(name, xml)| (name.as_str(), xml.as_str())),
            &sheets,
            &parts,
        )?;
        self.workbook = jade_xlsx_reader::read_xlsx(&written)?;
        let before = std::mem::replace(&mut self.source, written);
        // One undo step per write; the recalculation below folds into it.
        self.remember(before);

        // Best effort: an edit whose dependents use unsupported functions
        // still lands, and the file's own
        // `fullCalcOnLoad` keeps other applications honest about it.
        self.recalculate_after_edit();
        Ok(())
    }

    /// Replaces whole parts — one undo step, no recalculation, since no
    /// cell changed.
    fn replace_parts(&mut self, parts: Vec<(String, String)>) -> Result<()> {
        let written = jade_xlsx_writer::write_xlsx_with_parts(&self.source, None, &[], &parts)?;
        self.workbook = jade_xlsx_reader::read_xlsx(&written)?;
        let before = std::mem::replace(&mut self.source, written);
        self.remember(before);
        Ok(())
    }

    /// Changes the sheet's page header and footer: each section given is
    /// set (`Some(None)` clears it), each left out is kept. One undo step.
    pub fn set_header_footer(
        &mut self,
        sheet: &str,
        change: &HeaderFooterChange,
    ) -> Result<HeaderFooterView> {
        if change.is_empty() {
            return Err(HeadlessError::EmptyHeaderFooterChange);
        }
        let index = self.sheet_index(sheet)?;
        let sheet_model = &self.workbook.sheets[index];
        let mut header_footer = sheet_model.header_footer.clone().unwrap_or(HeaderFooter {
            scale_with_doc: true,
            align_with_margins: true,
            ..HeaderFooter::default()
        });
        let apply = |slot: &mut Option<String>, value: &Option<Option<String>>| {
            if let Some(value) = value {
                *slot = value.clone().filter(|text| !text.is_empty());
            }
        };
        apply(&mut header_footer.odd_header, &change.odd_header);
        apply(&mut header_footer.odd_footer, &change.odd_footer);
        apply(&mut header_footer.even_header, &change.even_header);
        apply(&mut header_footer.even_footer, &change.even_footer);
        apply(&mut header_footer.first_header, &change.first_header);
        apply(&mut header_footer.first_footer, &change.first_footer);
        if let Some(flag) = change.different_odd_even {
            header_footer.different_odd_even = flag;
        }
        if let Some(flag) = change.different_first {
            header_footer.different_first = flag;
        }
        if let Some(flag) = change.scale_with_doc {
            header_footer.scale_with_doc = flag;
        }
        if let Some(flag) = change.align_with_margins {
            header_footer.align_with_margins = flag;
        }

        let package = jade_opc::OpcPackage::open(&self.source)?;
        let part_name = sheet_model.part_name.clone();
        let xml = package
            .part_str(&part_name)?
            .ok_or_else(|| jade_xlsx_writer::WriteError::MissingPart(part_name.clone()))?;
        let patched = jade_xlsx_writer::set_header_footer(&xml, &header_footer)?;
        self.replace_parts(vec![(part_name, patched)])?;
        Ok(self.workbook.sheets[index]
            .header_footer
            .as_ref()
            .map(header_footer_view)
            .unwrap_or_default())
    }

    /// Puts a note (a legacy comment) on a cell, replacing the one it has.
    /// The comments and VML parts are created when the sheet has none. One
    /// undo step.
    pub fn set_note(
        &mut self,
        sheet: &str,
        reference: &str,
        text: &str,
        author: &str,
    ) -> Result<CommentView> {
        let index = self.sheet_index(sheet)?;
        let at = CellRef::parse(reference)
            .map_err(|_| HeadlessError::BadReference(reference.to_string()))?;
        let part_name = self.workbook.sheets[index].part_name.clone();
        let mut package = jade_opc::OpcPackage::open(&self.source)?;
        jade_xlsx_writer::set_note(&mut package, &part_name, at, author, text)?;
        self.replace_package(package)?;
        self.workbook.sheets[index]
            .comment_at(at)
            .map(|comment| CommentView {
                author: comment.author.clone(),
                text: comment.text.clone(),
            })
            .ok_or_else(|| HeadlessError::BadReference(reference.to_string()))
    }

    /// Removes the note on a cell. `false`, changing nothing, when the cell
    /// has none.
    pub fn delete_note(&mut self, sheet: &str, reference: &str) -> Result<bool> {
        let index = self.sheet_index(sheet)?;
        let at = CellRef::parse(reference)
            .map_err(|_| HeadlessError::BadReference(reference.to_string()))?;
        let part_name = self.workbook.sheets[index].part_name.clone();
        let mut package = jade_opc::OpcPackage::open(&self.source)?;
        if !jade_xlsx_writer::delete_note(&mut package, &part_name, at)? {
            return Ok(false);
        }
        self.replace_package(package)?;
        Ok(true)
    }

    /// Makes an edited package the workbook — one undo step, no
    /// recalculation, since no cell changed.
    fn replace_package(&mut self, package: jade_opc::OpcPackage) -> Result<()> {
        let written = package.write()?;
        self.workbook = jade_xlsx_reader::read_xlsx(&written)?;
        let before = std::mem::replace(&mut self.source, written);
        self.remember(before);
        Ok(())
    }

    /// Puts `text` in the shape called `shape` (its name, `TextBox 1`, or
    /// its id) — one paragraph per line, in the font the shape's text had.
    /// One undo step.
    pub fn set_shape_text(&mut self, sheet: &str, shape: &str, text: &str) -> Result<ShapeView> {
        let index = self.sheet_index(sheet)?;
        let sheet_model = &self.workbook.sheets[index];
        let found = sheet_model
            .shape_named(shape)
            .ok_or_else(|| HeadlessError::NoSuchShape(shape.to_string()))?;
        if found.text.is_none() {
            return Err(HeadlessError::ShapeHasNoText(shape.to_string()));
        }
        let id = found.id;
        let package = jade_opc::OpcPackage::open(&self.source)?;
        let rels_name = jade_opc::rels_part_name(&sheet_model.part_name);
        let drawing_part = package
            .part_str(&rels_name)?
            .map(|xml| jade_opc::parse_relationships(&xml))
            .transpose()?
            .and_then(|rels| {
                rels.target_by_type_suffix("/drawing")
                    .map(|target| jade_opc::resolve_target(&sheet_model.part_name, target))
            })
            .ok_or_else(|| HeadlessError::NoSuchShape(shape.to_string()))?;
        let xml = package
            .part_str(&drawing_part)?
            .ok_or_else(|| jade_xlsx_writer::WriteError::MissingPart(drawing_part.clone()))?;
        let patched = jade_xlsx_writer::set_shape_text(&xml, id, text)?
            .ok_or_else(|| HeadlessError::ShapeHasNoText(shape.to_string()))?;
        self.replace_parts(vec![(drawing_part, patched)])?;
        self.workbook.sheets[index]
            .shapes
            .iter()
            .find(|shape| shape.id == id)
            .map(shape_view)
            .ok_or_else(|| HeadlessError::NoSuchShape(shape.to_string()))
    }

    /// Continues the run in `source` into the rest of `target`, the way
    /// dragging the fill handle does: numbers as a linear series, a lone
    /// date by the day, counting text by its number, anything else as a
    /// repeated pattern with formulas moved. `target` has to be `source`
    /// grown along one axis. One write, and so one undo step; a target equal
    /// to the source writes nothing.
    pub fn fill(&mut self, sheet: &str, source: &str, target: &str) -> Result<FillSummary> {
        let index = self.sheet_index(sheet)?;
        let source_range = parse_range(source)?;
        let target_range = parse_range(target)?;
        if fill::fill_axis(source_range, target_range).is_none() {
            return Err(HeadlessError::FillShape {
                run: source.to_string(),
                target: target.to_string(),
            });
        }
        ensure_dense_result_fits(
            target_range.end.column - target_range.start.column + 1,
            target_range.end.row - target_range.start.row + 1,
        )?;

        let sheet = &self.workbook.sheets[index];
        let sheet_name = sheet.name.clone();
        let values = fill::plan_fill(sheet, &self.workbook.styles, source_range, target_range)
            .into_iter()
            .map(|write| (format_reference(write.at), write.value, write.style_index))
            .collect::<Vec<_>>();
        if !values.is_empty() {
            self.set_cells_styled(&sheet_name, &values)?;
        }
        Ok(FillSummary {
            sheet: sheet_name,
            source: format_range(source_range),
            target: format_range(target_range),
            cells: values
                .into_iter()
                .map(|(reference, ..)| reference)
                .collect(),
        })
    }

    /// Excel's format painter: gives the cells of `target` the formats of
    /// `source`, tiled when the target is larger, and — when the target is a
    /// single cell — stamps the source's whole shape from it. Values,
    /// formulas and caches stay as they are. One write, one undo step.
    pub fn copy_format(
        &mut self,
        sheet: &str,
        source: &str,
        target: &str,
    ) -> Result<FormatSummary> {
        let index = self.sheet_index(sheet)?;
        let source_range = parse_range(source)?;
        let mut target_range = parse_range(target)?;
        if target_range.start == target_range.end {
            target_range.end = CellRef::new(
                target_range.start.column + (source_range.end.column - source_range.start.column),
                target_range.start.row + (source_range.end.row - source_range.start.row),
            );
        }
        ensure_dense_result_fits(
            target_range.end.column - target_range.start.column + 1,
            target_range.end.row - target_range.start.row + 1,
        )?;

        let sheet = &self.workbook.sheets[index];
        let sheet_name = sheet.name.clone();
        let source_rows = source_range.end.row - source_range.start.row + 1;
        let source_columns = source_range.end.column - source_range.start.column + 1;
        let mut edits = Vec::new();
        for row in target_range.start.row..=target_range.end.row {
            for column in target_range.start.column..=target_range.end.column {
                let from = CellRef::new(
                    source_range.start.column
                        + (column - target_range.start.column) % source_columns,
                    source_range.start.row + (row - target_range.start.row) % source_rows,
                );
                edits.push((
                    index,
                    CellEdit {
                        reference: CellRef::new(column, row),
                        value: NewValue::Unchanged,
                        style_index: Some(effective_style(sheet, from)),
                    },
                ));
            }
        }
        let cells = edits
            .iter()
            .map(|(_, edit)| format_reference(edit.reference))
            .collect();
        self.write_edits(edits)?;
        Ok(FormatSummary {
            sheet: sheet_name,
            source: format_range(source_range),
            target: format_range(target_range),
            cells,
        })
    }

    /// Base formatting at the merged-cell anchor, independent of conditional styles
    /// and the display theme. Unresolved colors/number formats remain unspecified.
    pub fn format_at(&self, sheet_name: &str, reference: &str) -> Result<FormatChange> {
        let sheet = &self.workbook.sheets[self.sheet_index(sheet_name)?];
        let reference = CellRef::parse(reference)
            .map_err(|_| HeadlessError::BadReference(reference.to_string()))?;
        let reference = sheet
            .merge_at(reference)
            .map_or(reference, |range| range.start);
        let styles = &self.workbook.styles;
        let format = styles.cell_format(Some(effective_style(sheet, reference)));
        let font = format.and_then(|format| styles.font(format));
        let alignment = format.map(|format| format.alignment).unwrap_or_default();
        let color = |color| {
            jade_xlsx_layout::color::rgba(color)
                .filter(|rgba| rgba[3] == 255)
                .map(|rgba| format!("{:02X}{:02X}{:02X}", rgba[0], rgba[1], rgba[2]))
        };
        let fill = format.and_then(|format| styles.fill(format));
        let fill_color = match fill.and_then(|fill| fill.pattern.as_deref()) {
            None | Some("none") => Some(String::new()),
            Some("solid") => fill.and_then(|fill| fill.foreground).and_then(color),
            _ => None,
        };
        Ok(FormatChange {
            bold: Some(font.is_some_and(|font| font.bold)),
            italic: Some(font.is_some_and(|font| font.italic)),
            underline: Some(font.is_some_and(|font| font.underline)),
            strike: Some(font.is_some_and(|font| font.strike)),
            font_name: font.and_then(|font| font.name.clone()),
            font_size: font.and_then(|font| font.size_points),
            font_color: font.and_then(|font| font.color).and_then(color),
            fill_color,
            number_format: match format {
                None => Some("General".into()),
                Some(format) if format.number_format_id.is_none() => Some("General".into()),
                Some(format) => styles.number_format_code(format).map(str::to_owned),
            },
            horizontal: Some(alignment.horizontal),
            vertical: Some(alignment.vertical),
            wrap_text: Some(alignment.wrap_text),
            ..FormatChange::default()
        })
    }

    /// Changes how the cells of `range` look — bold, a fill, a number
    /// format, alignment — keeping what each cell has that the change does
    /// not name. Each distinct look in the range becomes one `cellXfs`
    /// entry, reused when the file already has it, so pressing bold twice
    /// adds nothing the second time. Values and formulas stay. One write,
    /// one undo step.
    pub fn set_format(
        &mut self,
        sheet: &str,
        range: &str,
        change: &FormatChange,
    ) -> Result<SetFormatSummary> {
        let index = self.sheet_index(sheet)?;
        let range = parse_format_range(range)?;
        if change.is_empty() {
            return Err(HeadlessError::EmptyFormatChange);
        }

        let full_columns = range.start.column == 0
            && range.end.column == jade_xlsx_reader::model::MAX_COLUMN_COUNT - 1;
        let full_rows =
            range.start.row == 0 && range.end.row == jade_xlsx_reader::model::MAX_ROW_COUNT - 1;
        if full_columns || full_rows {
            let before = self.workbook.styles.cell_formats.len();
            let result = jade_xlsx_writer::format_axes(
                &self.source,
                &self.workbook,
                &self.workbook.sheets[index].name,
                range,
                change,
            )?;
            let cells = result.cells.into_iter().map(format_reference).collect();
            self.workbook = jade_xlsx_reader::read_xlsx(&result.bytes)?;
            let before_bytes = std::mem::replace(&mut self.source, result.bytes);
            self.remember(before_bytes);
            return Ok(SetFormatSummary {
                sheet: self.workbook.sheets[index].name.clone(),
                range: format_range(range),
                cells,
                styles_added: self
                    .workbook
                    .styles
                    .cell_formats
                    .len()
                    .saturating_sub(before),
            });
        }
        ensure_dense_result_fits(
            range.end.column - range.start.column + 1,
            range.end.row - range.start.row + 1,
        )?;

        let (part_name, mut styles_xml) = jade_xlsx_writer::styles_part(&self.source)?;
        let sheet_model = &self.workbook.sheets[index];
        let sheet_name = sheet_model.name.clone();
        let before = self.workbook.styles.cell_formats.len();
        let mut derived: BTreeMap<u32, u32> = BTreeMap::new();
        let mut edits = Vec::new();
        for row in range.start.row..=range.end.row {
            for column in range.start.column..=range.end.column {
                let at = CellRef::new(column, row);
                let base = effective_style(sheet_model, at);
                let style_index = match derived.get(&base) {
                    Some(index) => *index,
                    None => {
                        let (next, index) =
                            jade_xlsx_writer::add_cell_format(&styles_xml, Some(base), change)?;
                        styles_xml = next;
                        derived.insert(base, index);
                        index
                    }
                };
                edits.push((
                    index,
                    CellEdit {
                        reference: at,
                        value: NewValue::Unchanged,
                        style_index: Some(style_index),
                    },
                ));
            }
        }
        let cells = edits
            .iter()
            .map(|(_, edit)| format_reference(edit.reference))
            .collect();
        self.write_edits_with_styles(edits, Some((part_name, styles_xml)))?;
        Ok(SetFormatSummary {
            sheet: sheet_name,
            range: format_range(range),
            cells,
            styles_added: self.workbook.styles.cell_formats.len() - before,
        })
    }

    /// Inserts `count` rows before row `before_row` (1-based, as Excel
    /// numbers them). Everything below moves down — cells, merges,
    /// validations — and every formula in the workbook that pointed below
    /// follows, as do the defined names. One undo step.
    pub fn insert_rows(
        &mut self,
        sheet: &str,
        before_row: u32,
        count: u32,
    ) -> Result<StructureSummary> {
        let at = before_row
            .checked_sub(1)
            .ok_or_else(|| HeadlessError::BadReference(before_row.to_string()))?;
        self.restructure(sheet, Axis::Rows, at, count, true)
    }

    /// Deletes `count` rows starting at row `first_row` (1-based). Ranges
    /// that covered them shrink; references into them become `#REF!`.
    pub fn delete_rows(
        &mut self,
        sheet: &str,
        first_row: u32,
        count: u32,
    ) -> Result<StructureSummary> {
        let at = first_row
            .checked_sub(1)
            .ok_or_else(|| HeadlessError::BadReference(first_row.to_string()))?;
        self.restructure(sheet, Axis::Rows, at, count, false)
    }

    /// Inserts `count` columns before column `before_column` (`C`).
    pub fn insert_columns(
        &mut self,
        sheet: &str,
        before_column: &str,
        count: u32,
    ) -> Result<StructureSummary> {
        let at = parse_column(before_column)?;
        self.restructure(sheet, Axis::Columns, at, count, true)
    }

    /// Deletes `count` columns starting at column `first_column` (`C`).
    pub fn delete_columns(
        &mut self,
        sheet: &str,
        first_column: &str,
        count: u32,
    ) -> Result<StructureSummary> {
        let at = parse_column(first_column)?;
        self.restructure(sheet, Axis::Columns, at, count, false)
    }

    fn restructure(
        &mut self,
        sheet: &str,
        axis: Axis,
        at: u32,
        count: u32,
        inserted: bool,
    ) -> Result<StructureSummary> {
        let index = self.sheet_index(sheet)?;
        let change = StructureChange {
            sheet: self.workbook.sheets[index].name.clone(),
            axis,
            at,
            count,
            inserted,
        };
        let written = jade_xlsx_writer::restructure(&self.source, &self.workbook, &change)?;
        self.workbook = jade_xlsx_reader::read_xlsx(&written)?;
        let before = std::mem::replace(&mut self.source, written);
        self.remember(before);
        if axis == Axis::Rows {
            if let Some(state) = self.filter_row_state.get_mut(sheet) {
                state.manual = state
                    .manual
                    .iter()
                    .filter_map(|row| change.map_index(*row))
                    .collect();
                state.filtered = state
                    .filtered
                    .iter()
                    .filter_map(|row| change.map_index(*row))
                    .collect();
            }
        }
        self.persist_filter_row_state(sheet)?;
        self.recalculate_after_edit();
        Ok(StructureSummary {
            sheet: change.sheet,
            axis: match axis {
                Axis::Rows => "rows".to_string(),
                Axis::Columns => "columns".to_string(),
            },
            at: match axis {
                Axis::Rows => (at + 1).to_string(),
                Axis::Columns => column_label(at),
            },
            count,
            inserted,
        })
    }

    /// Recalculates every formula and refreshes the cached values in the file.
    ///
    /// Formulas the engine cannot evaluate keep their cached values and are
    /// reported in the summary. When nothing was skipped, the file's
    /// `fullCalcOnLoad` request is withdrawn — every cache is now current —
    /// and `formula_state` reads `cached` again.
    pub fn recalculate(&mut self) -> Result<RecalculationSummary> {
        let manual = self
            .filter_row_state
            .iter()
            .map(|(sheet, state)| {
                (
                    sheet.clone(),
                    state.manual.difference(&state.filtered).copied().collect(),
                )
            })
            .collect();
        let outcome = jade_xlsx_calc::evaluate_with_manual_filter_rows(
            &self.workbook,
            &self.calculation_time_zone,
            &manual,
        )?;

        let mut grouped: BTreeMap<usize, Vec<CachedValueEdit>> = BTreeMap::new();
        let mut verified = jade_xlsx_writer::VerifiedChartCells::new();
        let mut unchanged = 0usize;
        for cell in &outcome.cells {
            let value = match &cell.value {
                CellValue::Number(number) => CachedValue::Number(*number),
                CellValue::Text(text) => CachedValue::Text(text.clone()),
                CellValue::Bool(flag) => CachedValue::Bool(*flag),
                CellValue::Error(error) => CachedValue::Error(error.clone()),
                // An empty result has no `<v>` spelling; leave the cell as is.
                CellValue::Empty => continue,
            };
            verified
                .entry(self.workbook.sheets[cell.sheet].part_name.clone())
                .or_default()
                .insert(cell.reference);
            if self.cached_value(cell.sheet, cell.reference) == cell.value {
                unchanged += 1;
                continue;
            }
            grouped
                .entry(cell.sheet)
                .or_default()
                .push(CachedValueEdit {
                    reference: cell.reference,
                    value,
                });
        }

        let complete = outcome.skipped.is_empty();
        let request = !complete && !self.workbook.full_calculation_on_load;
        let refreshed = grouped.values().map(Vec::len).sum();
        let withdraw = complete && self.workbook.full_calculation_on_load;
        let charts = self
            .workbook
            .sheets
            .iter()
            .any(|sheet| sheet.shapes.iter().any(|shape| shape.chart.is_some()));
        if refreshed > 0 || withdraw || request || (charts && !verified.is_empty()) {
            let sheets: Vec<SheetCachedValues> = grouped
                .into_iter()
                .map(|(index, cells)| SheetCachedValues {
                    part_name: self.workbook.sheets[index].part_name.clone(),
                    cells,
                })
                .collect();
            let mut written = jade_xlsx_writer::write_cached_values_verified(
                &self.source,
                &sheets,
                withdraw,
                &verified,
            )?;
            if request {
                written = jade_xlsx_writer::mark_for_recalculation(&written)?;
            }
            let workbook = jade_xlsx_reader::read_xlsx(&written)?;
            if self.automatic_height_base.is_none() {
                self.automatic_height_base = Some(self.source.clone());
            }
            self.workbook = workbook;
            self.source = written;
        }

        self.source_time_zone = self.calculation_time_zone.clone();
        Ok(RecalculationSummary {
            refreshed,
            unchanged,
            skipped: outcome
                .skipped
                .into_iter()
                .map(|cell| SkippedFormula {
                    sheet: self.workbook.sheets[cell.sheet].name.clone(),
                    reference: format_reference(cell.reference),
                    reason: cell.reason,
                })
                .collect(),
        })
    }

    fn cached_value(&self, sheet: usize, reference: CellRef) -> CellValue {
        self.workbook.sheets[sheet]
            .rows
            .iter()
            .find(|row| row.index == reference.row)
            .and_then(|row| row.cells.iter().find(|cell| cell.reference == reference))
            .map(|cell| cell.value.clone())
            .unwrap_or(CellValue::Empty)
    }

    /// The parsed workbook, for a caller that needs more than this API offers.
    pub fn workbook(&self) -> &Workbook {
        &self.workbook
    }

    pub fn formula_state(&self, sheet_index: usize, reference: CellRef) -> Option<FormulaState> {
        let sheet = self.workbook.sheets.get(sheet_index)?;
        let row = sheet.rows.iter().find(|row| row.index == reference.row)?;
        formula_cache_state(
            find_cell(row, reference.column)?,
            self.calculation_is_stale(),
        )
    }

    fn calculation_is_stale(&self) -> bool {
        self.workbook.full_calculation_on_load
            || self.source_time_zone != self.calculation_time_zone
    }

    pub fn calculation_status(&self) -> jade_xlsx_headless_api::CalculationStatus {
        let mut status = jade_xlsx_headless_api::CalculationStatus {
            calculation_mode: self.workbook.calculation_mode.as_str().to_string(),
            calculate_on_save: self.workbook.calculate_on_save,
            date_system: match self.workbook.date_system {
                jade_xlsx_reader::DateSystem::Nineteen00 => 1900,
                jade_xlsx_reader::DateSystem::Nineteen04 => 1904,
            },
            formula_count: 0,
            cached: 0,
            stale: 0,
            uncalculated: 0,
            requires_recalculation: false,
        };
        for cell in self
            .workbook
            .sheets
            .iter()
            .flat_map(|sheet| &sheet.rows)
            .flat_map(|row| &row.cells)
        {
            match formula_cache_state(cell, self.calculation_is_stale()) {
                Some(FormulaState::Cached) => status.cached += 1,
                Some(FormulaState::Stale) => status.stale += 1,
                Some(FormulaState::Uncalculated) => status.uncalculated += 1,
                None => continue,
            }
            status.formula_count += 1;
        }
        status.requires_recalculation = status.stale > 0 || status.uncalculated > 0;
        status
    }

    pub fn chart_status(&self) -> Vec<jade_xlsx_layout::chart::ChartStatus> {
        jade_xlsx_layout::chart::chart_status(&self.workbook)
    }

    pub fn auto_filters(&self, sheet: &str) -> Result<Vec<jade_xlsx_headless_api::AutoFilterView>> {
        use jade_xlsx_headless_api::{AutoFilterView, FilterColumnView, FilterCriterionView};
        fn criterion(node: &jade_xlsx_reader::auto_filter::FilterCriterion) -> FilterCriterionView {
            FilterCriterionView {
                kind: node.kind.clone(),
                attributes: node.attributes.clone(),
                children: node.children.iter().map(criterion).collect(),
            }
        }
        let sheet = self.sheet(sheet)?;
        let mut views = vec![];
        let mut add = |filter: &jade_xlsx_reader::auto_filter::AutoFilter,
                       owner: String,
                       target: jade_xlsx_headless_api::AutoFilterTarget,
                       inferred: Option<CellRange>| {
            let reference = filter.reference.or(inferred.filter(|_| {
                filter
                    .issues
                    .iter()
                    .all(|issue| issue == "missing filter range")
            }));
            views.push(AutoFilterView {
                target,
                owner,
                reference: reference.map(format_range),
                has_criteria: filter.has_criteria(),
                filter_mode: sheet.filter_mode,
                columns: filter
                    .columns
                    .iter()
                    .map(|column| FilterColumnView {
                        column_id: column.column_id,
                        hidden_button: column.hidden_button,
                        show_button: column.show_button,
                        criterion: column.criterion.as_ref().map(criterion),
                    })
                    .collect(),
                issues: filter
                    .issues
                    .iter()
                    .filter(|issue| {
                        !(reference.is_some() && issue.as_str() == "missing filter range")
                    })
                    .cloned()
                    .collect(),
            });
        };
        if let Some(filter) = &sheet.auto_filter {
            add(
                filter,
                "worksheet".into(),
                jade_xlsx_headless_api::AutoFilterTarget::Worksheet,
                None,
            );
        }
        for table in &sheet.tables {
            if let Some(filter) = &table.filter {
                let mut reference = table.reference;
                if let Some(total) = table.totals_row() {
                    reference.end.row = total.saturating_sub(1);
                }
                add(
                    filter,
                    table.name.clone(),
                    jade_xlsx_headless_api::AutoFilterTarget::Table {
                        name: table.name.clone(),
                    },
                    Some(reference),
                );
            }
        }
        if sheet.filtered_row_ranges().1 {
            if views.is_empty() {
                views.push(AutoFilterView {
                    target: jade_xlsx_headless_api::AutoFilterTarget::Worksheet,
                    owner: "worksheet".into(),
                    reference: None,
                    has_criteria: true,
                    filter_mode: sheet.filter_mode,
                    columns: vec![],
                    issues: vec![],
                });
            }
            for view in &mut views {
                view.issues.push("filtered row range is unavailable".into());
            }
        }
        if sheet
            .filter_row_state
            .as_ref()
            .is_some_and(|state| state.invalid)
        {
            for view in &mut views {
                view.issues
                    .push("saved row visibility snapshot is invalid".into());
            }
        }
        Ok(views)
    }

    pub fn conditional_formatting_status(
        &self,
    ) -> jade_xlsx_headless_api::ConditionalFormattingStatus {
        let mut status = jade_xlsx_headless_api::ConditionalFormattingStatus {
            rule_count: 0,
            supported_rules: 0,
            unsupported: Vec::new(),
        };
        for (sheet_index, sheet) in self.workbook.sheets.iter().enumerate() {
            let conditional = jade_xlsx_layout::conditional_format::ConditionalStyles::new(
                sheet,
                &self.workbook.styles,
            );
            for format in &sheet.conditional_formats {
                for rule in &format.rules {
                    status.rule_count += 1;
                    if let Some(reason) = conditional.issue(rule) {
                        status.unsupported.push(
                            jade_xlsx_headless_api::ConditionalFormattingIssue {
                                sheet: sheet.name.clone(),
                                sheet_index,
                                ranges: format
                                    .ranges
                                    .iter()
                                    .map(|range| {
                                        if range.start == range.end {
                                            format_reference(range.start)
                                        } else {
                                            format_range(*range)
                                        }
                                    })
                                    .collect(),
                                rule_type: rule.rule_type.clone(),
                                priority: rule.priority,
                                reason,
                            },
                        );
                    } else {
                        status.supported_rules += 1;
                    }
                }
            }
        }
        status
    }

    pub fn summary(&self) -> WorkbookSummary {
        WorkbookSummary {
            sheets: self
                .workbook
                .sheets
                .iter()
                .enumerate()
                .map(|(index, sheet)| SheetSummary {
                    index,
                    name: sheet.name.clone(),
                    visibility: match sheet.visibility {
                        jade_xlsx_reader::SheetVisibility::Visible => SheetVisibility::Visible,
                        jade_xlsx_reader::SheetVisibility::Hidden => SheetVisibility::Hidden,
                        jade_xlsx_reader::SheetVisibility::VeryHidden => {
                            SheetVisibility::VeryHidden
                        }
                    },
                    used_columns: sheet.used_column_count(),
                    used_rows: sheet.used_row_count(),
                    used_range: used_range(sheet),
                    merges: sheet
                        .merges
                        .iter()
                        .map(|range| format_range(*range))
                        .collect(),
                    show_grid_lines: sheet.show_grid_lines,
                    freeze_pane: sheet.freeze_pane.as_ref().map(|pane| FreezePaneSummary {
                        frozen_columns: pane.frozen_columns,
                        frozen_rows: pane.frozen_rows,
                        top_left_cell: pane.top_left_cell.map(format_reference),
                    }),
                    tables: sheet
                        .tables
                        .iter()
                        .map(|table| TableSummary {
                            name: table.name.clone(),
                            display_name: table.display_name.clone(),
                            reference: format_range(table.reference),
                            header_row: table.header_row().map(|row| row + 1),
                            totals_row: table.totals_row().map(|row| row + 1),
                            columns: table.columns.clone(),
                            style: table.style_name.clone(),
                            auto_filter: table.auto_filter,
                        })
                        .collect(),
                    header_footer: sheet.header_footer.as_ref().map(header_footer_view),
                    page_breaks: (!sheet.row_breaks.is_empty() || !sheet.column_breaks.is_empty())
                        .then(|| PageBreaksView {
                            rows: sheet.row_breaks.iter().map(|brk| brk.before + 1).collect(),
                            columns: sheet
                                .column_breaks
                                .iter()
                                .map(|brk| column_label(brk.before))
                                .collect(),
                        }),
                    shapes: sheet.shapes.iter().map(shape_view).collect(),
                    notes: sheet
                        .comments
                        .iter()
                        .map(|comment| SheetNoteView {
                            reference: format_reference(comment.reference),
                            author: comment.author.clone(),
                            text: comment.text.clone(),
                        })
                        .collect(),
                })
                .collect(),
            defined_names: self
                .workbook
                .defined_names
                .iter()
                .map(|name| DefinedNameSummary {
                    name: name.name.clone(),
                    formula: formula_text::to_display_formula(&name.formula),
                    sheet_index: name.local_sheet_index,
                    sheet: name
                        .local_sheet_index
                        .and_then(|index| self.workbook.sheets.get(index))
                        .map(|sheet| sheet.name.clone()),
                    hidden: name.hidden,
                })
                .collect(),
            date_system: match self.workbook.date_system {
                DateSystem::Nineteen00 => 1900,
                DateSystem::Nineteen04 => 1904,
            },
            active_sheet_index: self.workbook.active_sheet_index,
            full_calculation_on_load: self.workbook.full_calculation_on_load,
        }
    }

    /// Resolves a sheet by name, or by index when the selector is a number.
    ///
    /// A workbook may legitimately have a sheet called `2`, so the name is
    /// tried first and the index is the fallback.
    pub fn sheet_index(&self, selector: &str) -> Result<usize> {
        if let Some(index) = self
            .workbook
            .sheets
            .iter()
            .position(|sheet| sheet.name == selector)
        {
            return Ok(index);
        }
        selector
            .parse::<usize>()
            .ok()
            .filter(|index| *index < self.workbook.sheets.len())
            .ok_or_else(|| HeadlessError::NoSuchSheet(selector.to_string()))
    }

    fn sheet(&self, selector: &str) -> Result<&Sheet> {
        Ok(&self.workbook.sheets[self.sheet_index(selector)?])
    }

    /// One cell.
    pub fn cell(&self, sheet: &str, reference: &str) -> Result<CellView> {
        let sheet = self.sheet(sheet)?;
        let reference = CellRef::parse(reference)
            .map_err(|_| HeadlessError::BadReference(reference.to_string()))?;
        Ok(self.view(sheet, reference))
    }

    /// A rectangle of cells, in row-major order, including the ones the file
    /// does not write.
    pub fn range(&self, sheet: &str, reference: &str) -> Result<RangeView> {
        let sheet = self.sheet(sheet)?;
        let range = CellRange::parse(reference)
            .map_err(|_| HeadlessError::BadReference(reference.to_string()))?;
        let (start, end) = normalise(range);

        ensure_dense_result_fits(end.column - start.column + 1, end.row - start.row + 1)?;

        let rows = (start.row..=end.row)
            .map(|row| {
                let source_row = find_row(sheet, row);
                (start.column..=end.column)
                    .map(|column| self.view_in_row(sheet, source_row, CellRef::new(column, row)))
                    .collect()
            })
            .collect();
        Ok(RangeView {
            sheet: sheet.name.clone(),
            reference: format_range(CellRange { start, end }),
            rows,
        })
    }

    /// The whole used range as display text, which is what reading a sheet
    /// usually means.
    pub fn sheet_text(&self, sheet: &str) -> Result<SheetText> {
        let sheet = self.sheet(sheet)?;
        let Some((start, end)) = used_bounds(sheet) else {
            return Ok(SheetText {
                sheet: sheet.name.clone(),
                reference: None,
                rows: Vec::new(),
            });
        };
        let columns = end.column - start.column + 1;
        let row_count = end.row - start.row + 1;
        ensure_dense_result_fits(columns, row_count)?;
        let rows = (start.row..=end.row)
            .map(|row| {
                let source_row = find_row(sheet, row);
                (start.column..=end.column)
                    .map(|column| self.display_in_row(sheet, source_row, CellRef::new(column, row)))
                    .collect()
            })
            .collect();
        Ok(SheetText {
            sheet: sheet.name.clone(),
            reference: Some(format_range(CellRange { start, end })),
            rows,
        })
    }

    /// Every cell whose display text or formula source contains `needle`.
    ///
    /// Searching the display text rather than the stored value is what makes a
    /// date findable by the date it shows. Formula source is searched as well
    /// so a headless caller can locate calculations by function or reference.
    pub fn find(&self, needle: &str, case_sensitive: bool) -> Vec<FindHit> {
        let target = if case_sensitive {
            needle.to_string()
        } else {
            needle.to_lowercase()
        };
        let mut hits = Vec::new();
        for (index, sheet) in self.workbook.sheets.iter().enumerate() {
            for row in &sheet.rows {
                for cell in &row.cells {
                    let display = self.display_of(sheet, row, cell);
                    let display_haystack = if case_sensitive {
                        display.clone()
                    } else {
                        display.to_lowercase()
                    };
                    let formula_matches = cell.formula.as_ref().is_some_and(|formula| {
                        if case_sensitive {
                            formula.contains(&target)
                        } else {
                            formula.to_lowercase().contains(&target)
                        }
                    });
                    if !target.is_empty() && (display_haystack.contains(&target) || formula_matches)
                    {
                        hits.push(FindHit {
                            sheet: sheet.name.clone(),
                            sheet_index: index,
                            reference: format_reference(cell.reference),
                            display,
                            formula: cell
                                .formula
                                .as_deref()
                                .map(formula_text::to_display_formula),
                        });
                    }
                }
            }
        }
        hits
    }

    fn view(&self, sheet: &Sheet, reference: CellRef) -> CellView {
        let row = find_row(sheet, reference.row);
        self.view_in_row(sheet, row, reference)
    }

    fn view_in_row(
        &self,
        sheet: &Sheet,
        row: Option<&jade_xlsx_reader::Row>,
        reference: CellRef,
    ) -> CellView {
        let cell = row.and_then(|row| find_cell(row, reference.column));
        let style_index = sheet.effective_style_index_in_row(
            reference,
            cell.and_then(|cell| cell.style_index),
            row,
        );
        let format = self.workbook.styles.cell_format(style_index);
        let value = cell.map(|cell| &cell.value).unwrap_or(&CellValue::Empty);
        let conditional_style = jade_xlsx_layout::conditional_format::ConditionalStyles::new(
            sheet,
            &self.workbook.styles,
        )
        .for_cell(reference, value);
        let number_format = conditional_style.number_format_code.or_else(|| {
            format
                .and_then(|format| self.workbook.styles.number_format_code(format))
                .map(str::to_string)
        });

        CellView {
            reference: format_reference(reference),
            row: reference.row,
            column: reference.column,
            kind: match value {
                CellValue::Empty => CellKind::Empty,
                CellValue::Number(_) => CellKind::Number,
                CellValue::Text(_) => CellKind::Text,
                CellValue::Bool(_) => CellKind::Bool,
                CellValue::Error(_) => CellKind::Error,
            },
            number: match value {
                CellValue::Number(number) => Some(*number),
                CellValue::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
                _ => None,
            },
            text: match value {
                CellValue::Text(text) | CellValue::Error(text) => Some(text.clone()),
                _ => None,
            },
            display: number_format::format_value(
                value,
                number_format.as_deref(),
                self.workbook.date_system,
            )
            .text,
            // As Excel shows it, without the file's `_xlfn.` prefixes.
            formula: cell.and_then(|cell| {
                cell.formula
                    .as_deref()
                    .map(jade_xlsx_calc::formula_text::to_display_formula)
            }),
            formula_state: cell
                .and_then(|cell| formula_cache_state(cell, self.calculation_is_stale())),
            number_format,
            hyperlink: sheet.hyperlink_at(reference).map(|link| HyperlinkView {
                target: link.target.clone(),
                location: link.location.clone(),
                display: link.display.clone(),
                tooltip: link.tooltip.clone(),
            }),
            comment: sheet.comment_at(reference).map(|comment| CommentView {
                author: comment.author.clone(),
                text: comment.text.clone(),
            }),
            data_validation: sheet.data_validation_at(reference).map(|validation| {
                DataValidationView {
                    ranges: validation
                        .ranges
                        .iter()
                        .map(|range| {
                            if range.start == range.end {
                                format_reference(range.start)
                            } else {
                                format_range(*range)
                            }
                        })
                        .collect(),
                    validation_type: validation.validation_type.clone(),
                    operator: validation.operator.clone(),
                    allow_blank: validation.allow_blank,
                    show_input_message: validation.show_input_message,
                    show_error_message: validation.show_error_message,
                    prompt_title: validation.prompt_title.clone(),
                    prompt: validation.prompt.clone(),
                    error_title: validation.error_title.clone(),
                    error: validation.error.clone(),
                    formula1: validation.formula1.clone(),
                    formula2: validation.formula2.clone(),
                }
            }),
            table: sheet.table_at(reference).map(|table| TableCellView {
                name: table.name.clone(),
                column: table.column_name_at(reference).map(str::to_string),
                header: table.header_row() == Some(reference.row),
                totals: table.totals_row() == Some(reference.row),
            }),
            border: None,
        }
    }

    fn display_in_row(
        &self,
        sheet: &Sheet,
        row: Option<&jade_xlsx_reader::Row>,
        reference: CellRef,
    ) -> String {
        let Some(row) = row else {
            return String::new();
        };
        find_cell(row, reference.column)
            .map(|cell| self.display_of(sheet, row, cell))
            .unwrap_or_default()
    }

    /// The text the shells would draw for this cell.
    fn display_of(&self, sheet: &Sheet, row: &jade_xlsx_reader::Row, cell: &Cell) -> String {
        let format = self
            .workbook
            .styles
            .cell_format(sheet.effective_style_index_in_row(
                cell.reference,
                cell.style_index,
                Some(row),
            ));
        let conditional_style = jade_xlsx_layout::conditional_format::ConditionalStyles::new(
            sheet,
            &self.workbook.styles,
        )
        .for_cell(cell.reference, &cell.value);
        number_format::format_value(
            &cell.value,
            conditional_style.number_format_code.as_deref().or_else(|| {
                format.and_then(|format| self.workbook.styles.number_format_code(format))
            }),
            self.workbook.date_system,
        )
        .text
    }
}

/// An immutable internal clipboard from one open workbook, including the
/// style table needed after Undo changes the live table's indices.
#[derive(Clone)]
pub struct ClipboardSnapshot {
    origin: usize,
    bytes: Vec<u8>,
    sheet: String,
    range: CellRange,
    cut: bool,
}

impl HeadlessWorkbook {
    pub fn copy_snapshot(&self, sheet: &str, range: &str, cut: bool) -> Result<ClipboardSnapshot> {
        let index = self.sheet_index(sheet)?;
        let range = parse_format_range(range)?;
        Ok(ClipboardSnapshot {
            origin: self.clipboard_origin,
            bytes: self.source.clone(),
            sheet: self.workbook.sheets[index].name.clone(),
            range,
            cut,
        })
    }

    pub fn paste_snapshot(
        &mut self,
        clipboard: &ClipboardSnapshot,
        sheet: &str,
        anchor: &str,
        keep_dimensions: bool,
    ) -> Result<CellRange> {
        if clipboard.origin != self.clipboard_origin {
            return Err(jade_xlsx_writer::WriteError::Clipboard(
                "the clipboard belongs to another open workbook".into(),
            )
            .into());
        }
        let at = CellRef::parse(anchor).map_err(|_| HeadlessError::BadReference(anchor.into()))?;
        let index = self.sheet_index(sheet)?;
        let request = jade_xlsx_writer::clipboard::PasteRequest {
            source_sheet: &clipboard.sheet,
            source: clipboard.range,
            target_sheet: &self.workbook.sheets[index].name,
            target: at,
            cut: clipboard.cut,
            keep_dimensions,
        };
        let (written, range) =
            jade_xlsx_writer::clipboard::paste_snapshot(&self.source, &clipboard.bytes, &request)?;
        self.workbook = jade_xlsx_reader::read_xlsx(&written)?;
        let before = std::mem::replace(&mut self.source, written);
        self.remember(before);
        self.recalculate_after_edit();
        Ok(range)
    }
}

fn ensure_dense_result_fits(columns: u32, rows: u32) -> Result<()> {
    let cells = u64::from(columns) * u64::from(rows);
    if cells > RANGE_CELL_LIMIT {
        return Err(HeadlessError::RangeTooLarge {
            cells,
            limit: RANGE_CELL_LIMIT,
        });
    }
    Ok(())
}

fn find_row(sheet: &Sheet, row: u32) -> Option<&jade_xlsx_reader::Row> {
    sheet.rows.iter().find(|candidate| candidate.index == row)
}

fn find_cell(row: &jade_xlsx_reader::Row, column: u32) -> Option<&Cell> {
    row.cells
        .iter()
        .find(|cell| cell.reference.column == column)
}

/// `A1`, the way a spreadsheet names a cell.
pub fn format_reference(reference: CellRef) -> String {
    format!("{}{}", column_label(reference.column), reference.row + 1)
}

/// `A1:D10`.
pub fn format_range(range: CellRange) -> String {
    format!(
        "{}:{}",
        format_reference(range.start),
        format_reference(range.end)
    )
}

/// A range with its corners the right way round, so `D10:A1` behaves.
/// The style a cell wears — its own, or the row's or column's default —
/// as the index to give a cell that should look the same. A cell with no
/// style at all resolves to 0, the default, so copying it clears formatting
/// the way Excel does.
pub fn effective_style(sheet: &Sheet, reference: CellRef) -> u32 {
    let row = find_row(sheet, reference.row);
    let own = row
        .and_then(|row| find_cell(row, reference.column))
        .and_then(|cell| cell.style_index);
    sheet
        .effective_style_index_in_row(reference, own, row)
        .unwrap_or(0)
}

/// `C` as a 0-based column index.
fn parse_column(letters: &str) -> Result<u32> {
    let trimmed = letters.trim();
    if trimmed.is_empty() || !trimmed.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(HeadlessError::BadReference(letters.to_string()));
    }
    CellRef::parse(&format!("{trimmed}1"))
        .map(|cell| cell.column)
        .map_err(|_| HeadlessError::BadReference(letters.to_string()))
}

/// A range reference parsed and normalised to start at its top-left.
fn parse_range(reference: &str) -> Result<CellRange> {
    let range = CellRange::parse(reference)
        .map_err(|_| HeadlessError::BadReference(reference.to_string()))?;
    let (start, end) = normalise(range);
    Ok(CellRange { start, end })
}

/// Complete-axis A1 references are accepted only by formatting, whose sparse
/// writer can represent them without allocating a dense cell rectangle.
fn parse_format_range(reference: &str) -> Result<CellRange> {
    if let Some((first, last)) = reference.trim().split_once(':') {
        let first = first.trim_start_matches('$');
        let last = last.trim_start_matches('$');
        if !first.is_empty() && !last.is_empty() {
            if first.bytes().all(|c| c.is_ascii_digit()) && last.bytes().all(|c| c.is_ascii_digit())
            {
                return parse_range(&format!("A{first}:XFD{last}"));
            }
            if first.bytes().all(|c| c.is_ascii_alphabetic())
                && last.bytes().all(|c| c.is_ascii_alphabetic())
            {
                return parse_range(&format!("{first}1:{last}1048576"));
            }
        }
    }
    parse_range(reference)
}

fn normalise(range: CellRange) -> (CellRef, CellRef) {
    (
        CellRef::new(
            range.start.column.min(range.end.column),
            range.start.row.min(range.end.row),
        ),
        CellRef::new(
            range.start.column.max(range.end.column),
            range.start.row.max(range.end.row),
        ),
    )
}

fn used_range(sheet: &Sheet) -> Option<String> {
    used_bounds(sheet).map(|(start, end)| format_range(CellRange { start, end }))
}

fn used_bounds(sheet: &Sheet) -> Option<(CellRef, CellRef)> {
    let mut cells = sheet.rows.iter().flat_map(|row| &row.cells);
    let first = cells.next()?.reference;
    Some(cells.fold((first, first), |(start, end), cell| {
        (
            CellRef::new(
                start.column.min(cell.reference.column),
                start.row.min(cell.reference.row),
            ),
            CellRef::new(
                end.column.max(cell.reference.column),
                end.row.max(cell.reference.row),
            ),
        )
    }))
}

fn header_footer_view(header_footer: &HeaderFooter) -> HeaderFooterView {
    HeaderFooterView {
        odd_header: header_footer.odd_header.clone(),
        odd_footer: header_footer.odd_footer.clone(),
        even_header: header_footer.even_header.clone(),
        even_footer: header_footer.even_footer.clone(),
        first_header: header_footer.first_header.clone(),
        first_footer: header_footer.first_footer.clone(),
        different_odd_even: header_footer.different_odd_even,
        different_first: header_footer.different_first,
        scale_with_doc: header_footer.scale_with_doc,
        align_with_margins: header_footer.align_with_margins,
    }
}

fn shape_view(shape: &Shape) -> ShapeView {
    ShapeView {
        id: shape.id,
        name: shape.name.clone(),
        kind: match shape.kind {
            ShapeKind::TextBox => ShapeKindView::TextBox,
            ShapeKind::Shape => ShapeKindView::Shape,
            ShapeKind::Picture => ShapeKindView::Picture,
            ShapeKind::Chart => ShapeKindView::Chart,
            ShapeKind::GraphicFrame => ShapeKindView::GraphicFrame,
            ShapeKind::Group => ShapeKindView::Group,
            ShapeKind::Connector => ShapeKindView::Connector,
        },
        anchor: match shape.to {
            Some(to) if to != shape.from => format_range(CellRange {
                start: shape.from,
                end: to,
            }),
            _ => format_reference(shape.from),
        },
        text: shape.text.clone(),
    }
}
