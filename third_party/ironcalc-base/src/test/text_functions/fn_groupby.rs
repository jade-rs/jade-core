#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

fn fill(model: &mut crate::Model) {
    for (row, (region, amount)) in [
        ("Region", "Sales"),
        ("West", "10"),
        ("East", "5"),
        ("West", "20"),
        ("North", "7"),
        ("East", "1"),
    ]
    .iter()
    .enumerate()
    {
        model._set(&format!("A{}", row + 1), region);
        model._set(&format!("B{}", row + 1), amount);
    }
}

#[test]
fn test_groupby_sums_by_name_sorted_with_a_total() {
    let mut model = new_empty_model();
    fill(&mut model);
    model._set("D1", "=GROUPBY(A2:A6,B2:B6,SUM)");
    model.evaluate();
    let column = |c: &str| {
        (1..=4)
            .map(|r| model._get_text(&format!("{c}{r}")))
            .collect::<Vec<_>>()
    };
    assert_eq!(column("D"), ["East", "North", "West", "Total"]);
    assert_eq!(column("E"), ["6", "7", "30", "43"]);
}

#[test]
fn test_groupby_with_a_lambda_headers_descending_and_no_total() {
    let mut model = new_empty_model();
    fill(&mut model);
    model._set("D1", "=GROUPBY(A1:A6,B1:B6,LAMBDA(v,MAX(v)),3,0,-2)");
    model.evaluate();
    let column = |c: &str| {
        (1..=5)
            .map(|r| model._get_text(&format!("{c}{r}")))
            .collect::<Vec<_>>()
    };
    assert_eq!(column("D"), ["Region", "West", "North", "East", ""]);
    assert_eq!(column("E"), ["Sales", "20", "7", "5", ""]);
}

#[test]
fn test_groupby_filters_and_counts() {
    let mut model = new_empty_model();
    fill(&mut model);
    model._set("D1", "=GROUPBY(A2:A6,B2:B6,COUNT,,-1,,B2:B6>4)");
    model.evaluate();
    // The total first, then the groups the filter leaves.
    assert_eq!(model._get_text("D1"), "Total");
    assert_eq!(model._get_text("E1"), "4");
    assert_eq!(model._get_text("D2"), "East");
    assert_eq!(model._get_text("E2"), "1");
    assert_eq!(model._get_text("D4"), "West");
    assert_eq!(model._get_text("E4"), "2");
}

#[test]
fn test_count_counts_the_numbers_in_an_array() {
    let mut model = new_empty_model();
    model._set("A1", "=COUNT({1;2;\"x\";TRUE})");
    model._set("A2", "=COUNT({1,2,3},4)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "2");
    assert_eq!(model._get_text("A2"), "4");
}

#[test]
fn test_groupby_missing_header_option_detects_text_then_number() {
    let mut model = new_empty_model();
    fill(&mut model);
    model._set("D1", "=GROUPBY(A1:A6,B1:B6,SUM)");
    model.evaluate();
    let labels = (1..=4)
        .map(|r| model._get_text(&format!("D{r}")))
        .collect::<Vec<_>>();
    assert_eq!(labels, ["East", "North", "West", "Total"]);
    assert_eq!(model._get_text("E4"), "43");
}

#[test]
fn test_groupby_empty_header_option_also_detects_headers() {
    let mut model = new_empty_model();
    fill(&mut model);
    model._set("D1", "=GROUPBY(A1:A6,B1:B6,SUM,,0)");
    model.evaluate();
    assert_eq!(model._get_text("D1"), "East");
    assert_eq!(model._get_text("D3"), "West");
    assert_eq!(model._get_text("D4"), "");
    assert_eq!(model._get_text("E3"), "30");
}

#[test]
fn test_groupby_explicit_no_headers_preserves_the_first_text_row() {
    let mut model = new_empty_model();
    fill(&mut model);
    model._set("D1", "=GROUPBY(A1:A6,B1:B6,SUM,0)");
    model.evaluate();
    assert_eq!(model._get_text("D3"), "Region");
    assert_eq!(model._get_text("E3"), "0");
    assert_eq!(model._get_text("D5"), "Total");
    assert_eq!(model._get_text("E5"), "43");
}

#[test]
fn test_groupby_auto_headers_are_shown_for_multiple_value_columns() {
    let mut model = new_empty_model();
    fill(&mut model);
    for (row, value) in ["Units", "1", "2", "3", "4", "5"].iter().enumerate() {
        model._set(&format!("C{}", row + 1), value);
    }
    model._set("E1", "=GROUPBY(A1:A6,B1:C6,SUM)");
    model.evaluate();
    assert_eq!(model._get_text("E1"), "Region");
    assert_eq!(model._get_text("F1"), "Sales");
    assert_eq!(model._get_text("G1"), "Units");
    assert_eq!(model._get_text("E2"), "East");
    assert_eq!(model._get_text("F2"), "6");
    assert_eq!(model._get_text("G2"), "7");
    assert_eq!(model._get_text("E5"), "Total");
    assert_eq!(model._get_text("G5"), "15");
}

#[test]
fn test_groupby_auto_headers_require_a_second_numeric_value() {
    let mut model = new_empty_model();
    model._set("A1", "West");
    model._set("B1", "Text");
    model._set("A2", "East");
    model._set("B2", "More text");
    model._set("D1", "=GROUPBY(A1:A2,B1:B2,COUNTA)");
    model.evaluate();
    assert_eq!(model._get_text("D1"), "East");
    assert_eq!(model._get_text("D2"), "West");
    assert_eq!(model._get_text("D3"), "Total");
    assert_eq!(model._get_text("E3"), "2");
}

#[test]
fn test_groupby_subtotals_each_first_key() {
    let mut model = new_empty_model();
    for (row, (region, product, amount)) in [
        ("West", "Tea", "10"),
        ("East", "Tea", "5"),
        ("West", "Rice", "20"),
        ("East", "Rice", "1"),
        ("West", "Tea", "3"),
    ]
    .iter()
    .enumerate()
    {
        model._set(&format!("A{}", row + 1), region);
        model._set(&format!("B{}", row + 1), product);
        model._set(&format!("C{}", row + 1), amount);
    }
    model._set("E1", "=GROUPBY(A1:B5,C1:C5,SUM,0,2)");
    model._set("I1", "=GROUPBY(A1:B5,C1:C5,SUM,0,-2)");
    model.evaluate();
    let grid = |columns: [&str; 3], rows: u32| {
        (1..=rows)
            .map(|row| {
                columns
                    .iter()
                    .map(|column| model._get_text(&format!("{column}{row}")))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        grid(["E", "F", "G"], 7),
        [
            ["East", "Rice", "1"],
            ["East", "Tea", "5"],
            ["East", "", "6"],
            ["West", "Rice", "20"],
            ["West", "Tea", "13"],
            ["West", "", "33"],
            ["Total", "", "39"],
        ]
    );
    // Subtotals and the total before their rows.
    assert_eq!(
        grid(["I", "J", "K"], 7),
        [
            ["Total", "", "39"],
            ["East", "", "6"],
            ["East", "Rice", "1"],
            ["East", "Tea", "5"],
            ["West", "", "33"],
            ["West", "Rice", "20"],
            ["West", "Tea", "13"],
        ]
    );
}

#[test]
fn test_groupby_percentof_against_the_grand_total() {
    let mut model = new_empty_model();
    fill(&mut model);
    model._set("D1", "=GROUPBY(A2:A6,B2:B6,PERCENTOF,0,0)");
    model._set(
        "G1",
        "=GROUPBY(A2:A6,B2:B6,LAMBDA(x,all,SUM(x)/SUM(all)),0,0)",
    );
    model.evaluate();
    // East 6, North 7, West 30 of 43.
    let share = |cell: &str| model._get_text(cell).parse::<f64>().unwrap();
    assert!(
        (share("E1") - 6.0 / 43.0).abs() < 1e-9,
        "{}",
        model._get_text("E1")
    );
    assert!((share("E3") - 30.0 / 43.0).abs() < 1e-9);
    assert!((share("H2") - 7.0 / 43.0).abs() < 1e-9);
}
