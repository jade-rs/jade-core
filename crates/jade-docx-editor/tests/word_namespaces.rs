//! Alternate namespace bindings must survive a real editor load/edit/save.

use std::io::{Cursor, Read, Write};

use jade_docx_editor::word_document::WordDocument;

const WORD: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

fn zip(parts: &[(&str, String)]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, xml) in parts {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    zip.by_name(name).unwrap().read_to_string(&mut xml).unwrap();
    xml
}

fn alias(xml: &str, default: bool) -> String {
    let xml = xml.replace("w:", "n:").replace("xmlns:w=", "xmlns:n=");
    if default {
        xml.replace("<n:", "<")
            .replace("</n:", "</")
            .replace("xmlns:n=", &format!("xmlns:n='{WORD}' xmlns="))
    } else {
        xml
    }
}

fn fixture(default: bool) -> Vec<u8> {
    zip(&[
        ("[Content_Types].xml", r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.into()),
        ("_rels/.rels", r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="main" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.into()),
        ("word/_rels/document.xml.rels", r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="styles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="header" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/></Relationships>"#.into()),
        ("word/document.xml", alias(&format!(r#"<w:document xmlns:w='{WORD}' xmlns:x='urn:extension' xmlns:r='http://schemas.openxmlformats.org/officeDocument/2006/relationships'><w:body><w:p x:keep='untouched'><w:pPr><w:pStyle w:val='Accent'/></w:pPr><w:r><w:rPr><w:b/></w:rPr><w:t>Original</w:t></w:r><x:opaque x:type='n:custom'><x:payload>unchanged</x:payload></x:opaque></w:p><w:sectPr><w:headerReference w:type='default' r:id='header'/></w:sectPr></w:body></w:document>"#), default)),
        ("word/styles.xml", alias(&format!(r#"<w:styles xmlns:w='{WORD}'><w:style w:type='paragraph' w:styleId='Accent'><w:name w:val='Accent'/><w:pPr><w:jc w:val='right'/></w:pPr></w:style></w:styles>"#), default)),
        ("word/header1.xml", alias(&format!(r#"<w:hdr xmlns:w='{WORD}'><w:p><w:r><w:t>Header text</w:t></w:r></w:p></w:hdr>"#), default)),
        ("customXml/item1.xml", format!(r#"<n:opaque xmlns:n='{WORD}' n:keep='yes'/>"#)),
    ])
}

fn paragraph_text(document: &WordDocument) -> String {
    document
        .paragraph(0)
        .unwrap()
        .runs
        .iter()
        .map(|run| run.text.as_str())
        .collect()
}

#[test]
fn aliased_and_default_namespaces_support_edit_save_reload() {
    for (default, deferred) in [(false, false), (true, false), (false, true), (true, true)] {
        let bytes = fixture(default);
        let custom = part(&bytes, "customXml/item1.xml");
        let mut document = if deferred {
            futures::executor::block_on(WordDocument::load_for_initial_render(
                &bytes,
                &Default::default(),
            ))
            .unwrap()
        } else {
            futures::executor::block_on(WordDocument::load(&bytes, &Default::default())).unwrap()
        };
        document.ensure_roundtrip_xml().unwrap();
        assert_eq!(paragraph_text(&document), "Original");
        assert!(document.paragraph(0).unwrap().runs[0]
            .properties
            .as_ref()
            .unwrap()
            .bold
            .unwrap());
        assert!(document.parts["word/styles.xml"]
            .content
            .contains("<w:style w:type='paragraph' w:styleId='Accent'>"));
        assert!(document.parts["word/header1.xml"]
            .content
            .contains("<w:t>Header text</w:t>"));
        document
            .insert_text_among_anchors(0, 8, 0, " edited")
            .unwrap();
        let saved = document.export_docx().unwrap();
        let xml = part(&saved, "word/document.xml");
        assert!(
            xml.contains("xmlns:n="),
            "original QName alias retained: {xml}"
        );
        assert!(xml.contains("x:keep=\"untouched\""));
        assert!(xml.contains("x:type=\"n:custom\""));
        assert!(xml.contains("<x:payload>unchanged</x:payload>"));
        assert_eq!(part(&saved, "customXml/item1.xml"), custom);
        let reopened =
            futures::executor::block_on(WordDocument::load(&saved, &Default::default())).unwrap();
        assert_eq!(paragraph_text(&reopened), "Original edited");
    }
}
