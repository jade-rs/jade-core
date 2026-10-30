//! A full diagnostic must count rendered pages, not retained bitmap surfaces.
mod common;

use common::{open_fixture, repo_path};
use sha2::{Digest, Sha256};

#[test]
fn revision_view_changes_do_not_leave_stale_pages_in_full_capture() {
    let source = std::fs::read(repo_path("public/fixtures/review-kinds.docx")).unwrap();
    let word: serde_json::Value = serde_json::from_slice(
        &std::fs::read(repo_path(
            "tests/word-oracle/mac-glyph-corpus-20261019/review-kinds.json",
        ))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        word["docx_sha256"],
        format!("{:x}", Sha256::digest(&source))
    );
    // Word's frozen final-view PDF has one inserted page break. Showing
    // deleted content additionally exposes the deleted page break.
    let final_pages = word["page_count"].as_u64().unwrap() as usize;
    assert_eq!(final_pages, 2);
    let mut document = open_fixture("public/fixtures/review-kinds.docx");
    document.set_formatting_marks(false).unwrap();
    for markup in [true, false, true, false] {
        document
            .set_revision_display(markup, false, Default::default())
            .unwrap();
        let expected = final_pages + usize::from(markup);
        assert_eq!(document.page_count(), expected, "markup={markup}");
        let snapshot = document.capture_full_layout_debug().unwrap();
        assert!(snapshot.complete);
        assert_eq!(snapshot.page_count, expected, "markup={markup}");
        assert_eq!(document.renderer().page_count(), expected);
        for page in 0..expected {
            assert!(snapshot
                .body_lines
                .iter()
                .any(|line| line.page == page && !line.text.trim().is_empty()));
        }
    }
}
