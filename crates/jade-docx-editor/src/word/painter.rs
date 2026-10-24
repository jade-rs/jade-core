//! 書式のコピー/貼り付け (Format Painter) over any [`WordSession`]: the
//! character formatting at the caret or the selection's start — and, when
//! the caret is bare or the selection holds whole paragraphs, the paragraph's
//! style and formatting too — taken up, and put on the next text selected,
//! as Word's paintbrush does. One click paints once; a double-click keeps it
//! painting until Escape (the shells keep that state).

use jade_docx_headless_api::{HeadlessCommand, HeadlessRunStyle};

use super::paragraph::{LineSpacing, ParagraphFormat, WordParagraphs};
use super::{css_color, Helpers, WordFormatting, WordSession};

/// What the painter carries.
#[derive(Clone, Debug, PartialEq)]
pub struct CopiedFormat {
    pub run: HeadlessRunStyle,
    /// The paragraph's style and format, when they were taken up too.
    pub paragraph: Option<(Option<String>, ParagraphFormat)>,
}

/// A format that differs from `now` everywhere, so that every part of `now`
/// is written: painting puts the whole of a format on the target.
fn unlike(now: &ParagraphFormat) -> ParagraphFormat {
    ParagraphFormat {
        alignment: String::new(),
        unit: now.unit,
        left: f32::NAN,
        right: f32::NAN,
        special: now.special,
        special_by: f32::NAN,
        before: f32::NAN,
        after: f32::NAN,
        line: LineSpacing::Multiple(f32::NAN),
        widow_control: !now.widow_control,
        keep_next: !now.keep_next,
        keep_lines: !now.keep_lines,
        page_break_before: !now.page_break_before,
    }
}

/// The paintbrush over every session.
pub trait WordFormatPainter: WordSession {
    /// Takes up the format at the caret, or at the selection's start.
    fn copy_format(&self) -> CopiedFormat {
        let run = self.run_style_at(self.format_anchor()).unwrap_or_default();
        let whole_paragraphs = match self.selection_range() {
            None => true,
            Some(range) if range.is_collapsed() => true,
            Some(range) => {
                let (start, end) = range.normalized();
                let end_length = self.text_of(end.paragraph).map_or(0, |text| text.len());
                start.offset == 0 && (end.offset >= end_length || end.paragraph > start.paragraph)
            }
        };
        let paragraph = whole_paragraphs.then(|| {
            let (first, _) = self.paragraph_span();
            let style = self
                .word_document()
                .paragraph(first)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|properties| properties.style.as_deref())
                .map(str::to_string);
            (style, self.paragraph_format())
        });
        CopiedFormat { run, paragraph }
    }

    /// Puts `copied` on the selection — or, at a bare caret, the word around
    /// it — and its paragraph format on the paragraphs they are in, as one
    /// undo step. The selection stays.
    fn paste_format(&mut self, copied: &CopiedFormat) -> Result<(), String> {
        let mut commands = Vec::new();
        if let Some((style_id, format)) = &copied.paragraph {
            let (start_paragraph, end_paragraph) = self.paragraph_span();
            commands.push(HeadlessCommand::SetParagraphStyle {
                start_paragraph,
                end_paragraph,
                style_id: style_id.clone(),
            });
            commands.extend(self.paragraph_format_commands(&unlike(format), format)?);
        }
        if let Some(((start_paragraph, start_offset), (end_paragraph, end_offset))) =
            self.run_span()
        {
            let run = &copied.run;
            let baseline = run.baseline.as_deref();
            commands.extend([
                HeadlessCommand::ApplyBold {
                    start_paragraph,
                    start_offset,
                    end_paragraph,
                    end_offset,
                    enabled: run.bold.unwrap_or(false),
                },
                HeadlessCommand::ApplyItalic {
                    start_paragraph,
                    start_offset,
                    end_paragraph,
                    end_offset,
                    enabled: run.italic.unwrap_or(false),
                },
                HeadlessCommand::ApplyUnderline {
                    start_paragraph,
                    start_offset,
                    end_paragraph,
                    end_offset,
                    kind: None,
                    enabled: run.underline.unwrap_or(false),
                },
                HeadlessCommand::ApplyStrike {
                    start_paragraph,
                    start_offset,
                    end_paragraph,
                    end_offset,
                    double: false,
                    enabled: run.strike.unwrap_or(false),
                },
            ]);
            // The baseline off first, then the one wanted on: each turns
            // the other off.
            let superscript = baseline == Some("superscript");
            let subscript = baseline == Some("subscript");
            let mut baselines = [
                HeadlessCommand::ApplySuperscript {
                    start_paragraph,
                    start_offset,
                    end_paragraph,
                    end_offset,
                    enabled: superscript,
                },
                HeadlessCommand::ApplySubscript {
                    start_paragraph,
                    start_offset,
                    end_paragraph,
                    end_offset,
                    enabled: subscript,
                },
            ];
            if superscript {
                baselines.reverse();
            }
            commands.extend(baselines);
            if let Some(font_size) = run.font_size {
                commands.push(HeadlessCommand::SetFontSize {
                    start_paragraph,
                    start_offset,
                    end_paragraph,
                    end_offset,
                    font_size,
                });
            }
            if let Some(font_family) = run.font_family.clone() {
                commands.push(HeadlessCommand::SetFontFamily {
                    start_paragraph,
                    start_offset,
                    end_paragraph,
                    end_offset,
                    font_family,
                });
            }
            let color = match run.color.as_deref() {
                Some(color) if color.eq_ignore_ascii_case("auto") => "auto".to_string(),
                Some(color) => css_color(color),
                None => "auto".to_string(),
            };
            commands.push(HeadlessCommand::SetFontColor {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                color,
            });
        } else if copied.paragraph.is_none() {
            return Err("select the text to paint".to_string());
        }
        self.apply_commands(commands, true)
    }
}

impl<T: WordSession + ?Sized> WordFormatPainter for T {}
