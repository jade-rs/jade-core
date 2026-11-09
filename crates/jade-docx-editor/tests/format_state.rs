//! What the フォント, 段落 and 文字体裁 dialogs open on: the state the
//! headless commands write is read back in the shapes they take.

mod common;

use jade_docx_editor::commands::CommandExecutor;
use jade_docx_editor::headless_api::apply_headless_commands_with_executor_to_document;
use jade_docx_editor::word_document::format_state::ParagraphPaginationState;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::{
    HeadlessCommand, HeadlessEditRequest, HeadlessJapaneseTypography, HeadlessParagraphTypography,
    HeadlessRunFormat,
};

fn apply(document: &mut WordDocument, command: HeadlessCommand) {
    let mut executor = CommandExecutor::new();
    let request = HeadlessEditRequest {
        commands: vec![command],
    };
    apply_headless_commands_with_executor_to_document(document, &mut executor, None, &request)
        .expect("apply");
}

fn document() -> WordDocument {
    common::load(&common::package(
        "<w:p><w:r><w:t>前半</w:t></w:r><w:r><w:t>後半</w:t></w:r></w:p>",
        &[],
    ))
}

#[test]
fn the_font_dialog_opens_on_the_run_at_the_offset() {
    let mut document = document();
    // The package has no styles part: the dialog opens on Word's built-in
    // Normal, 游明朝 10.5 pt with kerning and 11 pt complex-script text.
    assert_eq!(
        document.run_format_at(0, 0),
        HeadlessRunFormat {
            latin_font: Some("游明朝".into()),
            east_asia_font: Some("游明朝".into()),
            complex_font: Some("Times New Roman".into()),
            font_size: Some(10.5),
            complex_font_size: Some(11.0),
            kerning: Some(1.0),
            language: Some("en-US".into()),
            east_asia_language: Some("ja-JP".into()),
            ..Default::default()
        }
    );
    apply(
        &mut document,
        HeadlessCommand::SetRunFormat {
            start_paragraph: 0,
            start_offset: 0,
            end_paragraph: 0,
            end_offset: "前半".len(),
            format: Box::new(HeadlessRunFormat {
                underline: Some("double".into()),
                spacing: Some(1.5),
                small_caps: Some(true),
                ..Default::default()
            }),
        },
    );
    let first = document.run_format_at(0, 0);
    assert_eq!(first.underline.as_deref(), Some("double"));
    assert_eq!(first.spacing, Some(1.5));
    assert_eq!(first.small_caps, Some(true));
    // The second run was not touched; the paragraph end reads the last run.
    let end = "前半後半".len();
    assert_eq!(document.run_format_at(0, end).underline, None);
}

#[test]
fn the_paragraph_dialog_reads_pagination_and_snap_to_grid() {
    let mut document = document();
    assert_eq!(
        document.paragraph_pagination(0),
        ParagraphPaginationState {
            snap_to_grid: true,
            ..Default::default()
        }
    );
    apply(
        &mut document,
        HeadlessCommand::SetParagraphPagination {
            start_paragraph: 0,
            end_paragraph: 0,
            keep_next: Some(true),
            keep_lines: None,
            page_break_before: Some(true),
            widow_control: Some(false),
        },
    );
    apply(
        &mut document,
        HeadlessCommand::SetParagraphSnapToGrid {
            start_paragraph: 0,
            end_paragraph: 0,
            snap: false,
        },
    );
    let state = document.paragraph_pagination(0);
    assert!(state.keep_next);
    assert!(state.page_break_before);
    assert!(!state.keep_lines);
    assert!(!state.widow_control);
    assert!(!state.snap_to_grid);
}

#[test]
fn typography_reads_the_paragraph_over_the_document_over_word() {
    let mut document = document();
    let word = document.paragraph_typography(0);
    assert_eq!(word.kinsoku, Some(true));
    assert_eq!(word.top_line_punctuation, Some(false));
    apply(
        &mut document,
        HeadlessCommand::SetParagraphTypography {
            start_paragraph: 0,
            end_paragraph: 0,
            typography: HeadlessParagraphTypography {
                kinsoku: Some(false),
                ..Default::default()
            },
        },
    );
    assert_eq!(document.paragraph_typography(0).kinsoku, Some(false));
}

#[test]
fn the_japanese_typography_dialog_reads_settings() {
    // 文字体裁 lives in settings.xml, which the bare test package lacks.
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures/hello.docx");
    let mut document = common::load(&std::fs::read(path).expect("fixture"));
    assert_eq!(
        document.japanese_typography().kinsoku_level.as_deref(),
        Some("standard")
    );
    apply(
        &mut document,
        HeadlessCommand::SetJapaneseTypography {
            typography: HeadlessJapaneseTypography {
                kinsoku_level: Some("strict".into()),
                character_spacing_control: Some("compressPunctuation".into()),
                ..Default::default()
            },
        },
    );
    let typography = document.japanese_typography();
    assert_eq!(typography.kinsoku_level.as_deref(), Some("strict"));
    assert_eq!(
        typography.character_spacing_control.as_deref(),
        Some("compressPunctuation")
    );
}

#[test]
fn font_state_resolves_styles_without_materializing_them() {
    let mut document = common::load(&common::package(
        r#"<w:p><w:pPr><w:pStyle w:val="Effects"/></w:pPr><w:r><w:t>前半</w:t></w:r><w:r><w:rPr><w:rStyle w:val="Blue"/><w:b w:val="0"/><w:spacing w:val="0"/></w:rPr><w:t>後半</w:t></w:r></w:p><w:p><w:pPr><w:pStyle w:val="Effects"/></w:pPr><w:r><w:rPr><w:i/></w:rPr></w:r></w:p><w:p><w:pPr><w:pStyle w:val="Effects"/></w:pPr></w:p>"#,
        &[],
    ));
    document.styles_part = Some(jade_docx_reader::StylesPart::from_xml(
        r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:docDefaults><w:rPrDefault><w:rPr><w:caps/></w:rPr></w:rPrDefault></w:docDefaults><w:style w:type="paragraph" w:styleId="Effects"><w:rPr><w:b/><w:spacing w:val="40"/><w14:glow w14:rad="63500"><w14:srgbClr w14:val="FF0000"/></w14:glow></w:rPr></w:style><w:style w:type="character" w:styleId="Blue"><w:rPr><w14:glow w14:rad="76200"><w14:srgbClr w14:val="0000FF"/></w14:glow></w:rPr></w:style></w:styles>"#,
    ).unwrap());
    let before: Vec<_> = document
        .paragraph(0)
        .unwrap()
        .runs
        .iter()
        .map(|run| run.properties.clone())
        .collect();
    let first = document.run_format_at(0, 0);
    assert_eq!(first.caps, Some(true));
    assert_eq!(first.bold, Some(true));
    assert_eq!(first.spacing, Some(2.0));
    assert_eq!(first.glow_radius, Some(5.0));
    assert_eq!(first.glow_color.as_deref(), Some("FF0000"));
    let second = document.run_format_at(0, "前半".len());
    assert_eq!(second.bold, Some(false));
    assert_eq!(second.spacing, Some(0.0));
    assert_eq!(second.glow_radius, Some(6.0));
    assert_eq!(second.glow_color.as_deref(), Some("0000FF"));
    assert_eq!(document.run_format_at(0, usize::MAX), second);
    let dialog = jade_docx_editor::word::font::FontDialog::open(second);
    assert_eq!(dialog.change(), HeadlessRunFormat::default());
    let empty = document.run_format_at(1, 0);
    assert_eq!(empty.italic, Some(true));
    assert_eq!(empty.glow_radius, Some(5.0));
    assert_eq!(document.run_format_at(2, 0).glow_radius, Some(5.0));
    assert_eq!(document.run_format_at(99, 0), HeadlessRunFormat::default());
    assert_eq!(
        before,
        document
            .paragraph(0)
            .unwrap()
            .runs
            .iter()
            .map(|run| run.properties.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn selected_formats_respect_runs_paragraphs_and_utf8_boundaries() {
    use jade_docx_editor::word::font::FontDialog;
    let document = common::load(&common::package(
        r#"<w:p><w:r><w:rPr><w:b/><w:sz w:val="24"/></w:rPr><w:t>前半</w:t></w:r><w:r><w:rPr><w:b w:val="0"/><w:sz w:val="36"/></w:rPr><w:t>後半</w:t></w:r></w:p><w:p/><w:p><w:r><w:rPr><w:i/></w:rPr><w:t>次</w:t></w:r></w:p>"#,
        &[],
    ));
    let end = "前半後半".len();
    let formats = document.run_formats_in_range((0, 0), (0, end));
    assert_eq!(formats.len(), 2);
    let dialog = FontDialog::open_selection(formats);
    assert!(dialog.is_mixed("bold"));
    assert!(dialog.is_mixed("font_size"));
    assert!(dialog.change().is_empty());
    // A half-open boundary never picks up the following run or paragraph.
    let first = document.run_formats_in_range((0, 0), (0, "前半".len()));
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].bold, Some(true));
    assert_eq!(document.run_formats_in_range((0, 0), (1, 0)).len(), 3);
    assert_eq!(document.run_formats_in_range((0, 0), (2, 0)).len(), 4);
    assert_eq!(document.run_formats_in_range((2, 0), (0, 0)).len(), 4);
    assert_eq!(
        document.run_formats_in_range((0, end), (0, end))[0].bold,
        Some(false)
    );
}

#[test]
fn changing_one_mixed_field_preserves_other_fields_through_save_and_undo() {
    use jade_docx_editor::commands::set_run_format::SetRunFormatCommand;
    use jade_docx_editor::word::font::FontDialog;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};
    let mut session = common::Session::open(
        r#"<w:p><w:r><w:rPr><w:b/><w:sz w:val="24"/></w:rPr><w:t>前半</w:t></w:r><w:r><w:rPr><w:b w:val="0"/><w:sz w:val="36"/></w:rPr><w:t>後半</w:t></w:r></w:p>"#,
        &[],
    );
    let end = "前半後半".len();
    let before = session.xml();
    let mut dialog =
        FontDialog::open_selection(session.document.run_formats_in_range((0, 0), (0, end)));
    dialog.format.bold = Some(false);
    session.run(Box::new(SetRunFormatCommand::new(
        SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, end)),
        dialog.change(),
    )));
    let reopened = session.reopened();
    assert_eq!(reopened.run_format_at(0, 0).bold, Some(false));
    assert_eq!(reopened.run_format_at(0, "前半".len()).bold, Some(false));
    assert_eq!(reopened.run_format_at(0, 0).font_size, Some(12.0));
    assert_eq!(
        reopened.run_format_at(0, "前半".len()).font_size,
        Some(18.0)
    );
    session.undo();
    assert_eq!(session.xml(), before);
}

#[test]
fn paragraph_marks_participate_in_selection_without_leaking_into_text() {
    use jade_docx_editor::commands::set_run_format::SetRunFormatCommand;
    use jade_docx_editor::word::font::FontDialog;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};
    let mut session = common::Session::open(
        r#"<w:p><w:pPr><w:keepNext/><w:rPr><w:b/><w:sz w:val="36"/><w:spacing w:val="30"/><w:noProof/><w:lang w:val="ja-JP"/><w:ins w:id="71" w:author="Mark author"/></w:rPr></w:pPr><w:r><w:rPr><w:b w:val="0"/><w:sz w:val="24"/></w:rPr><w:t>前</w:t></w:r></w:p><w:p><w:pPr><w:rPr><w:i/><w:sz w:val="40"/></w:rPr></w:pPr></w:p><w:p><w:r><w:t>後</w:t></w:r></w:p>"#,
        &[],
    );
    let end = "前".len();
    assert_eq!(session.document.run_format_at(0, end).bold, Some(false));
    assert_eq!(session.document.run_format_at(1, 0).italic, Some(true));
    assert_eq!(session.document.run_format_at(1, 0).font_size, Some(20.0));
    let text = session.document.run_formats_in_range((0, 0), (0, end));
    assert_eq!(text.len(), 1);
    assert_eq!(text[0].font_size, Some(12.0));
    let marked = session.document.run_formats_in_range((0, 0), (1, 0));
    assert_eq!(marked.len(), 2);
    assert!(FontDialog::open_selection(marked).is_mixed("bold"));
    let mark_only = session.document.run_formats_in_range((1, 0), (0, end));
    assert_eq!(mark_only.len(), 1);
    assert_eq!(mark_only[0].font_size, Some(18.0));
    assert_eq!(mark_only[0].spacing, Some(1.5));
    assert_eq!(mark_only[0].no_proof, Some(true));
    // Select two marks, but no text in either boundary paragraph.
    session.run(Box::new(SetRunFormatCommand::new(
        SelectionRange::new(TextPosition::new(2, 0), TextPosition::new(0, end)),
        HeadlessRunFormat {
            bold: Some(false),
            spacing: Some(2.0),
            latin_font: Some("Arial".into()),
            ..Default::default()
        },
    )));
    let reopened = session.reopened();
    let marks = reopened.run_formats_in_range((0, end), (2, 0));
    assert_eq!(marks.len(), 2);
    for mark in &marks {
        assert_eq!(mark.bold, Some(false));
        assert_eq!(mark.spacing, Some(2.0));
        assert_eq!(mark.latin_font.as_deref(), Some("Arial"));
    }
    assert_eq!(marks[0].font_size, Some(18.0));
    assert_eq!(marks[0].no_proof, Some(true));
    assert!(session.xml().contains("ja-JP"));
    assert_eq!(marks[1].font_size, Some(20.0));
    assert_eq!(marks[1].italic, Some(true));
    assert_eq!(reopened.run_format_at(0, 0).spacing, None);
    assert_eq!(reopened.run_format_at(0, 0).font_size, Some(12.0));
    assert!(reopened.paragraph_pagination(0).keep_next);
    assert!(reopened
        .paragraph(0)
        .unwrap()
        .properties
        .as_ref()
        .unwrap()
        .mark_revision
        .is_some());
    assert!(session.xml().contains("Mark author"));
    session.undo();
    let restored = session.reopened();
    let mark = &restored.run_formats_in_range((0, end), (1, 0))[0];
    assert_eq!(mark.bold, Some(true));
    assert_eq!(mark.spacing, Some(1.5));
    assert_eq!(mark.no_proof, Some(true));
    assert!(session.xml().contains("ja-JP"));
    assert_eq!(restored.run_format_at(1, 0).bold, None);
    assert_eq!(restored.run_format_at(1, 0).spacing, None);
    assert!(restored
        .paragraph(0)
        .unwrap()
        .properties
        .as_ref()
        .unwrap()
        .mark_revision
        .is_some());
}

#[test]
fn text_only_formatting_preserves_marks_and_undo_removes_new_mark_properties() {
    use jade_docx_editor::commands::set_run_format::SetRunFormatCommand;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};
    let mut session = common::Session::open(
        r#"<w:p><w:pPr><w:rPr><w:sz w:val="36"/></w:rPr></w:pPr><w:r><w:t>A</w:t></w:r></w:p><w:p/><w:p/>"#,
        &[],
    );
    session.run(Box::new(SetRunFormatCommand::new(
        SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 1)),
        HeadlessRunFormat {
            font_size: Some(24.0),
            ..Default::default()
        },
    )));
    let reopened = session.reopened();
    assert_eq!(reopened.run_format_at(0, 0).font_size, Some(24.0));
    assert_eq!(
        reopened.run_formats_in_range((0, 1), (1, 0))[0].font_size,
        Some(18.0)
    );
    let before = session.xml();
    session.run(Box::new(SetRunFormatCommand::new(
        SelectionRange::new(TextPosition::new(1, 0), TextPosition::new(2, 0)),
        HeadlessRunFormat {
            italic: Some(true),
            ..Default::default()
        },
    )));
    assert_eq!(session.reopened().run_format_at(1, 0).italic, Some(true));
    session.undo();
    assert_eq!(session.xml(), before);
}

#[test]
fn individual_bold_and_size_commands_format_selected_marks_and_undo() {
    use jade_docx_editor::commands::{set_bold::SetBoldCommand, set_font_size::SetFontSizeCommand};
    use jade_docx_layout::selection::{SelectionRange, TextPosition};
    let mut session = common::Session::open(
        r#"<w:p><w:pPr><w:keepNext/><w:rPr><w:b w:val="0"/><w:bCs/><w:sz w:val="36"/><w:szCs w:val="44"/><w:spacing w:val="30"/></w:rPr></w:pPr><w:r><w:rPr><w:b w:val="0"/><w:sz w:val="24"/></w:rPr><w:t>前</w:t></w:r></w:p><w:p/><w:p><w:pPr><w:rPr><w:i/></w:rPr></w:pPr><w:r><w:t>後</w:t></w:r></w:p>"#,
        &[],
    );
    let range = SelectionRange::new(TextPosition::new(2, 0), TextPosition::new(0, 0));
    session.run(Box::new(SetBoldCommand::new(range, true)));
    session.run(Box::new(SetFontSizeCommand::new(range, 10.3)));
    let changed = session.reopened();
    let formats = changed.run_formats_in_range((0, 0), (2, 0));
    assert_eq!(formats.len(), 3); // text and two marks
    for format in &formats {
        assert_eq!(format.bold, Some(true));
        assert_eq!(format.font_size, Some(10.5));
    }
    assert_eq!(formats[1].complex_bold, Some(true));
    assert_eq!(formats[1].complex_font_size, Some(10.5));
    assert_eq!(formats[1].spacing, Some(1.5));
    assert!(changed.paragraph_pagination(0).keep_next);
    assert_eq!(changed.run_format_at(2, 0).bold, None);
    assert_eq!(changed.run_format_at(2, 0).font_size, Some(10.5));
    session.undo();
    let restored_size = session.reopened();
    let formats = restored_size.run_formats_in_range((0, 0), (2, 0));
    assert_eq!(formats[0].font_size, Some(12.0));
    assert_eq!(formats[1].font_size, Some(18.0));
    assert_eq!(formats[1].complex_font_size, Some(22.0));
    assert_eq!(formats[2].font_size, Some(10.5));
    assert!(formats.iter().all(|format| format.bold == Some(true)));
    session.undo();
    let restored = session.reopened();
    let formats = restored.run_formats_in_range((0, 0), (2, 0));
    assert_eq!(formats[0].bold, Some(false));
    assert_eq!(formats[1].bold, Some(false));
    assert_eq!(formats[2].bold, None);
    session.redo();
    session.redo();
    assert_eq!(
        session.reopened().run_formats_in_range((0, 0), (2, 0)),
        changed.run_formats_in_range((0, 0), (2, 0))
    );
}

#[test]
fn individual_commands_handle_mark_only_ranges_without_changing_text() {
    use jade_docx_editor::commands::{set_bold::SetBoldCommand, set_font_size::SetFontSizeCommand};
    use jade_docx_layout::selection::{SelectionRange, TextPosition};
    let mut session = common::Session::open(
        r#"<w:p><w:r><w:rPr><w:b w:val="0"/><w:sz w:val="24"/></w:rPr><w:t>前</w:t></w:r></w:p><w:p/>"#,
        &[],
    );
    let before = session.xml();
    let range = SelectionRange::new(TextPosition::new(0, "前".len()), TextPosition::new(1, 0));
    session.run(Box::new(SetBoldCommand::new(range, true)));
    session.run(Box::new(SetFontSizeCommand::new(range, 18.0)));
    let reopened = session.reopened();
    let mark = &reopened.run_formats_in_range((0, "前".len()), (1, 0))[0];
    assert_eq!(mark.bold, Some(true));
    assert_eq!(mark.font_size, Some(18.0));
    assert_eq!(reopened.run_format_at(0, 0).bold, Some(false));
    assert_eq!(reopened.run_format_at(0, 0).font_size, Some(12.0));
    session.undo();
    session.undo();
    assert_eq!(session.xml(), before);
}

#[test]
fn bold_and_italic_actions_change_mark_scripts_and_restore_on_undo() {
    use jade_docx_editor::commands::{
        apply_bold::ApplyBoldCommand, apply_italic::ApplyItalicCommand,
    };
    use jade_docx_layout::selection::{SelectionRange, TextPosition};
    let mut session = common::Session::open(
        r#"<w:p><w:pPr><w:rPr><w:b/><w:bCs/><w:i/><w:iCs/><w:sz w:val="36"/></w:rPr></w:pPr><w:r><w:rPr><w:b/><w:i/></w:rPr><w:t>前</w:t></w:r></w:p><w:p/>"#,
        &[],
    );
    let range = SelectionRange::new(TextPosition::new(0, "前".len()), TextPosition::new(1, 0));
    session.run(Box::new(ApplyBoldCommand::new_with_enabled(range, false)));
    session.run(Box::new(ApplyItalicCommand::new_with_enabled(range, false)));
    let check_disabled = |document: &WordDocument| {
        let mark = &document.run_formats_in_range((0, "前".len()), (1, 0))[0];
        assert_eq!(mark.bold, Some(false));
        assert_eq!(mark.complex_bold, Some(false));
        assert_eq!(mark.italic, Some(false));
        assert_eq!(mark.complex_italic, Some(false));
        assert_eq!(mark.font_size, Some(18.0));
        assert_eq!(document.run_format_at(0, 0).bold, Some(true));
        assert_eq!(document.run_format_at(0, 0).italic, Some(true));
    };
    check_disabled(&session.reopened());
    session.undo();
    session.undo();
    let reopened = session.reopened();
    let mark = &reopened.run_formats_in_range((0, "前".len()), (1, 0))[0];
    assert_eq!(mark.bold, Some(true));
    assert_eq!(mark.complex_bold, Some(true));
    assert_eq!(mark.italic, Some(true));
    assert_eq!(mark.complex_italic, Some(true));
    session.redo();
    session.redo();
    check_disabled(&session.reopened());
}

#[test]
fn grow_and_shrink_step_text_and_marks_from_their_own_effective_sizes() {
    use jade_docx_editor::commands::adjust_font_size::{
        AdjustFontSizeCommand, FontSizeDirection, FontSizeStep,
    };
    use jade_docx_layout::selection::{SelectionRange, TextPosition};
    for (direction, step, expected) in [
        (
            FontSizeDirection::Grow,
            FontSizeStep::Ladder,
            [14.0, 20.0, 26.0],
        ),
        (
            FontSizeDirection::Shrink,
            FontSizeStep::Point,
            [11.0, 17.0, 23.0],
        ),
    ] {
        let mut session = common::Session::open(
            r#"<w:p><w:pPr><w:pStyle w:val="Base"/><w:keepNext/><w:rPr><w:sz w:val="36"/><w:szCs w:val="44"/><w:spacing w:val="30"/></w:rPr></w:pPr><w:r><w:rPr><w:sz w:val="24"/></w:rPr><w:t>前</w:t></w:r></w:p><w:p><w:pPr><w:pStyle w:val="Base"/></w:pPr></w:p><w:p><w:r><w:rPr><w:sz w:val="144"/></w:rPr><w:t>後</w:t></w:r></w:p>"#,
            &[],
        );
        session.document.styles_part = Some(jade_docx_reader::StylesPart::from_xml(
            r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="Base"><w:rPr><w:sz w:val="48"/></w:rPr></w:style></w:styles>"#,
        ).unwrap());
        let before = session.xml();
        let range = SelectionRange::new(TextPosition::new(2, 0), TextPosition::new(0, 0));
        session.run(Box::new(AdjustFontSizeCommand::with_step(
            range, direction, step,
        )));
        let reopened = session.reopened();
        let formats = reopened.run_formats_in_range((0, 0), (2, 0));
        assert_eq!(formats.len(), 3);
        for (format, size) in formats.iter().zip(expected) {
            assert_eq!(format.font_size, Some(size));
            assert_eq!(format.complex_font_size, Some(size));
        }
        assert_eq!(formats[1].spacing, Some(1.5));
        assert!(reopened.paragraph_pagination(0).keep_next);
        assert_eq!(reopened.run_format_at(2, 0).font_size, Some(72.0));
        session.undo();
        assert_eq!(session.xml(), before);
        session.redo();
        assert_eq!(
            session.reopened().run_formats_in_range((0, 0), (2, 0)),
            formats
        );
    }
}

#[test]
fn font_size_steps_change_only_the_selected_mark_and_undo_new_properties() {
    use jade_docx_editor::commands::adjust_font_size::{
        AdjustFontSizeCommand, FontSizeDirection, FontSizeStep,
    };
    use jade_docx_layout::selection::{SelectionRange, TextPosition};
    for (direction, step, expected) in [
        (FontSizeDirection::Grow, FontSizeStep::Ladder, 11.0),
        (FontSizeDirection::Shrink, FontSizeStep::Point, 9.5),
    ] {
        let mut session = common::Session::open(
            r#"<w:p><w:r><w:rPr><w:sz w:val="24"/></w:rPr><w:t>前</w:t></w:r></w:p><w:p/>"#,
            &[],
        );
        let before = session.xml();
        let range = SelectionRange::new(TextPosition::new(0, "前".len()), TextPosition::new(1, 0));
        session.run(Box::new(AdjustFontSizeCommand::with_step(
            range, direction, step,
        )));
        let reopened = session.reopened();
        assert_eq!(reopened.run_format_at(0, 0).font_size, Some(12.0));
        let formats = reopened.run_formats_in_range((0, "前".len()), (1, 0));
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].font_size, Some(expected));
        assert_eq!(formats[0].complex_font_size, Some(expected));
        session.undo();
        assert_eq!(session.xml(), before);
        session.redo();
        assert_eq!(
            session
                .reopened()
                .run_formats_in_range((0, "前".len()), (1, 0)),
            formats
        );
    }
}

#[test]
fn explicit_color_covers_selected_marks_and_replaces_theme_color_with_undo() {
    use jade_docx_editor::commands::set_font_color::SetFontColorCommand;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};

    let mut session = common::Session::open(
        r#"<w:p><w:pPr><w:keepNext/><w:rPr><w:color w:val="4472C4" w:themeColor="accent1" w:themeTint="80"/><w:i/><w:spacing w:val="30"/></w:rPr></w:pPr><w:r><w:rPr><w:color w:val="4472C4" w:themeColor="accent1"/></w:rPr><w:t>前</w:t></w:r></w:p><w:p/><w:p><w:r><w:rPr><w:color w:val="00FF00"/></w:rPr><w:t>後</w:t></w:r></w:p>"#,
        &[],
    );
    let before = session.xml();
    let range = SelectionRange::new(TextPosition::new(2, 0), TextPosition::new(0, 0));
    session.run(Box::new(SetFontColorCommand::new(range, "#ff0000")));
    let reopened = session.reopened();
    let formats = reopened.run_formats_in_range((0, 0), (2, 0));
    assert_eq!(formats.len(), 3);
    for format in &formats {
        assert_eq!(format.color.as_deref(), Some("FF0000"));
        assert_eq!(format.theme_color, None);
    }
    assert_eq!(formats[1].italic, Some(true));
    assert_eq!(formats[1].spacing, Some(1.5));
    assert!(reopened.paragraph_pagination(0).keep_next);
    assert_eq!(
        reopened.run_format_at(2, 0).color.as_deref(),
        Some("00FF00")
    );
    session.undo();
    assert_eq!(session.xml(), before);
    session.redo();
    assert_eq!(
        session.reopened().run_formats_in_range((0, 0), (2, 0)),
        formats
    );
}

#[test]
fn explicit_color_handles_mark_only_and_text_only_ranges_independently() {
    use jade_docx_editor::commands::set_font_color::SetFontColorCommand;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};

    for mark_only in [true, false] {
        let mut session = common::Session::open(
            r#"<w:p><w:r><w:rPr><w:color w:val="00FF00"/></w:rPr><w:t>前</w:t></w:r></w:p><w:p/>"#,
            &[],
        );
        let before = session.xml();
        let range = if mark_only {
            SelectionRange::new(TextPosition::new(0, "前".len()), TextPosition::new(1, 0))
        } else {
            SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, "前".len()))
        };
        session.run(Box::new(SetFontColorCommand::new(range, "112233")));
        let reopened = session.reopened();
        assert_eq!(
            reopened.run_format_at(0, 0).color.as_deref(),
            Some(if mark_only { "00FF00" } else { "112233" })
        );
        let marks = reopened.run_formats_in_range((0, "前".len()), (1, 0));
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].color.as_deref(), mark_only.then_some("112233"));
        session.undo();
        assert_eq!(session.xml(), before);
        session.redo();
        assert_eq!(
            session
                .reopened()
                .run_formats_in_range((0, "前".len()), (1, 0)),
            marks
        );
    }
}

#[test]
fn automatic_font_color_is_serialized_as_auto_for_text_and_marks() {
    use jade_docx_editor::commands::set_font_color::SetFontColorCommand;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};

    for color in ["auto", "#auto", " AUTO "] {
        let mut session = common::Session::open(
            r#"<w:p><w:pPr><w:rPr><w:color w:val="4472C4" w:themeColor="accent1"/></w:rPr></w:pPr><w:r><w:t>前</w:t></w:r></w:p><w:p/>"#,
            &[],
        );
        let before = session.xml();
        let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(1, 0));
        session.run(Box::new(SetFontColorCommand::new(range, color)));
        let xml = session.xml();
        assert_eq!(xml.matches("<w:color w:val=\"auto\"/>").count(), 2, "{xml}");
        let formats = session.reopened().run_formats_in_range((0, 0), (1, 0));
        assert_eq!(formats.len(), 2);
        for format in formats {
            assert_eq!(format.color.as_deref(), Some("auto"));
            assert_eq!(format.theme_color, None);
        }
        session.undo();
        assert_eq!(session.xml(), before);
    }
}

#[test]
fn underline_actions_include_marks_preserve_other_properties_and_undo() {
    use jade_docx_editor::commands::apply_underline::ApplyUnderlineCommand;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};

    for kind in ["single", "words", "double", "none"] {
        let mut session = common::Session::open(
            r#"<w:p><w:pPr><w:keepNext/><w:rPr><w:u w:val="wave" w:color="FF0000"/><w:i/><w:spacing w:val="30"/></w:rPr></w:pPr><w:r><w:rPr><w:u w:val="wave" w:color="FF0000"/></w:rPr><w:t>前</w:t></w:r></w:p><w:p/><w:p><w:r><w:rPr><w:u w:val="dash"/></w:rPr><w:t>後</w:t></w:r></w:p>"#,
            &[],
        );
        let before = session.xml();
        let range = SelectionRange::new(TextPosition::new(2, 0), TextPosition::new(0, 0));
        let command = match kind {
            "single" => ApplyUnderlineCommand::new(range),
            "none" => ApplyUnderlineCommand::new_with_enabled(range, false),
            _ => ApplyUnderlineCommand::new_with_kind(range, kind),
        };
        session.run(Box::new(command));
        let reopened = session.reopened();
        let formats = reopened.run_formats_in_range((0, 0), (2, 0));
        assert_eq!(formats.len(), 3);
        for format in &formats {
            assert_eq!(
                format.underline.as_deref(),
                (kind != "none").then_some(kind)
            );
        }
        for format in &formats[..2] {
            assert_eq!(
                format.underline_color.as_deref(),
                (kind != "none").then_some("FF0000")
            );
        }
        assert_eq!(formats[1].italic, Some(true));
        assert_eq!(formats[1].spacing, Some(1.5));
        assert!(reopened.paragraph_pagination(0).keep_next);
        assert_eq!(
            reopened.run_format_at(2, 0).underline.as_deref(),
            Some("dash")
        );
        if kind == "none" {
            // Explicit off must survive a save/reload rather than expose an inherited underline.
            assert_eq!(session.xml().matches("<w:u w:val=\"none\"/>").count(), 3);
        }
        session.undo();
        assert_eq!(session.xml(), before);
        session.redo();
        assert_eq!(
            session.reopened().run_formats_in_range((0, 0), (2, 0)),
            formats
        );
    }
}

#[test]
fn underline_mark_only_selection_leaves_text_unchanged_and_undo_removes_properties() {
    use jade_docx_editor::commands::apply_underline::ApplyUnderlineCommand;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};

    for kind in ["words", "none"] {
        let mut session = common::Session::open(
            r#"<w:p><w:r><w:rPr><w:u w:val="double"/></w:rPr><w:t>前</w:t></w:r></w:p><w:p/>"#,
            &[],
        );
        let before = session.xml();
        let range = SelectionRange::new(TextPosition::new(0, "前".len()), TextPosition::new(1, 0));
        session.run(Box::new(ApplyUnderlineCommand::new_with_kind(range, kind)));
        let reopened = session.reopened();
        assert_eq!(
            reopened.run_format_at(0, 0).underline.as_deref(),
            Some("double")
        );
        let marks = reopened.run_formats_in_range((0, "前".len()), (1, 0));
        assert_eq!(marks.len(), 1);
        assert_eq!(
            marks[0].underline.as_deref(),
            (kind != "none").then_some(kind)
        );
        session.undo();
        assert_eq!(session.xml(), before);
        session.redo();
        assert_eq!(
            session
                .reopened()
                .run_formats_in_range((0, "前".len()), (1, 0)),
            marks
        );
    }
}

#[test]
fn highlight_includes_selected_marks_and_restores_previous_properties() {
    use jade_docx_editor::commands::set_highlight_color::SetHighlightColorCommand;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};

    for color in ["yellow", "none"] {
        let mut session = common::Session::open(
            r#"<w:p><w:pPr><w:keepNext/><w:rPr><w:highlight w:val="cyan"/><w:i/><w:spacing w:val="30"/></w:rPr></w:pPr><w:r><w:rPr><w:highlight w:val="green"/></w:rPr><w:t>前</w:t></w:r></w:p><w:p/><w:p><w:pPr><w:rPr><w:highlight w:val="magenta"/></w:rPr></w:pPr><w:r><w:rPr><w:highlight w:val="blue"/></w:rPr><w:t>後</w:t></w:r></w:p>"#,
            &[],
        );
        let before = session.xml();
        let range = SelectionRange::new(TextPosition::new(2, 0), TextPosition::new(0, 0));
        session.run(Box::new(SetHighlightColorCommand::new(range, color)));
        let reopened = session.reopened();
        let mark = |index| {
            reopened
                .paragraph(index)
                .unwrap()
                .properties
                .as_ref()
                .unwrap()
                .mark_run_properties
                .as_ref()
                .unwrap()
        };
        assert_eq!(mark(0).highlight.as_deref(), Some(color));
        assert_eq!(mark(1).highlight.as_deref(), Some(color));
        assert_eq!(mark(2).highlight.as_deref(), Some("magenta"));
        assert_eq!(mark(0).italic, Some(true));
        assert_eq!(mark(0).char_spacing, Some(1.5));
        assert!(reopened.paragraph_pagination(0).keep_next);
        assert_eq!(
            reopened.paragraph(0).unwrap().runs[0]
                .properties
                .as_ref()
                .unwrap()
                .highlight
                .as_deref(),
            Some(color)
        );
        assert_eq!(
            reopened.paragraph(2).unwrap().runs[0]
                .properties
                .as_ref()
                .unwrap()
                .highlight
                .as_deref(),
            Some("blue")
        );
        let changed = session.xml();
        assert_eq!(
            changed
                .matches(&format!("<w:highlight w:val=\"{color}\"/>"))
                .count(),
            3
        );
        session.undo();
        assert_eq!(session.xml(), before);
        session.redo();
        assert_eq!(session.xml(), changed);
    }
}

#[test]
fn highlight_mark_only_and_text_only_ranges_remain_independent() {
    use jade_docx_editor::commands::set_highlight_color::SetHighlightColorCommand;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};

    for mark_only in [true, false] {
        let mut session = common::Session::open(
            r#"<w:p><w:r><w:rPr><w:highlight w:val="cyan"/></w:rPr><w:t>前</w:t></w:r></w:p><w:p/>"#,
            &[],
        );
        let before = session.xml();
        let range = if mark_only {
            SelectionRange::new(TextPosition::new(0, "前".len()), TextPosition::new(1, 0))
        } else {
            SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, "前".len()))
        };
        session.run(Box::new(SetHighlightColorCommand::new(range, "yellow")));
        let reopened = session.reopened();
        let paragraph = reopened.paragraph(0).unwrap();
        assert_eq!(
            paragraph.runs[0]
                .properties
                .as_ref()
                .unwrap()
                .highlight
                .as_deref(),
            Some(if mark_only { "cyan" } else { "yellow" })
        );
        assert_eq!(
            paragraph
                .properties
                .as_ref()
                .and_then(|p| p.mark_run_properties.as_ref())
                .and_then(|r| r.highlight.as_deref()),
            mark_only.then_some("yellow")
        );
        let changed = session.xml();
        session.undo();
        assert_eq!(session.xml(), before);
        session.redo();
        assert_eq!(session.xml(), changed);
    }
}

#[test]
fn clear_formatting_includes_marks_and_preserves_highlight_and_language() {
    use jade_docx_editor::commands::clear_formatting::ClearFormattingCommand;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};

    for mark_only in [false, true] {
        let mut session = common::Session::open(
            r#"<w:p><w:pPr><w:keepNext/><w:rPr><w:b/><w:sz w:val="36"/><w:color w:val="FF0000" w:themeColor="accent1"/><w:highlight w:val="yellow"/><w:lang w:val="ja-JP"/></w:rPr></w:pPr><w:r><w:rPr><w:b/><w:color w:val="FF0000" w:themeColor="accent1"/></w:rPr><w:t>A</w:t></w:r></w:p><w:p/><w:p><w:pPr><w:rPr><w:i/></w:rPr></w:pPr><w:r><w:t>B</w:t></w:r></w:p>"#,
            &[],
        );
        let before = session.xml();
        let range = SelectionRange::new(
            TextPosition::new(2, 0),
            TextPosition::new(0, usize::from(mark_only)),
        );
        session.run(Box::new(ClearFormattingCommand::new(range)));
        let reopened = session.reopened();
        let mark = reopened
            .paragraph(0)
            .unwrap()
            .properties
            .as_ref()
            .unwrap()
            .mark_run_properties
            .as_ref()
            .unwrap();
        assert_eq!(mark.bold, None);
        assert_eq!(mark.font_size, None);
        assert_eq!(mark.color, None);
        assert_eq!(mark.theme_color, None);
        assert_eq!(mark.highlight.as_deref(), Some("yellow"));
        assert_eq!(mark.language.as_deref(), Some("ja-JP"));
        assert!(reopened.paragraph_pagination(0).keep_next);
        assert!(reopened
            .paragraph(1)
            .unwrap()
            .properties
            .as_ref()
            .and_then(|p| p.mark_run_properties.as_ref())
            .is_none());
        assert_eq!(
            reopened
                .paragraph(2)
                .unwrap()
                .properties
                .as_ref()
                .unwrap()
                .mark_run_properties
                .as_ref()
                .unwrap()
                .italic,
            Some(true)
        );
        let text = reopened.paragraph(0).unwrap().runs[0]
            .properties
            .as_ref()
            .unwrap();
        assert_eq!(text.bold, mark_only.then_some(true));
        assert_eq!(text.theme_color.is_some(), mark_only);
        let changed = session.xml();
        session.undo();
        assert_eq!(session.xml(), before);
        session.redo();
        assert_eq!(session.xml(), changed);
    }
}

#[test]
fn strike_actions_format_selected_marks_and_keep_text_only_ranges_independent() {
    use jade_docx_editor::commands::apply_strike::ApplyStrikeCommand;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};

    for action in ["single", "double", "off"] {
        for scope in ["multi", "marks", "text"] {
            let mut session = common::Session::open(
                r#"<w:p><w:pPr><w:keepNext/><w:rPr><w:strike/><w:dstrike/><w:i/></w:rPr></w:pPr><w:r><w:rPr><w:strike/><w:dstrike/></w:rPr><w:t>A</w:t></w:r></w:p><w:p/><w:p><w:pPr><w:rPr><w:i/></w:rPr></w:pPr><w:r><w:t>B</w:t></w:r></w:p>"#,
                &[],
            );
            let before = session.xml();
            let range = match scope {
                "marks" => SelectionRange::new(TextPosition::new(2, 0), TextPosition::new(0, 1)),
                "text" => SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 1)),
                _ => SelectionRange::new(TextPosition::new(2, 0), TextPosition::new(0, 0)),
            };
            let command = match action {
                "double" => ApplyStrikeCommand::new_double(range),
                "off" => ApplyStrikeCommand::new_with_enabled(range, false),
                _ => ApplyStrikeCommand::new(range),
            };
            session.run(Box::new(command));
            let reopened = session.reopened();
            let expected = (Some(action == "single"), Some(action == "double"));
            let text = reopened.paragraph(0).unwrap().runs[0]
                .properties
                .as_ref()
                .unwrap();
            assert_eq!(
                (text.strike, text.double_strike),
                if scope == "marks" {
                    (Some(true), Some(true))
                } else {
                    expected
                }
            );
            let mark = reopened
                .paragraph(0)
                .unwrap()
                .properties
                .as_ref()
                .unwrap()
                .mark_run_properties
                .as_ref()
                .unwrap();
            assert_eq!(
                (mark.strike, mark.double_strike),
                if scope == "text" {
                    (Some(true), Some(true))
                } else {
                    expected
                }
            );
            assert_eq!(mark.italic, Some(true));
            assert!(reopened.paragraph_pagination(0).keep_next);
            let empty = reopened
                .paragraph(1)
                .unwrap()
                .properties
                .as_ref()
                .and_then(|p| p.mark_run_properties.as_ref());
            if scope == "text" {
                assert!(empty.is_none());
            } else {
                let empty = empty.unwrap();
                assert_eq!((empty.strike, empty.double_strike), expected);
            }
            let end = reopened
                .paragraph(2)
                .unwrap()
                .properties
                .as_ref()
                .unwrap()
                .mark_run_properties
                .as_ref()
                .unwrap();
            assert_eq!((end.strike, end.double_strike), (None, None));
            let changed = session.xml();
            session.undo();
            assert_eq!(session.xml(), before);
            session.redo();
            assert_eq!(session.xml(), changed);
        }
    }
}

#[test]
fn superscript_and_subscript_actions_include_marks_and_write_explicit_baseline_when_disabled() {
    use jade_docx_editor::commands::{
        apply_subscript::ApplySubscriptCommand, apply_superscript::ApplySuperscriptCommand, Command,
    };
    use jade_docx_layout::selection::{SelectionRange, TextPosition};

    for superscript in [true, false] {
        for enabled in [true, false] {
            for scope in ["multi", "marks", "text"] {
                let mut session = common::Session::open(
                    r#"<w:p><w:pPr><w:keepNext/><w:rPr><w:vertAlign w:val="subscript"/><w:i/></w:rPr></w:pPr><w:r><w:rPr><w:vertAlign w:val="subscript"/></w:rPr><w:t>A</w:t></w:r></w:p><w:p/><w:p><w:pPr><w:rPr><w:vertAlign w:val="superscript"/></w:rPr></w:pPr><w:r><w:t>B</w:t></w:r></w:p>"#,
                    &[],
                );
                let before = session.xml();
                let range = match scope {
                    "marks" => {
                        SelectionRange::new(TextPosition::new(2, 0), TextPosition::new(0, 1))
                    }
                    "text" => SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 1)),
                    _ => SelectionRange::new(TextPosition::new(2, 0), TextPosition::new(0, 0)),
                };
                let command: Box<dyn Command> = if superscript {
                    Box::new(ApplySuperscriptCommand::new_with_enabled(range, enabled))
                } else {
                    Box::new(ApplySubscriptCommand::new_with_enabled(range, enabled))
                };
                session.run(command);
                let reopened = session.reopened();
                let expected = if !enabled {
                    "baseline"
                } else if superscript {
                    "superscript"
                } else {
                    "subscript"
                };
                let text = reopened.paragraph(0).unwrap().runs[0]
                    .properties
                    .as_ref()
                    .unwrap();
                assert_eq!(
                    text.baseline.as_deref(),
                    Some(if scope == "marks" {
                        "subscript"
                    } else {
                        expected
                    })
                );
                let mark = reopened
                    .paragraph(0)
                    .unwrap()
                    .properties
                    .as_ref()
                    .unwrap()
                    .mark_run_properties
                    .as_ref()
                    .unwrap();
                assert_eq!(
                    mark.baseline.as_deref(),
                    Some(if scope == "text" {
                        "subscript"
                    } else {
                        expected
                    })
                );
                assert_eq!(mark.italic, Some(true));
                assert!(reopened.paragraph_pagination(0).keep_next);
                let empty = reopened
                    .paragraph(1)
                    .unwrap()
                    .properties
                    .as_ref()
                    .and_then(|p| p.mark_run_properties.as_ref());
                assert_eq!(
                    empty.and_then(|p| p.baseline.as_deref()),
                    if scope == "text" {
                        None
                    } else {
                        Some(expected)
                    }
                );
                assert_eq!(
                    reopened
                        .paragraph(2)
                        .unwrap()
                        .properties
                        .as_ref()
                        .unwrap()
                        .mark_run_properties
                        .as_ref()
                        .unwrap()
                        .baseline
                        .as_deref(),
                    Some("superscript")
                );
                let changed = session.xml();
                session.undo();
                assert_eq!(session.xml(), before);
                session.redo();
                assert_eq!(session.xml(), changed);
            }
        }
    }
}

#[test]
fn baseline_patch_overrides_inherited_vertical_alignment_for_text_and_marks() {
    use jade_docx_editor::commands::set_run_format::SetRunFormatCommand;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};

    for inherited in ["superscript", "subscript"] {
        let mut session =
            session_with_run_defaults(&format!(r#"<w:vertAlign w:val="{inherited}"/>"#));
        let before = session.xml();
        let formats = session.document.run_formats_in_range((0, 0), (2, 0));
        assert_eq!(formats.len(), 3);
        assert!(formats
            .iter()
            .all(|f| f.vertical_align.as_deref() == Some(inherited)));
        let range = SelectionRange::new(TextPosition::new(2, 0), TextPosition::new(0, 0));
        session.run(Box::new(SetRunFormatCommand::new(
            range,
            HeadlessRunFormat {
                vertical_align: Some("baseline".into()),
                ..Default::default()
            },
        )));
        let formats = session.document.run_formats_in_range((0, 0), (2, 0));
        assert!(formats
            .iter()
            .all(|f| f.vertical_align.as_deref() == Some("baseline")));
        assert_eq!(
            session
                .document
                .run_format_at(2, 0)
                .vertical_align
                .as_deref(),
            Some(inherited)
        );
        let changed = session.xml();
        assert_eq!(
            changed.matches("<w:vertAlign w:val=\"baseline\"/>").count(),
            3
        );
        let reopened = session.reopened();
        assert!(reopened
            .run_formats_in_range((0, 0), (2, 0))
            .iter()
            .all(|f| f.vertical_align.as_deref() == Some("baseline")));
        session.undo();
        assert_eq!(session.xml(), before);
        assert!(session
            .document
            .run_formats_in_range((0, 0), (2, 0))
            .iter()
            .all(|f| f.vertical_align.as_deref() == Some(inherited)));
        session.redo();
        assert_eq!(session.xml(), changed);
    }
}

#[test]
fn boolean_format_resets_override_inheritance_for_text_and_marks() {
    use jade_docx_editor::commands::set_run_format::SetRunFormatCommand;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};

    for tag in [
        "strike",
        "dstrike",
        "caps",
        "smallCaps",
        "vanish",
        "outline",
        "shadow",
        "emboss",
        "imprint",
        "w14:cntxtAlts",
    ] {
        let qname = if tag.starts_with("w14:") {
            tag.to_string()
        } else {
            format!("w:{tag}")
        };
        let value_name = if tag.starts_with("w14:") {
            "w14:val"
        } else {
            "w:val"
        };
        let mut session = session_with_run_defaults(&format!("<{qname}/>"));
        let get = |format: &HeadlessRunFormat| match tag {
            "strike" => format.strike,
            "dstrike" => format.double_strike,
            "caps" => format.caps,
            "smallCaps" => format.small_caps,
            "vanish" => format.hidden,
            "outline" => format.outline,
            "shadow" => format.shadow,
            "emboss" => format.emboss,
            "imprint" => format.engrave,
            "w14:cntxtAlts" => format.contextual_alternates,
            _ => unreachable!(),
        };
        let before = session.xml();
        let formats = session.document.run_formats_in_range((0, 0), (2, 0));
        assert_eq!(formats.len(), 3);
        assert!(
            formats.iter().all(|f| get(f) == Some(true)),
            "inherited {tag}"
        );
        let mut patch = HeadlessRunFormat::default();
        match tag {
            "strike" => patch.strike = Some(false),
            "dstrike" => patch.double_strike = Some(false),
            "caps" => patch.caps = Some(false),
            "smallCaps" => patch.small_caps = Some(false),
            "vanish" => patch.hidden = Some(false),
            "outline" => patch.outline = Some(false),
            "shadow" => patch.shadow = Some(false),
            "emboss" => patch.emboss = Some(false),
            "imprint" => patch.engrave = Some(false),
            "w14:cntxtAlts" => patch.contextual_alternates = Some(false),
            _ => unreachable!(),
        }
        session.run(Box::new(SetRunFormatCommand::new(
            SelectionRange::new(TextPosition::new(2, 0), TextPosition::new(0, 0)),
            patch,
        )));
        assert!(
            session
                .document
                .run_formats_in_range((0, 0), (2, 0))
                .iter()
                .all(|f| get(f) == Some(false)),
            "reset {tag}"
        );
        assert_eq!(get(&session.document.run_format_at(2, 0)), Some(true));
        let changed = session.xml();
        assert_eq!(
            changed
                .matches(&format!(r#"<{qname} {value_name}="0"/>"#))
                .count(),
            3
        );
        assert!(
            session
                .reopened()
                .run_formats_in_range((0, 0), (2, 0))
                .iter()
                .all(|f| get(f) == Some(false)),
            "reopened {tag}"
        );
        session.undo();
        assert_eq!(session.xml(), before);
        assert!(
            session
                .document
                .run_formats_in_range((0, 0), (2, 0))
                .iter()
                .all(|f| get(f) == Some(true)),
            "undo {tag}"
        );
        session.redo();
        assert_eq!(session.xml(), changed);
    }
}

#[test]
fn exclusive_formats_suppress_the_inherited_counterpart() {
    use jade_docx_editor::commands::set_run_format::SetRunFormatCommand;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};

    for (inherited, enabled) in [
        ("strike", "dstrike"),
        ("dstrike", "strike"),
        ("caps", "smallCaps"),
        ("smallCaps", "caps"),
        ("emboss", "imprint"),
        ("imprint", "emboss"),
    ] {
        let mut session = session_with_run_defaults(&format!("<w:{inherited}/>"));
        let get = |format: &HeadlessRunFormat, tag: &str| match tag {
            "strike" => format.strike,
            "dstrike" => format.double_strike,
            "caps" => format.caps,
            "smallCaps" => format.small_caps,
            "emboss" => format.emboss,
            "imprint" => format.engrave,
            _ => unreachable!(),
        };
        let mut patch = HeadlessRunFormat::default();
        match enabled {
            "strike" => patch.strike = Some(true),
            "dstrike" => patch.double_strike = Some(true),
            "caps" => patch.caps = Some(true),
            "smallCaps" => patch.small_caps = Some(true),
            "emboss" => patch.emboss = Some(true),
            "imprint" => patch.engrave = Some(true),
            _ => unreachable!(),
        }
        let before = session.xml();
        session.run(Box::new(SetRunFormatCommand::new(
            SelectionRange::new(TextPosition::new(2, 0), TextPosition::new(0, 0)),
            patch,
        )));
        for document in [&session.document, &session.reopened()] {
            let formats = document.run_formats_in_range((0, 0), (2, 0));
            assert_eq!(formats.len(), 3);
            for format in formats {
                assert_eq!(get(&format, inherited), Some(false), "suppress {inherited}");
                assert_eq!(get(&format, enabled), Some(true), "enable {enabled}");
            }
            assert_eq!(get(&document.run_format_at(2, 0), inherited), Some(true));
        }
        let changed = session.xml();
        session.undo();
        assert_eq!(session.xml(), before);
        assert!(session
            .document
            .run_formats_in_range((0, 0), (2, 0))
            .iter()
            .all(|f| get(f, inherited) == Some(true)));
        session.redo();
        assert_eq!(session.xml(), changed);
    }
}

/// Include styles in the actual package so reopening still exercises inheritance.
fn session_with_run_defaults(properties: &str) -> common::Session {
    use std::io::{Read, Write};
    let bytes = common::package(
        "<w:p><w:r><w:t>A</w:t></w:r></w:p><w:p/><w:p><w:r><w:t>B</w:t></w:r></w:p>",
        &[],
    );
    let mut source = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut output = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for index in 0..source.len() {
        let mut file = source.by_index(index).unwrap();
        let name = file.name().unwrap().into_owned();
        let mut content = String::new();
        file.read_to_string(&mut content).unwrap();
        if name == "word/_rels/document.xml.rels" {
            content = content.replace("</Relationships>", r#"<Relationship Id="rIdStyles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#);
        } else if name == "[Content_Types].xml" {
            content = content.replace("</Types>", r#"<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#);
        }
        output.start_file(name, options).unwrap();
        output.write_all(content.as_bytes()).unwrap();
    }
    output.start_file("word/styles.xml", options).unwrap();
    write!(output, r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:docDefaults><w:rPrDefault><w:rPr>{properties}</w:rPr></w:rPrDefault></w:docDefaults></w:styles>"#).unwrap();
    common::Session::new(common::load(&output.finish().unwrap().into_inner()))
}

#[test]
fn font_family_formats_selected_marks_and_preserves_other_script_slots() {
    use jade_docx_editor::commands::set_font_family::SetFontFamilyCommand;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};

    // Theme slots take precedence over concrete names at load. Exercise both
    // theme-driven and concrete original faces, rather than expecting shadowed
    // fallback names to survive the reader's normalization.
    for themed in [false, true] {
        let themes = if themed {
            r#" w:asciiTheme="majorAscii" w:hAnsiTheme="majorHAnsi" w:eastAsiaTheme="majorEastAsia" w:cstheme="majorBidi""#
        } else {
            ""
        };
        let fonts = format!(
            r#"<w:rFonts w:ascii="OldLatin" w:hAnsi="OldLatin" w:eastAsia="OldEast" w:cs="OldComplex"{themes}/>"#
        );
        for (family, east_asian) in [(" Arial ", false), (" 游明朝 ", true)] {
            for scope in ["multi", "marks", "text"] {
                let mut session = common::Session::open(&format!(
                "<w:p><w:pPr><w:keepNext/><w:rPr>{fonts}<w:i/></w:rPr></w:pPr><w:r><w:rPr>{fonts}<w:b/></w:rPr><w:t>A</w:t></w:r></w:p><w:p/><w:p><w:r><w:rPr><w:rFonts w:ascii=\"EndLatin\"/></w:rPr><w:t>B</w:t></w:r></w:p>"
            ), &[]);
                let before = session.xml();
                let range = if scope == "text" {
                    SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 1))
                } else {
                    SelectionRange::new(
                        TextPosition::new(2, 0),
                        TextPosition::new(0, usize::from(scope == "marks")),
                    )
                };
                session.run(Box::new(SetFontFamilyCommand::new(range, family)));
                let reopened = session.reopened();
                for document in [&session.document, &reopened] {
                    let paragraph = document.paragraph(0).unwrap();
                    let mark = paragraph
                        .properties
                        .as_ref()
                        .unwrap()
                        .mark_run_properties
                        .as_ref()
                        .unwrap();
                    let text = paragraph.runs[0].properties.as_ref().unwrap();
                    for (properties, selected) in
                        [(mark, scope != "text"), (text, scope != "marks")]
                    {
                        let actual = properties.fonts.as_ref().unwrap();
                        assert_eq!(
                            actual.ascii.as_deref(),
                            if selected {
                                Some(family.trim())
                            } else {
                                (!themed).then_some("OldLatin")
                            }
                        );
                        assert_eq!(actual.h_ansi.as_deref(), actual.ascii.as_deref());
                        assert_eq!(
                            actual.ascii_theme.as_deref(),
                            (!selected && themed).then_some("majorAscii")
                        );
                        assert_eq!(
                            actual.h_ansi_theme.as_deref(),
                            (!selected && themed).then_some("majorHAnsi")
                        );
                        assert_eq!(
                            actual.east_asia.as_deref(),
                            if selected && east_asian {
                                Some(family.trim())
                            } else {
                                (!themed).then_some("OldEast")
                            }
                        );
                        assert_eq!(
                            actual.complex.as_deref(),
                            if selected && east_asian {
                                Some(family.trim())
                            } else {
                                (!themed).then_some("OldComplex")
                            }
                        );
                        assert_eq!(
                            actual.east_asia_theme.as_deref(),
                            (themed && !(selected && east_asian)).then_some("majorEastAsia")
                        );
                        assert_eq!(
                            actual.complex_theme.as_deref(),
                            (themed && !(selected && east_asian)).then_some("majorBidi")
                        );
                    }
                    assert_eq!(mark.italic, Some(true));
                    assert_eq!(text.bold, Some(true));
                    assert!(document.paragraph_pagination(0).keep_next);
                    let empty_mark = document
                        .paragraph(1)
                        .unwrap()
                        .properties
                        .as_ref()
                        .and_then(|p| p.mark_run_properties.as_ref());
                    if scope == "text" {
                        assert!(empty_mark.is_none());
                    } else {
                        let fonts = empty_mark.unwrap().fonts.as_ref().unwrap();
                        assert_eq!(fonts.ascii.as_deref(), Some(family.trim()));
                        assert_eq!(
                            fonts.east_asia.as_deref(),
                            east_asian.then(|| family.trim())
                        );
                        assert_eq!(fonts.complex.as_deref(), east_asian.then(|| family.trim()));
                    }
                    assert_eq!(
                        document.paragraph(2).unwrap().runs[0]
                            .properties
                            .as_ref()
                            .unwrap()
                            .fonts
                            .as_ref()
                            .unwrap()
                            .ascii
                            .as_deref(),
                        Some("EndLatin")
                    );
                }
                let changed = session.xml();
                session.undo();
                assert_eq!(session.xml(), before, "undo {family} {scope}");
                session.redo();
                assert_eq!(session.xml(), changed, "redo {family} {scope}");
            }
        }
    }
}
