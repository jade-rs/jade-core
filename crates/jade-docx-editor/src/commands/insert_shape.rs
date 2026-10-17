use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::Run;
use crate::word_document::{FloatingObjectKind, FloatingObjectTarget, ShapeInsertSpec};
use jade_docx_layout::selection::{SelectionRange, TextPosition};

pub struct InsertShapeCommand {
    paragraph_index: usize,
    offset: usize,
    spec: ShapeInsertSpec,
    previous_runs: Option<Vec<Run>>,
    previous_selection: Option<SelectionRange>,
    inserted_run_index: Option<usize>,
}

impl InsertShapeCommand {
    pub fn new(paragraph_index: usize, offset: usize, spec: ShapeInsertSpec) -> Self {
        Self {
            paragraph_index,
            offset,
            spec,
            previous_runs: None,
            previous_selection: None,
            inserted_run_index: None,
        }
    }

    pub fn inserted_target(&self) -> Option<FloatingObjectTarget> {
        Some(FloatingObjectTarget {
            paragraph_index: self.paragraph_index,
            run_index: self.inserted_run_index?,
            kind: FloatingObjectKind::TextBox,
        })
    }
}

impl Command for InsertShapeCommand {
    fn name(&self) -> &'static str {
        "insert_shape"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.previous_selection = *ctx.selection;
        self.previous_runs = Some(
            ctx.document
                .paragraph(self.paragraph_index)
                .ok_or(CommandError::ParagraphUnavailable)?
                .runs
                .clone(),
        );
        self.inserted_run_index = Some(
            ctx.document
                .insert_shape_at(self.paragraph_index, self.offset, self.spec.clone())
                .map_err(CommandError::Custom)?,
        );
        *ctx.selection = Some(SelectionRange::collapsed(TextPosition::new(
            self.paragraph_index,
            self.offset,
        )));
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous_runs = self
            .previous_runs
            .as_ref()
            .ok_or_else(|| CommandError::Custom("insert_shape backup is missing".into()))?;
        let paragraph = ctx
            .document
            .paragraph_mut(self.paragraph_index)
            .ok_or(CommandError::ParagraphUnavailable)?;
        paragraph.runs = previous_runs.clone();
        ctx.document.sync_paragraph_text(self.paragraph_index);
        ctx.document.rebuild_paragraph_surfaces();
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}
