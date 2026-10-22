//! 拡張書式's ルビ (Word's Phonetic Guide) over any [`WordSession`]: the
//! selected text split into bases — the whole string, or one per character —
//! each with its guide text, 配置, フォント, サイズ and オフセット, applied as
//! one undo step; ルビの解除 takes them off.

use jade_docx_headless_api::HeadlessCommand;
pub use jade_docx_headless_api::HeadlessRuby;

use super::{Helpers, WordSession};

/// 配置: Word's ruby alignments, in its dialog's order.
pub const RUBY_ALIGNMENTS: [&str; 6] = [
    "center",
    "distributeLetter",
    "distributeSpace",
    "left",
    "right",
    "rightVertical",
];

/// One base and its guide.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RubyGroup {
    /// Byte offsets into the paragraph.
    pub start: usize,
    pub end: usize,
    pub base: String,
    pub ruby: String,
}

/// The ルビ dialog while it is open.
#[derive(Clone, Debug, PartialEq)]
pub struct RubyDialog {
    pub paragraph: usize,
    pub start: usize,
    pub end: usize,
    pub groups: Vec<RubyGroup>,
    pub align: String,
    /// Points; `None` is half the base's size.
    pub size: Option<f32>,
    /// Points; `None` is Word's default.
    pub raise: Option<f32>,
    pub font: Option<String>,
}

/// Whether Word gives a character a reading of its own when splitting per
/// character: kana and punctuation need none.
fn needs_reading(ch: char) -> bool {
    !matches!(ch,
        '\u{3040}'..='\u{30FF}' | '\u{3000}'..='\u{303F}' | '\u{FF01}'..='\u{FF65}')
        && !ch.is_ascii_punctuation()
        && !ch.is_whitespace()
}

impl RubyDialog {
    /// 文字単位: one base per character, keeping guides already typed for a
    /// base that stays the same.
    pub fn split_per_character(&mut self, text: &str) {
        let kept = std::mem::take(&mut self.groups);
        let mut groups = Vec::new();
        for (index, ch) in text[self.start..self.end].char_indices() {
            let start = self.start + index;
            let end = start + ch.len_utf8();
            let ruby = kept
                .iter()
                .find(|group| group.start == start && group.end == end)
                .map(|group| group.ruby.clone())
                .unwrap_or_default();
            groups.push(RubyGroup {
                start,
                end,
                base: ch.to_string(),
                ruby,
            });
        }
        // Kana and punctuation take no reading, so they join the base before
        // them, as Word groups 読み方 into 読 + み方.
        let mut merged: Vec<RubyGroup> = Vec::new();
        for group in groups {
            let joins = group.ruby.is_empty()
                && !group.base.chars().any(needs_reading)
                && merged.last().is_some();
            if joins {
                let last = merged.last_mut().expect("checked");
                last.end = group.end;
                last.base.push_str(&group.base);
            } else {
                merged.push(group);
            }
        }
        self.groups = merged;
    }

    /// 文字列全体: one base, with the guides run together.
    pub fn join_whole(&mut self, text: &str) {
        let ruby: String = self
            .groups
            .iter()
            .map(|group| group.ruby.as_str())
            .collect();
        self.groups = vec![RubyGroup {
            start: self.start,
            end: self.end,
            base: text[self.start..self.end].to_string(),
            ruby,
        }];
    }

    /// ルビの解除: every guide emptied, which OK writes as no ruby.
    pub fn clear(&mut self) {
        for group in &mut self.groups {
            group.ruby.clear();
        }
    }

    fn ruby(&self, text: &str) -> HeadlessRuby {
        HeadlessRuby {
            text: text.to_string(),
            align: Some(self.align.clone()),
            size: self.size,
            raise: self.raise,
            font: self.font.clone(),
        }
    }

    /// The commands OK runs: the range cleared, then each base with a guide.
    pub fn commands(&self) -> Vec<HeadlessCommand> {
        let mut commands = vec![HeadlessCommand::SetRuby {
            paragraph: self.paragraph,
            start_offset: self.start,
            end_offset: self.end,
            ruby: None,
        }];
        for group in self
            .groups
            .iter()
            .filter(|group| !group.ruby.trim().is_empty())
        {
            commands.push(HeadlessCommand::SetRuby {
                paragraph: self.paragraph,
                start_offset: group.start,
                end_offset: group.end,
                ruby: Some(self.ruby(group.ruby.trim())),
            });
        }
        commands
    }
}

/// 囲い文字's enclosures, in Word's order.
pub const ENCLOSURES: [&str; 4] = ["○", "□", "△", "◇"];

/// The commands, for every session.
pub trait WordAsian: WordSession {
    /// What 囲い文字 opens on: the selection's first character, or the one
    /// before the caret.
    fn enclosed_character_text(&self) -> String {
        let Some(range) = self.selection_range() else {
            return String::new();
        };
        let (start, end) = range.normalized();
        let text = self.text_of(start.paragraph).unwrap_or_default();
        let slice = if range.is_collapsed() {
            text.get(..start.offset)
                .and_then(|before| before.chars().next_back())
                .map(String::from)
        } else {
            let to = if end.paragraph == start.paragraph {
                end.offset
            } else {
                text.len()
            };
            text.get(start.offset..to)
                .and_then(|selected| selected.chars().next())
                .map(String::from)
        };
        slice.unwrap_or_default()
    }

    /// 囲い文字's OK: `text` in `enclosure` in place of the selection — or
    /// of the character before a caret, which is what it opened on.
    fn insert_enclosed_character(&mut self, text: &str, enclosure: &str) -> Result<(), String> {
        let range = self.selection_range().ok_or("no caret")?;
        let (mut start, end) = range.normalized();
        if range.is_collapsed() {
            let before = self
                .text_of(start.paragraph)
                .unwrap_or_default()
                .get(..start.offset)
                .and_then(|before| before.chars().next_back())
                .map_or(0, char::len_utf8);
            start.offset -= before;
        }
        let end_offset = if end.paragraph == start.paragraph {
            end.offset
        } else {
            self.text_of(start.paragraph).map_or(0, |text| text.len())
        };
        if text.trim().is_empty() {
            return Err("type the character to enclose".to_string());
        }
        self.apply_command(
            HeadlessCommand::InsertEnclosedCharacter {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: start.paragraph,
                end_offset,
                text: text.trim().to_string(),
                enclosure: enclosure.to_string(),
            },
            false,
        )
    }

    /// The ルビ dialog on the selection — or the word at a bare caret —
    /// within its first paragraph, with the rubies already there.
    fn ruby_dialog(&self) -> Result<RubyDialog, String> {
        let (start, end) = self
            .run_span()
            .ok_or_else(|| "select the text for the ruby".to_string())?;
        let paragraph = start.0;
        let text = self.text_of(paragraph).unwrap_or_default();
        let end_offset = if end.0 == paragraph {
            end.1
        } else {
            text.len()
        };
        let (start_offset, end_offset) = (start.1.min(text.len()), end_offset.min(text.len()));
        if start_offset >= end_offset {
            return Err("select the text for the ruby".to_string());
        }
        // The rubies there now, as groups.
        let mut groups: Vec<RubyGroup> = Vec::new();
        let mut align = None;
        let mut size = None;
        let mut font = None;
        if let Some(model) = self.word_document().paragraph(paragraph) {
            let mut at = 0;
            for run in &model.runs {
                let run_end = at + run.text.len();
                if let Some(ruby) = run
                    .properties
                    .as_ref()
                    .and_then(|props| props.ruby.as_ref())
                {
                    if run_end > start_offset && at < end_offset {
                        match groups.last_mut() {
                            Some(group) if ruby.continuation && group.end == at => {
                                group.end = run_end.min(end_offset);
                                group.base = text[group.start..group.end].to_string();
                            }
                            _ => {
                                let begin = at.max(start_offset);
                                let finish = run_end.min(end_offset);
                                groups.push(RubyGroup {
                                    start: begin,
                                    end: finish,
                                    base: text[begin..finish].to_string(),
                                    ruby: ruby.text.clone(),
                                });
                                align = align.or(ruby.align.clone());
                                size = size.or(ruby.size_pt);
                                font = font.or(ruby
                                    .fonts
                                    .as_ref()
                                    .and_then(|fonts| fonts.east_asia.as_deref())
                                    .map(str::to_string));
                            }
                        }
                    }
                }
                at = run_end;
            }
        }
        let align = align
            .and_then(|lower| {
                RUBY_ALIGNMENTS
                    .iter()
                    .find(|value| value.to_ascii_lowercase() == lower)
                    .map(|value| value.to_string())
            })
            .unwrap_or_else(|| "distributeSpace".to_string());
        let mut dialog = RubyDialog {
            paragraph,
            start: start_offset,
            end: end_offset,
            groups: Vec::new(),
            align,
            size,
            raise: None,
            font,
        };
        if groups.is_empty() {
            dialog.join_whole(&text);
        } else {
            // The text between existing bases, as bases without a guide.
            let mut filled = Vec::new();
            let mut at = start_offset;
            for group in groups {
                if group.start > at {
                    filled.push(RubyGroup {
                        start: at,
                        end: group.start,
                        base: text[at..group.start].to_string(),
                        ruby: String::new(),
                    });
                }
                at = group.end;
                filled.push(group);
            }
            if at < end_offset {
                filled.push(RubyGroup {
                    start: at,
                    end: end_offset,
                    base: text[at..end_offset].to_string(),
                    ruby: String::new(),
                });
            }
            dialog.groups = filled;
        }
        Ok(dialog)
    }

    /// OK: the dialog's rubies, as one undo step.
    fn apply_ruby(&mut self, dialog: &RubyDialog) -> Result<(), String> {
        self.apply_commands(dialog.commands(), true)
    }
}

impl<T: WordSession + ?Sized> WordAsian for T {}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog(text: &str) -> RubyDialog {
        let mut dialog = RubyDialog {
            paragraph: 0,
            start: 0,
            end: text.len(),
            groups: Vec::new(),
            align: "distributeSpace".into(),
            size: None,
            raise: None,
            font: None,
        };
        dialog.join_whole(text);
        dialog
    }

    #[test]
    fn bases_split_per_character_and_join_whole() {
        let text = "読み方";
        let mut ruby = dialog(text);
        assert_eq!(ruby.groups.len(), 1);
        ruby.split_per_character(text);
        let bases: Vec<&str> = ruby
            .groups
            .iter()
            .map(|group| group.base.as_str())
            .collect();
        assert_eq!(bases, ["読み", "方"], "kana joins the kanji before it");
        ruby.groups[0].ruby = "よ".into();
        ruby.groups[1].ruby = "かた".into();
        let commands = ruby.commands();
        assert_eq!(commands.len(), 3, "cleared, then each base");
        assert!(
            matches!(&commands[2], HeadlessCommand::SetRuby { start_offset: 6, end_offset: 9, ruby: Some(ruby), .. } if ruby.text == "かた")
        );
        ruby.join_whole(text);
        assert_eq!(ruby.groups[0].ruby, "よかた");
        ruby.clear();
        assert_eq!(ruby.commands().len(), 1, "解除 only clears");
    }
}
