//! スペル チェックと文章校正: misspellings marked with a red wave and
//! grammar with a blue double line as the pages are drawn, and the 文章校正
//! pane that walks them one at a time — 変更, すべて変更, 無視, すべて無視,
//! 辞書に追加.
//!
//! The checking itself is the shell's: the native app asks macOS's spell
//! checker (`NSSpellChecker`), and anything else that implements
//! [`SpellChecker`] will do. What is checked, what is drawn and what the
//! pane does with an answer are here, for both shells.

pub use crate::plugins::spelling::{ProofingIssue, ProofingKind};
use jade_docx_headless_api::HeadlessCommand;

use super::find::{matches_in, FindOptions};
use super::{Position, WordSession};

/// A spelling and grammar checker.
pub trait SpellChecker {
    /// What is wrong in `text[start..end]`, as byte offsets into `text`.
    /// `language` is the span's `w:lang`, when it has one.
    fn check(
        &mut self,
        text: &str,
        start: usize,
        end: usize,
        language: Option<&str>,
    ) -> Vec<ProofingIssue>;

    /// What `text[start..end]` might have been meant to be, best first.
    fn suggestions(
        &mut self,
        text: &str,
        start: usize,
        end: usize,
        kind: ProofingKind,
        language: Option<&str>,
    ) -> Vec<String>;

    /// What is wrong with a grammar issue, in the checker's words.
    fn description(&mut self, _text: &str, _start: usize, _end: usize) -> Option<String> {
        None
    }

    /// すべて無視: `word` is right in this document from now on.
    fn ignore(&mut self, word: &str);

    /// 辞書に追加: `word` is right everywhere from now on.
    fn learn(&mut self, word: &str);
}

/// A document whose pages can show proofing marks.
pub trait WordProofingDisplay {
    /// Marks on or off; each change starts afresh, so what was ignored or
    /// learnt since is taken into account.
    fn set_proofing(&mut self, on: bool) -> Result<(), String>;
    fn proofing(&self) -> bool;
    /// Asks `checker` about what was drawn since last time and draws its
    /// answers; returns whether the pages changed.
    fn proof_with(&mut self, checker: &mut dyn SpellChecker) -> Result<bool, String>;
}

/// An issue in the document, with what the pane shows about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub paragraph: usize,
    pub start: usize,
    pub end: usize,
    /// The text at fault.
    pub text: String,
    pub kind: ProofingKind,
    pub suggestions: Vec<String>,
    /// The checker's note on a grammar issue.
    pub description: Option<String>,
}

impl Issue {
    pub fn start(&self) -> Position {
        (self.paragraph, self.start)
    }

    pub fn end(&self) -> Position {
        (self.paragraph, self.end)
    }
}

/// A paragraph's text, as the engine's offsets count it.
fn paragraph_text<S: WordSession + ?Sized>(session: &S, index: usize) -> Option<String> {
    let paragraph = session.word_document().paragraph(index)?;
    Some(paragraph.runs.iter().map(|run| run.text.as_str()).collect())
}

/// Proofing for every session.
pub trait WordProofing: WordSession {
    /// Every issue in paragraph `index`, left to right.
    fn issues_in(&self, index: usize, checker: &mut dyn SpellChecker) -> Vec<Issue> {
        let Some(text) = paragraph_text(self, index) else {
            return Vec::new();
        };
        if text.trim().is_empty() {
            return Vec::new();
        }
        let mut issues = checker.check(&text, 0, text.len(), None);
        issues.sort_by_key(|issue| (issue.start, issue.end));
        issues
            .into_iter()
            .filter(|issue| {
                issue.start < issue.end
                    && issue.end <= text.len()
                    && text.is_char_boundary(issue.start)
                    && text.is_char_boundary(issue.end)
            })
            .map(|issue| Issue {
                paragraph: index,
                start: issue.start,
                end: issue.end,
                text: text[issue.start..issue.end].to_string(),
                kind: issue.kind,
                suggestions: Vec::new(),
                description: None,
            })
            .collect()
    }

    /// The issue at or after `from`, round to the document's start; with
    /// its suggestions. `skip` is what 無視 passed over.
    fn next_issue(
        &self,
        from: Position,
        skip: &[(usize, usize, usize)],
        checker: &mut dyn SpellChecker,
    ) -> Option<Issue> {
        let count = self.word_document().paragraph_count();
        if count == 0 {
            return None;
        }
        let skipped = |issue: &Issue| skip.contains(&(issue.paragraph, issue.start, issue.end));
        let first = from.0.min(count - 1);
        let order = (first..count).chain(0..=first);
        for (step, index) in order.enumerate() {
            let wrapped = step > count - first - 1;
            let found = self.issues_in(index, checker).into_iter().find(|issue| {
                let after = if index == first && !wrapped {
                    issue.start >= from.1
                } else if index == first {
                    issue.start < from.1
                } else {
                    true
                };
                after && !skipped(issue)
            });
            if let Some(mut issue) = found {
                let text = paragraph_text(self, index).unwrap_or_default();
                issue.suggestions =
                    checker.suggestions(&text, issue.start, issue.end, issue.kind, None);
                if issue.kind == ProofingKind::Grammar {
                    issue.description = checker.description(&text, issue.start, issue.end);
                }
                return Some(issue);
            }
        }
        None
    }

    /// The issue the point `at` is in, for the menu a right-click opens.
    fn issue_at(&self, at: Position, checker: &mut dyn SpellChecker) -> Option<Issue> {
        let mut issue = self
            .issues_in(at.0, checker)
            .into_iter()
            .find(|issue| issue.start <= at.1 && at.1 <= issue.end)?;
        let text = paragraph_text(self, at.0)?;
        issue.suggestions = checker.suggestions(&text, issue.start, issue.end, issue.kind, None);
        Some(issue)
    }

    /// 変更: the issue's text replaced with `replacement`; returns where the
    /// text put in ends, for the pane to go on from.
    fn change_issue(&mut self, issue: &Issue, replacement: &str) -> Result<Position, String> {
        self.apply_command(
            HeadlessCommand::ReplaceRange {
                start_paragraph: issue.paragraph,
                start_offset: issue.start,
                end_paragraph: issue.paragraph,
                end_offset: issue.end,
                text: replacement.to_string(),
            },
            false,
        )?;
        Ok((issue.paragraph, issue.start + replacement.len()))
    }

    /// すべて変更: every whole-word `word` in the document replaced, as one
    /// undo step; answers how many.
    fn change_every(&mut self, word: &str, replacement: &str) -> Result<usize, String> {
        let options = FindOptions {
            match_case: true,
            whole_word: true,
        };
        let count = self.word_document().paragraph_count();
        let mut commands = Vec::new();
        for index in (0..count).rev() {
            let Some(text) = paragraph_text(self, index) else {
                continue;
            };
            for (start, end) in matches_in(&text, word, options).into_iter().rev() {
                commands.push(HeadlessCommand::ReplaceRange {
                    start_paragraph: index,
                    start_offset: start,
                    end_paragraph: index,
                    end_offset: end,
                    text: replacement.to_string(),
                });
            }
        }
        let changed = commands.len();
        if changed > 0 {
            self.apply_commands(commands, false)?;
        }
        Ok(changed)
    }
}

impl<T: WordSession + ?Sized> WordProofing for T {}

/// The 文章校正 pane: the issue it is on, the suggestion chosen, and the
/// occurrences 無視 passed over.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProofingPane {
    pub current: Option<Issue>,
    pub chosen: usize,
    /// The whole document has been checked: 文章校正が完了しました.
    pub finished: bool,
    pub skipped: Vec<(usize, usize, usize)>,
}

impl ProofingPane {
    /// The suggestion 変更 puts in.
    pub fn replacement(&self) -> Option<&str> {
        let issue = self.current.as_ref()?;
        issue.suggestions.get(self.chosen).map(String::as_str)
    }

    /// Moves on to the next issue from `from`.
    pub fn advance<S: WordProofing + ?Sized>(
        &mut self,
        session: &S,
        from: Position,
        checker: &mut dyn SpellChecker,
    ) {
        self.chosen = 0;
        self.current = session.next_issue(from, &self.skipped, checker);
        self.finished = self.current.is_none();
    }

    /// 無視: this occurrence passed over, the next one shown.
    pub fn ignore_once<S: WordProofing + ?Sized>(
        &mut self,
        session: &S,
        checker: &mut dyn SpellChecker,
    ) {
        let Some(issue) = self.current.take() else {
            return;
        };
        self.skipped.push((issue.paragraph, issue.start, issue.end));
        self.advance(session, issue.end(), checker);
    }
}

/// A checker over a fixed list of words, for documents in a language the
/// platform cannot check, and for tests: a word not in the list is
/// misspelt, its suggestions the listed words a letter or two away; a word
/// written twice running (`the the`) is a grammar issue, as Word marks it.
#[derive(Clone, Debug, Default)]
pub struct WordListChecker {
    words: std::collections::BTreeSet<String>,
    ignored: std::collections::BTreeSet<String>,
}

impl WordListChecker {
    pub fn new<'a>(words: impl IntoIterator<Item = &'a str>) -> Self {
        Self {
            words: words.into_iter().map(str::to_lowercase).collect(),
            ignored: Default::default(),
        }
    }

    fn knows(&self, word: &str) -> bool {
        let lower = word.to_lowercase();
        self.words.contains(&lower) || self.ignored.contains(&lower)
    }
}

/// The Latin words in `text[start..end]`, as byte ranges of `text`.
fn latin_words(text: &str, start: usize, end: usize) -> Vec<(usize, usize)> {
    let mut words = Vec::new();
    let mut current: Option<usize> = None;
    for (at, ch) in text[start..end].char_indices() {
        let at = start + at;
        let letter = ch.is_ascii_alphabetic() || (ch == '\'' && current.is_some());
        match (letter, current) {
            (true, None) => current = Some(at),
            (false, Some(from)) => {
                words.push((from, at));
                current = None;
            }
            _ => {}
        }
    }
    if let Some(from) = current {
        words.push((from, end));
    }
    words
        .into_iter()
        .map(|(from, to)| {
            let trimmed = text[from..to].trim_end_matches('\'');
            (from, from + trimmed.len())
        })
        .filter(|(from, to)| from < to)
        .collect()
}

/// Levenshtein distance, for suggestions.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substitution = previous + usize::from(ca != *cb);
            previous = row[j + 1];
            row[j + 1] = substitution.min(row[j] + 1).min(row[j + 1] + 1);
        }
    }
    row[b.len()]
}

impl SpellChecker for WordListChecker {
    fn check(
        &mut self,
        text: &str,
        start: usize,
        end: usize,
        _language: Option<&str>,
    ) -> Vec<ProofingIssue> {
        let words = latin_words(text, start, end);
        let mut issues = Vec::new();
        for (index, &(from, to)) in words.iter().enumerate() {
            let word = &text[from..to];
            if !self.knows(word) {
                issues.push(ProofingIssue {
                    start: from,
                    end: to,
                    kind: ProofingKind::Spelling,
                });
            } else if let Some(&(before, before_end)) = index.checked_sub(1).map(|at| &words[at]) {
                let between = &text[before_end..from];
                if between.chars().all(char::is_whitespace)
                    && text[before..before_end].eq_ignore_ascii_case(word)
                {
                    issues.push(ProofingIssue {
                        start: before,
                        end: to,
                        kind: ProofingKind::Grammar,
                    });
                }
            }
        }
        issues
    }

    fn suggestions(
        &mut self,
        text: &str,
        start: usize,
        end: usize,
        kind: ProofingKind,
        _language: Option<&str>,
    ) -> Vec<String> {
        let wrong = &text[start..end];
        if kind == ProofingKind::Grammar {
            // A word twice: once.
            return wrong
                .split_whitespace()
                .next()
                .map(|word| vec![word.to_string()])
                .unwrap_or_default();
        }
        let lower = wrong.to_lowercase();
        let mut close: Vec<(usize, &String)> = self
            .words
            .iter()
            .map(|word| (distance(&lower, word), word))
            .filter(|(far, _)| *far <= 2)
            .collect();
        close.sort();
        let capital = wrong.chars().next().is_some_and(char::is_uppercase);
        close
            .into_iter()
            .take(5)
            .map(|(_, word)| {
                if capital {
                    let mut chars = word.chars();
                    chars
                        .next()
                        .map(|first| first.to_uppercase().chain(chars).collect())
                        .unwrap_or_default()
                } else {
                    word.clone()
                }
            })
            .collect()
    }

    fn description(&mut self, _text: &str, _start: usize, _end: usize) -> Option<String> {
        Some("repeated word".to_string())
    }

    fn ignore(&mut self, word: &str) {
        self.ignored.insert(word.to_lowercase());
    }

    fn learn(&mut self, word: &str) {
        self.words.insert(word.to_lowercase());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_list_marks_misspellings_and_repeated_words() {
        let mut checker = WordListChecker::new(["the", "cat", "sat", "on", "mat"]);
        let text = "The cat sta on the the mat";
        let issues = checker.check(text, 0, text.len(), None);
        assert_eq!(
            issues,
            [
                ProofingIssue {
                    start: 8,
                    end: 11,
                    kind: ProofingKind::Spelling
                },
                ProofingIssue {
                    start: 15,
                    end: 22,
                    kind: ProofingKind::Grammar
                },
            ]
        );
        assert_eq!(
            checker.suggestions(text, 8, 11, ProofingKind::Spelling, None)[0],
            "sat"
        );
        assert_eq!(
            checker.suggestions(text, 15, 22, ProofingKind::Grammar, None),
            ["the"]
        );
        checker.ignore("sta");
        assert_eq!(checker.check(text, 0, 11, None), []);
        // Japanese is not this checker's to judge.
        assert!(checker.check("日本語の文", 0, 15, None).is_empty());
    }
}
