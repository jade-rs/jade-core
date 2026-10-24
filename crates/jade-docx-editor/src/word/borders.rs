//! The border styles the Word dialogs offer (線種とページ罫線と網かけの設定).
//!
//! Word's line list is the same in the paragraph, table and page border
//! tabs: every `ST_Border` line style (ECMA-376 Part 1 §17.18.2). The page
//! border tab adds 絵柄 (art borders), whose `w:sz` is a tile size in whole
//! points rather than a line width.

/// Every line style, with its label key, in the order Word's list shows them.
pub const LINE_STYLES: [(&str, &str); 24] = [
    ("single", "word.line.single"),
    ("dotted", "word.line.dotted"),
    ("dashSmallGap", "word.line.dash_small"),
    ("dashed", "word.line.dashed"),
    ("dotDash", "word.line.dot_dash"),
    ("dotDotDash", "word.line.dot_dot_dash"),
    ("double", "word.line.double"),
    ("triple", "word.line.triple"),
    ("thinThickSmallGap", "word.line.thin_thick_small"),
    ("thickThinSmallGap", "word.line.thick_thin"),
    ("thinThickThinSmallGap", "word.line.thin_thick_thin_small"),
    ("thinThickMediumGap", "word.line.thin_thick_medium"),
    ("thickThinMediumGap", "word.line.thick_thin_medium"),
    ("thinThickThinMediumGap", "word.line.thin_thick_thin_medium"),
    ("thinThickLargeGap", "word.line.thin_thick_large"),
    ("thickThinLargeGap", "word.line.thick_thin_large"),
    ("thinThickThinLargeGap", "word.line.thin_thick_thin_large"),
    ("wave", "word.line.wave"),
    ("doubleWave", "word.line.double_wave"),
    ("dashDotStroked", "word.line.dash_dot_stroked"),
    ("threeDEmboss", "word.line.emboss"),
    ("threeDEngrave", "word.line.engrave"),
    ("outset", "word.line.outset"),
    ("inset", "word.line.inset"),
];

/// The art borders the page border tab offers, with their label keys: a
/// representative set of Word's 絵柄 list.
pub const ART_STYLES: [(&str, &str); 16] = [
    ("apples", "word.art.apples"),
    ("hearts", "word.art.hearts"),
    ("stars", "word.art.stars"),
    ("starsBlack", "word.art.stars_black"),
    ("basicBlackDots", "word.art.dots"),
    ("basicBlackSquares", "word.art.squares"),
    ("basicBlackDashes", "word.art.dashes"),
    ("basicThinLines", "word.art.thin_lines"),
    ("checkered", "word.art.checkered"),
    ("christmasTree", "word.art.christmas_tree"),
    ("flowersTiny", "word.art.flowers"),
    ("balloons3Colors", "word.art.balloons"),
    ("sun", "word.art.sun"),
    ("snowflakes", "word.art.snowflakes"),
    ("zigZag", "word.art.zigzag"),
    ("triangles", "word.art.triangles"),
];

/// 線の太さ for an art border, in points (Word allows 1–31).
pub const ART_WIDTHS: [f32; 7] = [8.0, 10.0, 12.0, 16.0, 20.0, 24.0, 31.0];

/// The art border width Word starts from.
pub const DEFAULT_ART_WIDTH: f32 = 20.0;

/// The label key of `style`: a line style, an art border, or `thick`.
pub fn style_label(style: &str) -> &'static str {
    if style == "thick" {
        return "word.line.thick";
    }
    LINE_STYLES
        .iter()
        .chain(ART_STYLES.iter())
        .find(|(value, _)| value.eq_ignore_ascii_case(style))
        .map(|(_, label)| *label)
        .unwrap_or("word.line.single")
}

/// Whether `style` is one of the art borders offered.
pub fn is_art(style: &str) -> bool {
    ART_STYLES
        .iter()
        .any(|(value, _)| value.eq_ignore_ascii_case(style))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_line_style_has_its_own_label() {
        let mut labels: Vec<_> = LINE_STYLES.iter().map(|(_, label)| *label).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), LINE_STYLES.len());
        assert_eq!(
            style_label("thinThickSmallGap"),
            "word.line.thin_thick_small"
        );
        assert_eq!(style_label("hearts"), "word.art.hearts");
        assert_eq!(style_label("thick"), "word.line.thick");
        assert!(is_art("hearts") && !is_art("double"));
    }
}
