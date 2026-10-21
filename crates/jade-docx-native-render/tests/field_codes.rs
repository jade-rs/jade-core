//! Field codes on and off (⌥F9, ⇧F9), and the clock DATE fields show.

mod common;

use jade_docx_editor::field_datetime::FieldDateTime;
use jade_docx_editor::fields::{field_toggle_ids, FieldCodeView};
use jade_selection::{SelectionRange, TextPosition};

fn laid_out(document: &common::Doc) -> String {
    document.renderer().paragraph_debug_texts().join("\n")
}

#[test]
fn every_field_shows_its_code_in_braces_and_back() {
    let mut document = common::open_fixture("public/fixtures/document-property-fields.docx");
    let results = laid_out(&document);
    assert!(!results.contains("{ "), "{results}");

    document
        .set_field_code_view(FieldCodeView {
            all: true,
            ..FieldCodeView::default()
        })
        .unwrap();
    let codes = laid_out(&document);
    assert!(codes.contains("{ docproperty \"Title\""), "{codes}");
    assert!(codes.contains("{ DOCPROPERTY"), "{codes}");

    document
        .set_field_code_view(FieldCodeView::default())
        .unwrap();
    assert_eq!(laid_out(&document), results);
}

#[test]
fn one_field_toggles_against_the_rest() {
    let mut document = common::open_fixture("public/fixtures/document-property-fields.docx");
    let whole = |document: &common::Doc, index: usize| {
        let length = document.paragraph_text(index).map_or(0, |text| text.len());
        SelectionRange {
            anchor: TextPosition::new(index, 0),
            focus: TextPosition::new(index, length),
            ..SelectionRange::collapsed(TextPosition::new(index, 0))
        }
    };
    let paragraph = (0..document.paragraph_count())
        .find(|&index| {
            jade_docx_editor::fields::fields_at(document.word_document(), whole(&document, index))
                .len()
                == 1
        })
        .expect("a paragraph with one field");
    let ids = field_toggle_ids(document.word_document(), whole(&document, paragraph));
    assert_eq!(ids.len(), 1);
    document
        .set_field_code_view(FieldCodeView {
            all: false,
            toggled: ids.into_iter().collect(),
        })
        .unwrap();
    let texts = document.renderer().paragraph_debug_texts();
    let shown = texts.iter().filter(|text| text.contains("{ ")).count();
    assert_eq!(shown, 1, "{texts:?}");
}

#[test]
fn a_clock_reaches_date_fields() {
    let mut document = common::open_fixture("public/fixtures/document-property-fields.docx");
    document.set_selection_range(Some(SelectionRange::collapsed(TextPosition::new(0, 0))));
    // Inserted on 2000-01-01, the environment's default clock.
    document
        .execute_command(
            Box::new(jade_docx_editor::fields::FieldCommand::insert(
                "DATE \\@ \"yyyy年M月d日\"".to_string(),
                Default::default(),
            )),
            false,
        )
        .unwrap();
    assert!(
        laid_out(&document).contains("2000年1月1日"),
        "{}",
        laid_out(&document)
    );
    document.set_field_datetime(Some(FieldDateTime {
        year: 2026,
        month: 10,
        day: 21,
        hour: 9,
        minute: 0,
        second: 0,
        weekday: 3,
    }));
    document.repaint_all().unwrap();
    assert!(
        laid_out(&document).contains("2026年10月21日"),
        "{}",
        laid_out(&document)
    );
}
