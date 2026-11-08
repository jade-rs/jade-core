//! Word's フォント dialog applied as `set_run_format`: what it writes, that
//! the written package reads back to the same formatting, and that undo
//! takes it all back.

use std::io::Read;
use std::path::PathBuf;

use jade_docx_editor::commands::set_run_format::{run_format_of, SetRunFormatCommand};
use jade_docx_editor::commands::{CommandContext, CommandExecutor};
use jade_docx_editor::headless_api::apply_headless_commands_to_document;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::{
    HeadlessEastAsianLayout, HeadlessEditRequest, HeadlessRunFormat, HeadlessThemeColor,
};
use jade_docx_layout::selection::{SelectionRange, TextPosition};
use serde_json::json;

fn load_bytes(bytes: &[u8]) -> WordDocument {
    futures::executor::block_on(WordDocument::load(bytes, &Default::default())).expect("load")
}

fn load(fixture: &str) -> WordDocument {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures")
        .join(fixture);
    load_bytes(&std::fs::read(path).expect("fixture"))
}

fn document_xml(document: &WordDocument) -> String {
    let exported = document.export_docx().expect("export");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .expect("document part")
        .read_to_string(&mut xml)
        .expect("read");
    xml
}

/// The first body paragraph with at least `chars` characters of text.
fn paragraph_with_text(document: &WordDocument, chars: usize) -> usize {
    (0..document.paragraph_count())
        .find(|&index| {
            document.is_body_paragraph_surface(index)
                && document.paragraph(index).is_some_and(|paragraph| {
                    paragraph
                        .runs
                        .iter()
                        .map(|run| run.text.chars().count())
                        .sum::<usize>()
                        >= chars
                })
        })
        .expect("a paragraph with text")
}

fn format_at(document: &WordDocument, paragraph: usize, offset: usize) -> HeadlessRunFormat {
    let paragraph = document.paragraph(paragraph).expect("paragraph");
    let mut start = 0;
    for run in &paragraph.runs {
        let end = start + run.text.len();
        if offset < end && !run.text.is_empty() {
            return run
                .properties
                .as_ref()
                .map(run_format_of)
                .unwrap_or_default();
        }
        start = end;
    }
    HeadlessRunFormat::default()
}

fn full_format() -> HeadlessRunFormat {
    HeadlessRunFormat {
        latin_font: Some("Arial".into()),
        east_asia_font: Some("游明朝".into()),
        font_size: Some(14.0),
        bold: Some(true),
        underline: Some("double".into()),
        underline_color: Some("FF0000".into()),
        double_strike: Some(true),
        color: Some("4472C4".into()),
        theme_color: Some(HeadlessThemeColor {
            name: "accent1".into(),
            tint: None,
            shade: None,
        }),
        small_caps: Some(true),
        hidden: Some(true),
        emphasis: Some("dot".into()),
        shading: Some("FFFF00".into()),
        border: Some(true),
        scale: Some(150.0),
        spacing: Some(2.0),
        position: Some(3.0),
        kerning: Some(10.5),
        ligatures: Some("standard".into()),
        glow_color: Some("FF0000".into()),
        glow_radius: Some(5.0),
        text_outline_color: Some("0070C0".into()),
        east_asian_layout: Some(HeadlessEastAsianLayout {
            vert: true,
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[test]
fn the_font_dialog_writes_every_property_and_reads_back() {
    let mut document = load("hello.docx");
    let paragraph = paragraph_with_text(&document, 4);
    let request: HeadlessEditRequest = serde_json::from_value(json!({
        "commands": [{
            "type": "set_run_format",
            "start_paragraph": paragraph, "start_offset": 0,
            "end_paragraph": paragraph, "end_offset": 3,
            "format": serde_json::to_value(full_format()).unwrap(),
        }]
    }))
    .expect("request");
    apply_headless_commands_to_document(&mut document, &request).expect("apply");

    let xml = document_xml(&document);
    for expected in [
        r#"w:ascii="Arial""#,
        r#"w:eastAsia="游明朝""#,
        "<w:b/>",
        r#"<w:u w:val="double" w:color="FF0000"/>"#,
        "<w:dstrike/>",
        r#"w:themeColor="accent1""#,
        "<w:smallCaps/>",
        "<w:vanish/>",
        r#"<w:em w:val="dot"/>"#,
        r#"w:fill="FFFF00""#,
        r#"<w:bdr w:val="single""#,
        r#"<w:w w:val="150"/>"#,
        r#"<w:spacing w:val="40"/>"#,
        r#"<w:position w:val="6"/>"#,
        r#"<w:kern w:val="21"/>"#,
        r#"<w14:ligatures w14:val="standard"/>"#,
        r#"w:vert="1""#,
    ] {
        assert!(xml.contains(expected), "missing {expected} in {xml}");
    }

    // What was written reads back as what was set.
    let reread = load_bytes(&document.export_docx().unwrap());
    let format = format_at(&reread, paragraph, 0);
    assert_eq!(format.latin_font.as_deref(), Some("Arial"));
    assert_eq!(format.east_asia_font.as_deref(), Some("游明朝"));
    assert_eq!(format.underline.as_deref(), Some("double"));
    assert_eq!(format.double_strike, Some(true));
    assert_eq!(format.small_caps, Some(true));
    assert_eq!(format.hidden, Some(true));
    assert_eq!(format.emphasis.as_deref(), Some("dot"));
    assert_eq!(format.shading.as_deref(), Some("FFFF00"));
    assert_eq!(format.border, Some(true));
    assert_eq!(format.scale, Some(150.0));
    assert_eq!(format.spacing, Some(2.0));
    assert_eq!(format.position, Some(3.0));
    // A theme colour is shown as the document's theme has it, as Word
    // does, not as the cached `w:val` the dialog also wrote.
    assert_eq!(format.color.as_deref(), Some("00A2FF"));
    assert_eq!(
        format.theme_color.map(|theme| theme.name).as_deref(),
        Some("accent1")
    );
    assert!(format.east_asian_layout.is_some_and(|layout| layout.vert));
    assert_eq!(format.ligatures.as_deref(), Some("standard"));
    assert_eq!(format.glow_color.as_deref(), Some("FF0000"));
    assert_eq!(format.glow_radius, Some(5.0));
    assert_eq!(format.text_outline_color.as_deref(), Some("0070C0"));
    // The text after the range keeps its own formatting.
    assert_ne!(format_at(&reread, paragraph, 3).double_strike, Some(true));
}

#[test]
fn taking_properties_off_removes_them_and_undo_restores() {
    let mut document = load("hello.docx");
    let paragraph = paragraph_with_text(&document, 4);
    let before = document_xml(&document);
    let range = SelectionRange::new(
        TextPosition::new(paragraph, 0),
        TextPosition::new(paragraph, 3),
    );
    let mut selection = Some(range);
    let mut executor = CommandExecutor::new();
    {
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor
            .execute(
                Box::new(SetRunFormatCommand::new(range, full_format())),
                &mut ctx,
            )
            .unwrap();
        let off = HeadlessRunFormat {
            underline: Some("none".into()),
            double_strike: Some(false),
            emphasis: Some("none".into()),
            shading: Some("none".into()),
            border: Some(false),
            spacing: Some(0.0),
            position: Some(0.0),
            east_asian_layout: Some(HeadlessEastAsianLayout::default()),
            ligatures: Some("none".into()),
            ..Default::default()
        };
        executor
            .execute(Box::new(SetRunFormatCommand::new(range, off)), &mut ctx)
            .unwrap();
    }
    let xml = document_xml(&document);
    assert!(xml.contains(r#"<w:position w:val="0"/>"#));
    assert!(xml.contains(r#"<w:spacing w:val="0"/>"#));
    let reread = load_bytes(&document.export_docx().unwrap());
    assert_eq!(format_at(&reread, paragraph, 0).position, Some(0.0));
    assert_eq!(format_at(&reread, paragraph, 0).spacing, Some(0.0));
    assert_eq!(
        format_at(&reread, paragraph, 0).shading.as_deref(),
        Some("none")
    );
    assert!(xml.contains(r#"<w:shd w:val="nil"/>"#));
    assert!(xml.contains(r#"<w:bdr w:val="nil"/>"#));
    assert!(xml.contains(r#"<w:em w:val="none"/>"#));
    assert_eq!(
        format_at(&reread, paragraph, 0).emphasis.as_deref(),
        Some("none")
    );
    assert_eq!(format_at(&reread, paragraph, 0).border, Some(false));
    assert!(xml.contains(r#"w:combine="0""#));
    assert!(xml.contains(r#"w:vert="0""#));
    assert!(xml.contains(r#"<w14:ligatures w14:val="none"/>"#));
    assert_eq!(
        format_at(&reread, paragraph, 0).ligatures.as_deref(),
        Some("none")
    );
    assert!(!xml.contains("<w:dstrike/>"));
    {
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor.undo(&mut ctx).unwrap();
        executor.undo(&mut ctx).unwrap();
    }
    let format = format_at(&document, paragraph, 0);
    assert_ne!(format.double_strike, Some(true));
    assert_eq!(format.emphasis, None);
    assert!(document_xml(&document).len() <= before.len() + 64);
}

#[test]
fn zero_position_resets_inherited_shift_and_undo_restores_inheritance() {
    let mut document = load("hello.docx");
    document.styles_part = Some(jade_docx_reader::StylesPart::from_xml(
        r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:position w:val="12"/></w:rPr></w:rPrDefault></w:docDefaults></w:styles>"#,
    ).unwrap());
    let paragraph = paragraph_with_text(&document, 4);
    let effective_position = |document: &WordDocument| {
        let p = document.paragraph(paragraph).unwrap();
        document
            .styles_part
            .as_ref()
            .unwrap()
            .resolve_effective_run_properties(
                p.properties
                    .as_ref()
                    .and_then(|properties| properties.style.as_deref()),
                p.runs[0].properties.as_ref(),
            )
            .unwrap()
            .text_position
    };
    assert_eq!(effective_position(&document), Some(6.0));
    let range = SelectionRange::new(
        TextPosition::new(paragraph, 0),
        TextPosition::new(paragraph, 3),
    );
    let mut selection = Some(range);
    let mut executor = CommandExecutor::new();
    executor
        .execute(
            Box::new(SetRunFormatCommand::new(
                range,
                HeadlessRunFormat {
                    position: Some(0.0),
                    ..Default::default()
                },
            )),
            &mut CommandContext::new(&mut document, &mut selection),
        )
        .unwrap();
    assert_eq!(effective_position(&document), Some(0.0));
    assert!(document_xml(&document).contains(r#"<w:position w:val="0"/>"#));
    let reread = load_bytes(&document.export_docx().unwrap());
    assert_eq!(format_at(&reread, paragraph, 0).position, Some(0.0));
    executor
        .undo(&mut CommandContext::new(&mut document, &mut selection))
        .unwrap();
    assert_eq!(effective_position(&document), Some(6.0));
}

#[test]
fn zero_spacing_resets_inherited_spacing_and_undo_restores_inheritance() {
    let mut document = load("hello.docx");
    document.styles_part = Some(jade_docx_reader::StylesPart::from_xml(
        r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:spacing w:val="120"/></w:rPr></w:rPrDefault></w:docDefaults></w:styles>"#,
    ).unwrap());
    let paragraph = paragraph_with_text(&document, 4);
    let effective_spacing = |document: &WordDocument| {
        let p = document.paragraph(paragraph).unwrap();
        document
            .styles_part
            .as_ref()
            .unwrap()
            .resolve_effective_run_properties(
                p.properties
                    .as_ref()
                    .and_then(|properties| properties.style.as_deref()),
                p.runs[0].properties.as_ref(),
            )
            .unwrap()
            .char_spacing
    };
    assert_eq!(effective_spacing(&document), Some(6.0));
    let range = SelectionRange::new(
        TextPosition::new(paragraph, 0),
        TextPosition::new(paragraph, 3),
    );
    let mut selection = Some(range);
    let mut executor = CommandExecutor::new();
    executor
        .execute(
            Box::new(SetRunFormatCommand::new(
                range,
                HeadlessRunFormat {
                    spacing: Some(0.0),
                    ..Default::default()
                },
            )),
            &mut CommandContext::new(&mut document, &mut selection),
        )
        .unwrap();
    assert_eq!(effective_spacing(&document), Some(0.0));
    assert!(document_xml(&document).contains(r#"<w:spacing w:val="0"/>"#));
    let reread = load_bytes(&document.export_docx().unwrap());
    assert_eq!(format_at(&reread, paragraph, 0).spacing, Some(0.0));
    executor
        .undo(&mut CommandContext::new(&mut document, &mut selection))
        .unwrap();
    assert_eq!(effective_spacing(&document), Some(6.0));
}

#[test]
fn no_shading_cancels_inherited_shading_and_undo_restores_it() {
    let mut document = load("hello.docx");
    document.styles_part = Some(jade_docx_reader::StylesPart::from_xml(
        r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:shd w:val="clear" w:fill="FFFF00"/></w:rPr></w:rPrDefault></w:docDefaults></w:styles>"#,
    ).unwrap());
    let paragraph = paragraph_with_text(&document, 4);
    let effective_shading = |document: &WordDocument| {
        let p = document.paragraph(paragraph).unwrap();
        document
            .styles_part
            .as_ref()
            .unwrap()
            .resolve_effective_run_properties(
                p.properties
                    .as_ref()
                    .and_then(|properties| properties.style.as_deref()),
                p.runs[0].properties.as_ref(),
            )
            .unwrap()
            .shading
            .and_then(|shading| shading.resolved_background_color())
    };
    assert_eq!(effective_shading(&document), Some("#FFFF00".into()));
    let range = SelectionRange::new(
        TextPosition::new(paragraph, 0),
        TextPosition::new(paragraph, 3),
    );
    let mut selection = Some(range);
    let mut executor = CommandExecutor::new();
    executor
        .execute(
            Box::new(SetRunFormatCommand::new(
                range,
                HeadlessRunFormat {
                    shading: Some("none".into()),
                    ..Default::default()
                },
            )),
            &mut CommandContext::new(&mut document, &mut selection),
        )
        .unwrap();
    assert_eq!(effective_shading(&document), None);
    assert!(document_xml(&document).contains(r#"<w:shd w:val="nil"/>"#));
    let reread = load_bytes(&document.export_docx().unwrap());
    assert_eq!(
        format_at(&reread, paragraph, 0).shading.as_deref(),
        Some("none")
    );
    executor
        .undo(&mut CommandContext::new(&mut document, &mut selection))
        .unwrap();
    assert_eq!(effective_shading(&document), Some("#FFFF00".into()));
}

#[test]
fn no_border_cancels_inherited_border_and_undo_restores_it() {
    let mut document = load("hello.docx");
    document.styles_part = Some(jade_docx_reader::StylesPart::from_xml(
        r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:bdr w:val="single" w:sz="12" w:color="FF0000"/></w:rPr></w:rPrDefault></w:docDefaults></w:styles>"#,
    ).unwrap());
    let paragraph = paragraph_with_text(&document, 4);
    let effective_border = |document: &WordDocument| {
        let p = document.paragraph(paragraph).unwrap();
        document
            .styles_part
            .as_ref()
            .unwrap()
            .resolve_effective_run_properties(
                p.properties
                    .as_ref()
                    .and_then(|properties| properties.style.as_deref()),
                p.runs[0].properties.as_ref(),
            )
            .unwrap()
            .border
            .map(|border| border.is_visible())
    };
    assert_eq!(effective_border(&document), Some(true));
    let range = SelectionRange::new(
        TextPosition::new(paragraph, 0),
        TextPosition::new(paragraph, 3),
    );
    let mut selection = Some(range);
    let mut executor = CommandExecutor::new();
    executor
        .execute(
            Box::new(SetRunFormatCommand::new(
                range,
                HeadlessRunFormat {
                    border: Some(false),
                    ..Default::default()
                },
            )),
            &mut CommandContext::new(&mut document, &mut selection),
        )
        .unwrap();
    assert_eq!(effective_border(&document), Some(false));
    assert!(document_xml(&document).contains(r#"<w:bdr w:val="nil"/>"#));
    let reread = load_bytes(&document.export_docx().unwrap());
    assert_eq!(format_at(&reread, paragraph, 0).border, Some(false));
    executor
        .undo(&mut CommandContext::new(&mut document, &mut selection))
        .unwrap();
    assert_eq!(effective_border(&document), Some(true));
}

#[test]
fn no_emphasis_cancels_inherited_emphasis_and_undo_restores_it() {
    let mut document = load("hello.docx");
    document.styles_part = Some(jade_docx_reader::StylesPart::from_xml(
        r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:em w:val="dot"/></w:rPr></w:rPrDefault></w:docDefaults></w:styles>"#,
    ).unwrap());
    let paragraph = paragraph_with_text(&document, 4);
    let effective_emphasis = |document: &WordDocument| {
        let p = document.paragraph(paragraph).unwrap();
        document
            .styles_part
            .as_ref()
            .unwrap()
            .resolve_effective_run_properties(
                p.properties
                    .as_ref()
                    .and_then(|properties| properties.style.as_deref()),
                p.runs[0].properties.as_ref(),
            )
            .unwrap()
            .emphasis
            .map(|mark| mark.to_string())
    };
    assert_eq!(effective_emphasis(&document), Some("dot".into()));
    let range = SelectionRange::new(
        TextPosition::new(paragraph, 0),
        TextPosition::new(paragraph, 3),
    );
    let mut selection = Some(range);
    let mut executor = CommandExecutor::new();
    executor
        .execute(
            Box::new(SetRunFormatCommand::new(
                range,
                HeadlessRunFormat {
                    emphasis: Some("none".into()),
                    ..Default::default()
                },
            )),
            &mut CommandContext::new(&mut document, &mut selection),
        )
        .unwrap();
    assert_eq!(effective_emphasis(&document), Some("none".into()));
    assert!(document_xml(&document).contains(r#"<w:em w:val="none"/>"#));
    let reread = load_bytes(&document.export_docx().unwrap());
    assert_eq!(
        format_at(&reread, paragraph, 0).emphasis,
        Some("none".into())
    );
    executor
        .undo(&mut CommandContext::new(&mut document, &mut selection))
        .unwrap();
    assert_eq!(effective_emphasis(&document), Some("dot".into()));
}

#[test]
fn multiple_stylistic_sets_round_trip_and_reset_with_undo() {
    let mut document = load("hello.docx");
    let paragraph = paragraph_with_text(&document, 4);
    let range = SelectionRange::new(
        TextPosition::new(paragraph, 0),
        TextPosition::new(paragraph, 3),
    );
    let mut selection = Some(range);
    let mut executor = CommandExecutor::new();
    for sets in [vec![20, 1, 4, 4], vec![]] {
        executor
            .execute(
                Box::new(SetRunFormatCommand::new(
                    range,
                    HeadlessRunFormat {
                        stylistic_sets: Some(sets.clone()),
                        ..Default::default()
                    },
                )),
                &mut CommandContext::new(&mut document, &mut selection),
            )
            .unwrap();
        let mut expected = sets;
        expected.sort_unstable();
        expected.dedup();
        let reread = load_bytes(&document.export_docx().unwrap());
        assert_eq!(
            format_at(&reread, paragraph, 0).stylistic_sets,
            Some(expected)
        );
    }
    assert!(document_xml(&document).contains("<w14:stylisticSets/>"));
    executor
        .undo(&mut CommandContext::new(&mut document, &mut selection))
        .unwrap();
    assert_eq!(
        format_at(&document, paragraph, 0).stylistic_sets,
        Some(vec![1, 4, 20])
    );
    let reread = load_bytes(&document.export_docx().unwrap());
    assert_eq!(
        format_at(&reread, paragraph, 0).stylistic_sets,
        Some(vec![1, 4, 20])
    );
}

#[test]
fn visual_effects_keep_opacity_and_explicitly_reset_inheritance() {
    let mut document = load("hello.docx");
    document.styles_part = Some(jade_docx_reader::StylesPart::from_xml(
        r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:docDefaults><w:rPrDefault><w:rPr><w14:glow w14:rad="63500"><w14:srgbClr w14:val="FF0000"><w14:alpha w14:val="40000"/></w14:srgbClr></w14:glow><w14:textOutline w14:w="12700"><w14:solidFill><w14:srgbClr w14:val="00FF00"/></w14:solidFill></w14:textOutline></w:rPr></w:rPrDefault></w:docDefaults></w:styles>"#,
    ).unwrap());
    let paragraph = paragraph_with_text(&document, 4);
    let effective = |document: &WordDocument| {
        let p = document.paragraph(paragraph).unwrap();
        document
            .styles_part
            .as_ref()
            .unwrap()
            .resolve_effective_run_properties(
                p.properties.as_ref().and_then(|p| p.style.as_deref()),
                p.runs[0].properties.as_ref(),
            )
            .unwrap()
            .text_effects
            .unwrap()
    };
    assert_eq!(effective(&document).glow_alpha, Some(0.4));
    let range = SelectionRange::new(
        TextPosition::new(paragraph, 0),
        TextPosition::new(paragraph, 3),
    );
    let mut selection = Some(range);
    let mut executor = CommandExecutor::new();
    executor
        .execute(
            Box::new(SetRunFormatCommand::new(
                range,
                HeadlessRunFormat {
                    glow_color: Some("none".into()),
                    text_outline_color: Some("none".into()),
                    ..Default::default()
                },
            )),
            &mut CommandContext::new(&mut document, &mut selection),
        )
        .unwrap();
    assert_eq!(effective(&document).glow_radius, Some(0.0));
    assert_eq!(effective(&document).outline_width, Some(0.0));
    let reread = load_bytes(&document.export_docx().unwrap());
    let reset = reread.paragraph(paragraph).unwrap().runs[0]
        .properties
        .as_ref()
        .unwrap()
        .text_effects
        .as_ref()
        .unwrap();
    assert_eq!(reset.glow_radius, Some(0.0));
    assert_eq!(reset.outline_width, Some(0.0));
    executor
        .undo(&mut CommandContext::new(&mut document, &mut selection))
        .unwrap();
    assert_eq!(effective(&document).glow_radius, Some(5.0));
    assert_eq!(effective(&document).outline_width, Some(1.0));

    // A directly imported opacity must survive a radius-only edit.
    let props = document.paragraph_mut(paragraph).unwrap().runs[0]
        .properties
        .get_or_insert_with(Default::default);
    props.text_effects = Some(Box::new(jade_docx_reader::document::RunTextEffects {
        glow_color: Some("#FF0000".into()),
        glow_radius: Some(5.0),
        glow_alpha: Some(0.4),
        ..Default::default()
    }));
    executor
        .execute(
            Box::new(SetRunFormatCommand::new(
                range,
                HeadlessRunFormat {
                    glow_radius: Some(7.0),
                    ..Default::default()
                },
            )),
            &mut CommandContext::new(&mut document, &mut selection),
        )
        .unwrap();
    let reread = load_bytes(&document.export_docx().unwrap());
    let effects = reread.paragraph(paragraph).unwrap().runs[0]
        .properties
        .as_ref()
        .unwrap()
        .text_effects
        .as_ref()
        .unwrap();
    assert_eq!(effects.glow_radius, Some(7.0));
    assert_eq!(effects.glow_alpha, Some(0.4));
    assert!(document_xml(&document).contains(r#"w14:val="40000""#));
}

#[test]
fn partial_visual_effect_edits_preserve_inherited_settings() {
    for patch in [
        HeadlessRunFormat {
            glow_radius: Some(7.0),
            ..Default::default()
        },
        HeadlessRunFormat {
            glow_color: Some("0000FF".into()),
            ..Default::default()
        },
        HeadlessRunFormat {
            text_outline_width: Some(2.0),
            ..Default::default()
        },
        HeadlessRunFormat {
            text_outline_color: Some("0000FF".into()),
            ..Default::default()
        },
    ] {
        let mut document = load("hello.docx");
        document.styles_part = Some(jade_docx_reader::StylesPart::from_xml(
            r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:docDefaults><w:rPrDefault><w:rPr><w14:glow w14:rad="63500"><w14:srgbClr w14:val="FF0000"><w14:alpha w14:val="40000"/></w14:srgbClr></w14:glow><w14:textOutline w14:w="12700"><w14:solidFill><w14:srgbClr w14:val="00FF00"/></w14:solidFill></w14:textOutline></w:rPr></w:rPrDefault></w:docDefaults></w:styles>"#,
        ).unwrap());
        let paragraph = paragraph_with_text(&document, 4);
        let before: Vec<_> = document
            .paragraph(paragraph)
            .unwrap()
            .runs
            .iter()
            .map(|run| (run.text.clone(), run.properties.clone()))
            .collect();
        let range = SelectionRange::new(
            TextPosition::new(paragraph, 1),
            TextPosition::new(paragraph, 3),
        );
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        executor
            .execute(
                Box::new(SetRunFormatCommand::new(range, patch.clone())),
                &mut CommandContext::new(&mut document, &mut selection),
            )
            .unwrap();
        let reread = load_bytes(&document.export_docx().unwrap());
        let format = format_at(&reread, paragraph, 1);
        if patch.glow_radius.is_some() || patch.glow_color.is_some() {
            assert_eq!(format.glow_radius, Some(patch.glow_radius.unwrap_or(5.0)));
            assert_eq!(
                format.glow_color,
                Some(patch.glow_color.clone().unwrap_or("FF0000".into()))
            );
            let p = reread.paragraph(paragraph).unwrap();
            let effects = p
                .runs
                .iter()
                .find(|run| {
                    run.properties.as_ref().is_some_and(|props| {
                        props
                            .text_effects
                            .as_ref()
                            .is_some_and(|effects| effects.glow_alpha.is_some())
                    })
                })
                .unwrap()
                .properties
                .as_ref()
                .unwrap()
                .text_effects
                .as_ref()
                .unwrap();
            assert_eq!(effects.glow_alpha, Some(0.4));
            assert!(
                effects.outline_width.is_none(),
                "unmodified outline remains inherited"
            );
        } else {
            assert_eq!(
                format.text_outline_width,
                Some(patch.text_outline_width.unwrap_or(1.0))
            );
            assert_eq!(
                format.text_outline_color,
                Some(patch.text_outline_color.clone().unwrap_or("00FF00".into()))
            );
            assert!(
                format.glow_radius.is_none(),
                "unmodified glow remains inherited"
            );
        }
        assert!(format_at(&reread, paragraph, 0).glow_radius.is_none());
        assert!(format_at(&reread, paragraph, 3)
            .text_outline_width
            .is_none());
        executor
            .undo(&mut CommandContext::new(&mut document, &mut selection))
            .unwrap();
        assert_eq!(
            document
                .paragraph(paragraph)
                .unwrap()
                .runs
                .iter()
                .map(|run| (run.text.clone(), run.properties.clone()))
                .collect::<Vec<_>>(),
            before
        );
    }
}
