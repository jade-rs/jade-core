use super::*;
use crate::draw_command::{DrawCommand, GlyphRunStep, TextStyle};
use jade_docx_layout::line_metrics::{
    set_word_layout_platform, word_layout_platform, WordLayoutPlatform,
};
use jade_painter::recording::{Op, RecordingPainter};

struct RestorePlatform(WordLayoutPlatform);

impl Drop for RestorePlatform {
    fn drop(&mut self) {
        set_word_layout_platform(self.0);
    }
}

fn platform(value: WordLayoutPlatform) -> RestorePlatform {
    let previous = word_layout_platform();
    set_word_layout_platform(value);
    RestorePlatform(previous)
}

fn steps() -> Vec<GlyphRunStep> {
    // a, soft hyphen, b, a real line-end dash, nonbreaking hyphen.
    [
        (0, 1, 0.0),
        (1, 3, 8.0),
        (3, 4, 8.0),
        (4, 5, 14.0),
        (5, 8, 20.0),
    ]
    .into_iter()
    .map(|(start, end, x)| GlyphRunStep {
        text_start: start,
        text_end: end,
        font_index: 0,
        x_offset: x,
    })
    .collect()
}

fn paint(style: &TextStyle, font: &str) -> Vec<Op> {
    let painter = RecordingPainter::new();
    execute_command_with_font_state(
        &painter,
        &DrawCommand::TextRun {
            text: CommandText::from("a\u{ad}b-\u{2011}"),
            x: 10.0,
            y: 20.0,
            style: style.clone(),
            fonts: vec![Rc::from(font)],
            glyphs: steps(),
        },
        &mut FontStateTracker::new(),
    )
    .unwrap();
    painter.ops()
}

#[test]
fn windows_paints_only_visible_glyphs_at_their_original_positions() {
    let _restore = platform(WordLayoutPlatform::Windows);
    let actual: Vec<_> = paint(&TextStyle::default(), "12pt Arial")
        .into_iter()
        .filter_map(|op| match op {
            Op::FillText(text, x, y) => Some((text, x, y)),
            _ => None,
        })
        .collect();
    assert_eq!(
        actual,
        vec![
            ("a".to_string(), 10.0, 20.0),
            ("b".to_string(), 18.0, 20.0),
            ("-".to_string(), 24.0, 20.0),
            ("\u{2011}".to_string(), 30.0, 20.0),
        ]
    );
}

#[test]
fn mac_keeps_its_existing_source_glyph_paint_path() {
    let _restore = platform(WordLayoutPlatform::Mac);
    assert!(!CommandText::from("a\u{ad}b").requires_windows_control_paint());
    let actual: Vec<_> = paint(&TextStyle::default(), "12pt Arial")
        .into_iter()
        .filter_map(|op| match op {
            Op::FillText(text, x, y) => Some((text, x, y)),
            _ => None,
        })
        .collect();
    assert_eq!(
        actual,
        vec![
            ("a".to_string(), 10.0, 20.0),
            ("\u{ad}".to_string(), 18.0, 20.0),
            ("b".to_string(), 18.0, 20.0),
            ("-".to_string(), 24.0, 20.0),
            ("\u{2011}".to_string(), 30.0, 20.0),
        ]
    );
}

#[test]
fn windows_effect_scaled_and_featured_paths_do_not_reintroduce_the_control() {
    let _restore = platform(WordLayoutPlatform::Windows);
    let mut styles = vec![TextStyle::default()];
    for effect in ["outline", "shadow", "emboss", "imprint", "scaled"] {
        let mut style = TextStyle::default();
        match effect {
            "outline" => style.outline = true,
            "shadow" => style.shadow = true,
            "emboss" => style.emboss = true,
            "imprint" => style.imprint = true,
            _ => style.text_scale_x = 0.8,
        }
        styles.push(style);
    }
    for style in styles {
        for font in ["12pt Arial", "ot:liga 12px Arial"] {
            let painted: Vec<_> = paint(&style, font)
                .into_iter()
                .filter_map(|op| match op {
                    Op::FillText(text, ..)
                    | Op::StrokeText(text, ..)
                    | Op::FillShapedText(text, ..) => Some(text),
                    _ => None,
                })
                .collect();
            assert!(!painted.is_empty());
            assert!(painted.iter().all(|text| !text.contains('\u{ad}')));
            assert!(painted.iter().any(|text| text.contains('-')));
            assert!(painted.iter().any(|text| text.contains('\u{2011}')));
        }
    }
}

#[test]
fn literal_source_slices_paint_hyphens_with_the_original_fonts_and_positions() {
    let _restore = platform(WordLayoutPlatform::Windows);
    let text = "\u{ad}\u{ad}";
    let steps = [
        GlyphRunStep {
            text_start: 0,
            text_end: 2,
            font_index: 0,
            x_offset: 0.0,
        },
        GlyphRunStep {
            text_start: 2,
            text_end: 4,
            font_index: 1,
            x_offset: 7.0,
        },
    ];
    let style = TextStyle {
        literal_soft_hyphen: true,
        ..Default::default()
    };
    let fonts = [Rc::from("12pt Arial"), Rc::from("12pt Calibri")];
    let painter = RecordingPainter::new();
    execute_cached_text_run_with_font_state(
        &painter,
        &CommandText::from(text),
        10.0,
        20.0,
        &style,
        &fonts,
        &steps,
        &mut FontStateTracker::new(),
    )
    .unwrap();
    let actual: Vec<_> = painter
        .ops()
        .into_iter()
        .filter_map(|op| match op {
            Op::FillText(t, x, y) => Some((t, x, y)),
            _ => None,
        })
        .collect();
    assert_eq!(
        actual,
        [("-".to_string(), 10.0, 20.0), ("-".to_string(), 17.0, 20.0)]
    );
    let selected: Vec<_> = painter
        .ops()
        .into_iter()
        .filter_map(|op| match op {
            Op::SetFont(font) => Some(font),
            _ => None,
        })
        .collect();
    assert_eq!(selected, ["12pt Arial", "12pt Calibri"]);
    assert_eq!(text, "\u{ad}\u{ad}");
    assert_eq!(steps[1].text_start, 2);
    let encoded = serde_json::to_string(&style).unwrap();
    assert!(
        serde_json::from_str::<TextStyle>(&encoded)
            .unwrap()
            .literal_soft_hyphen
    );
    assert!(!serde_json::to_string(&TextStyle::default())
        .unwrap()
        .contains("literal_soft_hyphen"));
}

#[test]
fn literal_source_ink_survives_effect_scaling_and_feature_shaping_paths() {
    let _restore = platform(WordLayoutPlatform::Windows);
    for effect in ["plain", "outline", "shadow", "emboss", "imprint", "scaled"] {
        let mut style = TextStyle {
            literal_soft_hyphen: true,
            ..Default::default()
        };
        match effect {
            "outline" => style.outline = true,
            "shadow" => style.shadow = true,
            "emboss" => style.emboss = true,
            "imprint" => style.imprint = true,
            "scaled" => style.text_scale_x = 0.8,
            _ => {}
        }
        for font in ["12pt Arial", "ot:liga 12px Arial"] {
            let reference = RecordingPainter::new();
            let expected_steps = [
                (0, 1, 0.0),
                (1, 2, 8.0),
                (2, 3, 8.0),
                (3, 4, 14.0),
                (4, 7, 20.0),
            ]
            .into_iter()
            .map(|(start, end, x)| GlyphRunStep {
                text_start: start,
                text_end: end,
                font_index: 0,
                x_offset: x,
            })
            .collect::<Vec<_>>();
            execute_cached_text_run_with_font_state(
                &reference,
                &CommandText::from("a-b-\u{2011}"),
                10.0,
                20.0,
                &style,
                &[Rc::from(font)],
                &expected_steps,
                &mut FontStateTracker::new(),
            )
            .unwrap();
            assert_eq!(paint(&style, font), reference.ops(), "{effect}/{font}");
        }
    }
}

#[test]
fn literal_source_annotation_does_not_change_mac_painting() {
    let _restore = platform(WordLayoutPlatform::Mac);
    let style = TextStyle {
        literal_soft_hyphen: true,
        ..Default::default()
    };
    assert_eq!(
        paint(&style, "12pt Arial"),
        paint(&TextStyle::default(), "12pt Arial")
    );
}

#[test]
fn a_hidden_control_does_not_change_the_font_state_of_following_text() {
    let _restore = platform(WordLayoutPlatform::Windows);
    let painter = RecordingPainter::new();
    let reference = RecordingPainter::new();
    let mut state = FontStateTracker::new();
    let style = TextStyle::default();
    let fonts = [Rc::from("12pt Arial")];
    let control = [GlyphRunStep {
        text_start: 0,
        text_end: 2,
        font_index: 0,
        x_offset: 0.0,
    }];
    execute_cached_text_run_with_font_state(
        &painter,
        &CommandText::from("\u{ad}"),
        10.0,
        20.0,
        &style,
        &fonts,
        &control,
        &mut state,
    )
    .unwrap();
    assert!(painter.ops().is_empty());
    let visible = [GlyphRunStep {
        text_start: 0,
        text_end: 1,
        font_index: 0,
        x_offset: 0.0,
    }];
    execute_cached_text_run_with_font_state(
        &painter,
        &CommandText::from("b"),
        18.0,
        20.0,
        &style,
        &fonts,
        &visible,
        &mut state,
    )
    .unwrap();
    execute_cached_text_run_with_font_state(
        &reference,
        &CommandText::from("b"),
        18.0,
        20.0,
        &style,
        &fonts,
        &visible,
        &mut FontStateTracker::new(),
    )
    .unwrap();
    assert_eq!(painter.ops(), reference.ops());
}

#[test]
fn optional_hyphen_formatting_marks_remain_visible() {
    let _restore = platform(WordLayoutPlatform::Windows);
    let painter = RecordingPainter::new();
    execute_command(
        &painter,
        &DrawCommand::ControlMark {
            symbol: Rc::from("\u{ac}"),
            x: 10.0,
            baseline: 20.0,
            style: TextStyle::default(),
        },
    )
    .unwrap();
    assert_eq!(
        painter
            .ops()
            .iter()
            .filter(|op| matches!(op, Op::FillText(text, ..) if text == "\u{ac}"))
            .count(),
        1
    );
}

#[test]
fn discretionary_hyphen_cached_presence_keeps_unicode_and_platform_identity() {
    for value in [WordLayoutPlatform::Windows, WordLayoutPlatform::Mac] {
        let _restore = platform(value);
        for ordinary in ["", "plain ASCII", "a\u{16d}b", "ね", "a\u{2011}b"] {
            assert!(!CommandText::from(ordinary).requires_windows_control_paint());
        }
        assert_eq!(
            CommandText::from("a\u{ad}b").requires_windows_control_paint(),
            value == WordLayoutPlatform::Windows
        );
    }
}

#[test]
fn zero_width_cached_presence_keeps_legacy_wire_and_normal_replay_identity() {
    assert_eq!(
        std::mem::size_of::<CommandText>(),
        std::mem::size_of::<Rc<str>>()
    );
    for value in [WordLayoutPlatform::Windows, WordLayoutPlatform::Mac] {
        let _restore = platform(value);
        for (source, soft, zero_width) in [
            ("", false, false),
            ("ordinary ASCII", false, false),
            ("日本語・ね", false, false),
            ("a\u{200e}b", false, false),
            ("a\u{2}", false, false),
            ("a\u{ad}b", true, false),
            ("A\u{200c}B", false, true),
            ("\u{200d}", false, true),
            ("A\u{200b}B", false, true),
            ("A\u{ad}\u{200d}B", true, true),
            ("👩\u{200d}💻", false, true),
        ] {
            let text = CommandText::from(source);
            let wire = serde_json::to_string(&text).unwrap();
            assert_eq!(wire, serde_json::to_string(source).unwrap());
            let restored: CommandText = serde_json::from_str(&wire).unwrap();
            assert_eq!(restored.as_ref(), source);
            assert_eq!(restored.has_soft_hyphen(), soft);
            assert_eq!(restored.has_zero_width_controls(), zero_width);
            assert_eq!(
                restored.requires_windows_control_paint(),
                value == WordLayoutPlatform::Windows && (soft || zero_width),
                "{source:?} {value:?}"
            );
        }
    }
}

#[test]
fn discretionary_hyphen_cached_text_preserves_legacy_wire_and_unicode_identity() {
    for (source, has_control) in [
        ("plain ASCII", false),
        ("a\u{16d}bね", false),
        ("a\u{ad}b", true),
    ] {
        let text = CommandText::from(source);
        let cloned = text.clone();
        assert!(Rc::ptr_eq(&text.encoded, &cloned.encoded));
        assert_eq!(cloned.has_soft_hyphen(), has_control);
        let wire = serde_json::to_string(&text).unwrap();
        assert_eq!(wire, serde_json::to_string(source).unwrap());
        let restored: CommandText = serde_json::from_str(&wire).unwrap();
        assert_eq!(restored.as_ref(), source);
        assert_eq!(restored.has_soft_hyphen(), has_control);
    }
}

#[test]
fn discretionary_hyphen_text_run_legacy_roundtrip_and_offset_keep_cached_presence() {
    let source = "a\u{ad}b-\u{2011}";
    let command = DrawCommand::TextRun {
        text: source.into(),
        x: 10.0,
        y: 20.0,
        style: TextStyle::default(),
        fonts: vec![Rc::from("12pt Arial")],
        glyphs: steps(),
    };
    let wire = serde_json::to_value(&command).unwrap();
    let fields = wire["TextRun"].as_object().unwrap();
    assert_eq!(fields.len(), 6);
    assert_eq!(fields["text"], source);
    let decoded: DrawCommand = serde_json::from_value(wire).unwrap();
    assert_eq!(decoded, command);
    let DrawCommand::TextRun { text, y, .. } = decoded.with_y_offset(5.0) else {
        panic!("expected text run")
    };
    assert_eq!(y, 25.0);
    assert_eq!(text.as_ref(), source);
    assert!(text.has_soft_hyphen());
}

#[test]
fn shared_control_cache_keeps_empty_unicode_and_terminal_control_bytes_exact() {
    for source in [
        "",
        "plain",
        "\0",
        "\u{1}",
        "a\0\u{1}",
        "\u{ad}",
        "日\u{ad}😀\0\u{1}",
    ] {
        let text = CommandText::from(source);
        let cloned = text.clone();
        drop(text);
        assert_eq!(cloned.as_ref(), source);
        assert_eq!(cloned.has_soft_hyphen(), source.contains('\u{ad}'));
        assert_eq!(format!("{cloned:?}"), format!("{source:?}"));
        assert_eq!(cloned.to_string(), source);
        let json = serde_json::to_string(&cloned).unwrap();
        assert_eq!(json, serde_json::to_string(source).unwrap());
        let restored: CommandText = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.as_ref(), source);
        assert_eq!(restored, cloned);
    }
}
