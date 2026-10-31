//! Compare real incremental edits with commands followed by a full repaint.
//! The reference never arms the completed-pagination typing shortcut.

mod common;
#[path = "typing_differential/composition_history.rs"]
mod composition_history;
#[path = "typing_differential/formatting_model.rs"]
mod formatting_model;
#[path = "typing_differential/history_model.rs"]
mod history_model;
#[path = "typing_differential/mixed_model.rs"]
mod mixed_model;
#[path = "typing_differential/note_history_model.rs"]
mod note_history_model;
#[path = "typing_differential/numbering_style_model.rs"]
mod numbering_style_model;
#[path = "typing_differential/paragraph_format_model.rs"]
mod paragraph_format_model;
#[path = "typing_differential/paragraph_model.rs"]
mod paragraph_model;
#[path = "typing_differential/revision_history_model.rs"]
mod revision_history_model;
#[path = "typing_differential/search_model.rs"]
mod search_model;
#[path = "typing_differential/story_history_model.rs"]
mod story_history_model;
#[path = "typing_differential/table_history_model.rs"]
mod table_history_model;
#[path = "typing_differential/viewport_model.rs"]
mod viewport_model;

use common::{open_bytes, Doc};
use jade_docx_headless_api::{HeadlessCommand, HeadlessDeleteDirection};
use std::io::{Cursor, Write};

const COORD_EPSILON: f64 = 1e-7;

fn package(body: &str, section: &str) -> Vec<u8> {
    let document = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body>{body}<w:sectPr><w:pgSz w:w="7200" w:h="7200"/><w:pgMar w:top="720" w:right="720" w:bottom="720" w:left="720"/>{section}</w:sectPr></w:body></w:document>"#
    );
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, xml) in [
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        ),
        (
            "word/_rels/document.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="link" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/typing" TargetMode="External"/></Relationships>"#,
        ),
        ("word/document.xml", document.as_str()),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn rich_body() -> &'static str {
    r#"
    <w:p><w:r><w:t>first 日本語 alpha</w:t></w:r></w:p>
    <w:p><w:r><w:rPr><w:b/></w:rPr><w:t>bold</w:t></w:r><w:r><w:t xml:space="preserve"> plain é 😀</w:t></w:r></w:p>
    <w:p><w:hyperlink r:id="link"><w:r><w:t>first link</w:t></w:r></w:hyperlink></w:p>
    <w:p/>
    <w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:r><w:t>second page</w:t></w:r></w:p>
    <w:tbl><w:tblPr><w:tblW w:w="5760" w:type="dxa"/><w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid><w:gridCol w:w="2880"/><w:gridCol w:w="2880"/></w:tblGrid>
    <w:tr><w:tc><w:tcPr><w:tcW w:w="2880" w:type="dxa"/><w:tcMar><w:left w:w="360" w:type="dxa"/></w:tcMar></w:tcPr><w:p/></w:tc>
    <w:tc><w:tcPr><w:tcW w:w="2880" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>cell 日本語</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
    <w:p><w:hyperlink r:id="link"><w:r><w:t>second link</w:t></w:r></w:hyperlink></w:p>
    <w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:r><w:t>third page</w:t></w:r></w:p>
    <w:p><w:r><w:t>last 日本語 omega</w:t></w:r></w:p>
    "#
}

fn assert_same(actual: &mut Doc, reference: &mut Doc, context: &str) {
    assert_eq!(
        actual.paragraph_count(),
        reference.paragraph_count(),
        "{context}: paragraphs"
    );
    assert_eq!(
        actual.page_count(),
        reference.page_count(),
        "{context}: pages"
    );
    assert_eq!(
        actual.selection_range(),
        reference.selection_range(),
        "{context}: selection"
    );
    assert_eq!(
        actual.can_undo(),
        reference.can_undo(),
        "{context}: undo availability"
    );
    assert_eq!(
        actual.can_redo(),
        reference.can_redo(),
        "{context}: redo availability"
    );
    for paragraph in 0..reference.paragraph_count() {
        let text = reference.paragraph_text(paragraph).unwrap();
        assert_eq!(
            actual.paragraph_text(paragraph).as_deref(),
            Some(text.as_str()),
            "{context}: paragraph {paragraph}"
        );
        // Include every valid UTF-8 byte boundary, including the empty/end
        // caret, rather than only checking the edited paragraph's first line.
        for offset in text
            .char_indices()
            .map(|(offset, _)| offset)
            .chain([text.len()])
        {
            let expected = reference.position_rect(paragraph, offset);
            match (actual.position_rect(paragraph, offset), expected) {
                (Some(actual), Some(expected)) => {
                    // Cached Y translations can round differently from a
                    // fresh layout. Keep this far below a raster subpixel;
                    // the page pixels below still have to match exactly.
                    for (name, actual, expected) in [
                        ("x", actual.x, expected.x),
                        ("y", actual.document_y, expected.document_y),
                        ("width", actual.width, expected.width),
                        ("height", actual.height, expected.height),
                    ] {
                        assert!(
                            (actual - expected).abs() < COORD_EPSILON,
                            "{context}: caret {paragraph}:{offset} {name}: {actual} vs {expected}"
                        );
                    }
                }
                (actual, expected) => assert_eq!(
                    actual, expected,
                    "{context}: caret {paragraph}:{offset} availability"
                ),
            }
            if let Some(rect) = expected {
                let point = (rect.x + 0.25, rect.document_y + rect.height / 2.0);
                assert_eq!(
                    actual.position_at(point.0, point.1),
                    reference.position_at(point.0, point.1),
                    "{context}: hit {paragraph}:{offset}"
                );
            }
        }
    }
    for page in 0..reference.page_count() {
        assert_page_pixels(actual, reference, page, context);
    }
    let metadata = |document: &Doc| {
        serde_json::to_value((
            document.renderer().table_resize_metadata(),
            document.renderer().cell_layouts_debug_ref(),
            document.renderer().hyperlink_regions(),
        ))
        .unwrap()
    };
    assert_metadata(&metadata(actual), &metadata(reference), context);
}

fn assert_metadata(actual: &serde_json::Value, expected: &serde_json::Value, context: &str) {
    use serde_json::Value;
    match (actual, expected) {
        (Value::Number(actual), Value::Number(expected))
            if actual.is_f64() && expected.is_f64() =>
        {
            assert!(
                (actual.as_f64().unwrap() - expected.as_f64().unwrap()).abs() < COORD_EPSILON,
                "{context}: {actual} vs {expected}"
            );
        }
        (Value::Array(actual), Value::Array(expected)) => {
            assert_eq!(actual.len(), expected.len(), "{context}: metadata length");
            for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                assert_metadata(actual, expected, &format!("{context}[{index}]"));
            }
        }
        (Value::Object(actual), Value::Object(expected)) => {
            assert_eq!(
                actual.keys().collect::<Vec<_>>(),
                expected.keys().collect::<Vec<_>>(),
                "{context}: metadata fields"
            );
            for (key, expected) in expected {
                assert_metadata(&actual[key], expected, &format!("{context}.{key}"));
            }
        }
        // IDs, indices, text and array ordering remain exact; only floating
        // coordinates get the same tolerance as the caret comparison above.
        _ => assert_eq!(actual, expected, "{context}: metadata"),
    }
}

fn assert_page_pixels(actual: &Doc, reference: &Doc, page: usize, context: &str) {
    let (actual, width, height) = actual.page_pixels(page).unwrap();
    let (expected, expected_width, expected_height) = reference.page_pixels(page).unwrap();
    assert_eq!(
        (width, height),
        (expected_width, expected_height),
        "{context}: page {page} size"
    );
    assert_eq!(
        actual.len(),
        expected.len(),
        "{context}: pixel buffer length"
    );
    assert_eq!(actual.len(), width as usize * height as usize * 4);
    if actual == expected {
        return;
    }
    let first_difference = actual
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .position(|(a, b)| a != b);
    assert!(
        first_difference.is_none(),
        "{context}: page {page} pixels, first difference {:?}",
        first_difference.map(|index| (
            index % width as usize,
            index / width as usize,
            &actual[4 * index..4 * index + 4],
            &expected[4 * index..4 * index + 4]
        ))
    );
}

fn apply_incremental(document: &mut Doc, command: &HeadlessCommand, visible_end: usize) {
    match command {
        HeadlessCommand::InsertText {
            paragraph,
            offset,
            text,
        } => {
            document
                .insert_text(*paragraph, *offset, text, visible_end)
                .unwrap();
        }
        HeadlessCommand::DeleteText {
            paragraph,
            offset,
            direction,
            count,
        } => {
            document
                .delete_text(
                    *paragraph,
                    *offset,
                    matches!(direction, HeadlessDeleteDirection::Backward),
                    *count,
                    visible_end,
                )
                .unwrap();
        }
        HeadlessCommand::ReplaceRange {
            start_paragraph,
            start_offset,
            end_paragraph,
            end_offset,
            text,
        } => {
            assert_eq!(start_paragraph, end_paragraph);
            document
                .replace_range(
                    *start_paragraph,
                    *start_offset,
                    *end_offset,
                    text,
                    visible_end,
                )
                .unwrap();
        }
        HeadlessCommand::InsertParagraphBreak {
            paragraph,
            offset,
            count,
        } => {
            assert_eq!(*count, 1);
            document
                .insert_paragraph_break(*paragraph, *offset)
                .unwrap();
        }
        _ => panic!("unsupported test command: {command:?}"),
    }
}

// Small deterministic PRNG: failures reproduce from the seed and step printed
// in every assertion, without a runtime dependency or an unbounded fuzz run.
fn next(state: &mut u64) -> usize {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state >> 32) as usize
}

fn mixed_edits(seed: u64) {
    let bytes = package(rich_body(), "");
    let mut actual = open_bytes(&bytes);
    let mut reference = open_bytes(&bytes);
    assert_eq!(actual.page_count(), 3);
    assert!(!actual.renderer().hyperlink_regions().is_empty());
    assert!(!actual.renderer().cell_layouts_debug_ref().is_empty());
    assert_same(&mut actual, &mut reference, "initial");
    let mut state = seed;
    let mut preserved = 0;
    let mut fallback = 0;
    for step in 0..48 {
        let context = format!("seed={seed:#x}, step={step}");
        if step % 12 == 6 {
            actual.undo().unwrap();
            reference.undo().unwrap();
        } else if step % 12 == 7 {
            actual.redo().unwrap();
            reference.redo().unwrap();
        } else {
            let paragraph = next(&mut state) % actual.paragraph_count();
            let text = actual.paragraph_text(paragraph).unwrap();
            let boundaries: Vec<_> = text
                .char_indices()
                .map(|(offset, _)| offset)
                .chain([text.len()])
                .collect();
            let offset = boundaries[next(&mut state) % boundaries.len()];
            let command = match step % 12 {
                3 => HeadlessCommand::ReplaceRange {
                    start_paragraph: paragraph,
                    start_offset: offset,
                    end_paragraph: paragraph,
                    end_offset: text.len(),
                    text: "変換済み".into(),
                },
                4 | 10 => HeadlessCommand::DeleteText {
                    paragraph,
                    offset,
                    direction: if step % 12 == 4 {
                        HeadlessDeleteDirection::Backward
                    } else {
                        HeadlessDeleteDirection::Forward
                    },
                    count: 1,
                },
                8 => HeadlessCommand::InsertParagraphBreak {
                    paragraph,
                    offset,
                    count: 1,
                },
                kind => HeadlessCommand::InsertText {
                    paragraph,
                    offset,
                    text: match kind {
                        0 => "a".into(),
                        1 => "日本語".into(),
                        2 => "😀e\u{301}".into(),
                        5 => "折り返し long text ".repeat(16),
                        9 => String::new(),
                        _ => "\tZ".into(),
                    },
                },
            };
            // Generous enough to include newly created pages, so all pixels
            // have a defined expectation even after a wrapping edit.
            apply_incremental(&mut actual, &command, 100);
            if matches!(command, HeadlessCommand::InsertText { .. }) {
                if actual.renderer().last_completed_pagination_edit_preserved() {
                    preserved += 1;
                } else {
                    fallback += 1;
                }
            }
            reference.apply_command(command, false).unwrap();
        }
        assert_same(&mut actual, &mut reference, &context);
    }
    assert!(
        preserved > 0,
        "seed {seed:#x}: shortcut was never exercised"
    );
    assert!(fallback > 0, "seed {seed:#x}: fallback was never exercised");
    // A new renderer also checks the final state independently of both
    // sessions' caches. Selection/history are session state, not DOCX parts.
    let mut reopened = open_bytes(&actual.export_docx().unwrap());
    reopened.set_selection_range(actual.selection_range());
    for page in 0..actual.page_count() {
        assert!(
            actual.page_pixels(page).unwrap() == reopened.page_pixels(page).unwrap(),
            "seed {seed:#x}: reopened page {page}"
        );
    }
    assert_eq!(actual.page_count(), reopened.page_count());
}

#[test]
fn mixed_unicode_edits_match_full_repaint_seed_one() {
    mixed_edits(0x1234_5678_9abc_def0);
}

#[test]
fn mixed_unicode_edits_match_full_repaint_seed_two() {
    mixed_edits(0xcafe_f00d_9876_5432);
}

#[test]
fn mixed_unicode_edits_match_full_repaint_seed_three() {
    mixed_edits(0xdead_beef_1122_3344);
}

#[test]
fn stable_typing_across_visible_pages_keeps_all_pixels_and_hit_targets() {
    let bytes = package(rich_body(), "");
    let mut actual = open_bytes(&bytes);
    let mut reference = open_bytes(&bytes);
    // Move both forward and backward through the document. Every edit has
    // its own visible bound, leaving the other page bitmaps untouched.
    for (step, paragraph) in [0, 4, 8, 4, 0, 8].into_iter().enumerate() {
        let rect = actual.position_rect(paragraph, 0).unwrap();
        let visible_end = actual.page_and_offset(rect.document_y).0 + 1;
        let command = HeadlessCommand::InsertText {
            paragraph,
            offset: 0,
            text: "a".into(),
        };
        apply_incremental(&mut actual, &command, visible_end);
        assert!(
            actual.renderer().last_completed_pagination_edit_preserved(),
            "step {step}: shortcut was not exercised"
        );
        reference.apply_command(command, false).unwrap();
        assert_same(
            &mut actual,
            &mut reference,
            &format!("visible-page step {step}"),
        );
    }
}

#[test]
fn typing_in_centered_and_bottom_aligned_sections_matches_full_repaint() {
    for alignment in ["center", "bottom"] {
        for visible_end in [1, 100] {
            let bytes = package(rich_body(), &format!(r#"<w:vAlign w:val="{alignment}"/>"#));
            let mut actual = open_bytes(&bytes);
            let mut reference = open_bytes(&bytes);
            // A full repaint must also be independent of the page where the
            // previous walk ended, even before the first edit.
            reference.repaint_all().unwrap();
            assert_same(&mut actual, &mut reference, &format!("{alignment} initial"));
            for (step, text) in ["あ".to_string(), "折り返し ".repeat(35), "a".to_string()]
                .into_iter()
                .enumerate()
            {
                let command = HeadlessCommand::InsertText {
                    paragraph: 0,
                    offset: 0,
                    text,
                };
                apply_incremental(&mut actual, &command, visible_end);
                reference.apply_command(command, false).unwrap();
                let context = format!("{alignment} bound {visible_end} step {step}");
                assert_page_pixels(&actual, &reference, 0, &context);
                if !actual.renderer().last_layout_complete() {
                    actual.repaint_all().unwrap();
                }
                assert_same(&mut actual, &mut reference, &context);
            }
        }
    }
}

#[test]
fn typing_that_crosses_columns_matches_full_repaint() {
    let bytes = package(rich_body(), r#"<w:cols w:num="2" w:space="360"/>"#);
    let mut actual = open_bytes(&bytes);
    let mut reference = open_bytes(&bytes);
    for text in [
        "a".to_string(),
        "段組みをまたぐ日本語 ".repeat(80),
        "😀".to_string(),
    ] {
        let command = HeadlessCommand::InsertText {
            paragraph: 0,
            offset: 0,
            text,
        };
        apply_incremental(&mut actual, &command, 100);
        reference.apply_command(command, false).unwrap();
        assert_same(&mut actual, &mut reference, "column transition");
    }
}

fn check_fixture_typing(fixture: &str) {
    let path = format!("public/fixtures/{fixture}.docx");
    let mut actual = common::open_fixture(&path);
    let mut reference = common::open_fixture(&path);
    let paragraph = common::paragraph_on_first_page(&mut actual);
    reference.place_caret(paragraph, 0).unwrap();
    assert_same(&mut actual, &mut reference, fixture);
    for (step, text) in ["あ", "a", "日本語"].into_iter().enumerate() {
        let command = HeadlessCommand::InsertText {
            paragraph,
            offset: 0,
            text: text.into(),
        };
        apply_incremental(&mut actual, &command, 1);
        reference.apply_command(command, false).unwrap();
        // A fallback bounded walk only promises current pixels through page
        // one. Validate them before completing its deferred offscreen layout.
        assert_page_pixels(&actual, &reference, 0, fixture);
        if !actual.renderer().last_layout_complete() {
            actual.repaint_all().unwrap();
        }
        assert_same(
            &mut actual,
            &mut reference,
            &format!("{fixture}: step {step}"),
        );
    }
}

#[test]
fn bounded_typing_with_section_headers_and_footers_matches_full_repaint() {
    check_fixture_typing("header-footer-sections");
}

#[test]
fn bounded_typing_with_section_page_fields_matches_full_repaint() {
    check_fixture_typing("header-footer-sectionpages");
}

#[test]
fn bounded_typing_with_hyperlink_fields_matches_full_repaint() {
    check_fixture_typing("hyperlink-field-edge-cases");
}

#[test]
fn typing_after_display_changes_matches_full_repaint() {
    let bytes = package(rich_body(), "");
    let mut actual = open_bytes(&bytes);
    let mut reference = open_bytes(&bytes);
    for (step, (marks, dark, scale)) in [
        (true, false, 1.25),
        (false, true, 2.0),
        (true, true, 0.75),
        (false, false, 1.0),
    ]
    .into_iter()
    .enumerate()
    {
        for document in [&mut actual, &mut reference] {
            document.set_formatting_marks(marks).unwrap();
            document.set_dark_paper(dark).unwrap();
            document.set_raster_scale(scale).unwrap();
        }
        assert_same(
            &mut actual,
            &mut reference,
            &format!("before display edit {step}"),
        );
        let command = HeadlessCommand::InsertText {
            paragraph: 0,
            offset: 0,
            text: "日".into(),
        };
        apply_incremental(&mut actual, &command, 1);
        assert!(actual.renderer().last_completed_pagination_edit_preserved());
        reference.apply_command(command, false).unwrap();
        assert_same(&mut actual, &mut reference, &format!("display step {step}"));
    }
}
