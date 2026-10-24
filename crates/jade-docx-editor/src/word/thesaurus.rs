//! 校閲 › 文章校正 › 類義語辞典: the words that mean what the word at the
//! caret means, by sense, and 挿入 to put one in its place (keeping its
//! capitals). Where the words come from is the shell's: the system's
//! thesaurus in the native app (read through [`parse_thesaurus_entry`]), a
//! small built-in English list in the browser ([`BuiltinThesaurus`]).

use jade_docx_headless_api::HeadlessCommand;

use super::{Helpers, Position, WordSession};

/// One sense of a word and its synonyms.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sense {
    /// What the sense is: the part of speech, an example, or the first
    /// synonym.
    pub label: String,
    pub synonyms: Vec<String>,
}

/// Where synonyms come from.
pub trait Thesaurus {
    fn lookup(&self, word: &str) -> Vec<Sense>;
}

/// The parts of speech an entry names.
const PARTS_OF_SPEECH: [&str; 10] = [
    "noun",
    "verb",
    "adjective",
    "adverb",
    "preposition",
    "conjunction",
    "名詞",
    "動詞",
    "形容詞",
    "副詞",
];

/// A thesaurus entry as the system dictionary gives it as text — `word
/// part-of-speech 1 an example. synonym, synonym; synonym … ANTONYMS ….
/// 2 …` — split into its senses. An example may also end in a colon.
pub fn parse_thesaurus_entry(text: &str) -> Vec<Sense> {
    let mut senses = Vec::new();
    let mut part_of_speech = String::new();
    for chunk in split_senses(text) {
        let chunk = chunk.trim();
        let numbered = chunk.starts_with(|ch: char| ch.is_ascii_digit());
        if numbered {
            let body = chunk
                .trim_start_matches(|ch: char| ch.is_ascii_digit())
                .trim();
            let cut = body
                .find(": ")
                .map(|at| (at, 2))
                .or_else(|| body.find(". ").map(|at| (at, 2)));
            if let Some((at, skip)) = cut {
                let example = body[..at].trim();
                let rest = &body[at + skip..];
                let rest = rest.split("ANTONYMS").next().unwrap_or(rest);
                let synonyms: Vec<String> = rest
                    .split([',', '、', ';', '；'])
                    .map(clean_synonym)
                    .filter(|synonym| !synonym.is_empty())
                    .fold(Vec::new(), |mut list, synonym| {
                        if !list.contains(&synonym) {
                            list.push(synonym);
                        }
                        list
                    });
                if !synonyms.is_empty() {
                    let example: String = example.chars().take(60).collect();
                    let label = match (part_of_speech.is_empty(), example.is_empty()) {
                        (_, true) => synonyms[0].clone(),
                        (true, false) => example,
                        (false, false) => format!("{part_of_speech}: {example}"),
                    };
                    senses.push(Sense { label, synonyms });
                }
            }
        }
        // A part of speech closing the chunk names the senses after it.
        if let Some(last) = chunk.split_whitespace().last() {
            if PARTS_OF_SPEECH.contains(&last) {
                part_of_speech = last.to_string();
            }
        }
    }
    senses
}

/// The text cut where a numbered sense starts (` 1 `, ` 2 `, …).
fn split_senses(text: &str) -> Vec<&str> {
    let mut cuts = vec![0];
    let bytes = text.as_bytes();
    for at in 1..bytes.len().saturating_sub(1) {
        let mut end = at;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        let starts = end > at
            && end - at <= 2
            && bytes[at - 1] == b' '
            && bytes.get(end) == Some(&b' ')
            && (at < 2 || !bytes[at - 2].is_ascii_digit());
        if starts && !(at >= 2 && bytes[at - 2] == b',') {
            cuts.push(at);
        }
    }
    cuts.push(text.len());
    cuts.dedup();
    cuts.windows(2)
        .map(|pair| &text[pair[0]..pair[1]])
        .collect()
}

/// Register and region labels the dictionary puts before synonyms.
const LABELS: [&str; 17] = [
    "informal",
    "formal",
    "dated",
    "rare",
    "archaic",
    "literary",
    "humorous",
    "derogatory",
    "technical",
    "British English",
    "North American English",
    "Australian English",
    "Northern England",
    "Scottish",
    "Irish",
    "Indian English",
    "South African English",
];

/// A synonym without the dictionary's notes: `(informal)`, register and
/// region labels, a trailing full stop.
fn clean_synonym(raw: &str) -> String {
    let mut text = raw.to_string();
    while let (Some(open), Some(close)) = (text.find('('), text.find(')')) {
        if close < open {
            break;
        }
        text.replace_range(open..=close, "");
    }
    // `last. noun`: a sentence end, then what the next senses are.
    if let Some(at) = text.find(". ") {
        text.truncate(at);
    }
    let mut text = text.trim().trim_end_matches('.').trim().to_string();
    loop {
        let before = text.len();
        for label in LABELS {
            if let Some(rest) = text.strip_prefix(label) {
                if rest.is_empty() || rest.starts_with([' ', ',']) {
                    text = rest.trim_start_matches([' ', ',']).to_string();
                }
            }
        }
        if text.len() == before {
            break;
        }
    }
    // A usage note, not a word.
    if text.split_whitespace().count() > 5 || text.contains("ANTONYMS") {
        return String::new();
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A small English thesaurus for where the system has none.
#[derive(Default)]
pub struct BuiltinThesaurus;

const BUILTIN: &[(&str, &[&str])] = &[
    (
        "good",
        &[
            "fine",
            "excellent",
            "great",
            "superior",
            "decent",
            "satisfactory",
        ],
    ),
    (
        "bad",
        &["poor", "inferior", "unsatisfactory", "awful", "terrible"],
    ),
    (
        "big",
        &["large", "great", "huge", "sizable", "substantial", "major"],
    ),
    ("small", &["little", "tiny", "minor", "compact", "slight"]),
    (
        "happy",
        &[
            "glad",
            "pleased",
            "cheerful",
            "content",
            "delighted",
            "joyful",
        ],
    ),
    (
        "sad",
        &["unhappy", "sorrowful", "downcast", "gloomy", "dejected"],
    ),
    ("fast", &["quick", "rapid", "swift", "speedy", "brisk"]),
    ("slow", &["unhurried", "leisurely", "gradual", "sluggish"]),
    (
        "important",
        &[
            "significant",
            "crucial",
            "essential",
            "vital",
            "key",
            "major",
        ],
    ),
    (
        "difficult",
        &["hard", "challenging", "demanding", "tough", "complex"],
    ),
    (
        "easy",
        &["simple", "straightforward", "effortless", "uncomplicated"],
    ),
    (
        "begin",
        &["start", "commence", "initiate", "launch", "open"],
    ),
    (
        "end",
        &["finish", "conclude", "close", "terminate", "complete"],
    ),
    (
        "show",
        &["display", "present", "demonstrate", "reveal", "exhibit"],
    ),
    ("use", &["employ", "utilize", "apply", "exercise"]),
    ("make", &["create", "produce", "build", "construct", "form"]),
    ("help", &["assist", "aid", "support", "serve"]),
    ("change", &["alter", "modify", "adjust", "amend", "revise"]),
    ("get", &["obtain", "acquire", "receive", "gain", "secure"]),
    ("give", &["provide", "supply", "offer", "grant", "present"]),
    ("need", &["require", "want", "necessitate"]),
    ("think", &["believe", "consider", "suppose", "reckon"]),
    (
        "say",
        &["state", "declare", "mention", "remark", "announce"],
    ),
    (
        "clear",
        &["plain", "evident", "obvious", "apparent", "distinct"],
    ),
    ("new", &["recent", "fresh", "novel", "modern", "current"]),
    ("old", &["former", "previous", "aged", "ancient", "earlier"]),
    ("right", &["correct", "accurate", "proper", "appropriate"]),
    (
        "wrong",
        &["incorrect", "mistaken", "inaccurate", "erroneous"],
    ),
    ("buy", &["purchase", "acquire", "procure"]),
    (
        "agreement",
        &["contract", "arrangement", "deal", "accord", "settlement"],
    ),
    ("document", &["paper", "record", "report", "file"]),
    (
        "company",
        &["firm", "business", "corporation", "enterprise"],
    ),
    ("result", &["outcome", "consequence", "effect", "upshot"]),
    (
        "problem",
        &["issue", "difficulty", "trouble", "complication"],
    ),
    ("idea", &["concept", "notion", "thought", "plan"]),
    ("improve", &["enhance", "better", "upgrade", "refine"]),
    ("increase", &["raise", "boost", "expand", "grow", "augment"]),
    (
        "decrease",
        &["reduce", "lower", "lessen", "diminish", "cut"],
    ),
    ("allow", &["permit", "let", "enable", "authorize"]),
    ("ask", &["request", "inquire", "query", "question"]),
];

impl Thesaurus for BuiltinThesaurus {
    fn lookup(&self, word: &str) -> Vec<Sense> {
        let key = word.to_lowercase();
        BUILTIN
            .iter()
            .filter(|(head, synonyms)| *head == key || synonyms.contains(&key.as_str()))
            .map(|(head, synonyms)| {
                let mut list: Vec<String> = std::iter::once(*head)
                    .chain(synonyms.iter().copied())
                    .filter(|synonym| *synonym != key)
                    .map(str::to_string)
                    .collect();
                list.dedup();
                Sense {
                    label: (*head).to_string(),
                    synonyms: list,
                }
            })
            .collect()
    }
}

/// `synonym` with the capitals of `word`: all capitals, or a capital first.
pub fn matching_case(word: &str, synonym: &str) -> String {
    let letters: Vec<char> = word.chars().filter(|ch| ch.is_alphabetic()).collect();
    if letters.len() > 1 && letters.iter().all(|ch| ch.is_uppercase()) {
        return synonym.to_uppercase();
    }
    if word.chars().next().is_some_and(char::is_uppercase) {
        let mut chars = synonym.chars();
        if let Some(first) = chars.next() {
            return first.to_uppercase().chain(chars).collect();
        }
    }
    synonym.to_string()
}

/// 類義語辞典 for every session.
pub trait WordThesaurus: WordSession {
    /// The word the selection is, or the caret is in, with where it is.
    fn word_for_thesaurus(&self) -> Option<(Position, Position, String)> {
        let range = self.selection_range()?;
        let (start, end) = range.normalized();
        let text = self.text_of(start.paragraph)?;
        if start != end && start.paragraph == end.paragraph {
            let word = text.get(start.offset..end.offset)?.trim().to_string();
            return (!word.is_empty()).then_some((
                (start.paragraph, start.offset),
                (end.paragraph, end.offset),
                word,
            ));
        }
        let at = start.offset.min(text.len());
        let is_word = |ch: char| ch.is_alphanumeric() || ch == '\'' || ch == '-';
        let from = text[..at]
            .char_indices()
            .rev()
            .take_while(|(_, ch)| is_word(*ch))
            .last()
            .map_or(at, |(index, _)| index);
        let to = text[at..]
            .char_indices()
            .find(|(_, ch)| !is_word(*ch))
            .map_or(text.len(), |(index, _)| at + index);
        let word = text[from..to].trim_matches(['\'', '-']).to_string();
        (!word.is_empty()).then_some(((start.paragraph, from), (start.paragraph, to), word))
    }

    /// 挿入: `synonym` in the word's place, with its capitals.
    fn replace_with_synonym(
        &mut self,
        (start, end, word): &(Position, Position, String),
        synonym: &str,
    ) -> Result<(), String> {
        self.apply_command(
            HeadlessCommand::ReplaceRange {
                start_paragraph: start.0,
                start_offset: start.1,
                end_paragraph: end.0,
                end_offset: end.1,
                text: matching_case(word, synonym),
            },
            false,
        )
    }
}

impl<T: WordSession + ?Sized> WordThesaurus for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dictionary_entry_splits_into_senses() {
        // Laid out as the system thesaurus gives an entry (made-up text).
        let entry = "glad adjective 1 we were glad to see the sun. pleased, happy, delighted, cheered; informal chuffed, tickled; British English informal over the moon. ANTONYMS sorry. 2 she would be glad to help. willing, ready, eager; dated fain. noun 1 a glad of the field. meadow, lea. ";
        let senses = parse_thesaurus_entry(entry);
        assert_eq!(senses.len(), 3, "{senses:?}");
        assert_eq!(senses[0].label, "adjective: we were glad to see the sun");
        assert_eq!(
            senses[0].synonyms,
            [
                "pleased",
                "happy",
                "delighted",
                "cheered",
                "chuffed",
                "tickled",
                "over the moon"
            ]
        );
        assert_eq!(senses[1].synonyms, ["willing", "ready", "eager", "fain"]);
        assert_eq!(senses[2].label, "noun: a glad of the field");
        assert_eq!(senses[2].synonyms, ["meadow", "lea"]);
        // An example ending in a colon reads too.
        let colon = parse_thesaurus_entry("1 she looked glad: cheerful, merry.");
        assert_eq!(colon[0].synonyms, ["cheerful", "merry"]);
    }

    #[test]
    fn synonyms_take_the_words_capitals() {
        assert_eq!(matching_case("Happy", "glad"), "Glad");
        assert_eq!(matching_case("HAPPY", "glad"), "GLAD");
        assert_eq!(matching_case("happy", "glad"), "glad");
    }

    #[test]
    fn the_builtin_list_finds_a_word_from_either_side() {
        let thesaurus = BuiltinThesaurus;
        assert!(thesaurus.lookup("Begin")[0]
            .synonyms
            .contains(&"start".to_string()));
        let found = thesaurus.lookup("start");
        assert_eq!(found[0].label, "begin");
        assert!(!found[0].synonyms.contains(&"start".to_string()));
        assert!(thesaurus.lookup("xyzzy").is_empty());
    }
}
