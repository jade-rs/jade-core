use jade_xlsx_calc::validation::{list, validate, Candidate, ValidationResult::*};
use jade_xlsx_reader::{CellRange, CellRef, CellValue, DataValidation, Workbook};

fn workbook() -> Workbook {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/grid-operations-workbook.xlsx"
    ))
    .unwrap();
    jade_xlsx_reader::read_xlsx(&bytes).unwrap()
}
fn rule(kind: &str, range: &str, formula: &str) -> DataValidation {
    DataValidation {
        ranges: vec![CellRange::parse(range).unwrap()],
        validation_type: Some(kind.into()),
        operator: Some("lessThan".into()),
        formula1: Some(formula.into()),
        show_error_message: true,
        ..Default::default()
    }
}
fn at(s: &str) -> CellRef {
    CellRef::parse(s).unwrap()
}
fn number(n: f64) -> Candidate {
    Candidate::Value(CellValue::Number(n))
}
#[test]
fn references_and_relative_bounds_are_evaluated_at_the_candidate_cell() {
    let mut book = workbook();
    // The fixture B2/B3 values are 10 and 20, rather than literal bounds.
    let r = rule("whole", "H2:H4", "$B2");
    assert_eq!(
        validate(&book, 0, at("H2"), &r, &number(15.0), "UTC").unwrap(),
        Invalid
    );
    assert_eq!(
        validate(&book, 0, at("H3"), &r, &number(15.0), "UTC").unwrap(),
        Valid
    );
    assert_eq!(
        validate(&book, 0, at("H3"), &r, &number(15.5), "UTC").unwrap(),
        Invalid
    );
    let r = rule("decimal", "H3", "10");
    assert_eq!(
        validate(
            &book,
            0,
            at("H3"),
            &r,
            &Candidate::Formula("6+6".into()),
            "UTC"
        )
        .unwrap(),
        Invalid
    );
    assert_eq!(
        validate(
            &book,
            0,
            at("H3"),
            &r,
            &Candidate::Formula("3+3".into()),
            "UTC"
        )
        .unwrap(),
        Valid
    );
    book.date_system = jade_xlsx_reader::DateSystem::Nineteen04;
    let r = rule("date", "H3", "DATE(1904,1,3)");
    assert_eq!(
        validate(&book, 0, at("H3"), &r, &number(1.0), "UTC").unwrap(),
        Valid
    );
    assert_eq!(
        validate(&book, 0, at("H3"), &r, &number(2.0), "UTC").unwrap(),
        Invalid
    );
}
#[test]
fn custom_predicates_see_the_new_value_and_do_not_change_the_workbook() {
    let book = workbook();
    let original = book.clone();
    let r = rule("custom", "B2:B10", "COUNTIF($B$2:$B$10,B2)=1");
    assert_eq!(
        validate(&book, 0, at("B2"), &r, &number(20.0), "UTC").unwrap(),
        Invalid
    );
    assert_eq!(
        validate(&book, 0, at("B2"), &r, &number(35.0), "UTC").unwrap(),
        Valid
    );
    assert_eq!(book, original);
    let r = rule("textLength", "H2", "3");
    assert_eq!(
        validate(
            &book,
            0,
            at("H2"),
            &r,
            &Candidate::Value(CellValue::Text("東京".into())),
            "UTC"
        )
        .unwrap(),
        Valid
    );
    assert_eq!(
        validate(
            &book,
            0,
            at("H2"),
            &r,
            &Candidate::Value(CellValue::Text("東京都".into())),
            "UTC"
        )
        .unwrap(),
        Invalid
    );
}
#[test]
fn lists_resolve_names_hidden_sheets_and_dynamic_sources() {
    let mut book = workbook();
    let mut hidden = book.sheets[0].clone();
    hidden.name = "Hidden source".into();
    hidden.visibility = jade_xlsx_reader::SheetVisibility::Hidden;
    hidden.tables.clear();
    book.sheets.push(hidden);
    let sheet = "Hidden source";
    book.defined_names.push(jade_xlsx_reader::DefinedName {
        name: "Choices".into(),
        formula: format!("'{sheet}'!$B$2:$B$3"),
        local_sheet_index: None,
        hidden: false,
    });
    for source in [
        "Choices",
        "OFFSET('Hidden source'!$B$2,0,0,2,1)",
        "INDIRECT(\"'Hidden source'!B2:B3\")",
    ] {
        let r = rule("list", "H2:H3", source);
        assert_eq!(
            list(&book, 0, at("H2"), &r, "UTC").unwrap(),
            Some(vec![CellValue::Number(10.0), CellValue::Number(20.0)]),
            "{source}"
        );
        assert_eq!(
            validate(&book, 0, at("H2"), &r, &number(10.0), "UTC").unwrap(),
            Valid,
            "{source}"
        );
        assert_eq!(
            validate(&book, 0, at("H2"), &r, &number(15.0), "UTC").unwrap(),
            Invalid,
            "{source}"
        );
    }
    let r = rule("list", "H2", "\"東京, 大阪\"");
    assert_eq!(
        list(&book, 0, at("H2"), &r, "UTC").unwrap(),
        Some(vec![
            CellValue::Text("東京".into()),
            CellValue::Text(" 大阪".into())
        ])
    );
    let r = rule("list", "H2", "NO_SUCH_FUNCTION()");
    assert_eq!(list(&book, 0, at("H2"), &r, "UTC").unwrap(), None);
    assert_eq!(
        validate(&book, 0, at("H2"), &r, &number(10.0), "UTC").unwrap(),
        Unsupported
    );
}
