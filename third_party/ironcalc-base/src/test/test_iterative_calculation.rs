#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn without_iteration_a_cycle_is_an_error() {
    let mut model = new_empty_model();
    model._set("A1", "=A1+1");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#CIRC!");
}

#[test]
fn a_counter_goes_round_the_iterations() {
    // Excel, 100 iterations: =A1+1 shows 100, and each recalculation adds
    // another 100.
    let mut model = new_empty_model();
    model.set_iterative_calculation(Some((100, 0.001)));
    model._set("A1", "=A1+1");
    model._set("B1", "=A1*2");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"100");
    assert_eq!(model._get_text("B1"), *"200");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"200");
}

#[test]
fn a_cycle_settles_within_the_maximum_change() {
    // Newton's square root of 2, and the classic interest on a balance
    // that includes the interest.
    let mut model = new_empty_model();
    model.set_iterative_calculation(Some((100, 0.000001)));
    model._set("A1", "=IF(A1=0,1,(A1+2/A1)/2)");
    model._set("B1", "1000");
    model._set("B2", "=B1+B3");
    model._set("B3", "=B2*0.1");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"1.414213562");
    // B2 = 1000 / 0.9, to within the maximum change.
    let value = |cell: &str| model._get_text(cell).parse::<f64>().unwrap();
    assert!((value("B2") - 1000.0 / 0.9).abs() < 0.00001);
    assert!((value("B3") - 100.0 / 0.9).abs() < 0.00001);
}

#[test]
fn the_maximum_iterations_bounds_the_passes() {
    let mut model = new_empty_model();
    model.set_iterative_calculation(Some((10, 0.001)));
    model._set("A1", "=A1+1");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"10");
    // From the value it was saved with.
    let mut model = new_empty_model();
    model.set_iterative_calculation(Some((1, 0.001)));
    model._set("A1", "=A1+1");
    model
        .set_formula_cached_value(0, 1, 1, crate::types::FormulaValue::Number(41.0))
        .unwrap();
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"42");
}

#[test]
fn precision_as_displayed_keeps_what_is_shown() {
    let mut model = new_empty_model();
    model._set("A1", "=1/3");
    model._set("A2", "=A1*3");
    model._set("B1", "=1/3");
    model._set("B2", "=B1*3");
    model.set_precision_as_displayed(Some(Box::new(|_, row, column| {
        (column == 1).then(|| if row == 1 { "0.00" } else { "0.0000" }.to_string())
    })));
    model.evaluate();
    // A1 shows 0.33 and holds 0.33, so A2 is 0.99.
    assert_eq!(model._get_text("A2"), *"0.99");
    // Column B has no format given: full precision.
    assert_eq!(model._get_text("B2"), *"1");
}
