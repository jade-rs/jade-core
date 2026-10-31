//! IME user actions across rich runs, hyperlinks, cells and offscreen pages.

use super::{assert_page_pixels, assert_same, open_bytes, package, rich_body, Doc};
use jade_docx_headless_api::HeadlessCommand;
use jade_selection::{SelectionRange, TextPosition};

fn texts(document: &Doc) -> Vec<String> {
    (0..document.paragraph_count())
        .map(|paragraph| document.paragraph_text(paragraph).unwrap())
        .collect()
}

fn byte_offset(text: &str, character: usize) -> usize {
    text.char_indices()
        .nth(character)
        .map_or(text.len(), |(offset, _)| offset)
}

fn check(
    actual: &mut Doc,
    reference: &mut Doc,
    expected: &[String],
    history: usize,
    redo: bool,
    context: &str,
) {
    for document in [&*actual, &*reference] {
        assert_eq!(texts(document), expected, "{context}: text");
        assert_eq!(document.history_len(), history, "{context}: history");
        assert_eq!(document.can_undo(), history > 0, "{context}: undo");
        assert_eq!(document.can_redo(), redo, "{context}: redo");
        assert_eq!(
            document.renderer().selection_range(),
            document.selection_range(),
            "{context}: renderer selection"
        );
    }
    assert_same(actual, reference, context);
}

#[test]
fn grouped_composition_preserves_rich_text_cells_selection_and_persistence() {
    // Cover a bold/plain boundary, a hyperlink, an empty cell, a nonempty
    // cell, and a paragraph on the last page. Character positions deliberately
    // include multibyte text; the public API receives UTF-8 byte offsets.
    for (paragraph, start_character, end_character) in
        [(1, 2, 7), (2, 0, 5), (5, 0, 0), (6, 5, 8), (9, 5, 7)]
    {
        let bytes = package(rich_body(), "");
        let mut actual = open_bytes(&bytes);
        let mut reference = open_bytes(&bytes);
        let prefix = HeadlessCommand::InsertText {
            paragraph: 0,
            offset: 0,
            text: "prefix ".into(),
        };
        actual.insert_text(0, 0, "prefix ", 1).unwrap();
        reference.apply_command(prefix, false).unwrap();
        let before = texts(&actual);
        let start = byte_offset(&before[paragraph], start_character);
        let end = byte_offset(&before[paragraph], end_character);
        let selection = Some(SelectionRange::new(
            TextPosition::new(paragraph, end),
            TextPosition::new(paragraph, start),
        ));
        for document in [&mut actual, &mut reference] {
            document.set_selection_range(selection);
            document.repaint_all().unwrap();
        }
        let mark = actual.history_len();
        assert_eq!(mark, 1);
        // ReplaceRange can invalidate downstream layout. Ask the bounded API
        // for all three pages because check() compares even offscreen carets
        // and cell metadata; a one-page pass promises only that page's layout.
        let visible_end = actual.page_count();
        let before_docx = actual.export_docx().unwrap();
        let mut marked_end = end;
        let mut expected = before.clone();
        let mut history = mark;
        for reading in ["に", "にほ", "にほ", "にほん😀", "日本"] {
            let context = format!("paragraph {paragraph}, preedit {reading:?}");
            let changed = &expected[paragraph][start..marked_end] != reading;
            expected[paragraph].replace_range(start..marked_end, reading);
            actual
                .replace_range(paragraph, start, marked_end, reading, visible_end)
                .unwrap();
            reference
                .apply_command(
                    HeadlessCommand::ReplaceRange {
                        start_paragraph: paragraph,
                        start_offset: start,
                        end_paragraph: paragraph,
                        end_offset: marked_end,
                        text: reading.into(),
                    },
                    false,
                )
                .unwrap();
            history += usize::from(changed);
            marked_end = start + reading.len();
            // Some clients merge after every update, others at commit time.
            if paragraph % 2 == 0 {
                actual.merge_history_since(mark);
                reference.merge_history_since(mark);
                history = mark + 1;
            }
            check(
                &mut actual,
                &mut reference,
                &expected,
                history,
                false,
                &context,
            );
        }
        actual.merge_history_since(mark);
        reference.merge_history_since(mark);
        let committed_selection = actual.selection_range();
        assert_eq!(
            committed_selection,
            Some(SelectionRange::collapsed(TextPosition::new(
                paragraph,
                start + "日本".len(),
            )))
        );
        check(
            &mut actual,
            &mut reference,
            &expected,
            mark + 1,
            false,
            "composition committed",
        );
        actual.undo().unwrap();
        reference.undo().unwrap();
        // The explicit ReplaceRange command supplies its own forward range
        // to history, independent of the selection direction before the call.
        let restored_selection = Some(SelectionRange::new(
            TextPosition::new(paragraph, start),
            TextPosition::new(paragraph, end),
        ));
        assert_eq!(actual.selection_range(), restored_selection);
        check(
            &mut actual,
            &mut reference,
            &before,
            mark,
            true,
            "undo composition keeps preceding edit",
        );
        let mut before_reopened = open_bytes(&before_docx);
        before_reopened.set_selection_range(restored_selection);
        before_reopened.repaint_all().unwrap();
        for page in 0..actual.page_count() {
            assert_page_pixels(
                &actual,
                &before_reopened,
                page,
                "undo restores original run and hyperlink formatting",
            );
        }
        actual.redo().unwrap();
        reference.redo().unwrap();
        assert_eq!(actual.selection_range(), committed_selection);
        check(
            &mut actual,
            &mut reference,
            &expected,
            mark + 1,
            false,
            "redo composition restores final reading",
        );
        actual.set_selection_range(None);
        actual.repaint_all().unwrap();
        let reopened = open_bytes(&actual.export_docx().unwrap());
        assert_eq!(texts(&reopened), expected);
        assert!(!reopened.can_undo());
        assert!(!reopened.can_redo());
        assert_eq!(actual.page_count(), reopened.page_count());
        for page in 0..actual.page_count() {
            assert_page_pixels(
                &actual,
                &reopened,
                page,
                "committed composition export/reopen",
            );
        }
    }
}
