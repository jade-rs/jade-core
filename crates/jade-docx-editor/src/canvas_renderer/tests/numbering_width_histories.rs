//! Width-only numbering must agree with the former visual plan across edits.
use super::*;
use jade_docx_layout::line_metrics::WordLayoutPlatform;
use proptest::prelude::*;

#[derive(Clone, Debug)]
enum Edit {
    Label(String),
    Style(u16),
    Context(u16),
    ClearMeasurements,
}

fn edit() -> impl Strategy<Value = Edit> {
    let label = prop_oneof![
        prop::sample::select(vec![
            "".to_owned(),
            " ".into(),
            "17. ".into(),
            "第１７条".into(),
            "AVé\u{301}税😀 ".into(),
            "אב(١) ".into(),
            "A\0B  ".into(),
            "👩\u{200d}💻🇯🇵 ".into(),
            "１２３\u{3000}".into(),
            "IV. ".repeat(40),
        ]),
        prop::collection::vec(
            prop::sample::select(vec!['A', 'V', '1', '税', 'é', '\u{301}', '😀', ' ', '\0']),
            0..24
        )
        .prop_map(|chars| chars.into_iter().collect()),
    ];
    prop_oneof![
        3 => label.prop_map(Edit::Label),
        2 => any::<u16>().prop_map(Edit::Style),
        2 => any::<u16>().prop_map(Edit::Context),
        1 => Just(Edit::ClearMeasurements),
    ]
}

fn properties(code: u16) -> RunProperties {
    RunProperties {
        font_size: Some([8.0, 10.5, 16.0][usize::from(code % 3)]),
        char_spacing: Some([-0.75, 0.0, 0.25][usize::from((code / 3) % 3)]),
        text_scale: Some([50.0, 100.0, 150.0][usize::from((code / 9) % 3)]),
        text_position: Some(if code & 32 == 0 { 0.0 } else { 2.0 }),
        kerning: (code & 64 != 0).then_some(1.0),
        bold: Some(code & 128 != 0),
        fonts: Some(RunFonts {
            ascii: Some(
                if code & 256 == 0 {
                    "Arial"
                } else {
                    "Courier New"
                }
                .into(),
            ),
            h_ansi: Some("Times New Roman".into()),
            east_asia: Some("MS Gothic".into()),
            complex: Some("Arial".into()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn renderer_with_width(
    width: f64,
) -> (
    CanvasRenderer<jade_painter::recording::RecordingPainter>,
    jade_painter::recording::RecordingPainter,
) {
    let painter = jade_painter::recording::RecordingPainter::with_width_per_char(width);
    let renderer = CanvasRenderer::with_host(
        Box::new(NonCanvasHost {
            painter: painter.clone(),
        }),
        true,
    )
    .unwrap();
    (renderer, painter)
}

fn check_history(edits: &[Edit], host_width: f64) {
    let (mut measured, measured_painter) = renderer_with_width(host_width);
    let (mut reference, reference_painter) = renderer_with_width(host_width);
    let mut text = "1. ".to_owned();
    let mut style_code = 0;
    let mut context_code = 0;
    for edit in edits {
        match edit {
            Edit::Label(value) => text.clone_from(value),
            Edit::Style(value) => style_code = *value,
            Edit::Context(value) => context_code = *value,
            Edit::ClearMeasurements => {
                measured.clear_text_measure_cache();
                reference.clear_text_measure_cache();
            }
        }
        let props = properties(style_code);
        let style = measured.resolve_applied_run_style_cached(Some(&props));
        let reference_style = reference.resolve_applied_run_style_cached(Some(&props));
        let mut paragraph = ParagraphStyle {
            word_platform: if context_code & 1 == 0 {
                WordLayoutPlatform::Mac
            } else {
                WordLayoutPlatform::Windows
            },
            enable_line_break_rules: context_code & 2 != 0,
            doc_grid_snap_to_chars: context_code & 4 != 0,
            doc_grid_char_space_px: (context_code & 8 != 0).then_some(0.75),
            ..Default::default()
        };
        paragraph
            .east_asian_rules
            .balance_single_byte_double_byte_width = context_code & 16 != 0;
        let context = NumberingTextContext {
            paragraph_style: (context_code & 32 != 0).then_some(&paragraph),
            suffix_space: context_code & 64 != 0,
            following_grapheme: [None, Some("A"), Some("税"), Some(" ")]
                [usize::from((context_code / 128) % 4)],
            margin_line_number: context_code & 512 != 0,
        };
        let ink = if context_code & 1024 == 0 {
            FragmentInkBounds::Unused
        } else {
            FragmentInkBounds::Measure
        };
        let arena = Bump::new();
        measured_painter.clear();
        reference_painter.clear();
        // The true specialization retains the pre-optimization visual walk.
        // Use a separate renderer so both cold and warm font/measurement state
        // must produce the same side effects after every edit.
        let full = reference
            .build_numbering_layout::<true>(&arena, &text, &reference_style, context, ink)
            .unwrap();
        let width = measured
            .build_numbering_layout::<false>(&arena, &text, &style, context, ink)
            .unwrap();
        assert_eq!(
            (
                width.width.to_bits(),
                width.top_offset.to_bits(),
                width.bottom_offset.to_bits(),
                width.fragment.start,
                width.fragment.end,
                width.fragment.width.to_bits()
            ),
            (
                full.width.to_bits(),
                full.top_offset.to_bits(),
                full.bottom_offset.to_bits(),
                full.fragment.start,
                full.fragment.end,
                full.fragment.width.to_bits()
            ),
            "{edit:?}: {text:?}"
        );
        assert_eq!(width.end_state, full.end_state, "{edit:?}: {text:?}");
        assert!(width.fragment.glyphs.is_empty() && width.glyph_steps.is_empty());
        let covered: String = full
            .glyph_steps
            .iter()
            .map(|step| step.text(&text))
            .collect();
        assert_eq!(covered, text, "visual plans must retain all marker text");
        assert_eq!(measured_painter.ops(), reference_painter.ops());

        measured_painter.clear();
        reference_painter.clear();
        let actual = measured
            .measure_numbering_text_plan(&text, &style, context)
            .unwrap();
        let expected = reference
            .build_numbering_layout::<true>(
                &arena,
                &text,
                &reference_style,
                context,
                FragmentInkBounds::Unused,
            )
            .unwrap()
            .width;
        assert_eq!(actual.to_bits(), expected.to_bits());
        assert_eq!(measured_painter.ops(), reference_painter.ops());

        // Measurement may precede painting on the same renderer. It must not
        // change the emitted commands, painted ink, or auxiliary capture.
        let capture = context_code & 2048 != 0;
        measured.set_auxiliary_story_capture_enabled(capture);
        reference.set_auxiliary_story_capture_enabled(capture);
        measured_painter.clear();
        reference_painter.clear();
        let mut commands = Vec::new();
        let mut reference_commands = Vec::new();
        let painted_width = measured
            .draw_numbering_text_plan(
                (&text, 0),
                (40.0, 20.0),
                Some(&props),
                &style,
                context,
                &mut Some(&mut commands),
            )
            .unwrap();
        let reference_width = reference
            .draw_numbering_text_plan(
                (&text, 0),
                (40.0, 20.0),
                Some(&props),
                &reference_style,
                context,
                &mut Some(&mut reference_commands),
            )
            .unwrap();
        assert_eq!(painted_width.to_bits(), expected.to_bits());
        assert_eq!(painted_width.to_bits(), reference_width.to_bits());
        assert_eq!(measured_painter.ops(), reference_painter.ops());
        assert_eq!(commands, reference_commands);
        let [DrawCommand::TextRun {
            text: command_text,
            glyphs,
            ..
        }] = commands.as_slice()
        else {
            panic!("expected the positioned marker command");
        };
        assert_eq!(command_text.as_ref(), text);
        assert_eq!(glyphs.len(), full.glyph_steps.len());
        for (glyph, planned) in glyphs.iter().zip(&full.glyph_steps) {
            assert_eq!(glyph.text(&text), Some(planned.text(&text)));
            assert_eq!(glyph.x_offset.to_bits(), planned.x_offset.to_bits());
        }
        assert_eq!(
            format!("{:?}", measured.auxiliary_story_debug_lines()),
            format!("{:?}", reference.auxiliary_story_debug_lines())
        );
    }
}

#[test]
fn numbering_width_preserves_suffix_unicode_and_cache_transitions() {
    let mut edits = Vec::new();
    for label in ["", " ", "  ", "17. ", "第１７条", "AVé\u{301}😀 ", "אב١ "] {
        edits.push(Edit::Label(label.into()));
        for context in [0, 64, 127, 4095] {
            edits.extend([
                Edit::Context(context),
                Edit::Style(511),
                Edit::ClearMeasurements,
                Edit::Style(0),
            ]);
        }
    }
    for width in [0.0, 7.25] {
        check_history(&edits, width);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]
    #[test]
    fn numbering_width_matches_visual_plan_after_edits(edits in prop::collection::vec(edit(), 1..40), host_width in 0u8..64) {
        check_history(&edits, f64::from(host_width) / 4.0);
    }
}
