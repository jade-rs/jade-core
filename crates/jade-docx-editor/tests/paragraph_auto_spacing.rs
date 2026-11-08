//! Switching automatic spacing to fixed points must survive DOCX reopening.
mod common;
use jade_docx_editor::commands::set_paragraph_line_spacing::SetParagraphLineSpacingCommand;
use jade_docx_editor::word_document::WordDocument;
use std::io::{Read, Write};

fn package() -> Vec<u8> {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures/numbering-by-style.docx");
    let mut input = zip::ZipArchive::new(std::fs::File::open(fixture).unwrap()).unwrap();
    let mut output = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for index in 0..input.len() {
        let mut file = input.by_index(index).unwrap();
        let name = file.name().unwrap().into_owned();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        if name == "word/document.xml" {
            let mut xml = String::from_utf8(bytes).unwrap();
            let start = xml.find("<w:body>").unwrap() + "<w:body>".len();
            let end = xml.find("<w:sectPr").unwrap();
            xml.replace_range(start..end, concat!(
                r#"<w:p><w:pPr><w:pStyle w:val="Auto"/><w:keepNext/></w:pPr><w:r><w:t>Inherited</w:t></w:r></w:p>"#,
                r#"<w:p><w:pPr><w:spacing w:beforeAutospacing="1" w:afterAutospacing="1" w:custom="keep"/></w:pPr><w:r><w:t>Direct</w:t></w:r></w:p>"#,
            ));
            bytes = xml.into_bytes();
        } else if name == "word/styles.xml" {
            bytes = concat!(
                r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
                r#"<w:style w:type="paragraph" w:styleId="Normal" w:default="1"><w:name w:val="Normal"/></w:style>"#,
                r#"<w:style w:type="paragraph" w:styleId="Auto"><w:name w:val="Auto"/><w:pPr><w:spacing w:beforeAutospacing="1" w:afterAutospacing="1"/></w:pPr></w:style></w:styles>"#,
            ).as_bytes().to_vec();
        }
        output
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        output.write_all(&bytes).unwrap();
    }
    output.finish().unwrap().into_inner()
}

fn check(document: &WordDocument, index: usize, before: f32, automatic: bool) {
    let spacing = document
        .paragraph(index)
        .unwrap()
        .properties
        .as_ref()
        .unwrap()
        .spacing
        .as_ref()
        .unwrap();
    assert_eq!(spacing.before, Some(before));
    assert_eq!(spacing.before_auto, automatic);
    assert_eq!(spacing.after, Some(14.0));
    assert!(
        spacing.after_auto,
        "unmodified automatic spacing must survive"
    );
}

#[test]
fn fixing_auto_spacing_to_the_same_number_is_an_edit() {
    for index in [0, 1] {
        for points in [14.0, 6.0, 0.0] {
            let mut session = common::Session::new(common::load(&package()));
            let original = session.xml();
            session.run(Box::new(SetParagraphLineSpacingCommand::with_spacing(
                index,
                index,
                None,
                None,
                Some(points),
                None,
            )));
            check(&session.document, index, points, false);
            check(&session.reopened(), index, points, false);
            assert!(session.xml().contains(r#"w:beforeAutospacing="0""#));
            let edited = session.xml();
            session.undo();
            assert_eq!(session.xml(), original);
            check(&session.reopened(), index, 14.0, true);
            session.redo();
            assert_eq!(session.xml(), edited);
            check(&session.reopened(), index, points, false);
        }
    }
}

#[test]
fn changing_line_height_leaves_automatic_spacing_untouched() {
    for index in [0, 1] {
        let mut session = common::Session::new(common::load(&package()));
        session.run(Box::new(SetParagraphLineSpacingCommand::new(
            index,
            index,
            Some(1.5),
        )));
        check(&session.document, index, 14.0, true);
        check(&session.reopened(), index, 14.0, true);
    }
}

#[test]
fn fixing_after_spacing_preserves_automatic_before_spacing() {
    for index in [0, 1] {
        for points in [14.0, 6.0, 0.0] {
            let mut session = common::Session::new(common::load(&package()));
            let original = session.xml();
            session.run(Box::new(SetParagraphLineSpacingCommand::with_spacing(
                index,
                index,
                None,
                None,
                None,
                Some(points),
            )));
            for document in [&session.document, &session.reopened()] {
                let spacing = document
                    .paragraph(index)
                    .unwrap()
                    .properties
                    .as_ref()
                    .unwrap()
                    .spacing
                    .as_ref()
                    .unwrap();
                assert_eq!((spacing.before, spacing.before_auto), (Some(14.0), true));
                assert_eq!((spacing.after, spacing.after_auto), (Some(points), false));
            }
            assert!(session.xml().contains(r#"w:afterAutospacing="0""#));
            session.undo();
            assert_eq!(session.xml(), original);
            check(&session.reopened(), index, 14.0, true);
            session.redo();
            let reopened = session.reopened();
            let spacing = reopened
                .paragraph(index)
                .unwrap()
                .properties
                .as_ref()
                .unwrap()
                .spacing
                .as_ref()
                .unwrap();
            assert_eq!((spacing.after, spacing.after_auto), (Some(points), false));
        }
    }
}

fn inheritance_package() -> Vec<u8> {
    let mut input = zip::ZipArchive::new(std::io::Cursor::new(package())).unwrap();
    let mut output = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for index in 0..input.len() {
        let mut file = input.by_index(index).unwrap();
        let name = file.name().unwrap().into_owned();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        if name == "word/styles.xml" {
            bytes = concat!(
                r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
                r#"<w:style w:type="paragraph" w:styleId="Auto" w:default="1"><w:name w:val="Auto"/><w:pPr><w:spacing w:before="80" w:afterLines="150" w:beforeAutospacing="1" w:afterAutospacing="1"/></w:pPr></w:style>"#,
                r#"<w:style w:type="paragraph" w:styleId="Off"><w:name w:val="Off"/><w:basedOn w:val="Auto"/><w:pPr><w:spacing w:beforeAutospacing="0" w:afterAutospacing="false"/></w:pPr></w:style>"#,
                r#"<w:style w:type="paragraph" w:styleId="Again"><w:name w:val="Again"/><w:basedOn w:val="Off"/><w:pPr><w:spacing w:beforeAutospacing="on" w:afterAutospacing="true"/></w:pPr></w:style></w:styles>"#,
            ).as_bytes().to_vec();
        } else if name == "word/document.xml" {
            bytes = format!(r#"<w:document {}><w:body>
                <w:p><w:pPr><w:pStyle w:val="Off"/></w:pPr><w:r><w:t>Style off</w:t></w:r></w:p>
                <w:p><w:pPr><w:pStyle w:val="Again"/><w:spacing w:beforeAutospacing="false" w:afterAutospacing="off"/></w:pPr><w:r><w:t>Direct off</w:t></w:r></w:p>
                <w:p><w:pPr><w:spacing w:before="160"/></w:pPr><w:r><w:t>Auto inherited</w:t></w:r></w:p>
                <w:p><w:pPr><w:spacing w:beforeAutospacing="off"/></w:pPr><w:r><w:t>One side off</w:t></w:r></w:p>
                </w:body></w:document>"#, common::NAMESPACES).into_bytes();
        }
        output
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        output.write_all(&bytes).unwrap();
    }
    output.finish().unwrap().into_inner()
}

#[test]
fn explicit_auto_off_restores_authored_spacing_through_style_cascades() {
    let document = common::load(&inheritance_package());
    for (stage, document) in [&document, &common::load(&document.export_docx().unwrap())]
        .into_iter()
        .enumerate()
    {
        for index in [0, 1] {
            let spacing = document
                .paragraph(index)
                .unwrap()
                .properties
                .as_ref()
                .unwrap()
                .spacing
                .as_ref()
                .unwrap();
            assert_eq!(
                (spacing.before, spacing.before_auto),
                (Some(4.0), false),
                "paragraph {index}"
            );
            assert_eq!(
                (spacing.after_lines, spacing.after_auto),
                (Some(1.5), false),
                "paragraph {index}"
            );
        }
        let spacing = document
            .paragraph(2)
            .unwrap()
            .properties
            .as_ref()
            .unwrap()
            .spacing
            .as_ref()
            .unwrap();
        assert_eq!((spacing.before, spacing.before_auto), (Some(14.0), true));
        let spacing = document
            .paragraph(3)
            .unwrap()
            .properties
            .as_ref()
            .unwrap()
            .spacing
            .as_ref()
            .unwrap();
        assert_eq!(
            (spacing.before, spacing.before_auto),
            (Some(4.0), false),
            "stage {stage}: {spacing:?}"
        );
        assert_eq!((spacing.after, spacing.after_auto), (Some(14.0), true));
    }
}

fn restyling_package() -> Vec<u8> {
    let mut input = zip::ZipArchive::new(std::io::Cursor::new(inheritance_package())).unwrap();
    let mut output = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for index in 0..input.len() {
        let mut file = input.by_index(index).unwrap();
        let name = file.name().unwrap().into_owned();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        if name == "word/styles.xml" {
            bytes = String::from_utf8(bytes)
                .unwrap()
                .replace(
                    r#"<w:name w:val="Auto"/>"#,
                    r#"<w:name w:val="Auto"/><w:next w:val="Off"/>"#,
                )
                .replace(
                    r#"<w:spacing w:beforeAutospacing="0" w:afterAutospacing="false"/>"#,
                    r#"<w:spacing w:before="240" w:beforeAutospacing="0" w:afterAutospacing="false"/>"#,
                )
                .replace(
                    r#"<w:name w:val="Again"/>"#,
                    r#"<w:name w:val="Again"/><w:next w:val="Auto"/>"#,
                )
                .into_bytes();
        } else if name == "word/document.xml" {
            bytes = String::from_utf8(bytes).unwrap()
                .replace("<w:pPr><w:spacing", r#"<w:pPr><w:pStyle w:val="Auto"/><w:spacing"#)
                .replace("</w:body>", r#"<w:p><w:pPr><w:pStyle w:val="Auto"/><w:spacing w:beforeAutospacing="1" w:afterAutospacing="1"/></w:pPr><w:r><w:t>Direct on</w:t></w:r></w:p></w:body>"#)
                .into_bytes();
        }
        output
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        output.write_all(&bytes).unwrap();
    }
    output.finish().unwrap().into_inner()
}

fn check_restyled(
    document: &WordDocument,
    index: usize,
    before: f32,
    before_auto: bool,
    after_auto: bool,
) {
    let spacing = document
        .paragraph(index)
        .unwrap()
        .properties
        .as_ref()
        .unwrap()
        .spacing
        .as_ref()
        .unwrap();
    assert_eq!(
        (spacing.before, spacing.before_auto),
        (Some(before), before_auto),
        "paragraph {index}: {spacing:?}"
    );
    assert_eq!(
        (spacing.after, spacing.after_lines, spacing.after_auto),
        if after_auto {
            (Some(14.0), None, true)
        } else {
            (None, Some(1.5), false)
        },
        "paragraph {index}: {spacing:?}"
    );
}

#[test]
fn style_changes_keep_direct_auto_flags_and_authored_numbers() {
    use jade_docx_editor::commands::set_paragraph_style::SetParagraphStyleCommand;
    for (index, style, before, before_auto, after_auto) in [
        (1, "Auto", 4.0, false, false),
        (2, "Off", 8.0, false, false),
        (3, "Again", 12.0, false, true),
        (4, "Off", 14.0, true, true),
    ] {
        let mut session = common::Session::new(common::load(&restyling_package()));
        let original = session.xml();
        session.run(Box::new(SetParagraphStyleCommand::new(
            index,
            index,
            Some(style.into()),
        )));
        check_restyled(&session.document, index, before, before_auto, after_auto);
        check_restyled(&session.reopened(), index, before, before_auto, after_auto);
        let edited = session.xml();
        session.undo();
        assert_eq!(session.xml(), original);
        session.redo();
        assert_eq!(session.xml(), edited);
        check_restyled(&session.document, index, before, before_auto, after_auto);
        check_restyled(&session.reopened(), index, before, before_auto, after_auto);
    }
}

#[test]
fn next_style_keeps_direct_auto_flags_and_authored_numbers() {
    use jade_docx_editor::commands::insert_paragraph_break::InsertParagraphBreakCommand;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};
    for (index, before, before_auto, after_auto) in [
        (1, 4.0, false, false),
        (2, 8.0, false, false),
        (3, 12.0, false, false),
        (4, 14.0, true, true),
    ] {
        let mut session = common::Session::new(common::load(&restyling_package()));
        let original = session.xml();
        let offset = session
            .document
            .paragraph(index)
            .unwrap()
            .runs
            .iter()
            .map(|run| run.text.chars().count())
            .sum();
        session.run(Box::new(InsertParagraphBreakCommand::new(
            SelectionRange::collapsed(TextPosition::new(index, offset)),
        )));
        check_restyled(
            &session.document,
            index + 1,
            before,
            before_auto,
            after_auto,
        );
        check_restyled(
            &session.reopened(),
            index + 1,
            before,
            before_auto,
            after_auto,
        );
        let edited = session.xml();
        session.undo();
        assert_eq!(session.xml(), original);
        session.redo();
        assert_eq!(session.xml(), edited);
        check_restyled(
            &session.document,
            index + 1,
            before,
            before_auto,
            after_auto,
        );
        check_restyled(
            &session.reopened(),
            index + 1,
            before,
            before_auto,
            after_auto,
        );
    }
}
