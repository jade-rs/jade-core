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
            theme_color: None,
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
                text_controls: None,
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
            theme_color: None,
        };
        let ops = render(&document(
            None,
            Some(PageBorders {
                top: Some(art.clone()),
                bottom: Some(art.clone()),
                left: Some(art.clone()),
                right: Some(art),
                offset_from: Some("page".into()),
                ..PageBorders::default()
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
                ..PageBorders::default()
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

    /// A `#ff00aa` single frame with every edge `space` points from its
    /// reference.
    fn frame(
        space: u32,
        offset_from: &str,
        display: Option<&str>,
        z_order: Option<&str>,
    ) -> PageBorders {
        let edge = BorderEdge {
            space: Some(space),
            ..edge("single", 8, "#ff00aa")
        };
        PageBorders {
            top: Some(edge.clone()),
            bottom: Some(edge.clone()),
            left: Some(edge.clone()),
            right: Some(edge),
            offset_from: Some(offset_from.into()),
            display: display.map(Into::into),
            z_order: z_order.map(Into::into),
        }
    }

    /// One section of two pages: the second paragraph starts page 2.
    fn two_page_document(page: PageBorders) -> Document {
        let mut document = document(None, Some(page));
        let body = document.body.as_mut().unwrap();
        let mut second = body.elements[0].as_paragraph().unwrap().clone();
        second.runs[0].text = "Second".into();
        second.properties = Some(ParagraphProperties {
            page_break_before: Some(true),
            ..ParagraphProperties::default()
        });
        body.elements.push(BodyElement::from_paragraph(second));
        document
    }

    fn render_with(
        document: &Document,
    ) -> (
        CanvasRenderer<jade_painter::recording::RecordingPainter>,
        Vec<Op>,
    ) {
        let mut renderer = recording_renderer();
        renderer.render(document, &Options::default()).unwrap();
        let ops = renderer.host.painter().inner().ops();
        (renderer, ops)
    }

    /// The ops of each page, split where a page is cleared whole.
    fn split_pages(ops: &[Op]) -> Vec<&[Op]> {
        let starts: Vec<usize> = ops
            .iter()
            .enumerate()
            .filter(|(_, op)| matches!(op, Op::ClearRect(x, y, w, h) if *x == 0.0 && *y == 0.0 && *w > 100.0 && *h > 100.0))
            .map(|(index, _)| index)
            .collect();
        starts
            .iter()
            .enumerate()
            .map(|(n, start)| &ops[*start..starts.get(n + 1).copied().unwrap_or(ops.len())])
            .collect()
    }

    fn frame_strokes(ops: &[Op]) -> usize {
        ops.iter()
            .filter(|op| matches!(op, Op::SetStrokeStyle(color) if color.eq_ignore_ascii_case("#ff00aa")))
            .count()
    }

    /// The frame's segments, as (x0, y0, x1, y1).
    fn frame_segments(ops: &[Op]) -> Vec<(f64, f64, f64, f64)> {
        let mut segments = Vec::new();
        let mut in_frame = false;
        let mut from = None;
        for op in ops {
            match op {
                Op::SetStrokeStyle(color) => in_frame = color.eq_ignore_ascii_case("#ff00aa"),
                Op::MoveTo(x, y) if in_frame => from = Some((*x, *y)),
                Op::LineTo(x, y) if in_frame => {
                    if let Some((x0, y0)) = from.replace((*x, *y)) {
                        segments.push((x0, y0, *x, *y));
                    }
                }
                _ => {}
            }
        }
        segments
    }

    #[test]
    fn a_first_page_frame_is_drawn_on_the_sections_first_page_only() {
        let (_, ops) = render_with(&two_page_document(frame(
            24,
            "page",
            Some("firstPage"),
            None,
        )));
        let pages = split_pages(&ops);
        assert_eq!(pages.len(), 2, "two pages");
        assert!(frame_strokes(pages[0]) > 0, "the cover page is framed");
        assert_eq!(frame_strokes(pages[1]), 0, "page 2 is not");
    }

    #[test]
    fn a_not_first_page_frame_skips_the_sections_first_page() {
        let (_, ops) = render_with(&two_page_document(frame(
            24,
            "page",
            Some("notFirstPage"),
            None,
        )));
        let pages = split_pages(&ops);
        assert_eq!(pages.len(), 2, "two pages");
        assert_eq!(frame_strokes(pages[0]), 0, "the first page is not framed");
        assert!(frame_strokes(pages[1]) > 0, "page 2 is");
        let (_, every) = render_with(&two_page_document(frame(24, "page", None, None)));
        let every = split_pages(&every);
        assert!(
            frame_strokes(every[0]) > 0 && frame_strokes(every[1]) > 0,
            "allPages is the default"
        );
    }

    #[test]
    fn a_page_anchored_frame_sits_its_space_from_the_page_edge() {
        let (renderer, ops) = render_with(&document(None, Some(frame(10, "page", None, None))));
        // The 1pt rule's near side is 10pt in: its line is half a rule on.
        let ten = points_to_px(10.0) + points_to_px(1.0) / 2.0;
        let segments = frame_segments(&ops);
        let top = segments
            .iter()
            .filter(|(_, y0, _, y1)| (y0 - y1).abs() < f64::EPSILON)
            .map(|(_, y, _, _)| *y)
            .fold(f64::MAX, f64::min);
        assert!(
            (top - ten).abs() <= 0.75,
            "top edge at 10pt ({ten}px), not {top}px"
        );
        let right = segments
            .iter()
            .filter(|(x0, _, x1, _)| (x0 - x1).abs() < f64::EPSILON)
            .map(|(x, _, _, _)| *x)
            .fold(f64::MIN, f64::max);
        assert!(
            (right - (renderer.page_width - ten)).abs() <= 0.75,
            "right edge 10pt in from the page's right ({right}px)"
        );
    }

    #[test]
    fn a_text_anchored_frame_sits_its_space_outside_the_margins() {
        let (renderer, ops) = render_with(&document(None, Some(frame(4, "text", None, None))));
        let expected = renderer.margin_left - points_to_px(4.0) - points_to_px(1.0) / 2.0;
        let left = frame_segments(&ops)
            .iter()
            .filter(|(x0, _, x1, _)| (x0 - x1).abs() < f64::EPSILON)
            .map(|(x, _, _, _)| *x)
            .fold(f64::MAX, f64::min);
        assert!(
            (left - expected).abs() <= 0.75,
            "left edge 4pt outside the {}px margin: expected {expected}px, got {left}px",
            renderer.margin_left
        );
    }

    #[test]
    fn a_back_frame_is_painted_before_the_text_and_a_front_one_after() {
        let first_text = |ops: &[Op]| {
            ops.iter()
                .position(|op| matches!(op, Op::FillText(..) | Op::FillShapedText(..)))
                .expect("body text")
        };
        let first_frame = |ops: &[Op]| {
            ops.iter()
                .position(|op| matches!(op, Op::SetStrokeStyle(color) if color.eq_ignore_ascii_case("#ff00aa")))
                .expect("the frame")
        };
        let (_, back) = render_with(&document(None, Some(frame(24, "page", None, Some("back")))));
        assert!(
            first_frame(&back) < first_text(&back),
            "zOrder=back goes under the text"
        );
        let (_, front) = render_with(&document(None, Some(frame(24, "page", None, None))));
        assert!(
            first_frame(&front) > first_text(&front),
            "front (the default) goes over it"
        );
        assert_eq!(
            frame_strokes(&front),
            frame_strokes(&back),
            "each is drawn once"
        );
    }

    /// Renders a package whose body is `body_xml`.
    fn render_package(
        body_xml: &str,
    ) -> (
        CanvasRenderer<jade_painter::recording::RecordingPainter>,
        Vec<Op>,
    ) {
        let bytes = super::super::section_tests::build_docx_with_body(body_xml);
        let word = futures::executor::block_on(crate::word_document::WordDocument::load(
            &bytes,
            &Options::default(),
        ))
        .expect("load package");
        render_with(word.document.as_ref().expect("document"))
    }

    fn paragraph_xml(text: &str) -> String {
        format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>")
    }

    /// A `#ff00aa` page frame 4pt outside the text, with the given
    /// container attributes.
    fn frame_xml(attributes: &str) -> String {
        let edge = |name: &str| {
            format!(r#"<w:{name} w:val="single" w:sz="8" w:space="4" w:color="FF00AA"/>"#)
        };
        format!(
            "<w:pgBorders {attributes}>{}{}{}{}</w:pgBorders>",
            edge("top"),
            edge("left"),
            edge("bottom"),
            edge("right")
        )
    }

    const SECTION_MARGINS_A: &str = r#"<w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>"#;

    /// One page: section A carries the frame, then a continuous break starts
    /// section B, which has none. (A change of margins would start a new
    /// page, so B shares A's.)
    fn continuous_break_after_framed_section(attributes: &str) -> String {
        format!(
            r#"{}<w:p><w:pPr><w:sectPr>{}{SECTION_MARGINS_A}</w:sectPr></w:pPr><w:r><w:t>A end</w:t></w:r></w:p>{}<w:sectPr><w:type w:val="continuous"/>{SECTION_MARGINS_A}</w:sectPr>"#,
            paragraph_xml("A"),
            frame_xml(attributes),
            paragraph_xml("B"),
        )
    }

    /// The frame's leftmost vertical line, in px.
    fn frame_left_line(ops: &[Op]) -> f64 {
        frame_segments(ops)
            .iter()
            .filter(|(x0, _, x1, _)| (x0 - x1).abs() < f64::EPSILON)
            .map(|(x, _, _, _)| *x)
            .fold(f64::MAX, f64::min)
    }

    #[test]
    fn a_page_shared_by_two_sections_draws_the_top_sections_frame_in_either_layer() {
        // Section A's 1in margin, 4pt space and half the 1pt rule.
        let expected_left = points_to_px(72.0 - 4.0 - 0.5);
        for z_order in ["", r#"w:zOrder="back""#] {
            let (_, ops) = render_package(&continuous_break_after_framed_section(z_order));
            let pages = split_pages(&ops);
            assert_eq!(pages.len(), 1, "{z_order:?}: both sections share one page");
            assert!(
                frame_strokes(pages[0]) > 0,
                "{z_order:?}: the page shows section A's frame"
            );
            let left = frame_left_line(pages[0]);
            assert!(
                (left - expected_left).abs() <= 0.75,
                "{z_order:?}: placed 4pt outside A's margin ({expected_left}px), not {left}px"
            );
        }
    }

    #[test]
    fn a_first_page_frame_of_a_later_section_shows_on_that_sections_first_page() {
        for z_order in ["", r#"w:zOrder="back""#] {
            // Page 1 is section A; section B starts on page 2 and runs onto
            // page 3.
            let body = format!(
                r#"{}<w:p><w:pPr><w:sectPr>{SECTION_MARGINS_A}</w:sectPr></w:pPr><w:r><w:t>A end</w:t></w:r></w:p>{}<w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:r><w:t>B 2</w:t></w:r></w:p><w:sectPr>{}{SECTION_MARGINS_A}</w:sectPr>"#,
                paragraph_xml("A"),
                paragraph_xml("B 1"),
                frame_xml(&format!(r#"w:display="firstPage" {z_order}"#)),
            );
            let (_, ops) = render_package(&body);
            let pages = split_pages(&ops);
            assert_eq!(pages.len(), 3, "{z_order:?}: three pages");
            let framed: Vec<bool> = pages.iter().map(|page| frame_strokes(page) > 0).collect();
            assert_eq!(
                framed,
                [false, true, false],
                "{z_order:?}: only section B's first page is framed"
            );
        }
    }
}
