//! Word's character formatting beyond the ribbon's toggles, over any
//! [`WordSession`]: the フォント dialog (⌘D) with its フォント and 詳細設定
//! tabs, the ribbon's underline styles, theme colours and custom colours,
//! 文字の網かけ and 囲み線, 文字種の変換 (Change Case, 全角 / 半角) and the
//! 拡張書式 commands that are run formatting (縦中横, 組み文字, 割注, 文字の
//! 拡大/縮小).
//!
//! Every change is one `set_run_format` command — one undo step — carrying
//! only what the dialog changed, so a property the user left alone keeps
//! whatever each run had.

use jade_docx_headless_api::HeadlessCommand;
pub use jade_docx_headless_api::{
    HeadlessChangeCaseMode, HeadlessEastAsianLayout, HeadlessRunFormat, HeadlessThemeColor,
};

use super::{Helpers, Position, WordSession};
#[cfg(test)]
use crate::font_format::parse_hex;

pub use crate::font_format::*;

/// The commands, for every session.
pub trait WordCharacters: WordSession {
    /// The formatting of the run at the caret, or at the start of the
    /// selection: what the dialog opens on.
    fn run_format_at(&self, (paragraph, offset): Position) -> HeadlessRunFormat {
        self.word_document().run_format_at(paragraph, offset)
    }

    /// What the フォント dialog opens on.
    fn character_format(&self) -> HeadlessRunFormat {
        self.run_format_at(self.format_anchor())
    }

    /// Opens the フォント dialog on the selection.
    fn font_dialog(&self) -> FontDialog {
        if let Some(range) = self.selection_range().filter(|range| !range.is_collapsed()) {
            let (start, end) = range.normalized();
            FontDialog::open_selection(
                self.word_document().run_formats_in_range(
                    (start.paragraph, start.offset),
                    (end.paragraph, end.offset),
                ),
            )
        } else {
            FontDialog::open(self.character_format())
        }
    }

    /// Applies a patch to the selection — or the word at a bare caret — as
    /// one undo step. An empty patch changes nothing.
    fn apply_run_format(&mut self, format: HeadlessRunFormat) -> Result<(), String> {
        if format.is_empty() {
            return Ok(());
        }
        self.run_on_span(|start, end| HeadlessCommand::SetRunFormat {
            start_paragraph: start.0,
            start_offset: start.1,
            end_paragraph: end.0,
            end_offset: end.1,
            format: Box::new(format),
        })
    }

    /// The dialog's OK.
    fn apply_font_dialog(&mut self, dialog: &FontDialog) -> Result<(), String> {
        self.apply_run_format(dialog.change())
    }

    /// The ribbon's underline list: a style, or `none`.
    fn set_underline_style(&mut self, kind: &str) -> Result<(), String> {
        self.apply_run_format(HeadlessRunFormat {
            underline: Some(kind.to_string()),
            ..Default::default()
        })
    }

    /// 下線の色: `RRGGBB` or `auto`. An underline is put on when there is
    /// none, as Word does.
    fn set_underline_color(&mut self, color: &str) -> Result<(), String> {
        let underlined = self
            .character_format()
            .underline
            .is_some_and(|kind| kind != "none");
        self.apply_run_format(HeadlessRunFormat {
            underline: (!underlined).then(|| "single".to_string()),
            underline_color: Some(color.trim_start_matches('#').to_string()),
            ..Default::default()
        })
    }

    /// A テーマの色 swatch: the colour it draws, with its theme reference.
    fn set_theme_font_color(&mut self, swatch: &ThemeSwatch) -> Result<(), String> {
        self.apply_run_format(HeadlessRunFormat {
            color: Some(swatch.color.clone()),
            theme_color: Some(swatch.theme.clone()),
            ..Default::default()
        })
    }

    /// 文字の網かけ: Word's button lays a 15 % grey; again takes it off.
    fn toggle_character_shading(&mut self) -> Result<(), String> {
        let shaded = self
            .character_format()
            .shading
            .is_some_and(|fill| fill != "none");
        self.apply_run_format(HeadlessRunFormat {
            shading: Some(if shaded { "none" } else { "D9D9D9" }.to_string()),
            ..Default::default()
        })
    }

    /// 囲み線: a box round the text, or none.
    fn toggle_character_border(&mut self) -> Result<(), String> {
        let boxed = self.character_format().border == Some(true);
        self.apply_run_format(HeadlessRunFormat {
            border: Some(!boxed),
            ..Default::default()
        })
    }

    /// 文字の拡大/縮小: the width in percent.
    fn set_character_scale(&mut self, percent: f32) -> Result<(), String> {
        self.apply_run_format(HeadlessRunFormat {
            scale: Some(percent),
            ..Default::default()
        })
    }

    /// 縦中横, 組み文字, 割注, or none of them.
    fn set_asian_layout(&mut self, layout: AsianLayout) -> Result<(), String> {
        self.apply_run_format(HeadlessRunFormat {
            east_asian_layout: Some(layout.layout()),
            ..Default::default()
        })
    }

    /// 文字種の変換: changes the selection's text, keeping its runs'
    /// formatting. Needs a selection, as Word's does.
    fn change_case(&mut self, mode: HeadlessChangeCaseMode) -> Result<(), String> {
        let range = self
            .selection_range()
            .filter(|range| !range.is_collapsed())
            .ok_or_else(|| "select the text to change".to_string())?;
        let (start, end) = range.normalized();
        self.apply_command(
            HeadlessCommand::ChangeCase {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                mode,
            },
            false,
        )
    }
}

impl<T: WordSession + ?Sized> WordCharacters for T {}

#[cfg(test)]
mod tests {
    #[test]
    fn the_highlight_palette_offers_every_st_highlight_color() {
        let names: Vec<_> = super::HIGHLIGHT_COLORS
            .iter()
            .map(|(name, _)| *name)
            .collect();
        for value in [
            "black",
            "blue",
            "cyan",
            "green",
            "magenta",
            "red",
            "yellow",
            "white",
            "darkBlue",
            "darkCyan",
            "darkGreen",
            "darkMagenta",
            "darkRed",
            "darkYellow",
            "darkGray",
            "lightGray",
        ] {
            assert!(names.contains(&value), "{value} cannot be chosen");
        }
    }

    use super::*;

    #[test]
    fn stylistic_set_choices_preserve_other_sets_and_allow_default() {
        let mut dialog = FontDialog::open(HeadlessRunFormat {
            stylistic_set: Some(1),
            stylistic_sets: Some(vec![1, 4]),
            ..Default::default()
        });
        assert_eq!(dialog.change(), HeadlessRunFormat::default());
        dialog.toggle_stylistic_set(20);
        assert_eq!(dialog.change().stylistic_sets, Some(vec![1, 4, 20]));
        dialog.toggle_stylistic_set(4);
        assert_eq!(dialog.change().stylistic_sets, Some(vec![1, 20]));
        dialog.toggle_stylistic_set(1);
        dialog.toggle_stylistic_set(20);
        assert_eq!(dialog.change().stylistic_sets, Some(vec![]));
        assert_eq!(dialog.change().stylistic_set, Some(0));
        let before = dialog.change();
        dialog.toggle_stylistic_set(21);
        assert_eq!(dialog.change(), before);
        let mut legacy = FontDialog::open(HeadlessRunFormat {
            stylistic_set: Some(3),
            ..Default::default()
        });
        legacy.toggle_stylistic_set(4);
        assert_eq!(legacy.change().stylistic_sets, Some(vec![3, 4]));
    }

    #[test]
    fn the_theme_palette_tints_and_shades_as_word_writes_them() {
        let palette = theme_palette(&[("accent1".to_string(), "4472C4".to_string())]);
        assert_eq!(palette.len(), 6);
        assert!(palette.iter().all(|row| row.len() == 10));
        let accent = &palette[0][4];
        assert_eq!(accent.color, "4472C4");
        assert_eq!(accent.theme.name, "accent1");
        // 80 % lighter is tint 33 and draws Word's D9E2F3 (give or take a
        // rounding step); 50 % darker is shade 80.
        let lighter = &palette[1][4];
        assert_eq!(lighter.theme.tint.as_deref(), Some("33"));
        let (r, g, b) = parse_hex(&lighter.color).unwrap();
        let (wr, wg, wb) = parse_hex("D9E2F3").unwrap();
        assert!((r - wr).abs() < 0.02 && (g - wg).abs() < 0.02 && (b - wb).abs() < 0.02);
        assert_eq!(palette[5][4].theme.shade.as_deref(), Some("80"));
        // White only darkens, black only lightens.
        assert_eq!(palette[1][0].theme.shade.as_deref(), Some("F2"));
        assert_eq!(palette[1][1].theme.tint.as_deref(), Some("80"));
        assert_eq!(palette[1][1].color, "7F7F7F", "Word writes 7F7F7F");
    }

    #[test]
    fn mixed_values_are_blank_and_explicit_off_is_a_change() {
        let mut dialog = FontDialog::open_selection([
            HeadlessRunFormat {
                bold: Some(true),
                font_size: Some(12.0),
                ..Default::default()
            },
            HeadlessRunFormat {
                font_size: Some(18.0),
                ..Default::default()
            },
            HeadlessRunFormat {
                bold: Some(true),
                font_size: Some(12.0),
                ..Default::default()
            },
        ]);
        assert!(dialog.is_mixed("bold"));
        assert!(dialog.is_mixed("font_size"));
        assert_eq!(dialog.format.bold, None);
        assert_eq!(dialog.format.font_size, None);
        assert_eq!(dialog.effect_state(Effect::Strike), Some(false));
        assert!(dialog.change().is_empty());
        dialog.format.bold = Some(false);
        assert!(!dialog.is_mixed("bold"));
        assert_eq!(
            dialog.change(),
            HeadlessRunFormat {
                bold: Some(false),
                ..Default::default()
            }
        );
    }

    #[test]
    fn implicit_and_explicit_defaults_are_not_mixed() {
        let dialog = FontDialog::open_selection([
            HeadlessRunFormat::default(),
            HeadlessRunFormat {
                bold: Some(false),
                underline: Some("none".into()),
                spacing: Some(0.0),
                stylistic_sets: Some(vec![]),
                ..Default::default()
            },
        ]);
        assert!(!dialog.is_mixed("bold"));
        assert!(!dialog.is_mixed("spacing"));
        assert!(!dialog.is_mixed("stylistic_sets"));
        assert!(dialog.change().is_empty());
    }

    #[test]
    fn the_dialog_applies_only_what_changed() {
        let mut dialog = FontDialog::open(HeadlessRunFormat {
            bold: Some(true),
            font_size: Some(10.5),
            ..Default::default()
        });
        assert!(dialog.change().is_empty());
        dialog.set_effect(Effect::DoubleStrike, true);
        dialog.format.emphasis = Some("dot".into());
        dialog.step_spacing(1.5);
        let change = dialog.change();
        assert_eq!(change.double_strike, Some(true));
        assert_eq!(change.emphasis.as_deref(), Some("dot"));
        assert_eq!(change.spacing, Some(1.5));
        // Untouched: the bold and the size stay each run's own.
        assert_eq!(change.bold, None);
        assert_eq!(change.font_size, None);
        // Exclusive pairs, as Word keeps them.
        dialog.set_effect(Effect::Strike, true);
        assert!(!dialog.effect(Effect::DoubleStrike));
        dialog.set_effect(Effect::Emboss, true);
        dialog.set_effect(Effect::Engrave, true);
        assert!(!dialog.effect(Effect::Emboss));
    }

    #[test]
    fn the_font_lists_put_japanese_fonts_in_their_own_list() {
        let installed = [
            "Arial",
            "Hiragino Mincho ProN",
            "ヒラギノ角ゴシック",
            "Times New Roman",
            ".SF NS",
        ]
        .map(String::from);
        let (japanese, latin) = font_lists(&installed);
        assert!(japanese.contains(&"Hiragino Mincho ProN".to_string()));
        assert!(!japanese.contains(&"Arial".to_string()));
        assert!(latin.contains(&"Arial".to_string()));
        assert!(!latin.iter().any(|name| name.starts_with('.')));
        assert_eq!(latin[0], "Times New Roman", "Word's common faces first");
        // Without a list from the system, Word's usual families.
        let (japanese, _) = font_lists(&[]);
        assert_eq!(japanese[0], "游明朝");
    }
}
