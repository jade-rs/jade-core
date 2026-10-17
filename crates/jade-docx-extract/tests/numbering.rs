use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    assert!(path.exists(), "fixture not found: {}", path.display());
    path
}

fn public_fixture(name: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures")
        .join(name);
    assert!(
        path.exists(),
        "public fixture not found: {}",
        path.display()
    );
    path
}

#[test]
fn numbering_handles_deletions() {
    let paragraphs =
        jade_docx_extract::extract_from_path(fixture("numbering-del.docx")).expect("extract");

    assert_eq!(paragraphs.len(), 3);

    assert_eq!(paragraphs[0].numbering_text, "1.");
    assert_eq!(paragraphs[0].accepted_numbering_text, "1.");
    assert_eq!(paragraphs[0].text, "Hello");
    assert_eq!(paragraphs[0].accepted_text, "Hello");

    assert_eq!(paragraphs[1].numbering_text, "2.");
    assert_eq!(paragraphs[1].accepted_numbering_text, "");
    assert_eq!(paragraphs[1].text, "~~World~~");
    assert_eq!(paragraphs[1].accepted_text, "");

    assert_eq!(paragraphs[2].numbering_text, "3.");
    assert_eq!(paragraphs[2].accepted_numbering_text, "2.");
    assert_eq!(paragraphs[2].text, "Foo");
    assert_eq!(paragraphs[2].accepted_text, "Foo");
}

#[test]
fn numbering_from_styles() {
    let paragraphs =
        jade_docx_extract::extract_from_path(fixture("numbering-by-style.docx")).expect("extract");

    insta::assert_yaml_snapshot!("numbering_by_style", paragraphs);
}

#[test]
fn accepted_lines_concatenate_numbering_and_text() {
    let lines =
        jade_docx_extract::extract_lines_from_path(fixture("numbering-del.docx")).expect("extract");

    assert_eq!(
        lines,
        vec!["1.Hello".to_string(), String::new(), "2.Foo".to_string()]
    );
}

#[test]
fn numbering_continues_across_num_id_with_same_abstract_without_override() {
    let paragraphs =
        jade_docx_extract::extract_from_path(public_fixture("wrong-num.docx")).expect("extract");

    let article_numbers: Vec<String> = paragraphs
        .iter()
        .filter_map(|p| {
            let normalized = normalize_numbering_text(&p.numbering_text);
            if normalized.starts_with("第") && normalized.contains('条') {
                Some(normalized)
            } else {
                None
            }
        })
        .take(2)
        .collect();

    assert_eq!(
        article_numbers,
        vec!["第1条".to_string(), "第2条".to_string()]
    );
}

fn normalize_numbering_text(raw: &str) -> String {
    raw.trim_matches(|c| c == ' ' || c == '\u{3000}')
        .chars()
        .map(|ch| match ch {
            '０'..='９' => ((ch as u32) - ('０' as u32) + ('0' as u32))
                .try_into()
                .unwrap_or(ch),
            _ => ch,
        })
        .collect()
}
