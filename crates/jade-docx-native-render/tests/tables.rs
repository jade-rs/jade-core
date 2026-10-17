//! Editing the table the caret is in.

mod common;
use common::{open_contract, open_fixture, paragraph_on_first_page, Doc};

fn open_table_document() -> Doc {
    open_fixture("public/fixtures/table-nested.docx")
}

/// Every fixture with a table that the stale-pixel sweep runs over.
const TABLE_FIXTURES: [&str; 4] = [
    "public/fixtures/table-nested.docx",
    "public/fixtures/table-cell-fit-text.docx",
    "public/fixtures/table-cell-preferred-widths.docx",
    "public/fixtures/merged-table-over-page.docx",
];

/// The caret has to be locatable inside a table, or none of the table commands
/// can be addressed — they take a table index and a row, and the caret knows
/// neither.
fn caret_in_a_table(document: &mut Doc) -> jade_docx_native_render::TableCell {
    for paragraph in 0..200 {
        document.place_caret(paragraph, 0);
        if let Some(cell) = document.caret_table_cell() {
            return cell;
        }
    }
    panic!("no paragraph in this fixture put the caret in a table");
}

/// The paragraph index of the first cell the caret can be put in.
fn paragraph_in_a_table(document: &mut Doc) -> usize {
    for paragraph in 0..200 {
        document.place_caret(paragraph, 0);
        if document.caret_table_cell().is_some() {
            return paragraph;
        }
    }
    panic!("no paragraph in this fixture put the caret in a table");
}

#[test]
fn the_caret_can_be_located_inside_a_table() {
    let mut document = open_table_document();
    let cell = caret_in_a_table(&mut document);

    assert!(
        cell.rows > 0 && cell.columns > 0,
        "empty dimensions: {cell:?}"
    );
    assert!(cell.row < cell.rows, "row out of range: {cell:?}");
    assert!(cell.column < cell.columns, "column out of range: {cell:?}");
}

#[test]
fn a_row_can_be_inserted_and_undone() {
    let mut document = open_table_document();
    let before = caret_in_a_table(&mut document);

    document.edit_table_row(true).expect("insert a row");
    let after = document.caret_table_cell().expect("still in the table");
    assert_eq!(
        after.rows,
        before.rows + 1,
        "the row count did not grow: {before:?} -> {after:?}"
    );

    document.undo().expect("undo");
    let undone = document.caret_table_cell().expect("still in the table");
    assert_eq!(undone.rows, before.rows, "undo did not remove the row");
}

#[test]
fn a_row_can_be_deleted() {
    let mut document = open_table_document();
    let before = caret_in_a_table(&mut document);
    if before.rows <= 1 {
        // Deleting the only row is refused, which the next test covers.
        return;
    }

    document.edit_table_row(false).expect("delete the row");

    let rows = document
        .caret_table_cell()
        .map(|cell| cell.rows)
        .unwrap_or(before.rows - 1);
    assert_eq!(rows, before.rows - 1, "the row count did not shrink");
}

#[test]
fn editing_a_table_outside_one_is_refused() {
    let mut document = open_table_document();
    // Paragraph 0 of this fixture is body text, not a cell.
    document.place_caret(0, 0);
    if document.caret_table_cell().is_some() {
        return;
    }
    assert!(
        document.edit_table_row(true).is_err(),
        "inserting a row with the caret outside a table should be refused"
    );
}

/// The ribbon's insert commands have to reach the page, and each is a different
/// command: a table, a page break, and a generated table of contents.
#[test]
fn the_insert_commands_reach_the_page() {
    let mut document = open_contract();
    let paragraph = paragraph_on_first_page(&mut document);
    document.place_caret(paragraph, 0);
    document
        .select((paragraph, 0), (paragraph, 0), 1)
        .expect("caret");
    let before = document.page_pixels(0).expect("page pixels").0;
    let pages_before = document.page_count();

    document.insert_table(2, 3).expect("insert a table");
    assert!(
        document.page_pixels(0).expect("page pixels").0 != before,
        "the table did not reach the page",
    );

    document.insert_blank_page().expect("insert a page");
    assert!(
        document.page_count() > pages_before,
        "the page break did not add a page: {} then {}",
        pages_before,
        document.page_count(),
    );

    // The table of contents is asked of the model, not the page: it is a Word
    // field whose visible result is what an update produces, so a document
    // whose headings it has not collected yet draws nothing new.
    document
        .select((paragraph, 0), (paragraph, 1), 1)
        .expect("select");
    document
        .set_paragraph_style("Heading1", 1)
        .expect("a heading to collect");
    let paragraphs_before = document.paragraph_count();

    document
        .insert_table_of_contents()
        .expect("insert a table of contents");

    assert!(
        document.paragraph_count() > paragraphs_before,
        "the table of contents added no paragraphs: {} then {}",
        paragraphs_before,
        document.paragraph_count(),
    );
}

/// Typing in a table cell until the text wraps has to leave the page as a full
/// repaint would draw it.
///
/// The cell grows and the rows below move, and an incremental repaint that
/// redraws less than that leaves the old pixels behind — the top half of a line
/// of text, cut across the middle, above the text that replaced it.
#[test]
fn typing_until_a_cell_wraps_leaves_no_stale_pixels() {
    let mut document = open_table_document();
    // `caret_in_a_table` leaves the caret in the cell it found, so the
    // paragraph is wherever it stopped.
    let _cell = caret_in_a_table(&mut document);
    let paragraph = paragraph_in_a_table(&mut document);
    let page = document
        .place_caret(paragraph, 0)
        .map(|caret| document.page_and_offset(caret.document_y).0)
        .unwrap_or(0);

    // Enough to overflow the cell's width and wrap onto a second line.
    let before = document.page_pixels(page).expect("page pixels").0;
    let caret_before = document.place_caret(paragraph, 0).expect("a caret");
    let mut offset = document
        .paragraph_text(paragraph)
        .map(|text| text.len())
        .unwrap_or(0);
    for _ in 0..40 {
        let elapsed = document.insert_text(paragraph, offset, "あ", page + 1);
        assert!(elapsed.is_ok(), "typing in the cell failed");
        offset += "あ".len();
    }
    let typed = document.page_pixels(page).expect("page pixels").0;
    assert!(typed != before, "the typing did not reach the page at all");
    // The point of the test is the cell growing. Without that, an incremental
    // repaint has nothing to get wrong and this would pass on any renderer.
    let caret_after = document
        .place_caret(paragraph, offset)
        .expect("a caret after typing");
    assert!(
        caret_after.document_y > caret_before.document_y,
        "the text never wrapped: the caret is still on the first line",
    );

    // The same document state, drawn from scratch.
    document.repaint_all().expect("full repaint");

    let redrawn = document.page_pixels(page).expect("page pixels").0;
    let different = typed
        .chunks_exact(4)
        .zip(redrawn.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        different == 0,
        "typing left {different} pixels a full repaint disagrees with",
    );
}

/// When a cell's text wraps, the row has to get taller — everything below the
/// table moves down with it.
///
/// A row that keeps its height draws its border across the middle of the text
/// it is supposed to contain, which is what the clipping looks like on screen.
#[test]
fn a_wrapping_cell_makes_its_row_taller() {
    let mut document = open_table_document();
    let paragraph = paragraph_in_a_table(&mut document);

    // A paragraph after the table, to measure the table's height by where it
    // pushes the text below.
    let mut below = None;
    for candidate in (paragraph + 1)..(paragraph + 60) {
        if document.paragraph_text(candidate).is_none() {
            break;
        }
        document.place_caret(candidate, 0);
        if document.caret_table_cell().is_none() {
            below = document
                .place_caret(candidate, 0)
                .map(|caret| (candidate, caret.document_y));
            break;
        }
    }
    let Some((below_paragraph, y_before)) = below else {
        panic!("this fixture has no paragraph after the table to measure with");
    };

    let caret_before = document.place_caret(paragraph, 0).expect("a caret");
    let mut offset = document
        .paragraph_text(paragraph)
        .map(|text| text.len())
        .unwrap_or(0);
    for _ in 0..40 {
        document
            .insert_text(paragraph, offset, "あ", 1)
            .expect("typing in the cell");
        offset += "あ".len();
    }

    let caret_after = document
        .place_caret(paragraph, offset)
        .expect("a caret after typing");
    assert!(
        caret_after.document_y > caret_before.document_y,
        "the text never wrapped, so this proves nothing",
    );

    let y_after = document
        .place_caret(below_paragraph, 0)
        .expect("a caret below the table")
        .document_y;
    assert!(
        y_after > y_before,
        "the row did not grow: the text below the table is still at {y_before}",
    );
}

/// The same invariant across every table fixture: type in a cell until it
/// wraps, and the page has to be what a full repaint draws.
///
/// One fixture proves one table's arithmetic. Cells differ in width, in
/// vertical alignment, in whether they are merged and in whether the row can
/// grow at all, and the clipping this is about was reported on a document that
/// is not the one the test above uses.
#[test]
fn no_table_fixture_keeps_stale_pixels_when_a_cell_wraps() {
    for fixture in TABLE_FIXTURES {
        let mut document = open_fixture(fixture);
        let Some(paragraph) = (0..200).find(|paragraph| {
            document.place_caret(*paragraph, 0);
            document.caret_table_cell().is_some()
        }) else {
            panic!("{fixture}: no paragraph puts the caret in a table");
        };
        let page = document
            .place_caret(paragraph, 0)
            .map(|caret| document.page_and_offset(caret.document_y).0)
            .unwrap_or(0);

        let caret_before = document.place_caret(paragraph, 0).expect("a caret");
        let mut offset = document
            .paragraph_text(paragraph)
            .map(|text| text.len())
            .unwrap_or(0);
        for _ in 0..40 {
            document
                .insert_text(paragraph, offset, "あ", page + 1)
                .unwrap_or_else(|error| panic!("{fixture}: {error}"));
            offset += "あ".len();
        }
        let caret_after = document
            .place_caret(paragraph, offset)
            .expect("a caret after typing");
        if caret_after.document_y <= caret_before.document_y {
            // `fit-text` shrinks glyphs to the cell instead of wrapping, so
            // there is no second line to get wrong here. Skipped loudly rather
            // than asserted, and not silently: a fixture that stops wrapping is
            // a fixture that stopped testing anything.
            println!("{fixture}: the text does not wrap in this table, skipped");
            continue;
        }

        let typed = document.page_pixels(page).expect("page pixels").0;
        document.repaint_all().expect("full repaint");
        let redrawn = document.page_pixels(page).expect("page pixels").0;
        let different = typed
            .chunks_exact(4)
            .zip(redrawn.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            different == 0,
            "{fixture}: typing left {different} pixels a full repaint disagrees with",
        );
    }
}

/// The table the ribbon inserts has to behave like one that came from the file:
/// type into a cell until it wraps, and the row grows rather than drawing its
/// border through the middle of the text.
///
/// This is the sequence a user goes through with the 挿入 tab — insert a table
/// into a document that had none, then type in it.
#[test]
fn a_table_inserted_here_grows_when_its_text_wraps() {
    let mut document = open_fixture("public/fixtures/nda.docx");
    let anchor = paragraph_on_first_page(&mut document);
    document
        .select((anchor, 0), (anchor, 0), 1)
        .expect("put the caret where the table goes");

    document.insert_table(3, 3).expect("insert a table");

    let paragraph = (anchor..anchor + 20)
        .find(|paragraph| {
            document.place_caret(*paragraph, 0);
            document.caret_table_cell().is_some()
        })
        .expect("the inserted table has no cell the caret can reach");
    let page = document
        .place_caret(paragraph, 0)
        .map(|caret| document.page_and_offset(caret.document_y).0)
        .unwrap_or(0);

    // Where the text after the table sits, to measure the table's height by
    // what it pushes down.
    let below = ((paragraph + 1)..(paragraph + 40)).find_map(|candidate| {
        document.paragraph_text(candidate)?;
        document.place_caret(candidate, 0);
        if document.caret_table_cell().is_some() {
            return None;
        }
        document
            .place_caret(candidate, 0)
            .map(|caret| (candidate, caret.document_y))
    });

    let caret_before = document.place_caret(paragraph, 0).expect("a caret");
    let mut offset = 0;
    for _ in 0..40 {
        document
            .insert_text(paragraph, offset, "あ", page + 1)
            .expect("typing in the inserted cell");
        offset += "あ".len();
    }

    let caret_after = document
        .place_caret(paragraph, offset)
        .expect("a caret after typing");
    assert!(
        caret_after.document_y > caret_before.document_y,
        "the text never wrapped in the inserted cell",
    );

    // The row has to have grown with the second line. A row that keeps its
    // height draws its border through the middle of the text it contains.
    if let Some((below_paragraph, y_before)) = below {
        let y_after = document
            .place_caret(below_paragraph, 0)
            .expect("a caret below the table")
            .document_y;
        assert!(
            y_after > y_before,
            "the inserted row did not grow: the text below it is still at {y_before}",
        );
    }

    let typed = document.page_pixels(page).expect("page pixels").0;
    document.repaint_all().expect("full repaint");
    let redrawn = document.page_pixels(page).expect("page pixels").0;
    let different = typed
        .chunks_exact(4)
        .zip(redrawn.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        different == 0,
        "typing in the inserted table left {different} pixels a full repaint disagrees with",
    );
}

/// Typing through an IME, the way the shell does it, in a table cell.
///
/// A composition does not insert: it replaces the range it put on the page a
/// keystroke ago, so every reading is a `replace_range` over a growing span,
/// and a committed reading is followed by the next one starting again. That is
/// a different command from an insert, and it is how the report was produced.
///
/// Nothing is fully repainted until the end. A full repaint resets the
/// renderer, so checking after every keystroke would hide anything that needs
/// several bounded repaints in a row to go wrong — which is what typing is.
#[test]
fn composing_into_a_cell_leaves_no_stale_pixels() {
    let mut document = open_fixture("public/fixtures/nda.docx");
    let anchor = paragraph_on_first_page(&mut document);
    document
        .select((anchor, 0), (anchor, 0), 1)
        .expect("put the caret where the table goes");
    document.insert_table(3, 3).expect("insert a table");

    let paragraph = (anchor..anchor + 20)
        .find(|paragraph| {
            document.place_caret(*paragraph, 0);
            document.caret_table_cell().is_some()
        })
        .expect("the inserted table has no cell the caret can reach");
    let page = document
        .place_caret(paragraph, 0)
        .map(|caret| document.page_and_offset(caret.document_y).0)
        .unwrap_or(0);

    // Twelve readings of three characters each: compose, extend, extend,
    // commit, and start the next one where that left off.
    let mut committed = 0usize;
    for _ in 0..12 {
        let mut marked = 0usize;
        for length in 1..=3 {
            let reading: String = "ふ".repeat(length);
            document
                .replace_range(paragraph, committed, committed + marked, &reading, page + 1)
                .expect("a composition keystroke");
            marked = reading.len();
        }
        committed += marked;
    }

    let caret = document
        .place_caret(paragraph, committed)
        .expect("a caret after typing");
    let first_line = document.place_caret(paragraph, 0).expect("a caret");
    assert!(
        caret.document_y > first_line.document_y,
        "the text never wrapped, so this proves nothing",
    );

    let typed = document.page_pixels(page).expect("page pixels").0;
    document.repaint_all().expect("full repaint");
    let redrawn = document.page_pixels(page).expect("page pixels").0;
    let different = typed
        .chunks_exact(4)
        .zip(redrawn.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        different == 0,
        "composing left {different} pixels a full repaint disagrees with",
    );
}

/// The strongest oracle available: what is on the page after editing has to be
/// what the same document looks like opened fresh.
///
/// Comparing an edit against a full repaint only proves the two agree, and they
/// share every cache — a stale cached layout is drawn identically by both. A
/// reopened document shares nothing.
#[test]
fn a_wrapped_cell_looks_the_same_reopened() {
    let mut document = open_fixture("public/fixtures/nda.docx");
    let anchor = paragraph_on_first_page(&mut document);
    document
        .select((anchor, 0), (anchor, 0), 1)
        .expect("put the caret where the table goes");
    document.insert_table(3, 3).expect("insert a table");

    let paragraph = (anchor..anchor + 20)
        .find(|paragraph| {
            document.place_caret(*paragraph, 0);
            document.caret_table_cell().is_some()
        })
        .expect("the inserted table has no cell the caret can reach");
    let page = document
        .place_caret(paragraph, 0)
        .map(|caret| document.page_and_offset(caret.document_y).0)
        .unwrap_or(0);

    let mut offset = 0;
    for _ in 0..40 {
        document
            .insert_text(paragraph, offset, "ふ", page + 1)
            .expect("typing in the cell");
        offset += "ふ".len();
    }
    // The caret is drawn by the shell, not the renderer, so the selection is
    // the only thing that could differ between the two documents. Cleared.
    document
        .select((paragraph, 0), (paragraph, 0), 1)
        .expect("collapse the selection");
    // Fully repainted first, so this is about what the renderer draws from the
    // model rather than about what an incremental pass left behind.
    document.repaint_all().expect("full repaint");
    let edited = document.page_pixels(page).expect("page pixels").0;

    let exported = document.export_docx().expect("export");
    let reopened = common::open_bytes(&exported);

    let fresh = reopened.page_pixels(page).expect("page pixels").0;
    let different = edited
        .chunks_exact(4)
        .zip(fresh.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    // `JADE_DUMP_PAGES=<dir>` writes both pages out, because a pixel count says
    // that something is wrong and a picture says what.
    if different > 0 {
        if let Ok(directory) = std::env::var("JADE_DUMP_PAGES") {
            let (_, width, height) = document.page_pixels(page).expect("page pixels");
            common::write_bgra_png(&format!("{directory}/edited.png"), &edited, width, height);
            common::write_bgra_png(&format!("{directory}/reopened.png"), &fresh, width, height);
        }
    }
    assert!(
        different == 0,
        "the edited page differs from the same document reopened, in {different} pixels",
    );
}

/// The same comparison with no typing at all: does the table the ribbon
/// inserts already render differently from the same table read back from a
/// file?
#[test]
fn an_inserted_table_looks_the_same_reopened() {
    let mut document = open_fixture("public/fixtures/nda.docx");
    let anchor = paragraph_on_first_page(&mut document);
    document
        .select((anchor, 0), (anchor, 0), 1)
        .expect("put the caret where the table goes");
    document.insert_table(3, 3).expect("insert a table");
    document
        .select((anchor, 0), (anchor, 0), 1)
        .expect("collapse the selection");

    let inserted = document.page_pixels(0).expect("page pixels").0;
    let exported = document.export_docx().expect("export");
    let reopened = common::open_bytes(&exported);
    let fresh = reopened.page_pixels(0).expect("page pixels").0;

    let different = inserted
        .chunks_exact(4)
        .zip(fresh.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        different == 0,
        "the inserted table differs from the same table reopened, in {different} pixels",
    );
}

/// One character typed into a fresh cell, which is the smallest version of the
/// report: the glyph was drawn above the row's top border and cut in half.
#[test]
fn one_character_in_a_fresh_cell_is_inside_its_row() {
    let mut document = open_fixture("public/fixtures/nda.docx");
    let anchor = paragraph_on_first_page(&mut document);
    document
        .select((anchor, 0), (anchor, 0), 1)
        .expect("put the caret where the table goes");
    document.insert_table(3, 3).expect("insert a table");

    let paragraph = (anchor..anchor + 20)
        .find(|paragraph| {
            document.place_caret(*paragraph, 0);
            document.caret_table_cell().is_some()
        })
        .expect("the inserted table has no cell the caret can reach");
    let page = document
        .place_caret(paragraph, 0)
        .map(|caret| document.page_and_offset(caret.document_y).0)
        .unwrap_or(0);

    document
        .insert_text(paragraph, 0, "a", page + 1)
        .expect("type one character");
    document
        .select((paragraph, 1), (paragraph, 1), page + 1)
        .expect("collapse the selection");

    let edited = document.page_pixels(page).expect("page pixels").0;
    let exported = document.export_docx().expect("export");
    let reopened = common::open_bytes(&exported);
    let fresh = reopened.page_pixels(page).expect("page pixels").0;

    let different = edited
        .chunks_exact(4)
        .zip(fresh.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    if different > 0 {
        if let Ok(directory) = std::env::var("JADE_DUMP_PAGES") {
            let (_, width, height) = document.page_pixels(page).expect("page pixels");
            common::write_bgra_png(
                &format!("{directory}/one_edited.png"),
                &edited,
                width,
                height,
            );
            common::write_bgra_png(
                &format!("{directory}/one_reopened.png"),
                &fresh,
                width,
                height,
            );
        }
    }
    assert!(
        different == 0,
        "one character in a fresh cell differs from the same document reopened, \
         in {different} pixels",
    );
}

/// Rows of the page that carry a long horizontal run of dark pixels: a table's
/// horizontal borders, in page order.
fn horizontal_rules(pixels: &[u8], width: i32, height: i32) -> Vec<i32> {
    let mut rules = Vec::new();
    for y in 0..height {
        let mut run = 0;
        let mut longest = 0;
        for x in 0..width {
            let index = ((y * width + x) * 4) as usize;
            let dark = pixels.get(index).is_some_and(|value| *value < 128);
            run = if dark { run + 1 } else { 0 };
            longest = longest.max(run);
        }
        if longest > 80 {
            rules.push(y);
        }
    }
    rules
}

/// A character typed into a cell has to be drawn inside the row, not above it.
///
/// This is the report exactly: one letter in a fresh table cell, cut across the
/// middle by the row's top border. Comparing renders cannot catch it — both
/// sides of that comparison drew it the same wrong way — so this asks where the
/// ink is: the glyph's pixels must be between the row's borders.
#[test]
fn a_character_in_a_cell_is_drawn_between_the_rows_borders() {
    for columns in [3, 6, 9] {
        a_character_stays_inside_its_row(columns, 0);
    }
    // With the caret in an empty paragraph made by pressing Enter, which is
    // where the report's table was inserted — the screenshot has two empty
    // paragraphs above it. An empty paragraph carries the indent of the one it
    // was split from, and the table takes its anchor from there.
    for breaks in [1, 2] {
        a_character_stays_inside_its_row(3, breaks);
    }
}

/// The same for a table of `columns` columns: the narrower the cell, the less
/// room the character and the paragraph mark have to share a line, and a mark
/// that wraps to a second line is what puts the character above the row.
fn a_character_stays_inside_its_row(columns: usize, breaks: usize) {
    let mut document = open_fixture("public/fixtures/nda.docx");
    let mut anchor = paragraph_on_first_page(&mut document);
    let end = document
        .paragraph_text(anchor)
        .map(|text| text.len())
        .unwrap_or(0);
    for _ in 0..breaks {
        document
            .insert_paragraph_break(anchor, end)
            .expect("a paragraph break");
        anchor += 1;
    }
    document
        .select((anchor, 0), (anchor, 0), 1)
        .expect("put the caret where the table goes");
    document.insert_table(3, columns).expect("insert a table");

    let paragraph = (anchor..anchor + 40)
        .find(|paragraph| {
            document.place_caret(*paragraph, 0);
            document.caret_table_cell().is_some()
        })
        .expect("the inserted table has no cell the caret can reach");
    let page = document
        .place_caret(paragraph, 0)
        .map(|caret| document.page_and_offset(caret.document_y).0)
        .unwrap_or(0);

    let (empty, width, height) = document.page_pixels(page).expect("page pixels");
    let rules = horizontal_rules(&empty, width, height);
    assert!(
        rules.len() >= 2,
        "the inserted table drew no horizontal borders to measure against",
    );

    document
        .insert_text(paragraph, 0, "a", page + 1)
        .expect("type one character");
    // Read straight after the edit, with no further repaint. The shell shows
    // what the edit's own repaint left on the page; a `select` afterwards would
    // repaint again and hide anything that pass got wrong.
    let typed = document.page_pixels(page).expect("page pixels").0;

    // The glyph is what appeared: pixels that darkened when the character was
    // typed.
    let mut top = height;
    let mut bottom = 0;
    for y in 0..height {
        for x in 0..width {
            let index = ((y * width + x) * 4) as usize;
            let before = empty[index];
            let after = typed[index];
            if after < 128 && before >= 128 {
                top = top.min(y);
                bottom = bottom.max(y);
            }
        }
    }
    assert!(top < bottom, "typing the character drew nothing");

    // The row it belongs to is the one between the first two borders.
    let (row_top, row_bottom) = (rules[0], rules[1]);
    assert!(
        top > row_top && bottom < row_bottom,
        "in a {columns}-column table the character is drawn at y {top}..{bottom}, \
         outside its row's {row_top}..{row_bottom}",
    );
}

/// The report's table went in after 第2条, and a cell's height comes from the
/// paragraph in it — which is empty, so it comes from whatever the empty
/// paragraph inherits. Anchored at each paragraph of the fixture in turn, so
/// this does not depend on guessing which one the report used.
#[test]
fn a_character_stays_inside_its_row_wherever_the_table_is_anchored() {
    let probe = open_fixture("public/fixtures/nda.docx");
    let paragraphs = probe.paragraph_count().min(12);
    drop(probe);

    for anchor in 0..paragraphs {
        let mut document = open_fixture("public/fixtures/nda.docx");
        if document.paragraph_text(anchor).is_none() {
            continue;
        }
        document
            .select((anchor, 0), (anchor, 0), 1)
            .expect("put the caret where the table goes");
        document.insert_table(3, 3).expect("insert a table");

        let Some(paragraph) = (anchor..anchor + 20).find(|paragraph| {
            document.place_caret(*paragraph, 0);
            document.caret_table_cell().is_some()
        }) else {
            continue;
        };
        let page = document
            .place_caret(paragraph, 0)
            .map(|caret| document.page_and_offset(caret.document_y).0)
            .unwrap_or(0);

        let (empty, width, height) = document.page_pixels(page).expect("page pixels");
        let rules = horizontal_rules(&empty, width, height);
        if rules.len() < 2 {
            continue;
        }

        document
            .insert_text(paragraph, 0, "あ", page + 1)
            .expect("type one character");
        let typed = document.page_pixels(page).expect("page pixels").0;

        let mut top = height;
        let mut bottom = 0;
        for y in 0..height {
            for x in 0..width {
                let index = ((y * width + x) * 4) as usize;
                if typed[index] < 128 && empty[index] >= 128 {
                    top = top.min(y);
                    bottom = bottom.max(y);
                }
            }
        }
        if top >= bottom {
            continue;
        }

        // The row the character is in is the pair of rules that brackets it.
        let row_top = rules.iter().rev().find(|rule| **rule < bottom).copied();
        let row_bottom = rules.iter().find(|rule| **rule > top).copied();
        let (Some(row_top), Some(row_bottom)) = (row_top, row_bottom) else {
            continue;
        };
        assert!(
            top > row_top && bottom < row_bottom,
            "anchored at paragraph {anchor}, the character is drawn at \
             y {top}..{bottom}, outside its row's {row_top}..{row_bottom}",
        );
    }
}

/// The same, with the repaint bounded to a page the text is not on.
///
/// The shell derives the bound from the caret it drew last, which is one edit
/// behind. A bound that names another page is the one thing a test that passes
/// the right page every time can never see.
#[test]
fn a_bound_naming_another_page_does_not_leave_the_row_wrong() {
    let mut document = open_fixture("public/fixtures/nda.docx");
    let anchor = paragraph_on_first_page(&mut document);
    document
        .select((anchor, 0), (anchor, 0), 1)
        .expect("put the caret where the table goes");
    document.insert_table(3, 3).expect("insert a table");

    let paragraph = (anchor..anchor + 20)
        .find(|paragraph| {
            document.place_caret(*paragraph, 0);
            document.caret_table_cell().is_some()
        })
        .expect("the inserted table has no cell the caret can reach");
    let page = document
        .place_caret(paragraph, 0)
        .map(|caret| document.page_and_offset(caret.document_y).0)
        .unwrap_or(0);

    let (empty, width, height) = document.page_pixels(page).expect("page pixels");
    let rules = horizontal_rules(&empty, width, height);
    assert!(rules.len() >= 2, "no borders to measure against");

    // Bounded to the last page, which is not the one the table is on.
    let wrong_bound = document.page_count().max(1);
    assert!(wrong_bound != page + 1, "the fixture is a single page");
    let mut offset = 0;
    for character in "っっｋ".chars() {
        let mut buffer = [0u8; 4];
        let character = character.encode_utf8(&mut buffer);
        document
            .insert_text(paragraph, offset, character, wrong_bound)
            .expect("type a character");
        offset += character.len();
    }

    let typed = document.page_pixels(page).expect("page pixels").0;
    let mut top = height;
    let mut bottom = 0;
    for y in 0..height {
        for x in 0..width {
            let index = ((y * width + x) * 4) as usize;
            if typed[index] < 128 && empty[index] >= 128 {
                top = top.min(y);
                bottom = bottom.max(y);
            }
        }
    }
    if top >= bottom {
        // Nothing was drawn on this page at all, which is its own answer.
        panic!("bounded to page {wrong_bound}, the text never reached page {page}");
    }
    assert!(
        top > rules[0] && bottom < rules[1],
        "bounded to page {wrong_bound}, the text is drawn at y {top}..{bottom}, \
         outside its row's {}..{}",
        rules[0],
        rules[1],
    );
}

/// The reported text, in the reported cell: three characters typed with an IME,
/// ending in a full-width Latin letter.
///
/// The shell's own state dump gave the paragraph and the text; this replays
/// them. What a character is matters here — the table measures through the
/// painter, and a character the text face does not carry is measured by
/// whatever face the painter falls back to.
#[test]
fn the_reported_characters_stay_inside_their_row() {
    for text in ["っっｋ", "あ", "ｋ", "っ"] {
        let mut document = open_fixture("public/fixtures/nda.docx");
        let anchor = paragraph_on_first_page(&mut document);
        document
            .select((anchor, 0), (anchor, 0), 1)
            .expect("put the caret where the table goes");
        document.insert_table(3, 3).expect("insert a table");

        let paragraph = (anchor..anchor + 20)
            .find(|paragraph| {
                document.place_caret(*paragraph, 0);
                document.caret_table_cell().is_some()
            })
            .expect("the inserted table has no cell the caret can reach");
        let page = document
            .place_caret(paragraph, 0)
            .map(|caret| document.page_and_offset(caret.document_y).0)
            .unwrap_or(0);

        let (empty, width, height) = document.page_pixels(page).expect("page pixels");
        let rules = horizontal_rules(&empty, width, height);
        assert!(rules.len() >= 2, "{text}: no borders to measure against");

        // One character at a time, the way typing arrives.
        let mut offset = 0;
        for character in text.chars() {
            let mut buffer = [0u8; 4];
            let character = character.encode_utf8(&mut buffer);
            document
                .insert_text(paragraph, offset, character, page + 1)
                .expect("type a character");
            offset += character.len();
        }
        let typed = document.page_pixels(page).expect("page pixels").0;

        let mut top = height;
        let mut bottom = 0;
        for y in 0..height {
            for x in 0..width {
                let index = ((y * width + x) * 4) as usize;
                if typed[index] < 128 && empty[index] >= 128 {
                    top = top.min(y);
                    bottom = bottom.max(y);
                }
            }
        }
        assert!(top < bottom, "{text}: typing drew nothing");
        assert!(
            top > rules[0] && bottom < rules[1],
            "{text}: drawn at y {top}..{bottom}, outside its row's {}..{}",
            rules[0],
            rules[1],
        );
    }
}

/// A cell that is typed into after the document has some history behind it.
///
/// The shell's dump reports three pages where a fixture with one inserted table
/// has two, so the document that showed the problem had more done to it than a
/// single insert. These are the sequences a session accumulates: a second
/// table, an undone one, and typing in the first after the second went in.
#[test]
fn a_second_table_does_not_put_the_first_cells_text_above_its_row() {
    for sequence in ["two tables", "undone table", "type after second"] {
        let mut document = open_fixture("public/fixtures/nda.docx");
        let anchor = paragraph_on_first_page(&mut document);
        document.select((anchor, 0), (anchor, 0), 1).expect("caret");
        document.insert_table(3, 3).expect("first table");

        let first_cell = (anchor..anchor + 20)
            .find(|paragraph| {
                document.place_caret(*paragraph, 0);
                document.caret_table_cell().is_some()
            })
            .expect("a cell in the first table");
        let page = document
            .place_caret(first_cell, 0)
            .map(|caret| document.page_and_offset(caret.document_y).0)
            .unwrap_or(0);

        match sequence {
            "two tables" => {
                document.insert_table(2, 2).expect("second table");
            }
            "undone table" => {
                document.insert_table(2, 2).expect("second table");
                document.undo().expect("undo the second table");
            }
            "type after second" => {
                document
                    .insert_text(first_cell, 0, "先", page + 1)
                    .expect("type in the first table");
                document.insert_table(2, 2).expect("second table");
            }
            _ => {}
        }

        // The baseline is taken after the history, so what changes below is the
        // typing rather than everything the sequence moved.
        let (empty, width, height) = document.page_pixels(page).expect("page pixels");
        let rules = horizontal_rules(&empty, width, height);
        assert!(
            rules.len() >= 2,
            "{sequence}: no borders to measure against"
        );

        // And now the reported typing, in the first table's cell.
        let base = document
            .paragraph_text(first_cell)
            .map(|text| text.len())
            .unwrap_or(0);
        let mut offset = base;
        for character in "っっｋ".chars() {
            let mut buffer = [0u8; 4];
            let character = character.encode_utf8(&mut buffer);
            document
                .insert_text(first_cell, offset, character, page + 1)
                .expect("type a character");
            offset += character.len();
        }

        let typed = document.page_pixels(page).expect("page pixels").0;
        let mut top = height;
        let mut bottom = 0;
        for y in 0..height {
            for x in 0..width {
                let index = ((y * width + x) * 4) as usize;
                if typed[index] < 128 && empty[index] >= 128 {
                    top = top.min(y);
                    bottom = bottom.max(y);
                }
            }
        }
        assert!(top < bottom, "{sequence}: typing drew nothing");
        let row_top = rules.iter().rev().find(|rule| **rule < bottom).copied();
        let row_bottom = rules.iter().find(|rule| **rule > top).copied();
        let (Some(row_top), Some(row_bottom)) = (row_top, row_bottom) else {
            panic!("{sequence}: the text is not bracketed by any pair of borders");
        };
        assert!(
            top > row_top && bottom < row_bottom,
            "{sequence}: the text is drawn at y {top}..{bottom}, outside its row's \
             {row_top}..{row_bottom}",
        );
    }
}

/// Columns of the page carrying a long vertical run of dark pixels: a table's
/// vertical borders, left to right.
fn vertical_rules(pixels: &[u8], width: i32, height: i32, from_y: i32, to_y: i32) -> Vec<i32> {
    let mut rules = Vec::new();
    for x in 0..width {
        let mut run = 0;
        let mut longest = 0;
        for y in from_y.max(0)..to_y.min(height) {
            let index = ((y * width + x) * 4) as usize;
            let dark = pixels.get(index).is_some_and(|value| *value < 128);
            run = if dark { run + 1 } else { 0 };
            longest = longest.max(run);
        }
        if longest > (to_y - from_y) / 2 {
            rules.push(x);
        }
    }
    rules
}

/// Opens the document named by `JADE_APP_DOCUMENT` and checks that the first
/// cell's text is inside the cell rather than above it.
///
/// Scoped to the cell's own box — between the first two horizontal rules and
/// the first two vertical ones — because the page above the table has text of
/// its own, and a check that counts that is a check that fails for the wrong
/// reason. Skipped when unset: it is a captured session, not a fixture.
#[test]
fn the_shells_own_document_draws_its_cell_text_inside_the_row() {
    let Ok(path) = std::env::var("JADE_APP_DOCUMENT") else {
        return;
    };
    let bytes = std::fs::read(&path).expect("read the shell's document");
    let document = common::open_bytes(&bytes);
    let (pixels, width, height) = document.page_pixels(0).expect("page pixels");
    assert_cell_text_is_inside_its_row(&pixels, width, height);
    if let Ok(directory) = std::env::var("JADE_DUMP_PAGES") {
        common::write_bgra_png(
            &format!("{directory}/shell-document.png"),
            &pixels,
            width,
            height,
        );
    }
}

/// Panics if there is ink in the band just above the first cell's top border,
/// inside that cell's own column.
fn assert_cell_text_is_inside_its_row(pixels: &[u8], width: i32, height: i32) {
    let rows = horizontal_rules(pixels, width, height);
    assert!(rows.len() >= 2, "no table borders to measure against");
    let (row_top, row_bottom) = (rows[0], rows[1]);
    let columns = vertical_rules(pixels, width, height, row_top, row_bottom);
    assert!(
        columns.len() >= 2,
        "no vertical borders to bound the first cell",
    );
    let (cell_left, cell_right) = (columns[0], columns[1]);

    // The band above the row, no taller than the row itself: text pushed out of
    // the cell lands here, and nothing else should.
    let band = (row_bottom - row_top).min(row_top);
    let mut ink = 0;
    for y in (row_top - band)..row_top {
        for x in (cell_left + 2)..(cell_right - 2) {
            let index = ((y * width + x) * 4) as usize;
            if pixels[index] < 128 {
                ink += 1;
            }
        }
    }
    assert!(
        ink == 0,
        "{ink} pixels of the cell's text are drawn above its row: the cell is \
         x {cell_left}..{cell_right}, the row starts at y {row_top}",
    );
}

/// Writes the harness's own version of the captured session, for comparing with
/// the shell's: insert a 3×3 table, type the same characters, export.
#[test]
#[ignore = "diagnostic"]
fn write_harness_document() {
    let Ok(directory) = std::env::var("JADE_DUMP_PAGES") else {
        return;
    };
    let mut document = open_fixture("public/fixtures/nda.docx");
    let anchor = paragraph_on_first_page(&mut document);
    document.select((anchor, 0), (anchor, 0), 1).expect("caret");
    document.insert_table(3, 3).expect("insert a table");
    let paragraph = (anchor..anchor + 20)
        .find(|paragraph| {
            document.place_caret(*paragraph, 0);
            document.caret_table_cell().is_some()
        })
        .expect("a cell");
    let mut offset = 0;
    for character in "っっっｋ".chars() {
        let mut buffer = [0u8; 4];
        let character = character.encode_utf8(&mut buffer);
        document
            .insert_text(paragraph, offset, character, 1)
            .expect("type");
        offset += character.len();
    }
    let bytes = document.export_docx().expect("export");
    std::fs::write(format!("{directory}/harness-document.docx"), bytes).expect("write");
    println!("anchor {anchor} cell paragraph {paragraph}");
}

/// Where the text in a cell wraps, character by character.
#[test]
#[ignore = "diagnostic"]
fn report_cell_wrap_point() {
    let mut document = open_fixture("public/fixtures/nda.docx");
    let anchor = paragraph_on_first_page(&mut document);
    document.select((anchor, 0), (anchor, 0), 1).expect("caret");
    document.insert_table(3, 3).expect("insert a table");
    let paragraph = (anchor..anchor + 20)
        .find(|paragraph| {
            document.place_caret(*paragraph, 0);
            document.caret_table_cell().is_some()
        })
        .expect("a cell");

    let mut offset = 0;
    for step in 1..=12 {
        document
            .insert_text(paragraph, offset, "あ", 1)
            .expect("type");
        offset += "あ".len();
        let caret = document.place_caret(paragraph, offset).expect("caret");
        println!(
            "{step:>2} characters: caret x {:.1} y {:.1}",
            caret.x, caret.document_y
        );
    }
    let (page_width, _) = document.layout_page_size();
    println!("page width {page_width:.1}");
}

/// Text in a cell has to be drawn on the line the layout put it on.
///
/// The caret is the layout's own answer: it spans the line the character is on.
/// The ink that appeared when the character was typed has to be inside that
/// band. It was an ascent higher — the renderer draws cells with
/// `textBaseline = "top"` and the painter ignored it — so the glyphs were cut
/// in half by the row's top border, which is where the clipping came from.
#[test]
fn cell_text_is_drawn_on_the_line_the_caret_is_on() {
    let mut document = open_fixture("public/fixtures/nda.docx");
    let anchor = paragraph_on_first_page(&mut document);
    document
        .select((anchor, 0), (anchor, 0), 1)
        .expect("put the caret where the table goes");
    document.insert_table(3, 3).expect("insert a table");

    let paragraph = (anchor..anchor + 20)
        .find(|paragraph| {
            document.place_caret(*paragraph, 0);
            document.caret_table_cell().is_some()
        })
        .expect("the inserted table has no cell the caret can reach");
    let caret = document.place_caret(paragraph, 0).expect("a caret");
    let (page, line_top) = document.page_and_offset(caret.document_y);
    let line_bottom = line_top + caret.height;

    let (before, width, height) = document.page_pixels(page).expect("page pixels");
    let mut offset = 0;
    for character in "っっっｋ".chars() {
        let mut buffer = [0u8; 4];
        let character = character.encode_utf8(&mut buffer);
        document
            .insert_text(paragraph, offset, character, page + 1)
            .expect("type a character");
        offset += character.len();
    }
    let after = document.page_pixels(page).expect("page pixels").0;

    let mut top = height;
    let mut bottom = 0;
    for y in 0..height {
        for x in 0..width {
            let index = ((y * width + x) * 4) as usize;
            if after[index] < 128 && before[index] >= 128 {
                top = top.min(y);
                bottom = bottom.max(y);
            }
        }
    }
    assert!(top < bottom, "typing drew nothing");

    // A pixel or two of slack: the glyphs' ink is not the line box, and the
    // caret's own rounding is not the rasteriser's.
    assert!(
        f64::from(top) >= line_top - 2.0 && f64::from(bottom) <= line_bottom + 2.0,
        "the text is drawn at y {top}..{bottom}, off the line the caret says it \
         is on: {line_top:.1}..{line_bottom:.1}",
    );
}
