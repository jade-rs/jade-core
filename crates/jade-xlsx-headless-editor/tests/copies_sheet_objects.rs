//! Move or Copy ▸ Create a copy with the sheet's pictures, charts and notes:
//! the pictures shared, the charts reading the copy's cells, the notes the
//! copy's own.

use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::ShapeKind;

fn open(name: &str) -> HeadlessWorkbook {
    let path = format!(
        "{}/../../public/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    HeadlessWorkbook::open(&std::fs::read(&path).unwrap()).expect("open")
}

#[test]
fn pictures_come_with_the_copy() {
    let mut workbook = open("pictures-workbook.xlsx");
    let source = workbook.workbook().sheets[0].name.clone();
    let pictures = |workbook: &HeadlessWorkbook, sheet: usize| {
        workbook.workbook().sheets[sheet]
            .shapes
            .iter()
            .filter(|shape| shape.kind == ShapeKind::Picture)
            .filter_map(|shape| {
                shape
                    .picture
                    .as_ref()?
                    .media
                    .as_ref()
                    .map(|m| m.bytes.len())
            })
            .collect::<Vec<_>>()
    };
    let before = pictures(&workbook, 0);
    assert!(!before.is_empty());
    let at = workbook.copy_sheet(&source, "Pictures (2)").expect("copy");
    assert_eq!(pictures(&workbook, at), before);
    let reopened = HeadlessWorkbook::open(&workbook.export_bytes().unwrap()).unwrap();
    assert_eq!(pictures(&reopened, at), before);
    assert_eq!(pictures(&reopened, 0), before);
}

#[test]
fn a_chart_on_the_copy_reads_the_copys_cells() {
    let mut workbook = open("charts-workbook.xlsx");
    let source = workbook.workbook().sheets[0].name.clone();
    let formulas = |workbook: &HeadlessWorkbook, sheet: usize| {
        workbook.workbook().sheets[sheet]
            .shapes
            .iter()
            .filter_map(|shape| shape.chart.as_ref())
            .flat_map(|chart| {
                chart
                    .series
                    .iter()
                    .map(|series| series.values.formula.clone())
            })
            .collect::<Vec<_>>()
    };
    let original = formulas(&workbook, 0);
    assert!(!original.is_empty());
    let at = workbook.copy_sheet(&source, "Charts (2)").expect("copy");
    let copied = formulas(&workbook, at);
    assert_eq!(copied.len(), original.len());
    // What read the sheet copied reads the copy; what read another sheet
    // still reads it.
    let mut moved = 0;
    for (copy, source) in copied.iter().zip(&original) {
        let (copy, source) = (
            copy.as_deref().unwrap_or_default(),
            source.as_deref().unwrap_or_default(),
        );
        if source.starts_with("Charts!") {
            assert_eq!(copy, source.replacen("Charts!", "'Charts (2)'!", 1));
            moved += 1;
        } else {
            assert_eq!(copy, source);
        }
    }
    assert!(moved > 0);
    // The two charts are parts of their own.
    let part = |sheet: usize| {
        workbook.workbook().sheets[sheet]
            .shapes
            .iter()
            .find_map(|shape| shape.chart.as_ref()?.part_name.clone())
    };
    assert_ne!(part(0), part(at));
    HeadlessWorkbook::open(&workbook.export_bytes().unwrap()).expect("reopens");
}

#[test]
fn notes_come_with_the_copy_and_stay_its_own() {
    let mut workbook = open("sample-workbook.xlsx");
    let source = workbook.workbook().sheets[0].name.clone();
    workbook
        .set_note(&source, "B3", "check this", "Reviewer")
        .expect("note");
    let at = workbook.copy_sheet(&source, "Noted (2)").expect("copy");
    let note = |workbook: &HeadlessWorkbook, sheet: usize| {
        workbook.workbook().sheets[sheet]
            .comments
            .iter()
            .map(|comment| comment.text.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(note(&workbook, at), note(&workbook, 0));
    workbook
        .set_note("Noted (2)", "B3", "changed on the copy", "Reviewer")
        .expect("note");
    assert!(note(&workbook, 0)[0].contains("check this"));
    assert!(note(&workbook, at)[0].contains("changed on the copy"));
}

/// A sheet with a pivot table copies with a pivot table of its own, in the
/// same place on the copy and reading the same cache; one with a
/// PivotChart is refused rather than copied in part.
#[test]
fn a_pivot_table_comes_with_the_copy_sharing_its_cache() {
    let mut workbook = open("pivot-cache-compat.xlsx");
    let tables = workbook.workbook().pivots.tables.len();
    let original = workbook.workbook().pivots.tables[0].clone();
    let source = workbook
        .workbook()
        .sheets
        .iter()
        .find(|sheet| sheet.part_name == original.sheet_part)
        .expect("the report's sheet")
        .name
        .clone();
    let at = workbook
        .copy_sheet(&source, &format!("{source} (2)"))
        .expect("copy");
    let reopened = HeadlessWorkbook::open(&workbook.export_bytes().expect("export")).expect("open");
    let book = reopened.workbook();
    assert_eq!(book.pivots.tables.len(), tables + 1);
    let copy_part = &book.sheets[at].part_name;
    let copy = book
        .pivots
        .tables
        .iter()
        .find(|table| &table.sheet_part == copy_part)
        .expect("the copy's pivot table");
    assert_ne!(copy.part_name, original.part_name);
    assert_eq!(copy.cache_id, original.cache_id, "one cache, shared");
    assert_eq!(copy.location, original.location);
    assert_eq!(
        book.pivots.caches.len(),
        workbook.workbook().pivots.caches.len()
    );
    assert!(book.pivots.issues.is_empty(), "{:?}", book.pivots.issues);
    // The report's cells come too.
    let from = book
        .sheets
        .iter()
        .position(|sheet| sheet.name == source)
        .expect("source");
    let corner = original.location.reference.end;
    let value = |sheet: usize| {
        book.sheets[sheet]
            .cell_at(corner)
            .map(|cell| cell.value.clone())
    };
    assert!(value(from).is_some());
    assert_eq!(value(from), value(at));

    let mut charted = open("pivot-chart-compat.xlsx");
    let sheets: Vec<String> = charted
        .workbook()
        .sheets
        .iter()
        .map(|sheet| sheet.name.clone())
        .collect();
    let with_chart = sheets
        .iter()
        .find(|name| {
            charted
                .workbook()
                .sheets
                .iter()
                .find(|sheet| &&sheet.name == name)
                .is_some_and(|sheet| sheet.shapes.iter().any(|shape| shape.chart.is_some()))
        })
        .expect("a sheet with a PivotChart")
        .clone();
    let count = charted.workbook().sheets.len();
    assert!(charted.copy_sheet(&with_chart, "Copied chart").is_err());
    assert_eq!(charted.workbook().sheets.len(), count);
}
