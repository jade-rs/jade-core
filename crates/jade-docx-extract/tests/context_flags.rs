use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    assert!(path.exists(), "fixture not found: {}", path.display());
    path
}

#[test]
fn marks_table_paragraphs() {
    let paragraphs =
        jade_docx_extract::extract_from_path(fixture("table2x2.docx")).expect("extract");

    let table_texts: Vec<_> = paragraphs
        .iter()
        .filter(|p| p.in_table)
        .map(|p| p.text.as_str())
        .collect();

    assert_eq!(table_texts, vec!["Hello", "World"]);
}

#[test]
fn marks_textbox_and_shape_paragraphs() {
    let paragraphs =
        jade_docx_extract::extract_from_path(fixture("textbox.docx")).expect("extract");

    let textbox_count = paragraphs.iter().filter(|p| p.in_textbox).count();
    assert!(textbox_count >= 2);

    assert!(
        paragraphs
            .iter()
            .any(|p| p.in_textbox && p.in_shape && p.text.contains("Hello")),
        "expected a shape-backed textbox paragraph"
    );
}

/// `table_path` を "t0r1c2" のような読みやすい形に潰す。
fn path_of(para: &jade_docx_extract::Paragraph) -> String {
    para.table_path
        .iter()
        .map(|at| format!("t{}r{}c{}", at.table, at.row, at.cell))
        .collect::<Vec<_>>()
        .join("/")
}

#[test]
fn records_the_row_and_cell_of_each_table_paragraph() {
    let paragraphs =
        jade_docx_extract::extract_from_path(fixture("table2x2.docx")).expect("extract");

    let cells: Vec<_> = paragraphs
        .iter()
        .filter(|p| p.in_table)
        .map(|p| (p.text.as_str(), path_of(p)))
        .collect();

    // 2x2 の表。行と列で別々の座標が付いていること。
    assert_eq!(
        cells,
        vec![
            ("Hello", "t0r0c0".to_string()),
            ("World", "t0r1c1".to_string()),
        ]
    );
}

#[test]
fn a_nested_table_keeps_the_outer_coordinates() {
    let paragraphs =
        jade_docx_extract::extract_from_path(fixture("table-nested-single.docx")).expect("extract");

    // 入れ子の段落は外側の表から順に座標が積まれる。
    let deepest = paragraphs
        .iter()
        .filter(|p| p.in_table)
        .map(|p| p.table_path.len())
        .max()
        .unwrap_or(0);
    assert!(deepest >= 1, "表の中の段落に座標が付いていない");

    for para in paragraphs.iter().filter(|p| p.in_table) {
        assert!(
            !para.table_path.is_empty(),
            "in_table なのに座標が空: {para:?}"
        );
    }
    for para in paragraphs.iter().filter(|p| !p.in_table) {
        assert!(
            para.table_path.is_empty(),
            "表の外なのに座標が付いている: {para:?}"
        );
    }
}

#[test]
fn an_empty_table_element_does_not_swallow_the_rest_of_the_document() {
    // 中身の無い <w:tbl/> は終了タグが来ない。以前は深さを足しっぱなしにして
    // いたので、これ以降の段落がすべて表の中だと報告されていた。
    let document_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:tbl/>
    <w:p><w:r><w:t>表のあとの本文</w:t></w:r></w:p>
  </w:body>
</w:document>"#;

    let paragraphs =
        jade_docx_extract::extract_from_xml_parts(document_xml, None, None).expect("extract");

    let body: Vec<_> = paragraphs
        .iter()
        .filter(|p| !p.text.trim().is_empty())
        .collect();
    assert_eq!(body.len(), 1);
    assert_eq!(body[0].text, "表のあとの本文");
    assert!(!body[0].in_table, "表の外の段落が表の中と判定されている");
    assert!(body[0].table_path.is_empty());
}
