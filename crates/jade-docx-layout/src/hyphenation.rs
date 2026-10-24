//! Where a word may be hyphenated: Liang's algorithm over the TeX hyphenation
//! patterns, as Word's automatic hyphenation (`w:autoHyphenation`) needs
//! them, for the language its run is tagged with (`w:lang`).
//!
//! The patterns are shipped verbatim from the hyph-utf8 package, each file
//! keeping its own copyright and licence notice (all listed in
//! `third_party/hyph-utf8-NOTICE.txt`): American English (Gerard D.C.
//! Kuiken's `ushyphmax`, with Knuth's original patterns; copying and
//! distribution with or without modification permitted), British English
//! (Dominik Wujastyk and Graham Toal, MIT), German in the 1996 reformed
//! spelling (Stephan Hennig, Werner Lemberg et al., MIT), French (Daniel
//! Flipo, Bernard Gaulle, Arthur Reutenauer, MIT), Spanish (Javier Bezos,
//! CervanTeX, MIT), Dutch (Piet Tutelaers, MIT) and Italian (Claudio Beccari,
//! LPPL 1.3 or MIT). They are parsed on first use.

use std::collections::HashMap;
use std::sync::OnceLock;

const EN_US: &str = include_str!("hyphenation/hyph-en-us.tex");
const EN_GB: &str = include_str!("hyphenation/hyph-en-gb.tex");
const DE: &str = include_str!("hyphenation/hyph-de-1996.tex");
const FR: &str = include_str!("hyphenation/hyph-fr.tex");
const ES: &str = include_str!("hyphenation/hyph-es.tex");
const NL: &str = include_str!("hyphenation/hyph-nl.tex");
const IT: &str = include_str!("hyphenation/hyph-it.tex");

/// The languages with patterns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Language {
    AmericanEnglish,
    BritishEnglish,
    German,
    French,
    Spanish,
    Dutch,
    Italian,
}

impl Language {
    /// The patterns for a BCP 47 tag; no tag is English, as Word's
    /// Japanese-UI documents leave text on their English proofing default.
    fn of(tag: Option<&str>) -> Option<Self> {
        let tag = tag.map(str::trim).unwrap_or("");
        if tag.is_empty() {
            return Some(Self::AmericanEnglish);
        }
        let lower = tag.to_ascii_lowercase();
        let primary = lower.split(['-', '_']).next().unwrap_or("");
        Some(match primary {
            "en" if lower.len() >= 5 && &lower[3..5] == "gb" => Self::BritishEnglish,
            "en" => Self::AmericanEnglish,
            "de" => Self::German,
            "fr" => Self::French,
            "es" => Self::Spanish,
            "nl" => Self::Dutch,
            "it" => Self::Italian,
            _ => return None,
        })
    }

    /// Letters that must stay before the first hyphen and after the last:
    /// what each pattern file declares (`hyphenmins`), English keeping the 3
    /// letters after the last hyphen that its patterns are made for.
    fn mins(self) -> (usize, usize) {
        match self {
            Self::AmericanEnglish | Self::BritishEnglish => (2, 3),
            _ => (2, 2),
        }
    }

    fn patterns(self) -> &'static Patterns {
        macro_rules! cached {
            ($source:expr) => {{
                static PATTERNS: OnceLock<Patterns> = OnceLock::new();
                PATTERNS.get_or_init(|| Patterns::parse($source, self.mins()))
            }};
        }
        match self {
            Self::AmericanEnglish => cached!(EN_US),
            Self::BritishEnglish => cached!(EN_GB),
            Self::German => cached!(DE),
            Self::French => cached!(FR),
            Self::Spanish => cached!(ES),
            Self::Dutch => cached!(NL),
            Self::Italian => cached!(IT),
        }
    }
}

/// One language's patterns, ready for lookup.
struct Patterns {
    /// Pattern letters (with `.` for a word edge) to the digits between them.
    patterns: HashMap<String, Vec<u8>>,
    /// Whole words hyphenated by hand: lower-case word to break positions.
    exceptions: HashMap<String, Vec<usize>>,
    longest: usize,
    /// Letters kept before the first hyphen and after the last.
    mins: (usize, usize),
}

impl Patterns {
    fn parse(source: &str, mins: (usize, usize)) -> Self {
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
            mins,
        }
    }

    /// Positions (in characters) where `word` may break: a break at `n`
    /// keeps the first `n` letters before the hyphen.
    fn points(&self, word: &str) -> Vec<usize> {
        let lower = word.to_lowercase();
        let length = lower.chars().count();
        let (left_min, right_min) = self.mins;
        if length < left_min + right_min {
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
        (left_min..=length - right_min)
            .filter(|&kept| values[kept + 1] % 2 == 1)
            .collect()
    }
}

/// Whether text in `language` (a BCP 47 tag from `w:lang`, or none) can be
/// hyphenated here: English, German, French, Spanish, Dutch or Italian, or
/// text with no language, which Word's Japanese-UI documents leave on their
/// English proofing default.
pub fn supports_language(language: Option<&str>) -> bool {
    Language::of(language).is_some()
}

/// Where `word` may take a hyphen, as counts of letters kept before it, in
/// increasing order. Empty for a word too short to break, one with anything
/// but letters, or a language without patterns.
pub fn hyphenation_points(word: &str, language: Option<&str>) -> Vec<usize> {
    let Some(language) = Language::of(language) else {
        return Vec::new();
    };
    if !word.chars().all(char::is_alphabetic) {
        return Vec::new();
    }
    language.patterns().points(word)
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
        assert!(hyphenation_points("Rechtsanwaltskanzlei", Some("ru-RU")).is_empty());
        assert!(hyphenation_points("x86-64", None).is_empty());
    }

    #[test]
    fn british_english_has_its_own_patterns() {
        assert_eq!(hyphenated("university", Some("en-GB")), "uni-ver-sity");
        assert!(!hyphenation_points("hyphenation", Some("en-GB")).is_empty());
    }

    #[test]
    fn german_french_spanish_dutch_and_italian_have_their_patterns() {
        assert_eq!(
            hyphenated("Silbentrennung", Some("de-DE")),
            "Sil-ben-tren-nung"
        );
        assert_eq!(
            hyphenated("Verantwortung", Some("de-CH")),
            "Ver-ant-wor-tung"
        );
        assert_eq!(hyphenated("ordinateur", Some("fr-FR")), "or-di-na-teur");
        assert_eq!(hyphenated("biblioteca", Some("es-ES")), "bi-blio-te-ca");
        assert_eq!(
            hyphenated("woordafbreking", Some("nl-NL")),
            "woord-af-bre-king"
        );
        assert_eq!(hyphenated("calcolatore", Some("it-IT")), "cal-co-la-to-re");
        // Accented letters are letters.
        assert!(!hyphenation_points("Universität", Some("de-DE")).is_empty());
        assert!(supports_language(Some("fr-CA")));
        assert!(!supports_language(Some("pt-BR")));
    }
}
