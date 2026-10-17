use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    assert!(path.exists(), "fixture not found: {}", path.display());
    path
}

#[test]
fn extracts_plain_paragraphs() {
    let paragraphs = jade_docx_extract::extract_from_path(fixture("hello.docx")).expect("extract");

    assert_eq!(paragraphs.len(), 2);

    assert_eq!(paragraphs[0].text, "Hello\n");
    assert_eq!(paragraphs[0].accepted_text, "Hello\n");
    assert_eq!(paragraphs[0].numbering_text, "");
    assert_eq!(paragraphs[0].accepted_numbering_text, "");
    assert!(!paragraphs[0].in_table);
    assert!(!paragraphs[0].in_textbox);
    assert!(!paragraphs[0].in_shape);

    assert_eq!(paragraphs[1].text, "World");
    assert_eq!(paragraphs[1].accepted_text, "World");
}
