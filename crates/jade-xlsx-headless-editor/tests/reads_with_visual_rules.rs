//! Reading a sheet that carries colour-scale and above-average rules gives
//! every cell the same answer however it is asked for: `range`, `sheet_text`
//! and `find` batch the work, `cell` does one cell at a time and is the
//! reference.

use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::{ConditionalStyle, HeadlessWorkbook, NewConditionalRule};
use jade_xlsx_reader::{CellRange, CellRef};
use std::path::PathBuf;

const SHEET: &str = "Operations";
const ROWS: u32 = 400;

fn open() -> HeadlessWorkbook {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/grid-operations-workbook.xlsx"
    ));
    HeadlessWorkbook::open_for_editing(&std::fs::read(path).expect("fixture")).expect("open")
}

#[test]
fn batched_reads_match_single_cell_reads_under_visual_and_ranked_rules() {
    let mut book = open();
    // Column Z is clear of the fixture's data.
    let values: Vec<(String, CellInput)> = (1..=ROWS)
        .map(|row| {
            (
                format!("Z{row}"),
                CellInput::Number(f64::from((row * 7919) % 1009)),
            )
        })
        .collect();
    book.set_cells(SHEET, &values).unwrap();
    let range = format!("Z1:Z{ROWS}");
    book.add_conditional_format(
        SHEET,
        &range,
        &NewConditionalRule::ColorScale(vec![0xF8696B, 0xFFEB84, 0x63BE7B]),
        ConditionalStyle::None,
    )
    .unwrap();
    book.add_conditional_format(
        SHEET,
        &range,
        &NewConditionalRule::AboveAverage,
        ConditionalStyle::LightRedFill,
    )
    .unwrap();
    // Both rules are live, so the reads below really do consult them.
    let status = book.conditional_formatting_status();
    assert_eq!((status.rule_count, status.supported_rules), (2, 2));

    // The oracle: one cell at a time.
    let single: Vec<_> = (1..=ROWS)
        .map(|row| book.cell(SHEET, &format!("Z{row}")).unwrap())
        .collect();
    assert!(single.iter().any(|cell| cell.display != "0"));

    // `range` over the same cells.
    let ranged = book.range(SHEET, &range).unwrap();
    assert_eq!(ranged.rows.len(), ROWS as usize);
    for (row, expected) in ranged.rows.iter().zip(&single) {
        assert_eq!(row.len(), 1);
        assert_eq!(&row[0], expected);
    }

    // `range` over a wider block still agrees cell by cell.
    let wide = book.range(SHEET, &format!("Y1:AA{ROWS}")).unwrap();
    for (index, row) in wide.rows.iter().enumerate() {
        assert_eq!(row[1], single[index]);
        let neighbour = book.cell(SHEET, &format!("Y{}", index + 1)).unwrap();
        assert_eq!(row[0], neighbour);
    }

    // `sheet_text` shows the same display text.
    let text = book.sheet_text(SHEET).unwrap();
    let reference = text.reference.as_deref().expect("used range");
    let bounds = CellRange::parse(reference).unwrap();
    for (index, expected) in single.iter().enumerate() {
        // `Z1` is row index 0; the oracle list starts there.
        let top = CellRef::parse("Z1").unwrap();
        let row = (top.row + index as u32 - bounds.start.row) as usize;
        let column = (top.column - bounds.start.column) as usize;
        assert_eq!(text.rows[row][column], expected.display);
    }

    // `find` reports the same display text for each hit, and finds each
    // cell whose single-cell display contains the needle.
    for needle in ["1", "77"] {
        let hits: Vec<_> = book
            .find(needle, false)
            .into_iter()
            .filter(|hit| hit.reference.starts_with('Z'))
            .collect();
        let expected: Vec<_> = single
            .iter()
            .filter(|cell| cell.display.contains(needle))
            .map(|cell| (cell.reference.clone(), cell.display.clone()))
            .collect();
        assert!(!expected.is_empty());
        let found: Vec<_> = hits
            .into_iter()
            .map(|hit| (hit.reference, hit.display))
            .collect();
        assert_eq!(found, expected);
    }
}
