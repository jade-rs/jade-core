//! A command that changes the page box — the page turned, another paper —
//! lays the document out and paints it as the same document opened with
//! that box: its pitch, its page count and every page's pixels. Built to
//! the old box, the renderer went on wrapping lines to its width.

mod common;
use common::{open_bytes, repo_path};
use jade_docx_headless_api::HeadlessCommand;

#[test]
fn a_page_turned_to_landscape_paints_as_one_opened_landscape() {
    // With the faces the app registers, so the reopened document draws
    // the same marks.
    let mut document =
        open_bytes(&std::fs::read(repo_path("public/fixtures/contract.docx")).unwrap());
    let portrait = (
        document.layout_page_size(),
        document.page_pitch(),
        document.page_count(),
    );
    document
        .apply_command(
            HeadlessCommand::SetPageOrientation { landscape: true },
            true,
        )
        .unwrap();
    let reopened = open_bytes(&document.export_docx().unwrap());
    let (width, height) = reopened.layout_page_size();
    assert!(width > height, "opened landscape: {width}x{height}");
    assert_eq!(document.layout_page_size(), (width, height));
    assert_eq!(document.page_pitch(), reopened.page_pitch());
    assert_eq!(document.page_count(), reopened.page_count());
    for index in 0..reopened.page_count() {
        let turned = document.page_pixels(index).unwrap();
        let opened = reopened.page_pixels(index).unwrap();
        assert_eq!((turned.1, turned.2), (opened.1, opened.2), "page {index}");
        assert!(turned.0 == opened.0, "page {index} is painted as opened");
    }

    // Undone, the pages are as they were.
    document.undo().unwrap();
    assert_eq!(
        (
            document.layout_page_size(),
            document.page_pitch(),
            document.page_count()
        ),
        portrait
    );
}
