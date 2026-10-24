//! 校閲 › アクセシビリティ チェック and 代替テキスト: what keeps a document
//! from reading well to someone using a screen reader or with low vision,
//! as Word's checker lists it — pictures and shapes without alternative
//! text, tables without a header row, heading levels skipped, link text that
//! says nothing about where it goes, text too faint against its background,
//! objects floating out of the reading order — each with where it is and,
//! where Word has one, its quick fix. And the 代替テキスト pane's edit: a
//! description, or 装飾用にする.

use crate::commands::snapshot_edit::{EditScope, SnapshotEditCommand};
pub use crate::word_document::AltText;
use crate::word_document::{FloatingObjectKind, FloatingObjectTarget, WordDocument};
use jade_docx_headless_api::{HeadlessCommand, HeadlessQuery, HeadlessQueryResult};

use crate::commands::format_image::ImageLayoutPreset;

use super::picture::{PlacedObject, WordPicture};
use super::{Helpers, Position, WordSession};
use crate::text_edit::TextEdit;

/// How much an issue matters, as Word's checker groups them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// エラー: content that is very hard or impossible to read.
    Error,
    /// 警告: content that is hard to read in most cases.
    Warning,
    /// ヒント: content that could be organised better.
    Tip,
}

/// What an issue is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IssueKind {
    /// 代替テキストがありません: a picture, chart or shape with no description
    /// that is not marked decorative.
    MissingAltText(FloatingObjectTarget),
    /// 表のヘッダーがありません: the table's first row is not a header row.
    NoHeaderRow { table_order_index: usize },
    /// 見出しのレベルが飛んでいます: a heading more than one level under the
    /// one before it.
    SkippedHeading { from: u8, to: u8 },
    /// 不明瞭なハイパーリンク テキスト: the link's text is its address, or
    /// "click here".
    UnclearLinkText,
    /// 読みにくいテキストのコントラスト: below WCAG's 4.5:1.
    LowContrast { ratio_tenths: u32 },
    /// 画像またはオブジェクトが行内にありません.
    NotInline(FloatingObjectTarget),
}

impl IssueKind {
    pub fn severity(&self) -> Severity {
        match self {
            IssueKind::MissingAltText(_) | IssueKind::NoHeaderRow { .. } => Severity::Error,
            IssueKind::UnclearLinkText
            | IssueKind::LowContrast { .. }
            | IssueKind::NotInline(_) => Severity::Warning,
            IssueKind::SkippedHeading { .. } => Severity::Tip,
        }
    }

    /// The i18n key of its name in the pane.
    pub fn label_key(&self) -> &'static str {
        match self {
            IssueKind::MissingAltText(_) => "a11y.missing_alt",
            IssueKind::NoHeaderRow { .. } => "a11y.no_header",
            IssueKind::SkippedHeading { .. } => "a11y.skipped_heading",
            IssueKind::UnclearLinkText => "a11y.unclear_link",
            IssueKind::LowContrast { .. } => "a11y.low_contrast",
            IssueKind::NotInline(_) => "a11y.not_inline",
        }
    }

    /// Whether the pane offers a one-click fix.
    pub fn has_quick_fix(&self) -> bool {
        matches!(
            self,
            IssueKind::MissingAltText(_) | IssueKind::NoHeaderRow { .. } | IssueKind::NotInline(_)
        )
    }
}

/// One issue the checker found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A11yIssue {
    pub kind: IssueKind,
    /// Where it is: the paragraph and the byte range in it.
    pub paragraph: usize,
    pub start: usize,
    pub end: usize,
    /// A few words of what is there, for the pane.
    pub excerpt: String,
}

impl A11yIssue {
    pub fn severity(&self) -> Severity {
        self.kind.severity()
    }

    pub fn start(&self) -> Position {
        (self.paragraph, self.start)
    }

    pub fn end(&self) -> Position {
        (self.paragraph, self.end)
    }
}

fn excerpt(text: &str) -> String {
    let trimmed: String = text.trim().chars().take(40).collect();
    if text.trim().chars().count() > 40 {
        format!("{trimmed}…")
    } else {
        trimmed
    }
}

/// Link text that says nothing of where the link goes.
pub fn is_unclear_link_text(text: &str, address: Option<&str>) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return true;
    }
    let lower = text.to_lowercase();
    let looks_like_address = lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("www.")
        || lower.starts_with("mailto:")
        || address.is_some_and(|address| {
            let address = address.trim_end_matches('/').to_lowercase();
            address == lower.trim_end_matches('/')
                || address
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    == lower.trim_end_matches('/')
        });
    const VAGUE: [&str; 12] = [
        "click here",
        "here",
        "link",
        "more",
        "read more",
        "this",
        "ここ",
        "こちら",
        "ここをクリック",
        "リンク",
        "詳細",
        "こちらをクリック",
    ];
    looks_like_address || VAGUE.contains(&lower.as_str())
}

/// `#RRGGBB` (or `RRGGBB`) as linear-light relative luminance (WCAG 2).
fn luminance(color: &str) -> Option<f64> {
    let hex = color.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let channel = |at: usize| -> Option<f64> {
        let value = f64::from(u8::from_str_radix(&hex[at..at + 2], 16).ok()?) / 255.0;
        Some(if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        })
    };
    Some(0.2126 * channel(0)? + 0.7152 * channel(2)? + 0.0722 * channel(4)?)
}

/// WCAG's contrast ratio between two colours.
pub fn contrast_ratio(foreground: &str, background: &str) -> Option<f64> {
    let (a, b) = (luminance(foreground)?, luminance(background)?);
    let (light, dark) = if a > b { (a, b) } else { (b, a) };
    Some((light + 0.05) / (dark + 0.05))
}

/// Word's highlight names as colours.
fn highlight_color(name: &str) -> Option<&'static str> {
    Some(match name {
        "yellow" => "#FFFF00",
        "green" => "#00FF00",
        "cyan" => "#00FFFF",
        "magenta" => "#FF00FF",
        "blue" => "#0000FF",
        "red" => "#FF0000",
        "darkBlue" => "#000080",
        "darkCyan" => "#008080",
        "darkGreen" => "#008000",
        "darkMagenta" => "#800080",
        "darkRed" => "#800000",
        "darkYellow" => "#808000",
        "darkGray" => "#808080",
        "lightGray" => "#C0C0C0",
        "black" => "#000000",
        "white" => "#FFFFFF",
        _ => return None,
    })
}

fn paragraph_text(document: &WordDocument, index: usize) -> Option<String> {
    let paragraph = document.paragraph(index)?;
    Some(paragraph.runs.iter().map(|run| run.text.as_str()).collect())
}

/// The accessibility checker and the 代替テキスト edit, for every session.
pub trait WordAccessibility: WordSession {
    /// The 代替テキスト of a picture or shape.
    fn alt_text(&self, target: FloatingObjectTarget) -> Option<AltText> {
        self.word_document().object_alt_text(target)
    }

    /// Sets a picture's or shape's 代替テキスト, as one undo step.
    fn set_alt_text(&mut self, target: FloatingObjectTarget, alt: AltText) -> Result<(), String> {
        let paragraph = target.paragraph_index;
        self.execute_command(
            Box::new(SnapshotEditCommand::with_scope(
                "set_alt_text",
                EditScope::Paragraphs(Some((paragraph, paragraph))),
                move |document| document.set_object_alt_text(target, &alt),
            )),
            true,
        )
    }

    /// Every issue in the document, errors first, then in reading order.
    fn accessibility_issues(&self) -> Vec<A11yIssue> {
        let document = self.word_document();
        let mut issues = Vec::new();
        let page = document
            .document
            .as_ref()
            .and_then(|document| document.background_color.clone())
            .unwrap_or_else(|| "#FFFFFF".to_string());
        let styles = document.styles_part.as_ref();
        let mut previous_heading: Option<u8> = None;
        let headings: std::collections::HashMap<usize, u8> = document
            .navigation_headings()
            .into_iter()
            .map(|heading| (heading.paragraph_index, heading.level))
            .collect();
        for index in 0..document.paragraph_count() {
            let Some(paragraph) = document.paragraph(index) else {
                continue;
            };
            let text = paragraph_text(document, index).unwrap_or_default();
            if let Some(&level) = headings.get(&index) {
                if let Some(from) = previous_heading.filter(|&from| level > from + 1) {
                    issues.push(A11yIssue {
                        kind: IssueKind::SkippedHeading { from, to: level },
                        paragraph: index,
                        start: 0,
                        end: text.len(),
                        excerpt: excerpt(&text),
                    });
                }
                previous_heading = Some(level);
            }
            let shading = paragraph
                .properties
                .as_ref()
                .and_then(|properties| properties.shading.as_ref())
                .and_then(|shading| shading.resolved_background_color());
            let mut offset = 0usize;
            let mut link: Option<(usize, usize, Option<String>)> = None;
            let mut contrast_reported = false;
            for (run_index, run) in paragraph.runs.iter().enumerate() {
                let (start, end) = (offset, offset + run.text.len());
                offset = end;
                let object = |kind| FloatingObjectTarget {
                    paragraph_index: index,
                    run_index,
                    kind,
                };
                let drawing = match (&run.image, &run.text_box) {
                    (Some(image), _) => Some((
                        object(FloatingObjectKind::Image),
                        image.layout.kind == crate::document::RunImageKind::Floating,
                    )),
                    (None, Some(text_box)) => Some((
                        object(FloatingObjectKind::TextBox),
                        text_box.layout.kind == crate::document::RunImageKind::Floating,
                    )),
                    _ => None,
                };
                if let Some((target, floating)) = drawing {
                    let alt = document.object_alt_text(target).unwrap_or_default();
                    let image_without_text = run.image.is_some()
                        || run
                            .text_box
                            .as_ref()
                            .is_some_and(|text_box| text_box.text.trim().is_empty());
                    if image_without_text && !alt.decorative && alt.description.is_none() {
                        issues.push(A11yIssue {
                            kind: IssueKind::MissingAltText(target),
                            paragraph: index,
                            start,
                            end,
                            excerpt: run
                                .image
                                .as_ref()
                                .and_then(|image| image.title.clone())
                                .unwrap_or_default(),
                        });
                    }
                    if floating && !alt.decorative {
                        issues.push(A11yIssue {
                            kind: IssueKind::NotInline(target),
                            paragraph: index,
                            start,
                            end,
                            excerpt: String::new(),
                        });
                    }
                }
                // A link is the run of runs that share its address.
                let address = run.hyperlink.as_ref().map(|hyperlink| {
                    hyperlink
                        .target
                        .clone()
                        .or_else(|| hyperlink.anchor.clone())
                        .unwrap_or_default()
                });
                match (&mut link, address) {
                    (Some((_, link_end, Some(current))), Some(address))
                        if *current == address && *link_end == start =>
                    {
                        *link_end = end;
                    }
                    (_, address) => {
                        if let Some((from, to, address)) = link.take() {
                            push_link(&mut issues, index, &text, from, to, address.as_deref());
                        }
                        link = address.map(|address| (start, end, Some(address)));
                    }
                }
                // Contrast: the run's resolved colour against what is behind it.
                if contrast_reported || run.text.trim().is_empty() {
                    continue;
                }
                let style = paragraph
                    .properties
                    .as_ref()
                    .and_then(|properties| properties.style.as_deref());
                let resolved = styles
                    .and_then(|styles| {
                        styles.resolve_effective_run_properties(style, run.properties.as_ref())
                    })
                    .or_else(|| run.properties.clone())
                    .unwrap_or_default();
                let Some(color) = resolved
                    .color
                    .as_deref()
                    .filter(|color| !color.eq_ignore_ascii_case("auto"))
                    .map(str::to_string)
                else {
                    continue;
                };
                let behind = resolved
                    .highlight
                    .as_deref()
                    .and_then(highlight_color)
                    .map(str::to_string)
                    .or_else(|| {
                        resolved
                            .shading
                            .as_ref()
                            .and_then(|shading| shading.resolved_background_color())
                    })
                    .or_else(|| shading.clone())
                    .unwrap_or_else(|| page.clone());
                if let Some(ratio) = contrast_ratio(&color, &behind) {
                    if ratio < 4.5 {
                        contrast_reported = true;
                        issues.push(A11yIssue {
                            kind: IssueKind::LowContrast {
                                ratio_tenths: (ratio * 10.0).round() as u32,
                            },
                            paragraph: index,
                            start,
                            end,
                            excerpt: excerpt(&run.text),
                        });
                    }
                }
            }
            if let Some((from, to, address)) = link.take() {
                push_link(&mut issues, index, &text, from, to, address.as_deref());
            }
        }
        // Tables: their first row a header row.
        if let Some(HeadlessQueryResult::DocumentStats { stats }) =
            self.query(HeadlessQuery::DocumentStats)
        {
            for table_order_index in 0..stats.table_count {
                let Some(HeadlessQueryResult::Table {
                    value: Some(table), ..
                }) = self.query(HeadlessQuery::Table { table_order_index })
                else {
                    continue;
                };
                if table.rows.len() < 2 || table.rows.first().is_some_and(|row| row.is_header) {
                    continue;
                }
                let paragraph = self.table_first_paragraph(table_order_index).unwrap_or(0);
                let first_row: Vec<String> = table
                    .rows
                    .first()
                    .map(|row| row.cells.iter().map(|cell| cell.text.clone()).collect())
                    .unwrap_or_default();
                issues.push(A11yIssue {
                    kind: IssueKind::NoHeaderRow { table_order_index },
                    paragraph,
                    start: 0,
                    end: 0,
                    excerpt: excerpt(&first_row.join(" | ")),
                });
            }
        }
        issues.sort_by_key(|issue| (issue.severity(), issue.paragraph, issue.start));
        issues
    }

    /// The pane's おすすめのアクション: 説明を追加 opens the 代替テキスト
    /// pane instead; this marks a picture decorative, makes a table's first
    /// row its header, or puts an object in line with the text.
    fn quick_fix(&mut self, issue: &A11yIssue) -> Result<(), String> {
        match &issue.kind {
            IssueKind::MissingAltText(target) => self.set_alt_text(
                *target,
                AltText {
                    description: None,
                    decorative: true,
                },
            ),
            IssueKind::NoHeaderRow { table_order_index } => self.apply_command(
                HeadlessCommand::SetTableRowProperties {
                    table_order_index: *table_order_index,
                    row: 0,
                    repeat_header: Some(true),
                    cant_split: None,
                },
                true,
            ),
            IssueKind::NotInline(target) => {
                let object = PlacedObject {
                    target: *target,
                    page: 0,
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 0.0,
                };
                self.set_picture_wrap(object, ImageLayoutPreset::Inline)
            }
            _ => Err("no quick fix".to_string()),
        }
    }
}

fn push_link(
    issues: &mut Vec<A11yIssue>,
    paragraph: usize,
    text: &str,
    start: usize,
    end: usize,
    address: Option<&str>,
) {
    let shown = text.get(start..end).unwrap_or_default();
    if is_unclear_link_text(shown, address) {
        issues.push(A11yIssue {
            kind: IssueKind::UnclearLinkText,
            paragraph,
            start,
            end,
            excerpt: excerpt(shown),
        });
    }
}

impl<T: WordSession + ?Sized> WordAccessibility for T {}

/// The 代替テキスト pane: the description being typed and 装飾用にする.
pub struct AltTextForm {
    pub target: FloatingObjectTarget,
    pub description: TextEdit,
    pub decorative: bool,
}

impl AltTextForm {
    pub fn new(target: FloatingObjectTarget, alt: AltText) -> Self {
        Self {
            target,
            description: TextEdit::new(alt.description.unwrap_or_default()),
            decorative: alt.decorative,
        }
    }

    /// What OK writes.
    pub fn alt_text(&self) -> AltText {
        let description = self.description.text.trim();
        AltText {
            description: (!description.is_empty() && !self.decorative)
                .then(|| description.to_string()),
            decorative: self.decorative,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_follows_wcag() {
        let black_on_white = contrast_ratio("#000000", "#FFFFFF").unwrap();
        assert!((black_on_white - 21.0).abs() < 0.01);
        assert!(contrast_ratio("#777777", "#FFFFFF").unwrap() < 4.5);
        assert!(contrast_ratio("#595959", "#FFFFFF").unwrap() > 4.5);
        assert!(contrast_ratio("auto", "#FFFFFF").is_none());
    }

    #[test]
    fn vague_or_bare_link_text_is_unclear() {
        assert!(is_unclear_link_text("https://example.com", None));
        assert!(is_unclear_link_text(
            "example.com",
            Some("https://example.com/")
        ));
        assert!(is_unclear_link_text("Click here", None));
        assert!(is_unclear_link_text("こちら", None));
        assert!(!is_unclear_link_text(
            "the 2024 annual report",
            Some("https://example.com")
        ));
    }

    #[test]
    fn decorative_drawings_need_no_description() {
        let target = FloatingObjectTarget {
            paragraph_index: 0,
            run_index: 0,
            kind: FloatingObjectKind::Image,
        };
        let mut form = AltTextForm::new(
            target,
            AltText {
                description: Some("chart".into()),
                decorative: false,
            },
        );
        assert_eq!(form.alt_text().description.as_deref(), Some("chart"));
        form.decorative = true;
        assert_eq!(
            form.alt_text(),
            AltText {
                description: None,
                decorative: true
            }
        );
    }
}
