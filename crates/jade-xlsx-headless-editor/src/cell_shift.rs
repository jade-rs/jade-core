use crate::{HeadlessWorkbook, Result};
use jade_xlsx_reader::{Axis, CellRange, CellShift};

impl HeadlessWorkbook {
    /// Insert/delete only the selection's column strip (rows axis) or row
    /// strip (columns axis), as the Insert/Delete Cells dialog does.
    pub fn shift_cells(
        &mut self,
        sheet: &str,
        range: CellRange,
        axis: Axis,
        inserted: bool,
    ) -> Result<()> {
        let index = self.sheet_index(sheet)?;
        self.guard(
            index,
            "shifting cells on a protected sheet is not allowed",
            |_| false,
        )?;
        let change = CellShift {
            sheet: self.workbook.sheets[index].name.clone(),
            range,
            axis,
            inserted,
        };
        let bytes =
            jade_xlsx_writer::cell_shift::shift_cells(&self.source, &self.workbook, &change)?;
        self.replace_package(jade_opc::OpcPackage::open(&bytes)?)?;
        self.recalculate_after_edit();
        Ok(())
    }
}
