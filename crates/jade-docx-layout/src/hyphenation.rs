//! Where an English word may be hyphenated: Liang's algorithm over the TeX
//! hyphenation patterns, as Word's automatic hyphenation (`w:autoHyphenation`)
//! needs them.
//!
//! The patterns are shipped verbatim from the hyph-utf8 package, each file
//! keeping its own copyright and licence notice: American English (Gerard
//! D.C. Kuiken's `ushyphmax`, with Knuth's original patterns; copying and
//! distribution with or without modification permitted) and British English
//! (Dominik Wujastyk and Graham Toal, MIT). They are parsed on first use.

use std::collections::HashMap;
use std::sync::OnceLock;

const EN_US: &str = include_str!("hyphenation/hyph-en-us.tex");
const EN_GB: &str = include_str!("hyphenation/hyph-en-gb.tex");

/// Letters that must stay before the first hyphen and after the last, as
/// both pattern files declare (`hyphenmins`).
const LEFT_MIN: usize = 2;
const RIGHT_MIN: usize = 3;

/// One language's patterns, ready for lookup.
struct Patterns {
    /// Pattern letters (with `.` for a word edge) to the digits between them.
    patterns: HashMap<String, Vec<u8>>,
    /// Whole words hyphenated by hand: lower-case word to break positions.
    exceptions: HashMap<String, Vec<usize>>,
    longest: usize,
}

impl Patterns {
    fn parse(source: &str) -> Self {
        let mut patterns = HashMap::new();
        let mut exceptions = HashMap::new();
        let mut longest = 0;
        let mut block: Option<bool> = None; // Some(true) patterns, Some(false) exceptions
        for line in source.lines() {
            let line = line.split('%').next().unwrap_or("");
            for token in line.split_whitespace() {
                if token.starts_with("\\patterns") {
                    block = Some(true);
                    continue;
                }
                if token.starts_with("\\hyphenation") {
                    block = Some(false);
                    continue;
                }
                let (token, closes) = match token.strip_suffix('}') {
                    Some(rest) => (rest, true),
                    None => (token, false),
                };
                let token = token.trim_start_matches('{');
                if !token.is_empty() {
                    match block {
                        Some(true) => {
                            let mut letters = String::new();
                            let mut values = vec![0u8];
                            for ch in token.chars() {
                                if let Some(digit) = ch.to_digit(10) {
                                    *values.last_mut().expect("one value per gap") = digit as u8;
                                } else {
                                    letters.push(ch);
                                    values.push(0);
                                }
                            }
                            longest = longest.max(letters.chars().count());
                            patterns.insert(letters, values);
                        }
                        Some(false) => {
                            let mut word = String::new();
                            let mut breaks = Vec::new();
                            for ch in token.chars() {
                                if ch == '-' {
                                    breaks.push(word.chars().count());
                                } else {
                                    word.push(ch);
                                }
                            }
                            exceptions.insert(word.to_lowercase(), breaks);
                        }
                        None => {}
                    }
                }
                if closes {
                    block = None;
                }
            }
        }
        Self {
            patterns,
            exceptions,
            longest,
        }
    }

    /// Positions (in characters) where `word` may break: a break at `n`
    /// keeps the first `n` letters before the hyphen.
    fn points(&self, word: &str) -> Vec<usize> {
        let lower = word.to_lowercase();
        let length = lower.chars().count();
        if length < LEFT_MIN + RIGHT_MIN {
            return Vec::new();
        }
        if let Some(breaks) = self.exceptions.get(&lower) {
            return breaks.clone();
        }
        let dotted: Vec<char> = std::iter::once('.')
            .chain(lower.chars())
            .chain(std::iter::once('.'))
            .collect();
        // values[i] is the gap before dotted[i].
        let mut values = vec![0u8; dotted.len() + 1];
        let mut key = String::new();
        for start in 0..dotted.len() {
            key.clear();
            for letter in dotted.iter().skip(start).take(self.longest) {
                key.push(*letter);
                if let Some(pattern) = self.patterns.get(&key) {
                    for (offset, value) in pattern.iter().enumerate() {
                        let slot = &mut values[start + offset];
                        *slot = (*slot).max(*value);
                    }
                }
            }
        }
        // The gap before letter n (1-based in `dotted`) breaks after n - 1
        // letters of the word.
        (LEFT_MIN..=length - RIGHT_MIN)
            .filter(|&kept| values[kept + 1] % 2 == 1)
            .collect()
    }
}

fn english(british: bool) -> &'static Patterns {
    static US: OnceLock<Patterns> = OnceLock::new();
    static GB: OnceLock<Patterns> = OnceLock::new();
    if british {
        GB.get_or_init(|| Patterns::parse(EN_GB))
    } else {
        US.get_or_init(|| Patterns::parse(EN_US))
    }
}

/// Whether text in `language` (a BCP 47 tag from `w:lang`, or none) can be
/// hyphenated here: English, or text with no language, which Word's
/// Japanese-UI documents leave on their English proofing default.
pub fn supports_language(language: Option<&str>) -> bool {
    language.is_none_or(|tag| {
        let tag = tag.trim();
        tag.is_empty()
            || tag
                .get(..2)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("en"))
    })
}

/// Where `word` may take a hyphen, as counts of letters kept before it, in
/// increasing order. Empty for a word too short to break, one with anything
/// but letters, or a language without patterns.
pub fn hyphenation_points(word: &str, language: Option<&str>) -> Vec<usize> {
    if !supports_language(language) || !word.chars().all(char::is_alphabetic) {
        return Vec::new();
    }
    let british = language.is_some_and(|tag| {
        tag.len() >= 5 && tag[3..].eq_ignore_ascii_case("gb") && tag[..2].eq_ignore_ascii_case("en")
    });
    english(british).points(word)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hyphenated(word: &str, language: Option<&str>) -> String {
        let points = hyphenation_points(word, language);
        let mut out = String::new();
        for (index, ch) in word.chars().enumerate() {
            if points.contains(&index) {
                out.push('-');
            }
            out.push(ch);
        }
        out
    }

    #[test]
    fn american_english_breaks_where_the_patterns_say() {
        assert_eq!(hyphenated("hyphenation", None), "hy-phen-ation");
        assert_eq!(hyphenated("computer", Some("en-US")), "com-puter");
        assert_eq!(hyphenated("Documents", Some("en-US")), "Doc-u-ments");
        // A hand-hyphenated exception wins over the patterns.
        assert_eq!(hyphenated("table", None), "ta-ble");
        assert_eq!(hyphenated("project", None), "project");
    }

    #[test]
    fn short_words_other_languages_and_non_letters_are_left_whole() {
        assert!(hyphenation_points("into", None).is_empty());
        assert!(hyphenation_points("hyphenation", Some("ja-JP")).is_empty());
        assert!(hyphenation_points("x86-64", None).is_empty());
    }

    #[test]
    fn british_english_has_its_own_patterns() {
        assert_eq!(hyphenated("university", Some("en-GB")), "uni-ver-sity");
        assert!(!hyphenation_points("hyphenation", Some("en-GB")).is_empty());
    }
}
