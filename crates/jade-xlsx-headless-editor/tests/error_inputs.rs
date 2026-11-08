use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;

#[test]
fn error_inputs_propagate_as_errors_and_survive_save_and_history() {
    let mut book = HeadlessWorkbook::open_for_editing(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    let sheet = book.workbook().sheets[0].name.clone();
    book.set(&sheet, "B20", CellInput::Formula("IFERROR(A20,42)".into()))
        .unwrap();
    book.set(&sheet, "C20", CellInput::Formula("ISFORMULA(A20)".into()))
        .unwrap();
    for error in [
        "#NULL!", "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#N/A",
    ] {
        assert_eq!(
            CellInput::parse(&error.to_ascii_lowercase()),
            CellInput::Error(error.into())
        );
        book.set(&sheet, "A20", CellInput::parse(error)).unwrap();
        assert_eq!(book.cell(&sheet, "A20").unwrap().display, error);
        assert_eq!(book.cell(&sheet, "B20").unwrap().number, Some(42.0));
        assert_eq!(book.cell(&sheet, "C20").unwrap().display, "FALSE");
    }
    let saved = book.save_bytes().unwrap();
    let reopened = HeadlessWorkbook::open_for_editing(&saved).unwrap();
    assert_eq!(reopened.cell(&sheet, "A20").unwrap().display, "#N/A");
    assert_eq!(reopened.cell(&sheet, "B20").unwrap().number, Some(42.0));
    book.set(&sheet, "A20", CellInput::Text("#N/A".into()))
        .unwrap();
    assert_eq!(
        book.cell(&sheet, "B20").unwrap().text.as_deref(),
        Some("#N/A")
    );
    book.undo().unwrap();
    assert_eq!(book.cell(&sheet, "B20").unwrap().number, Some(42.0));
    book.redo().unwrap();
    assert_eq!(
        book.cell(&sheet, "B20").unwrap().text.as_deref(),
        Some("#N/A")
    );
}
