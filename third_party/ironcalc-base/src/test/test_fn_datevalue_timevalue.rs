#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn timevalue_accepts_localized_month_name_date_prefixes() {
    for (locale, text) in [
        ("en", "22 June 2000 18:45"),
        ("en", "22 JUNE 2000 6:45 PM"),
        ("en", "June 22, 2000 18:45:00"),
        ("de", "22 JUNI 2000 18:45"),
        ("de", "22.06.2000 18:45"),
        ("fr", "22 JUIN 2000 18:45"),
        ("en", "29 February 1900 18:45"),
    ] {
        let mut model = new_empty_model();
        model.set_locale(locale).unwrap();
        model._set("A1", &format!("=TIMEVALUE(\"{text}\")"));
        model.evaluate();
        // 18:45 is exactly representable as 25/32 of a day.
        assert_eq!(
            model.get_cell_value_by_index(0, 1, 1).unwrap(),
            crate::cell::CellValue::Number(0.78125),
            "{locale}: {text}"
        );
    }
    for text in ["31 June 2000 18:45", "22 June 2000 25:45", "junk 18:45"] {
        let mut model = new_empty_model();
        model._set("A1", &format!("=TIMEVALUE(\"{text}\")"));
        model.evaluate();
        assert_eq!(model._get_text("A1"), "#VALUE!", "{text}");
    }
}

#[test]
fn datevalue_and_days_follow_regional_date_order_and_month_names() {
    for (locale, date, expected) in [
        ("en", "01/02/2024", "45293"),
        ("de", "01.02.2024", "45323"),
        ("de", "01/02/2024", "45323"),
        ("fr", "01/02/2024", "45323"),
        ("de", "1 MÄRZ 2024", "45352"),
        ("fr", "1 FÉVRIER 2024", "45323"),
        ("de", "2024-02-01T06:45", "45323"),
    ] {
        let mut model = new_empty_model();
        model.set_locale(locale).unwrap();
        model._set("A1", &format!("=DATEVALUE(\"{date}\")"));
        let separator = if locale == "en" { ',' } else { ';' };
        model._set("A2", &format!("=DAYS(\"{date}\"{separator}0)"));
        model.evaluate();
        assert_eq!(model._get_text("A1"), expected, "{locale}: {date}");
        assert_eq!(model._get_text("A2"), expected, "{locale}: {date}");
    }
    for (locale, date) in [("en", "22/06/2000"), ("de", "06/22/2000")] {
        let mut model = new_empty_model();
        model.set_locale(locale).unwrap();
        model._set("A1", &format!("=DATEVALUE(\"{date}\")"));
        model.evaluate();
        assert_eq!(model._get_text("A1"), "#VALUE!");
    }
}

#[test]
fn datevalue_preserves_month_names_and_date_spaces() {
    for text in [
        "2 OCT 2023",
        "OCTOBER 2, 2023",
        "2-OCT-2023",
        "2 October 2023 06:45",
        "2023-10-02T06:45",
    ] {
        let mut model = new_empty_model();
        model._set("A1", &format!("=DATEVALUE(\"{text}\")"));
        model._set("A2", &format!("=DAYS(\"{text}\",DATE(2023,10,1))"));
        model._set("A3", "=DATEVALUE(\"2 OCTOBER junk\")");
        model.evaluate();
        assert_eq!(model._get_text("A1"), "45201", "{text}");
        assert_eq!(model._get_text("A2"), "1", "{text}");
        assert_eq!(model._get_text("A3"), "#VALUE!");
    }
}

#[test]
fn datevalue_timevalue_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=DATEVALUE()");
    model._set("A2", "=TIMEVALUE()");
    model._set("A3", "=DATEVALUE(\"2000-01-01\")");
    model._set("A4", "=TIMEVALUE(\"12:00:00\")");
    model._set("A5", "=DATEVALUE(1,2)");
    model._set("A6", "=TIMEVALUE(1,2)");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#ERROR!");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
    assert_eq!(model._get_text("A3"), *"36526");
    assert_eq!(model._get_text("A4"), *"0.5");
    assert_eq!(model._get_text("A5"), *"#ERROR!");
    assert_eq!(model._get_text("A6"), *"#ERROR!");
}
