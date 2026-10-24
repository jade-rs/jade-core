//! Atomic paragraph indent and spacing update used by Word's paragraph dialog.

use super::set_paragraph_indent::SetParagraphIndentCommand;
use super::set_paragraph_line_spacing::SetParagraphLineSpacingCommand;
use super::{Command, CommandContext, CommandResult};
use crate::document::Indent;

/// The paragraph dialog's 特殊インデント control.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpecialIndent {
    /// 「なし」: remove first-line and hanging indents.
    None,
    /// 「字下げ」 by this many points.
    FirstLine(f32),
    /// 「ぶら下げ」 by this many points.
    Hanging(f32),
}

/// Applies the fields submitted by the paragraph dialog as one undo step.
///
/// Every field is optional because Word's dialog only writes the values the
/// user changed: an untouched left indent keeps its `w:leftChars`, untouched
/// spacing keeps `w:beforeLines`, and a multi-paragraph selection whose
/// paragraphs differ keeps each paragraph's own value for the untouched
/// fields instead of being flattened to the caret paragraph's.
pub struct SetParagraphMetricsCommand {
    start_paragraph: usize,
    end_paragraph: usize,
    left: Option<f32>,
    right: Option<f32>,
    before: Option<f32>,
    after: Option<f32>,
    special: Option<SpecialIndent>,
    indent: Option<SetParagraphIndentCommand>,
    spacing: Option<SetParagraphLineSpacingCommand>,
    indent_applied: bool,
    spacing_applied: bool,
    changed: bool,
}

impl SetParagraphMetricsCommand {
    pub fn new(
        start_paragraph: usize,
        end_paragraph: usize,
        left: Option<f32>,
        right: Option<f32>,
        before: Option<f32>,
        after: Option<f32>,
    ) -> Self {
        Self {
            start_paragraph,
            end_paragraph,
            left,
            right,
            before,
            after,
            special: None,
            indent: None,
            spacing: None,
            indent_applied: false,
            spacing_applied: false,
            changed: false,
        }
    }

    /// Folds the dialog's 特殊インデント change into the same undo step.
    pub fn with_special_indent(mut self, special: Option<SpecialIndent>) -> Self {
        self.special = special;
        self
    }

    fn normalized_range(&self) -> (usize, usize) {
        if self.start_paragraph <= self.end_paragraph {
            (self.start_paragraph, self.end_paragraph)
        } else {
            (self.end_paragraph, self.start_paragraph)
        }
    }

    fn indent_command(&self) -> Option<SetParagraphIndentCommand> {
        let (first_line, hanging, clear_special) = match self.special {
            None => (None, None, false),
            Some(SpecialIndent::None) => (None, None, true),
            Some(SpecialIndent::FirstLine(amount)) => (Some(amount), None, false),
            Some(SpecialIndent::Hanging(amount)) => (None, Some(amount), false),
        };
        if self.left.is_none() && self.right.is_none() && self.special.is_none() {
            return None;
        }
        Some(
            SetParagraphIndentCommand::new(
                self.start_paragraph,
                self.end_paragraph,
                self.left,
                self.right,
                first_line,
                hanging,
            )
            .with_clear_special(clear_special),
        )
    }

    fn spacing_command(&self) -> Option<SetParagraphLineSpacingCommand> {
        if self.before.is_none() && self.after.is_none() {
            return None;
        }
        Some(SetParagraphLineSpacingCommand::with_spacing(
            self.start_paragraph,
            self.end_paragraph,
            None,
            None,
            self.before,
            self.after,
        ))
    }

    fn indents(&self, ctx: &CommandContext<'_>) -> CommandResult<Vec<Option<Indent>>> {
        let (start, end) = self.normalized_range();
        (start..=end)
            .map(|paragraph_index| {
                ctx.document
                    .paragraph(paragraph_index)
                    .map(|paragraph| {
                        paragraph
                            .properties
                            .as_ref()
                            .and_then(|properties| properties.indent.clone())
                    })
                    .ok_or(super::CommandError::ParagraphUnavailable)
            })
            .collect()
    }
}

impl Command for SetParagraphMetricsCommand {
    fn tracks_formatting(&self) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "set_paragraph_metrics"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.indent_applied = false;
        self.spacing_applied = false;
        self.changed = false;
        self.indent = self.indent_command();
        self.spacing = self.spacing_command();
        let indents_before = self.indents(ctx)?;
        if let Some(indent) = self.indent.as_mut() {
            indent.apply(ctx)?;
            self.indent_applied = true;
        }
        if let Some(spacing) = self.spacing.as_mut() {
            if let Err(error) = spacing.apply(ctx) {
                if let Some(indent) = self.indent.as_mut().filter(|_| self.indent_applied) {
                    indent.undo(ctx)?;
                }
                self.indent_applied = false;
                return Err(error);
            }
            self.spacing_applied = true;
            self.changed |= spacing.should_record_history();
        }
        // Re-submitting the values already in place must not consume an
        // undo step (Word records nothing for an unchanged OK).
        if self.indent_applied {
            self.changed |= self.indents(ctx)? != indents_before;
        }
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if self.spacing_applied {
            if let Some(spacing) = self.spacing.as_mut() {
                spacing.undo(ctx)?;
            }
            self.spacing_applied = false;
        }
        if self.indent_applied {
            if let Some(indent) = self.indent.as_mut() {
                indent.undo(ctx)?;
            }
            self.indent_applied = false;
        }
        Ok(())
    }

    fn should_record_history(&self) -> bool {
        self.changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::set_paragraph_visuals::SetParagraphVisualsCommand;
    use crate::commands::CommandExecutor;
    use crate::document::{
        Body, BodyElement, Document, Paragraph, ParagraphProperties, Run, Spacing,
    };
    use crate::word_document::WordDocument;

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
    fn reapplying_current_metrics_does_not_consume_undo() {
        let mut document = sample_document();
        let mut selection = None;
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            SetParagraphMetricsCommand::new(0, 0, Some(-12.5), Some(6.0), Some(3.0), Some(9.0))
                .apply(&mut ctx)
                .expect("establish metrics");
        }

        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetParagraphVisualsCommand::shading(
                        0,
                        0,
                        Some("#FFF2CC".to_string()),
                    )),
                    &mut ctx,
                )
                .expect("set shading");
            executor
                .execute(
                    Box::new(SetParagraphMetricsCommand::new(
                        0,
                        0,
                        Some(-12.5),
                        Some(6.0),
                        Some(3.0),
                        Some(9.0),
                    )),
                    &mut ctx,
                )
                .expect("reapply current metrics");
            assert!(executor.undo(&mut ctx).expect("undo shading"));
        }

        assert!(document
            .paragraph(0)
            .and_then(|paragraph| paragraph.properties.as_ref())
            .and_then(|properties| properties.shading.as_ref())
            .is_none());
    }

    fn paragraph_with(indent: Indent, spacing: Spacing) -> Paragraph {
        let mut paragraph = sample_document().paragraph(0).cloned().expect("paragraph");
        paragraph.properties = Some(ParagraphProperties {
            indent: Some(indent),
            spacing: Some(spacing),
            ..Default::default()
        });
        paragraph
    }

    fn document_with(paragraphs: Vec<Paragraph>) -> WordDocument {
        WordDocument::from_document(Document {
            body: Some(Body {
                elements: paragraphs
                    .into_iter()
                    .map(BodyElement::from_paragraph)
                    .collect(),
            }),
            ..Document::default()
        })
    }

    fn indent_of(document: &WordDocument, index: usize) -> Indent {
        document
            .paragraph(index)
            .and_then(|paragraph| paragraph.properties.as_ref())
            .and_then(|properties| properties.indent.clone())
            .unwrap_or_default()
    }

    fn spacing_of(document: &WordDocument, index: usize) -> Spacing {
        document
            .paragraph(index)
            .and_then(|paragraph| paragraph.properties.as_ref())
            .and_then(|properties| properties.spacing.clone())
            .unwrap_or_default()
    }

    /// Word's paragraph dialog writes only the fields the user changed, so a
    /// right-indent edit keeps `w:leftChars` and `w:beforeLines`.
    #[test]
    fn untouched_fields_keep_character_and_line_units() {
        let mut document = document_with(vec![paragraph_with(
            Indent {
                left_chars: Some(200),
                left: Some(21.0),
                ..Default::default()
            },
            Spacing {
                before_lines: Some(0.5),
                before: Some(9.0),
                after: Some(4.0),
                ..Default::default()
            },
        )]);
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetParagraphMetricsCommand::new(
                        0,
                        0,
                        None,
                        Some(12.0),
                        None,
                        Some(6.0),
                    )),
                    &mut ctx,
                )
                .expect("apply right indent and after spacing");
        }
        let indent = indent_of(&document, 0);
        assert_eq!(indent.left_chars, Some(200));
        assert_eq!(indent.right, Some(12.0));
        let spacing = spacing_of(&document, 0);
        assert_eq!(spacing.before_lines, Some(0.5));
        assert_eq!(spacing.after, Some(6.0));
    }

    /// A spacing-only change over paragraphs with different indents must not
    /// flatten them to one value.
    #[test]
    fn spacing_only_change_keeps_each_paragraphs_indent() {
        let mut document = document_with(vec![
            paragraph_with(
                Indent {
                    left: Some(10.0),
                    ..Default::default()
                },
                Spacing::default(),
            ),
            paragraph_with(
                Indent {
                    left: Some(20.0),
                    right: Some(5.0),
                    ..Default::default()
                },
                Spacing::default(),
            ),
        ]);
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(SetParagraphMetricsCommand::new(
                        0,
                        1,
                        None,
                        None,
                        Some(6.0),
                        None,
                    )),
                    &mut ctx,
                )
                .expect("apply spacing before");
        }
        assert_eq!(indent_of(&document, 0).left, Some(10.0));
        assert_eq!(indent_of(&document, 1).left, Some(20.0));
        assert_eq!(indent_of(&document, 1).right, Some(5.0));
        assert_eq!(spacing_of(&document, 0).before, Some(6.0));
        assert_eq!(spacing_of(&document, 1).before, Some(6.0));
    }

    /// The dialog's OK is one undo step even when 特殊インデント changed too.
    #[test]
    fn special_indent_change_shares_the_dialogs_undo_step() {
        let mut document = sample_document();
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor
            .execute(
                Box::new(
                    SetParagraphMetricsCommand::new(0, 0, Some(12.0), None, Some(6.0), None)
                        .with_special_indent(Some(SpecialIndent::FirstLine(10.5))),
                ),
                &mut ctx,
            )
            .expect("apply dialog");
        let indent = indent_of(&document, 0);
        assert_eq!(indent.left, Some(12.0));
        assert_eq!(indent.first_line, Some(10.5));

        let mut ctx = CommandContext::new(&mut document, &mut selection);
        assert!(executor.undo(&mut ctx).expect("undo dialog"));
        assert!(
            !executor.can_undo(),
            "the dialog must be a single undo step"
        );
        assert_eq!(indent_of(&document, 0), Indent::default());
        assert_eq!(spacing_of(&document, 0).before, None);
    }

    /// 特殊インデント「なし」 alone keeps the left edge.
    #[test]
    fn special_indent_none_only_clears_first_line_and_hanging() {
        let mut document = document_with(vec![paragraph_with(
            Indent {
                left: Some(21.0),
                hanging: Some(10.5),
                ..Default::default()
            },
            Spacing::default(),
        )]);
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor
            .execute(
                Box::new(
                    SetParagraphMetricsCommand::new(0, 0, None, None, None, None)
                        .with_special_indent(Some(SpecialIndent::None)),
                ),
                &mut ctx,
            )
            .expect("clear special indent");
        let indent = indent_of(&document, 0);
        assert_eq!(indent.left, Some(21.0));
        assert_eq!(indent.hanging, None);
    }
}
