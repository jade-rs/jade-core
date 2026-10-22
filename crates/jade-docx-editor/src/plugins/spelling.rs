//! Ephemeral spelling results, independent of guest plugin configuration.
use super::{DecoratedRange, DecorationStyle};
use jade_docx_reader::document::RunProperties;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

const CACHE_LIMIT: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpellingRequest {
    pub key: String,
    pub text: String,
    pub start: usize,
    pub end: usize,
    pub language: Option<String>,
}

/// What a checker found wrong with a span of text.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ProofingKind {
    /// A misspelt word: Word's red wavy underline.
    Spelling,
    /// A grammar or style issue: Word's blue double underline.
    Grammar,
}

/// One issue a checker found, as byte offsets into the request's text.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProofingIssue {
    pub start: usize,
    pub end: usize,
    pub kind: ProofingKind,
}

/// Word's colours: red for spelling, blue for grammar.
pub const SPELLING_COLOR: &str = "#e53935";
pub const GRAMMAR_COLOR: &str = "#2B6CD4";

#[derive(Default)]
pub struct SpellingCache {
    pub enabled: bool,
    pub generation: u32,
    requests: HashMap<String, SpellingRequest>,
    order: VecDeque<String>,
    pending: Vec<SpellingRequest>,
    results: HashMap<String, Vec<DecoratedRange>>,
}

impl SpellingCache {
    pub fn configure(&mut self, enabled: bool, generation: u32) {
        *self = Self {
            enabled,
            generation,
            ..Self::default()
        };
    }

    pub fn take_pending(&mut self) -> Vec<SpellingRequest> {
        std::mem::take(&mut self.pending)
    }

    pub fn decorations<'a>(
        &mut self,
        text: &str,
        runs: impl IntoIterator<Item = (usize, usize, Option<&'a RunProperties>)>,
    ) -> Vec<DecoratedRange> {
        if !self.enabled || text.is_empty() {
            return Vec::new();
        }
        let mut spans: Vec<(usize, usize, Option<String>)> = Vec::new();
        for (start, end, properties) in runs {
            if start >= end
                || end > text.len()
                || !text.is_char_boundary(start)
                || !text.is_char_boundary(end)
            {
                continue;
            }
            let language = properties
                .and_then(|p| p.language.as_deref().or(p.east_asia_language.as_deref()))
                .map(str::to_owned);
            if let Some(last) = spans.last_mut() {
                if last.1 == start && last.2 == language {
                    last.1 = end;
                    continue;
                }
            }
            spans.push((start, end, language));
        }
        let mut decorations = Vec::new();
        for (start, end, language) in spans {
            let key =
                serde_json::to_string(&(text, start, end, &language)).expect("spelling cache key");
            if let Some(ranges) = self.results.get(&key) {
                decorations.extend(ranges.iter().cloned());
            } else if !self.requests.contains_key(&key) {
                // Retire old contexts so a long editing session can keep checking new text.
                if self.requests.len() >= CACHE_LIMIT {
                    if let Some(old) = self.order.pop_front() {
                        self.requests.remove(&old);
                        self.results.remove(&old);
                        self.pending.retain(|request| request.key != old);
                    }
                }
                let request = SpellingRequest {
                    key: key.clone(),
                    text: text.to_owned(),
                    start,
                    end,
                    language,
                };
                self.order.push_back(key.clone());
                self.requests.insert(key, request.clone());
                self.pending.push(request);
            }
        }
        decorations
    }

    pub fn apply(
        &mut self,
        generation: u32,
        key: &str,
        matches: &[(usize, usize)],
    ) -> Option<String> {
        let issues: Vec<ProofingIssue> = matches
            .iter()
            .map(|&(start, end)| ProofingIssue {
                start,
                end,
                kind: ProofingKind::Spelling,
            })
            .collect();
        self.apply_issues(generation, key, &issues)
    }

    /// Records what a checker found in request `key`'s span — misspellings
    /// drawn with a red wave, grammar with a blue double line — and returns
    /// the paragraph text when that changes what it shows.
    pub fn apply_issues(
        &mut self,
        generation: u32,
        key: &str,
        issues: &[ProofingIssue],
    ) -> Option<String> {
        if !self.enabled || generation != self.generation {
            return None;
        }
        let request = self.requests.get(key)?;
        let ranges: Vec<_> = issues
            .iter()
            .filter_map(|issue| {
                let (start, end) = (issue.start, issue.end);
                if start >= end
                    || start < request.start
                    || end > request.end
                    || !request.text.is_char_boundary(start)
                    || !request.text.is_char_boundary(end)
                {
                    return None;
                }
                Some(DecoratedRange {
                    start,
                    end,
                    style: match issue.kind {
                        ProofingKind::Spelling => DecorationStyle::Squiggly {
                            color: SPELLING_COLOR.into(),
                        },
                        ProofingKind::Grammar => DecorationStyle::DoubleUnderline {
                            color: GRAMMAR_COLOR.into(),
                        },
                    },
                })
            })
            .collect();
        let changed = self
            .results
            .get(key)
            .map_or(!ranges.is_empty(), |prev| prev != &ranges);
        self.results.insert(key.to_owned(), ranges);
        changed.then(|| request.text.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spelling_cache_separates_languages_and_rejects_stale_or_invalid_results() {
        let mut cache = SpellingCache::default();
        cache.configure(true, 1);
        let french = RunProperties {
            language: Some("fr-FR".into()),
            ..RunProperties::default()
        };
        let english = RunProperties {
            language: Some("en-US".into()),
            ..RunProperties::default()
        };
        let text = "😀 helllo";
        assert!(cache
            .decorations(text, [(0, text.len(), Some(&english))])
            .is_empty());
        let en = cache.take_pending().pop().unwrap();
        assert_eq!(
            cache.apply(1, &en.key, &[(5, 11), (1, 4), (5, 20)]),
            Some(text.into())
        );
        assert_eq!(
            cache
                .decorations(text, [(0, text.len(), Some(&english))])
                .len(),
            1
        );
        assert!(cache
            .decorations(text, [(0, text.len(), Some(&french))])
            .is_empty());
        assert_ne!(cache.take_pending()[0].key, en.key);
        cache.configure(false, 2);
        assert!(cache.apply(1, &en.key, &[(5, 11)]).is_none());
        assert!(cache
            .decorations(text, [(0, text.len(), Some(&english))])
            .is_empty());
    }

    #[test]
    fn grammar_issues_are_drawn_as_a_blue_double_line() {
        let mut cache = SpellingCache::default();
        cache.configure(true, 3);
        let text = "Their is a dog.";
        cache.decorations(text, [(0, text.len(), None)]);
        let request = cache.take_pending().pop().unwrap();
        let issues = [
            ProofingIssue {
                start: 0,
                end: 5,
                kind: ProofingKind::Grammar,
            },
            ProofingIssue {
                start: 11,
                end: 14,
                kind: ProofingKind::Spelling,
            },
        ];
        assert!(cache.apply_issues(3, &request.key, &issues).is_some());
        let drawn = cache.decorations(text, [(0, text.len(), None)]);
        assert_eq!(
            drawn.iter().map(|range| &range.style).collect::<Vec<_>>(),
            [
                &DecorationStyle::DoubleUnderline {
                    color: GRAMMAR_COLOR.into()
                },
                &DecorationStyle::Squiggly {
                    color: SPELLING_COLOR.into()
                },
            ]
        );
        // The same answer again changes nothing.
        assert!(cache.apply_issues(3, &request.key, &issues).is_none());
    }

    #[test]
    fn spelling_cache_keeps_accepting_new_text_after_capacity_is_reached() {
        let mut cache = SpellingCache::default();
        cache.configure(true, 1);
        let mut first = None;
        for i in 0..=CACHE_LIMIT {
            let text = format!("word{i}");
            cache.decorations(&text, [(0, text.len(), None)]);
            let request = cache.take_pending().pop().unwrap();
            first.get_or_insert_with(|| request.clone());
            cache.apply(1, &request.key, &[(0, text.len())]);
        }
        assert_eq!(cache.requests.len(), CACHE_LIMIT);
        assert_eq!(cache.results.len(), CACHE_LIMIT);
        assert!(cache.apply(1, &first.unwrap().key, &[(0, 5)]).is_none());
    }

    #[test]
    fn spelling_cache_merges_adjacent_runs_without_crossing_language_boundaries() {
        let mut cache = SpellingCache::default();
        cache.configure(true, 9);
        let french = RunProperties {
            language: Some("fr-FR".into()),
            ..RunProperties::default()
        };
        cache.decorations(
            "helllo monde",
            [(0, 2, None), (2, 7, None), (7, 12, Some(&french))],
        );
        let requests = cache.take_pending();
        assert_eq!((requests[0].start, requests[0].end), (0, 7));
        assert_eq!((requests[1].start, requests[1].end), (7, 12));
        assert!(cache.apply(8, &requests[0].key, &[(0, 6)]).is_none());
        assert!(cache.apply(9, &requests[1].key, &[(0, 6)]).is_none());
    }
}
