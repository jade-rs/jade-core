//! Paragraph selection and authoring values share the renderer's style resolver.
use crate::{style, Context, HeuristicMeasure, InheritedContent};
use jade_pptx_reader::{Bullet, Deck, Shape, ShapeItem, Spacing, TextAlign};
use serde::Serialize;

/// Paragraph separators belong to the preceding paragraph. Soft breaks do not
/// introduce paragraphs; an empty selected paragraph still receives formatting.
pub fn paragraph_indices_for_selection(
    lengths: &[usize],
    start: usize,
    end: usize,
) -> Option<Vec<usize>> {
    let total = lengths
        .iter()
        .try_fold(0usize, |sum, n| sum.checked_add(*n))?
        .checked_add(lengths.len().saturating_sub(1))?;
    if lengths.is_empty() || start > end || end > total {
        return None;
    }
    let mut base = 0;
    let mut indices = Vec::new();
    for (index, length) in lengths.iter().enumerate() {
        let mark = base + length;
        if start == end {
            if start >= base && start <= mark {
                return Some(vec![index]);
            }
        } else if (start <= mark && end > base) || (*length == 0 && start <= base && end == base) {
            indices.push(index);
        }
        if index + 1 < lengths.len() {
            base = mark + 1;
        }
    }
    Some(indices)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "mode", content = "value", rename_all = "lowercase")]
pub enum ParagraphSpacingState {
    Percent(f32),
    Points(f32),
}
impl ParagraphSpacingState {
    fn from(value: Spacing) -> Self {
        match value {
            Spacing::Percent(v) => Self::Percent(v * 100.0),
            Spacing::Points(v) => Self::Points(v),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParagraphFormatState {
    pub alignment: Option<String>,
    /// none, character or numbered; None identifies a mixed selection.
    pub bullet: Option<String>,
    pub character: Option<String>,
    pub scheme: Option<String>,
    pub start_at: Option<u32>,
    pub bullet_font: Option<String>,
    pub level: Option<u8>,
    /// None means mixed or left unspecified by the inheritance chain.
    pub margin_left_pt: Option<f64>,
    pub margin_right_pt: Option<f64>,
    pub indent_pt: Option<f64>,
    pub line_spacing: Option<ParagraphSpacingState>,
    pub space_before: Option<ParagraphSpacingState>,
    pub space_after: Option<ParagraphSpacingState>,
}
impl ParagraphFormatState {
    fn merge(&mut self, other: &Self) {
        macro_rules! merge { ($($field:ident),*) => { $(if self.$field != other.$field { self.$field = None; })* }; }
        merge!(
            alignment,
            bullet,
            character,
            scheme,
            start_at,
            bullet_font,
            level,
            margin_left_pt,
            margin_right_pt,
            indent_pt,
            line_spacing,
            space_before,
            space_after
        );
    }
}
fn shapes<'a>(items: &'a [ShapeItem], id: u32, matches: &mut Vec<&'a Shape>) {
    for item in items {
        match item {
            ShapeItem::Shape(s) if s.id == Some(id) => matches.push(s),
            ShapeItem::Group(g) => shapes(&g.children, id, matches),
            _ => {}
        }
    }
}
pub fn paragraph_format_for_deck_shape(
    deck: &Deck,
    index: usize,
    id: u32,
    start: usize,
    end: usize,
) -> Option<ParagraphFormatState> {
    let slide = deck.slides.get(index)?;
    let slide_theme = deck.theme_for_slide(index);
    let mut found = Vec::new();
    shapes(&slide.content.shapes, id, &mut found);
    if found.len() != 1 {
        return None;
    }
    let shape = found[0];
    let text = shape.text.as_ref()?;
    let lengths: Vec<_> = text
        .paragraphs
        .iter()
        .map(|p| p.runs.iter().map(|r| r.text.encode_utf16().count()).sum())
        .collect();
    let indices = paragraph_indices_for_selection(&lengths, start, end)?;
    let inherited = InheritedContent {
        layout: slide.layout_content.as_ref(),
        master: slide.master_content.as_ref(),
        theme_fonts: Some(slide_theme.fonts),
        default_text_style: Some(&deck.default_text_style),
        ..Default::default()
    };
    let theme = slide
        .content
        .effective_color_map(slide.layout_content.as_ref())
        .map(|map| slide_theme.colors.with_color_map(map.clone()));
    let context = Context {
        inherited: &inherited,
        theme: theme.as_ref().unwrap_or(slide_theme.colors),
        measure: &HeuristicMeasure,
    };
    let context = style::text_context_for_shape(shape, &context);
    let mut result: Option<ParagraphFormatState> = None;
    for index in indices {
        let p = &text.paragraphs[index];
        let r = style::resolve_paragraph(p, text.list_style.as_ref(), &context);
        let (kind, character, scheme) = match &r.bullet {
            Some(Bullet::Char(c)) => ("character", Some(c.clone()), None),
            Some(Bullet::Picture { .. }) => ("picture", None, None),
            Some(Bullet::AutoNumber { scheme }) => ("numbered", None, Some(scheme.clone())),
            _ => ("none", None, None),
        };
        let value = ParagraphFormatState {
            alignment: Some(
                match r.align.unwrap_or(TextAlign::Left) {
                    TextAlign::Left => "left",
                    TextAlign::Center => "center",
                    TextAlign::Right => "right",
                    TextAlign::Justify => "justify",
                    TextAlign::Distributed => "distributed",
                }
                .into(),
            ),
            bullet: Some(kind.into()),
            character,
            scheme,
            start_at: p.number_start,
            bullet_font: style::resolve_face(r.bullet_font.as_deref(), context.fonts),
            level: Some(p.level),
            margin_left_pt: r.margin_left_emu.map(|v| v as f64 / 12700.0),
            margin_right_pt: r.margin_right_emu.map(|v| v as f64 / 12700.0),
            indent_pt: r.indent_emu.map(|v| v as f64 / 12700.0),
            line_spacing: Some(ParagraphSpacingState::from(
                r.line_spacing.unwrap_or(Spacing::Percent(1.0)),
            )),
            space_before: Some(ParagraphSpacingState::from(
                r.space_before.unwrap_or(Spacing::Points(0.0)),
            )),
            space_after: Some(ParagraphSpacingState::from(
                r.space_after.unwrap_or(Spacing::Points(0.0)),
            )),
        };
        if let Some(result) = &mut result {
            result.merge(&value);
        } else {
            result = Some(value);
        }
    }
    result
}
