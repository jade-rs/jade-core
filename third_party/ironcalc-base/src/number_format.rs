use crate::{
    formatter::{self, format::Formatted},
    locale::get_locale,
    types::NumFmt,
};

// The standard built-in number formats defined by ECMA-376 (§18.8.30), indexed by `numFmtId`
// so that `DEFAULT_NUM_FMTS[id]` is the format code for that built-in id. Ids 23–36 are
// reserved/undefined by the spec and resolve to "general". A workbook-defined `numFmt` always
// takes precedence over this table (see `get_num_fmt`), so this is only consulted for a
// referenced-but-undefined built-in id.
const DEFAULT_NUM_FMTS: &[&str] = &[
    "general",                                                              // 0
    "0",                                                                    // 1
    "0.00",                                                                 // 2
    "#,##0",                                                                // 3
    "#,##0.00",                                                             // 4
    "$#,##0_);($#,##0)",                                                    // 5
    "$#,##0_);[Red]($#,##0)",                                               // 6
    "$#,##0.00_);($#,##0.00)",                                              // 7
    "$#,##0.00_);[Red]($#,##0.00)",                                         // 8
    "0%",                                                                   // 9
    "0.00%",                                                                // 10
    "0.00E+00",                                                             // 11
    "# ?/?",                                                                // 12
    "# ??/??",                                                              // 13
    "mm-dd-yy",                                                             // 14
    "d-mmm-yy",                                                             // 15
    "d-mmm",                                                                // 16
    "mmm-yy",                                                               // 17
    "h:mm AM/PM",                                                           // 18
    "h:mm:ss AM/PM",                                                        // 19
    "h:mm",                                                                 // 20
    "h:mm:ss",                                                              // 21
    "m/d/yy h:mm",                                                          // 22
    "general",                                                              // 23 (reserved)
    "general",                                                              // 24 (reserved)
    "general",                                                              // 25 (reserved)
    "general",                                                              // 26 (reserved)
    "general",                                                              // 27 (reserved)
    "general",                                                              // 28 (reserved)
    "general",                                                              // 29 (reserved)
    "general",                                                              // 30 (reserved)
    "general",                                                              // 31 (reserved)
    "general",                                                              // 32 (reserved)
    "general",                                                              // 33 (reserved)
    "general",                                                              // 34 (reserved)
    "general",                                                              // 35 (reserved)
    "general",                                                              // 36 (reserved)
    "#,##0_);(#,##0)",                                                      // 37
    "#,##0_);[Red](#,##0)",                                                 // 38
    "#,##0.00_);(#,##0.00)",                                                // 39
    "#,##0.00_);[Red](#,##0.00)",                                           // 40
    "_(* #,##0_);_(* \\(#,##0\\);_(* \"-\"_);_(@_)",                        // 41
    "_(\"$\"* #,##0_);_(\"$\"* \\(#,##0\\);_(\"$\"* \"-\"_);_(@_)",         // 42
    "_(* #,##0.00_);_(* \\(#,##0.00\\);_(* \"-\"??_);_(@_)",                // 43
    "_(\"$\"* #,##0.00_);_(\"$\"* \\(#,##0.00\\);_(\"$\"* \"-\"??_);_(@_)", // 44
    "mm:ss",                                                                // 45
    "[h]:mm:ss",                                                            // 46
    "mmss.0",                                                               // 47
    "##0.0E+0",                                                             // 48
    "@",                                                                    // 49
];

pub fn get_default_num_fmt_id(num_fmt: &str) -> Option<i32> {
    for (index, default_num_fmt) in DEFAULT_NUM_FMTS.iter().enumerate() {
        if default_num_fmt == &num_fmt {
            return Some(index as i32);
        };
    }
    None
}

pub fn get_num_fmt(num_fmt_id: i32, num_fmts: &[NumFmt]) -> String {
    // Check if it defined
    for num_fmt in num_fmts {
        if num_fmt.num_fmt_id == num_fmt_id {
            return num_fmt.format_code.clone();
        }
    }
    // Return one of the default ones
    if num_fmt_id < DEFAULT_NUM_FMTS.len() as i32 {
        return DEFAULT_NUM_FMTS[num_fmt_id as usize].to_string();
    }
    // Return general
    DEFAULT_NUM_FMTS[0].to_string()
}

pub fn get_new_num_fmt_index(num_fmts: &[NumFmt]) -> i32 {
    let mut index = DEFAULT_NUM_FMTS.len() as i32;
    let mut found = true;
    while found {
        found = false;
        for num_fmt in num_fmts {
            if num_fmt.num_fmt_id == index {
                found = true;
                index += 1;
                break;
            }
        }
    }
    index
}

/// Round positive finite values using 15 significant decimal digits and ties
/// away from zero, without scaling beyond the floating-point finite range.
pub(crate) fn format_decimal_abs(value: f64, decimals: i32) -> String {
    let scientific = format!("{value:.14e}");
    let (coefficient, exponent) = scientific.split_once('e').unwrap_or(("0", "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let digits: Vec<u8> = coefficient.bytes().filter(|c| *c != b'.').collect();
    let keep = exponent + 1 + decimals;
    let mut scaled = if keep > 0 {
        (0..keep as usize)
            .map(|i| digits.get(i).copied().unwrap_or(b'0'))
            .collect::<Vec<_>>()
    } else {
        vec![b'0']
    };
    let round_up = keep >= 0 && digits.get(keep as usize).is_some_and(|d| *d >= b'5');
    if round_up {
        let mut carry = true;
        for digit in scaled.iter_mut().rev() {
            if *digit == b'9' {
                *digit = b'0';
            } else {
                *digit += 1;
                carry = false;
                break;
            }
        }
        if carry {
            scaled.insert(0, b'1');
        }
    }
    let mut scaled: String = scaled.into_iter().map(char::from).collect();
    if decimals > 0 {
        let places = decimals as usize;
        if scaled.len() <= places {
            scaled = format!("{}{}", "0".repeat(places + 1 - scaled.len()), scaled);
        }
        scaled.insert(scaled.len() - places, '.');
    } else if decimals < 0 && scaled.bytes().any(|digit| digit != b'0') {
        scaled.push_str(&"0".repeat((-decimals) as usize));
    }
    scaled
}

#[derive(Clone, Copy)]
pub(crate) enum DecimalRounding {
    Nearest,
    AwayFromZero,
    TowardZero,
}

/// Round decimal digits without multiplying by a potentially infinite scale.
pub(crate) fn round_decimal(value: f64, decimals: f64, mode: DecimalRounding) -> Option<f64> {
    if !value.is_finite() || !decimals.is_finite() {
        return None;
    }
    if value == 0.0 {
        return Some(0.0);
    }
    let scientific = format!("{:.14e}", value.abs());
    let (coefficient, exponent) = scientific.split_once('e')?;
    let exponent: i32 = exponent.parse().ok()?;
    let digits: Vec<u8> = coefficient.bytes().filter(|digit| *digit != b'.').collect();
    let decimals = decimals.trunc();
    let keep = f64::from(exponent) + 1.0 + decimals;
    if keep >= digits.len() as f64 {
        return scientific
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .map(|n| n.copysign(value));
    }
    let (mut integer, increment) = if keep <= 0.0 {
        (
            0_u64,
            match mode {
                DecimalRounding::Nearest => keep == 0.0 && digits[0] >= b'5',
                DecimalRounding::AwayFromZero => true,
                DecimalRounding::TowardZero => false,
            },
        )
    } else {
        let keep = keep as usize;
        let integer = digits[..keep]
            .iter()
            .fold(0_u64, |n, digit| n * 10 + u64::from(digit - b'0'));
        let increment = match mode {
            DecimalRounding::Nearest => digits[keep] >= b'5',
            DecimalRounding::AwayFromZero => digits[keep..].iter().any(|digit| *digit != b'0'),
            DecimalRounding::TowardZero => false,
        };
        (integer, increment)
    };
    integer += u64::from(increment);
    if integer == 0 {
        return Some(0.0);
    }
    format!("{integer}e{}", -decimals)
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .map(|n| n.copysign(value))
}

pub fn to_precision(value: f64, precision: usize) -> f64 {
    if value.is_infinite() || value.is_nan() {
        return value;
    }
    to_precision_str(value, precision)
        .parse::<f64>()
        .unwrap_or({
            // TODO: do this in a way that does not require a possible error
            0.0
        })
}

/// It rounds a `f64` with `p` significant figures:
/// ```
///     use ironcalc_base::number_format;
///     assert_eq!(number_format::to_precision(0.1+0.2, 15), 0.3);
///     assert_eq!(number_format::to_excel_precision_str(0.1+0.2), "0.3");
/// ```
/// This intends to be equivalent to the js: `${parseFloat(value.toPrecision(precision)})`
/// See ([ecma](https://tc39.es/ecma262/#sec-number.prototype.toprecision)).
pub fn to_excel_precision_str(value: f64) -> String {
    to_precision_str(value, 15)
}

pub fn to_excel_precision(value: f64, precision: usize) -> f64 {
    if !value.is_finite() {
        return value;
    }

    let s = format!("{:.*e}", precision.saturating_sub(1), value);
    s.parse::<f64>().unwrap_or(value)
}

pub fn to_precision_str(value: f64, precision: usize) -> String {
    if !value.is_finite() {
        if value.is_infinite() {
            return "inf".to_string();
        } else {
            return "NaN".to_string();
        }
    }

    let s = format!("{:.*e}", precision.saturating_sub(1), value);
    let parsed = s.parse::<f64>().unwrap_or(value);

    // I would love to use the std library. There is not a speed concern here
    // problem is it doesn't do the right thing
    // Also ryu is my favorite _modern_ algorithm
    let mut buffer = ryu::Buffer::new();
    let text = buffer.format(parsed);
    // The above algorithm converts 2 to 2.0 regrettably
    if let Some(stripped) = text.strip_suffix(".0") {
        return stripped.to_string();
    }
    text.to_string()
}

pub fn format_number(value: f64, format_code: &str, locale: &str) -> Formatted {
    let locale = match get_locale(locale) {
        Ok(l) => l,
        Err(_) => {
            return Formatted {
                text: "#ERROR!".to_owned(),
                color: None,
                error: Some("Invalid locale".to_string()),
            }
        }
    };
    formatter::format::format_number(value, format_code, locale)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The default table is indexed by `numFmtId`: `get_num_fmt(id, &[])` must return the
    // ECMA-376 §18.8.30 built-in code for that id.
    #[test]
    fn builtin_ids_map_to_the_ecma376_codes() {
        assert_eq!(get_num_fmt(0, &[]), "general");
        assert_eq!(get_num_fmt(5, &[]), "$#,##0_);($#,##0)");
        assert_eq!(get_num_fmt(8, &[]), "$#,##0.00_);[Red]($#,##0.00)");
        assert_eq!(get_num_fmt(11, &[]), "0.00E+00");
        assert_eq!(get_num_fmt(37, &[]), "#,##0_);(#,##0)");
        // id 39 is the one that produced `#VALUE!` in the wild (was "t0.00").
        assert_eq!(get_num_fmt(39, &[]), "#,##0.00_);(#,##0.00)");
        assert_eq!(
            get_num_fmt(44, &[]),
            "_(\"$\"* #,##0.00_);_(\"$\"* \\(#,##0.00\\);_(\"$\"* \"-\"??_);_(@_)"
        );
        assert_eq!(get_num_fmt(49, &[]), "@");
        // Reserved ids 23–36 resolve to "general".
        assert_eq!(get_num_fmt(25, &[]), "general");
        // Out-of-range falls back to "general" too.
        assert_eq!(get_num_fmt(500, &[]), "general");
    }

    // A workbook-defined `numFmt` still wins over the default table.
    #[test]
    fn workbook_defined_num_fmt_takes_precedence() {
        let defined = vec![NumFmt {
            num_fmt_id: 39,
            format_code: "FILE-OWN".to_string(),
        }];
        assert_eq!(get_num_fmt(39, &defined), "FILE-OWN");
    }

    // Every corrected built-in code must actually be formattable by IronCalc's own formatter
    // (a valid number must NOT come back as an error). This is what was broken: the old garbage
    // codes (e.g. "t0.00") made the formatter fail on valid numbers.
    //
    // Exception: id 47 (`mmss.0`, elapsed minutes:seconds.tenths) is the correct ECMA-376 code
    // but IronCalc's *formatter* does not yet parse that pattern — a separate formatter gap, not
    // a table bug (the old code at id 47 didn't render either). Kept correct in the table so a
    // future formatter fix makes it work; excluded from this render smoke-check.
    #[test]
    fn corrected_builtins_format_without_error() {
        let mut failures = Vec::new();
        for id in [
            5, 6, 7, 8, 11, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 48, 49,
        ] {
            let code = get_num_fmt(id, &[]);
            let out = format_number(1234.5, &code, "en");
            if out.error.is_some() || out.text.contains("#VALUE!") || out.text.contains("#ERROR!") {
                failures.push((id, code, out.text, out.error));
            }
        }
        assert!(
            failures.is_empty(),
            "codes the formatter rejected: {failures:?}"
        );
    }

    // The specific regression: id 39 formats 175000 as a grouped 2-decimal number, not `#VALUE!`.
    #[test]
    fn id_39_formats_currency_value() {
        let out = format_number(175000.0, &get_num_fmt(39, &[]), "en");
        assert!(out.error.is_none());
        assert!(out.text.contains("175,000.00"), "got {:?}", out.text);
    }
}
