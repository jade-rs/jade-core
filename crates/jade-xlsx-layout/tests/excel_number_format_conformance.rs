//! What Excel shows for a value under a number format, case by case.
//!
//! A workbook computed right still reads wrong if `45306` under
//! `ggge"年"m"月"d"日"` shows as anything but 令和6年1月15日. Each case is a
//! value, a format code and the text Excel (Microsoft 365) displays — en-US
//! for the plain codes, Japanese for the `[$-411]` ones — covering what
//! business workbooks use: General's significant digits, separators and
//! scaling, percent and scientific, currency, sections with colours and
//! conditions, text, dates and times, elapsed time, fractions, and the
//! Japanese era and weekday codes. Every mismatch is listed at once.

use jade_xlsx_layout::number_format::{format_value, FormatColor};
use jade_xlsx_reader::{CellValue, DateSystem};

/// 2024-01-15, a Monday.
const JAN_15_2024: f64 = 45306.0;
/// 2019-04-30, the last day of Heisei.
const APR_30_2019: f64 = 43585.0;

const CASES: &[(f64, &str, &str)] = &[
    // ---- General ------------------------------------------------------------
    (1234.5, "General", "1234.5"),
    (-1234.5, "General", "-1234.5"),
    (123456789.0, "General", "123456789"),
    (1234567890123.0, "General", "1.23457E+12"),
    (0.0000000001234, "General", "1.234E-10"),
    (123456789012.0, "General", "1.23457E+11"),
    (0.6666666666666666, "General", "0.666666667"),
    (123.4567890123, "General", "123.456789"),
    (0.00001, "General", "0.00001"),
    (0.30000000000000004, "General", "0.3"),
    (0.3333333333333333, "General", "0.333333333"),
    // ---- numbers -------------------------------------------------------------
    (1234.567, "#,##0.00", "1,234.57"),
    (-1234.567, "#,##0.00", "-1,234.57"),
    (0.0, "#,##0.00", "0.00"),
    (1234567.0, "#,##0", "1,234,567"),
    (1234567.0, "#,##0,", "1,235"),
    (1234567.0, "0.0,,\"M\"", "1.2M"),
    (0.5, "0%", "50%"),
    (0.1234, "0.00%", "12.34%"),
    (12345.678, "0.00E+00", "1.23E+04"),
    (0.00012, "0.0E+0", "1.2E-4"),
    (5.0, "000", "005"),
    (1234567.0, "000-0000", "123-4567"),
    (0.0, "#", ""),
    (1.0, "0.##", "1."),
    (1234.5, "$#,##0.00", "$1,234.50"),
    (1234.0, "\\¥#,##0", "¥1,234"),
    (1234.0, "[$¥-411]#,##0", "¥1,234"),
    (-1234.0, "[$¥-411]#,##0;[$¥-411]-#,##0", "¥-1,234"),
    (-5.0, "#,##0;[Red]-#,##0", "-5"),
    (-5.0, "0.00;(0.00)", "(5.00)"),
    (0.0, "0;-0;\"zero\"", "zero"),
    (5.0, "0.00_)", "5.00 "),
    (1500.0, "[>=1000]#,##0,\"K\";0", "2K"),
    (500.0, "[>=1000]#,##0,\"K\";0", "500"),
    // ---- dates and times -------------------------------------------------------
    (JAN_15_2024, "yyyy/m/d", "2024/1/15"),
    (JAN_15_2024, "yyyy-mm-dd", "2024-01-15"),
    (JAN_15_2024, "m/d/yyyy", "1/15/2024"),
    (JAN_15_2024, "d-mmm-yy", "15-Jan-24"),
    (JAN_15_2024, "mmmm d, yyyy", "January 15, 2024"),
    (JAN_15_2024, "dddd", "Monday"),
    (JAN_15_2024, "ddd", "Mon"),
    (JAN_15_2024, "mmmmm", "J"),
    (JAN_15_2024 + 0.5, "yyyy/m/d h:mm", "2024/1/15 12:00"),
    (0.75, "h:mm AM/PM", "6:00 PM"),
    (0.0, "h:mm AM/PM", "12:00 AM"),
    (1.5, "[h]:mm", "36:00"),
    (61.0 / 1440.0, "[mm]:ss", "61:00"),
    (12.5 / 86400.0, "mm:ss.0", "00:12.5"),
    // ---- fractions ---------------------------------------------------------------
    (1.5, "# ?/?", "1 1/2"),
    (0.75, "?/?", "3/4"),
    (1.375, "# ?/8", "1 3/8"),
    (-1.5, "# ?/?", "-1 1/2"),
    // ---- Japanese ------------------------------------------------------------------
    (
        JAN_15_2024,
        "[$-411]ggge\"年\"m\"月\"d\"日\"",
        "令和6年1月15日",
    ),
    (JAN_15_2024, "[$-411]gge", "令6"),
    (JAN_15_2024, "[$-411]ge.m.d", "R6.1.15"),
    (JAN_15_2024, "[$-411]ggg ee", "令和 06"),
    (APR_30_2019, "[$-411]ggge\"年\"", "平成31年"),
    (JAN_15_2024, "[$-411]aaa", "月"),
    (JAN_15_2024, "[$-411]aaaa", "月曜日"),
    (
        JAN_15_2024,
        "[$-411]yyyy\"年\"m\"月\"d\"日\"(aaa)",
        "2024年1月15日(月)",
    ),
    (JAN_15_2024, "yyyy\"年\"m\"月\"d\"日\"", "2024年1月15日"),
];

#[test]
fn values_display_as_excel_displays_them() {
    let failures: Vec<String> = CASES
        .iter()
        .filter_map(|(value, code, expected)| {
            let shown = format_value(
                &CellValue::Number(*value),
                Some(code),
                DateSystem::Nineteen00,
            )
            .text;
            (shown != *expected)
                .then(|| format!("{value} as {code:?}: expected {expected:?}, shown {shown:?}"))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {} formats differ from Excel:\n{}",
        failures.len(),
        CASES.len(),
        failures.join("\n")
    );
}

#[test]
fn text_sections_and_colours_follow_excel() {
    let text = |code: &str| {
        format_value(
            &CellValue::Text("abc".into()),
            Some(code),
            DateSystem::Nineteen00,
        )
        .text
    };
    assert_eq!(text("@"), "abc");
    assert_eq!(text("0;0;0;\"[\"@\"]\""), "[abc]");
    // A number under a text format shows as General, as when Excel's
    // Number Format box is set to Text on a cell holding one.
    let number = |code: &str| {
        format_value(
            &CellValue::Number(1234.5),
            Some(code),
            DateSystem::Nineteen00,
        )
        .text
    };
    assert_eq!(number("@"), "1234.5");
    assert_eq!(number("\"x\"@"), "1234.5");
    assert_eq!(number("\"@\"0.0"), "@1234.5", "a quoted @ is a literal");
    let negative = format_value(
        &CellValue::Number(-5.0),
        Some("#,##0;[Red]-#,##0"),
        DateSystem::Nineteen00,
    );
    assert_eq!(negative.color, Some(FormatColor::Red));
}
