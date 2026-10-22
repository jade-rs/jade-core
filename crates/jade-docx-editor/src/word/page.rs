//! The レイアウト tab's ページ設定 and the 挿入 tab's ヘッダー, フッター and
//! ページ番号, over any [`WordSession`]: margins, orientation, paper size,
//! columns, section breaks, the header and footer text, and a PAGE field.
//!
//! The page commands act on the document's default section, as the engine
//! offers them. Header and footer edits reach parts outside the body, so
//! they go through a snapshot command: one undo step, in both shells.

use crate::commands::snapshot_edit::SnapshotEditCommand;
use jade_docx_headless_api::{
    HeadlessCommand, HeadlessHeaderFooterSnapshot, HeadlessQuery, HeadlessQueryResult,
    HeadlessSectionBreakType,
};

use super::{query, WordSession};

/// Page margins, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Margins {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

/// The default section's page, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageSetup {
    pub width: f32,
    pub height: f32,
    pub landscape: bool,
    pub margins: Margins,
    pub columns: u32,
}

/// The 余白 gallery's entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarginPreset {
    /// 標準 (Normal).
    Normal,
    /// 狭い (Narrow).
    Narrow,
    /// やや狭い (Moderate).
    Moderate,
    /// 広い (Wide).
    Wide,
}

impl MarginPreset {
    pub const ALL: [MarginPreset; 4] = [
        MarginPreset::Normal,
        MarginPreset::Narrow,
        MarginPreset::Moderate,
        MarginPreset::Wide,
    ];

    /// The margins, as Japanese Word or English Word gives them: their
    /// 標準 differs (35 and 30 mm against one inch); the rest are the same.
    pub fn margins(self, japanese: bool) -> Margins {
        let (top, bottom, left, right) = match self {
            MarginPreset::Normal if japanese => (99.25, 85.05, 85.05, 85.05),
            MarginPreset::Normal => (72.0, 72.0, 72.0, 72.0),
            MarginPreset::Narrow => (36.0, 36.0, 36.0, 36.0),
            MarginPreset::Moderate => (72.0, 72.0, 54.0, 54.0),
            MarginPreset::Wide => (72.0, 72.0, 144.0, 144.0),
        };
        Margins {
            top,
            bottom,
            left,
            right,
        }
    }
}

/// The サイズ gallery's papers, upright, in points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paper {
    A3,
    A4,
    A5,
    /// JIS B4 and B5, as Japanese Word lists them.
    B4,
    B5,
    Letter,
    Legal,
}

impl Paper {
    pub const ALL: [Paper; 7] = [
        Paper::A3,
        Paper::A4,
        Paper::A5,
        Paper::B4,
        Paper::B5,
        Paper::Letter,
        Paper::Legal,
    ];

    pub fn size(self) -> (f32, f32) {
        match self {
            Paper::A3 => (841.9, 1190.55),
            Paper::A4 => (595.3, 841.9),
            Paper::A5 => (419.55, 595.3),
            Paper::B4 => (728.5, 1031.8),
            Paper::B5 => (515.9, 728.5),
            Paper::Letter => (612.0, 792.0),
            Paper::Legal => (612.0, 1008.0),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Paper::A3 => "A3",
            Paper::A4 => "A4",
            Paper::A5 => "A5",
            Paper::B4 => "B4",
            Paper::B5 => "B5",
            Paper::Letter => "Letter",
            Paper::Legal => "Legal",
        }
    }

    /// The paper a page of `width` × `height` points is, either way up.
    pub fn of(width: f32, height: f32) -> Option<Paper> {
        let (short, long) = (width.min(height), width.max(height));
        Paper::ALL.into_iter().find(|paper| {
            let (w, h) = paper.size();
            (w - short).abs() < 2.0 && (h - long).abs() < 2.0
        })
    }
}

/// The 区切り menu's section breaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionBreak {
    /// 次のページから開始.
    NextPage,
    /// 現在の位置から開始.
    Continuous,
    /// 偶数ページから開始.
    EvenPage,
    /// 奇数ページから開始.
    OddPage,
}

/// Which story an edit is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Story {
    Header,
    Footer,
}

/// Where the ページ番号 gallery puts the number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberAlignment {
    Left,
    Center,
    Right,
}

/// What the ページ設定 group and the ページ番号 list do, as one command the
/// shells' ribbons send, as table tools send a [`super::TableCommand`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PageCommand {
    Margins(MarginPreset),
    Landscape(bool),
    Paper(Paper),
    Columns(u32),
    Break(SectionBreak),
    PageNumber(Story, NumberAlignment),
    RemovePageNumbers,
}

/// Page setup, headers and footers for every session.
pub trait WordPage: WordSession {
    /// Runs a ribbon's page command; `japanese` picks which Word's 標準
    /// margins are meant.
    fn page_command(&mut self, command: PageCommand, japanese: bool) -> Result<(), String> {
        match command {
            PageCommand::Margins(preset) => self.set_margins(preset.margins(japanese)),
            PageCommand::Landscape(landscape) => self.set_landscape(landscape),
            PageCommand::Paper(paper) => self.set_paper(paper),
            PageCommand::Columns(count) => self.set_columns(count),
            PageCommand::Break(kind) => self.insert_section_break(kind),
            PageCommand::PageNumber(story, alignment) => self.insert_page_number(story, alignment),
            PageCommand::RemovePageNumbers => self.remove_page_numbers(),
        }
    }

    /// Which 余白 preset the page's margins are, if any.
    fn margin_preset(&self, japanese: bool) -> Option<MarginPreset> {
        let margins = self.page_setup().margins;
        let near = |a: f32, b: f32| (a - b).abs() < 1.0;
        MarginPreset::ALL.into_iter().find(|preset| {
            let wanted = preset.margins(japanese);
            near(wanted.top, margins.top)
                && near(wanted.bottom, margins.bottom)
                && near(wanted.left, margins.left)
                && near(wanted.right, margins.right)
        })
    }

    /// The page of the section the caret is in (the final section without
    /// a caret), as the ribbon shows it: Word's page tools act on that
    /// section.
    fn page_setup(&self) -> PageSetup {
        let section = self.selection_range().and_then(|range| {
            self.word_document()
                .section_index_for_paragraph(range.focus.paragraph)
        });
        let layout = self.word_document().section_layout(section);
        PageSetup {
            width: layout.width,
            height: layout.height,
            landscape: layout.landscape,
            margins: Margins {
                top: layout.margin_top,
                bottom: layout.margin_bottom,
                left: layout.margin_left,
                right: layout.margin_right,
            },
            columns: layout.columns.count.max(1),
        }
    }

    /// 余白: the four margins, the header and footer distances left alone.
    fn set_margins(&mut self, margins: Margins) -> Result<(), String> {
        self.apply_command(
            HeadlessCommand::SetPageMargins {
                top: Some(margins.top),
                bottom: Some(margins.bottom),
                left: Some(margins.left),
                right: Some(margins.right),
                header: None,
                footer: None,
                gutter: None,
            },
            true,
        )
    }

    /// 印刷の向き.
    fn set_landscape(&mut self, landscape: bool) -> Result<(), String> {
        self.apply_command(HeadlessCommand::SetPageOrientation { landscape }, true)
    }

    /// サイズ: the paper, the page kept the way up it is.
    fn set_paper(&mut self, paper: Paper) -> Result<(), String> {
        let (portrait_width, portrait_height) = paper.size();
        self.apply_command(
            HeadlessCommand::SetPageSize {
                portrait_width,
                portrait_height,
            },
            true,
        )
    }

    /// 段組み: equal columns, Word's gutter of 1.5 characters between them
    /// (two for one column, which has none).
    fn set_columns(&mut self, count: u32) -> Result<(), String> {
        self.apply_command(
            HeadlessCommand::SetPageColumns {
                count: count.clamp(1, 12),
                space: 21.25,
                separator: false,
            },
            true,
        )
    }

    /// 区切り → セクション区切り, at the caret.
    fn insert_section_break(&mut self, kind: SectionBreak) -> Result<(), String> {
        let range = self.selection_range().ok_or("no caret")?;
        let (start, _) = range.normalized();
        self.apply_command(
            HeadlessCommand::InsertSectionBreak {
                paragraph: start.paragraph,
                offset: start.offset,
                break_type: match kind {
                    SectionBreak::NextPage => HeadlessSectionBreakType::NextPage,
                    SectionBreak::Continuous => HeadlessSectionBreakType::Continuous,
                    SectionBreak::EvenPage => HeadlessSectionBreakType::EvenPage,
                    SectionBreak::OddPage => HeadlessSectionBreakType::OddPage,
                },
            },
            false,
        )
    }

    /// The first line of the default header or footer: what the ヘッダーの
    /// 編集 field shows. Its tab-separated zones are Word's left, centre and
    /// right.
    fn story_text(&self, story: Story) -> String {
        let role = match story {
            Story::Header => "header",
            Story::Footer => "footer",
        };
        let stories: Vec<HeadlessHeaderFooterSnapshot> =
            match query(self.word_document(), HeadlessQuery::HeaderFooterText) {
                Some(HeadlessQueryResult::HeaderFooterText { stories }) => stories,
                _ => Vec::new(),
            };
        stories
            .into_iter()
            .find(|snapshot| snapshot.role == role && snapshot.variant == "default")
            .and_then(|snapshot| snapshot.lines.into_iter().next())
            .map(|line| line.text)
            .unwrap_or_default()
    }

    /// Sets the first line of the default header or footer, creating the
    /// story when the document has none. One undo step.
    fn set_story_text(&mut self, story: Story, text: &str) -> Result<(), String> {
        let text = text.to_string();
        let command =
            SnapshotEditCommand::new("set_header_footer_text", move |document| match story {
                Story::Header => document.replace_header_paragraph_text(0, &text),
                Story::Footer => document.replace_footer_paragraph_text(0, &text),
            });
        self.execute_command(Box::new(command), true)
    }

    /// ページ番号 → ページの上部 / 下部: a live PAGE field in the header or
    /// footer, in place of what it held, as Word's plain-number gallery does.
    fn insert_page_number(
        &mut self,
        story: Story,
        alignment: NumberAlignment,
    ) -> Result<(), String> {
        let alignment = match alignment {
            NumberAlignment::Left => "left",
            NumberAlignment::Center => "center",
            NumberAlignment::Right => "right",
        };
        let command = SnapshotEditCommand::new("insert_page_number", move |document| {
            document.insert_page_number(story == Story::Footer, alignment)
        });
        self.execute_command(Box::new(command), true)
    }

    /// ページ番号の削除.
    fn remove_page_numbers(&mut self) -> Result<(), String> {
        let command = SnapshotEditCommand::new("remove_page_numbers", |document| {
            document.remove_page_numbers().map(|_| ())
        });
        self.execute_command(Box::new(command), true)
    }
}

impl<T: WordSession + ?Sized> WordPage for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn papers_are_known_either_way_up() {
        assert_eq!(Paper::of(595.3, 841.9), Some(Paper::A4));
        assert_eq!(Paper::of(841.9, 595.3), Some(Paper::A4));
        assert_eq!(Paper::of(612.0, 792.0), Some(Paper::Letter));
        assert_eq!(Paper::of(100.0, 100.0), None);
    }

    #[test]
    fn normal_margins_follow_the_language() {
        assert_eq!(MarginPreset::Normal.margins(true).top, 99.25);
        assert_eq!(MarginPreset::Normal.margins(false).top, 72.0);
        assert_eq!(
            MarginPreset::Narrow.margins(true),
            MarginPreset::Narrow.margins(false)
        );
    }
}
