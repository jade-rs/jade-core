//! WordArt transforms (`prstTxWarp`): arch and circle text bends along an
//! ellipse in the shape a glyph at a time; other presets draw flat.

use jade_painter::recording::{Op, RecordingPainter};
use jade_pptx_layout::{layout_slide, HeuristicMeasure, SlideLayout};
use jade_pptx_reader::{parse_slide_xml, ShapeItem, SlideSize, ThemeColors};
use jade_pptx_render::{render_slide, RenderOptions};

fn layout(preset: &str) -> SlideLayout {
    let xml = format!(
        r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree>
        <p:sp><p:nvSpPr><p:cNvPr id="2" name="W"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1828800" cy="914400"/></a:xfrm><a:prstGeom prst="rect"/></p:spPr>
        <p:txBody><a:bodyPr wrap="none"><a:prstTxWarp prst="{preset}"><a:avLst/></a:prstTxWarp></a:bodyPr><a:lstStyle/>
        <a:p><a:pPr algn="ctr"/><a:r><a:rPr lang="en-US" sz="2000"/><a:t>ABC</a:t></a:r></a:p></p:txBody></p:sp>
        </p:spTree></p:cSld></p:sld>"#
    );
    let content = parse_slide_xml(&xml).unwrap();
    let ShapeItem::Shape(shape) = &content.shapes[0] else {
        panic!("a shape");
    };
    let warp = shape.text.as_ref().unwrap().warp.clone();
    assert_eq!(warp.as_deref(), (preset != "textNoShape").then_some(preset));
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

fn glyph_turns(layout: &SlideLayout) -> Vec<f64> {
    let painter = RecordingPainter::new();
    render_slide(&painter, layout, &RenderOptions::default()).unwrap();
    painter
        .ops()
        .iter()
        .filter_map(|op| match op {
            Op::Rotate(angle) => Some(*angle),
            _ => None,
        })
        .collect()
}

#[test]
fn an_upward_arch_turns_each_glyph_with_the_curve() {
    let layout = layout("textArchUp");
    assert_eq!(
        layout.shapes[0].effects.text_warp.as_deref(),
        Some("textArchUp")
    );
    let turns = glyph_turns(&layout);
    // One turn per glyph: leaning left at the start, upright mid-way,
    // leaning right at the end.
    assert_eq!(turns.len(), 3);
    assert!(turns[0] < std::f64::consts::TAU - 0.2 && turns[0] > std::f64::consts::PI);
    assert!((turns[1] - std::f64::consts::TAU).abs() < 1e-6);
    assert!(turns[2] > std::f64::consts::TAU + 0.2);
}

#[test]
fn a_downward_arch_runs_along_the_bottom() {
    let turns = glyph_turns(&layout("textArchDown"));
    assert_eq!(turns.len(), 3);
    assert!(turns[0] > 0.2);
    assert!(turns[1].abs() < 1e-6);
    assert!(turns[2] < -0.2);
}

#[test]
fn presets_beyond_the_arcs_draw_flat() {
    assert!(glyph_turns(&layout("textWave1")).is_empty());
    assert!(glyph_turns(&layout("textNoShape")).is_empty());
}
