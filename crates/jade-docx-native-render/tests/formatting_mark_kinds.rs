//! Word のオプション → 表示 → 常に画面に表示する編集記号: spaces, tabs and
//! hidden text shown by kind, with the ¶ toggle off — and none of them in a
//! PDF.

mod common;
use common::{open_bytes, repo_path};
use jade_docx_editor::FormattingMarkKinds;

/// The contract with a first paragraph of a space, a tab and hidden text.
fn document() -> jade_docx_native_render::NativeDocument {
    let bytes = std::fs::read(repo_path("public/fixtures/contract.docx")).unwrap();
    let mut package = jade_opc::OpcPackage::open(&bytes).unwrap();
    let xml = package.part_str("word/document.xml").unwrap().unwrap();
    let paragraph = concat!(
        r#"<w:p><w:r><w:t xml:space="preserve">a b</w:t></w:r><w:r><w:tab/></w:r>"#,
        r#"<w:r><w:rPr><w:vanish/></w:rPr><w:t>HIDDEN</w:t></w:r>"#,
        r#"<w:r><w:t>end</w:t></w:r></w:p>"#
    );
    let at = xml.find("<w:body>").unwrap() + "<w:body>".len();
    let edited = format!("{}{paragraph}{}", &xml[..at], &xml[at..]);
    package.replace("word/document.xml", edited.into_bytes());
    let mut document = open_bytes(&package.write().unwrap());
    document.set_formatting_marks(false).unwrap();
    document
}

#[test]
fn marks_show_by_kind() {
    let mut document = document();
    let plain = document.page_pixels(0).unwrap().0;
    document
        .set_formatting_mark_kinds(FormattingMarkKinds {
            spaces: true,
            ..FormattingMarkKinds::default()
        })
        .unwrap();
    let spaces = document.page_pixels(0).unwrap().0;
    assert_ne!(plain, spaces, "the space's dot is drawn");
    document
        .set_formatting_mark_kinds(FormattingMarkKinds {
            tabs: true,
            ..FormattingMarkKinds::default()
        })
        .unwrap();
    let tabs = document.page_pixels(0).unwrap().0;
    assert_ne!(plain, tabs, "the tab's arrow is drawn");
    assert_ne!(spaces, tabs);
    document
        .set_formatting_mark_kinds(FormattingMarkKinds::default())
        .unwrap();
    assert_eq!(document.page_pixels(0).unwrap().0, plain, "and gone again");
}

#[test]
fn hidden_text_is_laid_out_when_shown() {
    let mut document = document();
    let text = document.paragraph_text(0).unwrap();
    assert!(text.contains("HIDDEN"));
    let end = text.len();
    let hidden_end = document.place_caret(0, end).unwrap().x;
    document
        .set_formatting_mark_kinds(FormattingMarkKinds {
            hidden_text: true,
            ..FormattingMarkKinds::default()
        })
        .unwrap();
    let shown_end = document.place_caret(0, end).unwrap().x;
    assert!(
        shown_end > hidden_end + 10.0,
        "the hidden word takes room: {hidden_end} → {shown_end}"
    );
}

#[test]
fn a_pdf_prints_no_marks() {
    let mut document = document();
    let before = document.export_pdf().unwrap();
    document
        .set_formatting_mark_kinds(FormattingMarkKinds::all())
        .unwrap();
    let after = document.export_pdf().unwrap();
    // The same pages: the marks are the screen's alone.
    assert_eq!(before.len(), after.len());
}

/// アンカー: a paragraph with a floating picture carries an anchor in the
/// left margin.
#[test]
fn anchors_show_in_the_margin() {
    let mut document = common::open_fixture("public/fixtures/floating-square-wrap.docx");
    document.set_formatting_marks(false).unwrap();
    let plain = document.page_pixels(0).unwrap();
    document
        .set_formatting_mark_kinds(FormattingMarkKinds {
            anchors: true,
            ..FormattingMarkKinds::default()
        })
        .unwrap();
    let anchored = document.page_pixels(0).unwrap();
    assert_ne!(plain.0, anchored.0, "the anchor is drawn");
    // Only in the left margin: everything right of it is as it was.
    let (width, scale) = (
        plain.1 as usize,
        plain.1 as f64 / document.layout_page_size().0,
    );
    let margin = (80.0 * scale) as usize;
    let changed_right_of_margin = plain
        .0
        .chunks(width * 4)
        .zip(anchored.0.chunks(width * 4))
        .any(|(a, b)| a[margin * 4..] != b[margin * 4..]);
    assert!(!changed_right_of_margin);
}
