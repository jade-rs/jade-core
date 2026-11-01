//! An explicit pivot refresh is one atomic edit, including source-formula
//! evaluation, all reports sharing the cache and dependent formula results.
mod lifecycle;
mod order;
use crate::{HeadlessWorkbook, Result};
use jade_xlsx_writer::pivot_refresh::{self, VerifiedPivotCells};
pub use lifecycle::{PivotCreationSummary, PivotDeletionSummary};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PivotRefreshSummary {
    pub cache_id: u32,
    pub source_sheet: String,
    pub source_rows: usize,
    pub tables: Vec<String>,
}
impl HeadlessWorkbook {
    /// Refresh every declared worksheet pivot in observed dependency order.
    /// Shared caches are deduplicated; all work commits as one history edit.
    pub fn refresh_all_pivots(&mut self) -> Result<Vec<PivotRefreshSummary>> {
        let options = jade_xlsx_layout::LayoutOptions::default()
            .with_normal_font_metrics(&self.workbook.styles);
        self.refresh_all_pivots_with_layout(&options)
    }
    /// The app's font metrics also fit all rebuilt reports in the same edit.
    pub fn refresh_all_pivots_with_layout(
        &mut self,
        options: &jade_xlsx_layout::LayoutOptions,
    ) -> Result<Vec<PivotRefreshSummary>> {
        if !self.workbook.pivots.issues.is_empty() {
            return Err(jade_xlsx_writer::WriteError::Pivot(
                "refresh all requires an unambiguous complete pivot catalog".into(),
            )
            .into());
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut targets = Vec::new();
        for table in &self.workbook.pivots.tables {
            if !seen.insert(table.cache_id) {
                continue;
            }
            let owner = self
                .workbook
                .sheets
                .iter()
                .find(|s| s.part_name == table.sheet_part)
                .ok_or_else(|| {
                    jade_xlsx_writer::WriteError::Pivot("missing pivot report owner".into())
                })?;
            targets.push((table.cache_id, owner.name.clone(), table.name.clone()));
        }
        if targets.is_empty() {
            return Ok(Vec::new());
        }
        let checkpoint = self.edit_checkpoint();
        let prepared = (|| -> Result<_> {
            let mut summaries = std::collections::BTreeMap::new();
            let mut completed = std::collections::BTreeMap::new();
            let limit = targets
                .len()
                .saturating_mul(targets.len().saturating_add(1))
                .min(16_384);
            let mut writes = 0usize;
            let mut dependency_work = 0usize;
            let mut fitted = std::collections::BTreeSet::new();
            let mut fitting_work = 0usize;
            loop {
                let (outcome, reads) = self.recalculate_with_dependencies(true, false)?;
                let reads =
                    reads.ok_or_else(|| order::refusal("missing pivot dependency trace"))?;
                let graph = order::graph(&self.workbook, &reads, &mut dependency_work)?;
                order::invalidate(&graph, &mut completed);
                if completed.len() == targets.len() {
                    break;
                }
                if writes >= limit {
                    return Err(order::refusal(
                        "pivot source dependencies did not stabilize within the refresh budget",
                    ));
                }
                let (id, sheet, name) = targets
                    .iter()
                    .find(|(id, _, _)| {
                        !completed.contains_key(id)
                            && graph[id].iter().all(|dep| completed.contains_key(dep))
                    })
                    .ok_or_else(|| order::refusal("no dependency-ready pivot source"))?;
                let verified = self.verified_pivot_cells(outcome.skipped.is_empty());
                let mut result =
                    pivot_refresh::refresh_pivot(&self.source, sheet, name, &verified)?;
                fitted.insert(*id);
                result.bytes =
                    self.fit_pivot_widths(&result.bytes, &fitted, options, &mut fitting_work)?;
                summaries.insert(
                    *id,
                    PivotRefreshSummary {
                        cache_id: result.cache_id,
                        source_sheet: result.source_sheet,
                        source_rows: result.source_rows,
                        tables: result.tables,
                    },
                );
                self.workbook = jade_xlsx_reader::read_xlsx(&result.bytes)?;
                self.source = result.bytes;
                writes += 1;
                completed.insert(*id, writes);
            }
            let summaries = summaries.into_values().collect();
            Ok((self.source.clone(), summaries))
        })();
        let restored = self.restore_edit_checkpoint(checkpoint);
        debug_assert!(restored);
        let (bytes, summaries) = prepared?;
        self.replace_package(jade_opc::OpcPackage::open(&bytes)?)?;
        // The final traced calculation already supplied these exact caches.
        // Do not run a second volatile evaluation after validating the graph.
        self.source_time_zone = self.calculation_time_zone.clone();
        Ok(summaries)
    }
    fn verified_pivot_sources(&mut self) -> Result<VerifiedPivotCells> {
        let outcome = self.recalculate()?;
        Ok(self.verified_pivot_cells(outcome.skipped.is_empty()))
    }
    fn verified_pivot_cells(&self, complete: bool) -> VerifiedPivotCells {
        let mut verified = VerifiedPivotCells::new();
        if complete {
            for sheet in &self.workbook.sheets {
                verified.insert(
                    sheet.part_name.clone(),
                    sheet
                        .rows
                        .iter()
                        .flat_map(|r| &r.cells)
                        .filter(|c| c.formula.is_some())
                        .map(|c| c.reference)
                        .collect(),
                );
            }
        }
        verified
    }
    pub fn refresh_pivot(&mut self, sheet: &str, name: &str) -> Result<PivotRefreshSummary> {
        let options = jade_xlsx_layout::LayoutOptions::default()
            .with_normal_font_metrics(&self.workbook.styles);
        self.refresh_pivot_with_layout(sheet, name, &options)
    }
    pub fn refresh_pivot_with_layout(
        &mut self,
        sheet: &str,
        name: &str,
        options: &jade_xlsx_layout::LayoutOptions,
    ) -> Result<PivotRefreshSummary> {
        self.change_pivot_with_layout(sheet, name, None, options)
    }
    pub fn update_pivot(
        &mut self,
        sheet: &str,
        name: &str,
        update: &pivot_refresh::PivotUpdate,
    ) -> Result<PivotRefreshSummary> {
        let options = jade_xlsx_layout::LayoutOptions::default()
            .with_normal_font_metrics(&self.workbook.styles);
        self.update_pivot_with_layout(sheet, name, update, &options)
    }
    pub fn update_pivot_with_layout(
        &mut self,
        sheet: &str,
        name: &str,
        update: &pivot_refresh::PivotUpdate,
        options: &jade_xlsx_layout::LayoutOptions,
    ) -> Result<PivotRefreshSummary> {
        self.change_pivot_with_layout(sheet, name, Some(update), options)
    }
    fn change_pivot_with_layout(
        &mut self,
        sheet: &str,
        name: &str,
        update: Option<&pivot_refresh::PivotUpdate>,
        options: &jade_xlsx_layout::LayoutOptions,
    ) -> Result<PivotRefreshSummary> {
        let index = self.sheet_index(sheet)?;
        self.guard(index, "refreshing pivots is not allowed", |p| {
            p.pivot_tables
        })?;
        let owner = self.workbook.sheets[index].name.clone();
        let checkpoint = self.edit_checkpoint();
        // Evaluation may update caches/row metrics/grants. Stage it against a
        // complete checkpoint, then restore before committing one history edit.
        let prepared = (|| -> Result<_> {
            let verified = self.verified_pivot_sources()?;
            let mut result = if let Some(update) = update {
                pivot_refresh::update_pivot(&self.source, &owner, name, update, &verified)?
            } else {
                pivot_refresh::refresh_pivot(&self.source, &owner, name, &verified)?
            };
            let mut fitting_work = 0;
            result.bytes = self.fit_pivot_widths(
                &result.bytes,
                &std::collections::BTreeSet::from([result.cache_id]),
                options,
                &mut fitting_work,
            )?;
            Ok(result)
        })();
        let restored = self.restore_edit_checkpoint(checkpoint);
        debug_assert!(restored);
        let result = prepared?;
        let summary = PivotRefreshSummary {
            cache_id: result.cache_id,
            source_sheet: result.source_sheet,
            source_rows: result.source_rows,
            tables: result.tables,
        };
        self.replace_package(jade_opc::OpcPackage::open(&result.bytes)?)?;
        self.recalculate_after_edit();
        Ok(summary)
    }
    fn fit_pivot_widths(
        &self,
        bytes: &[u8],
        caches: &std::collections::BTreeSet<u32>,
        options: &jade_xlsx_layout::LayoutOptions,
        work: &mut usize,
    ) -> Result<Vec<u8>> {
        let book = jade_xlsx_reader::read_xlsx(bytes)?;
        let refuse = |s: &str| jade_xlsx_writer::WriteError::Pivot(s.into());
        for table in book
            .pivots
            .tables
            .iter()
            .filter(|t| caches.contains(&t.cache_id))
        {
            let sheet = book
                .sheets
                .iter()
                .find(|s| s.part_name == table.sheet_part)
                .ok_or_else(|| refuse("missing pivot fitting sheet"))?;
            if matches!(
                table
                    .attributes
                    .get("applyWidthHeightFormats")
                    .map(|v| v.trim()),
                Some("0" | "false")
            ) || sheet.protection.as_ref().is_some_and(|p| !p.format_columns)
            {
                continue;
            }
            *work = work.saturating_add(sheet.rows.iter().map(|r| r.cells.len()).sum::<usize>());
            if *work > 20_000_000 {
                return Err(refuse("pivot fitting exceeds batch scan budget").into());
            }
        }
        let widths = jade_xlsx_layout::pivot_fit::fit_columns(&book, caches, options)
            .map_err(jade_xlsx_writer::WriteError::Pivot)?;
        let mut bytes = bytes.to_vec();
        for (name, mut sizes) in widths {
            let sheet = book
                .sheets
                .iter()
                .find(|s| s.name == name)
                .ok_or_else(|| refuse("missing pivot fitting sheet"))?;
            // Refresh permission and column-format permission are independent.
            // Keep protected widths while still allowing an authorized refresh.
            if sheet.protection.as_ref().is_some_and(|p| !p.format_columns) {
                continue;
            }
            sizes.retain(|column, size| {
                let current = sheet
                    .column_span(*column)
                    .and_then(|c| c.width_chars)
                    .or(sheet.default_column_width_chars)
                    .unwrap_or(jade_xlsx_layout::units::DEFAULT_COLUMN_WIDTH_CHARS);
                current != *size
            });
            if sizes.is_empty() {
                continue;
            }
            self.guard_axis_format(self.sheet_index(&name)?, jade_xlsx_reader::Axis::Columns)?;
            bytes = jade_xlsx_writer::dimensions::set_axis_sizes(
                &bytes,
                &book,
                &name,
                jade_xlsx_reader::Axis::Columns,
                &sizes,
            )?;
        }
        Ok(bytes)
    }
}
