//! Copy and paste that keep the formatting (元の書式を保持), over any
//! [`WordSession`].
//!
//! A copy takes the selection's runs with their character formatting and
//! each paragraph's style and alignment; a paste puts them back as one undo
//! step — text, paragraph breaks, then each run's format where it differs
//! from what the text would take on where it lands. The plain text goes on
//! the system clipboard as before; a shell keeps the copy beside it and
//! pastes it whenever the clipboard still holds that text.

use jade_docx_headless_api::{
    HeadlessCommand, HeadlessQuery, HeadlessQueryResult, HeadlessRunStyle,
};

use super::{query, Position, WordFormatting, WordSession};

/// A run of copied text and its character formatting.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipRun {
    pub text: String,
    pub style: HeadlessRunStyle,
}

/// A copied paragraph, or the part of one the selection held.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipParagraph {
    pub style_id: Option<String>,
    pub alignment: Option<String>,
    pub runs: Vec<ClipRun>,
}

/// What a copy took.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RichClip {
    pub paragraphs: Vec<ClipParagraph>,
}

impl RichClip {
    /// The text the system clipboard gets: paragraphs joined by newlines,
    /// which a plain paste turns back into paragraph breaks.
    pub fn plain_text(&self) -> String {
        self.paragraphs
            .iter()
            .map(|paragraph| {
                paragraph
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Whether `text` — what the system clipboard holds now — is this copy's,
    /// so pasting it should keep the formatting. Line endings may have been
    /// rewritten on the way.
    pub fn matches(&self, text: &str) -> bool {
        text.replace("\r\n", "\n").replace('\r', "\n") == self.plain_text()
    }
}

/// The formatting commands that give `start..end` of a paragraph `style`,
/// for each property `style` states and `inherited` does not already have.
fn format_commands(
    paragraph: usize,
    (start, end): (usize, usize),
    style: &HeadlessRunStyle,
    inherited: &HeadlessRunStyle,
) -> Vec<HeadlessCommand> {
    let (start_paragraph, start_offset, end_paragraph, end_offset) =
        (paragraph, start, paragraph, end);
    let mut commands = Vec::new();
    let differs = |wanted: Option<bool>, had: Option<bool>| {
        let wanted = wanted.unwrap_or(false);
        (wanted != had.unwrap_or(false)).then_some(wanted)
    };
    if let Some(enabled) = differs(style.bold, inherited.bold) {
        commands.push(HeadlessCommand::ApplyBold {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            enabled,
        });
    }
    if let Some(enabled) = differs(style.italic, inherited.italic) {
        commands.push(HeadlessCommand::ApplyItalic {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            enabled,
        });
    }
    if let Some(enabled) = differs(style.underline, inherited.underline) {
        commands.push(HeadlessCommand::ApplyUnderline {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            kind: None,
            enabled,
        });
    }
    if let Some(enabled) = differs(style.strike, inherited.strike) {
        commands.push(HeadlessCommand::ApplyStrike {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            double: false,
            enabled,
        });
    }
    if style.baseline != inherited.baseline {
        match style.baseline.as_deref() {
            Some("superscript") => commands.push(HeadlessCommand::ApplySuperscript {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                enabled: true,
            }),
            Some("subscript") => commands.push(HeadlessCommand::ApplySubscript {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                enabled: true,
            }),
            _ => {
                let off = if inherited.baseline.as_deref() == Some("subscript") {
                    HeadlessCommand::ApplySubscript {
                        start_paragraph,
                        start_offset,
                        end_paragraph,
                        end_offset,
                        enabled: false,
                    }
                } else {
                    HeadlessCommand::ApplySuperscript {
                        start_paragraph,
                        start_offset,
                        end_paragraph,
                        end_offset,
                        enabled: false,
                    }
                };
                commands.push(off);
            }
        }
    }
    if let Some(font_size) = style
        .font_size
        .filter(|size| Some(*size) != inherited.font_size)
    {
        commands.push(HeadlessCommand::SetFontSize {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            font_size,
        });
    }
    if let Some(family) = style
        .font_family
        .as_ref()
        .filter(|family| Some(*family) != inherited.font_family.as_ref())
    {
        commands.push(HeadlessCommand::SetFontFamily {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            font_family: family.clone(),
        });
    }
    if style.color != inherited.color {
        let color = match style.color.as_deref() {
            Some(color) if color.eq_ignore_ascii_case("auto") => "auto".to_string(),
            Some(color) => format!("#{}", color.trim_start_matches('#')),
            None => "auto".to_string(),
        };
        commands.push(HeadlessCommand::SetFontColor {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            color,
        });
    }
    commands
}

/// Word's 貼り付けのオプション: how a paste treats the copy's formatting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteMode {
    /// 元の書式を保持: the copy's character and paragraph formatting.
    KeepSource,
    /// 書式を結合 (貼り付け先の書式に合わせる, ⌥⇧⌘V): the destination's
    /// formatting, keeping the copy's bold, italic, underline, strikethrough
    /// and super/subscript.
    MergeFormatting,
    /// テキストのみ保持: the text alone, formatted as typing there would be.
    TextOnly,
}

impl PasteMode {
    pub const ALL: [PasteMode; 3] = [
        PasteMode::KeepSource,
        PasteMode::MergeFormatting,
        PasteMode::TextOnly,
    ];
}

/// The formats 形式を選択して貼り付け lists, and the paste each stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteFormat {
    /// Microsoft Word 文書.
    WordDocument,
    /// 書式付きテキスト (RTF).
    FormattedText,
    /// HTML 形式.
    Html,
    /// 書式なしテキスト.
    UnformattedText,
    /// 書式なしの Unicode テキスト.
    UnicodeText,
}

impl PasteFormat {
    pub const ALL: [PasteFormat; 5] = [
        PasteFormat::WordDocument,
        PasteFormat::FormattedText,
        PasteFormat::Html,
        PasteFormat::UnformattedText,
        PasteFormat::UnicodeText,
    ];

    /// The formats a clipboard offers: all of them for a copy made here,
    /// only the text ones for text from elsewhere.
    pub fn available(rich: bool) -> Vec<PasteFormat> {
        Self::ALL
            .into_iter()
            .filter(|format| rich || format.mode() == PasteMode::TextOnly)
            .collect()
    }

    pub fn mode(self) -> PasteMode {
        match self {
            PasteFormat::WordDocument | PasteFormat::FormattedText | PasteFormat::Html => {
                PasteMode::KeepSource
            }
            PasteFormat::UnformattedText | PasteFormat::UnicodeText => PasteMode::TextOnly,
        }
    }
}

impl RichClip {
    /// The copy as a paste in `mode` would put it in, given what typing
    /// at the destination would take on (`inherited`).
    pub fn for_mode(&self, mode: PasteMode, inherited: &HeadlessRunStyle) -> RichClip {
        if mode == PasteMode::KeepSource {
            return self.clone();
        }
        let paragraphs = self
            .paragraphs
            .iter()
            .map(|paragraph| ClipParagraph {
                style_id: None,
                alignment: None,
                runs: paragraph
                    .runs
                    .iter()
                    .map(|run| {
                        let mut style = inherited.clone();
                        if mode == PasteMode::MergeFormatting {
                            let keep = |source: Option<bool>, had: Option<bool>| {
                                if source == Some(true) {
                                    Some(true)
                                } else {
                                    had
                                }
                            };
                            style.bold = keep(run.style.bold, style.bold);
                            style.italic = keep(run.style.italic, style.italic);
                            style.underline = keep(run.style.underline, style.underline);
                            style.strike = keep(run.style.strike, style.strike);
                            if run.style.baseline.is_some() {
                                style.baseline = run.style.baseline.clone();
                            }
                        }
                        ClipRun {
                            text: run.text.clone(),
                            style,
                        }
                    })
                    .collect(),
            })
            .collect();
        RichClip { paragraphs }
    }

    /// Plain text from elsewhere as a copy: one paragraph per line.
    pub fn from_text(text: &str) -> RichClip {
        RichClip {
            paragraphs: text
                .replace("\r\n", "\n")
                .replace('\r', "\n")
                .split('\n')
                .map(|line| ClipParagraph {
                    runs: (!line.is_empty())
                        .then(|| ClipRun {
                            text: line.to_string(),
                            style: HeadlessRunStyle::default(),
                        })
                        .into_iter()
                        .collect(),
                    ..Default::default()
                })
                .collect(),
        }
    }
}

/// Formatted copy and paste for every session.
pub trait WordClipboard: WordSession {
    /// The selection's text with its formatting; `None` for a caret.
    fn copy_rich(&self) -> Option<RichClip> {
        let range = self
            .selection_range()
            .filter(|range| !range.is_collapsed())?;
        let (start, end) = range.normalized();
        let mut paragraphs = Vec::new();
        for index in start.paragraph..=end.paragraph {
            let Some(HeadlessQueryResult::Paragraph {
                value: Some(snapshot),
                ..
            }) = query(
                self.word_document(),
                HeadlessQuery::Paragraph { paragraph: index },
            )
            else {
                continue;
            };
            let from = if index == start.paragraph {
                start.offset
            } else {
                0
            };
            let to = if index == end.paragraph {
                end.offset
            } else {
                snapshot.text.len()
            };
            let mut runs: Vec<ClipRun> = snapshot
                .runs
                .iter()
                .filter_map(|run| {
                    let (run_start, run_end) = (run.start_offset.max(from), run.end_offset.min(to));
                    let text = snapshot.text.get(run_start..run_end)?;
                    (!text.is_empty()).then(|| ClipRun {
                        text: text.to_string(),
                        style: run.style.clone(),
                    })
                })
                .collect();
            // A paragraph whose runs the snapshot does not cover still copies
            // its text.
            let covered: usize = runs.iter().map(|run| run.text.len()).sum();
            if covered == 0 {
                if let Some(text) = snapshot.text.get(from..to.max(from)) {
                    if !text.is_empty() {
                        runs.push(ClipRun {
                            text: text.to_string(),
                            style: HeadlessRunStyle::default(),
                        });
                    }
                }
            }
            paragraphs.push(ClipParagraph {
                style_id: snapshot.style_id.clone(),
                alignment: snapshot.alignment.clone(),
                runs,
            });
        }
        Some(RichClip { paragraphs })
    }

    /// Pastes `clip` the way `mode` says — 元の書式を保持, 書式を結合 or
    /// テキストのみ保持 — as one undo step. Returns where the caret goes.
    fn paste_as(&mut self, clip: &RichClip, mode: PasteMode) -> Result<Position, String> {
        let range = self.selection_range().ok_or("no caret")?;
        let (start, _) = range.normalized();
        let inherited = self
            .run_style_at((start.paragraph, start.offset.saturating_sub(1)))
            .unwrap_or_default();
        self.paste_rich(&clip.for_mode(mode, &inherited))
    }

    /// Pastes `clip` in place of the selection, or at the caret, as one undo
    /// step. Returns where the caret goes: after what was pasted.
    fn paste_rich(&mut self, clip: &RichClip) -> Result<Position, String> {
        let range = self.selection_range().ok_or("no caret")?;
        let (start, _) = range.normalized();
        let mut commands = Vec::new();
        if !range.is_collapsed() {
            let (start, end) = range.normalized();
            commands.push(HeadlessCommand::ReplaceRange {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                text: String::new(),
            });
        }
        // What typed text would take on where the paste lands: the character
        // before the caret, as typing continues it.
        let inherited = self
            .run_style_at((start.paragraph, start.offset.saturating_sub(1)))
            .unwrap_or_default();
        let at = push_paste_commands(
            clip,
            (start.paragraph, start.offset),
            &inherited,
            &mut commands,
        );
        if commands.is_empty() {
            return Ok(at);
        }
        self.apply_commands(commands, false)?;
        Ok(at)
    }

    /// Drag and drop: `clip` dropped at `target`, and — for a move — the
    /// range it was dragged from taken out, as one undo step. `target` is
    /// where the pointer let go in the document as it was; returns where
    /// the dropped text ends.
    fn drop_rich(
        &mut self,
        clip: &RichClip,
        moved_from: Option<(Position, Position)>,
        target: Position,
    ) -> Result<Position, String> {
        let inherited = self
            .run_style_at((target.0, target.1.saturating_sub(1)))
            .unwrap_or_default();
        let mut commands = Vec::new();
        let mut at = target;
        if let Some((start, end)) = moved_from {
            let (start, end) = if start <= end {
                (start, end)
            } else {
                (end, start)
            };
            if target > start && target < end {
                return Err("dropped inside what was dragged".to_string());
            }
            commands.push(HeadlessCommand::ReplaceRange {
                start_paragraph: start.0,
                start_offset: start.1,
                end_paragraph: end.0,
                end_offset: end.1,
                text: String::new(),
            });
            at = position_after_removal(target, start, end);
        }
        let end = push_paste_commands(clip, at, &inherited, &mut commands);
        self.apply_commands(commands, false)?;
        Ok(end)
    }
}

/// Where `target` is once `start..end` has been taken out before it.
pub fn position_after_removal(target: Position, start: Position, end: Position) -> Position {
    if target <= start {
        target
    } else if target.0 == end.0 {
        (start.0, start.1 + target.1.saturating_sub(end.1))
    } else {
        (target.0 - (end.0 - start.0), target.1)
    }
}

/// The commands that put `clip` in at `start`, appended to `commands`;
/// answers where the pasted text ends.
fn push_paste_commands(
    clip: &RichClip,
    start: Position,
    inherited: &HeadlessRunStyle,
    commands: &mut Vec<HeadlessCommand>,
) -> Position {
    let inherited = inherited.clone();
    let mut at = start;
    let mut formats = Vec::new();
    let count = clip.paragraphs.len();
    for (index, paragraph) in clip.paragraphs.iter().enumerate() {
        if index > 0 {
            commands.push(HeadlessCommand::InsertParagraphBreak {
                paragraph: at.0,
                offset: at.1,
                count: 1,
            });
            at = (at.0 + 1, 0);
        }
        // Every copied paragraph but the last came with its mark, and the
        // mark carries the paragraph's style to the paragraph it ends
        // here; the last ends in the destination's own mark.
        if index + 1 < count {
            if let Some(style_id) = &paragraph.style_id {
                formats.push(HeadlessCommand::SetParagraphStyle {
                    start_paragraph: at.0,
                    end_paragraph: at.0,
                    style_id: Some(style_id.clone()),
                });
            }
            if let Some(alignment) = &paragraph.alignment {
                formats.push(HeadlessCommand::SetParagraphAlignment {
                    start_paragraph: at.0,
                    end_paragraph: at.0,
                    alignment: alignment.clone(),
                });
            }
        }
        for run in &paragraph.runs {
            commands.push(HeadlessCommand::InsertText {
                paragraph: at.0,
                offset: at.1,
                text: run.text.clone(),
            });
            let end = at.1 + run.text.len();
            formats.extend(format_commands(at.0, (at.1, end), &run.style, &inherited));
            at.1 = end;
        }
    }
    // Formats after all the text: a split carries a paragraph's direct
    // formatting into the next, so formatting first would be undone.
    commands.extend(formats);
    at
}

impl<T: WordSession + ?Sized> WordClipboard for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plain_text_joins_paragraphs_with_newlines() {
        let run = |text: &str| ClipRun {
            text: text.to_string(),
            style: HeadlessRunStyle::default(),
        };
        let clip = RichClip {
            paragraphs: vec![
                ClipParagraph {
                    runs: vec![run("Bold"), run(" plain")],
                    ..Default::default()
                },
                ClipParagraph {
                    runs: vec![run("next")],
                    ..Default::default()
                },
            ],
        };
        assert_eq!(clip.plain_text(), "Bold plain\nnext");
        assert!(clip.matches("Bold plain\r\nnext"));
        assert!(!clip.matches("Bold plain"));
    }

    #[test]
    fn merging_keeps_emphasis_and_text_only_keeps_nothing() {
        let clip = RichClip {
            paragraphs: vec![ClipParagraph {
                style_id: Some("Heading1".into()),
                alignment: Some("center".into()),
                runs: vec![ClipRun {
                    text: "Bold red".into(),
                    style: HeadlessRunStyle {
                        bold: Some(true),
                        font_size: Some(20.0),
                        color: Some("FF0000".into()),
                        font_family: Some("Arial".into()),
                        ..Default::default()
                    },
                }],
            }],
        };
        let destination = HeadlessRunStyle {
            font_size: Some(10.5),
            font_family: Some("游明朝".into()),
            ..Default::default()
        };
        let merged = clip.for_mode(PasteMode::MergeFormatting, &destination);
        let run = &merged.paragraphs[0].runs[0];
        assert_eq!(run.style.bold, Some(true));
        assert_eq!(run.style.font_size, Some(10.5));
        assert_eq!(run.style.font_family.as_deref(), Some("游明朝"));
        assert_eq!(run.style.color, None);
        assert_eq!(merged.paragraphs[0].style_id, None);
        let plain = clip.for_mode(PasteMode::TextOnly, &destination);
        assert_eq!(plain.paragraphs[0].runs[0].style, destination);
        assert_eq!(clip.for_mode(PasteMode::KeepSource, &destination), clip);
        // Text from elsewhere offers only the text formats.
        assert_eq!(
            PasteFormat::available(false),
            [PasteFormat::UnformattedText, PasteFormat::UnicodeText]
        );
        assert_eq!(RichClip::from_text("a\r\nb").plain_text(), "a\nb");
    }

    #[test]
    fn only_what_differs_is_formatted() {
        let bold = HeadlessRunStyle {
            bold: Some(true),
            font_size: Some(14.0),
            ..Default::default()
        };
        let plain = HeadlessRunStyle {
            font_size: Some(14.0),
            ..Default::default()
        };
        let commands = format_commands(3, (0, 4), &bold, &plain);
        assert_eq!(commands.len(), 1, "{commands:?}");
        assert!(matches!(
            commands[0],
            HeadlessCommand::ApplyBold { enabled: true, .. }
        ));
        // Into bold text, plain text has bold taken off.
        let commands = format_commands(3, (0, 4), &plain, &bold);
        assert!(matches!(
            commands[0],
            HeadlessCommand::ApplyBold { enabled: false, .. }
        ));
    }
}
