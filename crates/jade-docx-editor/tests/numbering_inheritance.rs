//! Partial direct numbering keeps its level while inheriting the style's list.
mod common;

use std::io::{Read, Write};

fn package(body: &str, default_numbered: bool) -> Vec<u8> {
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
            xml.replace_range(start..end, body);
            bytes = xml.into_bytes();
        } else if name == "word/styles.xml" && default_numbered {
            let xml = String::from_utf8(bytes)
                .unwrap()
                .replace(" w:default=\"1\"", "")
                .replace(r#"w:styleId="2""#, r#"w:styleId="2" w:default="1""#);
            bytes = xml.into_bytes();
        }

        output
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        output.write_all(&bytes).unwrap();
    }
    output.finish().unwrap().into_inner()
}

#[test]
fn direct_levels_survive_style_numbering_and_reopening() {
    let mut body = String::new();
    for level in [0, 2, 8] {
        body.push_str(&format!(r#"<w:p><w:pPr><w:pStyle w:val="2"/><w:numPr><w:ilvl w:val="{level}"/></w:numPr></w:pPr><w:r><w:t>Level {level}</w:t></w:r></w:p>"#));
    }
    body.push_str(r#"<w:p><w:pPr><w:pStyle w:val="2"/><w:numPr><w:numId w:val="0"/></w:numPr></w:pPr><w:r><w:t>No list</w:t></w:r></w:p>"#);
    body.push_str(
        r#"<w:p><w:pPr><w:pStyle w:val="2"/></w:pPr><w:r><w:t>Inherited level</w:t></w:r></w:p>"#,
    );
    let document = common::load(&package(&body, false));
    let before = common::document_xml(&document);
    let reopened = common::load(&document.export_docx().unwrap());
    for document in [&document, &reopened] {
        for (index, level) in [0, 2, 8].into_iter().enumerate() {
            let numbering = document
                .paragraph(index)
                .unwrap()
                .properties
                .as_ref()
                .unwrap()
                .numbering
                .as_ref()
                .unwrap();
            assert_eq!(numbering.id.as_deref(), Some("1"));
            assert_eq!(numbering.level, Some(level), "paragraph {index}");
        }
        let cleared = document
            .paragraph(3)
            .unwrap()
            .properties
            .as_ref()
            .unwrap()
            .numbering
            .as_ref()
            .unwrap();
        assert_eq!(cleared.id.as_deref(), Some("0"));
        assert_eq!(cleared.list_id(), None);
        let inherited = document
            .paragraph(4)
            .unwrap()
            .properties
            .as_ref()
            .unwrap()
            .numbering
            .as_ref()
            .unwrap();
        assert_eq!(inherited.id.as_deref(), Some("1"));
        assert_eq!(inherited.level, Some(1));
    }
    assert_eq!(before, common::document_xml(&reopened));
}

#[test]
fn default_style_numbering_is_loaded_cleared_and_undone() {
    use jade_docx_editor::commands::set_numbering::SetNumberingCommand;
    use jade_docx_editor::word_document::WordDocument;
    fn numbering(document: &WordDocument, index: usize) -> (Option<String>, Option<u32>) {
        let n = document
            .paragraph(index)
            .unwrap()
            .properties
            .as_ref()
            .and_then(|p| p.numbering.as_ref());
        (n.and_then(|n| n.id.clone()), n.and_then(|n| n.level))
    }
    let body = concat!(
        r#"<w:p><w:r><w:t>No properties</w:t></w:r></w:p>"#,
        r#"<w:p><w:pPr><w:keepNext/></w:pPr><w:r><w:t>No style</w:t></w:r></w:p>"#,
        r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/></w:numPr></w:pPr><w:r><w:t>Direct level</w:t></w:r></w:p>"#,
        r#"<w:p><w:pPr><w:numPr><w:numId w:val="0"/></w:numPr></w:pPr><w:r><w:t>No list</w:t></w:r></w:p>"#,
    );
    let mut session = common::Session::new(common::load(&package(body, true)));
    for document in [&session.document, &session.reopened()] {
        for index in [0, 1] {
            assert_eq!(numbering(document, index), (Some("1".into()), Some(1)));
        }
        assert_eq!(numbering(document, 2), (Some("1".into()), Some(0)));
        assert_eq!(numbering(document, 3).0.as_deref(), Some("0"));
    }
    let before = session.xml();
    session.run(Box::new(SetNumberingCommand::new(0, 2, None, None)));
    for document in [&session.document, &session.reopened()] {
        for index in 0..4 {
            assert_eq!(numbering(document, index).0.as_deref(), Some("0"));
        }
    }
    let after = session.xml();
    assert_eq!(after.matches(r#"<w:numId w:val="0"/>"#).count(), 4);
    assert!(session.document.paragraph_pagination(1).keep_next);
    session.undo();
    assert_eq!(session.xml(), before);
    assert_eq!(
        numbering(&session.reopened(), 2),
        (Some("1".into()), Some(0))
    );
    session.redo();
    assert_eq!(session.xml(), after);
    assert_eq!(numbering(&session.reopened(), 0).0.as_deref(), Some("0"));
}

#[test]
fn partial_numbering_matches_word_style_inheritance() {
    // Word for Mac: list levels 2, 2, 1 (no list), 2, 2 (no list), 3.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures/partial-style-numbering.docx");
    let document = common::load(&std::fs::read(path).unwrap());
    for document in [&document, &common::load(&document.export_docx().unwrap())] {
        for index in [0, 1, 3] {
            let numbering = document
                .paragraph(index)
                .unwrap()
                .properties
                .as_ref()
                .and_then(|p| p.numbering.as_ref())
                .unwrap();
            assert_eq!(numbering.list_id(), Some("1"), "paragraph {index}");
            assert_eq!(numbering.level, Some(1), "paragraph {index}");
        }
        for index in [2, 4] {
            assert!(document
                .paragraph(index)
                .unwrap()
                .properties
                .as_ref()
                .and_then(|p| p.numbering.as_ref())
                .and_then(|n| n.list_id())
                .is_none());
        }
        assert_eq!(
            document
                .paragraph(5)
                .unwrap()
                .properties
                .as_ref()
                .and_then(|p| p.numbering.as_ref())
                .unwrap()
                .level,
            Some(2)
        );
    }
    use jade_docx_editor::commands::set_numbering::SetNumberingCommand;
    let mut session = common::Session::new(document);
    let before = session.xml();
    session.run(Box::new(SetNumberingCommand::new(3, 3, None, None)));
    let after = session.xml();
    for document in [&session.document, &session.reopened()] {
        let numbering = document
            .paragraph(3)
            .unwrap()
            .properties
            .as_ref()
            .and_then(|p| p.numbering.as_ref())
            .unwrap();
        assert_eq!(numbering.id.as_deref(), Some("0"));
        assert_eq!(numbering.list_id(), None);
        assert_eq!(
            document
                .paragraph(1)
                .unwrap()
                .properties
                .as_ref()
                .and_then(|p| p.numbering.as_ref())
                .unwrap()
                .list_id(),
            Some("1")
        );
    }
    session.undo();
    assert_eq!(session.xml(), before);
    let reopened = session.reopened();
    let numbering = reopened
        .paragraph(3)
        .unwrap()
        .properties
        .as_ref()
        .and_then(|p| p.numbering.as_ref())
        .unwrap();
    assert_eq!(numbering.list_id(), Some("1"));
    assert_eq!(numbering.level, Some(1));
    session.redo();
    assert_eq!(session.xml(), after);
    assert_eq!(
        session
            .reopened()
            .paragraph(3)
            .unwrap()
            .properties
            .as_ref()
            .and_then(|p| p.numbering.as_ref())
            .unwrap()
            .id
            .as_deref(),
        Some("0")
    );
}
