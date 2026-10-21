//! Word's Japanese typography: the 体裁 tab on a paragraph and the
//! document's 文字体裁, written where Word writes them, read back into the
//! model the line breaker uses, and undone.

use std::io::Read;
use std::path::PathBuf;

use jade_docx_editor::commands::set_typography::{
    SetJapaneseTypographyCommand, SetParagraphTypographyCommand,
};
use jade_docx_editor::commands::{CommandContext, CommandExecutor};
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::{HeadlessJapaneseTypography, HeadlessParagraphTypography};

fn load_bytes(bytes: &[u8]) -> WordDocument {
    futures::executor::block_on(WordDocument::load(bytes, &Default::default())).expect("load")
}

fn load(fixture: &str) -> WordDocument {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures")
        .join(fixture);
    load_bytes(&std::fs::read(path).expect("fixture"))
}

fn part(document: &WordDocument, name: &str) -> String {
    let exported = document.export_docx().expect("export");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
    let mut xml = String::new();
    archive
        .by_name(name)
        .expect("part")
        .read_to_string(&mut xml)
        .expect("read");
    xml
}

fn body_paragraph(document: &WordDocument) -> usize {
    (0..document.paragraph_count())
        .find(|&index| document.is_body_paragraph_surface(index))
        .expect("a body paragraph")
}

#[test]
fn the_paragraph_options_are_written_read_back_and_undone() {
    let mut document = load("hello.docx");
    let paragraph = body_paragraph(&document);
    let mut selection = None;
    let mut executor = CommandExecutor::new();
    let typography = HeadlessParagraphTypography {
        kinsoku: Some(false),
        overflow_punctuation: Some(false),
        top_line_punctuation: Some(true),
        auto_space_de: Some(false),
        auto_space_dn: Some(false),
        word_wrap: Some(false),
    };
    {
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor
            .execute(
                Box::new(SetParagraphTypographyCommand::new(
                    paragraph, paragraph, typography,
                )),
                &mut ctx,
            )
            .unwrap();
    }
    let xml = part(&document, "word/document.xml");
    for expected in [
        r#"<w:kinsoku w:val="0"/>"#,
        r#"<w:wordWrap w:val="0"/>"#,
        r#"<w:overflowPunct w:val="0"/>"#,
        r#"<w:topLinePunct w:val="1"/>"#,
        r#"<w:autoSpaceDE w:val="0"/>"#,
        r#"<w:autoSpaceDN w:val="0"/>"#,
    ] {
        assert!(xml.contains(expected), "missing {expected}");
    }
    let reread = load_bytes(&document.export_docx().unwrap());
    let settings = reread
        .paragraph(paragraph)
        .and_then(|paragraph| paragraph.properties.as_ref())
        .and_then(|properties| properties.east_asian_settings.clone())
        .expect("paragraph typography");
    assert_eq!(settings.kinsoku, Some(false));
    assert_eq!(settings.auto_space_de, Some(false));
    assert_eq!(settings.top_line_punctuation, Some(true));

    // Back to Word's defaults: the elements go.
    {
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor
            .execute(
                Box::new(SetParagraphTypographyCommand::new(
                    paragraph,
                    paragraph,
                    HeadlessParagraphTypography {
                        kinsoku: Some(true),
                        auto_space_de: Some(true),
                        ..Default::default()
                    },
                )),
                &mut ctx,
            )
            .unwrap();
    }
    let xml = part(&document, "word/document.xml");
    assert!(!xml.contains("<w:kinsoku"));
    assert!(!xml.contains("<w:autoSpaceDE"));
    assert!(xml.contains(r#"<w:autoSpaceDN w:val="0"/>"#));
    {
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor.undo(&mut ctx).unwrap();
        executor.undo(&mut ctx).unwrap();
    }
    assert!(!part(&document, "word/document.xml").contains("<w:autoSpaceDN"));
}

#[test]
fn the_document_kinsoku_level_and_compression_go_into_settings() {
    let mut document = load("hello.docx");
    let original = part(&document, "word/settings.xml");
    let mut selection = None;
    let mut executor = CommandExecutor::new();
    let apply = |document: &mut WordDocument,
                 executor: &mut CommandExecutor,
                 selection: &mut Option<_>,
                 typography: HeadlessJapaneseTypography| {
        let mut ctx = CommandContext::new(document, selection);
        executor
            .execute(
                Box::new(SetJapaneseTypographyCommand::new(typography)),
                &mut ctx,
            )
            .unwrap();
    };
    apply(
        &mut document,
        &mut executor,
        &mut selection,
        HeadlessJapaneseTypography {
            kinsoku_level: Some("strict".into()),
            character_spacing_control: Some("compressPunctuation".into()),
            ..Default::default()
        },
    );
    let settings = part(&document, "word/settings.xml");
    assert!(
        settings.contains("<w:strictFirstAndLastChars/>"),
        "{settings}"
    );
    assert!(settings.contains(r#"<w:characterSpacingControl w:val="compressPunctuation"/>"#));
    let model = &document.document.as_ref().unwrap().east_asian_line_breaking;
    assert!(model.strict_first_and_last_chars);
    assert_eq!(
        model.character_spacing_control.as_deref(),
        Some("compressPunctuation")
    );

    apply(
        &mut document,
        &mut executor,
        &mut selection,
        HeadlessJapaneseTypography {
            kinsoku_level: Some("custom".into()),
            no_line_break_before: Some("、。」".into()),
            no_line_break_after: Some("「（".into()),
            ..Default::default()
        },
    );
    let settings = part(&document, "word/settings.xml");
    assert!(!settings.contains("strictFirstAndLastChars"));
    assert!(settings.contains(r#"<w:noLineBreaksBefore w:lang="ja-JP" w:val="、。」"/>"#));
    let model = &document.document.as_ref().unwrap().east_asian_line_breaking;
    assert!(model.no_line_break_before_replaces_default);
    assert_eq!(model.no_line_break_before.concat(), "、。」");
    // The set the fixture's writer left under a localized language name
    // is the one replaced, not kept beside it.
    assert_eq!(settings.matches("<w:noLineBreaksBefore").count(), 1);

    {
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor.undo(&mut ctx).unwrap();
        executor.undo(&mut ctx).unwrap();
    }
    assert_eq!(part(&document, "word/settings.xml"), original);
    assert!(
        !document
            .document
            .as_ref()
            .unwrap()
            .east_asian_line_breaking
            .strict_first_and_last_chars
    );
}
