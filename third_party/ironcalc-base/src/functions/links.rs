//! Jade additions: ENCODEURL and HYPERLINK, as Excel computes them.

use crate::calc_result::CalcResult;
use crate::expressions::parser::Node;
use crate::expressions::types::CellReferenceIndex;
use crate::model::Model;

/// `text` with every byte outside the URL's unreserved characters — ASCII
/// letters, digits and `- _ . ~` — written as `%XX`, as Excel's ENCODEURL
/// writes it (UTF-8, upper-case hex).
fn encode_url(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

impl<'a> Model<'a> {
    pub(crate) fn fn_encodeurl(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        match self.get_string(&args[0], cell) {
            Ok(text) => CalcResult::String(encode_url(&text)),
            Err(error) => error,
        }
    }

    /// The cell shows the friendly name — whatever it is, a number included
    /// — or the link itself when there is none. Following the link is the
    /// application's business, not the calculation's.
    pub(crate) fn fn_hyperlink(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        match args.len() {
            1 => match self.get_string(&args[0], cell) {
                Ok(link) => CalcResult::String(link),
                Err(error) => error,
            },
            2 => self.evaluate_node_in_context(&args[1], cell),
            _ => CalcResult::new_args_number_error(cell),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::encode_url;

    #[test]
    fn unreserved_characters_stay_and_the_rest_is_utf8_percent_encoded() {
        assert_eq!(encode_url("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(encode_url("a b/c?"), "a%20b%2Fc%3F");
        assert_eq!(encode_url("é"), "%C3%A9");
    }
}
