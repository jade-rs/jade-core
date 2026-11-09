use jade_docx_layout::font_measurer::{
    FontdueLineWrapContext, FontdueMeasurer, RegisteredFontStyle,
};
use jade_docx_layout::line_wrap::{
    layout_cell_text_vertically, LineWrapContext, ParagraphStyle, RunStyle,
};
use jade_docx_layout::measure_backend::WrapBackend;
use jade_docx_reader::RunProperties;
use std::{fmt::Debug, path::Path};

fn table_offset(bytes: &[u8], tag: &[u8]) -> usize {
    let count = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
    (0..count)
        .find_map(|i| {
            let record = 12 + i * 16;
            (&bytes[record..record + 4] == tag).then(|| {
                u32::from_be_bytes(bytes[record + 8..record + 12].try_into().unwrap()) as usize
            })
        })
        .unwrap()
}

// A non-em vertical advance proves these backends read font metrics.
fn test_font(vertical_metrics: bool) -> Vec<u8> {
    let mut bytes = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fonts/Noto_Sans_JP/static/NotoSansJP-Regular.ttf"),
    )
    .unwrap();
    let head = table_offset(&bytes, b"head");
    let em = u16::from_be_bytes(bytes[head + 18..head + 20].try_into().unwrap());
    let advance = em * 3 / 4;
    let vhea = table_offset(&bytes, b"vhea");
    let vmtx = table_offset(&bytes, b"vmtx");
    let count = u16::from_be_bytes(bytes[vhea + 34..vhea + 36].try_into().unwrap()) as usize;
    for i in 0..count {
        bytes[vmtx + i * 4..vmtx + i * 4 + 2].copy_from_slice(&advance.to_be_bytes());
    }
    if !vertical_metrics {
        let count = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
        for i in 0..count {
            let record = 12 + i * 16;
            if &bytes[record..record + 4] == b"vmtx" {
                bytes[record..record + 4].copy_from_slice(b"TEST");
            }
        }
    }
    bytes
}

fn assert_font_cell_layout<C: LineWrapContext>(context: &C)
where
    C::Error: Debug,
{
    for (descriptor, expected) in [("24pt 'Metric Test'", 24.0), ("12pt 'Metric Test'", 12.0)] {
        for text in ["i", "あ"] {
            let actual = context
                .measure_upright_vertical_advance(text, descriptor)
                .unwrap()
                .unwrap();
            assert!(
                (actual - expected).abs() < 1e-5,
                "{text}: {actual} vs {expected}"
            );
        }
        for text in ["", "ii", "a\u{20dd}", "\u{10ffff}"] {
            assert_eq!(
                context
                    .measure_upright_vertical_advance(text, descriptor)
                    .unwrap(),
                None
            );
        }
    }
    for text in ["iiii", "あいうえ"] {
        let runs = [RunStyle {
            start: 0,
            end: text.len(),
            inline_object: None,
            hyperlink: None,
            revision: None,
            properties: Some(
                RunProperties {
                    font_family: Some("Metric Test".into()),
                    font_size: Some(24.0),
                    text_scale: Some(50.0),
                    ..Default::default()
                }
                .into(),
            ),
        }];
        let layout = layout_cell_text_vertically(
            context,
            text,
            &runs,
            7,
            Some(24.0),
            40.0,
            0.0,
            &ParagraphStyle::default(),
        )
        .unwrap();
        assert_eq!(layout.layout.column_ranges, vec![0..2, 2..4]);
        assert_eq!(layout.layout.lines[0].clusters[0].start, 7);
        assert_eq!(layout.layout.lines[3].clusters[0].end, 7 + text.len());
        for line in &layout.layout.lines {
            assert!((line.clusters[0].width - 12.0).abs() < 1e-5);
        }
    }
}

#[test]
fn fontdue_vertical_metrics_reach_cell_layout_through_wrap_backend() {
    let mut measurer = FontdueMeasurer::new(12.0);
    measurer
        .register_font_bytes(
            "Metric Test",
            400,
            RegisteredFontStyle::Normal,
            &test_font(true),
        )
        .unwrap();
    let context = WrapBackend::Host(FontdueLineWrapContext::new(measurer, None));
    assert_font_cell_layout(&context);
}

#[test]
fn fontdue_missing_vertical_metrics_preserve_em_fallback() {
    let mut measurer = FontdueMeasurer::new(12.0);
    assert_eq!(
        measurer
            .measure_upright_vertical_advance("i", "24pt Test")
            .unwrap(),
        None
    );
    measurer
        .register_font_bytes(
            "Metric Test",
            400,
            RegisteredFontStyle::Normal,
            &test_font(false),
        )
        .unwrap();
    assert_eq!(
        measurer
            .measure_upright_vertical_advance("i", "24pt 'Metric Test'")
            .unwrap(),
        None
    );
    let context = FontdueLineWrapContext::new(measurer, None);
    let text = "iiii";
    let runs = [RunStyle {
        start: 0,
        end: text.len(),
        inline_object: None,
        hyperlink: None,
        revision: None,
        properties: Some(
            RunProperties {
                font_family: Some("Metric Test".into()),
                font_size: Some(24.0),
                ..Default::default()
            }
            .into(),
        ),
    }];
    let layout = layout_cell_text_vertically(
        &context,
        text,
        &runs,
        0,
        Some(64.0),
        40.0,
        0.0,
        &ParagraphStyle::default(),
    )
    .unwrap();
    assert_eq!(layout.layout.column_ranges, vec![0..2, 2..4]);
    assert_eq!(layout.layout.lines[0].clusters[0].width, 32.0);
}

#[cfg(feature = "native-shaping")]
#[test]
fn shaping_vertical_metrics_reach_cell_layout_through_wrap_backend() {
    use jade_docx_layout::shaping_measurer::{
        ShapingLineWrapContext, ShapingMeasurer, ShapingOptions,
    };
    let mut measurer = ShapingMeasurer::with_options(12.0, ShapingOptions::PLAIN_OPENTYPE);
    measurer
        .register_font_bytes(
            "Metric Test",
            400,
            RegisteredFontStyle::Normal,
            &test_font(true),
        )
        .unwrap();
    let context: WrapBackend<FontdueLineWrapContext> =
        WrapBackend::Shaping(Box::new(ShapingLineWrapContext::new(measurer, None)));
    assert_font_cell_layout(&context);
}

#[cfg(feature = "native-shaping")]
#[test]
fn shaping_vertical_advance_uses_character_fallback_and_records_coverage_gaps() {
    use jade_docx_layout::shaping_measurer::ShapingMeasurer;
    let mut measurer = ShapingMeasurer::new(12.0);
    let thai = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fonts/Noto_Sans_Thai/NotoSansThai-VariableFont_wdth,wght.ttf"),
    )
    .unwrap();
    measurer
        .register_font_bytes("Thai", 400, RegisteredFontStyle::Normal, &thai)
        .unwrap();
    measurer
        .register_font_bytes(
            "Metric Test",
            400,
            RegisteredFontStyle::Normal,
            &test_font(true),
        )
        .unwrap();
    assert_eq!(
        measurer.measure_upright_vertical_advance("あ", "24pt Thai"),
        Some(24.0)
    );
    assert_eq!(
        measurer.measure_upright_vertical_advance("ก", "24pt Thai"),
        None
    );
    assert_eq!(
        measurer.measure_upright_vertical_advance("\u{10ffff}", "24pt Thai"),
        None
    );
    assert!(measurer.uncovered_characters().contains(&'\u{10ffff}'));
}

#[test]
fn substituted_vertical_metrics_control_column_breaks_and_fallback() {
    let mut bytes = test_font(true);
    let face = rustybuzz::Face::from_slice(&bytes, 0).unwrap();
    let original = face.glyph_index('（').unwrap();
    let alternate = face.glyph_index('︵').unwrap();
    assert_ne!(original, alternate);
    let em = face.units_per_em() as u16;
    let vmtx = table_offset(&bytes, b"vmtx");
    // Keep the original glyph at an em; only its vertical alternate is 3/4 em.
    for (glyph, advance) in [(original, em), (alternate, em * 3 / 4)] {
        let offset = vmtx + usize::from(glyph.0) * 4;
        bytes[offset..offset + 2].copy_from_slice(&advance.to_be_bytes());
    }
    let mut measurer = FontdueMeasurer::new(12.0);
    let thai = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fonts/Noto_Sans_Thai/NotoSansThai-VariableFont_wdth,wght.ttf"),
    )
    .unwrap();
    measurer
        .register_font_bytes("Thai", 400, RegisteredFontStyle::Normal, &thai)
        .unwrap();
    measurer
        .register_font_bytes("Vertical", 400, RegisteredFontStyle::Normal, &bytes)
        .unwrap();
    let context = WrapBackend::Host(FontdueLineWrapContext::new(measurer, None));
    assert_substituted_cell_layout(&context);
    #[cfg(feature = "native-shaping")]
    {
        use jade_docx_layout::shaping_measurer::{
            ShapingLineWrapContext, ShapingMeasurer, ShapingOptions,
        };
        let mut measurer = ShapingMeasurer::with_options(12.0, ShapingOptions::PLAIN_OPENTYPE);
        measurer
            .register_font_bytes("Thai", 400, RegisteredFontStyle::Normal, &thai)
            .unwrap();
        measurer
            .register_font_bytes("Vertical", 400, RegisteredFontStyle::Normal, &bytes)
            .unwrap();
        assert_substituted_cell_layout(&ShapingLineWrapContext::new(measurer, None));
    }
}

fn assert_substituted_cell_layout<C: LineWrapContext>(context: &C)
where
    C::Error: Debug,
{
    for family in ["Vertical", "Thai"] {
        for (size, advance) in [(24.0, 24.0), (12.0, 12.0), (24.0, 24.0)] {
            let descriptor = format!("{size}pt '{family}'");
            assert_eq!(
                context
                    .measure_upright_vertical_advance("（", &descriptor)
                    .unwrap(),
                Some(advance)
            );
            let text = "（（（（";
            let runs = [RunStyle {
                start: 0,
                end: text.len(),
                properties: Some(
                    RunProperties {
                        font_family: Some(family.into()),
                        font_size: Some(size),
                        ..Default::default()
                    }
                    .into(),
                ),
                revision: None,
                hyperlink: None,
                inline_object: None,
            }];
            let layout = layout_cell_text_vertically(
                context,
                text,
                &runs,
                7,
                Some(advance * 2.0),
                40.0,
                0.0,
                &ParagraphStyle::default(),
            )
            .unwrap();
            assert_eq!(
                layout.layout.column_ranges,
                vec![0..2, 2..4],
                "{descriptor}"
            );
            for (index, line) in layout.layout.lines.iter().enumerate() {
                assert_eq!(line.clusters[0].width, advance);
                assert_eq!(line.clusters[0].start, 7 + index * '（'.len_utf8());
                assert_eq!(line.clusters[0].end, 7 + (index + 1) * '（'.len_utf8());
            }
        }
    }
}

#[test]
fn vertical_composed_graphemes_keep_one_cell_and_source_range() {
    let bytes = test_font(true);
    let mut measurer = FontdueMeasurer::new(12.0);
    measurer
        .register_font_bytes("Grapheme", 400, RegisteredFontStyle::Normal, &bytes)
        .unwrap();
    assert_composed_cell_layout(&FontdueLineWrapContext::new(measurer, None));
    #[cfg(feature = "native-shaping")]
    {
        use jade_docx_layout::shaping_measurer::{
            ShapingLineWrapContext, ShapingMeasurer, ShapingOptions,
        };
        let mut measurer = ShapingMeasurer::with_options(12.0, ShapingOptions::PLAIN_OPENTYPE);
        measurer
            .register_font_bytes("Grapheme", 400, RegisteredFontStyle::Normal, &bytes)
            .unwrap();
        assert_composed_cell_layout(&ShapingLineWrapContext::new(measurer, None));
    }
}

fn assert_composed_cell_layout<C: LineWrapContext>(context: &C)
where
    C::Error: Debug,
{
    for (cluster, composed) in [("か\u{3099}", "が"), ("a\u{301}", "á")] {
        for (size, advance) in [(24.0, 24.0), (12.0, 12.0), (24.0, 24.0)] {
            let descriptor = format!("{size}pt Grapheme");
            assert_eq!(
                context
                    .measure_upright_vertical_advance(cluster, &descriptor)
                    .unwrap(),
                Some(advance)
            );
            assert_eq!(
                context
                    .measure_upright_vertical_advance(composed, &descriptor)
                    .unwrap(),
                Some(advance)
            );
            let text = cluster.repeat(4);
            let runs = [RunStyle {
                start: 0,
                end: text.len(),
                inline_object: None,
                hyperlink: None,
                revision: None,
                properties: Some(
                    RunProperties {
                        font_family: Some("Grapheme".into()),
                        font_size: Some(size),
                        ..Default::default()
                    }
                    .into(),
                ),
            }];
            let layout = layout_cell_text_vertically(
                context,
                &text,
                &runs,
                7,
                Some(advance * 2.0),
                40.0,
                0.0,
                &ParagraphStyle::default(),
            )
            .unwrap();
            assert_eq!(layout.layout.column_ranges, vec![0..2, 2..4]);
            for (index, line) in layout.layout.lines.iter().enumerate() {
                assert_eq!(line.clusters.len(), 1);
                assert_eq!(line.clusters[0].width, advance);
                assert_eq!(line.clusters[0].start, 7 + index * cluster.len());
                assert_eq!(line.clusters[0].end, 7 + (index + 1) * cluster.len());
            }
        }
    }
}
