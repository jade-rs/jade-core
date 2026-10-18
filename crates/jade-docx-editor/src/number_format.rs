/// Formats a page number or note marker with its `ST_NumberFormat` token
/// (`w:pgNumType/@w:fmt`, `w:footnotePr/w:numFmt`). It is the same formatter
/// list levels use, so a section numbered 一, 二, 三 or ①, ②, ③ shows those
/// labels; unknown formats fall back to decimal.
pub(crate) fn format_number(n: usize, fmt: Option<&str>) -> String {
    let value = u32::try_from(n).unwrap_or(u32::MAX);
    jade_docx_reader::numbering::format_number_value(fmt.unwrap_or("decimal"), value, None)
}

/// The schema spelling of a format a page number or note marker may be set
/// to: any formatted `ST_NumberFormat` token except the language-dependent
/// English words (`ordinal`, `cardinalText`, `ordinalText`) and the non-numeric
/// `bullet` and `none`.
pub(crate) fn authorable_number_format(format: &str) -> Option<&'static str> {
    jade_docx_reader::numbering::canonical_number_format(format).filter(|token| {
        !matches!(
            *token,
            "ordinal" | "cardinalText" | "ordinalText" | "bullet" | "none"
        )
    })
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
    }

    #[test]
    fn page_numbers_and_note_marks_use_every_list_number_format() {
        assert_eq!(format_number(3, Some("japanesecounting")), "三");
        assert_eq!(format_number(12, Some("decimalfullwidth")), "１２");
        assert_eq!(format_number(2, Some("aiueofullwidth")), "イ");
        assert_eq!(format_number(28, Some("lowerletter")), "bb");
        assert_eq!(
            format_note_marker(3, Some("decimalenclosedcircle"), None, true),
            "③"
        );
        // Chicago Manual of Style footnote symbols, doubled after the fourth.
        assert_eq!(format_note_marker(1, Some("chicago"), None, true), "*");
        assert_eq!(format_note_marker(6, Some("chicago"), None, true), "††");
        assert_eq!(format_note_marker(2, None, None, true), "2");
        assert_eq!(format_note_marker(2, None, None, false), "ii");
    }

    #[test]
    fn authorable_formats_use_schema_spelling() {
        use super::authorable_number_format;
        assert_eq!(authorable_number_format("upperroman"), Some("upperRoman"));
        assert_eq!(
            authorable_number_format("japaneseCounting"),
            Some("japaneseCounting")
        );
        assert_eq!(authorable_number_format("chicago"), Some("chicago"));
        assert_eq!(authorable_number_format("ordinal"), None);
        assert_eq!(authorable_number_format("bullet"), None);
        assert_eq!(authorable_number_format("invalid"), None);
    }
}
