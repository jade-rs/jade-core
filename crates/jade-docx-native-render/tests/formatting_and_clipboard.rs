//! Formatting a selection, and reading it out for the clipboard.

mod common;
use common::{open_contract, paragraph_on_first_page, Doc};
use jade_docx_native_render::CharacterFormat;

fn page_pixels(document: &Doc) -> Vec<u8> {
    document.page_pixels(0).expect("page 1 pixels").0
}

#[test]
fn bolding_a_selection_changes_the_page() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    document
        .select((paragraph, 0), (paragraph, 12), 1)
        .expect("select");
    let before = page_pixels(&document);

    document
        .format_selection(CharacterFormat::Bold, 1)
        .expect("bold");

    assert_ne!(
        before,
        page_pixels(&document),
        "bold did not reach the page"
    );
}

/// Formatting is an edit, so it belongs on the undo stack like any other.
#[test]
fn formatting_can_be_undone() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    document
        .select((paragraph, 0), (paragraph, 12), 1)
        .expect("select");
    let before = page_pixels(&document);

    document
        .format_selection(CharacterFormat::Bold, 1)
        .expect("bold");
    assert!(document.can_undo());
    document.undo().expect("undo");

    assert_eq!(
        before,
        page_pixels(&document),
        "undoing the format did not restore the page"
    );
}

/// A second format has to apply to the same run: the command consumes the
/// selection, and without putting it back the next one formats nothing.
#[test]
fn a_selection_survives_being_formatted() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    document
        .select((paragraph, 0), (paragraph, 12), 1)
        .expect("select");

    document
        .format_selection(CharacterFormat::Bold, 1)
        .expect("bold");
    assert_eq!(
        document.selection(),
        Some(((paragraph, 0), (paragraph, 12))),
        "the selection was lost when it was formatted"
    );
    document
        .format_selection(CharacterFormat::Italic, 1)
        .expect("italic on the same run");
}

#[test]
fn the_selection_reads_back_as_text() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    let text = document.paragraph_text(paragraph).expect("text");
    let cut: usize = text.chars().take(6).map(char::len_utf8).sum();

    document
        .select((paragraph, 0), (paragraph, cut), 1)
        .expect("select");

    assert_eq!(
        document.selected_text().as_deref(),
        Some(&text[..cut]),
        "the clipboard text does not match the selection"
    );
}

/// A selection spanning paragraphs joins with newlines, which is what pasting
/// it back turns into paragraph breaks.
#[test]
fn a_multi_paragraph_selection_joins_with_newlines() {
    let mut document = open_contract();
    let first = paragraph_on_first_page(&mut document);
    let second = first + 1;
    let head = document.paragraph_text(first).expect("first");
    let tail = document.paragraph_text(second).expect("second");

    document
        .select((first, 0), (second, tail.len()), 1)
        .expect("select across paragraphs");

    let copied = document.selected_text().expect("selected text");
    assert!(
        copied.contains('\n'),
        "a selection across paragraphs came back as one line"
    );
    assert!(copied.starts_with(&head) && copied.ends_with(&tail));
}

/// The toolbar's font controls have to reach the page, and each of them is a
/// different command — a run-level one that consumes the selection, which is
/// why the selection is put back afterwards.
#[test]
fn the_font_controls_reach_the_page_and_keep_the_selection() {
    for (name, apply) in [
        (
            "size",
            Box::new(|document: &mut Doc| document.set_font_size(24.0, 1))
                as Box<dyn Fn(&mut Doc) -> Result<(), String>>,
        ),
        (
            "highlight",
            Box::new(|document: &mut Doc| document.set_highlight_color("yellow", 1)),
        ),
        (
            "strikethrough",
            Box::new(|document: &mut Doc| {
                document.format_selection(CharacterFormat::Strikethrough, 1)
            }),
        ),
        (
            "superscript",
            Box::new(|document: &mut Doc| {
                document.format_selection(CharacterFormat::Superscript, 1)
            }),
        ),
    ] {
        let mut document = open_contract();
        let paragraph = paragraph_on_first_page(&mut document);
        let text = document.paragraph_text(paragraph).expect("paragraph text");
        let end = text.char_indices().nth(4).expect("a fifth character").0;
        document
            .select((paragraph, 0), (paragraph, end), 1)
            .expect("select");
        let before = document.page_pixels(0).expect("page pixels").0;

        apply(&mut document).unwrap_or_else(|error| panic!("{name}: {error}"));

        // Compared as a bool: a page is megabytes, and a failing assert_ne!
        // would print both copies of it.
        assert!(
            document.page_pixels(0).expect("page pixels").0 != before,
            "{name} did not reach the page",
        );
        assert!(
            document.selection().is_some(),
            "{name} left no selection for the next command to apply to",
        );
    }
}

/// Text colour is drawn, not just recorded.
///
/// It has to be checked with the selection gone: the renderer paints the
/// highlight itself, and the highlight is over the very run whose colour
/// changed.
#[test]
fn a_font_colour_is_drawn_once_the_selection_is_gone() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    let text = document.paragraph_text(paragraph).expect("paragraph text");
    let end = text.char_indices().nth(4).expect("a fifth character").0;
    let plain = document.page_pixels(0).expect("page pixels").0;

    document
        .select((paragraph, 0), (paragraph, end), 1)
        .expect("select");
    document.set_font_color("C00000", 1).expect("colour");
    // Collapsed, so the highlight is no longer on top of the coloured run.
    document
        .select((paragraph, 0), (paragraph, 0), 1)
        .expect("deselect");

    assert_eq!(
        document
            .run_style_at(paragraph, 0)
            .and_then(|style| style.color),
        Some("#C00000".to_string()),
        "the colour did not reach the run",
    );
    assert!(
        document.page_pixels(0).expect("page pixels").0 != plain,
        "the colour reached the run but not the page",
    );
}

/// A family change need not move a pixel — every family is aliased to the one
/// bundled face here — so this asks the model instead of the page.
#[test]
fn the_font_family_reaches_the_run() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    let text = document.paragraph_text(paragraph).expect("paragraph text");
    let end = text.char_indices().nth(4).expect("a fifth character").0;
    document
        .select((paragraph, 0), (paragraph, end), 1)
        .expect("select");

    document
        .set_font_family("Times New Roman", 1)
        .expect("family");

    let style = document.run_style_at(paragraph, 0).expect("a run style");
    assert_eq!(style.font_family.as_deref(), Some("Times New Roman"));
}

/// Clearing formatting is the one that has to undo the others.
///
/// It removes the run's own formatting too, as Word's Ctrl+Space does: the
/// contract's title sets its size directly, and cleared it takes its style's.
/// So the run cleared once is the baseline bold has to come back to.
#[test]
fn clearing_formatting_puts_the_run_back() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    let text = document.paragraph_text(paragraph).expect("paragraph text");
    let end = text.char_indices().nth(4).expect("a fifth character").0;
    document
        .select((paragraph, 0), (paragraph, end), 1)
        .expect("select");
    document
        .format_selection(CharacterFormat::Clear, 1)
        .expect("clear");
    let plain = document.page_pixels(0).expect("page pixels").0;

    document
        .format_selection(CharacterFormat::Bold, 1)
        .expect("bold");
    assert!(
        document.page_pixels(0).expect("page pixels").0 != plain,
        "bold did not reach the page",
    );

    document
        .format_selection(CharacterFormat::Clear, 1)
        .expect("clear");

    assert!(
        document.page_pixels(0).expect("page pixels").0 == plain,
        "clearing did not put the run back the way it was",
    );
}

/// A style and a list are paragraph-level, so they apply to the paragraphs the
/// selection touches rather than to a run.
///
/// The style is asserted on the model: what a style *looks* like depends on
/// whether the document defines it, and applying one it does not define is a
/// paragraph pointing at a missing style — which is what Word stores too, and
/// draws no differently.
#[test]
fn a_paragraph_style_and_a_list_are_applied() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    document
        .select((paragraph, 0), (paragraph, 1), 1)
        .expect("select");

    document.set_paragraph_style("Heading1", 1).expect("style");
    assert_eq!(
        document.paragraph_style(paragraph).as_deref(),
        Some("Heading1"),
        "the style did not reach the paragraph",
    );

    let styled = document.page_pixels(0).expect("page pixels").0;
    document.set_numbering(Some("1"), 1).expect("numbering");
    assert!(
        document.page_pixels(0).expect("page pixels").0 != styled,
        "the list did not reach the page",
    );
}
