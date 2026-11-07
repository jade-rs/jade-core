use super::recording_host::RecordingHost;
use super::*;
use jade_painter::recording::{Op, RecordingPainter};
use proptest::prelude::*;

fn renderer() -> (CanvasRenderer<RecordingPainter>, RecordingPainter) {
    let painter = RecordingPainter::with_width_per_char(10.0);
    let renderer = CanvasRenderer::with_host(
        Box::new(RecordingHost {
            painter: painter.clone(),
        }),
        true,
    )
    .unwrap();
    (renderer, painter)
}

fn properties(size: f32, bold: bool, italic: bool, custom_ascii: bool) -> RunProperties {
    RunProperties {
        font_size: Some(size),
        bold: Some(bold),
        italic: Some(italic),
        // Complex-script styling and the other script families must not leak
        // into the Latin descriptor used for formatting marks.
        bold_complex: Some(!bold),
        italic_complex: Some(!italic),
        fonts: Some(RunFonts {
            ascii: custom_ascii.then(|| Rc::from("Control Mark Face")),
            h_ansi: Some("Wrong HAnsi".into()),
            east_asia: Some("Wrong East Asia".into()),
            complex: Some("Wrong Complex".into()),
            ..RunFonts::default()
        }),
        ..RunProperties::default()
    }
}

#[test]
fn control_mark_fonts_keep_literal_sizes_prefixes_and_ascii_families() {
    let (renderer, painter) = renderer();
    for (size, scaled) in [
        (12.0, "9.00"),
        (18.0, "13.50"),
        (10.5, "7.88"),
        (2.0, "1.50"),
        (1.0, "1.00"),
        (0.0, "1.00"),
        (-6.0, "1.00"),
    ] {
        for (bold, italic, prefix) in [
            (false, false, ""),
            (true, false, "bold "),
            (false, true, "italic "),
            (true, true, "bold italic "),
        ] {
            for custom in [false, true] {
                let mut props = properties(size, bold, italic, custom);
                if !custom {
                    // Specify ASCII explicitly so HAnsi cannot be its fallback.
                    props.fonts.as_mut().unwrap().ascii = Some("Times New Roman".into());
                }
                let style = renderer.resolve_applied_run_style_uncached(Some(&props));
                let family = if custom {
                    "'Control Mark Face', 'Times New Roman', serif"
                } else {
                    "'Times New Roman', serif"
                };
                let expected = format!("{prefix}{scaled}pt {family}");
                for symbol in ["¶", "→", "·", "¶"] {
                    painter.clear();
                    renderer
                        .draw_control_mark(&painter, symbol, 10.0, 100.0, &style)
                        .unwrap();
                    let first_font = painter.ops().into_iter().find_map(|op| match op {
                        Op::SetFont(font) => Some(font),
                        _ => None,
                    });
                    assert_eq!(first_font.as_deref(), Some(expected.as_str()));
                }
                assert_eq!(style.clone().control_mark_descriptor(), expected);
            }
        }
    }
}

#[test]
fn control_mark_fonts_keep_opentype_features_when_document_compatibility_changes() {
    use jade_docx_layout::font_resolver::{
        document_opentype_features_enabled, set_document_opentype_features_enabled,
    };
    let (renderer, _) = renderer();
    let mut props = properties(12.0, true, true, true);
    props.text_effects = Some(Box::new(jade_docx_reader::document::RunTextEffects {
        ligatures: Some("standard".into()),
        ..Default::default()
    }));
    let previous = document_opentype_features_enabled();
    set_document_opentype_features_enabled(true);
    let modern = renderer.resolve_applied_run_style_uncached(Some(&props));
    let expected = "ot:liga,-clig,-calt,-kern bold italic 9.00pt 'Control Mark Face', 'Times New Roman', serif";
    assert_eq!(modern.control_mark_descriptor(), expected);
    set_document_opentype_features_enabled(false);
    let legacy = renderer.resolve_applied_run_style_uncached(Some(&props));
    assert_eq!(
        legacy.control_mark_descriptor(),
        "bold italic 9.00pt 'Control Mark Face', 'Times New Roman', serif"
    );
    // An already resolved style owns its document's font inputs.
    assert_eq!(modern.clone().control_mark_descriptor(), expected);
    set_document_opentype_features_enabled(previous);
}

#[test]
fn warm_and_cloned_mark_styles_emit_the_uncached_painter_trace() {
    let (actual, actual_painter) = renderer();
    let (reference, reference_painter) = renderer();
    let props: Vec<_> = [(12.0, false, false), (18.0, true, true), (0.0, false, true)]
        .into_iter()
        .map(|(size, bold, italic)| properties(size, bold, italic, true))
        .collect();
    let mut styles: Vec<_> = props
        .iter()
        .map(|props| actual.resolve_applied_run_style_uncached(Some(props)))
        .collect();
    for round in 0..4 {
        for index in [0, 1, 2, 1, 0, 2] {
            let expected_style = reference.resolve_applied_run_style_uncached(Some(&props[index]));
            // The original path formatted this value on every draw. Supplying
            // it directly makes the reference independent of lazy resolution.
            let size = (f64::from(expected_style.font_size_pt) * 0.75).max(1.0) as f32;
            expected_style
                .control_mark_font
                .set(Rc::from(
                    expected_style.descriptor_with_size(ScriptCategory::Latin, size),
                ))
                .unwrap();
            let symbol = ["¶", "→", "·", "👩‍💻"][round];
            actual.activate_applied_run_style(&styles[index]);
            reference.activate_applied_run_style(&expected_style);
            let width = actual
                .draw_control_mark(&actual_painter, symbol, 5.0, 30.0, &styles[index])
                .unwrap();
            let expected_width = reference
                .draw_control_mark(&reference_painter, symbol, 5.0, 30.0, &expected_style)
                .unwrap();
            assert_eq!(width, expected_width);
            assert_eq!(actual_painter.ops(), reference_painter.ops());
            actual_painter.clear();
            reference_painter.clear();
            styles[index] = styles[index].clone();
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn control_mark_descriptor_preserves_the_uncached_float_contract(
        bits in any::<u32>(), bold in any::<bool>(), italic in any::<bool>(),
    ) {
        let (renderer, _) = renderer();
        let props = properties(f32::from_bits(bits), bold, italic, true);
        let style = renderer.resolve_applied_run_style_uncached(Some(&props));
        let size = (f64::from(style.font_size_pt) * 0.75).max(1.0) as f32;
        let expected = style.descriptor_with_size(ScriptCategory::Latin, size);
        prop_assert_eq!(style.control_mark_descriptor(), expected.as_str());
        let cloned = style.clone();
        prop_assert_eq!(cloned.control_mark_descriptor(), expected.as_str());
    }
}
