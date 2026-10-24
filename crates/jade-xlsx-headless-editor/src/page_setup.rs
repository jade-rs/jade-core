use crate::{HeadlessWorkbook, Result};
use jade_xlsx_reader::{CellRange, PageSetup};

impl HeadlessWorkbook {
    /// Changes paper, orientation, scaling, margins and repeated titles as one
    /// undo step. Printer relationships and other page attributes are retained.
    pub fn set_page_setup(&mut self, sheet: &str, setup: &PageSetup) -> Result<bool> {
        let index = self.sheet_index(sheet)?;
        if &self.workbook.sheets[index].page_setup == setup {
            return Ok(false);
        }
        let written = jade_xlsx_writer::page_setup::set_page_setup(
            &self.source,
            &self.workbook.sheets[index].name,
            setup,
        )?;
        self.replace_package(jade_opc::OpcPackage::open(&written)?)?;
        Ok(true)
    }

    pub fn set_print_area(&mut self, sheet: &str, range: Option<CellRange>) -> Result<bool> {
        let index = self.sheet_index(sheet)?;
        let written = jade_xlsx_writer::page_setup::set_print_area(
            &self.source,
            &self.workbook.sheets[index].name,
            range,
        )?;
        if written == self.source {
            return Ok(false);
        }
        self.replace_package(jade_opc::OpcPackage::open(&written)?)?;
        Ok(true)
    }
}
