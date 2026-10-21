//! Wraps a text body to its box and places each styled run on a line.
//!
//! The wrap itself is `jade_text_layout::wrap_text` over the paragraph's
//! concatenated text. What this module adds is the mapping back: which run
//! each wrapped grapheme came from, so a line becomes styled segments with
//! advancing x positions rather than one anonymous string — and the vertical
//! rhythm PowerPoint gives paragraphs: line spacing, space before and after,
//! bullets hanging in the margin, autofit shrinking, and anchoring.

use jade_pptx_reader::{
    emu_to_px, Autofit, Bullet, Spacing, TextAlign, TextAnchor, TextBody, TextInsets, TextParagraph,
};
use jade_text_layout::{wrap_text, WrapOptions};
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use crate::measure::RunStyle;
use crate::style::{paragraph_size_pt, resolve_paragraph, resolve_run, ResolvedRun, TextContext};
use crate::{
    resolve_color, ASCENT_RATIO, LINE_SPACING, PX_PER_PT, TEXT_INSET_X_EMU, TEXT_INSET_Y_EMU,
};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PlacedLine {
    /// The line box's top, in shape-local px.
    #[serde(default)]
    pub top: f64,
    /// The line box's height: the advance to the next line.
    #[serde(default)]
    pub height: f64,
    pub runs: Vec<PlacedRun>,
    /// Logical UTF-16 span and aligned start for empty-line caret placement.
    #[serde(default)]
    pub source_range: Option<(usize, usize)>,
    #[serde(default)]
    pub text_left: f64,
    /// Hard breaks or wrap spaces omitted from the painted end of this line.
    #[serde(default)]
    pub selection_tail: Option<(usize, usize)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedRun {
    pub text: String,
    /// Left edge, in shape-local px.
    pub x: f64,
    /// Baseline, in shape-local px.
    pub baseline_y: f64,
    /// Advance width, in px, as measured for layout.
    #[serde(default)]
    pub width: f64,
    pub size_pt: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    #[serde(default)]
    pub strike: bool,
    pub color: String,
    pub font: Option<String>,
    /// The face East Asian characters draw with, when the run names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub east_asian_font: Option<String>,
    /// The run's hyperlink target, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hyperlink: Option<String>,
    /// One UTF-16 source boundary per displayed grapheme, plus its end.
    /// Empty on generated bullets. Capitals may expand one source grapheme.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_offsets: Vec<usize>,
    /// `<a:highlight>` colour painted behind the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highlight: Option<String>,
    /// Extra advance after each character (`spc`), already in `width`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub letter_spacing_px: f64,
    /// The glyphs' outline (`<a:ln>` on the run).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_outline: Option<crate::PlacedOutline>,
    /// The glyphs' shadow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_shadow: Option<crate::PlacedShadow>,
    /// The glyphs' glow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_glow: Option<crate::PlacedGlow>,
    /// A picture bullet's image part: the run is the bullet, drawn as this
    /// picture in a square of `width` sitting on the baseline, its text
    /// the fallback when the picture cannot be drawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bullet_image: Option<String>,
    /// A non-single underline's `u` value (`dbl`, `wavy`, `dotted`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline_style: Option<String>,
    #[serde(default)]
    pub double_strike: bool,
}

/// PowerPoint's default tab spacing: one inch.
const DEFAULT_TAB_PX: f64 = 96.0;

fn is_zero(value: &f64) -> bool {
    *value == 0.0
}

/// The resolved `bodyPr` a text body lays out in, in px.
pub(crate) struct TextFrame {
    pub(crate) inset_left: f64,
    pub(crate) inset_top: f64,
    pub(crate) inset_right: f64,
    pub(crate) inset_bottom: f64,
    pub(crate) anchor: Option<TextAnchor>,
}

impl TextFrame {
    /// The frame for a body with `explicit` insets/anchor over the
    /// `inherited` ones, over PowerPoint's defaults.
    pub(crate) fn resolve(
        explicit: &TextInsets,
        anchor: Option<TextAnchor>,
        inherited: &TextInsets,
        inherited_anchor: Option<TextAnchor>,
    ) -> Self {
        let inset = |own: Option<i64>, from: Option<i64>, default: i64| {
            emu_to_px(own.or(from).unwrap_or(default))
        };
        Self {
            inset_left: inset(explicit.left_emu, inherited.left_emu, TEXT_INSET_X_EMU),
            inset_top: inset(explicit.top_emu, inherited.top_emu, TEXT_INSET_Y_EMU),
            inset_right: inset(explicit.right_emu, inherited.right_emu, TEXT_INSET_X_EMU),
            inset_bottom: inset(explicit.bottom_emu, inherited.bottom_emu, TEXT_INSET_Y_EMU),
            anchor: anchor.or(inherited_anchor),
        }
    }
}

/// Lays a body out in a box, anchored. Returns the lines and the content
/// height (top inset + lines + bottom inset), which is what the box would
/// need to be to show everything.
pub(crate) fn layout_text(
    text: &TextBody,
    box_width_px: f64,
    box_height_px: f64,
    context: &TextContext<'_>,
) -> (Vec<PlacedLine>, f64) {
    layout_text_in_region(text, box_width_px, box_height_px, None, context)
}

pub(crate) fn layout_text_in_region(
    text: &TextBody,
    box_width_px: f64,
    box_height_px: f64,
    region: Option<crate::Rect>,
    context: &TextContext<'_>,
) -> (Vec<PlacedLine>, f64) {
    let mut frame = TextFrame::resolve(&text.insets, text.anchor, &context.insets, context.anchor);
    if let Some(region) = region {
        frame.inset_left += region.x;
        frame.inset_top += region.y;
        frame.inset_right += box_width_px - region.x - region.width;
        frame.inset_bottom += box_height_px - region.y - region.height;
    }
    if let Some((count, spacing_emu)) = text.columns.filter(|(n, _)| *n > 1) {
        return layout_columns(
            text,
            box_width_px,
            box_height_px,
            frame,
            count,
            spacing_emu,
            context,
        );
    }
    let (mut lines, content_bottom) = match context.autofit.unwrap_or(text.autofit) {
        Autofit::Normal {
            font_scale,
            line_spacing_reduction,
        } => shrink_to_fit(
            text,
            box_width_px,
            box_height_px,
            &frame,
            context,
            stored_scale(font_scale, line_spacing_reduction),
        ),
        _ => flow_text(text, box_width_px, &frame, context),
    };
    anchor_lines(&mut lines, &frame, box_height_px, content_bottom);
    (lines, content_bottom + frame.inset_bottom)
}

/// The stored `fontScale`/`lnSpcReduction`, sanitised.
fn stored_scale(font_scale: f32, line_spacing_reduction: f32) -> (f32, f64) {
    (
        if font_scale > 0.0 && font_scale.is_finite() {
            font_scale.min(1.0)
        } else {
            1.0
        },
        if line_spacing_reduction.is_finite() {
            line_spacing_reduction.clamp(0.0, 0.9) as f64
        } else {
            0.0
        },
    )
}

/// The shrink steps PowerPoint's "shrink text on overflow" walks: line
/// spacing first, 10% then 20%, then the font down 7.5% at a time with the
/// spacing kept 20% tighter, to a quarter of its size — the pairs it
/// records as `fontScale`/`lnSpcReduction`.
fn autofit_steps() -> impl Iterator<Item = (f32, f64)> {
    [(1.0, 0.0), (1.0, 0.1), (1.0, 0.2), (0.925, 0.1)]
        .into_iter()
        .chain((0..=9).map(|step| (0.925 - 0.075 * step as f32, 0.2)))
}

/// `normAutofit`: the text at its stored scale when that fits; otherwise
/// the first of PowerPoint's shrink steps beyond the stored one that does
/// (or the last). A stored scale is where PowerPoint left the text, so a
/// deck opens as it was saved; text that has since grown — or that never
/// had a scale recorded — shrinks on from there.
fn shrink_to_fit(
    text: &TextBody,
    box_width_px: f64,
    box_height_px: f64,
    frame: &TextFrame,
    context: &TextContext<'_>,
    stored: (f32, f64),
) -> (Vec<PlacedLine>, f64) {
    // A hair of slack: substitute faces measure a little differently from
    // the ones PowerPoint fitted with, and a deck it saved unshrunk fit.
    let inner = (box_height_px - frame.inset_top - frame.inset_bottom).max(0.0);
    let limit = box_height_px - frame.inset_bottom + (inner * 0.02).max(0.5);
    let first = flow_text_scaled(text, box_width_px, frame, context, stored);
    if first.1 <= limit {
        return first;
    }
    let mut last = first;
    for step in autofit_steps().filter(|(scale, reduction)| {
        // Only steps that shrink further than what is stored.
        *scale < stored.0 - 1e-4 || (*scale <= stored.0 + 1e-4 && *reduction > stored.1 + 1e-4)
    }) {
        last = flow_text_scaled(text, box_width_px, frame, context, step);
        if last.1 <= limit {
            break;
        }
    }
    last
}

/// `numCol` text: flowed at one column's width, then dealt into columns
/// left to right, each starting again at the top inset when the last one
/// fills.
fn layout_columns(
    text: &TextBody,
    box_width_px: f64,
    box_height_px: f64,
    frame: TextFrame,
    count: u32,
    spacing_emu: i64,
    context: &TextContext<'_>,
) -> (Vec<PlacedLine>, f64) {
    let inner = (box_width_px - frame.inset_left - frame.inset_right).max(1.0);
    let spacing = emu_to_px(spacing_emu).max(0.0);
    let column = ((inner - spacing * (count - 1) as f64) / count as f64).max(1.0);
    let narrow = TextFrame {
        inset_right: frame.inset_right + (inner - column),
        ..frame
    };
    let (mut lines, _) = flow_text(text, box_width_px, &narrow, context);
    let limit = box_height_px - frame.inset_bottom;
    let mut index = 0u32;
    let mut lift = 0.0;
    let mut bottom = frame.inset_top;
    for line in &mut lines {
        // A column always takes at least its first line, however tall.
        let first_in_column = line.top - lift <= frame.inset_top + 0.5;
        if !first_in_column && index + 1 < count && line.top - lift + line.height > limit {
            index += 1;
            lift = line.top - frame.inset_top;
        }
        let shift = index as f64 * (column + spacing);
        line.top -= lift;
        line.text_left += shift;
        for run in &mut line.runs {
            run.x += shift;
            run.baseline_y -= lift;
        }
        bottom = f64::max(bottom, line.top + line.height);
    }
    (lines, bottom + frame.inset_bottom)
}

/// Shifts top-flowed lines for the frame's anchor. `content_bottom` is the
/// last line's bottom, from the shape's top. Bottom-anchored text that
/// overflows shifts upward, which is what PowerPoint does too.
pub(crate) fn anchor_lines(
    lines: &mut [PlacedLine],
    frame: &TextFrame,
    box_height_px: f64,
    content_bottom: f64,
) {
    let slack = (box_height_px - frame.inset_bottom) - content_bottom;
    let shift = match frame.anchor {
        Some(TextAnchor::Center) => slack / 2.0,
        Some(TextAnchor::Bottom) => slack,
        Some(TextAnchor::Top) | None => 0.0,
    };
    if shift != 0.0 && shift.is_finite() {
        for line in lines {
            line.top += shift;
            for run in &mut line.runs {
                run.baseline_y += shift;
            }
        }
    }
}

/// Flows every paragraph from the top inset down. Returns the lines and the
/// last line's bottom, from the shape's top.
pub(crate) fn flow_text(
    text: &TextBody,
    box_width_px: f64,
    frame: &TextFrame,
    context: &TextContext<'_>,
) -> (Vec<PlacedLine>, f64) {
    let scale = match context.autofit.unwrap_or(text.autofit) {
        Autofit::Normal {
            font_scale,
            line_spacing_reduction,
        } => stored_scale(font_scale, line_spacing_reduction),
        Autofit::None | Autofit::Shape => (1.0, 0.0),
    };
    flow_text_scaled(text, box_width_px, frame, context, scale)
}

/// [`flow_text`] at an explicit autofit font scale and line spacing
/// reduction.
fn flow_text_scaled(
    text: &TextBody,
    box_width_px: f64,
    frame: &TextFrame,
    context: &TextContext<'_>,
    (font_scale, spacing_reduction): (f32, f64),
) -> (Vec<PlacedLine>, f64) {
    let inner_width = (box_width_px - frame.inset_left - frame.inset_right).max(1.0);

    let mut lines = Vec::new();
    let mut line_top = frame.inset_top;
    // Auto-number bullets count per outline level within one text body.
    let mut counters: Vec<(u32, String)> = Vec::new();
    let mut body_offset = 0;
    for (index, paragraph) in text.paragraphs.iter().enumerate() {
        layout_paragraph(
            paragraph,
            &ParagraphBox {
                inner_width,
                inset_left: frame.inset_left,
                wrap: context.wrap.unwrap_or(text.wrap),
                first_in_body: index == 0,
                font_scale,
                spacing_reduction,
                body_offset,
            },
            text.list_style.as_ref(),
            &mut line_top,
            context,
            &mut counters,
            &mut lines,
        );
        body_offset += paragraph
            .runs
            .iter()
            .map(|run| run.text.encode_utf16().count())
            .sum::<usize>()
            + 1;
    }
    for index in 0..lines.len().saturating_sub(1) {
        if let (Some((_, end)), Some((next, _))) =
            (lines[index].source_range, lines[index + 1].source_range)
        {
            if next > end {
                lines[index].selection_tail = Some((end, next));
            }
        }
    }
    (lines, line_top)
}

struct ParagraphBox {
    inner_width: f64,
    inset_left: f64,
    wrap: bool,
    first_in_body: bool,
    font_scale: f32,
    spacing_reduction: f64,
    body_offset: usize,
}

/// The marker an auto-numbered bullet draws for its scheme: the numeral
/// style (arabic, alphabetic, roman) and the punctuation around it.
fn auto_number_marker(
    counters: &mut Vec<(u32, String)>,
    level: usize,
    scheme: &str,
    start: Option<u32>,
) -> String {
    if counters.len() <= level {
        counters.resize(level + 1, (0, String::new()));
    }
    // Another scheme at the same level starts its own count.
    if counters[level].1 != scheme {
        counters[level] = (0, scheme.to_string());
    }
    match start {
        Some(start) if counters[level].0 == 0 => counters[level].0 = start.max(1),
        _ => counters[level].0 += 1,
    }
    format_number(counters[level].0, scheme)
}

/// `arabicPeriod` → "1.", `alphaLcParenR` → "a)", `romanUcParenBoth` →
/// "(I)". Unknown schemes fall back to arabic with a period.
pub fn format_number(n: u32, scheme: &str) -> String {
    // The East Asian and enclosed schemes carry their own punctuation.
    match scheme {
        "circleNumDbPlain" | "circleNumWdWhitePlain" if (1..=20).contains(&n) => {
            return char::from_u32(0x2460 + n - 1).map_or(n.to_string(), String::from);
        }
        "circleNumWdBlackPlain" if (1..=10).contains(&n) => {
            return char::from_u32(0x2776 + n - 1).map_or(n.to_string(), String::from);
        }
        "circleNumWdBlackPlain" if (11..=20).contains(&n) => {
            return char::from_u32(0x24EB + n - 11).map_or(n.to_string(), String::from);
        }
        "arabicDbPeriod" => return format!("{}．", full_width(n)),
        "arabicDbPlain" => return full_width(n),
        // `Db` is the double-byte full stop; the others take an ASCII one.
        "ea1JpnChsDbPeriod" => return format!("{}．", kanji(n)),
        "ea1ChsPeriod" | "ea1ChtPeriod" | "ea1JpnKorPeriod" => {
            return format!("{}.", kanji(n));
        }
        "ea1JpnKorPlain" | "ea1ChsPlain" | "ea1ChtPlain" => return kanji(n),
        "arabic1Minus" => return format!("{n}-"),
        "arabic2Minus" => return format!("- {n} -"),
        _ => {}
    }
    let numeral = if scheme.starts_with("alphaLc") {
        alpha(n).to_lowercase()
    } else if scheme.starts_with("alphaUc") {
        alpha(n)
    } else if scheme.starts_with("romanLc") {
        roman(n).to_lowercase()
    } else if scheme.starts_with("romanUc") {
        roman(n)
    } else {
        n.to_string()
    };
    if scheme.ends_with("ParenBoth") {
        format!("({numeral})")
    } else if scheme.ends_with("ParenR") {
        format!("{numeral})")
    } else if scheme.ends_with("Plain") {
        numeral
    } else {
        format!("{numeral}.")
    }
}

/// 12 → １２.
fn full_width(n: u32) -> String {
    n.to_string()
        .chars()
        .map(|digit| char::from_u32(digit as u32 - '0' as u32 + 0xFF10).unwrap_or(digit))
        .collect()
}

/// 1 → 一, 10 → 十, 23 → 二十三, 105 → 百五.
fn kanji(n: u32) -> String {
    const DIGITS: [&str; 10] = ["〇", "一", "二", "三", "四", "五", "六", "七", "八", "九"];
    if n == 0 || n >= 10_000 {
        return n.to_string();
    }
    let mut out = String::new();
    for (unit, name) in [(1000, "千"), (100, "百"), (10, "十")] {
        let digit = (n / unit) % 10;
        if digit > 1 {
            out.push_str(DIGITS[digit as usize]);
        }
        if digit > 0 {
            out.push_str(name);
        }
    }
    if !n.is_multiple_of(10) {
        out.push_str(DIGITS[(n % 10) as usize]);
    }
    out
}

/// 1 → A, 26 → Z, 27 → AA.
fn alpha(n: u32) -> String {
    let mut letters = Vec::new();
    let mut n = n.max(1);
    while n > 0 {
        let rem = (n - 1) % 26;
        letters.push((b'A' + rem as u8) as char);
        n = (n - 1) / 26;
    }
    letters.iter().rev().collect()
}

fn roman(n: u32) -> String {
    const TABLE: [(u32, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut n = n.clamp(1, 3999);
    let mut out = String::new();
    for (value, digits) in TABLE {
        while n >= value {
            out.push_str(digits);
            n -= value;
        }
    }
    out
}

/// A line box's height for a run size and the paragraph's line spacing.
fn line_height_px(size_pt: f32, spacing: Option<Spacing>, reduction: f64) -> f64 {
    let size_px = size_pt as f64 * PX_PER_PT;
    match spacing {
        Some(Spacing::Points(points)) if points.is_finite() && points > 0.0 => {
            points as f64 * PX_PER_PT
        }
        Some(Spacing::Percent(percent)) if percent.is_finite() && percent > 0.0 => {
            size_px * LINE_SPACING * (percent as f64 - reduction).max(0.1)
        }
        _ => size_px * LINE_SPACING * (1.0 - reduction),
    }
}

/// Space before or after a paragraph, in px, for the paragraph's size.
fn paragraph_space_px(spacing: Option<Spacing>, size_pt: f32) -> f64 {
    match spacing {
        Some(Spacing::Points(points)) if points.is_finite() => (points as f64).max(0.0) * PX_PER_PT,
        Some(Spacing::Percent(percent)) if percent.is_finite() => {
            (percent as f64).max(0.0) * size_pt as f64 * PX_PER_PT
        }
        _ => 0.0,
    }
}

fn layout_paragraph(
    paragraph: &TextParagraph,
    boxed: &ParagraphBox,
    own_style: Option<&jade_pptx_reader::ListStyle>,
    line_top: &mut f64,
    context: &TextContext<'_>,
    counters: &mut Vec<(u32, String)>,
    out: &mut Vec<PlacedLine>,
) {
    let resolved = resolve_paragraph(paragraph, own_style, context);
    let runs: Vec<ResolvedRun> = paragraph
        .runs
        .iter()
        .map(|run| resolve_run(run, &resolved, context, boxed.font_scale))
        .collect();
    let paragraph_size_pt = runs
        .first()
        .map(|run| run.style.size_pt)
        .unwrap_or_else(|| paragraph_size_pt(paragraph, &resolved, boxed.font_scale));

    // PowerPoint ignores space-before on the first paragraph of a body.
    if !boxed.first_in_body {
        *line_top += paragraph_space_px(resolved.space_before, paragraph_size_pt);
    }

    // `cap="all"` shows in capitals: wrap and measure what is drawn.
    let run_texts: Vec<String> = paragraph
        .runs
        .iter()
        .map(|run| {
            if run.all_caps {
                run.text.to_uppercase()
            } else {
                run.text.clone()
            }
        })
        .collect();
    let full_text: String = run_texts.concat();

    // A shallower paragraph restarts the numbering of everything deeper.
    let level = paragraph.level as usize;
    for counter in counters.iter_mut().skip(level + 1) {
        counter.0 = 0;
    }
    let marker = match &resolved.bullet {
        Some(Bullet::Char(character)) => Some(character.clone()),
        Some(Bullet::AutoNumber { scheme }) => Some(auto_number_marker(
            counters,
            level,
            scheme,
            paragraph.number_start,
        )),
        // A picture bullet takes a glyph's place; the run carries the
        // picture, and the round bullet draws where it cannot.
        Some(Bullet::Picture { .. }) => Some("\u{2022}".to_string()),
        Some(Bullet::None) | None => None,
    };
    let bullet_image = match &resolved.bullet {
        Some(Bullet::Picture { image_part, .. }) => image_part.clone(),
        _ => None,
    };
    let marker_style = RunStyle {
        size_pt: paragraph_size_pt * resolved.bullet_size_percent.unwrap_or(1.0).max(0.01),
        bold: runs.first().map(|run| run.style.bold).unwrap_or(false),
        italic: runs.first().map(|run| run.style.italic).unwrap_or(false),
        font: crate::style::resolve_face(resolved.bullet_font.as_deref(), context.fonts)
            .or_else(|| runs.first().and_then(|run| run.style.font.clone())),
        east_asian_font: runs
            .first()
            .and_then(|run| run.style.east_asian_font.clone()),
    };
    let marker_width = match (&bullet_image, marker.as_deref()) {
        // A picture bullet is a square as tall as the text's ascent.
        (Some(_), Some(_)) => marker_style.size_pt as f64 * PX_PER_PT * crate::ASCENT_RATIO,
        (None, Some(marker)) => context.measure.measure(marker, &marker_style),
        _ => 0.0,
    };

    // Where the bullet and the text go. With `marL`/`indent` known (the
    // paragraph's or an inherited level's), the bullet sits at the first
    // line's indent and the text at the margin — PowerPoint's hanging
    // layout. Without either, fall back to the default list step per level.
    let (marker_x, text_left) = match (resolved.margin_left_emu, resolved.indent_emu) {
        (None, None) => {
            let slot = emu_to_px(crate::LEVEL_INDENT_EMU);
            let indent = level as f64 * slot;
            let marker_slot = if marker.is_some() { slot } else { 0.0 };
            (
                boxed.inset_left + indent,
                boxed.inset_left + indent + marker_slot,
            )
        }
        (margin, indent) => {
            let margin_px = emu_to_px(margin.unwrap_or(0));
            let indent_px = emu_to_px(indent.unwrap_or(0));
            let first_x = boxed.inset_left + margin_px + indent_px;
            let rest_x = boxed.inset_left + margin_px;
            match &marker {
                Some(_) => {
                    // A bullet that fits in its hanging slot leaves the text
                    // at the margin; a wider one pushes the text along by a
                    // quarter em, as PowerPoint does.
                    let after_marker = first_x + marker_width;
                    if after_marker <= rest_x {
                        (first_x, rest_x)
                    } else {
                        (
                            first_x,
                            after_marker + 0.25 * marker_style.size_pt as f64 * PX_PER_PT,
                        )
                    }
                }
                None => (first_x, first_x.max(rest_x)),
            }
        }
    };
    let text_offset = (text_left - boxed.inset_left).max(0.0);
    let right_margin = emu_to_px(resolved.margin_right_emu.unwrap_or(0)).max(0.0);
    let usable_width = (boxed.inner_width - text_offset - right_margin).max(1.0);
    let wrap_width = if boxed.wrap { usable_width } else { 1.0e12 };

    // Every non-newline grapheme with the run it came from, in text order.
    // `wrap_text` measures exactly these, in exactly this order.
    let mut mapped = Vec::new();
    let mut source_offset = boxed.body_offset;
    let mut displayed_byte = 0;
    for (run_index, run) in paragraph.runs.iter().enumerate() {
        let mut displayed = run_texts[run_index].graphemes(true);
        for source in run.text.graphemes(true) {
            let from = source_offset;
            source_offset += source.encode_utf16().count();
            let count = if run.all_caps {
                source.to_uppercase().graphemes(true).count()
            } else {
                1
            };
            for index in 0..count {
                if let Some(g) = displayed.next() {
                    let to = if index + 1 == count {
                        source_offset
                    } else {
                        from
                    };
                    mapped.push((
                        displayed_byte,
                        displayed_byte + g.len(),
                        run_index,
                        from,
                        to,
                        source_offset,
                    ));
                    displayed_byte += g.len();
                }
            }
        }
    }
    // Graphemes can cross authored run boundaries (combining marks, ZWJ).
    // Resolve their provenance against the concatenated text used by wrapping.
    let mut stream = Vec::new();
    let mut newline_offsets = Vec::new();
    let mut at = 0;
    for (byte, g) in full_text.grapheme_indices(true) {
        while at < mapped.len() && mapped[at].1 <= byte {
            at += 1;
        }
        let Some(first) = mapped.get(at) else {
            break;
        };
        let mut last = at;
        while last + 1 < mapped.len() && mapped[last].1 < byte + g.len() {
            last += 1;
        }
        if g == "\n" {
            newline_offsets.push(first.3);
        } else {
            stream.push((g, first.2, first.3, mapped[last].4, mapped[last].5));
        }
    }

    let mut next = 0usize;
    let fallback_style = RunStyle {
        size_pt: paragraph_size_pt,
        bold: false,
        italic: false,
        font: None,
        east_asian_font: None,
    };
    let mut measure_grapheme = |grapheme: &str| -> f64 {
        let style = stream
            .get(next)
            .map(|(_, run, _, _, _)| &runs[*run].style)
            .or_else(|| runs.first().map(|run| &run.style))
            .unwrap_or(&fallback_style);
        let spacing = stream
            .get(next)
            .map_or(0.0, |(_, run, _, _, _)| runs[*run].spacing_px);
        next += 1;
        context.measure.measure(grapheme, style) + spacing
    };
    let wrapped = wrap_text(
        &full_text,
        wrap_width,
        &mut measure_grapheme,
        WrapOptions::default(),
    );

    // Map each wrapped line's graphemes back onto the stream. A soft wrap
    // swallows the space it broke at, so a space in the stream that the line
    // does not start with is consumed silently.
    let first_line_index = out.len();
    let mut cursor = 0usize;
    let mut logical_cursor = boxed.body_offset;
    for (line_index, line_text) in wrapped.iter().enumerate() {
        let mut segments: Vec<(usize, String, Vec<usize>)> = Vec::new();
        let mut coverage_end = logical_cursor;
        for grapheme in line_text.graphemes(true) {
            while cursor < stream.len() && stream[cursor].0 == " " && grapheme != " " {
                cursor += 1;
            }
            if cursor >= stream.len() {
                break;
            }
            let (g, run_index, from, to, full_end) = stream[cursor];
            coverage_end = coverage_end.max(full_end);
            cursor += 1;
            match segments.last_mut() {
                Some((last, text, offsets)) if *last == run_index => {
                    text.push_str(g);
                    offsets.push(to);
                }
                _ => segments.push((run_index, g.to_string(), vec![from, to])),
            }
        }

        let source_end = segments
            .last()
            .and_then(|s| s.2.last())
            .copied()
            .unwrap_or(logical_cursor);
        let expand = resolved.align == Some(TextAlign::Distributed)
            || resolved.align == Some(TextAlign::Justify)
                && line_index + 1 < wrapped.len()
                && !newline_offsets.contains(&source_end);
        let (segments, expansion_slots) = if expand {
            crate::justification::split(segments, resolved.align == Some(TextAlign::Distributed))
        } else {
            let count = segments.len();
            (segments, vec![false; count])
        };

        let line_size_pt = segments
            .iter()
            .map(|(run, _, _)| runs[*run].style.size_pt)
            .fold(0.0f32, f32::max);
        let line_size_pt = if line_size_pt > 0.0 {
            line_size_pt
        } else {
            paragraph_size_pt
        };
        let size_px = line_size_pt as f64 * PX_PER_PT;
        let height = line_height_px(line_size_pt, resolved.line_spacing, boxed.spacing_reduction);
        // Extra leading goes above the glyphs, as PowerPoint places it.
        let baseline = *line_top + height - size_px * (LINE_SPACING - ASCENT_RATIO);

        let widths: Vec<f64> = segments
            .iter()
            .map(|(run, text, _)| {
                context.measure.measure(text, &runs[*run].style)
                    + runs[*run].spacing_px * text.graphemes(true).count() as f64
            })
            .collect();
        let line_width: f64 = widths.iter().sum();
        let slack = usable_width - line_width;
        // Wrapped text never overflows, so the slack is never negative in a
        // way that matters; unwrapped text centres on the box regardless.
        let slack = if boxed.wrap { slack.max(0.0) } else { slack };
        let align_offset = match resolved.align {
            Some(TextAlign::Center) => slack / 2.0,
            Some(TextAlign::Right) => slack,
            _ => 0.0,
        };

        let slot_count = expansion_slots.iter().filter(|slot| **slot).count();
        let extra = if slot_count > 0 {
            slack.max(0.0) / slot_count as f64
        } else {
            0.0
        };
        let widths: Vec<_> = widths
            .into_iter()
            .zip(expansion_slots)
            .map(|(width, slot)| width + if slot { extra } else { 0.0 })
            .collect();

        let source_start = segments
            .first()
            .and_then(|s| s.2.first())
            .copied()
            .unwrap_or(logical_cursor);
        let source_end = segments
            .last()
            .and_then(|s| s.2.last())
            .copied()
            .unwrap_or(source_start);
        logical_cursor = source_end + usize::from(newline_offsets.contains(&source_end));
        let mut x = text_left + align_offset;
        let mut placed = Vec::new();
        // The marker draws once, on the paragraph's first line.
        if out.len() == first_line_index {
            if let Some(marker_text) = &marker {
                let color = resolved
                    .bullet_color
                    .as_ref()
                    .map(|color| resolve_color(color, context.theme))
                    .or_else(|| runs.first().map(|run| run.color.clone()))
                    .unwrap_or_else(|| crate::style::default_text_color(context.theme));
                placed.push(PlacedRun {
                    text: marker_text.clone(),
                    x: marker_x,
                    baseline_y: baseline,
                    width: marker_width,
                    size_pt: marker_style.size_pt,
                    bold: marker_style.bold,
                    italic: marker_style.italic,
                    underline: false,
                    strike: false,
                    color,
                    font: marker_style.font.clone(),
                    east_asian_font: marker_style.east_asian_font.clone(),
                    hyperlink: None,
                    source_offsets: Vec::new(),
                    highlight: None,
                    letter_spacing_px: 0.0,
                    text_outline: None,
                    text_shadow: None,
                    text_glow: None,
                    underline_style: None,
                    double_strike: false,
                    bullet_image: bullet_image.clone(),
                });
            }
        }
        // Tabs advance to the next default stop, measured from the inset.
        let default_tab = paragraph
            .default_tab_emu
            .map_or(DEFAULT_TAB_PX, emu_to_px)
            .max(1.0);
        let tab_stop = |x: f64| -> f64 {
            let from = x - boxed.inset_left;
            // The paragraph's own stops first, then the default spacing.
            if let Some(stop) = paragraph
                .tab_stops_emu
                .iter()
                .map(|emu| emu_to_px(*emu))
                .find(|stop| *stop > from + 0.5)
            {
                return boxed.inset_left + stop;
            }
            boxed.inset_left + ((from / default_tab).floor() + 1.0) * default_tab
        };
        let mut pieces = Vec::new();
        for ((run_index, text, source_offsets), width) in segments.into_iter().zip(widths) {
            if !text.contains('\t') || expand {
                pieces.push((run_index, text, source_offsets, Some(width)));
                continue;
            }
            // One piece per tab: the text up to and including it.
            let mut chunk = String::new();
            let mut start = 0;
            let graphemes: Vec<&str> = text.graphemes(true).collect();
            for (index, g) in graphemes.iter().enumerate() {
                chunk.push_str(g);
                if *g == "\t" || index + 1 == graphemes.len() {
                    let offsets = source_offsets
                        .get(start..=(index + 1).min(source_offsets.len().saturating_sub(1)))
                        .map(<[usize]>::to_vec)
                        .unwrap_or_default();
                    pieces.push((run_index, std::mem::take(&mut chunk), offsets, None));
                    start = index + 1;
                }
            }
        }
        for (run_index, text, source_offsets, width) in pieces {
            let run = &runs[run_index];
            let width = width.unwrap_or_else(|| {
                let glyphs = text.trim_end_matches('\t');
                let drawn = context.measure.measure(glyphs, &run.style)
                    + run.spacing_px * glyphs.graphemes(true).count() as f64;
                if text.ends_with('\t') {
                    tab_stop(x + drawn) - x
                } else {
                    drawn
                }
            });
            placed.push(PlacedRun {
                text,
                x,
                baseline_y: baseline - run.baseline_shift_px,
                width,
                size_pt: run.style.size_pt,
                bold: run.style.bold,
                italic: run.style.italic,
                underline: run.underline,
                strike: run.strike,
                color: run.color.clone(),
                font: run.style.font.clone(),
                east_asian_font: run.style.east_asian_font.clone(),
                hyperlink: run.hyperlink.clone(),
                source_offsets,
                highlight: run.highlight.clone(),
                letter_spacing_px: run.spacing_px,
                text_outline: run.outline.clone(),
                text_shadow: run.shadow.clone(),
                text_glow: run.glow.clone(),
                bullet_image: None,
                underline_style: run.underline_style.clone(),
                double_strike: run.double_strike,
            });
            x += width;
        }

        out.push(PlacedLine {
            top: *line_top,
            height,
            runs: placed,
            source_range: Some((source_start, coverage_end.max(source_end))),
            text_left: text_left + align_offset,
            selection_tail: None,
        });
        *line_top += height;
    }

    *line_top += paragraph_space_px(resolved.space_after, paragraph_size_pt);
}

#[cfg(test)]
mod numbering_tests {
    use super::format_number;

    #[test]
    fn east_asian_and_enclosed_schemes() {
        assert_eq!(format_number(3, "circleNumDbPlain"), "③");
        assert_eq!(format_number(12, "circleNumWdBlackPlain"), "⓬");
        assert_eq!(format_number(12, "arabicDbPeriod"), "１２．");
        assert_eq!(format_number(23, "ea1JpnChsDbPeriod"), "二十三．");
        assert_eq!(format_number(105, "ea1JpnKorPlain"), "百五");
        assert_eq!(format_number(4, "romanLcParenR"), "iv)");
    }
}
