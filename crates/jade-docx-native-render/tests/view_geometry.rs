//! A view's own page box — Web レイアウト's window-wide strips, 閲覧モード's
//! columns — in place of the sections': the text wraps to it, no header,
//! footer or page border is drawn, and the sections' pages come back after.

mod common;
use common::{open_contract, paragraph_on_first_page};
use jade_docx_editor::canvas_renderer::ViewGeometry;

fn strip(width: f64) -> ViewGeometry {
    ViewGeometry {
        page_width: width,
        page_height: 1400.0,
        margin_top: 0.0,
        margin_right: 24.0,
        margin_bottom: 0.0,
        margin_left: 24.0,
        hide_page_chrome: true,
    }
}

#[test]
fn the_text_wraps_to_the_view_and_comes_back() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    let page = document.layout_page_size();
    let pages = document.page_count();
    let left = document.place_caret(paragraph, 0).unwrap().x;

    assert!(document.set_view_geometry(Some(strip(420.0))).unwrap());
    assert_eq!(document.layout_page_size(), (420.0, 1400.0));
    let caret = document.place_caret(paragraph, 0).unwrap();
    assert!(
        caret.x < left,
        "the text starts at the view's margin: {} < {left}",
        caret.x
    );
    // Narrower: the same text takes more lines, and more strips than the
    // pages had of their height.
    let strips = document.page_count();
    assert!(
        strips as f64 * 1400.0 > pages as f64 * page.1 * 0.8,
        "{strips} strips"
    );
    // Wider: fewer.
    document.set_view_geometry(Some(strip(1600.0))).unwrap();
    assert!(document.page_count() < strips);

    assert!(document.set_view_geometry(None).unwrap());
    assert_eq!(document.layout_page_size(), page);
    assert_eq!(document.page_count(), pages);
    assert_eq!(document.place_caret(paragraph, 0).unwrap().x, left);
}

#[test]
fn edits_follow_the_view() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    document.set_view_geometry(Some(strip(500.0))).unwrap();
    document.insert_text(paragraph, 0, "web ", 1).unwrap();
    assert!(document
        .paragraph_text(paragraph)
        .unwrap()
        .starts_with("web "));
    assert_eq!(
        document.layout_page_size(),
        (500.0, 1400.0),
        "an edit keeps the view's box"
    );
    // A PDF prints the pages, not the view.
    let pdf = document.export_pdf().unwrap();
    assert!(pdf.starts_with(b"%PDF"));
}

/// アウトライン: a paragraph set in by its level, from the margin.
#[test]
fn outline_indents_set_paragraphs_in() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    document.set_view_geometry(Some(strip(800.0))).unwrap();
    let flush = document.place_caret(paragraph, 0).unwrap().x;
    let mut indents = std::collections::HashMap::new();
    indents.insert(paragraph, 96.0);
    assert!(document.set_outline_indents(Some(indents)).unwrap());
    let indented = document.place_caret(paragraph, 0).unwrap().x;
    assert!((indented - (24.0 + 96.0)).abs() < 2.0, "{indented}");
    assert!(indented > flush || (flush - 24.0).abs() > 1.0);
    document.set_outline_indents(None).unwrap();
    assert_eq!(document.place_caret(paragraph, 0).unwrap().x, flush);
}
