#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=CELL(\"address\",A1)");
    model._set("A2", "=CELL()");

    model._set("A3", "=INFO(\"recalc\")");
    model._set("A4", "=INFO()");

    model._set("A5", "=N(TRUE)");
    model._set("A6", "=N()");
    model._set("A7", "=N(1, 2)");

    model._set("A8", "=SHEETS()");
    model._set("A9", "=SHEETS(1)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"$A$1");
    assert_eq!(model._get_text("A2"), *"#ERROR!");

    assert_eq!(model._get_text("A3"), *"Automatic");
    assert_eq!(model._get_text("A4"), *"#ERROR!");

    assert_eq!(model._get_text("A5"), *"1");
    assert_eq!(model._get_text("A6"), *"#ERROR!");
    assert_eq!(model._get_text("A7"), *"#ERROR!");

    assert_eq!(model._get_text("A8"), *"1");
    assert_eq!(model._get_text("A9"), *"#VALUE!");
}

#[test]
fn info_timezone() {
    let mut model = new_empty_model();
    model._set("A1", "=INFO(\"timezone\")");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"UTC");

    model.set_timezone("America/Panama").unwrap();

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"America/Panama");
}

#[test]
fn info_timezones() {
    let mut model = new_empty_model();
    model._set("A1", "=INFO(\"timezones\")");
    model._set("B1", "=COUNTA(A:A)");

    model.evaluate();

    assert_ne!(model._get_text("A1"), *"");
    let timezones = model._get_text("B1").parse::<i32>().unwrap();
    assert!(timezones > 400);
}

#[test]
fn cell_filename() {
    // Default workbook name is model
    let mut model = new_empty_model();

    model._set("A1", "=CELL(\"filename\")");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"[model.xlsx]Sheet1");

    model.workbook.name = "Expenses".to_string();
    model.rename_sheet("Sheet1", "2026").unwrap();

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"[Expenses.xlsx]2026");
}

#[test]
fn cell_reads_the_format() {
    use crate::types::{Alignment, HorizontalAlignment, Style};
    let mut model = new_empty_model();
    let style = |num_fmt: &str, horizontal: Option<HorizontalAlignment>| Style {
        num_fmt: num_fmt.to_string(),
        alignment: horizontal.map(|horizontal| Alignment {
            horizontal,
            ..Default::default()
        }),
        ..Default::default()
    };
    model._set("A1", "1");
    model
        .set_cell_style(0, 1, 1, &style("$#,##0.00_);[Red]($#,##0.00)", None))
        .unwrap();
    model._set("A2", "text");
    model
        .set_cell_style(0, 2, 1, &style("(0.0)", Some(HorizontalAlignment::Center)))
        .unwrap();
    model._set("A3", "right");
    model
        .set_cell_style(
            0,
            3,
            1,
            &style("d-mmm-yy", Some(HorizontalAlignment::Right)),
        )
        .unwrap();
    model._set("B1", "=CELL(\"format\",A1)");
    model._set("B2", "=CELL(\"color\",A1)");
    model._set("B3", "=CELL(\"parentheses\",A1)");
    model._set("B4", "=CELL(\"format\",A2)");
    model._set("B5", "=CELL(\"parentheses\",A2)");
    model._set("B6", "=CELL(\"prefix\",A2)");
    model._set("B7", "=CELL(\"prefix\",A3)");
    model._set("B8", "=CELL(\"prefix\",A1)");
    model._set("B9", "=CELL(\"format\",A3)");
    model._set("B10", "=CELL(\"protect\",A1)");
    model._set("B11", "=CELL(\"format\",C1)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), *"C2-");
    assert_eq!(model._get_text("B2"), *"1");
    assert_eq!(model._get_text("B3"), *"0");
    assert_eq!(model._get_text("B4"), *"F1()");
    assert_eq!(model._get_text("B5"), *"1");
    assert_eq!(model._get_text("B6"), *"^");
    assert_eq!(model._get_text("B7"), *"\"");
    // Prefixes are for text only.
    assert_eq!(model._get_text("B8"), *"");
    assert_eq!(model._get_text("B9"), *"D1");
    assert_eq!(model._get_text("B10"), *"1");
    assert_eq!(model._get_text("B11"), *"G");
}

#[test]
fn cell_width_spills_the_default_flag() {
    let mut model = new_empty_model();
    model._set("A1", "=CELL(\"width\",C1)");
    model._set("A2", "=CELL(\"width\",D1)");
    // 20 characters wide, as a file states it.
    model.set_column_width(0, 4, 20.71 * 9.0).unwrap();
    model.evaluate();
    assert_eq!(model._get_text("B1"), *"TRUE");
    assert_eq!(model._get_text("A2"), *"20");
    assert_eq!(model._get_text("B2"), *"FALSE");
}

#[test]
fn cell_address_on_another_sheet_names_book_and_sheet() {
    let mut model = new_empty_model();
    model.new_sheet();
    model.add_sheet("My Data").unwrap();
    model._set("A1", "=CELL(\"address\",Sheet2!B3)");
    model._set("A2", "=CELL(\"address\",'My Data'!C4)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"[model.xlsx]Sheet2!$B$3");
    assert_eq!(model._get_text("A2"), *"'[model.xlsx]My Data'!$C$4");
    let document = crate::host::Document {
        file_name: Some("Budget.xlsx".to_string()),
        directory: Some("/Users/me/Documents/".to_string()),
        origin: Some((5, 3)),
        manual: true,
    };
    model._set("A3", "=CELL(\"filename\",A1)");
    model._set("A4", "=INFO(\"directory\")");
    model._set("A5", "=INFO(\"origin\")");
    model._set("A6", "=INFO(\"recalc\")");
    crate::host::with_document(document, || model.evaluate());
    assert_eq!(model._get_text("A1"), *"[Budget.xlsx]Sheet2!$B$3");
    assert_eq!(
        model._get_text("A3"),
        *"/Users/me/Documents/[Budget.xlsx]Sheet1"
    );
    assert_eq!(model._get_text("A4"), *"/Users/me/Documents/");
    assert_eq!(model._get_text("A5"), *"$A:$C$5");
    assert_eq!(model._get_text("A6"), *"Manual");
}

#[test]
fn sheet_and_sheets_of_a_reference() {
    let mut model = new_empty_model();
    model.new_sheet();
    model.new_sheet();
    model._set("A1", "=SHEET(Sheet3!A1)");
    model._set("A2", "=SHEET(Sheet2!A1:B4)");
    model._set("A3", "=SHEET(INDIRECT(\"Sheet3!B2\"))");
    model._set("A4", "=SHEET(\"Sheet2\")");
    model._set("A5", "=SHEETS(Sheet2!A1)");
    model._set("A6", "=INFO(\"origin\")");
    model._set("A7", "=INFO(\"osversion\")");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"3");
    assert_eq!(model._get_text("A2"), *"2");
    assert_eq!(model._get_text("A3"), *"3");
    assert_eq!(model._get_text("A4"), *"2");
    assert_eq!(model._get_text("A5"), *"1");
    assert_eq!(model._get_text("A6"), *"$A:$A$1");
    assert_ne!(model._get_text("A7"), *"");
}
