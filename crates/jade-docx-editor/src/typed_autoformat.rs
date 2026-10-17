//! Automatic formatting applies only to explicitly enabled interactive typing.

use jade_docx_headless_api::HeadlessMultilevelListPreset;

pub(crate) fn format_text(
    prefix: &str,
    text: &str,
    language: &str,
    capitalization: &str,
) -> String {
    if !language.is_empty()
        && !language.eq_ignore_ascii_case("en")
        && !language.to_ascii_lowercase().starts_with("en-")
    {
        return text.to_owned();
    }
    let mut context = prefix.to_owned();
    let mut output = String::with_capacity(text.len());
    for character in text.chars() {
        let previous = context.chars().next_back();
        let opening_quote = previous.is_none_or(|c| c.is_whitespace() || "([{“‘".contains(c));
        let character = match character {
            '"' if opening_quote => '“',
            '"' => '”',
            '\'' if opening_quote => '‘',
            '\'' => '’',
            c if c.is_ascii_lowercase() => {
                let capitalize = match capitalization {
                    "characters" => true,
                    "words" => previous.is_none_or(|c| !c.is_alphanumeric() && c != '’'),
                    "sentences" => sentence_start(&context),
                    _ => false,
                };
                if capitalize {
                    c.to_ascii_uppercase()
                } else {
                    c
                }
            }
            c => c,
        };
        output.push(character);
        context.push(character);
    }
    output
}

fn sentence_start(context: &str) -> bool {
    let line = context.rsplit(['\n', '\r']).next().unwrap_or("");
    let trimmed = line.trim_end_matches(|c: char| c.is_whitespace() || "“‘([{\"'".contains(c));
    trimmed.is_empty() || (trimmed.len() < line.len() && trimmed.ends_with(['.', '!', '?']))
}

pub(crate) fn list_start(
    prefix: &str,
    text: &str,
) -> Option<(HeadlessMultilevelListPreset, String)> {
    if prefix.len() > 2 || text.contains(['\n', '\r']) {
        return None;
    }
    let combined = format!("{prefix}{text}");
    for (marker, preset) in [
        ("* ", HeadlessMultilevelListPreset::BulletHierarchy),
        ("- ", HeadlessMultilevelListPreset::BulletHierarchy),
        ("1. ", HeadlessMultilevelListPreset::DecimalHierarchy),
    ] {
        if let Some(remainder) = combined.strip_prefix(marker) {
            if prefix.len() < marker.len() {
                return Some((preset, remainder.to_owned()));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentence_and_quote_formatting_is_independent_of_typing_batches() {
        let input = "hello. \"world\" isn't bad. next";
        let expected = "Hello. “World” isn’t bad. Next";
        assert_eq!(format_text("", input, "en-US", "sentences"), expected);
        let mut actual = String::new();
        for character in input.chars() {
            actual.push_str(&format_text(
                &actual,
                &character.to_string(),
                "en-GB",
                "sentences",
            ));
        }
        assert_eq!(actual, expected);
        assert_eq!(format_text("version 1.", "two", "en", "sentences"), "two");
        assert_eq!(format_text("😀\n", "hello", "en", "sentences"), "Hello");
    }

    #[test]
    fn language_and_capitalization_options_are_respected() {
        assert_eq!(
            format_text("", "日本語 \"hello\"", "ja-JP", "sentences"),
            "日本語 \"hello\""
        );
        assert_eq!(
            format_text("", "hello \"world\"", "en", "none"),
            "hello “world”"
        );
        assert_eq!(
            format_text("", "small words aren't caps", "en", "words"),
            "Small Words Aren’t Caps"
        );
        assert_eq!(format_text("", "abc 123", "en", "characters"), "ABC 123");
        assert_eq!(format_text("hello ", "'quoted'", "en", "none"), "‘quoted’");
    }

    #[test]
    fn only_a_complete_paragraph_start_marker_followed_by_space_starts_a_list() {
        assert!(list_start("*", " ").is_some());
        assert!(list_start("-", " ").is_some());
        assert!(list_start("1.", " ").is_some());
        assert_eq!(list_start("", "* item").unwrap().1, "item");
        assert_eq!(list_start("1", ". item").unwrap().1, "item");
        for (prefix, text) in [("a*", " "), (" -", " "), ("1.2", " "), ("* ", "item")] {
            assert!(list_start(prefix, text).is_none());
        }
    }
}
