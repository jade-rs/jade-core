//! Excel refuses a formula that gives a function too few or too many
//! arguments ("You've entered too few arguments for this function"), so a
//! shell can refuse it as it is typed instead of storing a formula that only
//! ever reads `#ERROR!`. Each case: a formula, and whether Excel refuses it.

use jade_xlsx_calc::formula_text::wrong_argument_count;

const CASES: &[(&str, bool)] = &[
    ("SUM()", true),
    ("ROUND(1)", true),
    ("IF(1)", true),
    ("ABS(1,2)", true),
    ("LEN(\"a\",\"b\")", true),
    // Not TODAY(1): the engine takes a time zone there, which Excel does not
    // have, and refusing less than Excel is the safe side.
    ("IF(SUM(),1,2)", true),
    ("SUM(1)", false),
    ("SUM(A1:A3,4)", false),
    ("ROUND(1.25,1)", false),
    ("IF(1,2)", false),
    ("LEFT(\"ab\")", false),
    ("NOW()", false),
    ("PI()", false),
    ("A1+1", false),
    ("SEQUENCE(3)", false),
    ("VLOOKUP(1,A1:B2,2)", false),
    ("NOTAFUNCTION(1)", false),
    ("1+", false),
];

#[test]
fn a_wrong_argument_count_is_found_as_excel_finds_it() {
    let wrong: Vec<String> = CASES
        .iter()
        .filter(|(formula, refused)| wrong_argument_count(formula) != *refused)
        .map(|(formula, refused)| format!("{formula}: Excel refuses it: {refused}"))
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
