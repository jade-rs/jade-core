//! Allocation-free view of the legacy paragraph-property hash inputs.
use jade_docx_reader::document::ParagraphProperties;

// Keep the field order and conversions identical to ParagraphHashableProps.
// String and str feed the same bytes to Hash; borrowing avoids three temporary
// strings whenever a paragraph's render-cache key is computed.
#[derive(Hash)]
pub(super) struct BorrowedParagraphHashableProps<'a> {
    alignment: Option<&'a str>,
    spacing_before: Option<i32>,
    spacing_after: Option<i32>,
    line_spacing: Option<i32>,
    indent_left: Option<i32>,
    indent_right: Option<i32>,
    indent_first_line: Option<i32>,
    indent_hanging: Option<i32>,
    indent_left_chars: Option<i32>,
    indent_right_chars: Option<i32>,
    style: Option<&'a str>,
    numbering_id: Option<&'a str>,
    numbering_level: Option<i32>,
}

impl<'a> From<&'a ParagraphProperties> for BorrowedParagraphHashableProps<'a> {
    #[inline]
    fn from(props: &'a ParagraphProperties) -> Self {
        let spacing = props.spacing.as_ref();
        let indent = props.indent.as_ref();
        let numbering = props.numbering.as_ref();
        let bidi = props.bidi.unwrap_or(false);
        Self {
            alignment: props.alignment.as_deref(),
            spacing_before: spacing.and_then(|s| s.before).map(|v| (v * 100.0) as i32),
            spacing_after: spacing.and_then(|s| s.after).map(|v| (v * 100.0) as i32),
            line_spacing: spacing.and_then(|s| s.line).map(|v| (v * 100.0) as i32),
            indent_left: indent
                .and_then(|i| i.physical_left(bidi))
                .map(|v| (v * 100.0) as i32),
            indent_right: indent
                .and_then(|i| i.physical_right(bidi))
                .map(|v| (v * 100.0) as i32),
            indent_first_line: indent
                .and_then(|i| i.first_line)
                .map(|v| (v * 100.0) as i32),
            indent_hanging: indent.and_then(|i| i.hanging).map(|v| (v * 100.0) as i32),
            indent_left_chars: indent.and_then(|i| i.physical_left_chars(bidi)),
            indent_right_chars: indent.and_then(|i| i.physical_right_chars(bidi)),
            style: props.style.as_deref(),
            numbering_id: numbering.and_then(|n| n.id.as_deref()),
            numbering_level: numbering.and_then(|n| n.level).map(|v| v as i32),
        }
    }
}

#[cfg(test)]
mod tests;
