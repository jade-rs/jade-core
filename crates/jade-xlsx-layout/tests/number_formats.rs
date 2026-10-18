use jade_xlsx_layout::number_format::{format_value, FormatColor, Formatted};
use jade_xlsx_reader::{builtin_number_format, CellValue, DateSystem};

fn number(value: f64, code: Option<&str>) -> String {
    format_value(&CellValue::Number(value), code, DateSystem::Nineteen00).text
}

fn formatted(value: f64, code: &str) -> Formatted {
    format_value(
        &CellValue::Number(value),
        Some(code),
        DateSystem::Nineteen00,
    )
}

fn in_1904(value: f64, code: &str) -> String {
    format_value(
        &CellValue::Number(value),
        Some(code),
        DateSystem::Nineteen04,
    )
    .text
}

// ── numbers ─────────────────────────────────────────────────────────────

#[test]
fn general_drops_trailing_zeros_and_the_point() {
    assert_eq!(number(1234.5, None), "1234.5");
    assert_eq!(number(1234.0, None), "1234");
    assert_eq!(number(0.5, Some("General")), "0.5");
}

#[test]
fn fixed_decimals_round_away_from_zero() {
    // Rust's formatter rounds half to even, which would print 1234 here.
    assert_eq!(number(1234.567, Some("0.00")), "1234.57");
    assert_eq!(number(1234.5, Some("0")), "1235");
}

#[test]
fn optional_decimal_places_show_only_significant_fraction_digits() {
    assert_eq!(number(12.345, Some("0.##")), "12.35");
    assert_eq!(number(12.3, Some("0.##")), "12.3");
    // Excel keeps a written point even with no digits after it.
    assert_eq!(number(12.0, Some("0.##")), "12.");
    assert_eq!(number(12.3, Some("0.00##")), "12.30");
    assert_eq!(number(1234.5, Some("#,##0.##")), "1,234.5");
}

#[test]
fn thousands_are_grouped() {
    assert_eq!(number(1234567.0, Some("#,##0")), "1,234,567");
    assert_eq!(number(1234567.891, Some("#,##0.00")), "1,234,567.89");
    assert_eq!(number(123.0, Some("#,##0")), "123");
}

#[test]
fn a_trailing_comma_scales_by_a_thousand() {
    assert_eq!(number(1_234_567.0, Some("#,##0,")), "1,235");
    assert_eq!(number(1_234_567_000.0, Some("#,##0,,")), "1,235");
}

#[test]
fn literals_keep_their_place_around_the_number() {
    assert_eq!(number(1234.5, Some(r#"#,##0.00"円""#)), "1,234.50円");
    assert_eq!(number(1234.5, Some(r##""¥"#,##0"##)), "¥1,235");
}

#[test]
fn a_currency_tag_prints_its_symbol() {
    // `[$¥-411]` is a currency plus a locale; the symbol is a literal and the
    // locale is not implemented.
    assert_eq!(number(1234.0, Some("[$¥-411]#,##0")), "¥1,234");
    assert_eq!(number(1234.0, Some("[$-411]#,##0")), "1,234");
}

#[test]
fn percent_scales_by_a_hundred() {
    assert_eq!(number(0.125, Some("0.0%")), "12.5%");
}

#[test]
fn scientific_notation_signs_and_pads_the_exponent() {
    assert_eq!(number(12345.0, Some("0.00E+00")), "1.23E+04");
    assert_eq!(number(0.00012, Some("0.00E+00")), "1.20E-04");
}

#[test]
fn built_in_elapsed_and_engineering_formats_match_excel_ids() {
    assert_eq!(
        number(
            0.5208333333,
            Some(builtin_number_format(45).expect("built-in minute and second"))
        ),
        "30:00"
    );
    assert_eq!(
        number(
            1.5,
            Some(builtin_number_format(46).expect("built-in elapsed time"))
        ),
        "36:00:00"
    );
    assert_eq!(
        number(
            0.52083475694,
            Some(builtin_number_format(47).expect("built-in compact time"))
        ),
        "3000.1"
    );
    assert_eq!(
        number(
            12_200_000.0,
            Some(builtin_number_format(48).expect("built-in engineering notation"))
        ),
        "12.2E+6"
    );
}

#[test]
fn fractions_use_the_requested_denominator_width() {
    assert_eq!(number(2.25, Some("# ?/?")), "2 1/4");
    assert_eq!(number(0.25, Some("# ?/?")), "1/4");
    assert_eq!(number(2.125, Some("# ??/??")), "2 1/8");
    assert_eq!(number(0.333, Some("?/??")), "1/3");
    assert_eq!(number(2.26, Some("# ?/8")), "2 2/8");
    assert_eq!(number(2.0, Some("# ?/?")), "2");
}

#[test]
fn built_in_fraction_formats_match_excel_ids() {
    assert_eq!(
        number(
            2.25,
            Some(builtin_number_format(12).expect("built-in # ?/?"))
        ),
        "2 1/4"
    );
    assert_eq!(
        number(
            2.125,
            Some(builtin_number_format(13).expect("built-in # ??/??"))
        ),
        "2 1/8"
    );
}

#[test]
fn built_in_currency_formats_match_excel_ids() {
    assert_eq!(
        number(
            1234.5,
            Some(builtin_number_format(5).expect("built-in dollar integer"))
        ),
        "$1,235"
    );
    assert_eq!(
        number(
            -1234.5,
            Some(builtin_number_format(5).expect("built-in dollar negative"))
        ),
        "($1,235)"
    );
    let red = formatted(
        -1234.5,
        builtin_number_format(8).expect("built-in red dollar decimals"),
    );
    assert_eq!(red.text, "($1,234.50)");
    assert_eq!(red.color, Some(FormatColor::Red));
}

#[test]
fn built_in_accounting_formats_match_excel_ids() {
    let dollars = builtin_number_format(44).expect("built-in dollar accounting decimals");
    assert_eq!(number(1234.5, Some(dollars)), "$1,234.50");
    assert_eq!(number(-1234.5, Some(dollars)), "($1,234.50)");
    assert_eq!(number(0.0, Some(dollars)), "$-");
}

// ── sections ────────────────────────────────────────────────────────────

#[test]
fn one_section_keeps_the_minus_sign() {
    assert_eq!(number(-1234.5, Some("#,##0.00")), "-1,234.50");
}

#[test]
fn a_negative_section_supplies_its_own_sign() {
    // The section is applied to the magnitude, so the parentheses are the sign.
    assert_eq!(number(-1234.0, Some("#,##0;(#,##0)")), "(1,234)");
    assert_eq!(number(1234.0, Some("#,##0;(#,##0)")), "1,234");
}

#[test]
fn built_in_negative_number_formats_keep_excel_representation() {
    let accounting = builtin_number_format(37).expect("built-in format 37");
    assert_eq!(number(-1234.0, Some(accounting)), "(1,234)");

    let red_accounting = builtin_number_format(40).expect("built-in format 40");
    let shown = formatted(-1234.5, red_accounting);
    assert_eq!(shown.text, "(1,234.50)");
    assert_eq!(shown.color, Some(FormatColor::Red));
}

#[test]
fn a_zero_section_applies_only_to_zero() {
    assert_eq!(number(0.0, Some(r#"#,##0;-#,##0;"-""#)), "-");
    assert_eq!(number(5.0, Some(r#"#,##0;-#,##0;"-""#)), "5");
}

#[test]
fn an_empty_section_hides_the_value() {
    // `0;;` is the usual way to show positives and nothing else.
    assert_eq!(number(5.0, Some("0;;")), "5");
    assert_eq!(number(-5.0, Some("0;;")), "");
    assert_eq!(number(0.0, Some("0;;")), "");
}

#[test]
fn the_text_section_places_the_text() {
    let formatted = format_value(
        &CellValue::Text("hello".into()),
        Some(r#"0;-0;0;"<"@">""#),
        DateSystem::Nineteen00,
    );
    assert_eq!(formatted.text, "<hello>");
}

#[test]
fn a_colour_comes_back_alongside_the_text() {
    let negative = formatted(-1234.0, "#,##0;[Red]-#,##0");
    assert_eq!(negative.text, "-1,234");
    assert_eq!(negative.color, Some(FormatColor::Red));

    // The positive section names no colour, so the font's own is left alone.
    assert_eq!(formatted(1234.0, "#,##0;[Red]-#,##0").color, None);
    assert_eq!(
        formatted(1.0, "[Color12]0").color,
        Some(FormatColor::Indexed(12))
    );
}

#[test]
fn conditional_sections_select_the_matching_representation() {
    assert_eq!(number(1500.0, Some("[>100]#,##0;0")), "1,500");
    assert_eq!(number(50.0, Some("[>100]#,##0;0")), "50");

    let negative = formatted(-5.0, "[>=0]0;[Red][<0]0");
    assert_eq!(negative.text, "-5");
    assert_eq!(negative.color, Some(FormatColor::Red));
}

// ── dates ───────────────────────────────────────────────────────────────

#[test]
fn a_serial_becomes_a_date() {
    assert_eq!(number(45000.0, Some("yyyy/mm/dd")), "2023/03/15");
    assert_eq!(number(45000.0, Some("m/d/yy")), "3/15/23");
    assert_eq!(number(45000.0, Some("d-mmm-yy")), "15-Mar-23");
    assert_eq!(number(45000.0, Some("mmmm d, yyyy")), "March 15, 2023");
    assert_eq!(number(45000.0, Some("mmmmm")), "M");
}

#[test]
fn weekday_names_come_from_the_date() {
    // 2023-03-15 was a Wednesday.
    assert_eq!(number(45000.0, Some("dddd")), "Wednesday");
    assert_eq!(number(45000.0, Some("ddd")), "Wed");
}

#[test]
fn excels_phantom_leap_day_is_kept() {
    // Lotus treated 1900 as a leap year and Excel kept the bug, so serial 60 is
    // a 29 February that never happened and 61 is 1 March.
    assert_eq!(number(1.0, Some("yyyy-mm-dd")), "1900-01-01");
    assert_eq!(number(59.0, Some("yyyy-mm-dd")), "1900-02-28");
    assert_eq!(number(60.0, Some("yyyy-mm-dd")), "1900-02-29");
    assert_eq!(number(61.0, Some("yyyy-mm-dd")), "1900-03-01");
}

#[test]
fn the_1904_system_shifts_the_epoch() {
    assert_eq!(in_1904(0.0, "yyyy-mm-dd"), "1904-01-01");
    // The same serial is four years and a day apart between the two systems.
    assert_eq!(in_1904(45000.0, "yyyy-mm-dd"), "2027-03-16");
}

#[test]
fn the_fraction_of_a_day_is_the_time() {
    assert_eq!(number(45000.5, Some("hh:mm")), "12:00");
    assert_eq!(number(45000.75, Some("h:mm AM/PM")), "6:00 PM");
    assert_eq!(number(45000.25, Some("h:mm AM/PM")), "6:00 AM");
    assert_eq!(number(0.5208333333, Some("hh:mm:ss")), "12:30:00");
}

#[test]
fn fractional_seconds_are_shown_and_carry_into_the_next_second() {
    let with_millis = 45_000.0 + (12.0 * 3_600.0 + 30.0 * 60.0 + 0.123) / 86_400.0;
    assert_eq!(number(with_millis, Some("hh:mm:ss.000")), "12:30:00.123");
    assert_eq!(number(with_millis, Some("hh:mm:ss.0")), "12:30:00.1");

    let rounds_over = 45_000.0 + 86_399.999_6 / 86_400.0;
    assert_eq!(
        number(rounds_over, Some("yyyy-mm-dd hh:mm:ss.000")),
        "2023-03-16 00:00:00.000"
    );
}

#[test]
fn m_is_a_minute_between_an_hour_and_a_second() {
    // The ambiguity that makes `mm` mean two things: next to an hour it is a
    // minute, next to a year it is a month.
    assert_eq!(
        number(45000.5, Some("yyyy-mm-dd hh:mm:ss")),
        "2023-03-15 12:00:00"
    );
    assert_eq!(number(45000.5, Some("mm")), "03");
    assert_eq!(number(45000.5, Some("hh:mm")), "12:00");
}

#[test]
fn elapsed_time_accumulates_instead_of_wrapping() {
    // 1.5 days is 36 hours, not noon on the second day.
    assert_eq!(number(1.5, Some("[h]:mm")), "36:00");
    assert_eq!(number(0.5, Some("[m]")), "720");
}

#[test]
fn a_time_that_rounds_up_to_midnight_moves_to_the_next_day() {
    // 0.9999999 of a day is a hair before midnight; rounding it to the second
    // must not produce hour 24.
    assert_eq!(
        number(45000.9999999, Some("yyyy-mm-dd hh:mm:ss")),
        "2023-03-16 00:00:00"
    );
}

#[test]
fn a_quoted_letter_is_a_literal_not_a_date_token() {
    assert_eq!(number(5.0, Some(r#"0"m""#)), "5m");
}

#[test]
fn literals_among_digit_placeholders_stay_in_place() {
    // A space between placeholders is a literal: digits fill the
    // placeholders from the right, as Excel's space-grouping trick shows.
    assert_eq!(number(1500.5, Some("0 ???")), "1 501");
    assert_eq!(number(42.0, Some("0 ???")), "0  42");
}

// ── other values ────────────────────────────────────────────────────────

#[test]
fn non_numeric_values_ignore_the_code() {
    assert_eq!(
        format_value(
            &CellValue::Text("text".into()),
            Some("0.00"),
            DateSystem::Nineteen00
        )
        .text,
        "text"
    );
    assert_eq!(
        format_value(&CellValue::Bool(true), None, DateSystem::Nineteen00).text,
        "TRUE"
    );
    assert_eq!(
        format_value(
            &CellValue::Error("#REF!".into()),
            Some("0.00"),
            DateSystem::Nineteen00
        )
        .text,
        "#REF!"
    );
    assert_eq!(
        format_value(&CellValue::Empty, Some("0.00"), DateSystem::Nineteen00).text,
        ""
    );
}
