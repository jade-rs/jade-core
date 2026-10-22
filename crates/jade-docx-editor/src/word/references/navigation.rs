//! Finding the way around a document: the ナビゲーション pane's headings
//! (with their pages, collapsed and expanded, promoted, demoted and moved
//! with their sections), its search results, the headings collapsed in the
//! body, 次のコメント / 前のコメント, and ジャンプ (Go To) — a page, a
//! section, a line, a bookmark, a comment, a note, a heading, a table, a
//! graphic, a field or an equation.

use std::collections::BTreeSet;

use crate::document::{BodyElement, RunFieldKind};
use jade_docx_headless_api::HeadlessCommand;
use jade_selection::TextPosition;

use super::edit_document;
use super::notes::WordNotes;
use super::NoteKind;
use crate::word::find::{FindOptions, WordFind};
use crate::word::review::{Comment, WordReview};
use crate::word::{Position, WordFormatting, WordSession};

/// A heading as the navigation pane lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineHeading {
    /// Its paragraph.
    pub paragraph: usize,
    /// 1 for 見出し 1 … 9.
    pub level: u8,
    pub text: String,
    /// The page (1-based) it starts on, once laid out.
    pub page: Option<usize>,
    /// Whether headings below it belong to it, so it can be collapsed.
    pub has_children: bool,
}

/// A match the pane's 結果 tab lists: where, and the text around it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchResult {
    pub paragraph: usize,
    pub start: usize,
    pub end: usize,
    /// Some text before the match, the match, and some after.
    pub before: String,
    pub matched: String,
    pub after: String,
    pub page: Option<usize>,
}

/// ジャンプ's 移動先.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoToTarget {
    Page,
    Section,
    Line,
    Bookmark,
    Comment,
    Footnote,
    Endnote,
    Field,
    Table,
    Graphic,
    Equation,
    Heading,
}

impl GoToTarget {
    pub const ALL: [GoToTarget; 12] = [
        GoToTarget::Page,
        GoToTarget::Section,
        GoToTarget::Line,
        GoToTarget::Bookmark,
        GoToTarget::Comment,
        GoToTarget::Footnote,
        GoToTarget::Endnote,
        GoToTarget::Field,
        GoToTarget::Table,
        GoToTarget::Graphic,
        GoToTarget::Equation,
        GoToTarget::Heading,
    ];

    pub fn label_key(self) -> &'static str {
        match self {
            GoToTarget::Page => "goto.page",
            GoToTarget::Section => "goto.section",
            GoToTarget::Line => "goto.line",
            GoToTarget::Bookmark => "goto.bookmark",
            GoToTarget::Comment => "goto.comment",
            GoToTarget::Footnote => "goto.footnote",
            GoToTarget::Endnote => "goto.endnote",
            GoToTarget::Field => "goto.field",
            GoToTarget::Table => "goto.table",
            GoToTarget::Graphic => "goto.graphic",
            GoToTarget::Equation => "goto.equation",
            GoToTarget::Heading => "goto.heading",
        }
    }
}

/// Where ジャンプ goes: a place in the text, or a page or line the shell
/// finds on its own layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GoTo {
    Position(Position),
    /// A 1-based page.
    Page(usize),
    /// A 1-based line of the document.
    Line(usize),
}

/// What is typed into ジャンプ: a number, `+n` or `-n` from where the caret
/// is, or nothing for the next one. Bookmarks take a name instead.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Spec {
    Next,
    Absolute(usize),
    Relative(i64),
}

fn parse_spec(spec: &str) -> Result<Spec, String> {
    let spec: String = spec
        .trim()
        .chars()
        .map(|ch| match ch {
            '０'..='９' => char::from(b'0' + (ch as u32 - '０' as u32) as u8),
            '＋' => '+',
            '－' | 'ー' => '-',
            other => other,
        })
        .collect();
    if spec.is_empty() {
        return Ok(Spec::Next);
    }
    if let Some(rest) = spec.strip_prefix('+') {
        return rest
            .parse::<i64>()
            .map(Spec::Relative)
            .map_err(|_| "番号を入力してください".to_string());
    }
    if let Some(rest) = spec.strip_prefix('-') {
        return rest
            .parse::<i64>()
            .map(|value| Spec::Relative(-value))
            .map_err(|_| "番号を入力してください".to_string());
    }
    spec.parse::<usize>()
        .map(Spec::Absolute)
        .map_err(|_| "番号を入力してください".to_string())
}

/// Picks from places in document order by a spec, from `at`.
fn pick(places: &[Position], spec: &Spec, at: Position) -> Option<Position> {
    match spec {
        Spec::Next => places.iter().find(|place| **place > at).copied(),
        Spec::Absolute(number) => places.get(number.checked_sub(1)?).copied(),
        Spec::Relative(step) => {
            // From the one at or before the caret.
            let current = places.iter().rposition(|place| *place <= at);
            let index = match current {
                Some(current) => current as i64 + step,
                None if *step > 0 => step - 1,
                None => return None,
            };
            usize::try_from(index)
                .ok()
                .and_then(|index| places.get(index))
                .copied()
        }
    }
}

/// The navigation pane, ジャンプ and the moves between comments.
pub trait WordNavigation: WordSession {
    /// The headings the pane lists, in document order.
    fn outline(&self) -> Vec<OutlineHeading> {
        let headings = self.word_document().navigation_headings();
        headings
            .iter()
            .enumerate()
            .map(|(index, heading)| OutlineHeading {
                paragraph: heading.paragraph_index,
                level: heading.level,
                text: heading.text.trim().to_string(),
                page: self.paragraph_page(heading.paragraph_index),
                has_children: headings
                    .get(index + 1)
                    .is_some_and(|next| next.level > heading.level),
            })
            .collect()
    }

    /// The heading whose section the paragraph is in: the last one at or
    /// before it, which the pane highlights.
    fn current_heading(&self, paragraph: usize) -> Option<usize> {
        self.outline()
            .iter()
            .rposition(|heading| heading.paragraph <= paragraph)
    }

    /// A heading's section: from its paragraph to the paragraph before the
    /// next heading at its level or above, as top-level body elements.
    fn heading_section(&self, heading: usize) -> Option<(usize, usize)> {
        let outline = self.outline();
        let index = outline.iter().position(|item| item.paragraph == heading)?;
        let level = outline[index].level;
        let document = self.word_document();
        let start = document.body_element_of(heading)?;
        let end = match outline[index + 1..].iter().find(|item| item.level <= level) {
            Some(next) => document.body_element_of(next.paragraph)?.checked_sub(1)?,
            None => document.body_element_count().checked_sub(1)?,
        };
        Some((start, end.max(start)))
    }

    /// The top-level body elements the collapsed headings hide: everything
    /// in each one's section but its own paragraph.
    fn hidden_by_collapse(&self, collapsed: &BTreeSet<usize>) -> Vec<usize> {
        let mut hidden = BTreeSet::new();
        for &heading in collapsed {
            if let Some((start, end)) = self.heading_section(heading) {
                hidden.extend(start + 1..=end);
            }
        }
        hidden.into_iter().collect()
    }

    /// Whether a paragraph is under a collapsed heading.
    fn is_collapsed_away(&self, paragraph: usize, hidden_blocks: &[usize]) -> bool {
        self.word_document()
            .body_element_of(paragraph)
            .is_some_and(|element| hidden_blocks.binary_search(&element).is_ok())
    }

    /// レベル上げ / レベル下げ: a heading and the headings under it a level
    /// up or down, one undo step. 見出し 1 goes no higher, 見出し 9 no
    /// lower.
    fn shift_heading_level(&mut self, heading: usize, promote: bool) -> Result<(), String> {
        let outline = self.outline();
        let index = outline
            .iter()
            .position(|item| item.paragraph == heading)
            .ok_or_else(|| "見出しではありません".to_string())?;
        let level = outline[index].level;
        if (promote && level == 1) || (!promote && level == 9) {
            return Ok(());
        }
        let mut affected = vec![outline[index].clone()];
        affected.extend(
            outline[index + 1..]
                .iter()
                .take_while(|item| item.level > level)
                .cloned(),
        );
        let styles = self.paragraph_styles();
        let style_for = |level: u8| -> String {
            let names = [format!("heading{level}"), format!("見出し{level}")];
            styles
                .iter()
                .find(|style| {
                    let token: String = style
                        .name
                        .chars()
                        .filter(|ch| !ch.is_whitespace())
                        .flat_map(char::to_lowercase)
                        .collect();
                    names.contains(&token)
                })
                .map(|style| style.id.clone())
                .unwrap_or_else(|| format!("Heading{level}"))
        };
        let commands = affected
            .iter()
            .map(|item| {
                let level = if promote {
                    item.level.saturating_sub(1).max(1)
                } else {
                    (item.level + 1).min(9)
                };
                HeadlessCommand::SetParagraphStyle {
                    start_paragraph: item.paragraph,
                    end_paragraph: item.paragraph,
                    style_id: Some(style_for(level)),
                }
            })
            .collect();
        self.apply_commands(commands, true)
    }

    /// Moves a heading's section before or after its neighbour at the same
    /// level, one undo step. Returns where the heading went.
    fn move_heading_section(&mut self, heading: usize, up: bool) -> Result<Position, String> {
        let outline = self.outline();
        let index = outline
            .iter()
            .position(|item| item.paragraph == heading)
            .ok_or_else(|| "見出しではありません".to_string())?;
        let level = outline[index].level;
        let neighbour = if up {
            outline[..index]
                .iter()
                .rev()
                .take_while(|item| item.level >= level)
                .find(|item| item.level == level)
        } else {
            outline[index + 1..]
                .iter()
                .take_while(|item| item.level >= level)
                .find(|item| item.level == level)
        }
        .map(|item| item.paragraph)
        .ok_or_else(|| "これ以上移動できません".to_string())?;
        let (start, end) = self
            .heading_section(heading)
            .ok_or_else(|| "見出しの範囲がありません".to_string())?;
        let (neighbour_start, neighbour_end) = self
            .heading_section(neighbour)
            .ok_or_else(|| "見出しの範囲がありません".to_string())?;
        let before = if up {
            neighbour_start
        } else {
            neighbour_end + 1
        };
        self.move_section_to(start, end, before)
    }

    /// Moves top-level body elements `start..=end` before element `before`
    /// (the pane's drag and drop), one undo step. Returns where the first
    /// one's first paragraph went.
    fn move_section_to(
        &mut self,
        start: usize,
        end: usize,
        before: usize,
    ) -> Result<Position, String> {
        edit_document(self, "move_section", false, move |document, selection| {
            document.move_body_elements(start, end, before)?;
            let landed = if before > end {
                before - (end + 1 - start)
            } else {
                before
            };
            if let Some(surface) = document.body_element_surface(landed) {
                *selection = Some(jade_selection::SelectionRange::collapsed(
                    TextPosition::new(surface, 0),
                ));
            }
            Ok(())
        })?;
        Ok(self
            .selection_range()
            .map(|range| (range.focus.paragraph, range.focus.offset))
            .unwrap_or((0, 0)))
    }

    /// The pane's 結果: every match of `query`, with its context and page.
    fn search_results(&self, query: &str) -> Vec<SearchResult> {
        if query.is_empty() {
            return Vec::new();
        }
        self.find_all(query, FindOptions::default())
            .into_iter()
            .map(|found| {
                let text = self
                    .word_document()
                    .paragraph(found.paragraph)
                    .map(|paragraph| {
                        paragraph
                            .runs
                            .iter()
                            .map(|run| run.text.as_str())
                            .collect::<String>()
                    })
                    .unwrap_or_default();
                let start = found.start.min(text.len());
                let end = found.end.min(text.len()).max(start);
                let before: String = text[..start]
                    .chars()
                    .rev()
                    .take(16)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let after: String = text[end..].chars().take(24).collect();
                SearchResult {
                    paragraph: found.paragraph,
                    start: found.start,
                    end: found.end,
                    before,
                    matched: text[start..end].to_string(),
                    after,
                    page: self.paragraph_page(found.paragraph),
                }
            })
            .collect()
    }

    /// 次のコメント / 前のコメント: the comment after (or before) `from`.
    fn step_comment(&self, from: Position, forwards: bool) -> Option<Comment> {
        let comments: Vec<Comment> = self
            .comments()
            .into_iter()
            .filter(|comment| comment.anchor.is_some())
            .collect();
        if forwards {
            comments
                .into_iter()
                .find(|comment| comment.anchor.is_some_and(|(start, _)| start > from))
        } else {
            comments
                .into_iter()
                .rfind(|comment| comment.anchor.is_some_and(|(start, _)| start < from))
        }
    }

    /// The places of a ジャンプ target, in document order.
    fn go_to_places(&self, target: GoToTarget) -> Vec<Position> {
        let document = self.word_document();
        let body = |paragraph: usize| document.is_body_paragraph_surface(paragraph);
        match target {
            GoToTarget::Page | GoToTarget::Line | GoToTarget::Bookmark => Vec::new(),
            GoToTarget::Section => document
                .document
                .as_ref()
                .map(|model| {
                    model
                        .sections
                        .iter()
                        .filter_map(|section| document.body_element_surface(section.start))
                        .map(|surface| (surface, 0))
                        .collect()
                })
                .unwrap_or_default(),
            GoToTarget::Comment => self
                .comments()
                .into_iter()
                .filter_map(|comment| comment.anchor.map(|(start, _)| start))
                .collect(),
            GoToTarget::Footnote | GoToTarget::Endnote => {
                let kind = if target == GoToTarget::Footnote {
                    NoteKind::Footnote
                } else {
                    NoteKind::Endnote
                };
                self.note_marks()
                    .into_iter()
                    .filter(|mark| mark.kind == kind)
                    .map(|mark| mark.at)
                    .collect()
            }
            GoToTarget::Heading => self
                .outline()
                .into_iter()
                .map(|heading| (heading.paragraph, 0))
                .collect(),
            GoToTarget::Table => {
                let mut places = Vec::new();
                let mut last = None;
                for paragraph in 0..document.paragraph_count() {
                    if !body(paragraph) {
                        continue;
                    }
                    let element = document.body_element_of(paragraph);
                    let in_table = element.is_some_and(|element| {
                        document
                            .document
                            .as_ref()
                            .and_then(|model| model.body.as_ref())
                            .and_then(|body| body.elements.get(element))
                            .is_some_and(|element| matches!(element, BodyElement::Table(_)))
                    });
                    if in_table && element != last {
                        places.push((paragraph, 0));
                    }
                    last = if in_table { element } else { None };
                }
                places
            }
            GoToTarget::Graphic | GoToTarget::Field | GoToTarget::Equation => {
                let mut places = Vec::new();
                for paragraph in 0..document.paragraph_count() {
                    if !body(paragraph) {
                        continue;
                    }
                    let Some(model) = document.paragraph(paragraph) else {
                        continue;
                    };
                    let mut offset = 0;
                    for run in &model.runs {
                        let hit = match target {
                            GoToTarget::Graphic => run.image.is_some() || run.text_box.is_some(),
                            GoToTarget::Equation => run.math.is_some(),
                            _ => run.field.as_ref().is_some_and(|field| {
                                !matches!(field.kind, RunFieldKind::NonDisplay { .. })
                            }),
                        };
                        if hit {
                            let at = document.model_to_rendered_text_position(TextPosition::new(
                                paragraph, offset,
                            ));
                            places.push((at.paragraph, at.offset));
                        }
                        offset += run.text.len();
                    }
                }
                places.dedup();
                places
            }
        }
    }

    /// ジャンプ: where a target and what was typed take the caret from `at`.
    fn go_to(&self, target: GoToTarget, spec: &str, at: Position) -> Result<GoTo, String> {
        match target {
            GoToTarget::Bookmark => {
                let name = spec.trim();
                let position = self
                    .word_document()
                    .bookmark_position(name)
                    .ok_or_else(|| format!("ブックマーク「{name}」はありません"))?;
                Ok(GoTo::Position((position.paragraph, position.offset)))
            }
            GoToTarget::Page | GoToTarget::Line => {
                let current = match target {
                    GoToTarget::Page => self.paragraph_page(at.0).unwrap_or(1),
                    _ => 1,
                };
                let number = match parse_spec(spec)? {
                    Spec::Next => current + 1,
                    Spec::Absolute(number) => number.max(1),
                    Spec::Relative(step) => (current as i64 + step).max(1) as usize,
                };
                Ok(match target {
                    GoToTarget::Page => GoTo::Page(number),
                    _ => GoTo::Line(number),
                })
            }
            _ => {
                let spec = parse_spec(spec)?;
                let places = self.go_to_places(target);
                if places.is_empty() {
                    return Err("指定した項目は文書にありません".to_string());
                }
                pick(&places, &spec, at)
                    .map(GoTo::Position)
                    .ok_or_else(|| "指定した項目は文書にありません".to_string())
            }
        }
    }
}

impl<T: WordSession + ?Sized> WordNavigation for T {}

#[cfg(test)]
mod tests {
    use super::{parse_spec, pick, Spec};

    #[test]
    fn what_is_typed_picks_a_place() {
        assert_eq!(parse_spec("").unwrap(), Spec::Next);
        assert_eq!(parse_spec("３").unwrap(), Spec::Absolute(3));
        assert_eq!(parse_spec("+2").unwrap(), Spec::Relative(2));
        assert_eq!(parse_spec("-1").unwrap(), Spec::Relative(-1));
        assert!(parse_spec("abc").is_err());
        let places = [(1, 0), (4, 0), (9, 0)];
        assert_eq!(pick(&places, &Spec::Next, (4, 0)), Some((9, 0)));
        assert_eq!(pick(&places, &Spec::Absolute(2), (0, 0)), Some((4, 0)));
        assert_eq!(pick(&places, &Spec::Relative(1), (4, 2)), Some((9, 0)));
        assert_eq!(pick(&places, &Spec::Relative(-1), (4, 2)), Some((1, 0)));
        assert_eq!(pick(&places, &Spec::Relative(1), (0, 0)), Some((1, 0)));
        assert_eq!(pick(&places, &Spec::Absolute(9), (0, 0)), None);
    }
}
