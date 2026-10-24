//! Charts, pictures and shapes moved, resized and deleted, as Excel does
//! when one is dragged by its border or a handle, or selected and deleted.
//! A protected sheet refuses them unless its protection allows editing
//! objects.

pub use jade_xlsx_writer::ShapeAnchor;

use crate::{HeadlessError, HeadlessWorkbook, Result};

const WHAT: &str = "editing objects is not allowed";

impl HeadlessWorkbook {
    /// The sheet's drawing part: its name and text.
    pub(crate) fn drawing_part(&self, index: usize) -> Result<Option<(String, String)>> {
        let sheet = &self.workbook.sheets[index];
        let package = jade_opc::OpcPackage::open(&self.source)?;
        let rels_name = jade_opc::rels_part_name(&sheet.part_name);
        let Some(id) = jade_xlsx_reader::object_protection::worksheet_drawing_relationship(
            &package.required_part_str(&sheet.part_name)?,
            false,
        )?
        else {
            return Ok(None);
        };
        let Some(name) = package
            .part_str(&rels_name)?
            .map(|xml| jade_xlsx_reader::object_protection::owned_relationships(&xml))
            .transpose()?
            .and_then(|rels| {
                rels.relationship(&id)
                    .filter(|r| Self::owned_relationship(r, "drawing"))
                    .map(|r| jade_opc::resolve_target(&sheet.part_name, &r.target))
            })
        else {
            return Ok(None);
        };
        let xml = package
            .part_str(&name)?
            .ok_or_else(|| jade_xlsx_writer::WriteError::MissingPart(name.clone()))?;
        Ok(Some((name, xml)))
    }

    /// Puts the object whose id is `id` over `anchor`: moved when its size
    /// is kept, resized when not. One undo step.
    pub fn place_shape(&mut self, sheet: &str, id: u32, anchor: &ShapeAnchor) -> Result<()> {
        let index = self.sheet_index(sheet)?;
        let (name, xml, _) = self.guarded_drawing(index, id, false, WHAT)?;
        let missing = || HeadlessError::NoSuchShape(id.to_string());
        let patched = jade_xlsx_writer::set_shape_anchor(&xml, id, anchor)?.ok_or_else(missing)?;
        self.replace_parts(vec![(name, patched)])
    }

    /// Deletes the object whose id is `id`. One undo step.
    pub fn delete_shape(&mut self, sheet: &str, id: u32) -> Result<()> {
        let index = self.sheet_index(sheet)?;
        let (name, xml, _) = self.guarded_drawing(index, id, false, WHAT)?;
        let missing = || HeadlessError::NoSuchShape(id.to_string());
        let patched = jade_xlsx_writer::delete_shape(&xml, id)?.ok_or_else(missing)?;
        self.replace_parts(vec![(name, patched)])
    }
}
