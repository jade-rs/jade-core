//! Conditional formats are made, listed, deleted and cleared as Excel's
//! Conditional Formatting menu does: highlight rules (greater than, less
//! than, between, equal to, text that contains, duplicate values), top and
//! bottom rules (top/bottom N items or percent, above/below average), data
//! bars and colour scales. Each draws as Excel draws it, saves as Excel
//! writes it, and is one undoable step.

use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::{
    ConditionalStyle, HeadlessError, HeadlessWorkbook, NewConditionalRule,
};
use jade_xlsx_layout::{layout_sheet, LayoutOptions};
use jade_xlsx_reader::{CellRef, Color};
use std::path::PathBuf;

const SHEET: &str = "Operations";

/// Operations!B2:B4 hold 10, 20, 30; A2:A4 Alpha, Beta, Gamma.
fn open() -> HeadlessWorkbook {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/grid-operations-workbook.xlsx"
    ));
    HeadlessWorkbook::open_for_editing(&std::fs::read(path).expect("fixture")).expect("open")
}

fn fill(book: &HeadlessWorkbook, cell: &str) -> Option<Color> {
    let layout = layout_sheet(
        &book.workbook().sheets[0],
        &book.workbook().styles,
        &LayoutOptions::default(),
    );
    layout
        .cell(CellRef::parse(cell).unwrap())
        .and_then(|cell| cell.fill.as_ref())
        .and_then(|fill| fill.foreground)
}

fn highlighted(book: &HeadlessWorkbook, cells: &[&str]) -> Vec<bool> {
    cells
        .iter()
        .map(|cell| fill(book, cell).is_some())
        .collect()
}

/// Excel's default highlight: light red fill with dark red text.
const LIGHT_RED: Color = rgb(0xFFC7CE);

const fn rgb(value: u32) -> Color {
    Color::Argb([0xFF, (value >> 16) as u8, (value >> 8) as u8, value as u8])
}

#[test]
fn a_highlight_rule_draws_saves_and_undoes() {
    let mut book = open();
    book.add_conditional_format(
        SHEET,
        "B2:B4",
        &NewConditionalRule::GreaterThan("15".into()),
        ConditionalStyle::LightRedFillDarkRedText,
    )
    .unwrap();
    assert_eq!(highlighted(&book, &["B2", "B3", "B4"]), [false, true, true]);
    assert_eq!(fill(&book, "B3"), Some(LIGHT_RED));
    // Saved as Excel writes it, and read back the same.
    let saved = book.export_bytes().unwrap();
    let reopened = HeadlessWorkbook::open(&saved).unwrap();
    assert_eq!(
        highlighted(&reopened, &["B2", "B3", "B4"]),
        [false, true, true]
    );
    let rules = reopened.conditional_formats(SHEET).unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].range, "B2:B4");
    // The rule follows the values.
    book.set_cells(SHEET, &[("B2".into(), CellInput::Number(99.0))])
        .unwrap();
    book.recalculate().unwrap();
    assert_eq!(highlighted(&book, &["B2", "B3", "B4"]), [true, true, true]);
    assert!(book.undo().unwrap());
    assert!(book.undo().unwrap());
    assert!(book.conditional_formats(SHEET).unwrap().is_empty());
    assert_eq!(
        highlighted(&book, &["B2", "B3", "B4"]),
        [false, false, false]
    );
}

#[test]
fn every_highlight_rule_matches_as_excel_does() {
    let cases: Vec<(NewConditionalRule, [bool; 3])> = vec![
        (
            NewConditionalRule::LessThan("20".into()),
            [true, false, false],
        ),
        (
            NewConditionalRule::Between("15".into(), "30".into()),
            [false, true, true],
        ),
        (
            NewConditionalRule::EqualTo("20".into()),
            [false, true, false],
        ),
        (
            NewConditionalRule::Top {
                rank: 1,
                percent: false,
            },
            [false, false, true],
        ),
        (
            NewConditionalRule::Bottom {
                rank: 1,
                percent: false,
            },
            [true, false, false],
        ),
        // 34% of three values rounds down to one, as Excel's does.
        (
            NewConditionalRule::Top {
                rank: 34,
                percent: true,
            },
            [false, false, true],
        ),
        (NewConditionalRule::AboveAverage, [false, false, true]),
        (NewConditionalRule::BelowAverage, [true, false, false]),
    ];
    for (rule, expected) in cases {
        let mut book = open();
        book.add_conditional_format(
            SHEET,
            "B2:B4",
            &rule,
            ConditionalStyle::YellowFillDarkYellowText,
        )
        .unwrap();
        assert_eq!(
            highlighted(&book, &["B2", "B3", "B4"]),
            expected,
            "{rule:?}"
        );
        let reopened = HeadlessWorkbook::open(&book.export_bytes().unwrap()).unwrap();
        assert_eq!(
            highlighted(&reopened, &["B2", "B3", "B4"]),
            expected,
            "{rule:?} reopened"
        );
    }
    // Text that contains, and duplicate values.
    let mut book = open();
    book.add_conditional_format(
        SHEET,
        "A2:A4",
        &NewConditionalRule::TextContains("mm".into()),
        ConditionalStyle::GreenFillDarkGreenText,
    )
    .unwrap();
    assert_eq!(
        highlighted(&book, &["A2", "A3", "A4"]),
        [false, false, true]
    );
    book.set_cells(SHEET, &[("B3".into(), CellInput::Number(10.0))])
        .unwrap();
    book.add_conditional_format(
        SHEET,
        "B2:B4",
        &NewConditionalRule::DuplicateValues,
        ConditionalStyle::LightRedFill,
    )
    .unwrap();
    assert_eq!(highlighted(&book, &["B2", "B3", "B4"]), [true, true, false]);
}

#[test]
fn data_bars_and_colour_scales_draw() {
    let mut book = open();
    book.add_conditional_format(
        SHEET,
        "B2:B4",
        &NewConditionalRule::DataBar(0x638EC6),
        ConditionalStyle::None,
    )
    .unwrap();
    let layout = layout_sheet(
        &book.workbook().sheets[0],
        &book.workbook().styles,
        &LayoutOptions::default(),
    );
    assert!(layout
        .cell(CellRef::parse("B4").unwrap())
        .unwrap()
        .data_bar
        .is_some());
    let mut book = open();
    book.add_conditional_format(
        SHEET,
        "B2:B4",
        &NewConditionalRule::ColorScale(vec![0xF8696B, 0x63BE7B]),
        ConditionalStyle::None,
    )
    .unwrap();
    assert_eq!(fill(&book, "B2"), Some(rgb(0xF8696B)));
    assert_eq!(fill(&book, "B4"), Some(rgb(0x63BE7B)));
}

/// The newest rule comes first, as Excel puts it; rules are listed in that
/// order, deleted one by one, and cleared from a range or the sheet.
#[test]
fn rules_are_listed_deleted_and_cleared() {
    let mut book = open();
    book.add_conditional_format(
        SHEET,
        "B2:B4",
        &NewConditionalRule::GreaterThan("15".into()),
        ConditionalStyle::LightRedFill,
    )
    .unwrap();
    book.add_conditional_format(
        SHEET,
        "C2:C4",
        &NewConditionalRule::LessThan("50".into()),
        ConditionalStyle::GreenFillDarkGreenText,
    )
    .unwrap();
    let rules = book.conditional_formats(SHEET).unwrap();
    assert_eq!(
        rules
            .iter()
            .map(|rule| rule.range.as_str())
            .collect::<Vec<_>>(),
        ["C2:C4", "B2:B4"]
    );
    assert!(
        rules[0].description.contains("50"),
        "{}",
        rules[0].description
    );
    book.delete_conditional_format(SHEET, 0).unwrap();
    assert_eq!(book.conditional_formats(SHEET).unwrap().len(), 1);
    assert!(fill(&book, "C2").is_none());
    // Clearing B3 leaves the rule on the rest of its cells.
    assert!(!book.clear_conditional_formats(SHEET, Some("A1")).unwrap());
    assert!(book.clear_conditional_formats(SHEET, Some("B3")).unwrap());
    let rules = book.conditional_formats(SHEET).unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].range, "B2 B4");
    assert!(fill(&book, "B3").is_none());
    assert!(fill(&book, "B4").is_some());
    book.clear_conditional_formats(SHEET, None).unwrap();
    assert!(book.conditional_formats(SHEET).unwrap().is_empty());
    assert!(fill(&book, "B4").is_none());
    assert!(matches!(
        book.delete_conditional_format(SHEET, 5),
        Err(HeadlessError::BadReference(_))
    ));
}

/// Excel 2010's data bars keep half of themselves in the sheet's `extLst`:
/// a linked `x14:cfRule` beside each rule, and standalone ones with their
/// own priority. A new rule ranks ahead of those too; deleting or clearing
/// a rule takes its other half with it, and leaves the rest whole.
#[test]
fn excel_2010_data_bars_stay_whole_around_edits() {
    const MODERN: &str = "Modern";
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/x14-bars-workbook.xlsx"
    ));
    let mut book =
        HeadlessWorkbook::open_for_editing(&std::fs::read(path).expect("fixture")).expect("open");
    let issues = |book: &HeadlessWorkbook| -> Vec<Vec<String>> {
        book.workbook().sheets[0]
            .conditional_formats
            .iter()
            .flat_map(|format| format.rules.iter().map(|rule| rule.issues.clone()))
            .collect()
    };
    let bars = |book: &HeadlessWorkbook| -> Vec<String> {
        let layout = layout_sheet(
            &book.workbook().sheets[0],
            &book.workbook().styles,
            &LayoutOptions::default(),
        );
        ["A5", "B5", "C5", "D5", "E5", "F5", "G5", "H5"]
            .iter()
            .filter(|cell| {
                layout
                    .cell(CellRef::parse(cell).unwrap())
                    .is_some_and(|cell| cell.data_bar.is_some())
            })
            .map(|cell| cell.to_string())
            .collect()
    };
    let before = book.conditional_formats(MODERN).unwrap();
    let before_issues = issues(&book);
    let before_bars = bars(&book);
    assert_eq!(before.len(), 8);
    assert_eq!(before[7].range, "H1:H5");

    book.add_conditional_format(
        MODERN,
        "A1:H1",
        &NewConditionalRule::GreaterThan("1000".into()),
        ConditionalStyle::LightRedFill,
    )
    .unwrap();
    let after = book.conditional_formats(MODERN).unwrap();
    assert_eq!(after.len(), 9);
    assert_eq!(after[0].range, "A1:H1");
    assert_eq!(
        after[8].range, "H1:H5",
        "the standalone x14 bar keeps its rank"
    );
    let mut expected_issues = vec![Vec::new()];
    expected_issues.extend(before_issues.clone());
    let mut now = issues(&book);
    now.sort();
    expected_issues.sort();
    assert_eq!(now, expected_issues, "no rule lost its x14 half");
    assert_eq!(bars(&book), before_bars);

    // Deleting A's bar takes its x14 half; the others are unchanged.
    let a = after.iter().position(|rule| rule.range == "A1:A5").unwrap();
    book.delete_conditional_format(MODERN, a).unwrap();
    let after = book.conditional_formats(MODERN).unwrap();
    assert_eq!(after.len(), 8);
    assert!(after.iter().all(|rule| rule.range != "A1:A5"));
    assert_eq!(bars(&book), before_bars[1..]);
    assert!(issues(&book)
        .iter()
        .all(|found| before_issues.contains(found) || found.is_empty()));

    // Clearing B and H takes B's pair and the standalone H.
    book.clear_conditional_formats(MODERN, Some("B1:B5"))
        .unwrap();
    book.clear_conditional_formats(MODERN, Some("H1:H5"))
        .unwrap();
    assert_eq!(bars(&book), ["C5", "D5", "E5", "F5", "G5"]);
    assert_eq!(book.conditional_formats(MODERN).unwrap().len(), 6);

    // Clearing the sheet leaves no half behind to come back as a bar.
    book.clear_conditional_formats(MODERN, None).unwrap();
    let reopened = HeadlessWorkbook::open(&book.export_bytes().unwrap()).unwrap();
    assert!(reopened.conditional_formats(MODERN).unwrap().is_empty());
    assert!(bars(&reopened).is_empty());
}
