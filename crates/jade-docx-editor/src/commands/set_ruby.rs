//! ルビ (Word's Phonetic Guide): puts a ruby over a range of a paragraph,
//! replaces the rubies it overlaps, or takes them off.
//!
//! The base runs keep their text and formatting; the model marks them with
//! the [`RubyAnnotation`] and the writer authors the `w:ruby` around them
//! (see `WordDocument::create_authored_ruby_run`). A ruby the range touches
//! is written afresh, so its XML is removed first.

use std::rc::Rc;

use jade_docx_headless_api::HeadlessRuby;

use super::text_edit::split_runs_at_preserving_source;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{RubyAnnotation, RunFonts};
use crate::docx_xml::XmlNodeId;
use crate::word_document::WordDocumentSnapshot;
use jade_docx_layout::selection::SelectionRange;

pub struct SetRubyCommand {
    paragraph: usize,
    start: usize,
    end: usize,
    ruby: Option<HeadlessRuby>,
    previous: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
}

impl SetRubyCommand {
    pub fn new(paragraph: usize, start: usize, end: usize, ruby: Option<HeadlessRuby>) -> Self {
        Self {
            paragraph,
            start: start.min(end),
            end: start.max(end),
            ruby,
            previous: None,
            previous_selection: None,
        }
    }
}

/// `rubyAlign` as the model keeps it (lowercased, as the reader does).
fn align_token(align: Option<&str>) -> String {
    align
        .map(str::trim)
        .filter(|align| !align.is_empty())
        .unwrap_or("distributeSpace")
        .to_ascii_lowercase()
}

impl Command for SetRubyCommand {
    fn name(&self) -> &'static str {
        "set_ruby"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if self.start >= self.end {
            return Err(CommandError::InvalidSelection);
        }
        if !ctx.document.is_body_paragraph_surface(self.paragraph) {
            return Err(CommandError::Custom(
                "ruby can be set in body paragraphs only".into(),
            ));
        }
        if let Some(ruby) = &self.ruby {
            if ruby.text.trim().is_empty() {
                return Err(CommandError::Custom("the ruby text is empty".into()));
            }
        }
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or_else(|| CommandError::Custom("document is unavailable".into()))?;
        let previous_selection = *ctx.selection;

        let zero_width = ctx.document.zero_width_runs(self.paragraph);
        let paragraph = ctx
            .document
            .paragraph_mut(self.paragraph)
            .ok_or(CommandError::ParagraphUnavailable)?;
        let length: usize = paragraph.runs.iter().map(|run| run.text.len()).sum();
        let (start, end) = (self.start.min(length), self.end.min(length));
        if start >= end {
            return Err(CommandError::InvalidSelection);
        }
        // The range as runs of its own.
        let tail = split_runs_at_preserving_source(&mut paragraph.runs, end, &zero_width);
        let middle = split_runs_at_preserving_source(&mut paragraph.runs, start, &zero_width);
        if middle.iter().any(|run| {
            run.field.is_some()
                || run.math.is_some()
                || run.image.is_some()
                || run.text_box.is_some()
        }) {
            paragraph.runs.extend(middle);
            paragraph.runs.extend(tail);
            ctx.document.restore_snapshot(snapshot);
            return Err(CommandError::Custom("ruby can be set on text only".into()));
        }
        let first_middle = paragraph.runs.len();
        paragraph.runs.extend(middle);
        let after_middle = paragraph.runs.len();
        paragraph.runs.extend(tail);

        // Every ruby base the range touches: its runs go back to plain, and
        // their XML is written afresh.
        let mut bases: Vec<(usize, usize)> = Vec::new();
        let mut current: Option<usize> = None;
        for (index, run) in paragraph.runs.iter().enumerate() {
            match run
                .properties
                .as_ref()
                .and_then(|props| props.ruby.as_ref())
            {
                Some(ruby) if ruby.continuation && current.is_some() => {}
                Some(_) => {
                    if let Some(begin) = current.take() {
                        bases.push((begin, index));
                    }
                    current = Some(index);
                }
                None => {
                    if let Some(begin) = current.take() {
                        bases.push((begin, index));
                    }
                }
            }
        }
        if let Some(begin) = current {
            bases.push((begin, paragraph.runs.len()));
        }
        let touched: Vec<(usize, usize)> = bases
            .into_iter()
            .filter(|(begin, finish)| *begin < after_middle && *finish > first_middle)
            .collect();
        let mut detached: Vec<XmlNodeId> = Vec::new();
        let base_size = paragraph.runs[first_middle..after_middle]
            .iter()
            .find_map(|run| run.properties.as_ref().and_then(|props| props.font_size));
        for (begin, finish) in &touched {
            for run in &mut paragraph.runs[*begin..*finish] {
                if let Some(props) = run.properties.as_mut() {
                    props.ruby = None;
                }
                detached.extend(run.node_id.take());
                run.text_node_id = None;
            }
        }
        if let Some(ruby) = &self.ruby {
            let fonts = ruby.font.as_deref().map(|font| RunFonts {
                ascii: Some(Rc::from(font)),
                h_ansi: Some(Rc::from(font)),
                east_asia: Some(Rc::from(font)),
                ..RunFonts::default()
            });
            let annotation = RubyAnnotation {
                text: ruby.text.clone(),
                size_pt: ruby
                    .size
                    .or(base_size.map(|size| size / 2.0))
                    .filter(|size| size.is_finite() && *size > 0.0),
                align: Some(align_token(ruby.align.as_deref())),
                raise_half_points: ruby
                    .raise
                    .filter(|raise| raise.is_finite() && *raise >= 0.0)
                    .map(|raise| (raise * 2.0).round() as u32),
                fonts,
                continuation: false,
            };
            for (index, run) in paragraph.runs[first_middle..after_middle]
                .iter_mut()
                .filter(|run| !run.text.is_empty())
                .enumerate()
            {
                let mut annotation = annotation.clone();
                annotation.continuation = index > 0;
                run.properties.get_or_insert_with(Default::default).ruby =
                    Some(Box::new(annotation));
                detached.extend(run.node_id.take());
                run.text_node_id = None;
            }
        }
        ctx.document
            .remove_ruby_run_elements(self.paragraph, &detached);
        ctx.document
            .sync_paragraph_text_preserving_drawings(self.paragraph);
        self.previous = Some(snapshot);
        self.previous_selection = previous_selection;
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self
            .previous
            .take()
            .ok_or_else(|| CommandError::Custom("ruby snapshot is missing".into()))?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}
