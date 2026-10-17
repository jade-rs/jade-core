//! Unicode-aware case conversion that preserves run boundaries and formatting.

use super::run_backup::{capture_paragraph_runs, restore_paragraph_runs, ParagraphRunsBackup};
use super::text_edit::{split_runs_at_preserving_source, ZeroWidthRuns};
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{Paragraph, Run};
use jade_docx_headless_api::HeadlessChangeCaseMode;
use jade_docx_layout::selection::{SelectionRange, TextPosition};

pub struct ChangeCaseCommand {
    range: SelectionRange,
    mode: HeadlessChangeCaseMode,
    previous_runs: Option<Vec<ParagraphRunsBackup>>,
    previous_selection: Option<SelectionRange>,
}

impl ChangeCaseCommand {
    pub fn new(range: SelectionRange, mode: HeadlessChangeCaseMode) -> Self {
        Self {
            range,
            mode,
            previous_runs: None,
            previous_selection: None,
        }
    }
}

impl Command for ChangeCaseCommand {
    fn name(&self) -> &'static str {
        "change_case"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if self.range.is_collapsed() {
            return Err(CommandError::InvalidSelection);
        }
        let (start, end) = self.range.normalized();
        if ctx.document.paragraph_count() == 0
            || start.paragraph >= ctx.document.paragraph_count()
            || end.paragraph >= ctx.document.paragraph_count()
        {
            return Err(CommandError::ParagraphUnavailable);
        }

        self.previous_runs = Some(capture_paragraph_runs(ctx, start.paragraph, end.paragraph)?);
        self.previous_selection = *ctx.selection;

        let mut transformed_end = end;
        for paragraph_index in start.paragraph..=end.paragraph {
            let zero_width = ctx.document.zero_width_runs(paragraph_index);
            let paragraph = ctx
                .document
                .paragraph_mut(paragraph_index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            let paragraph_len = paragraph_len(paragraph);
            let range_start = if paragraph_index == start.paragraph {
                start.offset.min(paragraph_len)
            } else {
                0
            };
            let range_end = if paragraph_index == end.paragraph {
                end.offset.min(paragraph_len)
            } else {
                paragraph_len
            };
            if range_start >= range_end {
                continue;
            }

            let new_len = transform_run_range(
                &mut paragraph.runs,
                range_start,
                range_end,
                self.mode,
                &zero_width,
            );
            if paragraph_index == end.paragraph {
                transformed_end.offset = range_start + new_len;
            }
            ctx.document
                .sync_paragraph_text_preserving_drawings(paragraph_index);
        }

        let transformed = normalized_range_with_direction(self.range, start, transformed_end);
        *ctx.selection = Some(transformed);
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let backups = self
            .previous_runs
            .take()
            .ok_or_else(|| CommandError::Custom("missing backup for undo".into()))?;
        restore_paragraph_runs(ctx, &backups)?;
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

fn normalized_range_with_direction(
    original: SelectionRange,
    start: TextPosition,
    end: TextPosition,
) -> SelectionRange {
    if original.anchor <= original.focus {
        SelectionRange {
            anchor: start,
            focus: end,
        }
    } else {
        SelectionRange {
            anchor: end,
            focus: start,
        }
    }
}

fn transform_run_range(
    runs: &mut Vec<Run>,
    start: usize,
    end: usize,
    mode: HeadlessChangeCaseMode,
    zero_width: &ZeroWidthRuns,
) -> usize {
    let right = split_runs_at_preserving_source(runs, end, zero_width);
    let mut middle = split_runs_at_preserving_source(runs, start, zero_width);
    let mut left = std::mem::take(runs);
    let mut state = CaseState::new(mode);
    let mut transformed_len = 0;

    for run in &mut middle {
        run.text = state.transform(&run.text);
        transformed_len += run.text.len();
    }

    left.extend(middle);
    left.extend(right);
    *runs = left;
    transformed_len
}

pub(crate) struct CaseState {
    mode: HeadlessChangeCaseMode,
    capitalize_next_word: bool,
    capitalize_next_sentence: bool,
}

impl CaseState {
    pub(crate) fn new(mode: HeadlessChangeCaseMode) -> Self {
        Self {
            mode,
            capitalize_next_word: true,
            capitalize_next_sentence: true,
        }
    }

    pub(crate) fn transform(&mut self, text: &str) -> String {
        let mut output = String::with_capacity(text.len());
        for character in text.chars() {
            match self.mode {
                HeadlessChangeCaseMode::Upper => output.extend(character.to_uppercase()),
                HeadlessChangeCaseMode::Lower => output.extend(character.to_lowercase()),
                HeadlessChangeCaseMode::Title => {
                    if self.capitalize_next_word && character.is_alphabetic() {
                        output.extend(character.to_uppercase());
                        self.capitalize_next_word = false;
                    } else {
                        output.extend(character.to_lowercase());
                    }
                    if character.is_whitespace() {
                        self.capitalize_next_word = true;
                    }
                }
                HeadlessChangeCaseMode::Sentence => {
                    if self.capitalize_next_sentence && character.is_alphabetic() {
                        output.extend(character.to_uppercase());
                        self.capitalize_next_sentence = false;
                    } else {
                        output.extend(character.to_lowercase());
                    }
                    if matches!(character, '.' | '!' | '?' | '。' | '！' | '？') {
                        self.capitalize_next_sentence = true;
                    }
                }
            }
        }
        output
    }
}

fn paragraph_len(paragraph: &Paragraph) -> usize {
    paragraph.runs.iter().map(|run| run.text.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use std::path::PathBuf;

    fn run(text: &str) -> Run {
        Run {
            text: text.to_string(),
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
        }
    }

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../public/fixtures")
            .join(name)
    }

    #[test]
    fn title_case_keeps_context_across_run_boundaries() {
        let mut runs = vec![run("heLLo "), run("wORLD")];
        let len = transform_run_range(
            &mut runs,
            0,
            11,
            HeadlessChangeCaseMode::Title,
            &ZeroWidthRuns::default(),
        );
        assert_eq!(len, 11);
        assert_eq!(runs[0].text, "Hello ");
        assert_eq!(runs[1].text, "World");
    }

    #[test]
    fn upper_case_reports_unicode_expansion() {
        let mut runs = vec![run("straße")];
        let len = transform_run_range(
            &mut runs,
            0,
            7,
            HeadlessChangeCaseMode::Upper,
            &ZeroWidthRuns::default(),
        );
        assert_eq!(runs[0].text, "STRASSE");
        assert_eq!(len, 7);
    }

    #[test]
    fn case_change_exports_and_reopens_with_transformed_text() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 5));
        let mut selection = Some(range);
        let mut commands = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            commands
                .execute(
                    Box::new(ChangeCaseCommand::new(range, HeadlessChangeCaseMode::Upper)),
                    &mut ctx,
                )
                .expect("change case");
        }

        let exported = word_doc.export_docx().expect("export docx");
        let reopened = executor::block_on(WordDocument::load(&exported, &Options::default()))
            .expect("reopen export");
        let text = reopened
            .paragraph(0)
            .expect("first paragraph")
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>();
        assert!(text.starts_with("HELLO"), "reopened text: {text:?}");
    }
}
