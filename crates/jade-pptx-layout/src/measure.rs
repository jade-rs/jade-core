//! Text measurement, so wrapping happens against the font that will draw it.
//!
//! Layout runs before painting and cannot ask the painter; backends implement
//! this instead — the same arrangement `jade-xlsx-layout` uses.

use crate::PX_PER_PT;

/// The style facts that change a run's advance width.
#[derive(Debug, Clone, PartialEq)]
pub struct RunStyle {
    pub size_pt: f32,
    pub bold: bool,
    pub italic: bool,
    pub font: Option<String>,
    /// The face East Asian characters draw with, when the run names one.
    /// Carried here — not only on the placed run — so a measurer that asks
    /// the same backend the painter draws with can name both faces and get
    /// the width the glyphs will actually take.
    pub east_asian_font: Option<String>,
}

pub trait TextMeasure {
    /// Advance width of `text` in `style`, in px.
    fn measure(&self, text: &str, style: &RunStyle) -> f64;

    /// Advance to a character boundary with the following text's context.
    /// Shaping backends can override this with their cluster positions.
    fn caret_advance(&self, text: &str, byte: usize, style: &RunStyle) -> f64 {
        let full = self.measure(text, style);
        (full - self.measure(&text[byte..], style)).clamp(0.0, full.max(0.0))
    }
}

/// A deterministic stand-in used when no real measurer is supplied: half an
/// em for everything up to the East Asian blocks — Latin, punctuation, the
/// bullet glyphs — and a full em for CJK and fullwidth forms. Right for CJK,
/// roughly right for Latin, and above all stable, which is what tests want.
#[derive(Debug, Clone, Copy, Default)]
pub struct HeuristicMeasure;

impl HeuristicMeasure {
    /// Whether a character takes a full em in this approximation.
    pub fn is_wide(c: char) -> bool {
        let code = c as u32;
        // CJK radicals onward (kana, kanji, hangul), and the fullwidth forms;
        // halfwidth katakana (U+FF61–U+FF9F) stay narrow.
        code >= 0x2E80 && !(0xFF61..=0xFF9F).contains(&code) || (0x1100..0x1200).contains(&code)
    }
}

impl TextMeasure for HeuristicMeasure {
    fn measure(&self, text: &str, style: &RunStyle) -> f64 {
        let em = style.size_pt as f64 * PX_PER_PT;
        text.chars()
            .map(|c| if Self::is_wide(c) { em } else { em * 0.5 })
            .sum()
    }
}
