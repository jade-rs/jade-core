//! Find and Replace (検索と置換) over any [`WordSession`], with Word's two
//! options that matter most: 大文字と小文字を区別する (match case) and 完全に一致
//! する単語だけを検索する (whole words only).
//!
//! The engine's own search folds ASCII case only and knows nothing of word
//! edges, and its replace-all is exact, so both are done here over the
//! paragraphs' text; a replace-all is one batch of range replacements, one
//! undo step however many it makes.

use jade_docx_headless_api::HeadlessCommand;

use super::{Position, WordSession};

/// Word's search options.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindOptions {
    /// 大文字と小文字を区別する: `Jade` does not find `jade`.
    pub match_case: bool,
    /// 完全に一致する単語だけ: `cat` does not find the start of `catalog`.
    pub whole_word: bool,
}

/// A match: a paragraph and the byte range of the text found in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Found {
    pub paragraph: usize,
    pub start: usize,
    pub end: usize,
}

impl Found {
    pub fn start(&self) -> Position {
        (self.paragraph, self.start)
    }

    pub fn end(&self) -> Position {
        (self.paragraph, self.end)
    }
}

/// Where `query` matches in `text`, left to right and not overlapping, as
/// byte ranges of `text`.
pub fn matches_in(text: &str, query: &str, options: FindOptions) -> Vec<(usize, usize)> {
    let wanted: Vec<char> = query.chars().collect();
    if wanted.is_empty() {
        return Vec::new();
    }
    let same =
        |a: char, b: char| a == b || (!options.match_case && a.to_lowercase().eq(b.to_lowercase()));
    let mut found = Vec::new();
    let mut from = 0;
    'starts: for (start, _) in text.char_indices() {
        if start < from {
            continue;
        }
        let mut end = start;
        let mut rest = text[start..].chars();
        for &want in &wanted {
            match rest.next() {
                Some(ch) if same(ch, want) => end += ch.len_utf8(),
                _ => continue 'starts,
            }
        }
        if options.whole_word && !on_word_edges(text, start, end) {
            continue;
        }
        found.push((start, end));
        from = end;
    }
    found
}

/// Whether `text[start..end]` stands as a word: no letter or digit runs on
/// past either end of it, as Word's 完全に一致する単語 decides.
fn on_word_edges(text: &str, start: usize, end: usize) -> bool {
    let word = |ch: Option<char>| ch.is_some_and(|ch| ch.is_alphanumeric() || ch == '_');
    let first = text[start..end].chars().next();
    let last = text[start..end].chars().next_back();
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    !(word(first) && word(before) || word(last) && word(after))
}

/// Find and Replace for every session.
pub trait WordFind: WordSession {
    /// Every match of `query` in the document, in reading order.
    fn find_all(&self, query: &str, options: FindOptions) -> Vec<Found> {
        let document = self.word_document();
        (0..document.paragraph_count())
            .filter_map(|index| Some((index, document.paragraph(index)?)))
            .flat_map(|(index, paragraph)| {
                let text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
                matches_in(&text, query, options)
                    .into_iter()
                    .map(move |(start, end)| Found {
                        paragraph: index,
                        start,
                        end,
                    })
            })
            .collect()
    }

    /// The first match after `from` — the caret, or the end of the match
    /// just shown — wrapping round to the document's start, as 次を検索 does.
    fn find_next(&self, query: &str, options: FindOptions, from: Position) -> Option<Found> {
        let found = self.find_all(query, options);
        found
            .iter()
            .find(|hit| hit.start() >= from)
            .or_else(|| found.first())
            .copied()
    }

    /// The last match before `before`, wrapping round to the end.
    fn find_previous(&self, query: &str, options: FindOptions, before: Position) -> Option<Found> {
        let found = self.find_all(query, options);
        found
            .iter()
            .rev()
            .find(|hit| hit.start() < before)
            .or_else(|| found.last())
            .copied()
    }

    /// Replaces one match (置換) with `replacement`; returns where the text
    /// put in ends, for the next search to go on from.
    fn replace_found(&mut self, found: Found, replacement: &str) -> Result<Position, String> {
        self.apply_command(
            HeadlessCommand::ReplaceRange {
                start_paragraph: found.paragraph,
                start_offset: found.start,
                end_paragraph: found.paragraph,
                end_offset: found.end,
                text: replacement.to_string(),
            },
            false,
        )?;
        Ok((found.paragraph, found.start + replacement.len()))
    }

    /// Replaces every match (すべて置換) as one undo step; answers how many.
    fn replace_all(
        &mut self,
        query: &str,
        replacement: &str,
        options: FindOptions,
    ) -> Result<usize, String> {
        let found = self.find_all(query, options);
        if found.is_empty() {
            return Ok(0);
        }
        // Last first, so each replacement leaves the offsets of the ones
        // before it as they were found.
        let commands = found
            .iter()
            .rev()
            .map(|hit| HeadlessCommand::ReplaceRange {
                start_paragraph: hit.paragraph,
                start_offset: hit.start,
                end_paragraph: hit.paragraph,
                end_offset: hit.end,
                text: replacement.to_string(),
            })
            .collect();
        self.apply_commands(commands, false)?;
        Ok(found.len())
    }
}

impl<T: WordSession + ?Sized> WordFind for T {}

#[cfg(test)]
mod tests {
    use super::*;

    const ANY: FindOptions = FindOptions {
        match_case: false,
        whole_word: false,
    };

    #[test]
    fn case_folds_unless_asked_not_to() {
        assert_eq!(matches_in("Jade, jade, JADE", "jade", ANY).len(), 3);
        let exact = FindOptions {
            match_case: true,
            ..ANY
        };
        assert_eq!(matches_in("Jade, jade, JADE", "jade", exact), [(6, 10)]);
        // Beyond ASCII too, and the offsets stay the text's own.
        assert_eq!(matches_in("Ärger ärger", "ÄRGER", ANY), [(0, 6), (7, 13)]);
    }

    #[test]
    fn whole_words_skip_matches_inside_a_word() {
        let words = FindOptions {
            whole_word: true,
            ..ANY
        };
        assert_eq!(
            matches_in("cat catalog cat.", "cat", words),
            [(0, 3), (12, 15)]
        );
        assert_eq!(matches_in("cat catalog cat.", "cat", ANY).len(), 3);
        // A query that ends in punctuation needs no word edge there.
        assert_eq!(matches_in("a, b", "a,", words), [(0, 2)]);
    }

    #[test]
    fn matches_do_not_overlap() {
        assert_eq!(matches_in("aaaa", "aa", ANY), [(0, 2), (2, 4)]);
        assert!(matches_in("text", "", ANY).is_empty());
        assert_eq!(matches_in("契約書の契約", "契約", ANY), [(0, 6), (12, 18)]);
    }
}
