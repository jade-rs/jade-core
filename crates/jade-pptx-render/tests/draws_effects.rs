//! Glow, inner shadow, reflection and soft edge, from slide XML through
//! layout to the painter calls they make.

use jade_painter::recording::{Op, RecordingPainter};
use jade_pptx_layout::{layout_slide, HeuristicMeasure, SlideLayout};
use jade_pptx_reader::{parse_slide_xml, SlideSize, ThemeColors};
use jade_pptx_render::{render_slide, RenderOptions};

/// One 2 × 1 inch rectangle at (1, 1) inch, filled red, with `effects`
/// inside its `<a:effectLst>` and `line` after its fill.
fn layout(effects: &str, line: &str) -> SlideLayout {
    let xml = format!(
        r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree>
        <p:sp><p:nvSpPr><p:cNvPr id="2" name="S"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="914400" y="914400"/><a:ext cx="1828800" cy="914400"/></a:xfrm>
        <a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>{line}
        <a:effectLst>{effects}</a:effectLst></p:spPr></p:sp></p:spTree></p:cSld></p:sld>"#
    );
    let content = parse_slide_xml(&xml).unwrap();
    layout_slide(
        SlideSize {
            width_emu: 9_144_000,
            height_emu: 6_858_000,
        },
        &content,
        &ThemeColors::default(),
        &HeuristicMeasure,
    )
}

fn ops_of(layout: &SlideLayout) -> Vec<Op> {
    let painter = RecordingPainter::new();
    render_slide(&painter, layout, &RenderOptions::default()).unwrap();
    painter.ops()
}

fn red_fill(ops: &[Op]) -> usize {
    ops.iter()
        .position(|op| *op == Op::FillRect(96.0, 96.0, 192.0, 96.0))
        .expect("the shape's own fill")
}

#[test]
fn layout_resolves_effects_to_px_and_colours() {
    let layout = layout(
        r#"<a:glow rad="127000"><a:srgbClr val="00B050"><a:alpha val="50000"/></a:srgbClr></a:glow>
           <a:softEdge rad="63500"/>"#,
        "",
    );
    let effects = &layout.shapes[0].effects;
    let glow = effects.glow.as_ref().unwrap();
    assert!((glow.radius_px - 13.333).abs() < 0.01);
    assert_eq!(glow.color, "rgba(0,176,80,0.5)");
    assert!((effects.soft_edge_px.unwrap() - 6.667).abs() < 0.01);
}

#[test]
fn a_glow_is_the_blurred_silhouette_under_the_shape() {
    let ops = ops_of(&layout(
        r#"<a:glow rad="127000"><a:srgbClr val="00B050"/></a:glow>"#,
        "",
    ));
    let colour = ops
        .iter()
        .position(|op| *op == Op::SetFillStyle("#00B050".to_string()))
        .expect("glow colour");
    // A quarter of the radius as the blur's deviation, the stroke a radius
    // wide so the colour reaches a radius past the edge.
    let blur = ops[colour..]
        .iter()
        .position(
            |op| matches!(op, Op::SetBlurFilter(b) if (*b - 127000.0 / 9525.0 / 4.0).abs() < 1e-6),
        )
        .expect("blurred");
    assert!(ops[colour..].contains(&Op::Stroke));
    assert!(colour + blur < red_fill(&ops), "the glow is under the fill");
}

#[test]
fn an_inner_shadow_is_clipped_to_the_shape_after_its_fill() {
    let ops = ops_of(&layout(
        r#"<a:innerShdw blurRad="38100" dist="38100" dir="2700000"><a:srgbClr val="000000"><a:alpha val="50000"/></a:srgbClr></a:innerShdw>"#,
        "",
    ));
    let fill = red_fill(&ops);
    let colour = ops
        .iter()
        .position(|op| *op == Op::SetFillStyle("rgba(0,0,0,0.5)".to_string()))
        .expect("shadow colour");
    assert!(colour > fill);
    // Clipped to the silhouette before the shadow colour is set.
    assert!(ops[fill..colour].contains(&Op::Clip));
    // The silhouette's hole is the rectangle moved down-right by the
    // distance at 45°.
    let offset = 38100.0 / 9525.0 * std::f64::consts::FRAC_1_SQRT_2;
    assert!(ops[colour..].iter().any(|op| matches!(op,
        Op::MoveTo(x, y) | Op::LineTo(x, y)
            if (*x - (96.0 + offset)).abs() < 1e-6 && (*y - (96.0 + offset)).abs() < 1e-6)));
    assert!(ops[colour..].contains(&Op::Fill));
}

#[test]
fn a_reflection_draws_the_shape_mirrored_and_faded_below_it() {
    let ops = ops_of(&layout(
        r#"<a:reflection stA="50000" endA="0" endPos="40000" dir="5400000" sy="-100000" algn="bl"/>"#,
        "",
    ));
    let own = ops
        .iter()
        .rposition(|op| *op == Op::FillRect(96.0, 96.0, 192.0, 96.0))
        .unwrap();
    // The mirror: a flip about the bottom edge (y = 192).
    let flip = ops
        .iter()
        .position(|op| *op == Op::Scale(1.0, -1.0))
        .expect("mirrored");
    assert!(flip < own, "the reflection is under the shape");
    assert!(ops[..flip].contains(&Op::Translate(96.0, 192.0)));
    // Every pass is translucent, and none is stronger than the start alpha.
    let alphas: Vec<f64> = ops[..own]
        .iter()
        .filter_map(|op| match op {
            Op::SetGlobalAlpha(alpha) => Some(*alpha),
            _ => None,
        })
        .collect();
    assert!(!alphas.is_empty());
    assert!(alphas
        .iter()
        .all(|alpha| *alpha > 0.0 && *alpha <= 0.5 + 1e-9));
}

#[test]
fn a_soft_edged_flat_fill_is_blurred_and_loses_its_outline() {
    let ops = ops_of(&layout(
        r#"<a:softEdge rad="127000"/>"#,
        r#"<a:ln w="12700"><a:solidFill><a:srgbClr val="000000"/></a:solidFill></a:ln>"#,
    ));
    let radius = 127000.0 / 9525.0;
    let blur = ops
        .iter()
        .position(|op| matches!(op, Op::SetBlurFilter(b) if (*b - radius / 4.0).abs() < 1e-6))
        .expect("blurred fill");
    assert_eq!(ops[blur + 1], Op::SetFillStyle("#FF0000".to_string()));
    // Shrunk by half the radius, so the fade straddles the inside edge.
    assert!(ops[..blur].contains(&Op::Scale((192.0 - radius) / 192.0, (96.0 - radius) / 96.0)));
    assert!(!ops.contains(&Op::SetStrokeStyle("#000000".to_string())));
}
