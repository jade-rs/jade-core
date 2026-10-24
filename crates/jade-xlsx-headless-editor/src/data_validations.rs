//! Authoring and clearing worksheet input rules as single undo transactions.
use crate::{HeadlessWorkbook, Result};
use jade_xlsx_reader::{CellRange, DataValidation};

impl HeadlessWorkbook {
    /// Removes validation only in these selections, with one undo snapshot.
    pub fn clear_data_validation_ranges(
        &mut self,
        sheet: &str,
        ranges: &[CellRange],
    ) -> Result<bool> {
        let sheet_index = self.sheet_index(sheet)?;
        self.guard(
            sheet_index,
            "editing data validation is not allowed",
            |_| false,
        )?;
        let part = self.workbook.sheets[sheet_index].part_name.clone();
        let package = jade_opc::OpcPackage::open(&self.source)?;
        let original = package.required_part_str(&part)?;
        let mut updated = original.clone();
        for range in ranges {
            updated =
                jade_xlsx_writer::data_validations::clear_data_validations(&updated, Some(*range))?;
        }
        if updated == original {
            return Ok(false);
        }
        self.replace_parts(vec![(part, updated)])?;
        Ok(true)
    }
    pub fn set_data_validation(
        &mut self,
        sheet: &str,
        index: Option<usize>,
        rule: &DataValidation,
    ) -> Result<usize> {
        let sheet_index = self.sheet_index(sheet)?;
        self.guard(
            sheet_index,
            "editing data validation is not allowed",
            |_| false,
        )?;
        let mut normalized = rule.clone();
        normalized.formula1 = rule
            .formula1
            .as_deref()
            .map(|f| crate::formula_text::to_file_formula(f.strip_prefix('=').unwrap_or(f)));
        normalized.formula2 = rule
            .formula2
            .as_deref()
            .map(|f| crate::formula_text::to_file_formula(f.strip_prefix('=').unwrap_or(f)));
        let at = index.unwrap_or(usize::MAX);
        if index.is_some()
            && self.workbook.sheets[sheet_index].data_validations.get(at) == Some(&normalized)
        {
            return Ok(at);
        }
        let part = self.workbook.sheets[sheet_index].part_name.clone();
        let package = jade_opc::OpcPackage::open(&self.source)?;
        let xml = package.required_part_str(&part)?;
        let updated =
            jade_xlsx_writer::data_validations::set_data_validation(&xml, index, &normalized)?;
        let at = match index {
            Some(index) => index,
            None => jade_xlsx_writer::data_validations::data_validation_insertion_index(&xml)?,
        };
        self.replace_parts(vec![(part, updated)])?;
        Ok(at)
    }
    pub fn delete_data_validation(&mut self, sheet: &str, index: usize) -> Result<()> {
        let sheet_index = self.sheet_index(sheet)?;
        self.guard(
            sheet_index,
            "editing data validation is not allowed",
            |_| false,
        )?;
        let part = self.workbook.sheets[sheet_index].part_name.clone();
        let package = jade_opc::OpcPackage::open(&self.source)?;
        let xml = package.required_part_str(&part)?;
        let updated = jade_xlsx_writer::data_validations::delete_data_validation(&xml, index)?;
        self.replace_parts(vec![(part, updated)])
    }
    pub fn clear_data_validations(
        &mut self,
        sheet: &str,
        range: Option<CellRange>,
    ) -> Result<bool> {
        let sheet_index = self.sheet_index(sheet)?;
        self.guard(
            sheet_index,
            "editing data validation is not allowed",
            |_| false,
        )?;
        let part = self.workbook.sheets[sheet_index].part_name.clone();
        let package = jade_opc::OpcPackage::open(&self.source)?;
        let xml = package.required_part_str(&part)?;
        let updated = jade_xlsx_writer::data_validations::clear_data_validations(&xml, range)?;
        if updated == xml {
            return Ok(false);
        }
        self.replace_parts(vec![(part, updated)])?;
        Ok(true)
    }
    /// Applies a rule over its ranges, replacing old rules only in those cells.
    pub fn apply_data_validation(&mut self, sheet: &str, rule: &DataValidation) -> Result<()> {
        let sheet_index = self.sheet_index(sheet)?;
        self.guard(
            sheet_index,
            "editing data validation is not allowed",
            |_| false,
        )?;
        jade_xlsx_writer::data_validations::validate(rule)?;
        let part = self.workbook.sheets[sheet_index].part_name.clone();
        let package = jade_opc::OpcPackage::open(&self.source)?;
        let mut xml = package.required_part_str(&part)?;
        for range in &rule.ranges {
            xml = jade_xlsx_writer::data_validations::clear_data_validations(&xml, Some(*range))?;
        }
        let mut normalized = rule.clone();
        normalized.formula1 = rule
            .formula1
            .as_deref()
            .map(|f| crate::formula_text::to_file_formula(f.strip_prefix('=').unwrap_or(f)));
        normalized.formula2 = rule
            .formula2
            .as_deref()
            .map(|f| crate::formula_text::to_file_formula(f.strip_prefix('=').unwrap_or(f)));
        let updated =
            jade_xlsx_writer::data_validations::set_data_validation(&xml, None, &normalized)?;
        self.replace_parts(vec![(part, updated)])
    }
}
