//! OpenType features carried inside a CSS font descriptor.
//!
//! Word 2010 lets a run ask for OpenType features (`w14:ligatures`,
//! `w14:numForm`, `w14:numSpacing`, `w14:stylisticSets`, `w14:cntxtAlts`).
//! Layout measures and painters draw text by font descriptor alone — the
//! descriptor is what every measurement cache is keyed on — so the features
//! travel inside it, as one extra leading token:
//!
//! ```text
//! ot:liga,clig,-calt,onum 11.00pt Calibri, sans-serif
//! ```
//!
//! A leading token sits where the CSS shorthand keeps style keywords (`bold`,
//! `italic`), which every descriptor parser here already skips past to find
//! the size, so a reader that knows nothing of features still finds the face.
//! A canvas, which would reject the whole shorthand, is handed
//! [`strip`]'s result instead.

/// What the features token starts with.
pub const PREFIX: &str = "ot:";

/// One feature setting: the OpenType tag and its value (0 turns it off).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontFeature {
    pub tag: [u8; 4],
    pub value: u32,
}

/// The features token of `descriptor`, without its prefix, if it has one.
pub fn features_token(descriptor: &str) -> Option<&str> {
    descriptor
        .split_whitespace()
        .find_map(|token| token.strip_prefix(PREFIX))
}

/// Whether `descriptor` asks for any OpenType features.
pub fn has_features(descriptor: &str) -> bool {
    // The token leads the descriptor wherever jade puts one in; a plain
    // descriptor is answered without scanning it.
    if descriptor.starts_with(PREFIX) {
        return true;
    }
    descriptor.contains(PREFIX) && features_token(descriptor).is_some()
}

/// `descriptor` without its features token: the plain CSS shorthand a canvas
/// accepts. Borrowed when there is nothing to remove.
pub fn strip(descriptor: &str) -> std::borrow::Cow<'_, str> {
    if !has_features(descriptor) {
        return std::borrow::Cow::Borrowed(descriptor);
    }
    std::borrow::Cow::Owned(
        descriptor
            .split_whitespace()
            .filter(|token| !token.starts_with(PREFIX))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// The features `descriptor` asks for, in order.
pub fn parse(descriptor: &str) -> Vec<FontFeature> {
    features_token(descriptor)
        .map(parse_token)
        .unwrap_or_default()
}

/// Parses a token body: comma-separated tags, `-tag` to turn one off.
pub fn parse_token(token: &str) -> Vec<FontFeature> {
    token
        .split(',')
        .filter_map(|item| {
            let (name, value) = match item.strip_prefix('-') {
                Some(name) => (name, 0),
                None => (item, 1),
            };
            let bytes = name.as_bytes();
            (bytes.len() == 4 && bytes.iter().all(u8::is_ascii_alphanumeric)).then(|| FontFeature {
                tag: [bytes[0], bytes[1], bytes[2], bytes[3]],
                value,
            })
        })
        .collect()
}

/// Whether any feature that is on joins or swaps glyphs across characters —
/// a ligature, or a contextual alternate — so a run has to be shaped whole
/// rather than a character at a time to look right.
pub fn joins_characters(features: &[FontFeature]) -> bool {
    features.iter().any(|feature| {
        feature.value != 0
            && matches!(
                &feature.tag,
                b"liga" | b"clig" | b"dlig" | b"hlig" | b"calt" | b"rlig"
            )
    })
}

/// The token for `features` (`ot:liga,-calt`), or `None` when empty.
pub fn token(features: &[FontFeature]) -> Option<String> {
    if features.is_empty() {
        return None;
    }
    let mut out = String::from(PREFIX);
    for (index, feature) in features.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        if feature.value == 0 {
            out.push('-');
        }
        out.push_str(std::str::from_utf8(&feature.tag).unwrap_or("????"));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn features_are_found_leading_or_not() {
        assert!(has_features("ot:liga 11.00pt Calibri, sans-serif"));
        assert!(has_features("bold ot:onum 11.00pt Calibri"));
        assert!(!has_features("italic 11.00pt Calibri, sans-serif"));
        assert!(!has_features("11.00pt 'Not:a font'"));
    }

    #[test]
    fn token_round_trips_and_strips() {
        let features = parse_token("liga,-calt,onum,ss03");
        assert_eq!(features.len(), 4);
        assert_eq!(features[1].value, 0);
        assert_eq!(token(&features).as_deref(), Some("ot:liga,-calt,onum,ss03"));
        let descriptor = "ot:liga,-calt bold 11.00pt Calibri, sans-serif";
        assert_eq!(parse(descriptor).len(), 2);
        assert_eq!(strip(descriptor), "bold 11.00pt Calibri, sans-serif");
        assert!(joins_characters(&parse(descriptor)));
        assert!(!joins_characters(&parse_token("onum,-liga")));
        assert_eq!(strip("11pt Calibri"), "11pt Calibri");
        assert!(parse("11pt Calibri").is_empty());
    }
}
