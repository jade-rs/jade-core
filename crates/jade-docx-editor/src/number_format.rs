/// Formats a positive Word number using the common numeral formats used by
/// page numbers and note markers. Unknown formats fall back to decimal.
pub(crate) fn format_number(n: usize, fmt: Option<&str>) -> String {
    match fmt {
        Some("upperroman") => to_roman(n, true),
        Some("lowerroman") => to_roman(n, false),
        Some("upperletter") => to_letter(n, true),
        Some("lowerletter") => to_letter(n, false),
        Some("numberindash" | "numberInDash") => format!("- {n} -"),
        _ => n.to_string(),
    }
}

/// Resolves a footnote/endnote marker from its 1-based document ordinal while
/// honoring `w:numFmt` and `w:numStart`.
pub(crate) fn format_note_marker(
    ordinal: usize,
    fmt: Option<&str>,
    start: Option<u32>,
    is_footnote: bool,
) -> String {
    let displayed = start.unwrap_or(1) as usize + ordinal.saturating_sub(1);
    // MS-OI29500 §2.1.306: Word's omitted endnote numFmt is lowerRoman;
    // ordinary footnotes use decimal. Renderer and caret mapping share it.
    let default_format = if is_footnote { "decimal" } else { "lowerroman" };
    format_number(displayed, Some(fmt.unwrap_or(default_format)))
}

fn to_roman(n: usize, upper: bool) -> String {
    if n == 0 {
        return n.to_string();
    }
    const TABLE: &[(usize, &str)] = &[
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut remaining = n;
    let mut out = String::new();
    for &(value, symbol) in TABLE {
        while remaining >= value {
            out.push_str(symbol);
            remaining -= value;
        }
    }
    if upper {
        out
    } else {
        out.to_lowercase()
    }
}

fn to_letter(n: usize, upper: bool) -> String {
    if n == 0 {
        return n.to_string();
    }
    let mut remaining = n;
    let mut chars = Vec::new();
    while remaining > 0 {
        let remainder = (remaining - 1) % 26;
        let base = if upper { b'A' } else { b'a' };
        chars.push((base + remainder as u8) as char);
        remaining = (remaining - 1) / 26;
    }
    chars.iter().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::{format_note_marker, format_number};

    #[test]
    fn dash_delimited_numbers_share_page_and_note_start_rules() {
        for token in ["numberInDash", "numberindash"] {
            assert_eq!(format_number(1, Some(token)), "- 1 -");
            assert_eq!(format_number(12, Some(token)), "- 12 -");
            assert_eq!(format_number(0, Some(token)), "- 0 -");
            for footnote in [true, false] {
                assert_eq!(
                    format_note_marker(3, Some(token), Some(9), footnote),
                    "- 11 -"
                );
            }
        }
        assert_eq!(format_number(7, Some("cardinalText")), "7");
        assert_eq!(format_note_marker(2, None, None, true), "2");
        assert_eq!(format_note_marker(2, None, None, false), "ii");
    }
}
