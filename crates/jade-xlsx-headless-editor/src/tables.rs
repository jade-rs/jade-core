//! Table editing uses one history snapshot for the definition, cells, related
//! package parts and recalculated dependent formulas.
use crate::{HeadlessWorkbook, Result};
use jade_xlsx_reader::CellRange;
pub use jade_xlsx_writer::tables::{
    TableOptions, TableResult, TableStyle, TableTotal, TableUpdate, TotalFunction,
};
impl HeadlessWorkbook {
    fn finish_table_edit(&mut self, result: TableResult) -> Result<String> {
        let name = result.name;
        self.replace_package(jade_opc::OpcPackage::open(&result.bytes)?)?;
        self.recalculate_after_edit();
        Ok(name)
    }
    pub fn convert_table_to_range(&mut self, sheet: &str, table: &str) -> Result<()> {
        self.convert_table_with_update(sheet, table, &Default::default())
    }
    pub fn convert_table_with_update(
        &mut self,
        sheet: &str,
        table: &str,
        update: &TableUpdate,
    ) -> Result<()> {
        let index = self.sheet_index(sheet)?;
        self.guard(index, "editing tables is not allowed", |_| false)?;
        let normalized = normalize(update);
        let edited = jade_xlsx_writer::tables::update_table(
            &self.source,
            &self.workbook.sheets[index].name,
            table,
            &normalized,
        )?;
        let bytes = jade_xlsx_writer::tables::convert_to_range(
            &edited.bytes,
            &self.workbook.sheets[index].name,
            &edited.name,
        )?;
        self.replace_package(jade_opc::OpcPackage::open(&bytes)?)?;
        self.recalculate_after_edit();
        Ok(())
    }
    pub fn create_table(
        &mut self,
        sheet: &str,
        reference: CellRange,
        options: &TableOptions,
    ) -> Result<String> {
        let index = self.sheet_index(sheet)?;
        self.guard(index, "editing tables is not allowed", |_| false)?;
        let sheet = &self.workbook.sheets[index].name;
        let result =
            jade_xlsx_writer::tables::create_table(&self.source, sheet, reference, options)?;
        self.finish_table_edit(result)
    }
    pub fn update_table(
        &mut self,
        sheet: &str,
        table: &str,
        update: &TableUpdate,
    ) -> Result<String> {
        let index = self.sheet_index(sheet)?;
        self.guard(index, "editing tables is not allowed", |_| false)?;
        let normalized = normalize(update);
        let sheet = &self.workbook.sheets[index].name;
        let result =
            jade_xlsx_writer::tables::update_table(&self.source, sheet, table, &normalized)?;
        self.finish_table_edit(result)
    }
}

fn normalize(update: &TableUpdate) -> TableUpdate {
    let mut normalized = update.clone();
    for (_, f) in &mut normalized.calculated_columns {
        *f = f
            .as_deref()
            .map(|s| crate::formula_text::to_file_formula(s.strip_prefix('=').unwrap_or(s)));
    }
    for (_, setting) in &mut normalized.totals {
        if let TableTotal::Formula(f) = setting {
            *f = crate::formula_text::to_file_formula(f.strip_prefix('=').unwrap_or(f));
        }
    }
    normalized
}
