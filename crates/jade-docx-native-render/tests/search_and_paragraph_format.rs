//! Finding text, and formatting whole paragraphs.

mod common;
use common::{open_contract, paragraph_on_first_page, Doc};

fn page_pixels(document: &Doc) -> Vec<u8> {
    document.page_pixels(0).expect("page 1 pixels").0
}

#[test]
fn a_search_finds_text_that_was_typed_in() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    const NEEDLE: &str = "検索対象マーカー";
    document
        .insert_text(paragraph, 0, NEEDLE, 1)
        .expect("insert");

    let hits = document.find(NEEDLE, false);

    assert_eq!(hits.len(), 1, "expected one hit, got {hits:?}");
    assert_eq!(hits[0].paragraph, paragraph);
    assert_eq!(hits[0].start_offset, 0);
    assert_eq!(hits[0].end_offset, NEEDLE.len());
}

/// A hit's offsets have to be usable as a selection, or finding something is
/// no use — you cannot show the user where it is.
#[test]
fn a_hit_can_be_selected() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    const NEEDLE: &str = "選択できる検索結果";
    document
        .insert_text(paragraph, 0, NEEDLE, 1)
        .expect("insert");

    let hit = *document.find(NEEDLE, false).first().expect("a hit");
    document
        .select(
            (hit.paragraph, hit.start_offset),
            (hit.paragraph, hit.end_offset),
            1,
        )
        .expect("select the hit");

    assert_eq!(document.selected_text().as_deref(), Some(NEEDLE));
}

#[test]
fn an_empty_query_finds_nothing() {
    let document = open_contract();
    assert!(document.find("", false).is_empty());
}

#[test]
fn alignment_reaches_the_page_and_can_be_undone() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    document.place_caret(paragraph, 0);
    let before = page_pixels(&document);

    document.set_alignment("center", 1).expect("centre");
    assert_ne!(
        before,
        page_pixels(&document),
        "alignment did not reach the page"
    );

    document.undo().expect("undo");
    assert_eq!(
        before,
        page_pixels(&document),
        "undo did not restore alignment"
    );
}

#[test]
fn indenting_reaches_the_page() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    document.place_caret(paragraph, 0);
    let before = page_pixels(&document);

    document.shift_indent(36.0, 1).expect("indent");

    assert_ne!(
        before,
        page_pixels(&document),
        "indent did not reach the page"
    );
}

/// Replacing one hit leaves the others alone. Replace-all is a different
/// button, and a user who picked this one meant this occurrence.
#[test]
fn one_hit_can_be_replaced_on_its_own() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    const NEEDLE: &str = "差し替え元";
    document
        .insert_text(paragraph, 0, &format!("{NEEDLE}と{NEEDLE}"), 1)
        .expect("insert");

    let hit = *document.find(NEEDLE, false).first().expect("a hit");
    document.replace_one(hit, "差し替え先", 1).expect("replace");

    let text = document.paragraph_text(paragraph).expect("paragraph text");
    assert!(
        text.starts_with(&format!("差し替え先と{NEEDLE}")),
        "expected only the first hit replaced, got {text:?}",
    );
}

/// Replace-all reports what it did, and undoes in one step. Undoing a
/// hundred-match replace one match at a time would bury whatever the user did
/// before it.
#[test]
fn replace_all_replaces_every_hit_and_undoes_in_one_step() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    const NEEDLE: &str = "全置換対象";
    let before = document.paragraph_text(paragraph).expect("paragraph text");
    document
        .insert_text(paragraph, 0, &format!("{NEEDLE}{NEEDLE}{NEEDLE}"), 1)
        .expect("insert");

    let replaced = document
        .replace_all(NEEDLE, "置換後", 1)
        .expect("replace all");

    assert_eq!(replaced, 3, "expected three replacements");
    assert!(
        document.find(NEEDLE, false).is_empty(),
        "the needle survived replace-all",
    );
    let after = document.paragraph_text(paragraph).expect("paragraph text");
    assert!(after.starts_with("置換後置換後置換後"), "got {after:?}");

    // One undo puts the three replacements back; a second undoes the insert
    // that set the paragraph up.
    document.undo().expect("undo the replace");
    assert_eq!(document.find(NEEDLE, false).len(), 3, "undo lost the hits");
    document.undo().expect("undo the insert");
    assert_eq!(
        document.paragraph_text(paragraph).expect("paragraph text"),
        before,
    );
}

/// Nothing to replace is not an error, and must not touch the document.
#[test]
fn replace_all_with_no_hits_changes_nothing() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    let before = page_pixels(&document);

    let replaced = document
        .replace_all("この文字列は文書にない", "x", 1)
        .expect("replace all");

    assert_eq!(replaced, 0);
    assert_eq!(
        document.paragraph_text(paragraph).expect("paragraph text"),
        document.paragraph_text(paragraph).expect("paragraph text"),
    );
    assert_eq!(page_pixels(&document), before, "the page changed anyway");
}

/// Redo has to put the whole group back, not the last replacement of it.
#[test]
fn redo_restores_every_replacement_of_a_replace_all() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    const NEEDLE: &str = "やり直し対象";
    document
        .insert_text(paragraph, 0, &format!("{NEEDLE}{NEEDLE}"), 1)
        .expect("insert");
    document
        .replace_all(NEEDLE, "やり直し後", 1)
        .expect("replace all");
    let replaced = document.paragraph_text(paragraph).expect("paragraph text");

    document.undo().expect("undo");
    document.redo().expect("redo");

    assert_eq!(
        document.paragraph_text(paragraph).expect("paragraph text"),
        replaced,
        "redo restored only part of the replace-all",
    );
}

/// The ribbon's paragraph controls, each of which is a different command and
/// each of which has to change the page.
#[test]
fn the_paragraph_controls_reach_the_page() {
    for (name, apply) in [
        (
            "line spacing",
            Box::new(|document: &mut Doc| document.set_line_spacing(2.0, 1))
                as Box<dyn Fn(&mut Doc) -> Result<(), String>>,
        ),
        (
            "space after",
            Box::new(|document: &mut Doc| document.set_paragraph_spacing(None, Some(24.0), 1)),
        ),
        (
            "shading",
            Box::new(|document: &mut Doc| document.set_paragraph_shading(Some("FFF2CC"), 1)),
        ),
        (
            "borders",
            Box::new(|document: &mut Doc| document.set_paragraph_borders(true, 1)),
        ),
        (
            "page colour",
            Box::new(|document: &mut Doc| document.set_page_background(Some("FFF2CC"))),
        ),
        (
            "page borders",
            Box::new(|document: &mut Doc| document.set_page_borders(true)),
        ),
    ] {
        let mut document = open_contract();
        let paragraph = paragraph_on_first_page(&mut document);
        document
            .select((paragraph, 0), (paragraph, 1), 1)
            .expect("select");
        let before = page_pixels(&document);

        apply(&mut document).unwrap_or_else(|error| panic!("{name}: {error}"));

        assert!(
            page_pixels(&document) != before,
            "{name} did not reach the page",
        );
    }
}

/// Clearing puts the page back, which is what a second click on the same
/// ribbon button has to do.
#[test]
fn shading_and_borders_can_be_cleared() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    document
        .select((paragraph, 0), (paragraph, 1), 1)
        .expect("select");
    let plain = page_pixels(&document);

    document
        .set_paragraph_shading(Some("FFF2CC"), 1)
        .expect("shade");
    document.set_paragraph_borders(true, 1).expect("box");
    assert!(page_pixels(&document) != plain, "nothing was applied");

    document.set_paragraph_borders(false, 1).expect("unbox");
    document.set_paragraph_shading(None, 1).expect("unshade");

    assert!(
        page_pixels(&document) == plain,
        "clearing the shading and the borders did not put the page back",
    );
}
