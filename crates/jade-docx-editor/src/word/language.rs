//! 校閲 → 言語 → 校正言語の設定: the language a selection is marked in
//! (`w:lang`), what proofing it gets (`w:noProof`), and 言語を自動的に検出
//! する — Japanese marked as East Asian text, the rest by its script.

use jade_docx_headless_api::{HeadlessCommand, HeadlessRunFormat};

use super::{Position, WordSession};

/// The languages the dialog offers: a BCP 47 tag, whether it is an East
/// Asian language (`w:eastAsia`), and its name's i18n key.
pub const LANGUAGES: [(&str, bool, &str); 14] = [
    ("ja-JP", true, "language.ja_jp"),
    ("en-US", false, "language.en_us"),
    ("en-GB", false, "language.en_gb"),
    ("zh-CN", true, "language.zh_cn"),
    ("zh-TW", true, "language.zh_tw"),
    ("ko-KR", true, "language.ko_kr"),
    ("fr-FR", false, "language.fr_fr"),
    ("de-DE", false, "language.de_de"),
    ("es-ES", false, "language.es_es"),
    ("it-IT", false, "language.it_it"),
    ("pt-BR", false, "language.pt_br"),
    ("ru-RU", false, "language.ru_ru"),
    ("th-TH", false, "language.th_th"),
    ("ar-SA", false, "language.ar_sa"),
];

/// Whether `tag` is marked as East Asian text.
pub fn is_east_asian(tag: &str) -> bool {
    LANGUAGES
        .iter()
        .find(|(each, _, _)| each.eq_ignore_ascii_case(tag))
        .is_some_and(|(_, east_asian, _)| *east_asian)
        || tag.starts_with("ja")
        || tag.starts_with("zh")
        || tag.starts_with("ko")
}

/// The language a character's script says, or `None` for what has no
/// language of its own (spaces, digits, punctuation).
pub fn language_of(ch: char) -> Option<&'static str> {
    let code = ch as u32;
    Some(match code {
        0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F => "ja-JP",
        0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF => "ja-JP",
        0xAC00..=0xD7AF | 0x1100..=0x11FF | 0x3130..=0x318F => "ko-KR",
        0x0E00..=0x0E7F => "th-TH",
        0x0600..=0x06FF => "ar-SA",
        0x0400..=0x04FF => "ru-RU",
        _ if ch.is_alphabetic() => "en-US",
        _ => return None,
    })
}

/// 言語を自動的に検出する: `text` cut into runs of one language, each as a
/// byte range and its tag; what has no language joins the run before it.
pub fn detect(text: &str) -> Vec<(usize, usize, &'static str)> {
    let mut runs: Vec<(usize, usize, &'static str)> = Vec::new();
    for (at, ch) in text.char_indices() {
        let end = at + ch.len_utf8();
        let language = language_of(ch);
        match (runs.last_mut(), language) {
            (Some(last), Some(language)) if last.2 == language => last.1 = end,
            (Some(last), None) => last.1 = end,
            (_, Some(language)) => runs.push((at, end, language)),
            (None, None) => {}
        }
    }
    runs
}

/// The language most of `text` is in.
pub fn dominant(text: &str) -> Option<&'static str> {
    let mut counts: Vec<(&'static str, usize)> = Vec::new();
    for ch in text.chars() {
        if let Some(language) = language_of(ch) {
            match counts.iter_mut().find(|(each, _)| *each == language) {
                Some(entry) => entry.1 += 1,
                None => counts.push((language, 1)),
            }
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(language, _)| language)
}

/// What 校正言語の設定 sets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanguageChoice {
    pub language: String,
    pub no_proof: bool,
    /// 言語を自動的に検出する: each run of one script gets its own.
    pub detect: bool,
}

/// The patch that marks text as `language`.
fn patch(language: &str, no_proof: bool) -> HeadlessRunFormat {
    let mut format = HeadlessRunFormat {
        no_proof: Some(no_proof),
        ..HeadlessRunFormat::default()
    };
    if is_east_asian(language) {
        format.east_asia_language = Some(language.to_string());
    } else {
        format.language = Some(language.to_string());
    }
    format
}

/// Language marking for every session.
pub trait WordLanguage: WordSession {
    /// Marks `start..end` with the choice, as one undo step.
    fn set_language(
        &mut self,
        start: Position,
        end: Position,
        choice: &LanguageChoice,
    ) -> Result<(), String> {
        let mut commands = Vec::new();
        if choice.detect {
            for paragraph in start.0..=end.0 {
                let text: String = self
                    .word_document()
                    .paragraph(paragraph)
                    .map(|p| p.runs.iter().map(|run| run.text.as_str()).collect())
                    .unwrap_or_default();
                let from = if paragraph == start.0 { start.1 } else { 0 };
                let to = if paragraph == end.0 {
                    end.1
                } else {
                    text.len()
                };
                for (run_start, run_end, language) in detect(&text) {
                    let (a, b) = (run_start.max(from), run_end.min(to));
                    if a < b {
                        commands.push(HeadlessCommand::SetRunFormat {
                            start_paragraph: paragraph,
                            start_offset: a,
                            end_paragraph: paragraph,
                            end_offset: b,
                            format: Box::new(patch(language, choice.no_proof)),
                        });
                    }
                }
            }
        } else {
            commands.push(HeadlessCommand::SetRunFormat {
                start_paragraph: start.0,
                start_offset: start.1,
                end_paragraph: end.0,
                end_offset: end.1,
                format: Box::new(patch(&choice.language, choice.no_proof)),
            });
        }
        if commands.is_empty() {
            return Ok(());
        }
        self.apply_commands(commands, true)
    }

    /// The language the text at `position` is marked in: its run's
    /// `w:lang`, East Asian for East Asian text.
    fn language_at(&self, position: Position) -> Option<String> {
        let paragraph = self.word_document().paragraph(position.0)?;
        let mut at = 0;
        for run in &paragraph.runs {
            let end = at + run.text.len();
            if position.1 >= at && position.1 <= end {
                let props = run.properties.as_ref()?;
                let ch = run
                    .text
                    .get(position.1.saturating_sub(at)..)?
                    .chars()
                    .next();
                let east = ch.and_then(language_of).is_some_and(is_east_asian);
                let tag = if east {
                    props
                        .east_asia_language
                        .as_deref()
                        .or(props.language.as_deref())
                } else {
                    props.language.as_deref()
                };
                return tag.map(str::to_string);
            }
            at = end;
        }
        None
    }
}

impl<T: WordSession + ?Sized> WordLanguage for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_say_the_language() {
        let text = "これは test です。";
        let runs = detect(text);
        assert_eq!(runs.len(), 3, "{runs:?}");
        assert_eq!(runs[0].2, "ja-JP");
        assert_eq!(&text[runs[1].0..runs[1].1], "test ");
        assert_eq!(runs[1].2, "en-US");
        assert_eq!(dominant("Hello 世界のみなさん"), Some("ja-JP"));
        assert_eq!(dominant("안녕하세요"), Some("ko-KR"));
        assert_eq!(dominant("123"), None);
        assert!(is_east_asian("ja-JP") && !is_east_asian("en-US"));
        assert_eq!(
            patch("ja-JP", false).east_asia_language.as_deref(),
            Some("ja-JP")
        );
        assert_eq!(patch("en-GB", true).language.as_deref(), Some("en-GB"));
    }
}
