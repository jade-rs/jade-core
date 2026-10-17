//! The number-format language is fed by files, not by this codebase, so the
//! formatter's first duty is to survive whatever a file says: any code and any
//! value must come back as text, never as a panic. Malformed codes fall
//! through to the general form rather than guessing.

use jade_xlsx_layout::number_format::format_value;
use jade_xlsx_reader::{CellValue, DateSystem};
use proptest::prelude::*;

fn any_value() -> impl Strategy<Value = CellValue> {
    prop_oneof![
        Just(CellValue::Empty),
        any::<f64>().prop_map(CellValue::Number),
        "[a-z0-9 #%._-]{0,12}".prop_map(CellValue::Text),
        any::<bool>().prop_map(CellValue::Bool),
        Just(CellValue::Error("#DIV/0!".into())),
    ]
}

/// Codes built from the format language's own vocabulary, malformed included:
/// unbalanced brackets, quotes and sections are all things real files contain.
fn format_code() -> impl Strategy<Value = String> {
    proptest::string::string_regex(r#"[0#?.,%@eE+\-/:;"\[\]$<>=ymdhsagr generl漢]{0,20}"#)
        .expect("regex")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn any_code_and_any_value_format_without_panicking(
        code in format_code(),
        value in any_value(),
        date_1904 in any::<bool>(),
    ) {
        let dates = if date_1904 {
            DateSystem::Nineteen04
        } else {
            DateSystem::Nineteen00
        };
        let _ = format_value(&value, Some(&code), dates);
        let _ = format_value(&value, None, dates);
    }

    #[test]
    fn a_finite_number_under_general_always_shows_something(
        number in any::<f64>().prop_filter("finite", |value| value.is_finite()),
    ) {
        let formatted = format_value(&CellValue::Number(number), None, DateSystem::Nineteen00);
        prop_assert!(!formatted.text.is_empty());
    }
}

#[test]
fn a_serial_past_the_calendar_falls_back_to_general() {
    // Found by the property above: `y` applied to 2.36e175 overflowed the day
    // arithmetic. Excel's calendar ends at 9999-12-31 (serial 2958465 in the
    // 1900 system); anything past it is not a date and shows the general form.
    let huge = format_value(
        &CellValue::Number(2.363721635666054e175),
        Some("y"),
        DateSystem::Nineteen00,
    );
    assert!(!huge.text.is_empty());

    let last_day = format_value(
        &CellValue::Number(2_958_465.0),
        Some("yyyy-mm-dd"),
        DateSystem::Nineteen00,
    );
    assert_eq!(last_day.text, "9999-12-31");

    let past = format_value(
        &CellValue::Number(2_958_466.0),
        Some("yyyy-mm-dd"),
        DateSystem::Nineteen00,
    );
    assert_eq!(past.text, "2958466");

    let last_1904 = format_value(
        &CellValue::Number(2_957_003.0),
        Some("yyyy-mm-dd"),
        DateSystem::Nineteen04,
    );
    assert_eq!(last_1904.text, "9999-12-31");
}
