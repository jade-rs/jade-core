//! Morph (`<p159:morph option>`): objects that appear on both slides move,
//! turn, resize and recolour from one to the other; the rest fade.
//! Objects match by name — a `!!` prefix forces a match — then by what
//! they hold (the same picture, the same text).

use jade_painter::Painter;
use jade_pptx_layout::{PlacedLine, PlacedRun, PlacedShape, Rect, SlideLayout};
use unicode_segmentation::UnicodeSegmentation;

use crate::animation::evaluate::{parse_hex, to_hex};
use crate::animation::frame::draw_faded;
use crate::{font_shorthand, render_background, DrawImage, RenderOptions};

/// 効果のオプション of Morph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MorphOption {
    #[default]
    ByObject,
    ByWord,
    ByChar,
}

impl MorphOption {
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("byWord") => Self::ByWord,
            Some("byChar") => Self::ByChar,
            _ => Self::ByObject,
        }
    }
}

type MatchRule = dyn Fn(&PlacedShape, &PlacedShape) -> bool;

/// Pairs of (from, to) shape indices that morph into each other.
pub fn match_shapes(from: &SlideLayout, to: &SlideLayout) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    let mut taken = vec![false; to.shapes.len()];
    let rules: [&MatchRule; 4] = [
        &|a, b| a.name.starts_with("!!") && a.name == b.name,
        &|a, b| !a.name.is_empty() && a.name == b.name && a.geometry == b.geometry,
        &|a, b| a.image_part.is_some() && a.image_part == b.image_part,
        &|a, b| {
            let text = shape_text(a);
            !text.is_empty() && text == shape_text(b)
        },
    ];
    let mut matched_from = vec![false; from.shapes.len()];
    for (rule_index, rule) in rules.iter().enumerate() {
        for (i, a) in from.shapes.iter().enumerate() {
            // A `!!` name matches only its namesake.
            if matched_from[i] || (rule_index > 0 && a.name.starts_with("!!")) {
                continue;
            }
            if let Some(j) = to
                .shapes
                .iter()
                .enumerate()
                .position(|(j, b)| !taken[j] && rule(a, b))
            {
                taken[j] = true;
                matched_from[i] = true;
                pairs.push((i, j));
            }
        }
    }
    pairs
}

fn shape_text(shape: &PlacedShape) -> String {
    shape
        .lines
        .iter()
        .flat_map(|line| &line.runs)
        .map(|run| run.text.as_str())
        .collect()
}

/// A layout `p` of the way between two shapes' geometry and colours.
pub fn morph_shape(a: &PlacedShape, b: &PlacedShape, p: f64) -> PlacedShape {
    let mut shape = if p < 0.5 { a.clone() } else { b.clone() };
    shape.rect = lerp_rect(a.rect, b.rect, p);
    shape.local_rect = match (a.local_rect, b.local_rect) {
        (Some(x), Some(y)) => Some(lerp_rect(x, y, p)),
        _ => None,
    };
    if a.local_rect.is_none() || b.local_rect.is_none() {
        shape.group_transforms.clear();
        shape.local_rect = None;
    }
    shape.rotation_rad = lerp(a.rotation_rad, b.rotation_rad, p);
    shape.fill = mix_color(a.fill.as_deref(), b.fill.as_deref(), p).or(shape.fill);
    if let (Some(outline), Some(x), Some(y)) = (
        shape.outline.as_mut(),
        a.outline.as_ref(),
        b.outline.as_ref(),
    ) {
        if let Some(color) = mix_color(Some(&x.color), Some(&y.color), p) {
            outline.color = color;
        }
        outline.width_px = lerp(x.width_px, y.width_px, p);
    }
    shape
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn lerp_rect(a: Rect, b: Rect, t: f64) -> Rect {
    Rect {
        x: lerp(a.x, b.x, t),
        y: lerp(a.y, b.y, t),
        width: lerp(a.width, b.width, t),
        height: lerp(a.height, b.height, t),
    }
}

fn mix_color(a: Option<&str>, b: Option<&str>, t: f64) -> Option<String> {
    let (a, b) = (parse_hex(a?)?, parse_hex(b?)?);
    Some(to_hex((
        lerp(a.0, b.0, t),
        lerp(a.1, b.1, t),
        lerp(a.2, b.2, t),
    )))
}

/// The pairs' layouts with unmatched shapes left out — what a show
/// without word-level text morphing draws, and what tests check.
pub fn morph_layouts(from: &SlideLayout, to: &SlideLayout, p: f64) -> SlideLayout {
    let mut layout = to.clone();
    layout.shapes = match_shapes(from, to)
        .into_iter()
        .map(|(i, j)| morph_shape(&from.shapes[i], &to.shapes[j], p))
        .collect();
    layout
}

/// One word or letter of a shape's text, where the slide draws it.
#[derive(Debug, Clone)]
struct Piece {
    text: String,
    run: PlacedRun,
    /// Slide px of the run's left edge and baseline.
    x: f64,
    baseline: f64,
    line_top: f64,
    line_height: f64,
}

fn pieces<P: Painter>(
    painter: &P,
    shape: &PlacedShape,
    by_char: bool,
    options: &RenderOptions,
) -> Vec<Piece> {
    let mut out = Vec::new();
    for line in &shape.lines {
        for run in &line.runs {
            painter.set_font(&font_shorthand(run, options));
            let measured = painter
                .measure_text_width(&run.text)
                .unwrap_or(run.width)
                .max(1e-6);
            let stretch = if run.width > 0.0 {
                run.width / measured
            } else {
                1.0
            };
            let units: Vec<(usize, &str)> = if by_char {
                run.text.grapheme_indices(true).collect()
            } else {
                run.text.split_word_bound_indices().collect()
            };
            for (at, unit) in units {
                if unit.trim().is_empty() {
                    continue;
                }
                let before = painter.measure_text_width(&run.text[..at]).unwrap_or(0.0) * stretch;
                let width = painter.measure_text_width(unit).unwrap_or(0.0) * stretch;
                let mut piece_run = run.clone();
                piece_run.text = unit.to_string();
                piece_run.width = width;
                piece_run.source_offsets = Vec::new();
                piece_run.x = 0.0;
                out.push(Piece {
                    text: unit.to_string(),
                    run: piece_run,
                    x: shape.rect.x + run.x + before,
                    baseline: shape.rect.y + run.baseline_y,
                    line_top: line.top,
                    line_height: line.height,
                });
            }
        }
    }
    out
}

fn piece_shape(template: &PlacedShape, piece: &Piece, x: f64, baseline: f64) -> PlacedShape {
    let mut shape = template.clone();
    shape.fill = None;
    shape.gradient = None;
    shape.outline = None;
    shape.image_part = None;
    shape.shadow = None;
    shape.pattern = None;
    shape.effects = Default::default();
    shape.rotation_rad = 0.0;
    shape.flip_h = false;
    shape.flip_v = false;
    shape.group_transforms.clear();
    shape.local_rect = None;
    shape.text_rotation_deg = 0.0;
    let top = baseline - piece.run.size_pt as f64 * jade_pptx_layout::PX_PER_PT;
    shape.rect = Rect {
        x,
        y: top - piece.line_top.min(0.0),
        width: piece.run.width.max(1.0),
        height: piece.line_height.max(1.0),
    };
    let mut run = piece.run.clone();
    run.baseline_y = baseline - shape.rect.y;
    shape.lines = vec![PlacedLine {
        top: 0.0,
        height: piece.line_height,
        runs: vec![run],
        ..PlacedLine::default()
    }];
    shape
}

/// Paints Morph `progress` of the way from `from` to `to`.
pub fn render_morph_frame<P: Painter>(
    painter: &P,
    from: &SlideLayout,
    to: &SlideLayout,
    options: &RenderOptions,
    draw_image: &mut DrawImage<'_>,
    option: MorphOption,
    progress: f64,
) -> Result<(), P::Error> {
    let p = progress.clamp(0.0, 1.0);
    render_background(painter, to, options)?;
    draw_faded(painter, 1.0 - p, |faded| {
        render_background(faded, from, options)
    })?;
    painter.set_text_baseline("alphabetic");
    let pairs = match_shapes(from, to);
    // Unmatched shapes fade: the old ones out, the new ones in.
    for (index, shape) in from.shapes.iter().enumerate() {
        if !pairs.iter().any(|(i, _)| *i == index) {
            draw_faded(painter, 1.0 - p, |faded| {
                crate::render_shape_clipped(faded, shape, options, draw_image, None)
            })?;
        }
    }
    for (index, shape) in to.shapes.iter().enumerate() {
        if !pairs.iter().any(|(_, j)| *j == index) {
            draw_faded(painter, p, |faded| {
                crate::render_shape_clipped(faded, shape, options, draw_image, None)
            })?;
        }
    }
    for (i, j) in pairs {
        let (a, b) = (&from.shapes[i], &to.shapes[j]);
        let mut body = morph_shape(a, b, p);
        let same_text = shape_text(a) == shape_text(b);
        if same_text && option == MorphOption::ByObject {
            crate::render_shape_clipped(painter, &body, options, draw_image, None)?;
            continue;
        }
        body.lines.clear();
        crate::render_shape_clipped(painter, &body, options, draw_image, None)?;
        if option == MorphOption::ByObject {
            // Different text crossfades inside the moving shape.
            let mut old = morph_shape(a, b, p);
            old.lines = a.lines.clone();
            strip(&mut old);
            let mut new = morph_shape(a, b, p);
            new.lines = b.lines.clone();
            strip(&mut new);
            draw_faded(painter, 1.0 - p, |faded| {
                crate::render_shape_clipped(faded, &old, options, draw_image, None)
            })?;
            draw_faded(painter, p, |faded| {
                crate::render_shape_clipped(faded, &new, options, draw_image, None)
            })?;
            continue;
        }
        // Words or letters travel from where they were to where they are.
        let by_char = option == MorphOption::ByChar;
        let old = pieces(painter, a, by_char, options);
        let new = pieces(painter, b, by_char, options);
        let mut used = vec![false; old.len()];
        for piece in &new {
            let partner = old
                .iter()
                .enumerate()
                .position(|(k, candidate)| !used[k] && candidate.text == piece.text);
            match partner {
                Some(k) => {
                    used[k] = true;
                    let source = &old[k];
                    let x = lerp(source.x, piece.x, p);
                    let baseline = lerp(source.baseline, piece.baseline, p);
                    let shape = piece_shape(b, piece, x, baseline);
                    crate::render_shape_clipped(painter, &shape, options, draw_image, None)?;
                }
                None => {
                    let shape = piece_shape(b, piece, piece.x, piece.baseline);
                    draw_faded(painter, p, |faded| {
                        crate::render_shape_clipped(faded, &shape, options, draw_image, None)
                    })?;
                }
            }
        }
        for (k, piece) in old.iter().enumerate() {
            if !used[k] {
                let shape = piece_shape(a, piece, piece.x, piece.baseline);
                draw_faded(painter, 1.0 - p, |faded| {
                    crate::render_shape_clipped(faded, &shape, options, draw_image, None)
                })?;
            }
        }
    }
    Ok(())
}

fn strip(shape: &mut PlacedShape) {
    shape.fill = None;
    shape.gradient = None;
    shape.outline = None;
    shape.image_part = None;
    shape.shadow = None;
    shape.pattern = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::animation::test_support::{layout, shape};

    #[test]
    fn matches_by_name_and_interpolates() {
        let mut from = layout(Vec::new());
        let mut to = layout(Vec::new());
        from.shapes = vec![
            shape(1, "!!Box", 0.0, "#000000"),
            shape(2, "Gone", 0.0, "#000000"),
        ];
        to.shapes = vec![
            shape(3, "Other", 0.0, "#000000"),
            shape(1, "!!Box", 200.0, "#FFFFFF"),
        ];
        assert_eq!(match_shapes(&from, &to), vec![(0, 1)]);
        let middle = morph_layouts(&from, &to, 0.5);
        assert_eq!(middle.shapes.len(), 1);
        assert_eq!(middle.shapes[0].rect.x, 100.0);
        assert_eq!(middle.shapes[0].fill.as_deref(), Some("#808080"));
        assert_eq!(MorphOption::parse(Some("byWord")), MorphOption::ByWord);
    }
}
