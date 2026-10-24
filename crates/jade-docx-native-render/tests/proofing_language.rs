//! 校正言語の設定: a selection's language (and スペル チェックと文章校正を
//! 行わない) set through the フォント patch, written to the run's `w:lang`
//! and `w:noProof`, and read back.

mod common;
use common::{open_contract, paragraph_on_first_page};
use jade_docx_headless_api::{HeadlessCommand, HeadlessRunFormat};

fn document_xml(document: &jade_docx_native_render::NativeDocument) -> String {
    let bytes = document.export_docx().unwrap();
    let package = jade_opc::OpcPackage::open(&bytes).unwrap();
    package.part_str("word/document.xml").unwrap().unwrap()
}

#[test]
fn a_selection_takes_a_language_and_gives_it_back() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    let set = |document: &mut jade_docx_native_render::NativeDocument,
               format: HeadlessRunFormat| {
        document
            .apply_commands(
                vec![HeadlessCommand::SetRunFormat {
                    start_paragraph: paragraph,
                    start_offset: 0,
                    end_paragraph: paragraph,
                    end_offset: 6,
                    format: Box::new(format),
                }],
                false,
            )
            .unwrap();
    };
    set(
        &mut document,
        HeadlessRunFormat {
            language: Some("en-GB".to_string()),
            no_proof: Some(true),
            ..HeadlessRunFormat::default()
        },
    );
    let xml = document_xml(&document);
    assert!(xml.contains(r#"w:val="en-GB""#), "the language is written");
    assert!(xml.contains("<w:noProof/>"), "and no proofing");
    let style = document
        .word_document()
        .paragraph(paragraph)
        .unwrap()
        .runs
        .first()
        .and_then(|run| run.properties.as_ref())
        .and_then(|props| props.language.as_deref().map(str::to_string));
    assert_eq!(style.as_deref(), Some("en-GB"));

    // Changed again: the one element follows, no second one.
    set(
        &mut document,
        HeadlessRunFormat {
            language: Some("fr-FR".to_string()),
            no_proof: Some(false),
            ..HeadlessRunFormat::default()
        },
    );
    let xml = document_xml(&document);
    assert!(xml.contains(r#"w:val="fr-FR""#));
    assert!(!xml.contains(r#"w:val="en-GB""#));
    // Undo puts it back.
    document.undo().unwrap();
    assert!(document_xml(&document).contains(r#"w:val="en-GB""#));
}
