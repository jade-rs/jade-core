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
    // Normal, 游明朝 10.5 pt with kerning.
    assert_eq!(
        document.run_format_at(0, 0),
        HeadlessRunFormat {
            latin_font: Some("游明朝".into()),
            east_asia_font: Some("游明朝".into()),
            complex_font: Some("Times New Roman".into()),
            font_size: Some(10.5),
            complex_font_size: Some(10.5),
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
