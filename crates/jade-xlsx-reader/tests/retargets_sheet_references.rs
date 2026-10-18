//! References follow a sheet that is renamed, and break when it is deleted.
//!
//! Excel rewrites every formula and defined name that names a renamed sheet,
//! quoting the new name when it has to be quoted, and leaves text inside
//! string literals alone (`INDIRECT("Sheet1!A1")` is not a reference). A
//! deleted sheet's references become `#REF!` — `Sheet2!A1` reads
//! `#REF!A1` — and a 3-D reference whose end sheet goes shrinks to the
//! sheets that remain.

use jade_xlsx_reader::{
    collapse_ref_errors, delete_sheet_in_formula, quote_sheet_name, rename_sheet_in_formula,
};
use proptest::prelude::*;

#[test]
fn a_renamed_sheet_is_renamed_wherever_a_formula_names_it() {
    let rename = |formula: &str| rename_sheet_in_formula(formula, "Sheet1", "Totals");
    assert_eq!(rename("Sheet1!A1+1"), "Totals!A1+1");
    assert_eq!(rename("SUM(Sheet1!$A$1:$B$2)*2"), "SUM(Totals!$A$1:$B$2)*2");
    assert_eq!(rename("'Sheet1'!A1"), "Totals!A1");
    // Sheet names compare without regard to case, as Excel compares them.
    assert_eq!(rename("sheet1!A1"), "Totals!A1");
    // A defined name scoped to the sheet, and a whole column.
    assert_eq!(rename("Sheet1!Rate*Sheet1!C:C"), "Totals!Rate*Totals!C:C");
    // Unqualified references and other sheets are not touched.
    assert_eq!(rename("A1+Sheet10!A1+Other!B2"), "A1+Sheet10!A1+Other!B2");
    // Nor is text in a string, an error literal, or another workbook.
    assert_eq!(
        rename(r#"INDIRECT("Sheet1!A1")&"Sheet1!""#),
        r#"INDIRECT("Sheet1!A1")&"Sheet1!""#
    );
    assert_eq!(
        rename("[1]Sheet1!A1+'[2]Sheet1'!A1"),
        "[1]Sheet1!A1+'[2]Sheet1'!A1"
    );
    assert_eq!(
        rename_sheet_in_formula("#REF!A1+REF!A1", "REF", "Fixed"),
        "#REF!A1+Fixed!A1"
    );
    // Structured references are a table's business, not a sheet's.
    assert_eq!(
        rename("Table1[[#This Row],[Sheet1!x]]+Sheet1!A1"),
        "Table1[[#This Row],[Sheet1!x]]+Totals!A1"
    );
}

#[test]
fn the_new_name_is_quoted_when_it_has_to_be() {
    assert_eq!(
        rename_sheet_in_formula("Sheet1!A1", "Sheet1", "Q3 2026"),
        "'Q3 2026'!A1"
    );
    assert_eq!(
        rename_sheet_in_formula("'It''s'!B2:B3", "It's", "O'Neil"),
        "'O''Neil'!B2:B3"
    );
    assert_eq!(
        rename_sheet_in_formula("'Q3 2026'!A1", "q3 2026", "Plain"),
        "Plain!A1"
    );
    assert_eq!(
        rename_sheet_in_formula("売上!A1*2", "売上", "売上サマリ"),
        "'売上サマリ'!A1*2"
    );
    // A name that reads as a cell reference has to be quoted.
    assert_eq!(
        rename_sheet_in_formula("Sheet1!A1", "Sheet1", "A1"),
        "'A1'!A1"
    );
    assert_eq!(quote_sheet_name("Sheet1"), "Sheet1");
    assert_eq!(quote_sheet_name("Data_2026.v2"), "Data_2026.v2");
    for needs_quotes in [
        "My Sheet",
        "A1",
        "XFD1048576",
        "R1C1",
        "R",
        "C",
        "rc",
        "R2C",
        "1st",
        ".x",
        "a-b",
        "O'Neil",
        "売上",
        "TRUE",
    ] {
        assert!(
            quote_sheet_name(needs_quotes).starts_with('\''),
            "{needs_quotes} -> {}",
            quote_sheet_name(needs_quotes)
        );
    }
    assert_eq!(quote_sheet_name("O'Neil"), "'O''Neil'");
}

#[test]
fn a_three_d_reference_renames_the_end_it_names() {
    assert_eq!(
        rename_sheet_in_formula("SUM(Jan:Mar!B2)", "Jan", "Q1 start"),
        "SUM('Q1 start:Mar'!B2)"
    );
    assert_eq!(
        rename_sheet_in_formula("SUM('Jan:Mar'!B2)", "Mar", "End"),
        "SUM(Jan:End!B2)"
    );
    assert_eq!(
        rename_sheet_in_formula("SUM(Jan:Mar!B2)", "Feb", "Middle"),
        "SUM(Jan:Mar!B2)"
    );
}

#[test]
fn a_deleted_sheet_leaves_ref_errors_behind() {
    let order = ["Jan", "Feb", "Mar", "Other Sheet"];
    let delete = |formula: &str, sheet: &str| delete_sheet_in_formula(formula, sheet, &order, &[]);
    assert_eq!(delete("Feb!A1*2", "Feb"), "#REF!A1*2");
    assert_eq!(
        delete("SUM('Other Sheet'!A1:B2)", "other sheet"),
        "SUM(#REF!A1:B2)"
    );
    assert_eq!(delete("Feb!Rate+A1+Jan!A1", "Feb"), "#REF!Rate+A1+Jan!A1");
    assert_eq!(delete(r#""Feb!A1"&Feb!A1"#, "Feb"), r#""Feb!A1"&#REF!A1"#);
    // A 3-D reference keeps the sheets that remain between its ends.
    assert_eq!(delete("SUM(Jan:Mar!B2)", "Jan"), "SUM(Feb:Mar!B2)");
    assert_eq!(delete("SUM(Jan:Mar!B2)", "Mar"), "SUM(Jan:Feb!B2)");
    assert_eq!(delete("SUM(Jan:Mar!B2)", "Feb"), "SUM(Jan:Mar!B2)");
    assert_eq!(delete("SUM(Jan:Feb!B2)", "Jan"), "SUM(Feb!B2)");
    assert_eq!(
        delete("SUM('Jan:Other Sheet'!B2)", "Jan"),
        "SUM('Feb:Other Sheet'!B2)"
    );
}

#[test]
fn a_deleted_sheets_tables_leave_ref_errors_behind() {
    let order = ["Data", "Report"];
    let delete =
        |formula: &str| delete_sheet_in_formula(formula, "Data", &order, &["Sales".to_string()]);
    assert_eq!(delete("SUM(Sales[Amount])"), "SUM(#REF!)");
    assert_eq!(delete("sales[[#This Row],[Amount]]*2"), "#REF!*2");
    assert_eq!(
        delete("SUM(Other[Amount])+Sales[#Totals]"),
        "SUM(Other[Amount])+#REF!"
    );
    // A bracket escaped inside a column name does not end the reference.
    assert_eq!(delete("Sales['[Net']]+1"), "#REF!+1");
}

#[test]
fn a_ref_error_qualifier_evaluates_as_the_error_alone() {
    assert_eq!(collapse_ref_errors("#REF!D7*2"), "#REF!*2");
    assert_eq!(collapse_ref_errors("SUM(#REF!B4:B6)+A1"), "SUM(#REF!)+A1");
    assert_eq!(collapse_ref_errors("#REF!$D$7"), "#REF!");
    assert_eq!(collapse_ref_errors("#REF!Rate+#REF!"), "#REF!+#REF!");
    assert_eq!(
        collapse_ref_errors(r##""#REF!A1"&'#REF!'!A1"##),
        r##""#REF!A1"&'#REF!'!A1"##
    );
    assert_eq!(collapse_ref_errors("Sheet1!A1"), "Sheet1!A1");
}

/// A sheet name as a formula writes it.
fn name() -> impl Strategy<Value = String> {
    prop_oneof![
        "[A-Za-z_][A-Za-z0-9_]{0,8}",
        "[A-Za-z][A-Za-z0-9 ']{0,8}[A-Za-z0-9]",
        "[売上データ表]{1,4}",
    ]
}

/// A formula built from references into `sheets`, strings and operators.
fn formula(sheets: Vec<String>) -> impl Strategy<Value = String> {
    let piece = (0..sheets.len(), 0u8..5, "[A-H][1-9]").prop_map(move |(sheet, kind, cell)| {
        let qualifier = quote_sheet_name(&sheets[sheet]);
        match kind {
            0 => format!("{qualifier}!{cell}"),
            1 => format!("SUM({qualifier}!{cell}:$H$9)"),
            2 => format!("\"{}!{cell}\"", sheets[sheet].replace('"', "\"\"")),
            3 => cell,
            _ => format!("{qualifier}!Rate"),
        }
    });
    prop::collection::vec(piece, 1..6).prop_map(|pieces| pieces.join("+"))
}

proptest! {
    /// Renaming there and back restores the formula exactly, and renaming
    /// to the same name changes nothing.
    #[test]
    fn renaming_there_and_back_is_the_identity(
        (sheets, text) in prop::collection::vec(name(), 2..4)
            .prop_filter("distinct names", |sheets| {
                let mut seen = std::collections::HashSet::new();
                sheets.iter().all(|sheet| seen.insert(sheet.to_lowercase()))
            })
            .prop_flat_map(|sheets| (Just(sheets.clone()), formula(sheets))),
        fresh in "[A-Za-z][A-Za-z0-9 ]{0,8}[A-Za-z0-9]",
    ) {
        prop_assume!(!sheets.iter().any(|sheet| sheet.eq_ignore_ascii_case(&fresh)));
        let renamed = rename_sheet_in_formula(&text, &sheets[0], &fresh);
        prop_assert_eq!(rename_sheet_in_formula(&renamed, &fresh, &sheets[0]), text.clone());
        prop_assert_eq!(rename_sheet_in_formula(&text, &sheets[0], &sheets[0]), text);
    }

    /// After a deletion no reference names the deleted sheet, and every
    /// reference that did is a `#REF!`.
    #[test]
    fn a_deleted_sheet_is_never_named_again(
        (sheets, text) in prop::collection::vec(name(), 2..4)
            .prop_filter("distinct names", |sheets| {
                let mut seen = std::collections::HashSet::new();
                sheets.iter().all(|sheet| seen.insert(sheet.to_lowercase()))
            })
            .prop_flat_map(|sheets| (Just(sheets.clone()), formula(sheets))),
    ) {
        let order: Vec<&str> = sheets.iter().map(String::as_str).collect();
        let deleted = delete_sheet_in_formula(&text, &sheets[0], &order, &[]);
        prop_assert_eq!(
            rename_sheet_in_formula(&deleted, &sheets[0], "Witness"),
            deleted.clone()
        );
        if deleted != text {
            prop_assert!(deleted.contains("#REF!"));
        }
        // Deleting a sheet the formula never names changes nothing.
        prop_assert_eq!(delete_sheet_in_formula(&text, "Witness", &order, &[]), text);
    }
}
