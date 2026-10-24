use super::*;
use jade_xlsx_writer::pivot_refresh::{PivotCreate, PivotDestination};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PivotCreationSummary {
    pub name: String,
    pub sheet: String,
    pub cache_id: u32,
    pub reference: jade_xlsx_reader::CellRange,
    pub source_rows: usize,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PivotDeletionSummary {
    pub removed_cache: bool,
    pub detached_charts: usize,
}
impl HeadlessWorkbook {
    pub fn create_pivot(&mut self, options: &PivotCreate) -> Result<PivotCreationSummary> {
        let layout = jade_xlsx_layout::LayoutOptions::default()
            .with_normal_font_metrics(&self.workbook.styles);
        self.create_pivot_with_layout(options, &layout)
    }
    pub fn create_pivot_with_layout(
        &mut self,
        options: &PivotCreate,
        layout: &jade_xlsx_layout::LayoutOptions,
    ) -> Result<PivotCreationSummary> {
        if let PivotDestination::Existing { sheet, .. } = &options.destination {
            let index = self.sheet_index(sheet)?;
            self.guard(index, "creating pivots is not allowed", |p| p.pivot_tables)?;
        }
        let checkpoint = self.edit_checkpoint();
        let prepared = (|| -> Result<_> {
            let verified = self.verified_pivot_sources()?;
            let mut result = pivot_refresh::create_pivot(&self.source, options, &verified)?;
            // Fitting permissions and sheet indices must use the staged graph,
            // including a newly created destination worksheet.
            self.workbook = jade_xlsx_reader::read_xlsx(&result.bytes)?;
            self.source = result.bytes.clone();
            let mut work = 0;
            result.bytes = self.fit_pivot_widths(
                &result.bytes,
                &[result.cache_id].into_iter().collect(),
                layout,
                &mut work,
            )?;
            Ok(result)
        })();
        let restored = self.restore_edit_checkpoint(checkpoint);
        debug_assert!(restored);
        let result = prepared?;
        let summary = PivotCreationSummary {
            name: result.name,
            sheet: result.sheet,
            cache_id: result.cache_id,
            reference: result.reference,
            source_rows: result.source_rows,
        };
        self.replace_package(jade_opc::OpcPackage::open(&result.bytes)?)?;
        self.recalculate_after_edit();
        Ok(summary)
    }
    pub fn delete_pivot(&mut self, sheet: &str, name: &str) -> Result<PivotDeletionSummary> {
        let index = self.sheet_index(sheet)?;
        self.guard(index, "removing pivots is not allowed", |p| p.pivot_tables)?;
        let result =
            pivot_refresh::delete_pivot(&self.source, &self.workbook.sheets[index].name, name)?;
        let summary = PivotDeletionSummary {
            removed_cache: result.removed_cache,
            detached_charts: result.detached_charts,
        };
        self.replace_package(jade_opc::OpcPackage::open(&result.bytes)?)?;
        self.recalculate_after_edit();
        Ok(summary)
    }
}
