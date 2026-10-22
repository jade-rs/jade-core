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
/// the run is not combined (or has fewer than two characters, or a tab or
/// break, which Word does not combine).
pub fn combined_run_properties<C: LineWrapContext>(
    ctx: &C,
    text: &str,
    properties: &RunProperties,
) -> Result<Option<RunProperties>, C::Error> {
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
    if characters.len() < 2 || text.contains(['\t', '\n']) {
        return Ok(None);
    }
    // Effective run properties carry the style's size; Word's default
    // otherwise.
    let base = properties.font_size.unwrap_or(10.5);
    let half = base / 2.0;
    let split = characters.len().div_ceil(2);
    let top: String = characters[..split].concat();
    let bottom: String = characters[split..].concat();
    let mut small = properties.clone();
    small.font_size = Some(half);
    small.font_size_complex = Some(half);
    let top_width = natural_width(ctx, &top, &small)?;
    let bottom_width = natural_width(ctx, &bottom, &small)?;
    let total = top_width + bottom_width;
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
    Ok(Some(small))
}
