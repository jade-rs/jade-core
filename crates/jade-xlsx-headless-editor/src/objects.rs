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
        let Some(name) = package
            .part_str(&rels_name)?
            .map(|xml| jade_opc::parse_relationships(&xml))
            .transpose()?
            .and_then(|rels| {
                rels.target_by_type_suffix("/drawing")
                    .map(|target| jade_opc::resolve_target(&sheet.part_name, target))
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
        self.guard(index, WHAT, |p| p.objects)?;
        let missing = || HeadlessError::NoSuchShape(id.to_string());
        let (name, xml) = self.drawing_part(index)?.ok_or_else(missing)?;
        let patched = jade_xlsx_writer::set_shape_anchor(&xml, id, anchor)?.ok_or_else(missing)?;
        self.replace_parts(vec![(name, patched)])
    }

    /// Deletes the object whose id is `id`. One undo step.
    pub fn delete_shape(&mut self, sheet: &str, id: u32) -> Result<()> {
        let index = self.sheet_index(sheet)?;
        self.guard(index, WHAT, |p| p.objects)?;
        let missing = || HeadlessError::NoSuchShape(id.to_string());
        let (name, xml) = self.drawing_part(index)?.ok_or_else(missing)?;
        let patched = jade_xlsx_writer::delete_shape(&xml, id)?.ok_or_else(missing)?;
        self.replace_parts(vec![(name, patched)])
    }
}
