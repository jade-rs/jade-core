//! Sheet names follow Excel's rules: 1 to 31 characters, none of
//! `[ ] : * ? / \`, no apostrophe at either end, not the reserved `History`,
//! and unique without regard to case.

use jade_xlsx_writer::sheets::validate_sheet_name;
use jade_xlsx_writer::WriteError;
use proptest::prelude::*;

#[test]
fn names_excel_accepts_are_accepted() {
    for name in [
        "Sheet1",
        "Q3 2026",
        "O'Neil",
        "売上サマリ",
        "a",
        "1234567890123456789012345678901",
        // 31 characters of a script outside ASCII.
        "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほま",
        "#REF",
        "History of sales",
    ] {
        assert!(
            validate_sheet_name(name, &["Other"]).is_ok(),
            "{name}: {:?}",
            validate_sheet_name(name, &["Other"])
        );
    }
}

#[test]
fn names_excel_refuses_are_refused() {
    for name in [
        "",
        "12345678901234567890123456789012",
        "a/b",
        "a\\b",
        "a?",
        "a*",
        "[a]",
        "a:b",
        "'lead",
        "trail'",
        "history",
        "HISTORY",
        "tab\there",
    ] {
        assert!(
            matches!(
                validate_sheet_name(name, &[]),
                Err(WriteError::InvalidSheetName { .. })
            ),
            "{name}"
        );
    }
    for (name, existing) in [("notes", "Notes"), ("Notes", "NOTES"), ("ÄRGER", "ärger")] {
        assert!(
            matches!(
                validate_sheet_name(name, &[existing]),
                Err(WriteError::DuplicateSheetName(_))
            ),
            "{name}"
        );
    }
}

proptest! {
    /// The rules, restated independently, agree with the validator on any
    /// string.
    #[test]
    fn the_validator_applies_exactly_the_rules(
        name in prop_oneof![
            "\\PC{0,40}",
            "[a-zA-Z0-9 '\\[\\]:*?/\\\\]{0,34}",
            Just("History".to_string()),
        ],
    ) {
        let valid = !name.is_empty()
            && name.encode_utf16().count() <= 31
            && !name.chars().any(|c| matches!(c, '[' | ']' | ':' | '*' | '?' | '/' | '\\') || c.is_control())
            && !name.starts_with('\'')
            && !name.ends_with('\'')
            && !name.eq_ignore_ascii_case("history");
        prop_assert_eq!(validate_sheet_name(&name, &[]).is_ok(), valid, "{:?}", name);
        if valid {
            prop_assert!(matches!(
                validate_sheet_name(&name, &[&name.to_uppercase()]),
                Err(WriteError::DuplicateSheetName(_))
            ));
        }
    }
}
