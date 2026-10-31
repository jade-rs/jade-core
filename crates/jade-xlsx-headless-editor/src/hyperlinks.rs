//! Cell hyperlinks as atomic package edits, optionally with visible link text.
use crate::{
    effective_style, ensure_dense_result_fits, CellEdit, FormatChange, HeadlessWorkbook, NewValue,
    Result,
};
use jade_xlsx_reader::{CellRange, CellRef, Hyperlink};
impl HeadlessWorkbook {
    fn guard_hyperlink(&self, index: usize, ranges: &[CellRange]) -> Result<()> {
        self.guard(index, "editing hyperlinks is not allowed", |p| {
            p.insert_hyperlinks
        })?;
        if self.workbook.sheets[index].protection.is_some() {
            for range in ranges {
                // Range guards must not walk a whole worksheet on invalid input.
                ensure_dense_result_fits(
                    range
                        .end
                        .column
                        .saturating_sub(range.start.column)
                        .saturating_add(1),
                    range
                        .end
                        .row
                        .saturating_sub(range.start.row)
                        .saturating_add(1),
                )?;
                self.guard_cells(
                    index,
                    (range.start.row..=range.end.row).flat_map(|row| {
                        (range.start.column..=range.end.column)
                            .map(move |column| CellRef::new(column, row))
                    }),
                )?;
            }
        }
        Ok(())
    }
    /// Set link metadata without changing cell text or formatting.
    pub fn set_hyperlink(&mut self, sheet: &str, link: &Hyperlink) -> Result<bool> {
        let index = self.sheet_index(sheet)?;
        self.guard_hyperlink(index, &[link.reference])?;
        if self.workbook.sheets[index]
            .hyperlinks
            .iter()
            .any(|old| old == link)
        {
            return Ok(false);
        }
        let part = self.workbook.sheets[index].part_name.clone();
        let mut package = jade_opc::OpcPackage::open(&self.source)?;
        if !jade_xlsx_writer::hyperlinks::set_hyperlink(&mut package, &part, link)? {
            return Ok(false);
        }
        self.replace_package(package)?;
        Ok(true)
    }
    /// Insert the link and its blue/underlined style. An explicit label updates
    /// the top-left non-formula cell. An empty cell uses the destination text.
    /// Formula cells keep their formula. The complete operation is one undo step.
    pub fn insert_hyperlink(
        &mut self,
        sheet: &str,
        link: &Hyperlink,
        label: Option<&str>,
    ) -> Result<()> {
        let index = self.sheet_index(sheet)?;
        self.guard_hyperlink(index, &[link.reference])?;
        let range = link.reference;
        ensure_dense_result_fits(
            range
                .end
                .column
                .saturating_sub(range.start.column)
                .saturating_add(1),
            range
                .end
                .row
                .saturating_sub(range.start.row)
                .saturating_add(1),
        )?;
        let sheet_model = &self.workbook.sheets[index];
        let part = sheet_model.part_name.clone();
        let mut package = jade_opc::OpcPackage::open(&self.source)?;
        jade_xlsx_writer::hyperlinks::set_hyperlink(&mut package, &part, link)?;
        let change = FormatChange {
            font_color: Some("0563C1".into()),
            underline: Some(true),
            ..Default::default()
        };
        let (styles_part, mut styles_xml) = jade_xlsx_writer::styles_part(&self.source)?;
        let mut styles = std::collections::BTreeMap::new();
        let mut edits = Vec::new();
        let top = sheet_model.cell_at(range.start);
        let text = if top.is_some_and(|c| c.formula.is_some()) {
            None
        } else {
            label.map(str::to_string).or_else(|| {
                top.is_none_or(|c| c.value == jade_xlsx_reader::CellValue::Empty)
                    .then(|| {
                        link.target
                            .clone()
                            .or_else(|| link.location.clone())
                            .unwrap_or_default()
                    })
            })
        };
        for row in range.start.row..=range.end.row {
            for column in range.start.column..=range.end.column {
                let at = CellRef::new(column, row);
                let base = effective_style(sheet_model, at);
                let style = match styles.get(&base) {
                    Some(index) => *index,
                    None => {
                        let (next, index) =
                            jade_xlsx_writer::add_cell_format(&styles_xml, Some(base), &change)?;
                        styles_xml = next;
                        styles.insert(base, index);
                        index
                    }
                };
                edits.push((
                    index,
                    CellEdit {
                        reference: at,
                        value: if at == range.start {
                            text.as_ref()
                                .map(|s| NewValue::Text(s.clone()))
                                .unwrap_or(NewValue::Unchanged)
                        } else {
                            NewValue::Unchanged
                        },
                        style_index: Some(style),
                    },
                ));
            }
        }
        let base = package.write()?;
        self.write_edits_with_base(
            edits,
            Some((styles_part, styles_xml)),
            Vec::new(),
            Some(&base),
        )
    }
    pub fn clear_hyperlinks(&mut self, sheet: &str, ranges: &[CellRange]) -> Result<bool> {
        let index = self.sheet_index(sheet)?;
        self.guard_hyperlink(index, ranges)?;
        let part = self.workbook.sheets[index].part_name.clone();
        let mut package = jade_opc::OpcPackage::open(&self.source)?;
        if !jade_xlsx_writer::hyperlinks::clear_hyperlinks(&mut package, &part, ranges)? {
            return Ok(false);
        }
        self.replace_package(package)?;
        Ok(true)
    }
}
