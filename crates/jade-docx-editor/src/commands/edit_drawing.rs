//! One undoable edit of a drawing object's XML: the 図の形式 / 図形の書式
//! tools — outline, effects, corrections, geometry, the anchor's options,
//! names — change the object's `w:drawing` as Word would write it, and the
//! object is read again from it (see `word_document::objects`).

use super::snapshot_edit::{EditScope, SnapshotEditCommand};
use crate::docx_xml::XmlElement;
use crate::word_document::{FloatingObjectTarget, WordDocument};

/// What an edit does to a drawing: given the document (to add media, read
/// the object) and the object's `w:drawing`, changes the drawing.
pub type DrawingEdit = Box<dyn FnMut(&mut WordDocument, &mut XmlElement) -> Result<(), String>>;

/// A command that runs `edit` on the drawing of the object at `target` as one
/// undo step. The paragraph holding the object is what changes, with any
/// media the edit adds.
pub fn edit_drawing_command(
    name: &'static str,
    target: FloatingObjectTarget,
    mut edit: impl FnMut(&mut WordDocument, &mut XmlElement) -> Result<(), String> + 'static,
) -> SnapshotEditCommand {
    SnapshotEditCommand::with_scope(
        name,
        EditScope::Paragraphs(Some((target.paragraph_index, target.paragraph_index))),
        move |document: &mut WordDocument| {
            let mut drawing = document.object_drawing(target)?;
            edit(document, &mut drawing)?;
            document.replace_object_drawing(target, drawing)
        },
    )
}
