//! Text style inheritance: what a paragraph and its runs look like once
//! every unsaid property has been filled in from the shape's own list style,
//! the layout and master placeholders it inherits from, the master's text
//! styles, and the presentation's default text style — in that order.

use jade_pptx_reader::{
    Color, LevelStyle, ListStyle, RunProps, Shape, TextAnchor, TextInsets, TextParagraph, TextRun,
    ThemeColors, ThemeFonts,
};

use crate::measure::{RunStyle, TextMeasure};
use crate::{resolve_color, Context, DEFAULT_SIZE_PT};

/// The master text style a placeholder kind falls under: `titleStyle` for
/// titles, `bodyStyle` for bodies and everything that behaves like one,
/// `otherStyle` for the rest (dates, footers, slide numbers).
pub(crate) fn master_kind(kind: &str) -> &str {
    match kind {
        "ctrTitle" => "title",
        "subTitle" | "obj" | "pic" | "chart" | "tbl" | "dgm" | "media" | "clipArt" | "" => "body",
        other => other,
    }
}

/// Everything text layout needs beyond the text body itself.
pub(crate) struct TextContext<'a> {
    pub(crate) theme: &'a ThemeColors,
    pub(crate) fonts: Option<&'a ThemeFonts>,
    /// Inherited list styles, most specific first. The body's own
    /// `lstStyle` is not here; text layout consults it before these.
    pub(crate) chain: Vec<&'a ListStyle>,
    /// The shape style's `fontRef` colour, when the shape has one.
    pub(crate) font_color: Option<&'a Color>,
    /// `bodyPr` facts the placeholder chain supplies when the body itself
    /// leaves them unsaid.
    pub(crate) anchor: Option<TextAnchor>,
    pub(crate) insets: TextInsets,
    /// Run defaults a table style gives the cell, ahead of the chain.
    pub(crate) cell_defaults: RunProps,
    pub(crate) wrap: Option<bool>,
    pub(crate) autofit: Option<jade_pptx_reader::Autofit>,
    pub(crate) vertical: Option<jade_pptx_reader::TextVertical>,
    pub(crate) measure: &'a dyn TextMeasure,
}

pub(crate) fn text_context_for_shape<'a>(
    shape: &'a Shape,
    context: &'a Context<'a>,
) -> TextContext<'a> {
    let inherited = context.inherited;
    let mut chain: Vec<&ListStyle> = Vec::new();
    let mut anchor = None;
    let mut insets = TextInsets::default();
    let mut body = crate::placeholder::Body::default();
    if let Some(text) = &shape.text {
        body.inherit(text);
    }
    match &shape.placeholder {
        Some(placeholder) => {
            for found in crate::placeholder::inherited_shapes(shape, inherited) {
                let Some(text) = found.text.as_ref() else {
                    continue;
                };
                if let Some(style) = &text.list_style {
                    chain.push(style);
                }
                body.inherit(text);
                anchor = anchor.or(text.anchor);
                merge_insets(&mut insets, &text.insets);
            }
            if let Some(styles) = inherited
                .master
                .and_then(|master| master.text_styles.as_ref())
            {
                let parents = crate::placeholder::parents(placeholder, inherited);
                let role = parents[0]
                    .and_then(crate::placeholder::definition)
                    .map_or(placeholder, |(ph, _)| ph);
                chain.push(match master_kind(&role.kind) {
                    "title" => &styles.title,
                    "body" => &styles.body,
                    _ => &styles.other,
                });
            }
            if let Some(default) = inherited.default_text_style {
                chain.push(default);
            }
        }
        None => {
            if let Some(default) = inherited.default_text_style {
                chain.push(default);
            }
            if let Some(styles) = inherited
                .master
                .and_then(|master| master.text_styles.as_ref())
            {
                chain.push(&styles.other);
            }
        }
    }
    TextContext {
        theme: context.theme,
        fonts: inherited.theme_fonts,
        chain,
        font_color: crate::placeholder::visual(shape, inherited)
            .style
            .and_then(|style| style.font_color.as_ref()),
        anchor,
        insets,
        cell_defaults: RunProps::default(),
        wrap: body.wrap,
        autofit: body.autofit,
        vertical: body.vertical.flatten(),
        measure: context.measure,
    }
}

/// Table cells inherit like plain shapes: the presentation default, then the
/// master's other style.
pub(crate) fn text_context_for_table<'a>(context: &'a Context<'a>) -> TextContext<'a> {
    let inherited = context.inherited;
    let mut chain: Vec<&ListStyle> = Vec::new();
    if let Some(default) = inherited.default_text_style {
        chain.push(default);
    }
    if let Some(styles) = inherited
        .master
        .and_then(|master| master.text_styles.as_ref())
    {
        chain.push(&styles.other);
    }
    TextContext {
        theme: context.theme,
        fonts: inherited.theme_fonts,
        chain,
        font_color: None,
        anchor: None,
        insets: TextInsets::default(),
        cell_defaults: RunProps::default(),
        wrap: None,
        autofit: None,
        vertical: None,
        measure: context.measure,
    }
}

fn merge_insets(into: &mut TextInsets, from: &TextInsets) {
    into.left_emu = into.left_emu.or(from.left_emu);
    into.top_emu = into.top_emu.or(from.top_emu);
    into.right_emu = into.right_emu.or(from.right_emu);
    into.bottom_emu = into.bottom_emu.or(from.bottom_emu);
}

/// A paragraph with every inheritable property filled in as far as the chain
/// can. Still-`None` fields fall back to layout constants.
#[derive(Debug, Clone, Default)]
pub(crate) struct ResolvedParagraph {
    pub(crate) align: Option<jade_pptx_reader::TextAlign>,
    pub(crate) margin_left_emu: Option<i64>,

    pub(crate) margin_right_emu: Option<i64>,
    pub(crate) indent_emu: Option<i64>,
    pub(crate) bullet: Option<jade_pptx_reader::Bullet>,
    pub(crate) bullet_font: Option<String>,
    pub(crate) bullet_font_follow_text: bool,
    pub(crate) bullet_size_percent: Option<f32>,
    pub(crate) bullet_color: Option<Color>,
    pub(crate) line_spacing: Option<jade_pptx_reader::Spacing>,
    pub(crate) space_before: Option<jade_pptx_reader::Spacing>,
    pub(crate) space_after: Option<jade_pptx_reader::Spacing>,
    pub(crate) default_run: RunProps,
}

pub(crate) fn resolve_paragraph(
    paragraph: &TextParagraph,
    own: Option<&ListStyle>,
    context: &TextContext<'_>,
) -> ResolvedParagraph {
    let mut resolved = ResolvedParagraph {
        align: paragraph.align,
        margin_left_emu: paragraph.margin_left_emu,
        margin_right_emu: paragraph.margin_right_emu,
        indent_emu: paragraph.indent_emu,
        bullet: paragraph.bullet.clone(),
        bullet_font: paragraph.bullet_font.clone(),
        bullet_font_follow_text: paragraph.bullet_font_follow_text,
        bullet_size_percent: paragraph.bullet_size_percent,
        bullet_color: paragraph.bullet_color.clone(),
        line_spacing: paragraph.line_spacing,
        space_before: paragraph.space_before,
        space_after: paragraph.space_after,
        default_run: context.cell_defaults.clone(),
    };
    let level = paragraph.level as usize;
    // The paragraph's level first through the whole chain; then level 0 of
    // the chain for anything no style states at that level.
    let passes: &[usize] = if level == 0 { &[0] } else { &[level, 0] };
    for pass in passes {
        for style in own.into_iter().chain(context.chain.iter().copied()) {
            if let Some(level_style) = style.level(*pass) {
                merge_level(&mut resolved, level_style);
            }
        }
    }
    resolved
}

fn merge_level(into: &mut ResolvedParagraph, from: &LevelStyle) {
    into.align = into.align.or(from.align);
    into.margin_left_emu = into.margin_left_emu.or(from.margin_left_emu);
    into.margin_right_emu = into.margin_right_emu.or(from.margin_right_emu);
    into.indent_emu = into.indent_emu.or(from.indent_emu);
    if into.bullet.is_none() {
        into.bullet = from.bullet.clone();
    }
    if into.bullet_font.is_none() && !into.bullet_font_follow_text {
        into.bullet_font = from.bullet_font.clone();
        into.bullet_font_follow_text = from.bullet_font_follow_text;
    }
    into.bullet_size_percent = into.bullet_size_percent.or(from.bullet_size_percent);
    if into.bullet_color.is_none() {
        into.bullet_color = from.bullet_color.clone();
    }
    into.line_spacing = into.line_spacing.or(from.line_spacing);
    into.space_before = into.space_before.or(from.space_before);
    into.space_after = into.space_after.or(from.space_after);
    let run = &mut into.default_run;
    run.size_pt = run.size_pt.or(from.default_run.size_pt);
    run.bold = run.bold.or(from.default_run.bold);
    run.italic = run.italic.or(from.default_run.italic);
    run.underline = run.underline.or(from.default_run.underline);
    if run.color.is_none() {
        run.color = from.default_run.color.clone();
    }
    if run.font.is_none() {
        run.font = from.default_run.font.clone();
    }
    if run.east_asian_font.is_none() {
        run.east_asian_font = from.default_run.east_asian_font.clone();
    }
}

/// A run with nothing left unsaid.
#[derive(Debug, Clone)]
pub(crate) struct ResolvedRun {
    pub(crate) style: RunStyle,
    pub(crate) underline: bool,
    pub(crate) strike: bool,
    pub(crate) color: String,
    pub(crate) hyperlink: Option<String>,
    /// How far the baseline rises (negative: drops), in px.
    pub(crate) baseline_shift_px: f64,
    pub(crate) highlight: Option<String>,
    /// Extra advance after every character, in px.
    pub(crate) spacing_px: f64,
    pub(crate) outline: Option<crate::PlacedOutline>,
    pub(crate) shadow: Option<crate::PlacedShadow>,
    pub(crate) underline_style: Option<String>,
    pub(crate) double_strike: bool,
}

/// The paragraph's size when it has no runs, or for the bullet: the
/// resolved default, scaled by autofit.
pub(crate) fn paragraph_size_pt(
    paragraph: &TextParagraph,
    resolved: &ResolvedParagraph,
    font_scale: f32,
) -> f32 {
    paragraph
        .end_size_pt
        .or(resolved.default_run.size_pt)
        .unwrap_or(DEFAULT_SIZE_PT)
        * font_scale
}

pub(crate) fn resolve_run(
    run: &TextRun,
    resolved: &ResolvedParagraph,
    context: &TextContext<'_>,
    font_scale: f32,
) -> ResolvedRun {
    let defaults = &resolved.default_run;
    let full_size_pt = run.size_pt.or(defaults.size_pt).unwrap_or(DEFAULT_SIZE_PT) * font_scale;
    // Super- and subscripts draw at two thirds size, raised or dropped by
    // their baseline fraction of the full size.
    let baseline = run.baseline.unwrap_or(0.0);
    let size_pt = if baseline != 0.0 {
        full_size_pt * 2.0 / 3.0
    } else {
        full_size_pt
    };
    let bold = run.bold_stated.or(defaults.bold).unwrap_or(run.bold);
    let italic = run.italic_stated.or(defaults.italic).unwrap_or(run.italic);
    // Explicit run color wins over the theme's hyperlink default.
    let link_color = run
        .hyperlink
        .as_ref()
        .and_then(|_| context.theme.get("hlink"))
        .map(|hex| format!("#{hex}"));
    let color = run
        .color
        .as_ref()
        .map(|c| resolve_color(c, context.theme))
        .or(link_color)
        .unwrap_or_else(|| {
            defaults
                .color
                .as_ref()
                .or(context.font_color)
                .map(|color| resolve_color(color, context.theme))
                .unwrap_or_else(|| default_text_color(context.theme))
        });
    ResolvedRun {
        style: RunStyle {
            size_pt,
            bold,
            italic,
            font: resolve_face(
                run.font.as_deref().or(defaults.font.as_deref()),
                context.fonts,
            ),
            east_asian_font: resolve_face(
                run.east_asian_font
                    .as_deref()
                    .or(defaults.east_asian_font.as_deref()),
                context.fonts,
            ),
        },
        underline: run.underline_stated.unwrap_or_else(|| {
            run.underline || run.hyperlink.is_some() || defaults.underline.unwrap_or(false)
        }),
        strike: run.strike,
        color,
        hyperlink: run.hyperlink.as_ref().and_then(|link| link.target.clone()),
        baseline_shift_px: baseline as f64 * full_size_pt as f64 * crate::PX_PER_PT,
        highlight: run
            .highlight
            .as_ref()
            .map(|color| resolve_color(color, context.theme)),
        spacing_px: run.spacing_pt.unwrap_or(0.0) as f64 * crate::PX_PER_PT,
        outline: crate::resolve_outline(run.outline.as_ref(), None, context.theme, None),
        shadow: crate::place_shadow(run.shadow.as_ref(), context.theme),
        underline_style: run.underline_style.clone(),
        double_strike: run.double_strike,
    }
}

pub(crate) fn default_text_color(theme: &ThemeColors) -> String {
    theme
        .get("tx1")
        .map(|hex| format!("#{hex}"))
        .unwrap_or_else(|| "#000000".to_string())
}

/// A typeface name, with `+mj-lt`-style references resolved through the
/// theme. A reference the theme cannot resolve becomes `None` so the
/// renderer's default face applies rather than a literal "+mn-lt".
pub(crate) fn resolve_face(face: Option<&str>, fonts: Option<&ThemeFonts>) -> Option<String> {
    let face = face?;
    if face.starts_with('+') {
        fonts
            .and_then(|fonts| fonts.resolve(face))
            .map(str::to_string)
    } else {
        Some(face.to_string())
    }
}
