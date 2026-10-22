//! Word's ハイフネーション: レイアウト ▸ ハイフネーション's なし / 自動, and
//! the ハイフネーションのオプション dialog — the document's automatic
//! hyphenation, its zone, the limit on consecutive hyphens and whether words
//! in capitals are hyphenated. A paragraph opts out with the 段落 dialog's
//! 段落の自動ハイフネーションを行わない (`paragraph_options`).

use jade_docx_headless_api::{HeadlessCommand, HeadlessHyphenationPatch};

use super::page_setup::{mm_to_points, points_to_mm};
use super::WordSession;

/// What the ハイフネーション menu offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HyphenationChoice {
    /// なし.
    Off,
    /// 自動.
    Automatic,
    /// ハイフネーションのオプション...
    Options,
}

impl HyphenationChoice {
    pub const ALL: [HyphenationChoice; 3] = [
        HyphenationChoice::Off,
        HyphenationChoice::Automatic,
        HyphenationChoice::Options,
    ];

    pub fn label_key(self) -> &'static str {
        match self {
            HyphenationChoice::Off => "hyphenation.none",
            HyphenationChoice::Automatic => "hyphenation.automatic",
            HyphenationChoice::Options => "hyphenation.options",
        }
    }
}

/// The ハイフネーション dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct HyphenationDialog {
    /// 文書を自動的にハイフネーションする.
    pub auto: bool,
    /// 英大文字の単語をハイフネーションする.
    pub hyphenate_caps: bool,
    /// ハイフネーション ゾーン, as typed, in millimetres.
    pub zone: String,
    /// 連続するハイフンの制限, as typed: blank or 0 for 制限なし.
    pub limit: String,
}

impl HyphenationDialog {
    /// The dialog on the document's settings.
    pub fn open(settings: &crate::document::HyphenationSettings) -> Self {
        Self {
            auto: settings.auto,
            hyphenate_caps: !settings.do_not_hyphenate_caps,
            zone: format_mm(points_to_mm(settings.zone_pt())),
            limit: settings
                .consecutive_limit
                .filter(|limit| *limit > 0)
                .map(|limit| limit.to_string())
                .unwrap_or_default(),
        }
    }

    /// What OK applies, or why it cannot.
    pub fn patch(&self) -> Result<HeadlessHyphenationPatch, &'static str> {
        let zone = self
            .zone
            .trim()
            .trim_end_matches("mm")
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|mm| mm.is_finite() && (0.0..=558.8).contains(mm))
            .ok_or("hyphenation.bad_zone")?;
        let limit = match self.limit.trim() {
            "" => 0,
            text => text
                .parse::<u32>()
                .ok()
                .filter(|limit| *limit <= 32767)
                .ok_or("hyphenation.bad_limit")?,
        };
        Ok(HeadlessHyphenationPatch {
            auto: Some(self.auto),
            zone: Some(mm_to_points(zone)),
            consecutive_limit: Some(limit),
            hyphenate_caps: Some(self.hyphenate_caps),
        })
    }
}

/// A length in millimetres as the dialog shows it: one decimal, without a
/// trailing `.0`.
fn format_mm(mm: f32) -> String {
    let rounded = (mm * 100.0).round() / 100.0;
    let text = format!("{rounded:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The document's hyphenation, for any Word session.
pub trait WordHyphenation: WordSession {
    /// The document's settings.
    fn hyphenation_settings(&self) -> crate::document::HyphenationSettings {
        self.word_document().hyphenation()
    }

    /// Which of なし / 自動 the menu ticks.
    fn hyphenation_choice(&self) -> HyphenationChoice {
        if self.hyphenation_settings().auto {
            HyphenationChoice::Automatic
        } else {
            HyphenationChoice::Off
        }
    }

    /// なし or 自動, as one undo step.
    fn set_auto_hyphenation(&mut self, on: bool) -> Result<(), String> {
        self.apply_hyphenation(HeadlessHyphenationPatch {
            auto: Some(on),
            ..Default::default()
        })
    }

    /// Opens the ハイフネーション dialog.
    fn hyphenation_dialog(&self) -> HyphenationDialog {
        HyphenationDialog::open(&self.hyphenation_settings())
    }

    /// The dialog's OK. A value the dialog cannot take comes back as its
    /// message key, and nothing changes.
    fn apply_hyphenation_dialog(&mut self, dialog: &HyphenationDialog) -> Result<(), String> {
        let patch = dialog.patch().map_err(str::to_string)?;
        self.apply_hyphenation(patch)
    }

    /// Applies `patch` as one undo step.
    fn apply_hyphenation(&mut self, patch: HeadlessHyphenationPatch) -> Result<(), String> {
        self.apply_command(HeadlessCommand::SetHyphenation { patch }, true)
    }
}

impl<T: WordSession + ?Sized> WordHyphenation for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dialog_shows_words_defaults_in_millimetres() {
        let dialog = HyphenationDialog::open(&Default::default());
        assert!(!dialog.auto && dialog.hyphenate_caps);
        assert_eq!(dialog.zone, "6.35");
        assert_eq!(dialog.limit, "");
        let patch = dialog.patch().unwrap();
        assert_eq!(patch.consecutive_limit, Some(0));
        assert!((patch.zone.unwrap() - 18.0).abs() < 0.01);
    }

    #[test]
    fn values_it_cannot_take_are_refused() {
        let mut dialog = HyphenationDialog::open(&Default::default());
        dialog.zone = "wide".into();
        assert_eq!(dialog.patch(), Err("hyphenation.bad_zone"));
        dialog.zone = "10".into();
        dialog.limit = "-1".into();
        assert_eq!(dialog.patch(), Err("hyphenation.bad_limit"));
        dialog.limit = "3".into();
        assert_eq!(dialog.patch().unwrap().consecutive_limit, Some(3));
    }
}
