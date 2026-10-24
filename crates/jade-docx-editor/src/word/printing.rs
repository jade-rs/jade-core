//! Word's 印刷 settings and what they do to what is printed, for both
//! shells: which pages (all, the current one, the selection, or a range
//! such as `1,3,5-12` or `p1s2-p3s3`), whether the markup prints, 両面印刷
//! and 部単位, 1 枚あたりのページ数 and 用紙サイズに合わせる, and Word's
//! 印刷オプション — 印刷前にフィールドを更新する, 背景の色とイメージを印刷
//! する, 隠し文字を印刷する.
//!
//! The settings decide three things: the `.docx` that is laid out for
//! printing ([`print_docx`]: no background, hidden text shown, only the
//! selection), the pages of it that print ([`parse_ranges`]), and how
//! they sit on the sheets ([`impose`]). The shells turn that into pixels or
//! a PDF; natively, jade-office places the vector pages on the sheets.

use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
use jade_opc::OpcPackage;

use super::page::Paper;

/// 両面印刷.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Duplex {
    #[default]
    OneSided,
    /// 長辺を綴じる.
    LongEdge,
    /// 短辺を綴じる.
    ShortEdge,
}

/// Which pages print.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum PrintRange {
    #[default]
    All,
    /// 現在のページ.
    Current,
    /// 選択した部分.
    Selection,
    /// ユーザー指定の範囲: `1,3,5-12`, `p1s2-p3s3`, `s2`.
    Custom(String),
}

/// The 印刷 dialog's settings.
#[derive(Clone, Debug, PartialEq)]
pub struct PrintSettings {
    pub copies: u32,
    /// 部単位で印刷.
    pub collate: bool,
    pub duplex: Duplex,
    pub range: PrintRange,
    /// 変更履歴/コメントの印刷.
    pub markup: bool,
    /// 1 枚あたりのページ数: 1, 2, 4, 6, 8 or 16.
    pub pages_per_sheet: u32,
    /// 用紙サイズに合わせる: the paper the pages are scaled onto, or `None`
    /// for their own size.
    pub scale_to: Option<Paper>,
    /// 印刷前にフィールドを更新する.
    pub update_fields: bool,
    /// 背景の色とイメージを印刷する.
    pub background: bool,
    /// 隠し文字を印刷する.
    pub hidden_text: bool,
}

impl Default for PrintSettings {
    /// Word's: one copy, collated, one-sided, every page with its markup,
    /// one page a sheet at its own size, no background, no hidden text.
    fn default() -> Self {
        Self {
            copies: 1,
            collate: true,
            duplex: Duplex::OneSided,
            range: PrintRange::All,
            markup: true,
            pages_per_sheet: 1,
            scale_to: None,
            update_fields: false,
            background: false,
            hidden_text: false,
        }
    }
}

/// The numbers of pages per sheet Word offers.
pub const PAGES_PER_SHEET: [u32; 6] = [1, 2, 4, 6, 8, 16];

impl PrintSettings {
    /// Whether the document printed differs from the one on screen, so it
    /// has to be laid out again for printing.
    pub fn changes_document(&self, has_background: bool, has_hidden_text: bool) -> bool {
        (has_background && !self.background)
            || (has_hidden_text && self.hidden_text)
            || self.range == PrintRange::Selection
    }
}

// ---- page ranges -----------------------------------------------------------------

/// A page as a range names it: its section (from 1) and the number its
/// PAGE field shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageLabel {
    pub section: usize,
    pub number: usize,
}

/// Why a range could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RangeError {
    /// The text is not a range: the part that is not.
    Syntax(String),
    /// No page has that number: the part naming it.
    NoSuchPage(String),
}

impl std::fmt::Display for RangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RangeError::Syntax(part) => write!(f, "not a page range: {part}"),
            RangeError::NoSuchPage(part) => write!(f, "no such page: {part}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Term {
    /// A number on its own.
    Page(usize),
    /// `pNsM`, or `pN` (section 1).
    PageInSection(usize, usize),
    /// `sM`: the whole section.
    Section(usize),
}

fn parse_term(text: &str) -> Option<Term> {
    let text = text.trim().to_ascii_lowercase();
    if text.is_empty() {
        return None;
    }
    if let Some(rest) = text.strip_prefix('p') {
        let (page, section) = match rest.split_once('s') {
            Some((page, section)) => (page.trim().parse().ok()?, section.trim().parse().ok()?),
            None => (rest.trim().parse().ok()?, 1),
        };
        return Some(Term::PageInSection(page, section));
    }
    if let Some(rest) = text.strip_prefix('s') {
        return rest.trim().parse().ok().map(Term::Section);
    }
    text.parse().ok().map(Term::Page)
}

/// The pages `text` names, as indices into `labels` (one per page, in
/// order), in the order named. As in Word: `3` is the page numbered 3 (the
/// first one, when sections restart their numbering), `p2s3` is the page
/// numbered 2 in section 3, `s2` is all of section 2, and a range runs from
/// its first page to its last — `p1s2-p3s3`, `5-12`, `s1-s2`.
pub fn parse_ranges(text: &str, labels: &[PageLabel]) -> Result<Vec<usize>, RangeError> {
    let find_first = |term: Term, from: usize| -> Option<usize> {
        match term {
            Term::Page(number) => labels
                .iter()
                .skip(from)
                .position(|label| label.number == number)
                .map(|index| index + from)
                .or_else(|| (number >= 1 && number <= labels.len()).then(|| number - 1)),
            Term::PageInSection(number, section) => labels
                .iter()
                .position(|label| label.section == section && label.number == number),
            Term::Section(section) => labels.iter().position(|label| label.section == section),
        }
    };
    let find_last = |term: Term, from: usize| -> Option<usize> {
        match term {
            Term::Section(section) => labels.iter().rposition(|label| label.section == section),
            other => find_first(other, from),
        }
    };
    let mut out = Vec::new();
    for part in text.split([',', '、', ';', '，']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (start, end) = match part.split_once(['-', '–', '〜', '~']) {
            Some((start, end)) => {
                let start =
                    parse_term(start).ok_or_else(|| RangeError::Syntax(part.to_string()))?;
                let end = parse_term(end).ok_or_else(|| RangeError::Syntax(part.to_string()))?;
                (start, end)
            }
            None => {
                let term = parse_term(part).ok_or_else(|| RangeError::Syntax(part.to_string()))?;
                (term, term)
            }
        };
        let first = find_first(start, 0).ok_or_else(|| RangeError::NoSuchPage(part.to_string()))?;
        let last = find_last(end, first).ok_or_else(|| RangeError::NoSuchPage(part.to_string()))?;
        if last >= first {
            out.extend(first..=last);
        } else {
            out.extend((last..=first).rev());
        }
    }
    if out.is_empty() {
        return Err(RangeError::Syntax(text.to_string()));
    }
    Ok(out)
}

/// The pages `settings` print, out of `labels.len()`: `current` is the page
/// the caret is on.
pub fn pages_to_print(
    settings: &PrintSettings,
    labels: &[PageLabel],
    current: usize,
) -> Result<Vec<usize>, RangeError> {
    match &settings.range {
        // The selection is laid out on its own: all of what that makes.
        PrintRange::All | PrintRange::Selection => Ok((0..labels.len()).collect()),
        PrintRange::Current => Ok(vec![current.min(labels.len().saturating_sub(1))]),
        PrintRange::Custom(text) => parse_ranges(text, labels),
    }
}

// ---- sheets ------------------------------------------------------------------------

/// A page on a sheet: where its top-left corner goes and how much it is
/// scaled, in points from the sheet's top-left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub page: usize,
    pub x: f64,
    pub y: f64,
    pub scale: f64,
}

/// A sheet of paper and the pages on it.
#[derive(Clone, Debug, PartialEq)]
pub struct Sheet {
    pub width: f64,
    pub height: f64,
    pub placements: Vec<Placement>,
}

/// The column-and-row layouts `count` pages can sit in.
fn grids(count: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for columns in 1..=count {
        let rows = count.div_ceil(columns);
        if columns * rows == count || (columns * rows >= count && rows == 1) {
            out.push((columns, rows));
        }
    }
    out
}

/// Lays `pages` (indices into `sizes`, each page's size in points) out on
/// sheets: `per_sheet` to a sheet, in reading order, each scaled to fit its
/// cell; on `paper` when given, otherwise on the first page's paper. A
/// layout of several pages turns the sheet whichever way fits them larger,
/// as Word does for two pages to a sheet.
pub fn impose(
    pages: &[usize],
    sizes: &[(f64, f64)],
    per_sheet: u32,
    paper: Option<(f64, f64)>,
) -> Vec<Sheet> {
    let per_sheet = per_sheet.max(1) as usize;
    let mut sheets = Vec::new();
    for group in pages.chunks(per_sheet) {
        let largest = group.iter().fold((1.0f64, 1.0f64), |(w, h), page| {
            let (pw, ph) = sizes.get(*page).copied().unwrap_or((612.0, 792.0));
            (w.max(pw), h.max(ph))
        });
        let first = sizes.get(group[0]).copied().unwrap_or(largest);
        if per_sheet == 1 && paper.is_none() {
            sheets.push(Sheet {
                width: first.0,
                height: first.1,
                placements: vec![Placement {
                    page: group[0],
                    x: 0.0,
                    y: 0.0,
                    scale: 1.0,
                }],
            });
            continue;
        }
        let base = paper.unwrap_or(first);
        let (short, long) = (base.0.min(base.1), base.0.max(base.1));
        let mut best: Option<(f64, f64, f64, u32, u32)> = None;
        let orientations: Vec<(f64, f64)> = if per_sheet == 1 {
            // One page: the paper turned the way the page is.
            if first.0 > first.1 {
                vec![(long, short)]
            } else {
                vec![(short, long)]
            }
        } else {
            vec![(short, long), (long, short)]
        };
        for (sheet_width, sheet_height) in orientations {
            for (columns, rows) in grids(per_sheet as u32) {
                let cell = (sheet_width / columns as f64, sheet_height / rows as f64);
                let scale = (cell.0 / largest.0).min(cell.1 / largest.1);
                if best.is_none_or(|(_, _, best_scale, _, _)| scale > best_scale + 1e-9) {
                    best = Some((sheet_width, sheet_height, scale, columns, rows));
                }
            }
        }
        let (width, height, scale, columns, _rows) = best.expect("a grid fits");
        let cell = (
            width / columns as f64,
            height / ((per_sheet as u32).div_ceil(columns)) as f64,
        );
        let placements = group
            .iter()
            .enumerate()
            .map(|(slot, page)| {
                let (pw, ph) = sizes.get(*page).copied().unwrap_or(largest);
                let column = slot as u32 % columns;
                let row = slot as u32 / columns;
                Placement {
                    page: *page,
                    x: column as f64 * cell.0 + (cell.0 - pw * scale) / 2.0,
                    y: row as f64 * cell.1 + (cell.1 - ph * scale) / 2.0,
                    scale,
                }
            })
            .collect();
        sheets.push(Sheet {
            width,
            height,
            placements,
        });
    }
    sheets
}

/// The sheets `settings` print `pages` on, the pages' sizes in points.
pub fn sheets(settings: &PrintSettings, pages: &[usize], sizes: &[(f64, f64)]) -> Vec<Sheet> {
    let paper = settings.scale_to.map(|paper| {
        let (width, height) = paper.size();
        (width as f64, height as f64)
    });
    impose(pages, sizes, settings.pages_per_sheet, paper)
}

// ---- the document printed ---------------------------------------------------------

/// Removes every element named `tag` (`<w:vanish/>`, `<w:vanish w:val="1"/>`,
/// `<w:background …>…</w:background>`) from `xml`.
fn remove_elements(xml: &str, tag: &str) -> String {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        let after = &rest[start + open.len()..];
        // `<w:vanishX` is another element.
        if !after.starts_with(['/', '>', ' ', '\t', '\r', '\n']) {
            out.push_str(&rest[..start + open.len()]);
            rest = after;
            continue;
        }
        out.push_str(&rest[..start]);
        let Some(end) = after.find('>') else {
            rest = "";
            break;
        };
        let self_closing = after[..end].ends_with('/');
        rest = &after[end + 1..];
        if !self_closing {
            match rest.find(&close) {
                Some(close_at) => rest = &rest[close_at + close.len()..],
                None => rest = "",
            }
        }
    }
    out.push_str(rest);
    out
}

/// Whether the document has a page background (ページの色), which prints
/// only when 背景の色とイメージを印刷する is on.
pub fn has_background(docx: &[u8]) -> bool {
    OpcPackage::open(docx)
        .ok()
        .and_then(|package| package.part_str("word/document.xml").ok().flatten())
        .is_some_and(|xml| xml.contains("<w:background"))
}

/// Whether the document has hidden text, which prints only when 隠し文字を
/// 印刷する is on.
pub fn has_hidden_text(docx: &[u8]) -> bool {
    let Ok(package) = OpcPackage::open(docx) else {
        return false;
    };
    package.part_names().iter().any(|name| {
        name.starts_with("word/")
            && name.ends_with(".xml")
            && package
                .part_str(name)
                .ok()
                .flatten()
                .is_some_and(|xml| xml.contains("<w:vanish"))
    })
}

/// The `.docx` laid out for printing: without its page background unless
/// that prints, with its hidden text shown when that prints.
pub fn print_docx(docx: &[u8], settings: &PrintSettings) -> Result<Vec<u8>, String> {
    let mut package = OpcPackage::open(docx).map_err(|error| error.to_string())?;
    let mut changed = false;
    if !settings.background {
        if let Some(xml) = package
            .part_str("word/document.xml")
            .map_err(|e| e.to_string())?
        {
            let stripped = remove_elements(&xml, "w:background");
            if stripped != xml {
                package.replace("word/document.xml", stripped.into_bytes());
                changed = true;
            }
        }
    }
    if settings.hidden_text {
        let names: Vec<String> = package
            .part_names()
            .iter()
            .filter(|name| name.starts_with("word/") && name.ends_with(".xml"))
            .cloned()
            .collect();
        for name in names {
            if let Some(xml) = package.part_str(&name).map_err(|e| e.to_string())? {
                let shown = remove_elements(&xml, "w:vanish");
                if shown != xml {
                    package.replace(&name, shown.into_bytes());
                    changed = true;
                }
            }
        }
    }
    if !changed {
        return Ok(docx.to_vec());
    }
    package.write().map_err(|error| error.to_string())
}

/// The `.docx` of only the selection `start..end` (paragraph, byte
/// offset): everything after it and before it deleted, as Word lays out
/// 選択した部分 to print it.
pub fn selection_docx(
    docx: &[u8],
    start: (usize, usize),
    end: (usize, usize),
) -> Result<Vec<u8>, String> {
    use crate::word_document::WordDocument;
    let (start, end) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    let mut document =
        futures::executor::block_on(WordDocument::load(docx, &crate::Options::default()))
            .map_err(|error| error.to_string())?;
    let last = document.paragraph_count().saturating_sub(1);
    let last_length = document.paragraph_text_len_at(last).unwrap_or(0);
    let mut commands = Vec::new();
    if (end.0, end.1) < (last, last_length) {
        commands.push(HeadlessCommand::ReplaceRange {
            start_paragraph: end.0,
            start_offset: end.1,
            end_paragraph: last,
            end_offset: last_length,
            text: String::new(),
        });
    }
    if start > (0, 0) {
        commands.push(HeadlessCommand::ReplaceRange {
            start_paragraph: 0,
            start_offset: 0,
            end_paragraph: start.0,
            end_offset: start.1,
            text: String::new(),
        });
    }
    if commands.is_empty() {
        return Ok(docx.to_vec());
    }
    crate::headless_api::apply_headless_commands_to_document(
        &mut document,
        &HeadlessEditRequest { commands },
    )
    .map_err(|error| error.to_string())?;
    document.export_docx().map_err(|error| format!("{error:?}"))
}

/// 印刷前にフィールドを更新する: the table of contents brought up to date
/// (the fields the pages compute — PAGE, NUMPAGES, DATE — are computed as
/// they are drawn). One undo step; nothing when there is no table of
/// contents.
pub fn update_fields(session: &mut dyn super::WordSession) {
    session
        .apply_command(HeadlessCommand::UpdateTableOfContents, true)
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(sections: &[usize]) -> Vec<PageLabel> {
        // Each section numbered from 1, as when every section restarts.
        let mut out = Vec::new();
        for (index, count) in sections.iter().enumerate() {
            for number in 1..=*count {
                out.push(PageLabel {
                    section: index + 1,
                    number,
                });
            }
        }
        out
    }

    #[test]
    fn ranges_read_as_word_reads_them() {
        let continuous: Vec<PageLabel> = (1..=20)
            .map(|number| PageLabel { section: 1, number })
            .collect();
        assert_eq!(
            parse_ranges("1,3,5-7", &continuous).unwrap(),
            vec![0, 2, 4, 5, 6]
        );
        assert_eq!(parse_ranges(" 2 、 4 ", &continuous).unwrap(), vec![1, 3]);
        // Sections that restart: p and s say which.
        let restarting = labels(&[3, 4, 5]);
        assert_eq!(
            parse_ranges("p1s2-p3s3", &restarting).unwrap(),
            (3..=9).collect::<Vec<_>>()
        );
        assert_eq!(parse_ranges("s2", &restarting).unwrap(), vec![3, 4, 5, 6]);
        assert_eq!(
            parse_ranges("s1-s2", &restarting).unwrap(),
            (0..=6).collect::<Vec<_>>()
        );
        assert_eq!(parse_ranges("P2S3", &restarting).unwrap(), vec![8]);
        // A bare number is the first page numbered so.
        assert_eq!(parse_ranges("2", &restarting).unwrap(), vec![1]);
        assert_eq!(
            parse_ranges("p9s2", &restarting),
            Err(RangeError::NoSuchPage("p9s2".into()))
        );
        assert_eq!(
            parse_ranges("a-b", &restarting),
            Err(RangeError::Syntax("a-b".into()))
        );
        assert!(parse_ranges("", &restarting).is_err());
    }

    #[test]
    fn current_page_and_selection_choose_their_pages() {
        let labels = labels(&[5]);
        let settings = PrintSettings {
            range: PrintRange::Current,
            ..PrintSettings::default()
        };
        assert_eq!(pages_to_print(&settings, &labels, 3).unwrap(), vec![3]);
        assert_eq!(
            pages_to_print(&PrintSettings::default(), &labels, 0).unwrap(),
            vec![0, 1, 2, 3, 4]
        );
    }

    #[test]
    fn two_portrait_pages_sit_side_by_side_on_a_turned_sheet() {
        let a4 = (595.3, 841.9);
        let sheets = impose(&[0, 1, 2], &[a4; 3], 2, None);
        assert_eq!(sheets.len(), 2);
        let sheet = &sheets[0];
        assert!(
            sheet.width > sheet.height,
            "the sheet is turned to landscape"
        );
        assert_eq!(sheet.placements.len(), 2);
        assert!(sheet.placements[0].x < sheet.placements[1].x);
        assert!((sheet.placements[0].scale - 841.9 / 2.0 / 595.3).abs() < 0.01);
        assert_eq!(sheets[1].placements.len(), 1);
        // Four to a sheet: two by two, the sheet as it was.
        let four = impose(&[0, 1, 2, 3], &[a4; 4], 4, None);
        assert!(four[0].height > four[0].width);
        assert!(four[0].placements[2].y > four[0].placements[0].y);
    }

    #[test]
    fn scaling_to_paper_fits_and_centres_the_page() {
        let letter = (612.0, 792.0);
        let sheets = impose(&[0], &[letter], 1, Some((595.3, 841.9)));
        let placement = sheets[0].placements[0];
        assert!((placement.scale - 595.3 / 612.0).abs() < 1e-6);
        assert!(placement.y > 0.0, "centred down the taller sheet");
        // A landscape page goes on the paper turned.
        let sheets = impose(&[0], &[(792.0, 612.0)], 1, Some((595.3, 841.9)));
        assert!(sheets[0].width > sheets[0].height);
    }

    #[test]
    fn printing_strips_the_background_and_shows_hidden_text() {
        let xml = r#"<w:document><w:background w:color="FF0000"><v:background/></w:background><w:body><w:p><w:r><w:rPr><w:vanish/><w:vanishX/></w:rPr><w:t>x</w:t></w:r><w:r><w:rPr><w:vanish w:val="1"/></w:rPr></w:r></w:p></w:body></w:document>"#;
        let stripped = remove_elements(xml, "w:background");
        assert!(!stripped.contains("background"));
        let shown = remove_elements(xml, "w:vanish");
        assert!(!shown.contains("<w:vanish/>") && !shown.contains("<w:vanish w:val"));
        assert!(shown.contains("<w:vanishX/>"));
    }
}
