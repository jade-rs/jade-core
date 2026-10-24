//! Complete gallery brushes preserve untouched ink and survive history and export.
mod common;
use common::Session;
use jade_docx_editor::commands::Command;
use jade_docx_editor::word::ink_tools::{InkBrush, InkTool, Pen, WordInkTools};
use jade_docx_editor::word::{CellLayout, WordSession};
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::HeadlessCommand;
use jade_selection::SelectionRange;
impl WordSession for Session {
    fn word_document(&self) -> &WordDocument {
        &self.document
    }
    fn selection_range(&self) -> Option<SelectionRange> {
        self.selection
    }
    fn set_selection_range(&mut self, range: Option<SelectionRange>) {
        self.selection = range;
    }
    fn apply_command(&mut self, command: HeadlessCommand, keep: bool) -> Result<(), String> {
        self.apply_commands(vec![command], keep)
    }
    fn apply_commands(
        &mut self,
        commands: Vec<HeadlessCommand>,
        _keep: bool,
    ) -> Result<(), String> {
        jade_docx_editor::headless_api::apply_headless_commands_to_document(
            &mut self.document,
            &jade_docx_headless_api::HeadlessEditRequest { commands },
        )
        .map(|_| ())
        .map_err(|error| format!("{error:?}"))
    }
    fn execute_command(&mut self, command: Box<dyn Command>, keep: bool) -> Result<(), String> {
        let before = self.selection;
        self.try_run(command)?;
        if keep {
            self.selection = before;
        }
        Ok(())
    }
    fn cell_id_for_paragraph(&self, _paragraph: usize) -> Option<usize> {
        None
    }
    fn cell_layout(&self, _cell_id: usize) -> Option<&CellLayout> {
        None
    }
    fn cell_layouts(&self) -> &[CellLayout] {
        &[]
    }
    fn table_order_for(&self, _table_id: usize) -> Option<usize> {
        None
    }
    fn hyperlink_regions(&self) -> Vec<jade_docx_editor::canvas_renderer::HyperlinkRegion> {
        Vec::new()
    }
    fn select_cells(&mut self, _anchor_cell: usize, _focus_cell: usize) -> Result<(), String> {
        Err("no tables in this session".into())
    }
}

#[test]
fn complete_gallery_brush_preserves_other_ink_and_round_trips_history() {
    let mut session = Session::open("<w:p><w:r><w:t>Ink</w:t></w:r></w:p>", &[]);
    let left = vec![(100.0, 120.0), (160.0, 150.0), (220.0, 130.0)];
    let right = vec![(300.0, 220.0), (360.0, 250.0), (420.0, 230.0)];
    session
        .draw_stroke(
            0,
            left.clone(),
            &Pen::new(InkTool::Pencil, "#123456", 0.5),
            false,
        )
        .unwrap();
    session
        .draw_stroke(
            0,
            right.clone(),
            &Pen::new(InkTool::Pen, "#654321", 0.7),
            false,
        )
        .unwrap();
    let before = session.ink_strokes();
    for brush in [
        InkBrush {
            tool: InkTool::Pen,
            color: "#0078D7".into(),
            width_px: 8.0,
            height_px: 8.0,
            effect: Some("gold".into()),
            transparency: 17,
        },
        InkBrush {
            tool: InkTool::Highlighter,
            color: "#FFFF00".into(),
            width_px: 2.0,
            height_px: 12.0,
            effect: None,
            transparency: 51,
        },
    ] {
        let index = session
            .ink_strokes()
            .iter()
            .position(|s| s.trace.points[0].0 < 250.0)
            .unwrap();
        session
            .restyle_strokes_with_brush(&[index], &brush)
            .unwrap();
        let after = session.ink_strokes();
        assert_eq!(after.len(), 2);
        assert_eq!(
            after
                .iter()
                .find(|s| s.trace.points[0].0 < 250.0)
                .unwrap()
                .trace
                .brush,
            brush
        );
        assert_eq!(
            after
                .iter()
                .find(|s| s.trace.points[0].0 > 250.0)
                .unwrap()
                .trace,
            before
                .iter()
                .find(|s| s.trace.points[0].0 > 250.0)
                .unwrap()
                .trace
        );
        let reopened = Session::new(session.reopened()).ink_strokes();
        assert_eq!(reopened.len(), after.len());
        for (saved, current) in reopened.iter().zip(&after) {
            assert_eq!(saved.trace.brush.tool, current.trace.brush.tool);
            assert_eq!(saved.trace.brush.color, current.trace.brush.color);
            assert_eq!(saved.trace.brush.effect, current.trace.brush.effect);
            assert_eq!(
                saved.trace.brush.transparency,
                current.trace.brush.transparency
            );
            // InkML and drawing anchors serialize finite-precision coordinates.
            assert!((saved.trace.brush.width_px - current.trace.brush.width_px).abs() < 0.005);
            assert!((saved.trace.brush.height_px - current.trace.brush.height_px).abs() < 0.005);
            assert_eq!(saved.trace.points.len(), current.trace.points.len());
            for (saved, current) in saved.trace.points.iter().zip(&current.trace.points) {
                assert!((saved.0 - current.0).abs() < 0.05);
                assert!((saved.1 - current.1).abs() < 0.05);
            }
        }
        session.undo();
        assert_eq!(
            session
                .ink_strokes()
                .iter()
                .map(|s| &s.trace)
                .collect::<Vec<_>>(),
            before.iter().map(|s| &s.trace).collect::<Vec<_>>()
        );
        session.redo();
        assert_eq!(
            session
                .ink_strokes()
                .iter()
                .map(|s| &s.trace)
                .collect::<Vec<_>>(),
            after.iter().map(|s| &s.trace).collect::<Vec<_>>()
        );
        session.undo();
    }
}
