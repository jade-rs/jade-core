//! Block content controls the shells mark on the page — a master
//! document's subdocuments among them — found from the paragraphs in them,
//! through edits, and placed without moving the caret.

mod common;

use jade_opc::OpcPackage;

fn with_content_control() -> Vec<u8> {
    let bytes = std::fs::read(common::repo_path("public/fixtures/hyperlink.docx")).unwrap();
    let mut package = OpcPackage::open(&bytes).unwrap();
    let xml = package.part_str("word/document.xml").unwrap().unwrap();
    let body_at = xml.find("<w:body>").unwrap() + "<w:body>".len();
    let control = r#"<w:p><w:r><w:t>Before</w:t></w:r></w:p><w:sdt><w:sdtPr><w:tag w:val="outer"/></w:sdtPr><w:sdtContent><w:p><w:r><w:t>Inside</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>Last</w:t></w:r></w:p></w:sdtContent></w:sdt>"#;
    package.replace(
        "word/document.xml",
        format!("{}{control}{}", &xml[..body_at], &xml[body_at..]).into_bytes(),
    );
    package.write().unwrap()
}

fn tags(document: &common::Doc) -> Vec<Vec<String>> {
    (0..document.paragraph_count())
        .map(|paragraph| {
            document
                .word_document()
                .block_content_control_tags(paragraph)
        })
        .collect()
}

#[test]
fn paragraphs_know_the_content_control_they_are_in_through_edits() {
    let mut document = common::open_bytes(&with_content_control());
    let outer = vec!["outer".to_string()];
    let found = tags(&document);
    assert_eq!(found[0], Vec::<String>::new());
    assert_eq!(&found[1..4], &[outer.clone(), outer.clone(), outer.clone()]);
    assert!(found[4].is_empty());
    // Splitting the paragraph before it and one inside it.
    document.insert_paragraph_break(0, 2).unwrap();
    document.insert_paragraph_break(2, 3).unwrap();
    let found = tags(&document);
    assert!(found[0].is_empty() && found[1].is_empty());
    assert_eq!(
        &found[2..6],
        &[outer.clone(), outer.clone(), outer.clone(), outer]
    );
    assert!(found[6].is_empty());
}

#[test]
fn a_place_is_found_without_moving_the_caret() {
    let mut document = common::open_bytes(&with_content_control());
    let caret = document.place_caret(0, 1).unwrap();
    let inside = document.position_rect(1, 0).unwrap();
    let last = document.position_rect(3, 4).unwrap();
    assert!(inside.document_y > caret.document_y);
    assert!(last.document_y > inside.document_y);
    assert_eq!(document.selection(), None);
    let again = document.place_caret(0, 1).unwrap();
    assert_eq!(again.document_y, caret.document_y);
}
