//! Structure of Japanese contracts and regulations: article numbers, article
//! titles, article-sized chunks, and the cross-references between them.
//!
//! These documents carry a second structure on top of the OOXML one. `第５条`
//! is the address a reader, a reviewer and a counterparty all use; the title
//! (`秘密保持`) is how they find it; and the body cites both (`前条の規定に
//! かかわらず`). None of that is expressed in the file format — Word only sees
//! paragraphs, and the article number is often not even in the run text but
//! rendered from `w:numPr`. Recovering it is what lets an agent read a contract
//! the way a lawyer does instead of scrolling a paragraph at a time.
//!
//! Everything here works on [`ParagraphInput`] rather than a document, so the
//! rules can be tested directly against the shapes real drafts use.

use jade_docx_headless_api::{HeadlessChunkSnapshot, HeadlessReferenceSnapshot};

/// One paragraph as the structure rules see it.
pub(crate) struct ParagraphInput {
    /// The paragraph's own run text.
    pub text: String,
    /// Numbering label Word renders in front of it (`第１条　`). Japanese
    /// contracts usually keep the article number here, not in the text.
    pub numbering_text: Option<String>,
    pub para_id: Option<String>,
    pub in_table: bool,
}

impl ParagraphInput {
    /// Text the structure rules read: the rendered numbering label followed by
    /// the paragraph's own text, which is what the reader sees on the page.
    fn lookup_text(&self) -> String {
        match self.numbering_text.as_deref() {
            Some(marker) => format!("{marker}{}", self.text),
            None => self.text.clone(),
        }
    }
}

/// How a document is cut into chunks.
pub(crate) struct ChunkOptions {
    pub disable_table_splitting: bool,
    pub enable_branch_splitting: bool,
    pub include_text: bool,
}

/// A Japanese ordinal parsed out of a heading or a citation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArticleNumber {
    /// As written, including the `第` and the counter (`第2条の2`).
    pub label: String,
    /// Normalised for comparison: `5`, or `2-2` for `第2条の2`.
    pub key: String,
    /// Bytes consumed from the front of the input.
    pub len: usize,
}

/// Parses `第N条`, `第N条のM`, `第N条のMのL` at the start of `text`.
pub(crate) fn parse_article_number(text: &str) -> Option<ArticleNumber> {
    parse_numbered_prefix(text, '条')
}

/// Parses a structural heading number (`第3章`, `第2節`) at the start of `text`.
pub(crate) fn parse_section_number(text: &str) -> Option<(ArticleNumber, char)> {
    ['編', '章', '節', '款', '目']
        .into_iter()
        .find_map(|counter| parse_numbered_prefix(text, counter).map(|number| (number, counter)))
}

/// Shared parser for `第<number><counter>` with stacked branch numbers.
fn parse_numbered_prefix(text: &str, counter: char) -> Option<ArticleNumber> {
    let rest = text.strip_prefix('第')?;
    let (value, rest) = take_number(rest)?;
    let rest = rest.strip_prefix(counter)?;

    let mut key = value.to_string();
    let mut consumed = text.len() - rest.len();
    let mut rest = rest;
    // Branch numbers stack: 第2条の2の3.
    while let Some(after_no) = rest.strip_prefix('の') {
        let Some((branch, next)) = take_number(after_no) else {
            break; // `の` followed by prose — the number ended at the counter.
        };
        key.push('-');
        key.push_str(&branch.to_string());
        rest = next;
        consumed = text.len() - rest.len();
    }

    Some(ArticleNumber {
        label: text[..consumed].to_string(),
        key,
        len: consumed,
    })
}

/// Consumes a leading number — ASCII, full width, or kanji — and returns its
/// value with the remaining text.
fn take_number(text: &str) -> Option<(u32, &str)> {
    let end = text
        .char_indices()
        .take_while(|(_, ch)| is_heading_number_char(*ch))
        .last()
        .map(|(index, ch)| index + ch.len_utf8())?;
    let value = japanese_number_value(&text[..end])?;
    Some((value, &text[end..]))
}

/// Numeric value of a Japanese ordinal: `12`, `１２`, `十二`, `一二` all read
/// as 12. Kanji may be positional (`一〇`) or use the myriad units (`二十三`),
/// and drafts mix both.
pub(crate) fn japanese_number_value(text: &str) -> Option<u32> {
    if text.is_empty() {
        return None;
    }
    if text.chars().all(is_ascii_or_full_width_digit) {
        return text
            .chars()
            .map(full_width_digit_to_ascii)
            .collect::<String>()
            .parse()
            .ok();
    }

    let mut total = 0u32;
    let mut current = 0u32;
    let mut positional = false;
    for ch in text.chars() {
        if let Some(digit) = kanji_digit(ch) {
            current = current.checked_mul(10)?.checked_add(digit)?;
            positional = true;
            continue;
        }
        let unit = match ch {
            '十' => 10,
            '百' => 100,
            '千' => 1000,
            _ => return None,
        };
        // A bare unit means one of it: 十 alone is 10, not 0.
        let multiplier = if current == 0 && !positional {
            1
        } else {
            current
        };
        total = total.checked_add(multiplier.max(1).checked_mul(unit)?)?;
        current = 0;
        positional = false;
    }
    total.checked_add(current)
}

fn kanji_digit(ch: char) -> Option<u32> {
    "〇一二三四五六七八九"
        .chars()
        .position(|digit| digit == ch)
        .map(|index| index as u32)
}

/// Recognises a Japanese article heading: `第1条` / `第１条` / `第一条`, an
/// optional branch number (`第2条の2`), and an optional title after a separator
/// (`第１条　（秘密情報）`, `第1条(目的)`).
///
/// The match is anchored and requires a separator (or end of text) after the
/// number, so prose quoting an article — `第3条の規定にかかわらず…` — is not
/// mistaken for a heading.
pub(crate) fn article_heading_number(text: &str) -> Option<ArticleNumber> {
    let trimmed = text.trim_start_matches(|ch: char| ch.is_whitespace() || ch == '　');
    let number = parse_article_number(trimmed)?;
    match trimmed[number.len..].chars().next() {
        None => Some(number),
        Some(ch) if is_heading_title_separator(ch) => Some(number),
        Some(_) => None,
    }
}

pub(crate) fn is_article_heading(text: &str) -> bool {
    article_heading_number(text).is_some()
}

pub(crate) fn is_heading_number_char(ch: char) -> bool {
    is_ascii_or_full_width_digit(ch) || "〇一二三四五六七八九十百千".contains(ch)
}

/// Characters that may sit between an article number and its title. Anything
/// else means the number is part of a sentence rather than a heading.
pub(crate) fn is_heading_title_separator(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            '（' | '(' | '【' | '〔' | '［' | '[' | '「' | '『' | '：' | ':' | '・' | '－' | '-'
        )
}

pub(crate) fn is_ascii_or_full_width_digit(ch: char) -> bool {
    ch.is_ascii_digit() || ('０'..='９').contains(&ch)
}

pub(crate) fn full_width_digit_to_ascii(ch: char) -> char {
    if ('０'..='９').contains(&ch) {
        let diff = (ch as u32) - ('０' as u32);
        char::from_u32(('0' as u32) + diff).unwrap_or(ch)
    } else {
        ch
    }
}

/// Longest a bare inline phrase may be before it reads as body text rather than
/// an article title. Real titles are short (`秘密保持`, `反社会的勢力の排除`);
/// the first sentence of an article is not.
const MAX_INLINE_TITLE_CHARS: usize = 24;

/// Where an article's title was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TitleSource {
    /// On the article line itself: `第１条　秘密保持`.
    Inline,
    /// On the parenthesised line above it: `（秘密保持）` then `第１条　…`.
    PrecedingLine,
}

impl TitleSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            TitleSource::Inline => "inline",
            TitleSource::PrecedingLine => "preceding_line",
        }
    }
}

/// A whole line that is nothing but a bracketed phrase — the form a Japanese
/// article title takes when it sits on its own line above the article.
pub(crate) fn bracketed_line_title(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let inner = trimmed
        .strip_prefix('（')
        .and_then(|rest| rest.strip_suffix('）'))
        .or_else(|| {
            trimmed
                .strip_prefix('(')
                .and_then(|rest| rest.strip_suffix(')'))
        })?;
    let inner = inner.trim();
    // A bracketed line that itself opens an article is a heading, not a title.
    if inner.is_empty() || inner.contains('（') || inner.contains('(') {
        return None;
    }
    Some(inner.to_string())
}

/// The title written on the article line itself, after the number.
///
/// A bracketed remainder is always a title. A bare one is only a title when it
/// is short and reads as a phrase — `第１条　秘密保持` is a title,
/// `第１条　乙は、甲に対し…` is the article's first sentence.
fn inline_title(after_number: &str) -> Option<String> {
    let rest = after_number.trim_start_matches(|ch: char| ch.is_whitespace() || ch == '　');
    if rest.is_empty() {
        return None;
    }
    if let Some(title) = bracketed_line_title(rest) {
        return Some(title);
    }
    let sentence_like = rest.contains('。')
        || rest.contains('、')
        || rest.contains('，')
        || rest.chars().count() > MAX_INLINE_TITLE_CHARS;
    (!sentence_like).then(|| rest.trim().to_string())
}

/// Resolves an article's title, preferring the parenthesised line above the
/// article over anything written inline.
///
/// Both forms are in daily use, and drafts mix them:
///
/// ```text
/// （秘密保持）        ← the title, on its own line
/// 第１条　ああああ     ← the article, whose text is body
///
/// 第１条　秘密保持     ← the title, written inline
/// ```
///
/// The preceding line wins because when it is present the inline remainder is
/// the article's body text, and guessing the other way round would report a
/// sentence as the title.
pub(crate) fn article_title(
    paragraphs: &[ParagraphInput],
    heading_index: usize,
) -> Option<(String, TitleSource, usize)> {
    let number = article_heading_number(&paragraphs.get(heading_index)?.lookup_text())?;
    article_title_for(paragraphs, heading_index, &number)
}

/// [`article_title`] for a caller that has already parsed the number.
fn article_title_for(
    paragraphs: &[ParagraphInput],
    heading_index: usize,
    number: &ArticleNumber,
) -> Option<(String, TitleSource, usize)> {
    if let Some(previous) = previous_non_empty(paragraphs, heading_index) {
        let text = paragraphs[previous].lookup_text();
        // A previous line that is itself a heading is a neighbour, not a title.
        if article_heading_number(&text).is_none() && parse_section_number(text.trim()).is_none() {
            if let Some(title) = bracketed_line_title(&text) {
                return Some((title, TitleSource::PrecedingLine, previous));
            }
        }
    }

    let text = paragraphs[heading_index].lookup_text();
    let trimmed = text.trim_start_matches(|ch: char| ch.is_whitespace() || ch == '　');
    let after_number = trimmed.get(number.len..)?;
    inline_title(after_number).map(|title| (title, TitleSource::Inline, heading_index))
}

fn previous_non_empty(paragraphs: &[ParagraphInput], index: usize) -> Option<usize> {
    (0..index)
        .rev()
        .find(|candidate| !paragraphs[*candidate].lookup_text().trim().is_empty())
}

/// What a paragraph is, structurally.
enum Boundary {
    Article(ArticleNumber),
    Section(ArticleNumber),
    None,
}

fn classify(paragraphs: &[ParagraphInput], index: usize, options: &ChunkOptions) -> Boundary {
    let text = paragraphs[index].lookup_text();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Boundary::None;
    }
    if let Some(number) = article_heading_number(trimmed) {
        // `第2条の2` continues `第2条` unless the caller asks for branch-level
        // chunks: a sub-article is usually read together with its parent.
        if !options.enable_branch_splitting && number.key.contains('-') {
            return Boundary::None;
        }
        return Boundary::Article(number);
    }
    if let Some((number, _counter)) = parse_section_number(trimmed) {
        return Boundary::Section(number);
    }
    Boundary::None
}

/// Cuts a document into article-sized chunks.
pub(crate) fn build_chunks(
    paragraphs: &[ParagraphInput],
    options: &ChunkOptions,
) -> Vec<HeadlessChunkSnapshot> {
    #[derive(Clone)]
    struct Pending {
        kind: &'static str,
        start: usize,
        number: Option<ArticleNumber>,
        title: Option<(String, TitleSource, usize)>,
    }

    let mut chunks: Vec<HeadlessChunkSnapshot> = Vec::new();
    let mut pending: Option<Pending> = None;
    let mut index = 0usize;

    // `next_start` is where the following chunk begins, so a chunk that the
    // next one swallowed whole — the title line above an article, which opens
    // that article rather than closing the block above it — is dropped.
    let flush = |pending: &mut Option<Pending>,
                 next_start: usize,
                 chunks: &mut Vec<HeadlessChunkSnapshot>| {
        let Some(open) = pending.take() else { return };
        if next_start <= open.start {
            return;
        }
        let index = chunks.len();
        chunks.push(finish_chunk(
            paragraphs,
            options,
            index,
            open.kind,
            open.start,
            next_start - 1,
            open.number,
            open.title,
        ));
    };

    while index < paragraphs.len() {
        // A table is a block: it either becomes its own chunk or is absorbed
        // into the article it appears in, but it is never cut in half.
        if paragraphs[index].in_table {
            let start = index;
            while index < paragraphs.len() && paragraphs[index].in_table {
                index += 1;
            }
            if !options.disable_table_splitting {
                flush(&mut pending, start, &mut chunks);
                chunks.push(finish_chunk(
                    paragraphs,
                    options,
                    chunks.len(),
                    "table",
                    start,
                    index - 1,
                    None,
                    None,
                ));
            }
            continue;
        }

        match classify(paragraphs, index, options) {
            Boundary::Article(number) => {
                let title = article_title_for(paragraphs, index, &number);
                // A title on its own line belongs to the article it announces,
                // not to the article that happens to end above it.
                let start = match title {
                    Some((_, TitleSource::PrecedingLine, line)) => line,
                    _ => index,
                };
                flush(&mut pending, start, &mut chunks);
                pending = Some(Pending {
                    kind: "article",
                    start,
                    number: Some(number),
                    title,
                });
            }
            Boundary::Section(number) => {
                flush(&mut pending, index, &mut chunks);
                pending = Some(Pending {
                    kind: "section",
                    start: index,
                    number: Some(number),
                    title: None,
                });
            }
            Boundary::None => {
                if pending.is_none() {
                    // Everything before the first heading — title block,
                    // parties, recitals — is one chunk of its own.
                    pending = Some(Pending {
                        kind: "preamble",
                        start: index,
                        number: None,
                        title: None,
                    });
                }
            }
        }
        index += 1;
    }
    flush(&mut pending, paragraphs.len(), &mut chunks);
    chunks
}

#[allow(clippy::too_many_arguments)]
fn finish_chunk(
    paragraphs: &[ParagraphInput],
    options: &ChunkOptions,
    index: usize,
    kind: &str,
    start: usize,
    end: usize,
    number: Option<ArticleNumber>,
    title: Option<(String, TitleSource, usize)>,
) -> HeadlessChunkSnapshot {
    let text = (start..=end.min(paragraphs.len().saturating_sub(1)))
        .map(|paragraph| paragraphs[paragraph].lookup_text())
        .collect::<Vec<_>>()
        .join("\n");
    HeadlessChunkSnapshot {
        index,
        kind: kind.to_string(),
        start_paragraph: start,
        end_paragraph: end,
        para_id: paragraphs.get(start).and_then(|p| p.para_id.clone()),
        number: number.as_ref().map(|number| number.label.clone()),
        number_key: number.as_ref().map(|number| number.key.clone()),
        title: title.as_ref().map(|(title, _, _)| title.clone()),
        title_source: title
            .as_ref()
            .map(|(_, source, _)| source.as_str().to_string()),
        title_paragraph: title.as_ref().map(|(_, _, paragraph)| *paragraph),
        char_count: text.chars().count(),
        text: if options.include_text {
            text
        } else {
            String::new()
        },
    }
}

/// One article, as the reference resolver needs to see it.
struct Article {
    paragraph: usize,
    key: String,
    label: String,
    title: Option<String>,
    para_id: Option<String>,
}

fn articles(paragraphs: &[ParagraphInput]) -> Vec<Article> {
    let options = ChunkOptions {
        disable_table_splitting: true,
        enable_branch_splitting: true,
        include_text: false,
    };
    (0..paragraphs.len())
        .filter_map(|index| match classify(paragraphs, index, &options) {
            Boundary::Article(number) => {
                let title =
                    article_title_for(paragraphs, index, &number).map(|(title, _, _)| title);
                Some(Article {
                    paragraph: index,
                    key: number.key,
                    label: number.label,
                    title,
                    para_id: paragraphs[index].para_id.clone(),
                })
            }
            _ => None,
        })
        .collect()
}

/// Finds every cross-reference in the body and resolves it to an article.
pub(crate) fn find_references(paragraphs: &[ParagraphInput]) -> Vec<HeadlessReferenceSnapshot> {
    let articles = articles(paragraphs);
    let mut references = Vec::new();

    for (index, paragraph) in paragraphs.iter().enumerate() {
        // Offsets have to index the paragraph's own text — the numbering label
        // is rendered by Word and is not part of it.
        let text = paragraph.text.as_str();
        let is_heading = article_heading_number(&paragraph.lookup_text()).is_some();
        let mut cursor = 0usize;

        while cursor < text.len() {
            let Some(found) = text[cursor..].find(['第', '前', '次', '本']) else {
                break;
            };
            let start = cursor + found;
            let rest = &text[start..];

            let reference = if rest.starts_with('第') {
                article_reference(rest, start, &articles)
            } else {
                relative_reference(rest, start, index, &articles)
            };

            match reference {
                // An article's own number is a heading, not a citation.
                Some(reference) if is_heading && reference.start_offset == 0 => {
                    cursor = reference.end_offset.max(start + 1);
                }
                Some(mut reference) => {
                    cursor = reference.end_offset.max(start + 1);
                    reference.paragraph = index;
                    references.push(reference);
                }
                None => cursor = start + rest.chars().next().map_or(1, char::len_utf8),
            }
        }
    }
    references
}

/// `第５条`, `第５条第２項`, `第２条の２`, `第３条から第７条まで`.
fn article_reference(
    rest: &str,
    start: usize,
    articles: &[Article],
) -> Option<HeadlessReferenceSnapshot> {
    let number = parse_article_number(rest)?;
    let mut end = number.len;

    // Sub-references narrow the citation (`第２項`, `第３号`) without changing
    // which article it points at.
    let mut sub_references = Vec::new();
    for counter in ['項', '号'] {
        if let Some(sub) = parse_numbered_prefix(&rest[end..], counter) {
            end += sub.len;
            sub_references.push(sub.label);
        }
    }

    // Range: 第３条から第７条まで.
    let mut range_end: Option<ArticleNumber> = None;
    if let Some(after_kara) = rest[end..].strip_prefix("から") {
        if let Some(upper) = parse_article_number(after_kara) {
            let after_upper = &after_kara[upper.len..];
            if let Some(after_made) = after_upper.strip_prefix("まで") {
                end = rest.len() - after_made.len();
                range_end = Some(upper);
            }
        }
    }

    let target = articles.iter().find(|article| article.key == number.key);
    let range_target = range_end
        .as_ref()
        .and_then(|upper| articles.iter().find(|article| article.key == upper.key));

    Some(HeadlessReferenceSnapshot {
        // `find_references` stamps the citing paragraph on the way out.
        paragraph: 0,
        start_offset: start,
        end_offset: start + end,
        text: rest[..end].to_string(),
        kind: "article".to_string(),
        target_number_key: Some(number.key),
        target_paragraph: target.map(|article| article.paragraph),
        target_para_id: target.and_then(|article| article.para_id.clone()),
        target_number: target.map(|article| article.label.clone()),
        target_title: target.and_then(|article| article.title.clone()),
        target_end_paragraph: range_target.map(|article| article.paragraph),
        target_end_number: range_target.map(|article| article.label.clone()),
        sub_references,
        resolved: target.is_some() && (range_end.is_none() || range_target.is_some()),
    })
}

/// `前条` / `次条` / `本条`, resolved against the article the text sits in.
fn relative_reference(
    rest: &str,
    start: usize,
    paragraph: usize,
    articles: &[Article],
) -> Option<HeadlessReferenceSnapshot> {
    let (marker, step): (&str, i32) = ["前条", "次条", "本条"]
        .into_iter()
        .find(|marker| rest.starts_with(marker))
        .map(|marker| match marker {
            "前条" => (marker, -1),
            "次条" => (marker, 1),
            _ => (marker, 0),
        })?;

    // The containing article is the last one at or before this paragraph.
    let current = articles
        .iter()
        .rposition(|article| article.paragraph <= paragraph);
    let target = current
        .and_then(|position| {
            let target = position as i32 + step;
            (target >= 0).then_some(target as usize)
        })
        .and_then(|position| articles.get(position));

    Some(HeadlessReferenceSnapshot {
        paragraph: 0,
        start_offset: start,
        end_offset: start + marker.len(),
        text: marker.to_string(),
        kind: "relative".to_string(),
        target_number_key: target.map(|article| article.key.clone()),
        target_paragraph: target.map(|article| article.paragraph),
        target_para_id: target.and_then(|article| article.para_id.clone()),
        target_number: target.map(|article| article.label.clone()),
        target_title: target.and_then(|article| article.title.clone()),
        target_end_paragraph: None,
        target_end_number: None,
        sub_references: Vec::new(),
        resolved: target.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(text: &str) -> ParagraphInput {
        ParagraphInput {
            text: text.to_string(),
            numbering_text: None,
            para_id: None,
            in_table: false,
        }
    }

    fn inputs(texts: &[&str]) -> Vec<ParagraphInput> {
        texts.iter().map(|text| input(text)).collect()
    }

    fn default_options() -> ChunkOptions {
        ChunkOptions {
            disable_table_splitting: false,
            enable_branch_splitting: false,
            include_text: true,
        }
    }

    #[test]
    fn japanese_numbers_read_in_every_form_drafts_use() {
        for (text, expected) in [
            ("12", 12),
            ("１２", 12),
            ("十二", 12),
            ("一二", 12),
            ("二十三", 23),
            ("十", 10),
            ("百二十", 120),
            ("一〇", 10),
        ] {
            assert_eq!(japanese_number_value(text), Some(expected), "{text}");
        }
        assert_eq!(japanese_number_value("あ"), None);
        assert_eq!(japanese_number_value(""), None);
    }

    #[test]
    fn article_numbers_normalise_to_a_comparable_key() {
        let number = parse_article_number("第２条の２　（例外）").expect("article number");
        assert_eq!(number.label, "第２条の２");
        assert_eq!(number.key, "2-2");
        assert_eq!(parse_article_number("第十二条").unwrap().key, "12");
        // Prose quoting an article is not an article number heading.
        assert!(article_heading_number("第3条の規定にかかわらず").is_none());
    }

    /// 見出しが括弧書きで前行に置かれている、もっとも普通の契約書の形。
    #[test]
    fn title_on_the_line_above_belongs_to_the_article_below() {
        let paragraphs = inputs(&["（秘密保持）", "第１条　ああああ", "２　いいいい"]);
        let chunks = build_chunks(&paragraphs, &default_options());

        assert_eq!(chunks.len(), 1, "{chunks:#?}");
        let article = &chunks[0];
        assert_eq!(article.kind, "article");
        assert_eq!(article.number.as_deref(), Some("第１条"));
        assert_eq!(article.title.as_deref(), Some("秘密保持"));
        assert_eq!(article.title_source.as_deref(), Some("preceding_line"));
        assert_eq!(article.title_paragraph, Some(0));
        // The title line opens the chunk rather than closing the one above it.
        assert_eq!(article.start_paragraph, 0);
        assert_eq!(article.end_paragraph, 2);
    }

    /// 見出しが条番号と同じ行に書かれている形。
    #[test]
    fn title_written_inline_is_read_as_the_title() {
        let paragraphs = inputs(&["第１条　秘密保持", "２　いいいい"]);
        let chunks = build_chunks(&paragraphs, &default_options());

        assert_eq!(chunks[0].title.as_deref(), Some("秘密保持"));
        assert_eq!(chunks[0].title_source.as_deref(), Some("inline"));
    }

    /// 条文の本文を見出しと誤認しないこと。
    #[test]
    fn a_sentence_after_the_article_number_is_not_a_title() {
        let paragraphs = inputs(&["第１条　乙は、甲に対し、本件業務を委託する。"]);
        let chunks = build_chunks(&paragraphs, &default_options());

        assert_eq!(chunks[0].number.as_deref(), Some("第１条"));
        assert_eq!(chunks[0].title, None, "body text must not become a title");
    }

    #[test]
    fn chunks_split_at_articles_and_keep_the_preamble_and_sections() {
        let paragraphs = inputs(&[
            "業務委託契約書",
            "甲と乙は、以下のとおり契約する。",
            "第１章　総則",
            "（目的）",
            "第１条　本契約は…",
            "（秘密保持）",
            "第２条　乙は…",
        ]);
        let chunks = build_chunks(&paragraphs, &default_options());

        let shape: Vec<(&str, Option<&str>, usize, usize)> = chunks
            .iter()
            .map(|chunk| {
                (
                    chunk.kind.as_str(),
                    chunk.title.as_deref(),
                    chunk.start_paragraph,
                    chunk.end_paragraph,
                )
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                ("preamble", None, 0, 1),
                ("section", None, 2, 2),
                ("article", Some("目的"), 3, 4),
                ("article", Some("秘密保持"), 5, 6),
            ],
        );
        assert_eq!(chunks[2].text, "（目的）\n第１条　本契約は…");
        assert_eq!(chunks[0].index, 0);
        assert_eq!(chunks[3].index, 3);
    }

    #[test]
    fn branch_articles_stay_with_their_parent_unless_asked_to_split() {
        let paragraphs = inputs(&["第２条　本文", "第２条の２　例外", "第３条　次条"]);

        let merged = build_chunks(&paragraphs, &default_options());
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].end_paragraph, 1, "第2条の2 stays with 第2条");

        let split = build_chunks(
            &paragraphs,
            &ChunkOptions {
                enable_branch_splitting: true,
                ..default_options()
            },
        );
        assert_eq!(split.len(), 3);
        assert_eq!(split[1].number_key.as_deref(), Some("2-2"));
    }

    #[test]
    fn tables_form_their_own_chunk_unless_splitting_is_disabled() {
        let mut paragraphs = inputs(&["第１条　本文", "表の中", "第２条　次"]);
        paragraphs[1].in_table = true;

        let split = build_chunks(&paragraphs, &default_options());
        assert_eq!(
            split.iter().map(|c| c.kind.as_str()).collect::<Vec<_>>(),
            vec!["article", "table", "article"],
        );

        let merged = build_chunks(
            &paragraphs,
            &ChunkOptions {
                disable_table_splitting: true,
                ..default_options()
            },
        );
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].end_paragraph, 1, "the table stays in 第1条");
    }

    #[test]
    fn include_text_false_keeps_the_count_but_drops_the_body() {
        let paragraphs = inputs(&["第１条　本文"]);
        let chunks = build_chunks(
            &paragraphs,
            &ChunkOptions {
                include_text: false,
                ..default_options()
            },
        );
        assert_eq!(chunks[0].text, "");
        assert_eq!(chunks[0].char_count, "第１条　本文".chars().count());
    }

    #[test]
    fn references_resolve_to_the_article_they_name() {
        let paragraphs = inputs(&[
            "（目的）",
            "第１条　本契約は…",
            "（秘密保持）",
            "第２条　前条の目的のため、第５条第２項に定める…",
            "（存続）",
            "第５条　…",
        ]);
        let references = find_references(&paragraphs);

        let by_text: Vec<(&str, Option<usize>, bool)> = references
            .iter()
            .map(|reference| {
                (
                    reference.text.as_str(),
                    reference.target_paragraph,
                    reference.resolved,
                )
            })
            .collect();
        assert_eq!(
            by_text,
            vec![("前条", Some(1), true), ("第５条第２項", Some(5), true)],
        );

        let cited = &references[1];
        assert_eq!(cited.sub_references, vec!["第２項".to_string()]);
        assert_eq!(cited.target_title.as_deref(), Some("存続"));
        assert_eq!(cited.target_number.as_deref(), Some("第５条"));
        // The offsets index the citing paragraph's own text.
        assert_eq!(
            &paragraphs[3].text[cited.start_offset..cited.end_offset],
            "第５条第２項",
        );
    }

    #[test]
    fn a_reference_to_a_missing_article_is_reported_unresolved() {
        let paragraphs = inputs(&["第１条　第９条に定める事項", "第２条　…"]);
        let references = find_references(&paragraphs);

        assert_eq!(references.len(), 1);
        assert_eq!(references[0].text, "第９条");
        assert!(!references[0].resolved);
        assert_eq!(references[0].target_paragraph, None);
        assert_eq!(references[0].target_number_key.as_deref(), Some("9"));
    }

    #[test]
    fn range_references_report_both_ends() {
        let paragraphs = inputs(&[
            "第１条　第２条から第３条までの規定は…",
            "第２条　…",
            "第３条　…",
        ]);
        let references = find_references(&paragraphs);

        assert_eq!(references[0].text, "第２条から第３条まで");
        assert_eq!(references[0].target_paragraph, Some(1));
        assert_eq!(references[0].target_end_paragraph, Some(2));
        assert!(references[0].resolved);
    }

    /// 条見出しそのものは参照ではない。
    #[test]
    fn an_article_heading_is_not_reported_as_a_reference_to_itself() {
        let paragraphs = inputs(&["第１条　秘密保持", "第２条　本条の定めにかかわらず"]);
        let references = find_references(&paragraphs);

        assert_eq!(references.len(), 1);
        assert_eq!(references[0].text, "本条");
        assert_eq!(references[0].target_paragraph, Some(1), "本条 is 第２条");
    }
}
