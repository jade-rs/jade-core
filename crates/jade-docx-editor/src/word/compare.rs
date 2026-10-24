//! 校閲 › 比較: 文書の比較 — two versions of a document made into a third,
//! the revised one with every difference from the original marked as a
//! tracked change put down to the name given — with Word's 比較の設定
//! (書式, 大文字/小文字の変更, 空白文字, コメント, and 変更の表示単位 by
//! character or by word), and 組み込み, which brings a reviewer's copy's
//! changes into the original under that reviewer's name.
//!
//! The diff is the engine's structural one (`jade-docx-diff`: paragraphs,
//! then tables and rows, then characters) and the redline its merge
//! (`jade-docx-merge`); the settings are applied to the diff in between, and
//! 書式 marks runs formatted differently as formatting changes.

use crate::word_document::WordDocument;
use jade_docx_diff::{BlockChange, ChangeKind, DiffOptions, Limits, Segment, SegmentKind};

/// 比較の設定.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompareOptions {
    /// 変更の表示名: who the differences are put down to.
    pub author: String,
    /// 書式: runs formatted otherwise are marked as formatting changes.
    pub formatting: bool,
    /// 大文字/小文字の変更: off, `Word` and `word` are the same.
    pub case_changes: bool,
    /// 空白文字: off, a difference only in spaces is none.
    pub white_space: bool,
    /// コメント: off, the comments are left out of the result.
    pub comments: bool,
    /// 変更の表示単位: 単語 (`true`) or 文字.
    pub by_word: bool,
}

impl CompareOptions {
    /// Word's defaults: everything compared, changes shown by word.
    pub fn new(author: &str) -> Self {
        Self {
            author: author.to_string(),
            formatting: true,
            case_changes: true,
            white_space: true,
            comments: true,
            by_word: true,
        }
    }
}

/// What a comparison found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompareSummary {
    pub insertions: usize,
    pub deletions: usize,
    pub formatting: usize,
}

/// A word, a space run, or one other character: the units changes are
/// shown in by word. Japanese has no spaces, so each of its characters is
/// a unit of its own, as Word treats it.
fn tokens(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start = 0usize;
    let mut kind: Option<u8> = None;
    for (at, ch) in text.char_indices() {
        let this = if ch.is_whitespace() {
            1
        } else if ch.is_ascii_alphanumeric() || (ch.is_alphabetic() && (ch as u32) < 0x2E80) {
            2
        } else {
            3
        };
        match kind {
            Some(previous) if previous == this && this != 3 => {}
            Some(_) => {
                tokens.push(&text[start..at]);
                start = at;
            }
            None => start = at,
        }
        kind = Some(this);
    }
    if start < text.len() {
        tokens.push(&text[start..]);
    }
    tokens
}

/// The old text and the new one a run of segments spells.
fn sides(segments: &[Segment]) -> (String, String) {
    let (mut old, mut new) = (String::new(), String::new());
    for segment in segments {
        match segment.kind {
            SegmentKind::Equal => {
                old.push_str(&segment.text);
                new.push_str(&segment.text);
            }
            SegmentKind::Delete => old.push_str(&segment.text),
            SegmentKind::Insert => new.push_str(&segment.text),
        }
    }
    (old, new)
}

fn push(segments: &mut Vec<Segment>, kind: SegmentKind, text: &str) {
    if text.is_empty() {
        return;
    }
    match segments.last_mut() {
        Some(last) if last.kind == kind => last.text.push_str(text),
        _ => segments.push(Segment {
            kind,
            text: text.to_string(),
        }),
    }
}

/// The diff of `old` and `new` by unit, `same` deciding which units match.
fn diff_units(old: &[&str], new: &[&str], same: impl Fn(&str, &str) -> bool) -> Vec<Segment> {
    let (n, m) = (old.len(), new.len());
    // Too long to compare unit by unit: all of one, all of the other.
    if n.saturating_mul(m) > 4_000_000 {
        let mut segments = Vec::new();
        push(&mut segments, SegmentKind::Delete, &old.concat());
        push(&mut segments, SegmentKind::Insert, &new.concat());
        return segments;
    }
    // Longest common subsequence, from the ends.
    let mut table = vec![0u32; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[at(i, j)] = if same(old[i], new[j]) {
                table[at(i + 1, j + 1)] + 1
            } else {
                table[at(i + 1, j)].max(table[at(i, j + 1)])
            };
        }
    }
    let mut segments = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if same(old[i], new[j]) {
            push(&mut segments, SegmentKind::Equal, new[j]);
            i += 1;
            j += 1;
        } else if table[at(i + 1, j)] >= table[at(i, j + 1)] {
            push(&mut segments, SegmentKind::Delete, old[i]);
            i += 1;
        } else {
            push(&mut segments, SegmentKind::Insert, new[j]);
            j += 1;
        }
    }
    for unit in &old[i..] {
        push(&mut segments, SegmentKind::Delete, unit);
    }
    for unit in &new[j..] {
        push(&mut segments, SegmentKind::Insert, unit);
    }
    segments
}

/// A changed paragraph's segments, as the settings show them.
fn resegment(segments: &[Segment], options: &CompareOptions) -> Vec<Segment> {
    let (old, new) = sides(segments);
    let fold = |text: &str| -> String {
        let text = if options.case_changes {
            text.to_string()
        } else {
            text.to_lowercase()
        };
        if options.white_space {
            text
        } else {
            text.split_whitespace().collect::<Vec<_>>().join(" ")
        }
    };
    if options.by_word {
        let (old_units, new_units) = (tokens(&old), tokens(&new));
        diff_units(&old_units, &new_units, |a, b| fold(a) == fold(b))
    } else {
        let old_units: Vec<&str> = old
            .char_indices()
            .map(|(at, ch)| &old[at..at + ch.len_utf8()])
            .collect();
        let new_units: Vec<&str> = new
            .char_indices()
            .map(|(at, ch)| &new[at..at + ch.len_utf8()])
            .collect();
        diff_units(&old_units, &new_units, |a, b| fold(a) == fold(b))
    }
}

/// The settings applied to every changed paragraph, in tables too; one
/// left with no change is unchanged, so the revised one's own formatting
/// stands.
fn apply_settings(blocks: &mut [BlockChange], options: &CompareOptions) {
    for block in blocks {
        match block {
            BlockChange::Paragraph(change) => {
                if change.kind != ChangeKind::Changed {
                    continue;
                }
                change.segments = resegment(&change.segments, options);
                if change
                    .segments
                    .iter()
                    .all(|segment| segment.kind == SegmentKind::Equal)
                {
                    change.kind = ChangeKind::Unchanged;
                }
            }
            BlockChange::Table(table) => {
                for row in &mut table.rows {
                    for cell in &mut row.cells {
                        apply_settings(&mut cell.blocks, options);
                    }
                }
            }
        }
    }
}

/// Blocks the engine's merge would mark: for the summary.
fn count(blocks: &[BlockChange], summary: &mut CompareSummary) {
    for block in blocks {
        match block {
            BlockChange::Paragraph(change) => match change.kind {
                ChangeKind::Added => summary.insertions += 1,
                ChangeKind::Removed => summary.deletions += 1,
                ChangeKind::Changed => {
                    for segment in &change.segments {
                        match segment.kind {
                            SegmentKind::Insert => summary.insertions += 1,
                            SegmentKind::Delete => summary.deletions += 1,
                            SegmentKind::Equal => {}
                        }
                    }
                }
                _ => {}
            },
            BlockChange::Table(table) => {
                for row in &table.rows {
                    match row.kind {
                        ChangeKind::Added => summary.insertions += 1,
                        ChangeKind::Removed => summary.deletions += 1,
                        _ => {
                            for cell in &row.cells {
                                count(&cell.blocks, summary);
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Runs `future`, which never waits: the document loader is `async` for
/// the browser's sake but reads everything it needs from the bytes.
fn ready<T>(future: impl std::future::Future<Output = T>) -> T {
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
    fn noop(_: *const ()) {}
    fn clone(_: *const ()) -> RawWaker {
        RawWaker::new(std::ptr::null(), &VTABLE)
    }
    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
    let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
    }
}

fn load(bytes: &[u8]) -> Result<WordDocument, String> {
    ready(WordDocument::load(bytes, &crate::Options::default())).map_err(|error| error.to_string())
}

/// The comments left out: their ranges, references and parts' text.
fn without_comments(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::{Read, Write};
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|error| error.to_string())?;
        let name = file.name().map_err(|err| err.to_string())?.into_owned();
        let mut content = Vec::new();
        file.read_to_end(&mut content)
            .map_err(|error| error.to_string())?;
        if name == "word/document.xml" {
            let xml = String::from_utf8_lossy(&content).into_owned();
            content = strip_comment_marks(&xml).into_bytes();
        }
        out.start_file(name, options)
            .map_err(|error| error.to_string())?;
        out.write_all(&content).map_err(|error| error.to_string())?;
    }
    Ok(out
        .finish()
        .map_err(|error| error.to_string())?
        .into_inner())
}

/// `document.xml` without its comment ranges and references.
fn strip_comment_marks(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = rest.find('>').map_or(rest.len(), |end| end + 1);
        let tag = &rest[..end];
        let name = tag
            .trim_start_matches(['<', '/'])
            .split([' ', '/', '>'])
            .next()
            .unwrap_or_default();
        let comment = matches!(
            name,
            "w:commentRangeStart" | "w:commentRangeEnd" | "w:commentReference"
        );
        if !comment {
            out.push_str(tag);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// 文書の比較: `revised` with every difference from `original` tracked as
/// `options.author`'s change at `date` (ISO 8601). Returns the document and
/// what was found.
pub fn compare_documents(
    original: &[u8],
    revised: &[u8],
    options: &CompareOptions,
    date: &str,
) -> Result<(Vec<u8>, CompareSummary), String> {
    let limits = Limits::default();
    let old = jade_docx_diff::load_document_bytes(original, &limits)
        .map_err(|error| error.to_string())?;
    let new =
        jade_docx_diff::load_document_bytes(revised, &limits).map_err(|error| error.to_string())?;
    let mut diff =
        jade_docx_diff::diff_documents("original", "revised", &old, &new, &DiffOptions::default());
    apply_settings(&mut diff.blocks, options);
    let mut summary = CompareSummary::default();
    count(&diff.blocks, &mut summary);
    let mut merged = docx_merge::merge_labelled(original, revised, &diff, &options.author, date)
        .map_err(|error| error.to_string())?;
    if options.formatting {
        let before = load(original)?;
        let mut after = load(&merged)?;
        summary.formatting = after.mark_formatting_differences(&before, &options.author, date);
        if summary.formatting > 0 {
            merged = after.export_docx().map_err(|error| error.to_string())?;
        }
    }
    if !options.comments {
        merged = without_comments(&merged)?;
    }
    Ok((merged, summary))
}

/// The reviewers a document's tracked changes are put down to, else who
/// last saved it: whose name 組み込み gives the reviewer's changes.
pub fn reviewer_of(document: &[u8]) -> Option<String> {
    let document = load(document).ok()?;
    for index in 0..document.paragraph_count() {
        let paragraph = document.paragraph(index)?;
        if let Some(author) = paragraph
            .runs
            .iter()
            .find_map(|run| run.revision.as_ref().and_then(|mark| mark.author.clone()))
        {
            return Some(author);
        }
    }
    let properties = super::query(
        &document,
        jade_docx_headless_api::HeadlessQuery::DocumentProperties,
    );
    match properties {
        Some(jade_docx_headless_api::HeadlessQueryResult::DocumentProperties { properties }) => {
            properties
                .get("lastModifiedBy")
                .or_else(|| properties.get("creator"))
                .cloned()
                .filter(|name| !name.is_empty())
        }
        _ => None,
    }
}

/// 組み込み: the reviewer's copy compared with the original, its
/// differences put down to the reviewer — the one its own tracked changes
/// name, else who last saved it — as Word combines one reviewer's copy into
/// the original.
pub fn combine_documents(
    original: &[u8],
    revised: &[u8],
    options: &CompareOptions,
    date: &str,
) -> Result<(Vec<u8>, CompareSummary), String> {
    let mut options = options.clone();
    if let Some(reviewer) = reviewer_of(revised) {
        options.author = reviewer;
    }
    compare_documents(original, revised, &options, date)
}

/// The 文書の比較 dialog's state, shared by both shells: the two
/// documents, the label and the settings, and whether it combines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompareForm {
    /// 元の文書: its name and bytes.
    pub original: Option<(String, Vec<u8>)>,
    /// 変更された文書.
    pub revised: Option<(String, Vec<u8>)>,
    pub options: CompareOptions,
    /// 組み込み rather than 比較.
    pub combine: bool,
}

impl CompareForm {
    pub fn new(author: &str, combine: bool) -> Self {
        Self {
            original: None,
            revised: None,
            options: CompareOptions::new(author),
            combine,
        }
    }

    /// OK: the result document and what it found, once both are chosen.
    pub fn run(&self, date: &str) -> Result<(Vec<u8>, CompareSummary), String> {
        let (Some((_, original)), Some((_, revised))) = (&self.original, &self.revised) else {
            return Err("choose both documents".to_string());
        };
        if self.combine {
            combine_documents(original, revised, &self.options, date)
        } else {
            compare_documents(original, revised, &self.options, date)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(kind: SegmentKind, text: &str) -> Segment {
        Segment {
            kind,
            text: text.to_string(),
        }
    }

    #[test]
    fn changes_are_shown_by_word_or_by_character() {
        let segments = [
            seg(SegmentKind::Equal, "the c"),
            seg(SegmentKind::Delete, "a"),
            seg(SegmentKind::Insert, "u"),
            seg(SegmentKind::Equal, "t sat"),
        ];
        let mut options = CompareOptions::new("Ann");
        let by_word = resegment(&segments, &options);
        let shown: Vec<(SegmentKind, &str)> = by_word
            .iter()
            .map(|segment| (segment.kind, segment.text.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                (SegmentKind::Equal, "the "),
                (SegmentKind::Delete, "cat"),
                (SegmentKind::Insert, "cut"),
                (SegmentKind::Equal, " sat"),
            ]
        );
        options.by_word = false;
        let by_character = resegment(&segments, &options);
        assert_eq!(by_character.len(), 4);
        assert_eq!(by_character[1].text, "a");
    }

    #[test]
    fn case_and_spacing_can_be_left_out() {
        let segments = [
            seg(SegmentKind::Delete, "The  Cat"),
            seg(SegmentKind::Insert, "the cat"),
        ];
        let mut options = CompareOptions::new("Ann");
        assert!(resegment(&segments, &options)
            .iter()
            .any(|segment| segment.kind != SegmentKind::Equal));
        options.case_changes = false;
        options.white_space = false;
        assert!(resegment(&segments, &options)
            .iter()
            .all(|segment| segment.kind == SegmentKind::Equal));
    }

    #[test]
    fn japanese_is_compared_a_character_at_a_time() {
        assert_eq!(tokens("契約 is fine"), ["契", "約", " ", "is", " ", "fine"]);
    }

    #[test]
    fn comment_marks_are_stripped() {
        let xml = r#"<w:p><w:commentRangeStart w:id="0"/><w:r><w:t>a</w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r></w:p>"#;
        assert_eq!(
            strip_comment_marks(xml),
            "<w:p><w:r><w:t>a</w:t></w:r><w:r></w:r></w:p>"
        );
    }
}
