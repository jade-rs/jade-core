use super::set_run_format::SetRunFormatCommand;
use super::{Command, CommandContext, CommandResult};
use jade_docx_headless_api::HeadlessRunFormat;
use jade_docx_layout::selection::SelectionRange;

/// Assigns an explicit bold state to selected text and paragraph marks.
pub struct SetBoldCommand {
    inner: SetRunFormatCommand,
}

impl SetBoldCommand {
    pub fn new(range: SelectionRange, value: bool) -> Self {
        Self {
            inner: SetRunFormatCommand::new(
                range,
                HeadlessRunFormat {
                    bold: Some(value),
                    ..Default::default()
                },
            ),
        }
    }
}

impl Command for SetBoldCommand {
    fn tracks_formatting(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "set_bold"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.inner.apply(ctx)
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.inner.undo(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::{Body, BodyElement, Document, Paragraph, Run};
    use crate::word_document::WordDocument;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};

    fn sample_document() -> WordDocument {
        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![Run {
                        text: "Hello".to_string(),
                        properties: None,
                        image: None,
                        text_box: None,
                        math: None,
                        field: None,
                        hyperlink: None,
                        node_id: None,
                        text_node_id: None,
                        revision_node_id: None,
                        comment_reference_ids: Vec::new(),
                        footnote_reference_ids: Vec::new(),
                        endnote_reference_ids: Vec::new(),
                        note_separator: None,
                        has_footnote_ref: false,
                        has_endnote_ref: false,
                        revision: None,
                    }],
                    properties: None,
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            ..Document::default()
        })
    }

    #[test]
    fn set_bold_false_disables_existing_bold() {
        let mut document = sample_document();
        let mut selection = Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, 5),
        ));
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetBoldCommand::new(
                        SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 5)),
                        true,
                    )),
                    &mut ctx,
                )
                .expect("set bold true");
            executor
                .execute(
                    Box::new(SetBoldCommand::new(
                        SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 5)),
                        false,
                    )),
                    &mut ctx,
                )
                .expect("set bold false");
        }

        let paragraph = document.paragraph(0).expect("paragraph");
        assert_eq!(
            paragraph.runs[0]
                .properties
                .as_ref()
                .and_then(|props| props.bold),
            Some(false)
        );
    }
}
