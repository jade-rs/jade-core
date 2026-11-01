#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

fn fill(model: &mut crate::Model) {
    for (row, (region, quarter, amount)) in [
        ("West", "Q1", "10"),
        ("East", "Q2", "5"),
        ("West", "Q2", "20"),
        ("East", "Q1", "1"),
        ("West", "Q1", "3"),
    ]
    .iter()
    .enumerate()
    {
        model._set(&format!("A{}", row + 1), region);
        model._set(&format!("B{}", row + 1), quarter);
        model._set(&format!("C{}", row + 1), amount);
    }
}

fn grid(model: &crate::Model, rows: u32, columns: &[&str]) -> Vec<Vec<String>> {
    (1..=rows)
        .map(|row| {
            columns
                .iter()
                .map(|column| model._get_text(&format!("{column}{row}")))
                .collect()
        })
        .collect()
}

#[test]
fn test_pivotby_crosses_rows_and_columns_with_totals() {
    let mut model = new_empty_model();
    fill(&mut model);
    model._set("E1", "=PIVOTBY(A1:A5,B1:B5,C1:C5,SUM)");
    model.evaluate();
    assert_eq!(
        grid(&model, 4, &["E", "F", "G", "H"]),
        [
            ["", "Q1", "Q2", "Total"],
            ["East", "1", "5", "6"],
            ["West", "13", "20", "33"],
            ["Total", "14", "25", "39"],
        ]
    );
}

#[test]
fn test_pivotby_without_totals_descending_with_a_lambda() {
    let mut model = new_empty_model();
    fill(&mut model);
    model._set(
        "E1",
        "=PIVOTBY(A1:A5,B1:B5,C1:C5,LAMBDA(v,COUNT(v)),0,0,-1,0,-1)",
    );
    model.evaluate();
    assert_eq!(
        grid(&model, 3, &["E", "F", "G"]),
        [["", "Q2", "Q1"], ["West", "1", "2"], ["East", "1", "1"]]
    );
    assert_eq!(model._get_text("H1"), "");
    assert_eq!(model._get_text("E4"), "");
}

#[test]
fn test_pivotby_wide_column_fields_and_multiple_value_headers() {
    let mut model = new_empty_model();
    for (row, values) in [
        [
            "Region", "Year", "Quarter", "Month", "Day", "Amount", "Count",
        ],
        ["West", "y1", "q1", "m1", "d1", "10", "1"],
        ["East", "y2", "q2", "m2", "d2", "20", "2"],
    ]
    .iter()
    .enumerate()
    {
        for (column, value) in ["A", "B", "C", "D", "E", "F", "G"].iter().zip(values) {
            model._set(&format!("{column}{}", row + 1), value);
        }
    }
    model._set("I1", "=PIVOTBY(A1:A3,B1:E3,F1:G3,SUM,3,0,1,0,1)");
    model.evaluate();
    assert_eq!(
        grid(&model, 7, &["I", "J", "K", "L", "M"]),
        [
            ["", "y1", "", "y2", ""],
            ["", "q1", "", "q2", ""],
            ["", "m1", "", "m2", ""],
            ["Region", "d1", "", "d2", ""],
            ["", "Amount", "Count", "Amount", "Count"],
            ["East", "", "", "20", "2"],
            ["West", "10", "1", "", ""],
        ]
    );
}
