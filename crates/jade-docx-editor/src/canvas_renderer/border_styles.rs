//! How each `ST_Border` line style (ECMA-376 Part 1 §17.18.2) is drawn: the
//! renderer's [`BorderKind`]s over the shared description in
//! [`jade_docx_layout::border_styles`], which the PDF renderer draws from too.

use super::table::BorderKind;
pub(super) use jade_docx_layout::border_styles::{
    pattern_polylines, pattern_runs, tone_color, BorderLineStyle, BorderSide, Pattern, Stripe, Tone,
};

impl BorderKind {
    /// The line style `self` draws with: art and hidden borders, which are
    /// drawn (or not) elsewhere, count as single.
    pub(super) fn line_style(self) -> BorderLineStyle {
        use BorderLineStyle as S;
        match self {
            BorderKind::None | BorderKind::Single | BorderKind::Art(_) => S::Single,
            BorderKind::Thick => S::Thick,
            BorderKind::Double => S::Double,
            BorderKind::Triple => S::Triple,
            BorderKind::Dashed => S::Dashed,
            BorderKind::DashSmallGap => S::DashSmallGap,
            BorderKind::Dotted => S::Dotted,
            BorderKind::DotDash => S::DotDash,
            BorderKind::DotDotDash => S::DotDotDash,
            BorderKind::Wave => S::Wave,
            BorderKind::DoubleWave => S::DoubleWave,
            BorderKind::DashDotStroked => S::DashDotStroked,
            BorderKind::ThinThickSmallGap => S::ThinThickSmallGap,
            BorderKind::ThickThinSmallGap => S::ThickThinSmallGap,
            BorderKind::ThinThickThinSmallGap => S::ThinThickThinSmallGap,
            BorderKind::ThinThickMediumGap => S::ThinThickMediumGap,
            BorderKind::ThickThinMediumGap => S::ThickThinMediumGap,
            BorderKind::ThinThickThinMediumGap => S::ThinThickThinMediumGap,
            BorderKind::ThinThickLargeGap => S::ThinThickLargeGap,
            BorderKind::ThickThinLargeGap => S::ThickThinLargeGap,
            BorderKind::ThinThickThinLargeGap => S::ThinThickThinLargeGap,
            BorderKind::ThreeDEmboss => S::ThreeDEmboss,
            BorderKind::ThreeDEngrave => S::ThreeDEngrave,
            BorderKind::Outset => S::Outset,
            BorderKind::Inset => S::Inset,
        }
    }
}

/// The rules of `kind` across its band.
pub(super) fn stripes(kind: BorderKind) -> Vec<Stripe> {
    jade_docx_layout::border_styles::stripes(kind.line_style())
}

/// How many authored line widths the band of `kind` spans
/// ([`jade_docx_layout::border_styles::band_units`]).
pub(super) fn band_units(kind: BorderKind) -> f64 {
    jade_docx_layout::border_styles::band_units(kind.line_style())
}

/// The pattern `kind` runs along its edge.
pub(super) fn pattern(kind: BorderKind) -> Pattern {
    jade_docx_layout::border_styles::pattern(kind.line_style())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_draws_as_the_style_its_value_names() {
        for (value, kind) in [
            ("double", BorderKind::Double),
            ("thinThickThinLargeGap", BorderKind::ThinThickThinLargeGap),
            ("dashDotStroked", BorderKind::DashDotStroked),
            ("inset", BorderKind::Inset),
        ] {
            assert_eq!(BorderLineStyle::parse(value), Some(kind.line_style()));
            assert_eq!(
                super::super::table::parse_border_kind(Some(&value.to_string())),
                kind
            );
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
