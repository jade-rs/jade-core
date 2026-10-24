//! Outline's level filter and navigation's folds share view state, without
//! changing the document or its undo history.
mod common;

use jade_docx_editor::commands::Command;
use jade_docx_editor::word::outline::OutlineDisplay;
use jade_docx_editor::word::{CellLayout, WordSession};
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::HeadlessCommand;
use jade_selection::SelectionRange;

struct ReadSession(WordDocument);
impl WordSession for ReadSession {
    fn word_document(&self) -> &WordDocument {
        &self.0
    }
    fn selection_range(&self) -> Option<SelectionRange> {
        None
    }
    fn set_selection_range(&mut self, _: Option<SelectionRange>) {
        panic!("read only")
    }
    fn apply_command(&mut self, _: HeadlessCommand, _: bool) -> Result<(), String> {
        panic!("read only")
    }
    fn apply_commands(&mut self, _: Vec<HeadlessCommand>, _: bool) -> Result<(), String> {
        panic!("read only")
    }
    fn execute_command(&mut self, _: Box<dyn Command>, _: bool) -> Result<(), String> {
        panic!("read only")
    }
    fn cell_id_for_paragraph(&self, _: usize) -> Option<usize> {
        None
    }
    fn cell_layout(&self, _: usize) -> Option<&CellLayout> {
        None
    }
    fn cell_layouts(&self) -> &[CellLayout] {
        &[]
    }
    fn table_order_for(&self, _: usize) -> Option<usize> {
        None
    }
    fn hyperlink_regions(&self) -> Vec<jade_docx_editor::canvas_renderer::HyperlinkRegion> {
        vec![]
    }
    fn select_cells(&mut self, _: usize, _: usize) -> Result<(), String> {
        panic!("read only")
    }
}

fn session() -> ReadSession {
    // A table makes surface paragraph indices differ from body element indices.
    ReadSession(common::load(&common::package(
        concat!(
            r#"<w:p><w:pPr><w:outlineLvl w:val="0"/></w:pPr><w:r><w:t>Parent</w:t></w:r></w:p>"#,
            r#"<w:p><w:r><w:t>Body</w:t></w:r></w:p>"#,
            r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:tcPr/><w:p><w:r><w:t>Cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
            r#"<w:p><w:pPr><w:outlineLvl w:val="1"/></w:pPr><w:r><w:t>Child</w:t></w:r></w:p>"#,
            r#"<w:p><w:r><w:t>Details</w:t></w:r></w:p>"#,
            r#"<w:p><w:pPr><w:outlineLvl w:val="0"/></w:pPr><w:r><w:t>Peer</w:t></w:r></w:p>"#,
            r#"<w:p><w:r><w:t>Tail</w:t></w:r></w:p>"#,
        ),
        &[],
    )))
}

#[test]
fn leaving_outline_removes_the_level_filter_and_preserves_folds() {
    let session = session();
    let before = common::document_xml(&session.0);
    let mut display = OutlineDisplay {
        collapsed: [0].into(),
        shown: 1,
        active: true,
    };
    assert_eq!(display.hidden_blocks(&session), [1, 2, 3, 4, 6]);
    display.active = false;
    assert_eq!(display.hidden_blocks(&session), [1, 2, 3, 4]);
    display.active = true;
    display.collapsed.clear();
    assert_eq!(
        display.hidden_blocks(&session),
        [1, 2, 3, 4, 6],
        "the shared level filter includes the table's body paragraph"
    );
    display.shown = 10;
    assert!(display.hidden_blocks(&session).is_empty());
    assert_eq!(common::document_xml(&session.0), before);
}

#[test]
fn nested_folds_are_deduplicated_and_invalid_headings_are_pruned() {
    let session = session();
    let child = session.0.navigation_headings()[1].paragraph_index;
    let mut display = OutlineDisplay {
        collapsed: [0, child, 1, usize::MAX].into(),
        ..Default::default()
    };
    assert_eq!(display.hidden_blocks(&session), [1, 2, 3, 4]);
    assert_eq!(display.collapsed, [0, child].into());
    display.collapsed.remove(&0);
    assert_eq!(display.hidden_blocks(&session), [4]);
    display.active = true;
    display.shown = 2;
    assert_eq!(display.hidden_blocks(&session), [1, 2, 4, 6]);
}
