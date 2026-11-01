#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_filterxml_spills_the_values_selected() {
    let mut model = new_empty_model();
    model._set("B1", "<list><n>3</n><n>5</n><x>a</x></list>");
    model._set("A1", "=FILTERXML(B1,\"//n\")");
    model._set("C1", "=SUM(FILTERXML(B1,\"//n\"))");
    model._set("D1", "=FILTERXML(B1,\"/list/x\")");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "3");
    assert_eq!(model._get_text("A2"), "5");
    assert_eq!(model._get_text("C1"), "8", "numbers come back as numbers");
    assert_eq!(model._get_text("D1"), "a");
}

#[test]
fn test_filterxml_errors() {
    let mut model = new_empty_model();
    model._set("A1", "=FILTERXML(\"<a><b></a>\",\"//b\")");
    model._set("A2", "=FILTERXML(\"<a/>\",\"//b\")");
    model._set("A3", "=FILTERXML(\"<a/>\")");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "#VALUE!");
    assert_eq!(model._get_text("A2"), "#VALUE!");
    assert_eq!(model._get_text("A3"), "#ERROR!");
}
