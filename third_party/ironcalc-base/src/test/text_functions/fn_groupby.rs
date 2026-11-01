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
