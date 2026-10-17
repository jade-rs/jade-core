//! Authoring toolbar values use the same inheritance resolver as drawing,
//! before autofit's display scale; None identifies a mixed selection.
use crate::{style, Context, HeuristicMeasure, InheritedContent};
use jade_pptx_reader::{Deck, Shape, ShapeItem, TextRun};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextFormatState {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub size_pt: Option<f32>,
    pub font: Option<String>,
    pub east_asian_font: Option<String>,
    pub color: Option<String>,
}
impl TextFormatState {
    fn resolved(run: &style::ResolvedRun) -> Self {
        Self {
            bold: Some(run.style.bold),
            italic: Some(run.style.italic),
            underline: Some(run.underline),
            size_pt: Some(run.style.size_pt),
            font: run.style.font.clone(),
            east_asian_font: run.style.east_asian_font.clone(),
            color: Some(run.color.clone()),
        }
    }
    fn merge(&mut self, other: &Self) {
        if self.bold != other.bold {
            self.bold = None;
        }
        if self.italic != other.italic {
            self.italic = None;
        }
        if self.underline != other.underline {
            self.underline = None;
        }
        if self.font != other.font {
            self.font = None;
        }
        if self.east_asian_font != other.east_asian_font {
            self.east_asian_font = None;
        }
        if self.color != other.color {
            self.color = None;
        }
        if self.size_pt != other.size_pt {
            self.size_pt = None;
        }
    }
}
fn shapes<'a>(items: &'a [ShapeItem], id: u32, matches: &mut Vec<&'a Shape>) {
    for item in items {
        match item {
            ShapeItem::Shape(shape) if shape.id == Some(id) => matches.push(shape),
            ShapeItem::Group(group) => shapes(&group.children, id, matches),
            _ => {}
        }
    }
}

pub fn text_format_for_deck_shape(
    deck: &Deck,
    index: usize,
    id: u32,
    start: usize,
    end: usize,
) -> Option<TextFormatState> {
    if start > end {
        return None;
    }
    let slide = deck.slides.get(index)?;
    let slide_theme = deck.theme_for_slide(index);
    let mut matches = Vec::new();
    shapes(&slide.content.shapes, id, &mut matches);
    if matches.len() != 1 {
        return None;
    }
    let shape = matches[0];
    let text = shape.text.as_ref()?;
    let length: usize = text
        .paragraphs
        .iter()
        .map(|p| {
            p.runs
                .iter()
                .map(|r| r.text.encode_utf16().count())
                .sum::<usize>()
        })
        .sum::<usize>()
        + text.paragraphs.len().saturating_sub(1);
    if end > length {
        return None;
    }
    let inherited = InheritedContent {
        layout: slide.layout_content.as_ref(),
        master: slide.master_content.as_ref(),
        theme_fonts: Some(slide_theme.fonts),
        default_text_style: Some(&deck.default_text_style),
        ..Default::default()
    };
    let overridden_theme = slide
        .content
        .effective_color_map(slide.layout_content.as_ref())
        .map(|map| slide_theme.colors.with_color_map(map.clone()));
    let context = Context {
        inherited: &inherited,
        theme: overridden_theme.as_ref().unwrap_or(slide_theme.colors),
        measure: &HeuristicMeasure,
    };
    let context = style::text_context_for_shape(shape, &context);
    let mut base = 0;
    let mut result: Option<TextFormatState> = None;
    let collapsed = start == end;
    for (p_index, p) in text.paragraphs.iter().enumerate() {
        let resolved = style::resolve_paragraph(p, text.list_style.as_ref(), &context);
        let length: usize = p.runs.iter().map(|r| r.text.encode_utf16().count()).sum();
        let paragraph_end = base + length;
        if collapsed && (start < base || start > paragraph_end) {
            base = paragraph_end + 1;
            continue;
        }
        let mark = TextRun {
            size_pt: p.end_run_props.size_pt.or(p.end_size_pt),
            bold_stated: p.end_run_props.bold,
            italic_stated: p.end_run_props.italic,
            underline_stated: p.end_run_props.underline,
            color: p.end_run_props.color.clone(),
            font: p.end_run_props.font.clone(),
            east_asian_font: p.end_run_props.east_asian_font.clone(),
            ..Default::default()
        };
        let mut at = base;
        let mut caret_run = None;
        for run in &p.runs {
            let to = at + run.text.encode_utf16().count();
            if collapsed
                && at <= start
                && (start <= to || run.text.is_empty())
                && caret_run.is_none()
            {
                caret_run = Some(run);
            }
            if !collapsed && start < to && end > at {
                let value =
                    TextFormatState::resolved(&style::resolve_run(run, &resolved, &context, 1.0));
                if let Some(current) = &mut result {
                    current.merge(&value);
                } else {
                    result = Some(value);
                }
            }
            at = to;
        }
        if collapsed {
            let run = caret_run.unwrap_or(&mark);
            return Some(TextFormatState::resolved(&style::resolve_run(
                run, &resolved, &context, 1.0,
            )));
        }
        if (p_index + 1 < text.paragraphs.len() && start <= paragraph_end && end > paragraph_end)
            || (length == 0 && start <= base && end >= base)
        {
            let value =
                TextFormatState::resolved(&style::resolve_run(&mark, &resolved, &context, 1.0));
            if let Some(current) = &mut result {
                current.merge(&value);
            } else {
                result = Some(value);
            }
        }
        base = paragraph_end + 1;
    }
    result
}
