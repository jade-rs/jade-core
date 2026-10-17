//! Measuring text, so wrapping happens against the font that will draw it.
//!
//! Layout runs before painting, so it cannot ask the painter directly. The
//! backends implement this instead: the browser one measures through the
//! canvas, the native one through Skia, and both then wrap at the same places
//! the glyphs will actually land.

use crate::ResolvedFont;

pub trait TextMeasure {
    /// Advance width of `text` in `font`, in px.
    fn measure(&self, text: &str, font: &ResolvedFont) -> f64;
}

/// A deterministic stand-in used when no real measurer is supplied.
///
/// Half an em for anything ASCII and a full em for anything else, which is the
/// right shape for CJK and roughly right for Latin. It exists so layout is
/// testable without a font, and so a caller that forgets to pass a measurer
/// gets plausible wrapping rather than none — the line ends will not match the
/// drawn glyphs, which is why every real backend passes its own.
#[derive(Debug, Clone, Copy, Default)]
pub struct HeuristicMeasure;

impl TextMeasure for HeuristicMeasure {
    fn measure(&self, text: &str, font: &ResolvedFont) -> f64 {
        let em = crate::units::points_to_px(font.size_points);
        text.chars()
            .map(|c| if c.is_ascii() { em * 0.5 } else { em })
            .sum()
    }
}
