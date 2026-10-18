//! Ruby (phonetic guide, `w:ruby`, ECMA-376 Part 1 §17.3.3.25) geometry.
//!
//! Word for Mac 16.114 (probes `ruby-20261018`, `ruby2-20261018`) lays a ruby
//! out as one box as wide as the wider of its base and its guide. The narrower
//! of the two is spread over that width by `w:rubyAlign`, the guide baseline
//! sits `w:hpsRaise` above the base baseline in whole points, and the line's
//! ascent grows to hold the guide.

use jade_docx_reader::document::{RubyAnnotation, RunProperties};

/// Space before the first character and between characters when `count`
/// characters are spread over `slack` extra width by `w:rubyAlign`
/// (§17.3.3.27). Word applies the same rule to whichever of the base and the
/// guide is narrower. Unknown or missing alignments centre.
pub fn ruby_distribution(align: Option<&str>, count: usize, slack: f64) -> (f64, f64) {
    if count == 0 || !slack.is_finite() || slack <= 0.0 {
        return (0.0, 0.0);
    }
    match align {
        Some("left") => (0.0, 0.0),
        Some("right") => (slack, 0.0),
        Some("distributeletter") if count > 1 => (0.0, slack / (count - 1) as f64),
        Some("distributespace") => {
            let gap = slack / count as f64;
            (gap / 2.0, gap)
        }
        _ => (slack / 2.0, 0.0),
    }
}

/// Guide baseline raise in points. Word drops the half point of an odd
/// `w:hpsRaise` (17 → 8pt, 21 → 10pt, 23 → 11pt).
pub fn ruby_raise_pt(raise_half_points: u32) -> f32 {
    (raise_half_points / 2) as f32
}

/// Guide font size (`w:rt` run size, else `w:rubyPr/w:hps`); without one,
/// half the base size. Degenerate sizes fall back to that default, and the
/// result never goes below 1pt.
pub fn ruby_font_size_pt(base_font_size_pt: f32, explicit_pt: Option<f32>) -> f32 {
    explicit_pt
        .filter(|size| size.is_finite() && *size > 0.0)
        .unwrap_or(base_font_size_pt * 0.5)
        .max(1.0)
}

/// Formatting the guide is measured and drawn with: the base run's faces and
/// language at the guide size, with the faces the guide run names itself.
pub fn ruby_run_properties(
    base: Option<&RunProperties>,
    base_font_size_pt: f32,
    ruby: &RubyAnnotation,
) -> RunProperties {
    let mut properties = RunProperties {
        font_size: Some(ruby_font_size_pt(base_font_size_pt, ruby.size_pt)),
        ..RunProperties::default()
    };
    if let Some(base) = base {
        properties.font_family = base.font_family.clone();
        properties.fonts = base.fonts.clone();
        properties.language = base.language.clone();
        properties.east_asia_language = base.east_asia_language.clone();
        properties.color = base.color.clone();
    }
    if let Some(guide) = ruby.fonts.as_ref() {
        let fonts = properties.fonts.get_or_insert_with(Default::default);
        for (target, source) in [
            (&mut fonts.ascii, &guide.ascii),
            (&mut fonts.h_ansi, &guide.h_ansi),
            (&mut fonts.east_asia, &guide.east_asia),
            (&mut fonts.complex, &guide.complex),
        ] {
            if source.is_some() {
                *target = source.clone();
            }
        }
    }
    properties
}

#[cfg(test)]
mod tests {
    use super::*;
    use jade_docx_reader::document::RunFonts;
    use std::rc::Rc;

    #[test]
    fn distribution_matches_word_for_every_alignment() {
        // ruby2 probe: 漢字 (21pt) under かんじかんじ (33pt), 12pt of slack.
        assert_eq!(ruby_distribution(Some("center"), 2, 12.0), (6.0, 0.0));
        assert_eq!(ruby_distribution(Some("left"), 2, 12.0), (0.0, 0.0));
        assert_eq!(ruby_distribution(Some("right"), 2, 12.0), (12.0, 0.0));
        assert_eq!(
            ruby_distribution(Some("distributeletter"), 2, 12.0),
            (0.0, 12.0)
        );
        assert_eq!(
            ruby_distribution(Some("distributespace"), 2, 12.0),
            (3.0, 6.0)
        );
        // とう (11pt) over 東京都特許 (52.5pt): the guide is spread instead.
        assert_eq!(
            ruby_distribution(Some("distributespace"), 2, 41.5),
            (10.375, 20.75)
        );
        // A single character centres under distributeLetter.
        assert_eq!(
            ruby_distribution(Some("distributeletter"), 1, 6.0),
            (3.0, 0.0)
        );
        assert_eq!(ruby_distribution(None, 3, 4.5), (2.25, 0.0));
        assert_eq!(ruby_distribution(Some("center"), 2, -1.0), (0.0, 0.0));
    }

    #[test]
    fn raise_drops_the_half_point() {
        assert_eq!(ruby_raise_pt(17), 8.0);
        assert_eq!(ruby_raise_pt(18), 9.0);
        assert_eq!(ruby_raise_pt(21), 10.0);
        assert_eq!(ruby_raise_pt(23), 11.0);
        assert_eq!(ruby_raise_pt(30), 15.0);
    }

    #[test]
    fn font_size_prefers_explicit_size_and_defaults_to_half_base() {
        assert_eq!(ruby_font_size_pt(12.0, Some(7.0)), 7.0);
        assert_eq!(ruby_font_size_pt(12.0, None), 6.0);
        assert_eq!(ruby_font_size_pt(12.0, Some(0.0)), 6.0);
        assert_eq!(ruby_font_size_pt(12.0, Some(f32::NAN)), 6.0);
        assert_eq!(ruby_font_size_pt(1.0, None), 1.0);
    }

    #[test]
    fn guide_takes_its_own_faces_over_the_base_faces() {
        let base = RunProperties {
            font_size: Some(10.5),
            bold: Some(true),
            underline: Some(true),
            fonts: Some(RunFonts {
                ascii: Some(Rc::from("Century")),
                east_asia: Some(Rc::from("MS Mincho")),
                ..RunFonts::default()
            }),
            ..RunProperties::default()
        };
        let ruby = RubyAnnotation {
            text: "じゆん".into(),
            size_pt: Some(5.5),
            fonts: Some(RunFonts {
                east_asia: Some(Rc::from("Noto Sans JP")),
                ..RunFonts::default()
            }),
            ..RubyAnnotation::default()
        };
        let guide = ruby_run_properties(Some(&base), 10.5, &ruby);
        assert_eq!(guide.font_size, Some(5.5));
        let fonts = guide.fonts.expect("fonts");
        assert_eq!(fonts.ascii.as_deref(), Some("Century"));
        assert_eq!(fonts.east_asia.as_deref(), Some("Noto Sans JP"));
        assert_eq!(guide.bold, None, "the base's emphasis stays on the base");
        assert_eq!(guide.underline, None);
    }
}
