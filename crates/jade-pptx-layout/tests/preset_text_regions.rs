use jade_pptx_layout::{layout_slide, HeuristicMeasure, PlacedGeometry, PlacedShape, Rect};
use jade_pptx_reader::{parse_slide_xml, SlideSize, ThemeColors};
use std::cell::Cell;

fn shape_xml(
    preset: &str,
    w: u32,
    h: u32,
    body: &str,
    paragraphs: &str,
    adjust: &str,
    transform: &str,
) -> String {
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="shape"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm {transform}><a:off x="952500" y="952500"/><a:ext cx="{}" cy="{}"/></a:xfrm><a:prstGeom prst="{preset}"><a:avLst>{adjust}</a:avLst></a:prstGeom></p:spPr><p:txBody><a:bodyPr {body}/><a:lstStyle/>{paragraphs}</p:txBody></p:sp>"#,
        w * 9525,
        h * 9525
    )
}
fn place(xml: &str) -> PlacedShape {
    place_with_measure(xml, &HeuristicMeasure)
}
fn place_with_measure(xml: &str, measure: &dyn jade_pptx_layout::TextMeasure) -> PlacedShape {
    let xml = format!(
        r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree>{xml}</p:spTree></p:cSld></p:sld>"#
    );
    layout_slide(
        SlideSize::default(),
        &parse_slide_xml(&xml).unwrap(),
        &ThemeColors::default(),
        measure,
    )
    .shapes
    .remove(0)
}
fn paragraph(text: &str) -> String {
    format!("<a:p><a:r><a:rPr sz=\"1800\"/><a:t>{text}</a:t></a:r></a:p>")
}
const ZERO: &str = "lIns=\"0\" rIns=\"0\" tIns=\"0\" bIns=\"0\"";
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
fn region_close(actual: Rect, expected: (f64, f64, f64, f64)) {
    for (a, b) in [actual.x, actual.y, actual.width, actual.height]
        .into_iter()
        .zip([expected.0, expected.1, expected.2, expected.3])
    {
        close(a, b);
    }
}

#[test]
fn native_text_rectangles_follow_the_preset_guides_before_body_insets() {
    let inset = (1.0 - std::f64::consts::FRAC_1_SQRT_2) / 2.0;
    for (preset, expected) in [
        (
            "ellipse",
            (
                320.0 * inset,
                160.0 * inset,
                320.0 * (1.0 - 2.0 * inset),
                160.0 * (1.0 - 2.0 * inset),
            ),
        ),
        (
            "roundRect",
            (7.810556208, 7.810556208, 304.378887584, 144.378887584),
        ),
        ("triangle", (80.0, 80.0, 160.0, 80.0)),
        ("diamond", (80.0, 40.0, 160.0, 80.0)),
        ("hexagon", (40.0, 20.0, 240.0, 120.0)),
    ] {
        let s = place(&shape_xml(
            preset,
            320,
            160,
            ZERO,
            &paragraph("日本語"),
            "",
            "",
        ));
        region_close(s.text_region.unwrap(), expected);
        close(s.lines[0].top, expected.1);
        close(s.lines[0].text_left, expected.0);
        close(s.lines[0].runs[0].x, expected.0);
        assert_eq!(s.rect.width, 320.0);
        assert_eq!(s.rect.height, 160.0);
    }
    let s = place(&shape_xml(
        "rect",
        320,
        160,
        ZERO,
        &paragraph("日本語"),
        "",
        "",
    ));
    assert!(s.text_region.is_none());
    close(s.lines[0].top, 0.0);
    close(s.lines[0].runs[0].x, 0.0);
}

#[test]
fn body_insets_and_vertical_anchor_apply_inside_the_geometry_rectangle() {
    let body = "lIns=\"19050\" rIns=\"38100\" tIns=\"28575\" bIns=\"47625\" anchor=\"b\"";
    let s = place(&shape_xml(
        "triangle",
        320,
        160,
        body,
        &paragraph("text"),
        "",
        "",
    ));
    close(s.lines[0].runs[0].x, 82.0);
    close(s.lines[0].top + s.lines[0].height, 155.0);
    let body = "lIns=\"0\" rIns=\"0\" tIns=\"0\" bIns=\"0\" anchor=\"ctr\"";
    let s = place(&shape_xml(
        "triangle",
        320,
        160,
        body,
        &paragraph("text"),
        "",
        "",
    ));
    close(s.lines[0].top + s.lines[0].height / 2.0, 120.0);
    let empty = place(&shape_xml("diamond", 320, 160, ZERO, "<a:p/>", "", ""));
    close(empty.lines[0].text_left, 80.0);
    close(empty.lines[0].top, 40.0);
    let g = empty
        .selection_geometry(0, 0, 0, false, &HeuristicMeasure)
        .unwrap();
    close(g.caret[0].x, 180.0);
    close(g.caret[0].y, 140.0);
}

#[test]
fn wrap_and_overflow_report_the_inner_area_rather_than_the_external_frame() {
    let p = paragraph("ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUVWXYZ");
    let rectangular = place(&shape_xml("rect", 320, 160, ZERO, &p, "", ""));
    let diamond = place(&shape_xml("diamond", 320, 160, ZERO, &p, "", ""));
    assert!(diamond.lines.len() > rectangular.lines.len());
    for line in &diamond.lines {
        assert!(line
            .runs
            .iter()
            .all(|r| r.x >= 80.0 && r.x + r.width <= 240.0 + 1e-6));
    }
    let nowrap = place(&shape_xml(
        "diamond",
        320,
        160,
        &format!("{ZERO} wrap=\"none\""),
        &p,
        "",
        "",
    ));
    assert!(nowrap.text_overflow_right_px() > 0.0);
    let inner = nowrap.text_region.unwrap();
    close(
        nowrap.text_overflow_right_px(),
        nowrap.lines[0].runs[0].x + nowrap.lines[0].runs[0].width - inner.x - inner.width,
    );
    let many = (0..4).map(|_| paragraph("line")).collect::<String>();
    let short = place(&shape_xml("diamond", 320, 160, ZERO, &many, "", ""));
    assert!(short.text_overflow_px() > 0.0);
    close(short.text_overflow_px(), short.text_bottom_px() - 120.0);
}

#[test]
fn adjusted_triangle_and_hexagon_outline_and_text_share_dimensions_and_clamps() {
    for (raw, apex) in [(-10, 0.0), (25000, 0.25), (200000, 1.0)] {
        let adjust = format!("<a:gd name=\"adj\" fmla=\"val {raw}\"/>");
        let s = place(&shape_xml(
            "triangle",
            320,
            160,
            ZERO,
            &paragraph("text"),
            &adjust,
            "",
        ));
        assert!(matches!(&s.geometry,PlacedGeometry::Polygon(p) if p[0]==(apex,0.0)));
        region_close(s.text_region.unwrap(), (160.0 * apex, 80.0, 160.0, 80.0));
    }
    for (w, h, x, inset) in [
        (320, 160, 0.125, 0.125),
        (160, 320, 0.25, 1.0 / 6.0),
        (160, 160, 0.25, 1.0 / 6.0),
    ] {
        let s = place(&shape_xml(
            "hexagon",
            w,
            h,
            ZERO,
            &paragraph("text"),
            "",
            "",
        ));
        let PlacedGeometry::Polygon(p) = s.geometry else {
            panic!()
        };
        close(p[0].0, x);
        region_close(
            s.text_region.unwrap(),
            (
                f64::from(w) * inset,
                f64::from(h) * inset,
                f64::from(w) * (1.0 - 2.0 * inset),
                f64::from(h) * (1.0 - 2.0 * inset),
            ),
        );
    }
    for (raw, x, inset) in [
        (-1, 0.0, 1.0 / 12.0),
        (75000, 0.375, 11.0 / 48.0),
        (200000, 0.5, 7.0 / 24.0),
    ] {
        let adjust = format!("<a:gd name=\"adj\" fmla=\"val {raw}\"/>");
        let s = place(&shape_xml(
            "hexagon",
            320,
            160,
            ZERO,
            &paragraph("text"),
            &adjust,
            "",
        ));
        let PlacedGeometry::Polygon(p) = s.geometry else {
            panic!()
        };
        close(p[0].0, x);
        close(s.text_region.unwrap().x, 320.0 * inset);
    }
}

#[test]
fn vertical_rotated_flipped_and_grouped_text_keeps_caret_hit_round_trips() {
    for vert in ["", "vert=\"vert\"", "vert=\"vert270\""] {
        let body = format!("{ZERO} {vert}");
        for transform in ["", "rot=\"1800000\" flipH=\"1\" flipV=\"1\""] {
            let shape = shape_xml(
                "triangle",
                320,
                240,
                &body,
                &paragraph("日本語abc"),
                "<a:gd name=\"adj\" fmla=\"val 25000\"/>",
                transform,
            );
            for grouped in [false, true] {
                let xml = if grouped {
                    format!(
                        r#"<p:grpSp><p:nvGrpSpPr><p:cNvPr id="10" name="group"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm rot="900000" flipH="1"><a:off x="0" y="0"/><a:ext cx="6096000" cy="3429000"/><a:chOff x="0" y="0"/><a:chExt cx="3048000" cy="2286000"/></a:xfrm></p:grpSpPr>{shape}</p:grpSp>"#
                    )
                } else {
                    shape.clone()
                };
                let s = place(&xml);
                for offset in 0..=6 {
                    let g = s
                        .selection_geometry(offset, offset, offset, false, &HeuristicMeasure)
                        .unwrap();
                    let hit = s
                        .hit_text(
                            (g.caret[0].x + g.caret[1].x) / 2.0,
                            (g.caret[0].y + g.caret[1].y) / 2.0,
                            &HeuristicMeasure,
                        )
                        .unwrap();
                    assert_eq!(hit.offset, offset, "{vert} {transform} grouped={grouped}");
                }
            }
        }
    }
}

#[test]
fn shape_autofit_converges_inside_nonrectangular_text_regions() {
    let many = (0..6).map(|_| paragraph("line")).collect::<String>();
    for preset in ["ellipse", "roundRect", "triangle", "diamond", "hexagon"] {
        let mut xml = shape_xml(
            preset,
            320,
            24,
            &format!("{ZERO} anchor=\"b\""),
            &many,
            "",
            "",
        );
        xml = xml.replace("/><a:lstStyle/>", "><a:spAutoFit/></a:bodyPr><a:lstStyle/>");
        let s = place(&xml);
        assert!(s.rect.height > 24.0);
        assert!(
            s.text_overflow_px() < 1e-6,
            "{preset}: {}",
            s.text_overflow_px()
        );
        let r = s.text_region.unwrap();
        assert!(s.lines[0].top >= r.y - 1e-6);
        assert!(
            s.lines.last().unwrap().top + s.lines.last().unwrap().height <= r.y + r.height + 1e-6
        );
    }
}

#[test]
fn autofit_uses_direct_height_solutions_without_repeated_full_text_walks() {
    struct Count(Cell<usize>);
    impl jade_pptx_layout::TextMeasure for Count {
        fn measure(&self, text: &str, style: &jade_pptx_layout::RunStyle) -> f64 {
            self.0.set(self.0.get() + 1);
            jade_pptx_layout::TextMeasure::measure(&HeuristicMeasure, text, style)
        }
    }
    let paragraphs = (0..6).map(|_| paragraph("line")).collect::<String>();
    for preset in [
        "rect",
        "ellipse",
        "roundRect",
        "triangle",
        "diamond",
        "hexagon",
    ] {
        let xml = shape_xml(preset, 320, 24, ZERO, &paragraphs, "", "");
        let measure = Count(Cell::new(0));
        place_with_measure(&xml, &measure);
        let baseline = measure.0.get();
        assert!(baseline > 0);
        measure.0.set(0);
        let xml = xml.replace("/><a:lstStyle/>", "><a:spAutoFit/></a:bodyPr><a:lstStyle/>");
        let placed = place_with_measure(&xml, &measure);
        let factor = if preset == "hexagon" { 16 } else { 3 };
        assert!(
            measure.0.get() <= baseline * factor,
            "{preset}: {} measurements vs {baseline} for one text flow",
            measure.0.get()
        );
        assert!(placed.text_overflow_px() < 1e-6);
    }
}
