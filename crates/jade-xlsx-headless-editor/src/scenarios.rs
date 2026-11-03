//! The Scenario Manager's scenarios, saved in the sheet's `<scenarios>`.

use crate::{HeadlessWorkbook, Result};
use jade_xlsx_reader::scenarios::Scenarios;

impl HeadlessWorkbook {
    /// The sheet's scenarios replaced by `scenarios` (none, or an empty
    /// list, takes them out): one undo step, false when nothing changed.
    pub fn set_scenarios(&mut self, sheet: &str, scenarios: Option<&Scenarios>) -> Result<bool> {
        let index = self.sheet_index(sheet)?;
        let wanted = scenarios.filter(|list| !list.scenarios.is_empty());
        if self.workbook.sheets[index].scenarios.as_ref() == wanted {
            return Ok(false);
        }
        let package = jade_opc::OpcPackage::open(&self.source)?;
        let part_name = self.workbook.sheets[index].part_name.clone();
        let xml = package
            .part_str(&part_name)?
            .ok_or_else(|| jade_xlsx_writer::WriteError::MissingPart(part_name.clone()))?;
        let patched = jade_xlsx_writer::scenarios::set_scenarios(&xml, wanted)?;
        if patched == xml {
            return Ok(false);
        }
        self.replace_parts(vec![(part_name, patched)])?;
        Ok(true)
    }
}
