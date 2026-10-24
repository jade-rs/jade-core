//! What Word's 印刷 settings do to the document laid out for printing.

use std::path::PathBuf;

use jade_docx_editor::word::printing::{
    has_background, has_hidden_text, print_docx, selection_docx, PrintSettings,
};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../public/fixtures"
        ))
        .join(name),
    )
    .unwrap()
}

fn document_xml(docx: &[u8]) -> String {
    jade_opc::OpcPackage::open(docx)
        .unwrap()
        .part_str("word/document.xml")
        .unwrap()
        .unwrap()
}

#[test]
fn hidden_text_prints_only_when_asked() {
    let docx = fixture("char-effects.docx");
    assert!(has_hidden_text(&docx));
    let unchanged = print_docx(&docx, &PrintSettings::default()).unwrap();
    assert!(has_hidden_text(&unchanged));
    let shown = print_docx(
        &docx,
        &PrintSettings {
            hidden_text: true,
            ..PrintSettings::default()
        },
    )
    .unwrap();
    assert!(!has_hidden_text(&shown));
    assert!(document_xml(&shown).contains("hidden"));
}

#[test]
fn the_page_colour_prints_only_when_asked() {
    // A document given a page colour.
    let mut package = jade_opc::OpcPackage::open(&fixture("hello.docx")).unwrap();
    let xml = package.part_str("word/document.xml").unwrap().unwrap();
    let coloured = xml.replacen("<w:body>", r#"<w:background w:color="FFFF00"/><w:body>"#, 1);
    package.replace("word/document.xml", coloured.into_bytes());
    let docx = package.write().unwrap();
    assert!(has_background(&docx));
    let printed = print_docx(&docx, &PrintSettings::default()).unwrap();
    assert!(!has_background(&printed));
    let kept = print_docx(
        &docx,
        &PrintSettings {
            background: true,
            ..PrintSettings::default()
        },
    )
    .unwrap();
    assert!(has_background(&kept));
}

#[test]
fn a_selection_prints_on_its_own() {
    let docx = fixture("hello.docx");
    let xml = document_xml(&docx);
    assert!(xml.contains("Hello"));
    // "Hello", a page break, then "World": keep "World".
    let selected = selection_docx(&docx, (1, 0), (1, 5)).unwrap();
    let xml = document_xml(&selected);
    assert!(xml.contains("World"));
    assert!(!xml.contains("Hello"));
}
