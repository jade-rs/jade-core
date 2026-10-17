//! SpreadsheetML's `ST_Xstring` escape: `_xHHHH_`.
//!
//! XML 1.0 cannot hold most control characters, and a conforming parser
//! normalises the one it can — a carriage return in a cell would come back as
//! a line feed. Excel therefore stores them as `_xHHHH_` UTF-16 code units,
//! and protects a literal substring that merely looks like one by escaping
//! its underscore as `_x005F_`.

/// Decodes `_xHHHH_` sequences; anything that is not exactly one is literal.
///
/// Decoding left-to-right also gives `_x005F_` its specified escaping
/// behaviour: `_x005F_x000A_` becomes the literal `_x000A_`, not a newline.
/// A high surrogate followed by a low one decodes as the supplementary
/// character the pair encodes.
pub fn decode_xstring(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = String::with_capacity(value.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if let Some(unit) = escaped_code_unit(bytes, index) {
            if (0xD800..=0xDBFF).contains(&unit) {
                if let Some(low) = escaped_code_unit(bytes, index + 7) {
                    if (0xDC00..=0xDFFF).contains(&low) {
                        let codepoint = 0x1_0000
                            + ((u32::from(unit) - 0xD800) << 10)
                            + (u32::from(low) - 0xDC00);
                        if let Some(character) = char::from_u32(codepoint) {
                            out.push(character);
                            index += 14;
                            continue;
                        }
                    }
                }
            } else if let Some(character) = char::from_u32(u32::from(unit)) {
                out.push(character);
                index += 7;
                continue;
            }
        }

        let character = value[index..].chars().next().expect("a character");
        out.push(character);
        index += character.len_utf8();
    }
    out
}

fn escaped_code_unit(bytes: &[u8], index: usize) -> Option<u16> {
    if index + 7 > bytes.len()
        || bytes[index] != b'_'
        || !matches!(bytes[index + 1], b'x' | b'X')
        || bytes[index + 6] != b'_'
    {
        return None;
    }
    let digits = std::str::from_utf8(&bytes[index + 2..index + 6]).ok()?;
    u16::from_str_radix(digits, 16).ok()
}

/// Escapes what XML cannot carry, leaving everything else alone.
///
/// Tab and line feed are valid XML and stay literal. A carriage return is
/// valid too, but a conforming parser normalises it to a line feed on read,
/// so it has to be escaped to survive a round trip. A literal `_xHHHH_` gets
/// its underscore escaped so it does not read back as a character.
pub fn encode_xstring(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0usize;
    for character in text.chars() {
        if needs_escape(character) {
            out.push_str(&format!("_x{:04X}_", character as u32));
        } else if character == '_' && escaped_code_unit(bytes, index).is_some() {
            out.push_str("_x005F_");
        } else {
            out.push(character);
        }
        index += character.len_utf8();
    }
    out
}

fn needs_escape(character: char) -> bool {
    matches!(character, '\u{0}'..='\u{1F}' if character != '\t' && character != '\n')
        || matches!(character, '\u{FFFE}' | '\u{FFFF}')
}

#[cfg(test)]
mod tests {
    use super::{decode_xstring, encode_xstring};

    #[test]
    fn a_string_survives_an_encode_decode_round_trip() {
        for awkward in [
            "line1\rline2",
            "_x000D_ kept literal",
            "tab\tand\nnewline stay",
            "control \u{1} char",
            "日本語 _x005F_ text",
            "surrogate pair intact 𠮷",
        ] {
            assert_eq!(decode_xstring(&encode_xstring(awkward)), awkward);
        }
    }

    #[test]
    fn a_surrogate_pair_of_escapes_decodes_as_one_character() {
        assert_eq!(decode_xstring("_xD842_x DFB7_"), "_xD842_x DFB7_");
        assert_eq!(decode_xstring("_xD842__xDFB7_"), "𠮷");
    }

    #[test]
    fn a_lone_surrogate_stays_literal() {
        assert_eq!(decode_xstring("_xD800_"), "_xD800_");
    }
}
