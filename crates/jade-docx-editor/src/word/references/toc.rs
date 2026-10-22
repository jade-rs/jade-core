//! 目次 beyond the default: the 目次 dialog — page numbers shown and
//! right-aligned, the tab leader, the levels, styles mapped to levels,
//! links — the 図表目次 for a caption label, the 手動作成目次 to type over,
//! and 目次の更新 of the whole table or its page numbers only.

use std::collections::HashMap;

use crate::document::{RunFieldKind, TocOptions};

use super::edit_document;
use crate::word::{WordFormatting, WordSession};

/// タブ リーダー.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabLeader {
    None,
    Dots,
    Dashes,
    Underline,
}

impl TabLeader {
    pub const ALL: [TabLeader; 4] = [
        TabLeader::None,
        TabLeader::Dots,
        TabLeader::Dashes,
        TabLeader::Underline,
    ];

    /// `w:tab/@w:leader`.
    pub fn token(self) -> &'static str {
        match self {
            TabLeader::None => "none",
            TabLeader::Dots => "dot",
            TabLeader::Dashes => "hyphen",
            TabLeader::Underline => "underscore",
        }
    }

    pub fn from_token(token: &str) -> Self {
        match token {
            "none" => TabLeader::None,
            "hyphen" => TabLeader::Dashes,
            "underscore" => TabLeader::Underline,
            _ => TabLeader::Dots,
        }
    }

    /// What the list shows.
    pub fn sample(self) -> &'static str {
        match self {
            TabLeader::None => "(なし)",
            TabLeader::Dots => "........",
            TabLeader::Dashes => "--------",
            TabLeader::Underline => "________",
        }
    }
}

/// What the dialog builds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TocKind {
    /// 目次: from headings (and styles mapped to levels).
    Contents,
    /// 図表目次: from the captions of a label, or every caption.
    Figures {
        label: String,
        /// ラベルと番号を含める.
        include_label: bool,
    },
}

/// The 目次 / 図表目次 dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TocDialog {
    pub kind: TocKind,
    /// ページ番号を表示する.
    pub show_page_numbers: bool,
    /// ページ番号を右揃えにする.
    pub right_align: bool,
    pub leader: TabLeader,
    /// ハイパーリンクを使う.
    pub hyperlinks: bool,
    /// アウトライン レベル: 1 to this.
    pub levels: u8,
    /// オプション: the paragraph styles and the level each gives, `None`
    /// for none: (style id, its name, level).
    pub styles: Vec<(String, String, Option<u8>)>,
    /// アウトライン レベル: paragraphs' own outline levels count too.
    pub outline_levels: bool,
    /// The table the dialog was opened on, which OK replaces.
    pub replacing: Option<usize>,
}

impl TocDialog {
    /// The engine's options for the dialog.
    pub fn options(&self) -> TocOptions {
        let mut options = TocOptions {
            outline_min: 1,
            outline_max: self.levels.clamp(1, 9),
            hyperlinks: self.hyperlinks,
            hide_page_numbers: !self.show_page_numbers,
            page_number_separator: (self.show_page_numbers && !self.right_align)
                .then(|| " ".to_string()),
            use_applied_outline_levels: self.outline_levels,
            custom_style_levels: self
                .styles
                .iter()
                .filter(|(id, name, level)| level.is_some() && builtin_level(id, name) != *level)
                .filter_map(|(id, _, level)| level.map(|level| (id.clone(), level)))
                .collect(),
            ..TocOptions::default()
        };
        if let TocKind::Figures {
            label,
            include_label,
        } = &self.kind
        {
            options.caption_label = Some(label.clone());
            options.omit_caption_label_and_number = !include_label;
            options.custom_style_levels.clear();
            options.use_applied_outline_levels = false;
        }
        options
    }

    /// The preview's lines: (level, text, page).
    pub fn preview(&self) -> Vec<(u8, String, &'static str)> {
        match &self.kind {
            TocKind::Contents => (1..=self.levels.min(3))
                .map(|level| {
                    (
                        level,
                        format!("見出し {level}"),
                        ["1", "3", "5"][level as usize - 1],
                    )
                })
                .collect(),
            TocKind::Figures {
                label,
                include_label,
            } => (1..=3)
                .map(|number| {
                    let text = if *include_label {
                        format!("{label} {number}: テキスト")
                    } else {
                        "テキスト".to_string()
                    };
                    (1, text, ["1", "3", "5"][number - 1])
                })
                .collect(),
        }
    }
}

/// The level a built-in heading style has by itself, by its id or name
/// (`Heading2`, `heading 2`, `見出し 2`).
fn builtin_level(id: &str, name: &str) -> Option<u8> {
    [id, name].into_iter().find_map(|value| {
        let normalized: String = value
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .flat_map(char::to_lowercase)
            .collect();
        normalized
            .strip_prefix("heading")
            .or_else(|| normalized.strip_prefix("見出し"))?
            .parse()
            .ok()
    })
}

/// Whether a `TOC` instruction builds a table of figures (`\\c` or `\\a`).
pub fn is_table_of_figures(instruction: &str) -> bool {
    crate::document::parse_toc_switches(instruction)
        .caption_label
        .is_some()
}

/// Tables of contents over every session.
pub trait WordToc: WordSession {
    /// The table of contents (or figures) a paragraph is in, by its order.
    fn toc_at(&self, paragraph: usize) -> Option<usize> {
        self.word_document()
            .tables_of_contents()
            .iter()
            .position(|block| (block.start..=block.end).contains(&paragraph))
    }

    /// The table 目次の更新 updates: the one at the caret, else the first.
    fn toc_to_update(&self) -> Option<usize> {
        let caret = self.selection_range().map(|range| range.focus.paragraph);
        caret
            .and_then(|paragraph| self.toc_at(paragraph))
            .or_else(|| (!self.word_document().tables_of_contents().is_empty()).then_some(0))
    }

    /// The 目次 dialog: the caret's table's settings when it is in one,
    /// Word's defaults otherwise.
    fn toc_dialog(&self, figures: Option<&str>) -> TocDialog {
        let document = self.word_document();
        let contents = document.tables_of_contents();
        let replacing = self
            .selection_range()
            .and_then(|range| self.toc_at(range.focus.paragraph))
            .filter(|&index| {
                let is_figures = crate::document::parse_toc_switches(&contents[index].instruction)
                    .caption_label
                    .is_some();
                is_figures == figures.is_some()
            });
        let current = replacing.map(|index| {
            (
                crate::document::parse_toc_switches(&contents[index].instruction),
                document.toc_leader(&contents[index]),
            )
        });
        let styles = self
            .paragraph_styles()
            .into_iter()
            .map(|style| {
                let level = builtin_level(&style.id, &style.name);
                (style.id, style.name, level)
            })
            .collect();
        let mut dialog = TocDialog {
            kind: match figures {
                Some(label) => TocKind::Figures {
                    label: label.to_string(),
                    include_label: true,
                },
                None => TocKind::Contents,
            },
            show_page_numbers: true,
            right_align: true,
            leader: TabLeader::Dots,
            hyperlinks: true,
            levels: 3,
            styles,
            outline_levels: true,
            replacing,
        };
        if let Some((options, leader)) = current {
            dialog.show_page_numbers = !options.hide_page_numbers;
            dialog.right_align = options.page_number_separator.is_none();
            dialog.leader = TabLeader::from_token(&leader);
            dialog.hyperlinks = options.hyperlinks;
            dialog.levels = options.outline_max.clamp(1, 9);
            dialog.outline_levels = options.use_applied_outline_levels;
            for (style, level) in options.custom_style_levels {
                if let Some(entry) = dialog
                    .styles
                    .iter_mut()
                    .find(|(id, name, _)| id.eq_ignore_ascii_case(&style) || *name == style)
                {
                    entry.2 = Some(level);
                }
            }
            if let (
                TocKind::Figures {
                    label,
                    include_label,
                },
                Some(caption),
            ) = (&mut dialog.kind, options.caption_label)
            {
                *label = caption;
                *include_label = !options.omit_caption_label_and_number;
            }
        }
        dialog
    }

    /// OK: the table built as the dialog says, after the caret's paragraph
    /// or over the table it was opened on — one undo step.
    fn apply_toc_dialog(&mut self, dialog: &TocDialog) -> Result<(), String> {
        let options = dialog.options();
        let leader = dialog.leader.token().to_string();
        let replacing = dialog.replacing;
        let after = self
            .selection_range()
            .map(|range| range.normalized().0.paragraph)
            .unwrap_or(0);
        edit_document(self, "table_of_contents", true, move |document, _| {
            match replacing {
                Some(index) => document.replace_table_of_contents(index, &options, &leader),
                None => document.insert_table_of_contents_with_leader(after, &options, &leader),
            }
            .map(|_| ())
        })
    }

    /// 目次の更新: the table at the caret (or the first) made again, or —
    /// `page_numbers_only` — its page numbers brought up to date with its
    /// entries left as they are. One undo step.
    fn update_toc(&mut self, page_numbers_only: bool) -> Result<(), String> {
        let index = self
            .toc_to_update()
            .ok_or_else(|| "更新する目次がありません".to_string())?;
        if !page_numbers_only {
            return edit_document(
                self,
                "update_table_of_contents",
                true,
                move |document, _| document.update_table_of_contents_at(index).map(|_| ()),
            );
        }
        // Each entry's bookmark, and the page its paragraph is on now.
        let document = self.word_document();
        let block = document.tables_of_contents()[index].clone();
        let model = document.document.as_ref();
        let mut pages = HashMap::new();
        for paragraph in block.start..=block.end {
            let Some(entry) = document.paragraph(paragraph) else {
                continue;
            };
            for run in &entry.runs {
                let Some(RunFieldKind::PageRef { name, .. }) =
                    run.field.as_ref().map(|field| &field.kind)
                else {
                    continue;
                };
                let page = model
                    .and_then(|model| {
                        model
                            .bookmarks
                            .iter()
                            .find(|bookmark| &bookmark.name == name)
                    })
                    .and_then(|_| document.bookmark_position(name))
                    .and_then(|position| self.paragraph_page(position.paragraph));
                if let Some(page) = page {
                    pages.insert(name.clone(), page.to_string());
                }
            }
        }
        edit_document(self, "update_page_numbers", true, move |document, _| {
            document.refresh_toc_page_numbers(index, &pages).map(|_| ())
        })
    }

    /// 手動作成目次: a 目次 heading and a line per level to type over.
    fn insert_manual_toc(&mut self) -> Result<(), String> {
        let after = self
            .selection_range()
            .map(|range| range.normalized().0.paragraph)
            .unwrap_or(0);
        let lines = vec![
            (
                1,
                "章のタイトルを入力してください (レベル 1)".to_string(),
                "1".to_string(),
            ),
            (
                2,
                "章のタイトルを入力してください (レベル 2)".to_string(),
                "2".to_string(),
            ),
            (
                3,
                "章のタイトルを入力してください (レベル 3)".to_string(),
                "3".to_string(),
            ),
        ];
        edit_document(
            self,
            "manual_table_of_contents",
            false,
            move |document, selection| {
                *selection =
                    Some(document.insert_manual_table_of_contents(after, "目次", &lines, "dot")?);
                Ok(())
            },
        )
    }
}

impl<T: WordSession + ?Sized> WordToc for T {}

#[cfg(test)]
mod tests {
    use super::{TabLeader, TocDialog, TocKind};

    fn dialog() -> TocDialog {
        TocDialog {
            kind: TocKind::Contents,
            show_page_numbers: true,
            right_align: true,
            leader: TabLeader::Dots,
            hyperlinks: true,
            levels: 3,
            styles: vec![
                ("1".to_string(), "heading 1".to_string(), Some(1)),
                ("Quote".to_string(), "Quote".to_string(), Some(2)),
            ],
            outline_levels: true,
            replacing: None,
        }
    }

    #[test]
    fn the_dialog_says_which_switches() {
        let options = dialog().options();
        assert_eq!(options.outline_max, 3);
        assert!(options.hyperlinks);
        assert_eq!(options.page_number_separator, None);
        assert_eq!(options.custom_style_levels, [("Quote".to_string(), 2)]);
        let mut left = dialog();
        left.right_align = false;
        assert_eq!(left.options().page_number_separator.as_deref(), Some(" "));
        left.show_page_numbers = false;
        assert!(left.options().hide_page_numbers);
        let mut figures = dialog();
        figures.kind = TocKind::Figures {
            label: "図".to_string(),
            include_label: false,
        };
        let options = figures.options();
        assert_eq!(options.caption_label.as_deref(), Some("図"));
        assert!(options.omit_caption_label_and_number);
        assert!(options.custom_style_levels.is_empty());
    }
}
