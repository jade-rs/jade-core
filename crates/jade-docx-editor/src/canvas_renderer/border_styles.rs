//! How each `ST_Border` line style (ECMA-376 Part 1 §17.18.2) is drawn.
//!
//! A border occupies a band across its thickness. Every style is described
//! the same way for the three places borders are drawn — paragraph borders
//! (filled rectangles, cached as draw commands), table cell borders and page
//! borders (strokes along the edge): a list of [`Stripe`]s across the band,
//! measured from its outer edge, and a [`Pattern`] along it.
//!
//! The proportions follow Word's line samples in the Borders and Shading
//! dialog: thin–thick families put the thin rule outside the bordered box, a
//! small gap equals the thin rule, a medium gap half the thick rule and a
//! large gap the thick rule; the 3-D styles shade from light to dark across
//! the band, `outset` lights the top and left edges and `inset` the bottom and
//! right ones.

use super::table::BorderKind;

/// Which edge of the bordered box a border is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BorderSide {
    Top,
    Bottom,
    Left,
    Right,
}

/// The colour of one stripe, relative to the border's colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tone {
    /// The border colour.
    Line,
    /// The border colour lightened towards white.
    Light,
    /// The border colour darkened towards black.
    Dark,
    /// `outset`: light on the top and left edges, the colour elsewhere.
    Raised,
    /// `inset`: the colour on the top and left edges, light elsewhere.
    Sunken,
}

/// One rule across a border's band, as fractions of the band's thickness
/// from its outer edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Stripe {
    pub start: f64,
    pub width: f64,
    pub tone: Tone,
}

/// How a border runs along its edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Pattern {
    Solid,
    /// Alternating on/off lengths in multiples of the band's thickness.
    Dash(&'static [f64]),
    /// `wave` (one line) and `doubleWave` (two).
    Wave {
        lines: u8,
    },
    /// `dashDotStroked`: dashes separated by short diagonal strokes.
    DashDotStroked,
}

/// Lays out `units` (rule, gap, rule, …; rules at even positions) across the
/// band, normalized to its thickness.
fn rules(units: &[(f64, Tone)]) -> Vec<Stripe> {
    let total: f64 = units.iter().map(|(width, _)| width).sum();
    let mut start = 0.0;
    let mut stripes = Vec::new();
    for (index, (width, tone)) in units.iter().enumerate() {
        if index % 2 == 0 {
            stripes.push(Stripe {
                start: start / total,
                width: width / total,
                tone: *tone,
            });
        }
        start += width;
    }
    stripes
}

const GAP: Tone = Tone::Line;

/// The rules of `kind` across its band.
pub(super) fn stripes(kind: BorderKind) -> Vec<Stripe> {
    use BorderKind::*;
    use Tone::*;
    match kind {
        Double => rules(&[(1.0, Line), (1.0, GAP), (1.0, Line)]),
        Triple => rules(&[
            (1.0, Line),
            (1.0, GAP),
            (1.0, Line),
            (1.0, GAP),
            (1.0, Line),
        ]),
        ThinThickSmallGap => rules(&[(1.0, Line), (1.0, GAP), (4.0, Line)]),
        ThickThinSmallGap => rules(&[(4.0, Line), (1.0, GAP), (1.0, Line)]),
        ThinThickThinSmallGap => rules(&[
            (1.0, Line),
            (1.0, GAP),
            (4.0, Line),
            (1.0, GAP),
            (1.0, Line),
        ]),
        ThinThickMediumGap => rules(&[(1.0, Line), (1.0, GAP), (2.0, Line)]),
        ThickThinMediumGap => rules(&[(2.0, Line), (1.0, GAP), (1.0, Line)]),
        ThinThickThinMediumGap => rules(&[
            (1.0, Line),
            (1.0, GAP),
            (2.0, Line),
            (1.0, GAP),
            (1.0, Line),
        ]),
        ThinThickLargeGap => rules(&[(1.0, Line), (3.0, GAP), (3.0, Line)]),
        ThickThinLargeGap => rules(&[(3.0, Line), (3.0, GAP), (1.0, Line)]),
        ThinThickThinLargeGap => rules(&[
            (1.0, Line),
            (3.0, GAP),
            (3.0, Line),
            (3.0, GAP),
            (1.0, Line),
        ]),
        ThreeDEmboss => vec![
            Stripe {
                start: 0.0,
                width: 0.5,
                tone: Light,
            },
            Stripe {
                start: 0.5,
                width: 0.5,
                tone: Dark,
            },
        ],
        ThreeDEngrave => vec![
            Stripe {
                start: 0.0,
                width: 0.5,
                tone: Dark,
            },
            Stripe {
                start: 0.5,
                width: 0.5,
                tone: Light,
            },
        ],
        Outset => vec![Stripe {
            start: 0.0,
            width: 1.0,
            tone: Raised,
        }],
        Inset => vec![Stripe {
            start: 0.0,
            width: 1.0,
            tone: Sunken,
        }],
        _ => vec![Stripe {
            start: 0.0,
            width: 1.0,
            tone: Line,
        }],
    }
}

/// How many authored line widths (`w:sz`) the band of `kind` spans when a
/// border is sized by its rule width, as table and page borders are: Word
/// draws each thin rule and a small gap at the authored width.
pub(super) fn band_units(kind: BorderKind) -> f64 {
    use BorderKind::*;
    match kind {
        Triple => 5.0,
        ThinThickSmallGap | ThickThinSmallGap => 1.5,
        ThinThickThinSmallGap => 2.0,
        ThinThickMediumGap | ThickThinMediumGap => 2.0,
        ThinThickThinMediumGap => 3.0,
        ThinThickLargeGap | ThickThinLargeGap => 2.5,
        ThinThickThinLargeGap => 3.5,
        ThreeDEmboss | ThreeDEngrave => 2.0,
        Wave => 3.0,
        DoubleWave => 4.5,
        DashDotStroked => 2.0,
        _ => 1.0,
    }
}

/// The pattern `kind` runs along its edge.
pub(super) fn pattern(kind: BorderKind) -> Pattern {
    use BorderKind::*;
    match kind {
        Dashed => Pattern::Dash(&[3.0, 2.0]),
        DashSmallGap => Pattern::Dash(&[3.0, 1.0]),
        Dotted => Pattern::Dash(&[1.0, 1.0]),
        DotDash => Pattern::Dash(&[4.0, 2.0, 1.0, 2.0]),
        DotDotDash => Pattern::Dash(&[4.0, 2.0, 1.0, 2.0, 1.0, 2.0]),
        Wave => Pattern::Wave { lines: 1 },
        DoubleWave => Pattern::Wave { lines: 2 },
        DashDotStroked => Pattern::DashDotStroked,
        _ => Pattern::Solid,
    }
}

/// Whether the band's outer edge for `side` faces the top or left of the page.
fn faces_top_left(side: BorderSide) -> bool {
    matches!(side, BorderSide::Top | BorderSide::Left)
}

/// Parses `#rrggbb` (or `rrggbb`); anything else is treated as black.
fn rgb(color: &str) -> (f64, f64, f64) {
    let hex = color.trim().trim_start_matches('#');
    if hex.len() == 6 {
        if let Ok(value) = u32::from_str_radix(hex, 16) {
            return (
                f64::from((value >> 16) & 0xff),
                f64::from((value >> 8) & 0xff),
                f64::from(value & 0xff),
            );
        }
    }
    (0.0, 0.0, 0.0)
}

fn mix(color: &str, toward: f64, amount: f64) -> String {
    let (r, g, b) = rgb(color);
    let channel = |value: f64| {
        (value + (toward - value) * amount)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    format!("#{:02x}{:02x}{:02x}", channel(r), channel(g), channel(b))
}

/// The colour a stripe of `tone` is painted in.
pub(super) fn tone_color(color: &str, tone: Tone, side: BorderSide) -> String {
    let light = || mix(color, 255.0, 0.6);
    match tone {
        Tone::Line => color.to_string(),
        Tone::Light => light(),
        Tone::Dark => mix(color, 0.0, 0.35),
        Tone::Raised if faces_top_left(side) => light(),
        Tone::Sunken if !faces_top_left(side) => light(),
        Tone::Raised | Tone::Sunken => color.to_string(),
    }
}

/// A wavy line along `length` with `amplitude` either side of its centre,
/// as points in the edge's own frame (x along the edge, y across it).
pub(super) fn wave_points(length: f64, amplitude: f64, period: f64) -> Vec<(f64, f64)> {
    if length <= 0.0 || period <= 0.0 {
        return Vec::new();
    }
    let steps_per_period = 12.0;
    let step = period / steps_per_period;
    let count = (length / step).ceil() as usize;
    (0..=count)
        .map(|index| {
            let x = (index as f64 * step).min(length);
            let phase = x / period * std::f64::consts::TAU;
            (x, amplitude * phase.sin())
        })
        .collect()
}

/// The polylines of `pattern` for an edge `length` long whose band is
/// `thickness` across, in the edge frame: x along the edge, y across the band
/// from its outer edge (0) to its inner edge (`thickness`). Returns the line
/// width with them. Empty for solid and dashed patterns, which are drawn as
/// stripes.
pub(super) fn pattern_polylines(
    pattern: Pattern,
    length: f64,
    thickness: f64,
) -> (f64, Vec<Vec<(f64, f64)>>) {
    match pattern {
        Pattern::Wave { lines } => {
            let lines = f64::from(lines.max(1));
            let lane = thickness / lines;
            let line_width = (lane / 3.0).max(0.5);
            let amplitude = ((lane - line_width) / 2.0).max(0.25);
            let period = (lane * 2.0).max(3.0);
            let polylines = (0..lines as usize)
                .map(|lane_index| {
                    let centre = lane * (lane_index as f64 + 0.5);
                    wave_points(length, amplitude, period)
                        .into_iter()
                        .map(|(x, y)| (x, centre + y))
                        .collect()
                })
                .collect();
            (line_width, polylines)
        }
        Pattern::DashDotStroked => {
            // A dash across the middle third, then a stroke leaning across
            // the whole band: — / — / …
            let line_width = (thickness / 3.0).max(0.5);
            let centre = thickness / 2.0;
            let dash = thickness * 3.0;
            let gap = thickness;
            let mut polylines = Vec::new();
            let mut x = 0.0;
            while x < length {
                let dash_end = (x + dash).min(length);
                polylines.push(vec![(x, centre), (dash_end, centre)]);
                let stroke_start = dash_end + gap;
                let stroke_end = stroke_start + thickness * 0.6;
                if stroke_end <= length {
                    polylines.push(vec![(stroke_start, thickness), (stroke_end, 0.0)]);
                }
                x = stroke_end + gap;
            }
            (line_width, polylines)
        }
        Pattern::Solid | Pattern::Dash(_) => (0.0, Vec::new()),
    }
}

/// `(start, length)` runs along an edge `length` long for `pattern`, the
/// dash lengths scaled by `thickness` (floored at 1px so hairlines still
/// dash). Solid and drawn-as-lines patterns are one full-length run.
pub(super) fn pattern_runs(pattern: Pattern, length: f64, thickness: f64) -> Vec<(f64, f64)> {
    if length <= 0.0 {
        return Vec::new();
    }
    let Pattern::Dash(lengths) = pattern else {
        return vec![(0.0, length)];
    };
    let unit = thickness.max(1.0);
    let mut runs = Vec::new();
    let mut position = 0.0;
    let mut index = 0usize;
    while position < length {
        let on = lengths[index % lengths.len()] * unit;
        let off = lengths[(index + 1) % lengths.len()] * unit;
        let run = on.min(length - position);
        if run > 0.0 {
            runs.push((position, run));
        }
        position += on + off;
        index += 2;
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covered(stripes: &[Stripe]) -> f64 {
        stripes.iter().map(|stripe| stripe.width).sum()
    }

    #[test]
    fn thin_thick_puts_the_thin_rule_outside_and_gaps_grow_by_family() {
        let small = stripes(BorderKind::ThinThickSmallGap);
        let medium = stripes(BorderKind::ThinThickMediumGap);
        let large = stripes(BorderKind::ThinThickLargeGap);
        for family in [&small, &medium, &large] {
            assert_eq!(family.len(), 2);
            assert!(
                family[0].width < family[1].width,
                "thin outside, thick inside"
            );
            assert_eq!(family[1].start + family[1].width, 1.0);
        }
        let gap = |stripes: &[Stripe]| stripes[1].start - stripes[0].width;
        assert!(gap(&small) < gap(&medium) && gap(&medium) < gap(&large));
        let reversed = stripes(BorderKind::ThickThinSmallGap);
        assert!(reversed[0].width > reversed[1].width);
        assert_eq!(stripes(BorderKind::ThinThickThinMediumGap).len(), 3);
        assert!(covered(&stripes(BorderKind::Triple)) < 1.0);
    }

    #[test]
    fn three_d_and_inset_styles_shade_by_side() {
        let emboss = stripes(BorderKind::ThreeDEmboss);
        assert_eq!((emboss[0].tone, emboss[1].tone), (Tone::Light, Tone::Dark));
        let engrave = stripes(BorderKind::ThreeDEngrave);
        assert_eq!(
            (engrave[0].tone, engrave[1].tone),
            (Tone::Dark, Tone::Light)
        );
        let black = "#000000";
        assert_eq!(tone_color(black, Tone::Raised, BorderSide::Top), "#999999");
        assert_eq!(tone_color(black, Tone::Raised, BorderSide::Bottom), black);
        assert_eq!(tone_color(black, Tone::Sunken, BorderSide::Left), black);
        assert_eq!(
            tone_color(black, Tone::Sunken, BorderSide::Right),
            "#999999"
        );
        assert_eq!(
            tone_color("#ff0000", Tone::Dark, BorderSide::Top),
            "#a60000"
        );
    }

    #[test]
    fn waves_stay_inside_their_band_and_double_waves_draw_two_lines() {
        let (width, lines) = pattern_polylines(Pattern::Wave { lines: 1 }, 40.0, 6.0);
        assert_eq!(lines.len(), 1);
        assert!(lines[0]
            .iter()
            .all(|&(_, y)| y - width / 2.0 >= -1e-9 && y + width / 2.0 <= 6.0 + 1e-9));
        assert!(lines[0].iter().any(|&(_, y)| y > 3.5) && lines[0].iter().any(|&(_, y)| y < 2.5));
        let (_, double) = pattern_polylines(Pattern::Wave { lines: 2 }, 40.0, 6.0);
        assert_eq!(double.len(), 2);
        assert!(double[0].iter().all(|&(_, y)| y <= 3.0 + 1e-9));
        assert!(double[1].iter().all(|&(_, y)| y >= 3.0 - 1e-9));
    }

    #[test]
    fn dash_dot_stroked_alternates_dashes_and_diagonal_strokes() {
        let (_, lines) = pattern_polylines(Pattern::DashDotStroked, 60.0, 3.0);
        assert!(lines.len() >= 4);
        assert_eq!(lines[0][0].1, lines[0][1].1, "a level dash");
        assert_ne!(lines[1][0].1, lines[1][1].1, "a leaning stroke");
    }

    #[test]
    fn every_style_has_a_distinct_drawing() {
        use BorderKind::*;
        let kinds = [
            Single,
            Double,
            Triple,
            Dashed,
            DashSmallGap,
            Dotted,
            DotDash,
            DotDotDash,
            Wave,
            DoubleWave,
            DashDotStroked,
            ThinThickSmallGap,
            ThickThinSmallGap,
            ThinThickThinSmallGap,
            ThinThickMediumGap,
            ThickThinMediumGap,
            ThinThickThinMediumGap,
            ThinThickLargeGap,
            ThickThinLargeGap,
            ThinThickThinLargeGap,
            ThreeDEmboss,
            ThreeDEngrave,
            Outset,
            Inset,
        ];
        for (index, a) in kinds.iter().enumerate() {
            for b in &kinds[index + 1..] {
                assert!(
                    stripes(*a) != stripes(*b) || pattern(*a) != pattern(*b),
                    "{a:?} and {b:?} draw alike"
                );
            }
        }
    }
}

#[cfg(test)]
mod rendering {
    use super::super::recording_host::recording_renderer;
    use super::super::*;
    use jade_docx_reader::document::{BorderEdge, PageBorders, ParagraphBorders};
    use jade_painter::recording::Op;

    fn edge(val: &str, size: u32, color: &str) -> BorderEdge {
        BorderEdge {
            val: Some(val.into()),
            size_eighths: Some(size),
            space: Some(1),
            color: Some(color.into()),
        }
    }

    fn document(borders: Option<ParagraphBorders>, page: Option<PageBorders>) -> Document {
        let paragraph = Paragraph {
            runs: vec![Run {
                text: "Bordered".into(),
                properties: None,
                image: None,
                text_box: None,
                math: None,
                field: None,
                hyperlink: None,
                node_id: None,
                text_node_id: None,
                revision_node_id: None,
                comment_reference_ids: Vec::new(),
                footnote_reference_ids: Vec::new(),
                endnote_reference_ids: Vec::new(),
                note_separator: None,
                has_footnote_ref: false,
                has_endnote_ref: false,
                revision: None,
            }],
            properties: Some(ParagraphProperties {
                borders,
                ..ParagraphProperties::default()
            }),
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(paragraph)],
            }),
            default_section: Some(SectionProperties {
                page_borders: page,
                ..SectionProperties::default()
            }),
            ..Document::default()
        }
    }

    fn render(document: &Document) -> Vec<Op> {
        let mut renderer = recording_renderer();
        renderer.render(document, &Options::default()).unwrap();
        renderer.host.painter().inner().ops()
    }

    /// Fill rectangles drawn in `color`, as (x, y, width, height).
    fn fills_in(ops: &[Op], color: &str) -> Vec<(f64, f64, f64, f64)> {
        let mut current = String::new();
        let mut rects = Vec::new();
        for op in ops {
            match op {
                Op::SetFillStyle(value) => current = value.clone(),
                Op::FillRect(x, y, w, h) if current.eq_ignore_ascii_case(color) => {
                    rects.push((*x, *y, *w, *h))
                }
                _ => {}
            }
        }
        rects
    }

    #[test]
    fn a_thin_thick_paragraph_border_draws_a_thin_rule_above_a_thick_one() {
        let ops = render(&document(
            Some(ParagraphBorders {
                top: Some(edge("thinThickSmallGap", 48, "#112233")),
                ..ParagraphBorders::default()
            }),
            None,
        ));
        let rules = fills_in(&ops, "#112233");
        assert_eq!(rules.len(), 2, "{rules:?}");
        let (thin, thick) = (rules[0], rules[1]);
        assert!(thin.1 < thick.1, "the thin rule is outside (above)");
        assert!(thin.3 * 3.0 < thick.3, "{thin:?} {thick:?}");
    }

    #[test]
    fn a_wave_paragraph_border_is_a_wavy_stroke() {
        let ops = render(&document(
            Some(ParagraphBorders {
                bottom: Some(edge("wave", 12, "#0000ff")),
                ..ParagraphBorders::default()
            }),
            None,
        ));
        let start = ops
            .iter()
            .position(|op| matches!(op, Op::SetStrokeStyle(color) if color == "#0000ff"))
            .expect("a blue stroke");
        let ys: Vec<f64> = ops[start..]
            .iter()
            .take_while(|op| !matches!(op, Op::Stroke))
            .filter_map(|op| match op {
                Op::LineTo(_, y) | Op::MoveTo(_, y) => Some(*y),
                _ => None,
            })
            .collect();
        assert!(ys.len() > 10);
        let (low, high) = ys
            .iter()
            .fold((f64::MAX, f64::MIN), |(lo, hi), y| (lo.min(*y), hi.max(*y)));
        assert!(high - low > 1.0, "the line rises and falls");
    }

    #[test]
    fn an_art_page_border_draws_its_motif_in_its_colours() {
        let art = BorderEdge {
            val: Some("hearts".into()),
            size_eighths: Some(12),
            space: Some(24),
            color: None,
        };
        let ops = render(&document(
            None,
            Some(PageBorders {
                top: Some(art.clone()),
                bottom: Some(art.clone()),
                left: Some(art.clone()),
                right: Some(art),
                offset_from: Some("page".into()),
            }),
        ));
        let hearts = ops
            .iter()
            .filter(|op| matches!(op, Op::BezierCurveTo(..)))
            .count();
        assert!(hearts > 40, "hearts all round the page ({hearts} curves)");
        assert!(ops.contains(&Op::SetFillStyle("#d0021b".into())));
    }

    #[test]
    fn an_outset_page_border_lights_its_top_and_darkens_its_bottom() {
        let outset = edge("outset", 24, "#000000");
        let ops = render(&document(
            None,
            Some(PageBorders {
                top: Some(outset.clone()),
                bottom: Some(outset),
                left: None,
                right: None,
                offset_from: None,
            }),
        ));
        let strokes: Vec<String> = ops
            .iter()
            .filter_map(|op| match op {
                Op::SetStrokeStyle(color) => Some(color.clone()),
                _ => None,
            })
            .collect();
        let light = strokes.iter().position(|color| color == "#999999");
        let dark = strokes.iter().rposition(|color| color == "#000000");
        assert!(light.is_some() && dark.is_some(), "{strokes:?}");
        assert!(light < dark, "top (light) is drawn before bottom (dark)");
    }
}
