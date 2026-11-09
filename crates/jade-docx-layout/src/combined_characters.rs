//! 組み文字 and 割注 (`w:eastAsianLayout w:combine`, ECMA-376 Part 1
//! §17.3.2.10) for layout.
//!
//! Word sets a combined run as two half-size lines stacked in one line's
//! height — the first half of its characters over the second — with
//! 割注's brackets (`w:combineBrackets`) full size on either side. On the
//! line it is as wide as its wider half plus the brackets. The renderer draws
//! it as one run at half size, horizontally scaled to that width; layout
//! gives the run the same properties so lines break where the drawing says.

use std::rc::Rc;

use jade_docx_reader::RunProperties;

use crate::line_wrap::{wrap_text_into_lines, LineWrapContext, ParagraphStyle, RunStyle};

/// `w:combineBrackets`' opening and closing brackets.
pub fn combine_brackets(brackets: Option<&str>) -> Option<(&'static str, &'static str)> {
    match brackets? {
        "round" => Some(("(", ")")),
        "square" => Some(("[", "]")),
        "angle" => Some(("<", ">")),
        "curly" => Some(("{", "}")),
        _ => None,
    }
}

/// The natural width of `text` set in `properties`, on one line.
fn natural_width<C: LineWrapContext>(
    ctx: &C,
    text: &str,
    properties: &RunProperties,
) -> Result<f64, C::Error> {
    let runs = [RunStyle {
        start: 0,
        end: text.len(),
        properties: Some(Rc::new(properties.clone())),
        revision: None,
        hyperlink: None,
        inline_object: None,
    }];
    let lines = wrap_text_into_lines(ctx, text, &runs, 0.0, &ParagraphStyle::default(), 0, 1.0e9)?;
    Ok(lines
        .iter()
        .flat_map(|line| line.clusters.iter())
        .map(|cluster| cluster.width)
        .sum())
}

/// The properties a combined run of `text` is laid out with, or `None` when
/// the run is not combined (or is empty, or contains a tab or break).
pub fn combined_run_properties<C: LineWrapContext>(
    ctx: &C,
    text: &str,
    properties: &RunProperties,
) -> Result<Option<RunProperties>, C::Error> {
    // Word gives fitText priority over eastAsianLayout on the same run.
    if properties.fit_text.is_some() {
        return Ok(None);
    }
    let Some(layout) = properties
        .east_asian_layout
        .as_deref()
        .filter(|layout| layout.combine)
    else {
        return Ok(None);
    };
    let characters: Vec<&str> = crate::graphemes::grapheme_indices(text)
        .map(|(_, grapheme)| grapheme)
        .collect();
    if characters.is_empty() || text.contains(['\t', '\n']) {
        return Ok(None);
    }
    // Effective run properties carry the style's size; Word's default
    // otherwise.
    let base = properties.font_size.unwrap_or(10.5);
    let half = base / 2.0;
    // A single character is half size on the surrounding baseline in Word.
    let split = if characters.len() == 1 {
        0
    } else {
        characters.len().div_ceil(2)
    };
    let top: String = characters[..split].concat();
    let bottom: String = characters[split..].concat();
    let mut small = properties.clone();
    small.font_size = Some(half);
    small.font_size_complex = Some(half);
    let top_width = natural_width(ctx, &top, &small)?;
    let bottom_width = natural_width(ctx, &bottom, &small)?;
    // The flattened representation has one extra tracking interval across
    // the split. Scale its full advance, including tracking, to the wider
    // row; measuring the halves' sum leaves that extra interval in layout.
    let total = natural_width(ctx, text, &small)?;
    if total <= f64::EPSILON {
        return Ok(None);
    }
    let bracket_width = match combine_brackets(layout.combine_brackets.as_deref()) {
        Some((open, close)) => {
            natural_width(ctx, open, properties)? + natural_width(ctx, close, properties)?
        }
        None => 0.0,
    };
    let scale = (top_width.max(bottom_width) + bracket_width) / total;
    small.text_scale = Some(properties.text_scale.unwrap_or(100.0) * scale as f32);
    small.char_spacing = properties
        .char_spacing
        .map(|spacing| spacing * scale as f32);
    Ok(Some(small))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font_resolver::{resolve_run_style_for_wrap, ResolvedRunStyle};

    struct FixedMetrics;
    impl LineWrapContext for FixedMetrics {
        type Error = ();
        fn resolve_run_style(
            &self,
            properties: Option<&RunProperties>,
        ) -> Result<Rc<ResolvedRunStyle>, ()> {
            Ok(resolve_run_style_for_wrap(properties, None))
        }
        fn measure_text(&self, text: &str, _font: &str) -> Result<f64, ()> {
            Ok(crate::graphemes::grapheme_indices(text).count() as f64 * 8.0)
        }
    }

    #[test]
    fn combined_spacing_allocates_only_the_wider_rows_width() {
        for text in ["株式会", "株式会社"] {
            for spacing in [-0.75, 1.5, 12.0] {
                for scale in [50.0, 100.0, 200.0] {
                    let properties = RunProperties {
                        font_size: Some(12.0),
                        char_spacing: Some(spacing),
                        text_scale: Some(scale),
                        east_asian_layout: Some(Box::new(
                            jade_docx_reader::document::EastAsianLayout {
                                combine: true,
                                ..Default::default()
                            },
                        )),
                        ..Default::default()
                    };
                    let combined = combined_run_properties(&FixedMetrics, text, &properties)
                        .unwrap()
                        .unwrap();
                    let width = natural_width(&FixedMetrics, text, &combined).unwrap();
                    // Both probes have a two-character wider row. Tracking is
                    // applied between its two glyphs, once, without w:w scaling.
                    let expected = 16.0 * scale as f64 / 100.0 + spacing as f64 * 96.0 / 72.0;
                    assert!((width - expected).abs() < 0.001,
                        "{text}, spacing={spacing}, scale={scale}: width={width}, expected={expected}");
                }
            }
        }
    }
}
