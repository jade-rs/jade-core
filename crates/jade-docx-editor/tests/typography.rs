//! Word's Japanese typography: the 体裁 tab on a paragraph and the
//! document's 文字体裁, written where Word writes them, read back into the
//! model the line breaker uses, and undone.

mod common;

use std::io::{Read, Write};
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

    // Defaults are explicit overrides, so inherited values cannot reappear.
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
    assert!(xml.contains(r#"<w:kinsoku w:val="1"/>"#));
    assert!(xml.contains(r#"<w:autoSpaceDE w:val="1"/>"#));
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

#[test]
fn paragraph_typography_overrides_inherited_values_after_reopening() {
    let tags = [
        "kinsoku",
        "wordWrap",
        "overflowPunct",
        "topLinePunct",
        "autoSpaceDE",
        "autoSpaceDN",
    ];
    let settings = |value| HeadlessParagraphTypography {
        kinsoku: Some(value),
        word_wrap: Some(value),
        overflow_punctuation: Some(value),
        top_line_punctuation: Some(value),
        auto_space_de: Some(value),
        auto_space_dn: Some(value),
    };
    let assert_settings = |document: &WordDocument, index, expected| {
        let actual = document.paragraph_typography(index);
        assert_eq!(actual.kinsoku, Some(expected));
        assert_eq!(actual.word_wrap, Some(expected));
        assert_eq!(actual.overflow_punctuation, Some(expected));
        assert_eq!(actual.top_line_punctuation, Some(expected));
        assert_eq!(actual.auto_space_de, Some(expected));
        assert_eq!(actual.auto_space_dn, Some(expected));
    };
    for inherited in [false, true] {
        let mut body = String::new();
        for text in ["A", "", "B"] {
            body.push_str(&format!(r#"<w:p><w:pPr><w:pStyle w:val="Child"/><w:keepNext/></w:pPr><w:r><w:rPr><w:i/></w:rPr><w:t>{text}</w:t></w:r></w:p>"#));
        }
        let inherited_xml: String = tags
            .iter()
            .map(|tag| format!(r#"<w:{tag} w:val="{}"/>"#, u8::from(inherited)))
            .collect();
        let bytes = styled_typography_docx(&body, &inherited_xml, "");
        let mut session = common::Session::new(common::load(&bytes));
        for index in 0..3 {
            assert_settings(&session.document, index, inherited);
        }
        let before = session.xml();
        session.run(Box::new(SetParagraphTypographyCommand::new(
            1,
            0,
            settings(!inherited),
        )));
        for document in [&session.document, &session.reopened()] {
            assert_settings(document, 0, !inherited);
            assert_settings(document, 1, !inherited);
            assert_settings(document, 2, inherited);
            for index in 0..3 {
                assert!(document.paragraph_pagination(index).keep_next);
            }
            assert_eq!(document.run_format_at(0, 0).italic, Some(true));
        }
        let changed = session.xml();
        for tag in tags {
            assert_eq!(
                changed
                    .matches(&format!(r#"<w:{tag} w:val="{}""#, u8::from(!inherited)))
                    .count(),
                2,
                "{tag}"
            );
        }
        session.undo();
        assert_eq!(session.xml(), before);
        for index in 0..3 {
            assert_settings(&session.document, index, inherited);
        }
        session.redo();
        assert_eq!(session.xml(), changed);
        assert_settings(&session.reopened(), 0, !inherited);
    }
}

fn styled_typography_docx(body: &str, inherited_xml: &str, child_xml: &str) -> Vec<u8> {
    let bytes = common::package(body, &[]);
    let mut input = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut output = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for index in 0..input.len() {
        let mut file = input.by_index(index).unwrap();
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
    write!(output, r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="Base"><w:pPr>{inherited_xml}</w:pPr></w:style><w:style w:type="paragraph" w:styleId="Child"><w:basedOn w:val="Base"/><w:pPr>{child_xml}</w:pPr></w:style></w:styles>"#).unwrap();
    output.finish().unwrap().into_inner()
}

#[test]
fn partial_paragraph_typography_keeps_each_inherited_option() {
    let tags = [
        "kinsoku",
        "wordWrap",
        "overflowPunct",
        "topLinePunct",
        "autoSpaceDE",
        "autoSpaceDN",
    ];
    fn values(document: &WordDocument, index: usize) -> [Option<bool>; 6] {
        let settings = document.paragraph_typography(index);
        [
            settings.kinsoku,
            settings.word_wrap,
            settings.overflow_punctuation,
            settings.top_line_punctuation,
            settings.auto_space_de,
            settings.auto_space_dn,
        ]
    }
    for inherited in [false, true] {
        let base: String = tags
            .iter()
            .map(|tag| format!(r#"<w:{tag} w:val="{}"/>"#, u8::from(inherited)))
            .collect();
        for direct in 0..tags.len() {
            let child = (direct + 1) % tags.len();
            let edited = (direct + 2) % tags.len();
            let child_xml = format!(r#"<w:{} w:val="{}"/>"#, tags[child], u8::from(!inherited));
            let body = format!(
                r#"<w:p><w:pPr><w:pStyle w:val="Child"/><w:{} w:val="{}"/><w:keepNext/></w:pPr><w:r><w:t>A</w:t></w:r></w:p><w:p><w:pPr><w:pStyle w:val="Child"/></w:pPr><w:r><w:t>B</w:t></w:r></w:p>"#,
                tags[direct],
                u8::from(!inherited)
            );
            let bytes = styled_typography_docx(&body, &base, &child_xml);
            let mut session = common::Session::new(common::load(&bytes));
            let mut expected = [Some(inherited); 6];
            expected[child] = Some(!inherited);
            let untouched = expected;
            expected[direct] = Some(!inherited);
            let original = expected;
            for doc in [&session.document, &session.reopened()] {
                assert_eq!(values(doc, 0), expected, "direct {}", tags[direct]);
                assert_eq!(values(doc, 1), untouched);
            }
            let before = session.xml();
            let mut patch = [None; 6];
            patch[edited] = Some(!inherited);
            session.run(Box::new(SetParagraphTypographyCommand::new(
                0,
                0,
                HeadlessParagraphTypography {
                    kinsoku: patch[0],
                    word_wrap: patch[1],
                    overflow_punctuation: patch[2],
                    top_line_punctuation: patch[3],
                    auto_space_de: patch[4],
                    auto_space_dn: patch[5],
                },
            )));
            expected[edited] = Some(!inherited);
            for doc in [&session.document, &session.reopened()] {
                assert_eq!(values(doc, 0), expected);
                assert_eq!(values(doc, 1), untouched);
                assert!(doc.paragraph_pagination(0).keep_next);
            }
            let changed = session.xml();
            session.undo();
            assert_eq!(session.xml(), before);
            assert_eq!(values(&session.document, 0), original);
            assert_eq!(values(&session.reopened(), 0), original);
            session.redo();
            assert_eq!(session.xml(), changed);
            assert_eq!(values(&session.reopened(), 0), expected);
        }
    }
}
