//! Measuring a slide's text against the backend that will draw it.
//!
//! The point of `PainterMeasure` is that layout and paint cannot disagree
//! about the font: a browser that wraps against Helvetica and then draws in
//! Hiragino puts line ends in the wrong place. So what is asserted here is
//! not a width — it is the font string the measurer pushed, next to the one
//! the renderer pushes for the same run.

use jade_painter::recording::{Op, RecordingPainter};
use jade_pptx_layout::{layout_slide, RunStyle, TextMeasure};
use jade_pptx_reader::{parse_slide_xml, SlideSize, ThemeColors};
use jade_pptx_render::{render_slide, PainterMeasure, RenderOptions};

fn fonts_pushed(ops: &[Op]) -> Vec<String> {
    ops.iter()
        .filter_map(|op| match op {
            Op::SetFont(font) => Some(font.clone()),
            _ => None,
        })
        .collect()
}

/// A run naming both a Latin and an East Asian face has to be measured with
/// both listed, in that order — otherwise a Japanese string is measured
/// against a face that has no glyphs for it.
#[test]
fn measures_with_both_faces_a_run_names() {
    let painter = RecordingPainter::with_width_per_char(9.0);
    let measure = PainterMeasure::new(painter.clone(), "Calibri");

    let width = measure.measure(
        "対応状況",
        &RunStyle {
            size_pt: 18.0,
            bold: true,
            italic: false,
            font: Some("Helvetica Neue".to_string()),
            east_asian_font: Some("Hiragino Kaku Gothic ProN".to_string()),
        },
    );

    assert_eq!(width, 36.0, "four characters at the backend's 9 px each");
    assert_eq!(
        fonts_pushed(&painter.ops()),
        vec!["bold 18pt 'Helvetica Neue', 'Hiragino Kaku Gothic ProN'"],
    );
}

/// The font is only pushed when it changes. Measuring runs a grapheme at a
/// time, and setting the font per grapheme is the expensive half.
#[test]
fn pushes_the_font_only_when_it_changes() {
    let painter = RecordingPainter::with_width_per_char(1.0);
    let measure = PainterMeasure::new(painter.clone(), "Calibri");
    let style = RunStyle {
        size_pt: 12.0,
        bold: false,
        italic: false,
        font: Some("Helvetica Neue".to_string()),
        east_asian_font: None,
    };
    let italic = RunStyle {
        italic: true,
        ..style.clone()
    };

    measure.measure("a", &style);
    measure.measure("b", &style);
    measure.measure("c", &italic);
    measure.measure("d", &style);

    assert_eq!(
        fonts_pushed(&painter.ops()),
        vec![
            "12pt 'Helvetica Neue'",
            "italic 12pt 'Helvetica Neue'",
            "12pt 'Helvetica Neue'",
        ],
        "the repeat of the first font is not pushed again",
    );
}

/// A run naming no face falls back to the render options' default, so the
/// measurer must be handed the same default the painting will use.
#[test]
fn falls_back_to_the_default_face() {
    let painter = RecordingPainter::with_width_per_char(1.0);
    let measure = PainterMeasure::new(painter.clone(), "Calibri");

    measure.measure(
        "x",
        &RunStyle {
            size_pt: 10.0,
            bold: false,
            italic: false,
            font: None,
            east_asian_font: None,
        },
    );

    assert_eq!(fonts_pushed(&painter.ops()), vec!["10pt 'Calibri'"]);
}

/// The measurer and the renderer have to agree. This lays out a slide with
/// one run through `PainterMeasure`, then paints it, and asserts the font the
/// paint pushed is the one the measurement pushed.
#[test]
fn measurement_and_paint_name_the_same_font() {
    // Built from XML rather than from the reader's structs: this asserts a
    // contract between two crates, and should not break when a third changes
    // a field.
    let xml = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
      <p:cSld><p:spTree>
        <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
        <p:grpSpPr/>
        <p:sp>
          <p:nvSpPr><p:cNvPr id="2" name="Body"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
          <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="4572000" cy="1143000"/></a:xfrm>
            <a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>
          <p:txBody><a:bodyPr/><a:p><a:r>
            <a:rPr sz="1800"><a:latin typeface="Helvetica Neue"/><a:ea typeface="Hiragino Kaku Gothic ProN"/></a:rPr>
            <a:t>対応状況</a:t>
          </a:r></a:p></p:txBody>
        </p:sp>
      </p:spTree></p:cSld>
    </p:sld>"#;
    let content = parse_slide_xml(xml).expect("slide");

    let measuring = RecordingPainter::with_width_per_char(9.0);
    let layout = layout_slide(
        SlideSize::default(),
        &content,
        &ThemeColors::default(),
        &PainterMeasure::new(measuring.clone(), "Calibri"),
    );

    let painting = RecordingPainter::new();
    render_slide(&painting, &layout, &RenderOptions::default()).expect("paint");

    let measured = fonts_pushed(&measuring.ops());
    let painted = fonts_pushed(&painting.ops());
    assert_eq!(
        measured,
        vec!["18pt 'Helvetica Neue', 'Hiragino Kaku Gothic ProN'"],
    );
    assert_eq!(
        painted, measured,
        "paint must name the faces the wrap was measured against",
    );
}
