//! Reusing a font string must not change what the painter is told.
//!
//! `PainterMeasure` skips rebuilding the font string when the style repeats,
//! and the arc-warp path builds one string per run rather than per glyph. Both
//! are only allowed to remove string construction: the recorded painter
//! commands, from measuring and from painting, stay exactly the same.

use std::cell::RefCell;

use jade_painter::recording::{Op, RecordingPainter};
use jade_painter::Painter;
use jade_pptx_layout::{layout_slide, HeuristicMeasure, RunStyle, SlideLayout, TextMeasure};
use jade_pptx_reader::{parse_slide_xml, SlideSize, ThemeColors};
use jade_pptx_render::{render_slide, PainterMeasure, RenderOptions};

/// The measurer as it was before the style check: the font string is rebuilt
/// for every call, and only the `set_font` push is skipped when unchanged.
struct OldMeasure<P: Painter> {
    painter: P,
    default_font: String,
    last_font: RefCell<String>,
}

impl<P: Painter> TextMeasure for OldMeasure<P> {
    fn measure(&self, text: &str, style: &RunStyle) -> f64 {
        let mut shorthand = String::new();
        if style.italic {
            shorthand.push_str("italic ");
        }
        if style.bold {
            shorthand.push_str("bold ");
        }
        let mut faces: Vec<&str> = Vec::new();
        for face in [style.font.as_deref(), style.east_asian_font.as_deref()]
            .into_iter()
            .flatten()
        {
            if !faces.contains(&face) {
                faces.push(face);
            }
        }
        if (faces.is_empty() || style.font.is_none())
            && !faces.contains(&self.default_font.as_str())
        {
            faces.push(&self.default_font);
        }
        let list = faces
            .iter()
            .map(|face| format!("'{face}'"))
            .collect::<Vec<_>>()
            .join(", ");
        shorthand.push_str(&format!("{}pt {list}", style.size_pt));
        if *self.last_font.borrow() != shorthand {
            self.painter.set_font(&shorthand);
            *self.last_font.borrow_mut() = shorthand;
        }
        self.painter
            .measure_text_width(text)
            .unwrap_or_else(|_| HeuristicMeasure.measure(text, style))
    }
}

fn run(size: u32, attrs: &str, faces: &str, text: &str) -> String {
    format!(
        r#"<a:r><a:rPr lang="en-US" sz="{size}" {attrs}>{faces}</a:rPr><a:t>{text}</a:t></a:r>"#
    )
}

/// Several runs in mixed faces and sizes, repeated characters, Latin and CJK.
fn slide_xml(warp: Option<&str>) -> String {
    let helv = r#"<a:latin typeface="Helvetica Neue"/>"#;
    let both =
        r#"<a:latin typeface="Helvetica Neue"/><a:ea typeface="Hiragino Kaku Gothic ProN"/>"#;
    let runs = [
        run(1800, "", helv, "aaaa bbbb aaaa 長い文章の例 "),
        run(1800, r#"b="1" i="1""#, both, "mixed 対応状況 mixed "),
        run(1400, "", "", "no face named, repeated eeeeeeee "),
        run(2400, r#"b="1""#, both, "日本語のテキスト "),
        run(1800, "", helv, "aaaa bbbb tail that wraps around"),
    ]
    .concat();
    let body_pr = match warp {
        Some(preset) => format!(
            r#"<a:bodyPr wrap="none"><a:prstTxWarp prst="{preset}"><a:avLst/></a:prstTxWarp></a:bodyPr>"#
        ),
        None => "<a:bodyPr/>".to_string(),
    };
    format!(
        r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree>
        <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>
        <p:sp><p:nvSpPr><p:cNvPr id="2" name="Body"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="5486400" cy="2743200"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>
        <p:txBody>{body_pr}<a:lstStyle/><a:p>{runs}</a:p></p:txBody></p:sp>
        </p:spTree></p:cSld></p:sld>"#
    )
}

fn layout_with(xml: &str, measure: &dyn TextMeasure) -> SlideLayout {
    let content = parse_slide_xml(xml).expect("slide");
    layout_slide(
        SlideSize::default(),
        &content,
        &ThemeColors::default(),
        measure,
    )
}

fn painted(layout: &SlideLayout) -> Vec<Op> {
    let painter = RecordingPainter::new();
    render_slide(&painter, layout, &RenderOptions::default()).expect("paint");
    painter.ops()
}

/// (measuring ops, painting ops) for one measurer.
type Recorded = (Vec<Op>, Vec<Op>);

/// Returns the recording for the new measurer and for the oracle.
fn both_ways(xml: &str) -> (Recorded, Recorded) {
    let new_painter = RecordingPainter::with_width_per_char(7.0);
    let new_layout = layout_with(xml, &PainterMeasure::new(new_painter.clone(), "Calibri"));
    let old_painter = RecordingPainter::with_width_per_char(7.0);
    let old = OldMeasure {
        painter: old_painter.clone(),
        default_font: "Calibri".to_string(),
        last_font: RefCell::new(String::new()),
    };
    let old_layout = layout_with(xml, &old);
    (
        (new_painter.ops(), painted(&new_layout)),
        (old_painter.ops(), painted(&old_layout)),
    )
}

#[test]
fn long_mixed_text_records_the_same_commands_as_the_old_measurer() {
    let (new, old) = both_ways(&slide_xml(None));
    let measures = new
        .0
        .iter()
        .filter(|op| matches!(op, Op::MeasureText(_)))
        .count();
    assert!(
        measures > 100,
        "the slide measures a lot of text: {measures}"
    );
    assert_eq!(new.0, old.0, "measuring commands");
    assert_eq!(new.1, old.1, "painting commands");
}

#[test]
fn warped_mixed_text_records_the_same_commands_as_the_old_measurer() {
    for preset in ["textArchUp", "textArchDown", "textCircle"] {
        let (new, old) = both_ways(&slide_xml(Some(preset)));
        assert_eq!(new.0, old.0, "{preset}: measuring commands");
        assert_eq!(new.1, old.1, "{preset}: painting commands");
        let rotations = new
            .1
            .iter()
            .filter(|op| matches!(op, Op::Rotate(_)))
            .count();
        assert!(rotations > 50, "{preset}: glyphs were drawn one at a time");
    }
}

/// One `set_font` per non-space glyph is part of the recorded sequence, and
/// each names the font of the run that glyph belongs to.
#[test]
fn a_warp_sets_the_run_font_before_every_glyph() {
    let painter = RecordingPainter::new();
    let layout = layout_with(&slide_xml(Some("textArchUp")), &HeuristicMeasure);
    render_slide(&painter, &layout, &RenderOptions::default()).expect("paint");
    let ops = painter.ops();
    let fills: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter_map(|(i, op)| matches!(op, Op::FillText(..)).then_some(i))
        .collect();
    assert!(!fills.is_empty());
    for index in fills {
        let before = &ops[..index];
        assert!(
            before
                .iter()
                .rev()
                .take(6)
                .any(|op| matches!(op, Op::SetFont(_))),
            "a font is set just before each glyph"
        );
    }
}
