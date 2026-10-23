//! Word's フォント dialog (⌘D) applied to a range: every character property
//! a [`HeadlessRunFormat`] patch sets — fonts per script, size, style,
//! underline and its colour, strikes, effects, emphasis marks, theme colours,
//! scale, spacing, position, kerning, shading, border, the OpenType features
//! and East Asian layout — as one undoable step.
//!
//! Properties the writer builds from the model (`w:b`, `w:u`, `w:color`, …)
//! are set on the model. The ones it carries as raw XML (`w:position`,
//! `w:spacing`, `w:em`, `w:shd`, `w:bdr`, `w:eastAsianLayout` and the `w14`
//! extensions) are set on the model for layout *and* replaced in the run's
//! preserved `w:rPr` children, which the writer emits when it rebuilds the
//! run's properties.

use std::rc::Rc;

use jade_docx_headless_api::{HeadlessEastAsianLayout, HeadlessRunFormat, HeadlessThemeColor};

use super::run_backup::{capture_paragraph_runs, restore_paragraph_runs, ParagraphRunsBackup};
use super::text_edit::{apply_run_properties, mutate_run_properties};
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{
    BorderEdge, EastAsianLayout, Paragraph, RunFonts, RunProperties, RunThemeColor, Shading,
};
use crate::docx_xml::{XmlAttribute, XmlElement, XmlNode, XmlNodeId};
use jade_docx_layout::selection::SelectionRange;

/// Applies a [`HeadlessRunFormat`] patch to the selected runs.
pub struct SetRunFormatCommand {
    range: SelectionRange,
    format: HeadlessRunFormat,
    previous_runs: Option<Vec<ParagraphRunsBackup>>,
    previous_selection: Option<SelectionRange>,
}

impl SetRunFormatCommand {
    pub fn new(range: SelectionRange, format: HeadlessRunFormat) -> Self {
        Self {
            range,
            format,
            previous_runs: None,
            previous_selection: None,
        }
    }
}

impl Command for SetRunFormatCommand {
    fn name(&self) -> &'static str {
        "set_run_format"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if self.range.is_collapsed() {
            return Err(CommandError::InvalidSelection);
        }
        let (start, end) = self.range.normalized();
        let paragraph_count = ctx.document.paragraph_count();
        if paragraph_count == 0
            || start.paragraph >= paragraph_count
            || end.paragraph >= paragraph_count
        {
            return Err(CommandError::ParagraphUnavailable);
        }
        self.previous_runs = Some(capture_paragraph_runs(ctx, start.paragraph, end.paragraph)?);
        self.previous_selection = *ctx.selection;

        let font_props = font_properties(&self.format);
        // One id for the range's group, as Word gives one combined or
        // vertical group one.
        let layout_id = (start.paragraph as i64) * 65_536 + start.offset as i64 + 1;
        let format = &self.format;

        for paragraph_index in start.paragraph..=end.paragraph {
            let zero_width = ctx.document.zero_width_runs(paragraph_index);
            let paragraph = ctx
                .document
                .paragraph_mut(paragraph_index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            let paragraph_len = paragraph_len(paragraph);
            let range_start = if paragraph_index == start.paragraph {
                start.offset.min(paragraph_len)
            } else {
                0
            };
            let range_end = if paragraph_index == end.paragraph {
                end.offset.min(paragraph_len)
            } else {
                paragraph_len
            };
            if range_start >= range_end {
                continue;
            }
            let mut restyled = Vec::new();
            if let Some(props) = font_props.as_ref() {
                // Merged, so the font properties are recorded as the run's
                // own rather than as inherited from its style.
                restyled.extend(apply_run_properties(
                    &mut paragraph.runs,
                    range_start,
                    range_end,
                    props,
                    &zero_width,
                ));
            }
            restyled.extend(mutate_run_properties(
                &mut paragraph.runs,
                range_start,
                range_end,
                &zero_width,
                |props| apply_patch(props, format, layout_id),
            ));
            restyled.sort_by_key(|id| id.raw());
            restyled.dedup();
            ctx.document
                .sync_paragraph_run_properties(paragraph_index, &restyled);
        }
        *ctx.selection = Some(self.range);
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let backups = self
            .previous_runs
            .take()
            .ok_or_else(|| CommandError::Custom("missing backup for undo".into()))?;
        restore_paragraph_runs(ctx, &backups)?;
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

fn paragraph_len(paragraph: &Paragraph) -> usize {
    paragraph.runs.iter().map(|run| run.text.len()).sum()
}

fn non_empty(value: &Option<String>) -> Option<Rc<str>> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(Rc::from)
}

/// The font properties of the patch, merged so they count as direct.
fn font_properties(format: &HeadlessRunFormat) -> Option<RunProperties> {
    let latin = non_empty(&format.latin_font);
    let east_asia = non_empty(&format.east_asia_font);
    let complex = non_empty(&format.complex_font);
    let fonts = (latin.is_some() || east_asia.is_some() || complex.is_some()).then(|| RunFonts {
        ascii: latin.clone(),
        h_ansi: latin.clone(),
        east_asia: east_asia.clone(),
        complex: complex.clone(),
        ..RunFonts::default()
    });
    let props = RunProperties {
        font_family: latin.or(east_asia),
        fonts,
        font_size: format
            .font_size
            .filter(|size| size.is_finite() && *size > 0.0),
        font_size_complex: format
            .complex_font_size
            .filter(|size| size.is_finite() && *size > 0.0),
        kerning: format
            .kerning
            .filter(|points| points.is_finite())
            .map(|points| points.max(0.0)),
        text_scale: format
            .scale
            .filter(|percent| percent.is_finite())
            .map(|percent| percent.clamp(1.0, 600.0)),
        ..RunProperties::default()
    };
    (!props.is_empty()).then_some(props)
}

/// The same normalization for story XML patches as for body commands.
pub(crate) fn apply_to_properties(
    properties: &mut RunProperties,
    format: &HeadlessRunFormat,
    layout_id: i64,
) {
    if let Some(fonts) = font_properties(format) {
        super::text_edit::merge_run_properties(properties, &fonts);
    }
    apply_patch(properties, format, layout_id);
}

/// A preserved `w:rPr` child built from scratch; the writer gives it a real
/// id when it emits it.
fn element(name: &str, attributes: &[(&str, String)], children: Vec<XmlNode>) -> XmlNode {
    XmlNode::Element(XmlElement {
        id: XmlNodeId::new(0),
        name: name.to_string(),
        attributes: attributes
            .iter()
            .map(|(name, value)| XmlAttribute {
                name: name.to_string(),
                value: value.clone(),
            })
            .collect(),
        children,
    })
}

/// Replaces (or with `None` removes) the preserved child named `name`.
fn set_preserved(props: &mut RunProperties, name: &str, node: Option<XmlNode>) {
    let children = &mut props.preserved_rpr.0;
    children.retain(|child| !matches!(child, XmlNode::Element(element) if element.name == name));
    if let Some(node) = node {
        children.push(node);
    }
}

fn hex(color: &str) -> String {
    color.trim().trim_start_matches('#').to_ascii_uppercase()
}

fn is_none(value: &str) -> bool {
    matches!(value.trim(), "" | "none" | "nil")
}

fn on(value: bool) -> Option<bool> {
    value.then_some(true)
}

/// The patch's properties on one run.
fn apply_patch(props: &mut RunProperties, format: &HeadlessRunFormat, layout_id: i64) {
    if let Some(latin) = non_empty(&format.latin_font) {
        if let Some(fonts) = props.fonts.as_mut() {
            fonts.ascii = Some(latin.clone());
            fonts.h_ansi = Some(latin.clone());
            fonts.ascii_theme = None;
            fonts.h_ansi_theme = None;
        }
        if let Some(fonts) = props.direct_font_properties.fonts.as_mut() {
            fonts.ascii_theme = None;
            fonts.h_ansi_theme = None;
        }
    }
    if non_empty(&format.east_asia_font).is_some() {
        for fonts in [
            props.fonts.as_mut(),
            props.direct_font_properties.fonts.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            fonts.east_asia_theme = None;
        }
    }
    if non_empty(&format.complex_font).is_some() {
        for fonts in [
            props.fonts.as_mut(),
            props.direct_font_properties.fonts.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            fonts.complex_theme = None;
        }
    }
    if let Some(value) = format.bold {
        props.bold = Some(value);
    }
    if let Some(value) = format.italic {
        props.italic = Some(value);
    }
    if let Some(value) = format.complex_bold {
        props.bold_complex = Some(value);
    }
    if let Some(value) = format.complex_italic {
        props.italic_complex = Some(value);
    }
    if let Some(kind) = format.underline.as_deref() {
        if is_none(kind) {
            props.underline = Some(false);
            props.underline_kind = None;
            props.underline_color = None;
        } else {
            props.underline = Some(true);
            props.underline_kind = (kind != "single").then(|| Rc::from(kind));
        }
    }
    if let Some(color) = format.underline_color.as_deref() {
        props.underline_color = if color.trim().eq_ignore_ascii_case("auto") || is_none(color) {
            None
        } else {
            Some(Rc::from(hex(color).as_str()))
        };
    }
    if let Some(value) = format.strike {
        props.strike = on(value);
        if value {
            props.double_strike = None;
        }
    }
    if let Some(value) = format.double_strike {
        props.double_strike = on(value);
        if value {
            props.strike = None;
        }
    }
    if let Some(align) = format.vertical_align.as_deref() {
        props.baseline = match align {
            "superscript" | "subscript" => Some(Rc::from(align)),
            _ => None,
        };
    }
    if let Some(color) = format.color.as_deref() {
        props.color = Some(if color.trim().eq_ignore_ascii_case("auto") {
            Rc::from("#auto")
        } else {
            Rc::from(format!("#{}", hex(color)).as_str())
        });
        props.theme_color = format
            .theme_color
            .as_ref()
            .filter(|theme| !theme.name.trim().is_empty())
            .map(|theme| Box::new(theme_color(theme)));
    }
    if let Some(value) = format.caps {
        props.caps = on(value);
        if value {
            props.small_caps = None;
        }
    }
    if let Some(value) = format.small_caps {
        props.small_caps = on(value);
        if value {
            props.caps = None;
        }
    }
    if let Some(value) = format.hidden {
        props.vanish = on(value);
    }
    if format.language.is_some() || format.east_asia_language.is_some() {
        let current = props.preserved_rpr.0.iter().find_map(|child| match child {
            XmlNode::Element(element) if element.name == "w:lang" => Some(element.clone()),
            _ => None,
        });
        let attribute = |name: &str| {
            current.as_ref().and_then(|element| {
                element
                    .attributes
                    .iter()
                    .find(|attribute| attribute.name == name)
                    .map(|attribute| attribute.value.clone())
            })
        };
        let latin = match &format.language {
            Some(value) if is_none(value) => None,
            Some(value) => Some(value.trim().to_string()),
            None => attribute("w:val"),
        };
        let east_asia = match &format.east_asia_language {
            Some(value) if is_none(value) => None,
            Some(value) => Some(value.trim().to_string()),
            None => attribute("w:eastAsia"),
        };
        let bidi = attribute("w:bidi");
        props.language = latin.as_deref().map(Rc::from);
        props.east_asia_language = east_asia.as_deref().map(Rc::from);
        let mut attributes = Vec::new();
        if let Some(latin) = latin {
            attributes.push(("w:val", latin));
        }
        if let Some(east_asia) = east_asia {
            attributes.push(("w:eastAsia", east_asia));
        }
        if let Some(bidi) = bidi {
            attributes.push(("w:bidi", bidi));
        }
        set_preserved(
            props,
            "w:lang",
            (!attributes.is_empty()).then(|| element("w:lang", &attributes, Vec::new())),
        );
    }
    if let Some(value) = format.no_proof {
        // Off is written as off rather than left out, so a w:noProof the
        // run was read with follows it.
        let attributes: &[(&str, String)] = if value {
            &[]
        } else {
            &[("w:val", "0".to_string())]
        };
        set_preserved(
            props,
            "w:noProof",
            Some(element("w:noProof", attributes, Vec::new())),
        );
    }
    if let Some(value) = format.outline {
        props.outline = on(value);
    }
    if let Some(value) = format.shadow {
        props.shadow = on(value);
    }
    if let Some(value) = format.emboss {
        props.emboss = on(value);
        if value {
            props.imprint = None;
        }
    }
    if let Some(value) = format.engrave {
        props.imprint = on(value);
        if value {
            props.emboss = None;
        }
    }
    if let Some(mark) = format.emphasis.as_deref() {
        if is_none(mark) {
            props.emphasis = None;
            set_preserved(props, "w:em", None);
        } else {
            let token = match mark.to_ascii_lowercase().as_str() {
                "comma" => "comma",
                "circle" => "circle",
                "underdot" => "underDot",
                _ => "dot",
            };
            props.emphasis = Some(Rc::from(token.to_ascii_lowercase().as_str()));
            set_preserved(
                props,
                "w:em",
                Some(element("w:em", &[("w:val", token.to_string())], Vec::new())),
            );
        }
    }
    if let Some(fill) = format.shading.as_deref() {
        if is_none(fill) {
            props.shading = None;
            set_preserved(props, "w:shd", None);
        } else {
            let fill = hex(fill);
            props.shading = Some(Shading {
                val: Some("clear".to_string()),
                color: None,
                fill: Some(format!("#{fill}")),
            });
            set_preserved(
                props,
                "w:shd",
                Some(element(
                    "w:shd",
                    &[
                        ("w:val", "clear".to_string()),
                        ("w:color", "auto".to_string()),
                        ("w:fill", fill),
                    ],
                    Vec::new(),
                )),
            );
        }
    }
    if let Some(boxed) = format.border {
        if boxed {
            props.border = Some(BorderEdge {
                val: Some("single".to_string()),
                size_eighths: Some(4),
                space: Some(0),
                color: None,
            });
            set_preserved(
                props,
                "w:bdr",
                Some(element(
                    "w:bdr",
                    &[
                        ("w:val", "single".to_string()),
                        ("w:sz", "4".to_string()),
                        ("w:space", "0".to_string()),
                        ("w:color", "auto".to_string()),
                    ],
                    Vec::new(),
                )),
            );
        } else {
            props.border = None;
            set_preserved(props, "w:bdr", None);
        }
    }
    if let Some(points) = format.spacing.filter(|points| points.is_finite()) {
        let twips = (points * 20.0).round() as i32;
        if twips == 0 {
            props.char_spacing = None;
            set_preserved(props, "w:spacing", None);
        } else {
            props.char_spacing = Some(twips as f32 / 20.0);
            set_preserved(
                props,
                "w:spacing",
                Some(element(
                    "w:spacing",
                    &[("w:val", twips.to_string())],
                    Vec::new(),
                )),
            );
        }
    }
    if let Some(points) = format.position.filter(|points| points.is_finite()) {
        let half_points = (points * 2.0).round() as i32;
        if half_points == 0 {
            props.text_position = None;
            set_preserved(props, "w:position", None);
        } else {
            props.text_position = Some(half_points as f32 / 2.0);
            set_preserved(
                props,
                "w:position",
                Some(element(
                    "w:position",
                    &[("w:val", half_points.to_string())],
                    Vec::new(),
                )),
            );
        }
    }
    if let Some(layout) = format.east_asian_layout.as_ref() {
        apply_east_asian_layout(props, layout, layout_id);
    }
    apply_text_effects(props, format);
}

pub(crate) fn theme_color(theme: &HeadlessThemeColor) -> RunThemeColor {
    let byte = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| Rc::from(value.to_ascii_uppercase().as_str()))
    };
    RunThemeColor {
        name: Rc::from(theme.name.trim()),
        tint: byte(&theme.tint),
        shade: byte(&theme.shade),
    }
}

fn apply_east_asian_layout(props: &mut RunProperties, layout: &HeadlessEastAsianLayout, id: i64) {
    if layout.is_empty() {
        props.east_asian_layout = None;
        set_preserved(props, "w:eastAsianLayout", None);
        return;
    }
    let brackets = layout
        .combine_brackets
        .as_deref()
        .filter(|brackets| layout.combine && !is_none(brackets))
        .map(Rc::from);
    let model = EastAsianLayout {
        id: Some(id),
        combine: layout.combine,
        combine_brackets: brackets.clone(),
        vert: layout.vert,
        vert_compress: layout.vert && layout.vert_compress,
    };
    let mut attributes = vec![("w:id", id.to_string())];
    if model.combine {
        attributes.push(("w:combine", "1".to_string()));
    }
    if let Some(brackets) = brackets {
        attributes.push(("w:combineBrackets", brackets.to_string()));
    }
    if model.vert {
        attributes.push(("w:vert", "1".to_string()));
    }
    if model.vert_compress {
        attributes.push(("w:vertCompress", "1".to_string()));
    }
    props.east_asian_layout = Some(Box::new(model));
    set_preserved(
        props,
        "w:eastAsianLayout",
        Some(element("w:eastAsianLayout", &attributes, Vec::new())),
    );
}

/// EMU per point.
const EMU_PER_POINT: f32 = 12_700.0;

fn srgb(color: &str, children: Vec<XmlNode>) -> XmlNode {
    element("w14:srgbClr", &[("w14:val", hex(color))], children)
}

fn apply_text_effects(props: &mut RunProperties, format: &HeadlessRunFormat) {
    let mut effects = props.text_effects.as_deref().cloned().unwrap_or_default();
    let before = effects.clone();
    let token = |value: &str, default: &str| -> Option<Rc<str>> {
        (!is_none(value) && value != default).then(|| Rc::from(value))
    };
    if let Some(value) = format.ligatures.as_deref() {
        effects.ligatures = token(value, "");
        set_preserved(
            props,
            "w14:ligatures",
            effects
                .ligatures
                .as_ref()
                .map(|value| element("w14:ligatures", &[("w14:val", value.to_string())], vec![])),
        );
    }
    if let Some(value) = format.number_form.as_deref() {
        effects.number_form = token(value, "default");
        set_preserved(
            props,
            "w14:numForm",
            effects
                .number_form
                .as_ref()
                .map(|value| element("w14:numForm", &[("w14:val", value.to_string())], vec![])),
        );
    }
    if let Some(value) = format.number_spacing.as_deref() {
        effects.number_spacing = token(value, "default");
        set_preserved(
            props,
            "w14:numSpacing",
            effects
                .number_spacing
                .as_ref()
                .map(|value| element("w14:numSpacing", &[("w14:val", value.to_string())], vec![])),
        );
    }
    if let Some(set) = format.stylistic_set {
        effects.stylistic_set = (1..=20).contains(&set).then_some(set);
        set_preserved(
            props,
            "w14:stylisticSets",
            effects.stylistic_set.map(|set| {
                element(
                    "w14:stylisticSets",
                    &[],
                    vec![element(
                        "w14:styleSet",
                        &[("w14:id", set.to_string())],
                        vec![],
                    )],
                )
            }),
        );
    }
    if let Some(value) = format.contextual_alternates {
        effects.contextual_alternates = on(value);
        set_preserved(
            props,
            "w14:cntxtAlts",
            on(value).map(|_| element("w14:cntxtAlts", &[], vec![])),
        );
    }
    if format.glow_color.is_some() || format.glow_radius.is_some() {
        if let Some(color) = format.glow_color.as_deref() {
            effects.glow_color =
                (!is_none(color)).then(|| Rc::from(format!("#{}", hex(color)).as_str()));
        }
        if let Some(radius) = format.glow_radius.filter(|radius| radius.is_finite()) {
            effects.glow_radius = Some(radius.clamp(0.0, 150.0));
        }
        let node = effects.glow_color.as_ref().map(|color| {
            let radius = effects.glow_radius.unwrap_or(5.0);
            let alpha = ((1.0 - effects.glow_alpha.unwrap_or(0.6)) * 100_000.0).round() as i32;
            element(
                "w14:glow",
                &[(
                    "w14:rad",
                    ((radius * EMU_PER_POINT).round() as i64).to_string(),
                )],
                vec![srgb(
                    color,
                    vec![element(
                        "w14:alpha",
                        &[("w14:val", alpha.to_string())],
                        vec![],
                    )],
                )],
            )
        });
        if node.is_none() {
            effects.glow_radius = None;
            effects.glow_alpha = None;
        }
        set_preserved(props, "w14:glow", node);
    }
    if format.text_outline_color.is_some() || format.text_outline_width.is_some() {
        if let Some(color) = format.text_outline_color.as_deref() {
            effects.outline_color =
                (!is_none(color)).then(|| Rc::from(format!("#{}", hex(color)).as_str()));
        }
        if let Some(width) = format.text_outline_width.filter(|width| width.is_finite()) {
            effects.outline_width = Some(width.clamp(0.0, 20.0));
        }
        let node = effects.outline_color.as_ref().map(|color| {
            let width = effects.outline_width.unwrap_or(0.75);
            element(
                "w14:textOutline",
                &[
                    (
                        "w14:w",
                        ((width * EMU_PER_POINT).round() as i64).to_string(),
                    ),
                    ("w14:cap", "flat".to_string()),
                    ("w14:cmpd", "sng".to_string()),
                    ("w14:algn", "ctr".to_string()),
                ],
                vec![
                    element("w14:solidFill", &[], vec![srgb(color, vec![])]),
                    element("w14:prstDash", &[("w14:val", "solid".to_string())], vec![]),
                    element("w14:round", &[], vec![]),
                ],
            )
        });
        if node.is_none() {
            effects.outline_width = None;
        }
        set_preserved(props, "w14:textOutline", node);
    }
    if effects != before {
        props.text_effects = (!effects.is_empty()).then(|| Box::new(effects));
    }
}

/// What a run has, as the フォント dialog shows it.
pub fn run_format_of(props: &RunProperties) -> HeadlessRunFormat {
    let fonts = props.fonts.as_ref();
    let text = |value: &Option<Rc<str>>| value.as_deref().map(str::to_string);
    let color = props.color.as_deref().map(|color| {
        let bare = color.trim_start_matches('#');
        if bare.eq_ignore_ascii_case("auto") {
            "auto".to_string()
        } else {
            bare.to_ascii_uppercase()
        }
    });
    let effects = props.text_effects.as_deref();
    HeadlessRunFormat {
        latin_font: fonts
            .and_then(RunFonts::preferred_latin)
            .map(str::to_string)
            .or_else(|| text(&props.font_family)),
        east_asia_font: fonts
            .and_then(RunFonts::preferred_east_asia)
            .map(str::to_string),
        complex_font: fonts
            .and_then(RunFonts::preferred_complex)
            .map(str::to_string),
        font_size: props.font_size,
        complex_font_size: props.font_size_complex.or(props.font_size),
        bold: props.bold,
        italic: props.italic,
        complex_bold: props.bold_complex,
        complex_italic: props.italic_complex,
        underline: match props.underline {
            Some(true) => Some(
                props
                    .underline_kind
                    .as_deref()
                    .filter(|kind| !kind.is_empty())
                    .unwrap_or("single")
                    .to_string(),
            ),
            _ => None,
        },
        underline_color: props
            .underline_color
            .as_deref()
            .map(|color| color.trim_start_matches('#').to_string()),
        strike: props.strike,
        double_strike: props.double_strike,
        vertical_align: text(&props.baseline),
        color,
        theme_color: props.theme_color.as_ref().map(|theme| HeadlessThemeColor {
            name: theme.name.to_string(),
            tint: text(&theme.tint),
            shade: text(&theme.shade),
        }),
        caps: props.caps,
        small_caps: props.small_caps,
        hidden: props.vanish,
        language: text(&props.language),
        east_asia_language: text(&props.east_asia_language),
        no_proof: props.preserved_rpr.0.iter().find_map(|child| match child {
            XmlNode::Element(element) if element.name == "w:noProof" => {
                Some(!element.attributes.iter().any(|attribute| {
                    attribute.name == "w:val"
                        && matches!(attribute.value.as_str(), "0" | "false" | "off")
                }))
            }
            _ => None,
        }),
        emphasis: props.emphasis.as_deref().map(|mark| match mark {
            "underdot" => "underDot".to_string(),
            other => other.to_string(),
        }),
        shading: props
            .shading
            .as_ref()
            .and_then(|shading| shading.fill.as_deref().or(shading.color.as_deref()))
            .map(|fill| fill.trim_start_matches('#').to_string()),
        border: props.border.as_ref().map(BorderEdge::is_visible),
        scale: props.text_scale,
        spacing: props.char_spacing,
        position: props.text_position,
        kerning: props.kerning,
        outline: props.outline,
        shadow: props.shadow,
        emboss: props.emboss,
        engrave: props.imprint,
        ligatures: effects.and_then(|effects| text(&effects.ligatures)),
        number_spacing: effects.and_then(|effects| text(&effects.number_spacing)),
        number_form: effects.and_then(|effects| text(&effects.number_form)),
        stylistic_set: effects.and_then(|effects| effects.stylistic_set),
        contextual_alternates: effects.and_then(|effects| effects.contextual_alternates),
        glow_color: effects
            .and_then(|effects| effects.glow_color.as_deref())
            .map(|color| color.trim_start_matches('#').to_string()),
        glow_radius: effects.and_then(|effects| effects.glow_radius),
        text_outline_color: effects
            .and_then(|effects| effects.outline_color.as_deref())
            .map(|color| color.trim_start_matches('#').to_string()),
        text_outline_width: effects.and_then(|effects| effects.outline_width),
        east_asian_layout: props
            .east_asian_layout
            .as_ref()
            .map(|layout| HeadlessEastAsianLayout {
                combine: layout.combine,
                combine_brackets: text(&layout.combine_brackets),
                vert: layout.vert,
                vert_compress: layout.vert_compress,
            }),
    }
}
