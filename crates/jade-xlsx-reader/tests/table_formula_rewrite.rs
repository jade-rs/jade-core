use jade_xlsx_reader::rewrite_table_references;
#[test]
fn table_rewrite_keeps_text_qualifiers_external_links_and_escaped_selectors() {
    let cases = [
        ("SUM(売上[金額])+ROWS(売上)", "SUM(Sales[金額])+ROWS(Sales)"),
        ("SUM(売上[[#Data],[A']B]])", "SUM(Sales[[#Data],[A']B]])"),
        ("\"売上[A]\"&売上[A]", "\"売上[A]\"&Sales[A]"),
        (
            "[1]Sheet1!売上[A]+'[book.xlsx]Sheet 1'!売上[A]+売上[A]",
            "[1]Sheet1!売上[A]+'[book.xlsx]Sheet 1'!売上[A]+Sales[A]",
        ),
        (
            "売上!A1+'売上'!A1+Other[売上]",
            "売上!A1+'売上'!A1+Other[売上]",
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(
            rewrite_table_references(source, |name, selector| name
                .filter(|n| *n == "売上")
                .map(|_| format!("Sales{}", selector.unwrap_or_default()))),
            expected
        );
    }
    assert_eq!(
        rewrite_table_references("[@Amount]*2", |name, selector| name
            .is_none()
            .then(|| format!("Current{}", selector.unwrap()))),
        "Current[@Amount]*2"
    );
}
#[test]
fn translating_formulas_does_not_treat_column_name_text_as_cell_references() {
    use jade_xlsx_reader::{translate_formula, transpose_formula, CellRef};
    let source = "SUM(Sales[Cost A1 USD])+I3+$N$1";
    let origin = CellRef::parse("K3").unwrap();
    assert_eq!(
        translate_formula(source, origin, CellRef::parse("K4").unwrap()),
        "SUM(Sales[Cost A1 USD])+I4+$N$1"
    );
    assert_eq!(
        transpose_formula(source, origin, CellRef::parse("L4").unwrap()),
        "SUM(Sales[Cost A1 USD])+L2+$N$1"
    );
}
