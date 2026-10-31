//! Standard SpreadsheetML conditional rules and differential styles.

use crate::{error::Result, BorderEdge, CellRange, Color, Fill};
use jade_opc::local_name;
use quick_xml::{
    events::{BytesStart, Event},
    name::ResolveResult,
    NsReader,
};
use serde::{Deserialize, Serialize};
mod x14;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConditionalFormatting {
    pub ranges: Vec<CellRange>,
    pub rules: Vec<ConditionalRule>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConditionalRule {
    pub rule_type: String,
    pub priority: u32,
    pub differential_style_index: Option<u32>,
    pub stop_if_true: bool,
    pub operator: Option<String>,
    pub text: Option<String>,
    pub formulas: Vec<String>,
    #[serde(default)]
    pub visual: Option<ConditionalVisual>,
    #[serde(default)]
    pub issues: Vec<String>,
    /// What `top10` and `aboveAverage` rules rank by.
    #[serde(default)]
    pub ranking: RuleRanking,
    /// A `timePeriod` rule's period: `today`, `last7Days`, `thisMonth`...
    #[serde(default)]
    pub time_period: Option<String>,
}

/// A `top10` or `aboveAverage` rule's settings; the defaults are what the
/// file leaves unsaid.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RuleRanking {
    /// `top10`: how many items, or what percent of them.
    pub rank: Option<u32>,
    pub percent: bool,
    pub bottom: bool,
    /// `aboveAverage`: below the average instead, counting the average
    /// itself when `equal_average`, or `std_dev` deviations away.
    pub below_average: bool,
    pub equal_average: bool,
    pub std_dev: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConditionalThreshold {
    pub kind: String,
    pub value: Option<String>,
    /// An icon set's threshold counts a value equal to it (`gte`, the
    /// default); without it only greater values reach the icon.
    #[serde(default = "default_true")]
    pub gte: bool,
}

fn default_true() -> bool {
    true
}

/// One icon of a custom (x14) icon set: the set it is borrowed from and its
/// position there, or none for `NoIcons`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomIcon {
    pub icon_set: String,
    pub index: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ConditionalVisual {
    ColorScale {
        thresholds: Vec<ConditionalThreshold>,
        colors: Vec<Color>,
    },
    DataBar {
        thresholds: Vec<ConditionalThreshold>,
        colors: Vec<Color>,
        min_length: u32,
        max_length: u32,
        show_value: bool,
        #[serde(default)]
        options: Option<Box<DataBarOptions>>,
    },
    /// `iconSet`: one icon per threshold band, the lowest band first.
    IconSet {
        /// `3TrafficLights1` when the file leaves it out.
        icon_set: String,
        thresholds: Vec<ConditionalThreshold>,
        reverse: bool,
        show_value: bool,
        /// x14 custom icons, one per threshold (lowest first); `None` is
        /// `NoIcons`. Empty for a set used as it is.
        #[serde(default)]
        custom: Vec<Option<CustomIcon>>,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataBarDirection {
    Context,
    LeftToRight,
    RightToLeft,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataBarAxisPosition {
    Automatic,
    Middle,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataBarOptions {
    pub linked_id: Option<String>,
    pub gradient: bool,
    pub border_color: Option<Color>,
    pub negative_color: Option<Color>,
    pub negative_border_color: Option<Color>,
    pub axis_color: Option<Color>,
    pub negative_same_color: bool,
    pub negative_same_border: bool,
    pub axis_position: DataBarAxisPosition,
    pub direction: DataBarDirection,
    pub context_right_to_left: bool,
}

impl ConditionalVisual {
    fn thresholds_mut(&mut self) -> &mut Vec<ConditionalThreshold> {
        match self {
            Self::ColorScale { thresholds, .. }
            | Self::DataBar { thresholds, .. }
            | Self::IconSet { thresholds, .. } => thresholds,
        }
    }
    fn colors_mut(&mut self) -> Option<&mut Vec<Color>> {
        match self {
            Self::ColorScale { colors, .. } | Self::DataBar { colors, .. } => Some(colors),
            Self::IconSet { .. } => None,
        }
    }
}

/// Omitted properties inherit from the cell; an explicit false flag clears it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DifferentialStyle {
    pub text_color: Option<Color>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub fill: Option<Fill>,
    pub left: Option<DifferentialBorder>,
    pub right: Option<DifferentialBorder>,
    pub top: Option<DifferentialBorder>,
    pub bottom: Option<DifferentialBorder>,
    /// A table style's borders between its cells (`<vertical>`,
    /// `<horizontal>`); conditional formats do not use them.
    #[serde(default)]
    pub inside_vertical: Option<DifferentialBorder>,
    #[serde(default)]
    pub inside_horizontal: Option<DifferentialBorder>,
    pub number_format_code: Option<String>,
    pub unsupported_properties: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DifferentialBorder {
    Clear,
    Edge(BorderEdge),
}

fn spreadsheet_namespace(namespace: ResolveResult<'_>) -> bool {
    match namespace {
        ResolveResult::Unbound => true,
        ResolveResult::Bound(namespace) => matches!(
            namespace.as_ref(),
            b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
        ),
        ResolveResult::Unknown(_) => false,
    }
}

fn attribute(element: &BytesStart<'_>, name: &[u8]) -> Result<Option<String>> {
    for value in element.attributes() {
        let value = value?;
        if value.key.as_ref() == name {
            return Ok(Some(value.unescape_value()?.into_owned()));
        }
    }
    Ok(None)
}

fn flag(value: Option<String>) -> bool {
    value.is_some_and(|value| value == "1" || value == "true")
}

fn enabled(value: Option<String>) -> bool {
    !value.is_some_and(|value| matches!(value.as_str(), "0" | "false" | "none"))
}

pub fn parse_conditional_formats(xml: &str) -> Result<Vec<ConditionalFormatting>> {
    if !xml.contains("conditionalFormatting") {
        return Ok(Vec::new());
    }
    let mut reader = NsReader::from_str(xml);
    let mut formats = Vec::new();
    let mut format: Option<ConditionalFormatting> = None;
    let mut rule: Option<ConditionalRule> = None;
    let mut formula: Option<String> = None;
    let mut stack: Vec<(bool, Vec<u8>)> = Vec::new();
    let mut format_depth = None;
    let mut rule_depth = None;
    let mut visual_depth = None;
    let mut formula_depth = None;
    let mut buf = Vec::new();
    loop {
        let (namespace, event) = reader.read_resolved_event_into(&mut buf)?;
        let main = spreadsheet_namespace(namespace);
        let parent = |name: &[u8]| {
            stack
                .last()
                .is_some_and(|(owned, local)| *owned && local == name)
        };
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let name = local_name(element.name().as_ref()).to_vec();
                if main {
                    match name.as_slice() {
                        b"conditionalFormatting" if stack.len() == 1 && parent(b"worksheet") => {
                            let ranges = attribute(element, b"sqref")?
                                .unwrap_or_default()
                                .split_ascii_whitespace()
                                .map(CellRange::parse)
                                .collect::<Result<Vec<_>>>()?;
                            if !empty {
                                format_depth = Some(stack.len() + 1);
                                format = Some(ConditionalFormatting {
                                    ranges,
                                    rules: Vec::new(),
                                });
                            }
                        }
                        b"cfRule"
                            if format_depth == Some(stack.len())
                                && parent(b"conditionalFormatting")
                                && format.is_some() =>
                        {
                            let parsed = ConditionalRule {
                                rule_type: attribute(element, b"type")?.unwrap_or_default(),
                                priority: attribute(element, b"priority")?
                                    .and_then(|value| value.parse().ok())
                                    .unwrap_or(u32::MAX),
                                differential_style_index: attribute(element, b"dxfId")?
                                    .and_then(|value| value.parse().ok()),
                                stop_if_true: attribute(element, b"stopIfTrue")?
                                    .is_some_and(|value| value == "1" || value == "true"),
                                operator: attribute(element, b"operator")?,
                                text: attribute(element, b"text")?,
                                formulas: Vec::new(),
                                visual: None,
                                issues: Vec::new(),
                                time_period: attribute(element, b"timePeriod")?,
                                ranking: RuleRanking {
                                    rank: attribute(element, b"rank")?
                                        .and_then(|value| value.parse().ok()),
                                    percent: flag(attribute(element, b"percent")?),
                                    bottom: flag(attribute(element, b"bottom")?),
                                    below_average: !enabled(attribute(element, b"aboveAverage")?),
                                    equal_average: flag(attribute(element, b"equalAverage")?),
                                    std_dev: attribute(element, b"stdDev")?
                                        .and_then(|value| value.parse().ok()),
                                },
                            };
                            if empty {
                                format.as_mut().unwrap().rules.push(parsed);
                            } else {
                                rule_depth = Some(stack.len() + 1);
                                rule = Some(parsed);
                            }
                        }
                        b"formula"
                            if rule_depth == Some(stack.len())
                                && parent(b"cfRule")
                                && rule.is_some() =>
                        {
                            if empty {
                                rule.as_mut().unwrap().formulas.push(String::new());
                            } else {
                                formula_depth = Some(stack.len() + 1);
                                formula = Some(String::new());
                            }
                        }
                        b"iconSet"
                            if rule_depth == Some(stack.len())
                                && parent(b"cfRule")
                                && rule.is_some() =>
                        {
                            if !empty {
                                visual_depth = Some(stack.len() + 1);
                            }
                            let rule = rule.as_mut().unwrap();
                            if rule.visual.is_some() {
                                rule.issues.push("multiple visual definitions".into());
                            }
                            rule.visual = Some(ConditionalVisual::IconSet {
                                icon_set: attribute(element, b"iconSet")?
                                    .unwrap_or_else(|| "3TrafficLights1".into()),
                                thresholds: Vec::new(),
                                reverse: flag(attribute(element, b"reverse")?),
                                show_value: enabled(attribute(element, b"showValue")?),
                                custom: Vec::new(),
                            });
                        }
                        b"colorScale" | b"dataBar"
                            if rule_depth == Some(stack.len())
                                && parent(b"cfRule")
                                && rule.is_some() =>
                        {
                            if !empty {
                                visual_depth = Some(stack.len() + 1);
                            }
                            let rule = rule.as_mut().unwrap();
                            if rule.visual.is_some() {
                                rule.issues.push("multiple visual definitions".into());
                            }
                            let thresholds = Vec::new();
                            let colors = Vec::new();
                            rule.visual = Some(if name == b"colorScale" {
                                ConditionalVisual::ColorScale { thresholds, colors }
                            } else {
                                let mut length = |key: &[u8], default| -> Result<u32> {
                                    Ok(match attribute(element, key)? {
                                        None => default,
                                        Some(value) => value.parse().unwrap_or_else(|_| {
                                            rule.issues.push("invalid data bar length".into());
                                            u32::MAX
                                        }),
                                    })
                                };
                                let min_length = length(b"minLength", 10)?;
                                let max_length = length(b"maxLength", 90)?;
                                let show_value = match attribute(element, b"showValue")?.as_deref()
                                {
                                    None | Some("1" | "true") => true,
                                    Some("0" | "false") => false,
                                    _ => {
                                        rule.issues.push("invalid showValue flag".into());
                                        true
                                    }
                                };
                                ConditionalVisual::DataBar {
                                    thresholds,
                                    colors,
                                    min_length,
                                    max_length,
                                    show_value,
                                    options: None,
                                }
                            });
                        }
                        b"cfvo"
                            if visual_depth == Some(stack.len())
                                && (parent(b"colorScale")
                                    || parent(b"dataBar")
                                    || parent(b"iconSet"))
                                && rule.is_some() =>
                        {
                            if let Some(visual) = rule.as_mut().unwrap().visual.as_mut() {
                                visual.thresholds_mut().push(ConditionalThreshold {
                                    kind: attribute(element, b"type")?.unwrap_or_default(),
                                    value: attribute(element, b"val")?,
                                    gte: enabled(attribute(element, b"gte")?),
                                });
                            }
                        }
                        b"color"
                            if visual_depth == Some(stack.len())
                                && (parent(b"colorScale") || parent(b"dataBar"))
                                && rule.is_some() =>
                        {
                            let rule = rule.as_mut().unwrap();
                            if attribute(element, b"tint")?.is_some_and(|value| {
                                value.parse::<f32>().map_or(true, |tint| {
                                    !tint.is_finite() || !(-1.0..=1.0).contains(&tint)
                                })
                            }) {
                                rule.issues.push("invalid visual color tint".into());
                            }
                            match crate::styles::parse_color(element)? {
                                Some(color) => {
                                    if let Some(colors) =
                                        rule.visual.as_mut().and_then(ConditionalVisual::colors_mut)
                                    {
                                        colors.push(color);
                                    }
                                }
                                None => rule.issues.push("invalid visual color".into()),
                            }
                        }
                        b"extLst"
                            if rule.is_some()
                                && ((rule_depth == Some(stack.len()) && parent(b"cfRule"))
                                    || (visual_depth == Some(stack.len())
                                        && (parent(b"colorScale") || parent(b"dataBar")))
                                    || (visual_depth == Some(stack.len().saturating_sub(1))
                                        && parent(b"cfvo"))) =>
                        {
                            rule.as_mut()
                                .unwrap()
                                .issues
                                .push("conditional formatting extensions are not supported".into());
                        }
                        _ => {}
                    }
                }
                if !empty {
                    stack.push((main, name));
                }
            }
            Event::Text(text)
                if formula_depth == Some(stack.len())
                    && parent(b"formula")
                    && formula.is_some() =>
            {
                formula.as_mut().unwrap().push_str(&text.decode()?)
            }
            Event::CData(text)
                if formula_depth == Some(stack.len())
                    && parent(b"formula")
                    && formula.is_some() =>
            {
                formula.as_mut().unwrap().push_str(&text.decode()?)
            }
            Event::GeneralRef(reference)
                if formula_depth == Some(stack.len())
                    && parent(b"formula")
                    && formula.is_some() =>
            {
                if let Some(character) = reference.resolve_char_ref()? {
                    formula.as_mut().unwrap().push(character);
                } else if let Some(value) =
                    quick_xml::escape::resolve_predefined_entity(&reference.decode()?)
                {
                    formula.as_mut().unwrap().push_str(value);
                }
            }
            Event::End(ref element) => {
                let qualified = element.name();
                let name = local_name(qualified.as_ref());
                if main
                    && stack
                        .last()
                        .is_some_and(|(owned, local)| *owned && local == name)
                {
                    match name {
                        b"formula" if formula_depth == Some(stack.len()) => {
                            if let (Some(rule), Some(formula)) = (rule.as_mut(), formula.take()) {
                                rule.formulas.push(formula);
                            }
                            formula_depth = None;
                        }
                        b"cfRule" if rule_depth == Some(stack.len()) => {
                            if let (Some(format), Some(rule)) = (format.as_mut(), rule.take()) {
                                format.rules.push(rule);
                            }
                            rule_depth = None;
                        }
                        b"conditionalFormatting" if format_depth == Some(stack.len()) => {
                            if let Some(format) = format.take() {
                                formats.push(format);
                            }
                            format_depth = None;
                        }
                        b"colorScale" | b"dataBar" | b"iconSet"
                            if visual_depth == Some(stack.len()) =>
                        {
                            visual_depth = None;
                        }
                        _ => {}
                    }
                }
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    x14::reconcile(xml, &mut formats)?;
    Ok(formats)
}

pub(crate) fn apply_theme(
    formats: &mut [ConditionalFormatting],
    theme: &crate::theme::ThemePalette,
) {
    for rule in formats.iter_mut().flat_map(|format| &mut format.rules) {
        if let Some(visual) = rule.visual.as_mut() {
            if let ConditionalVisual::DataBar {
                options: Some(options),
                ..
            } = visual
            {
                for color in [
                    &mut options.border_color,
                    &mut options.negative_color,
                    &mut options.negative_border_color,
                    &mut options.axis_color,
                ]
                .into_iter()
                .flatten()
                {
                    if let Color::Theme {
                        index, resolved, ..
                    } = color
                    {
                        *resolved = theme.color(*index);
                    }
                }
            }
            for color in visual.colors_mut().into_iter().flatten() {
                if let Color::Theme {
                    index, resolved, ..
                } = color
                {
                    *resolved = theme.color(*index);
                }
            }
        }
    }
}

pub fn parse_differential_styles(xml: &str) -> Result<Vec<DifferentialStyle>> {
    let mut reader = NsReader::from_str(xml);
    let mut styles = Vec::new();
    let mut in_dxfs = false;
    let mut font = false;
    let mut fill = false;
    let mut border = false;
    let mut edge: Option<(String, Option<BorderEdge>)> = None;
    let mut style: Option<DifferentialStyle> = None;
    let mut buf = Vec::new();
    loop {
        let (namespace, event) = reader.read_resolved_event_into(&mut buf)?;
        let main = spreadsheet_namespace(namespace);
        match event {
            Event::Start(ref element) | Event::Empty(ref element) if main => {
                let empty = matches!(event, Event::Empty(_));
                let name = local_name(element.name().as_ref()).to_vec();
                if name == b"dxfs" {
                    in_dxfs = !empty;
                } else if name == b"dxf" && in_dxfs {
                    font = false;
                    fill = false;
                    border = false;
                    edge = None;
                    if empty {
                        styles.push(DifferentialStyle::default());
                    } else {
                        style = Some(DifferentialStyle::default());
                    }
                } else if let Some(style) = style.as_mut() {
                    match name.as_slice() {
                        b"font" => font = !empty,
                        b"b" if font => style.bold = Some(enabled(attribute(element, b"val")?)),
                        b"i" if font => style.italic = Some(enabled(attribute(element, b"val")?)),
                        b"u" if font => {
                            let value = attribute(element, b"val")?;
                            if value.as_deref().is_some_and(|value| {
                                !matches!(value, "single" | "none" | "0" | "1" | "true" | "false")
                            }) {
                                style
                                    .unsupported_properties
                                    .push("underline variant".into());
                            }
                            style.underline = Some(enabled(value));
                        }
                        b"strike" if font => {
                            style.strike = Some(enabled(attribute(element, b"val")?))
                        }
                        b"color" if font => style.text_color = crate::styles::parse_color(element)?,
                        b"fill" => {
                            fill = !empty;
                            style.fill = Some(Fill::default());
                        }
                        b"patternFill" if fill => {
                            style.fill.as_mut().unwrap().pattern =
                                attribute(element, b"patternType")?
                        }
                        b"fgColor" if fill => {
                            style.fill.as_mut().unwrap().foreground =
                                crate::styles::parse_color(element)?
                        }
                        b"bgColor" if fill => {
                            style.fill.as_mut().unwrap().background =
                                crate::styles::parse_color(element)?
                        }
                        b"border" => border = !empty,
                        b"left" | b"right" | b"top" | b"bottom" | b"vertical" | b"horizontal"
                            if border =>
                        {
                            let name = String::from_utf8_lossy(&name).into_owned();
                            let parsed = attribute(element, b"style")?
                                .map(|style| BorderEdge { style, color: None });
                            if empty {
                                set_edge(style, &name, parsed);
                            } else {
                                edge = Some((name, parsed));
                            }
                        }
                        b"color" if edge.is_some() => {
                            if let Some((_, Some(edge))) = edge.as_mut() {
                                edge.color = crate::styles::parse_color(element)?;
                            }
                        }
                        b"numFmt" => style.number_format_code = attribute(element, b"formatCode")?,
                        b"gradientFill" => {
                            style.fill = None;
                            style.unsupported_properties.push("gradientFill".into());
                        }
                        b"name" | b"sz" | b"alignment" | b"protection" | b"diagonal" => style
                            .unsupported_properties
                            .push(String::from_utf8_lossy(&name).into_owned()),
                        _ => {}
                    }
                }
            }
            Event::End(ref element) if main => match local_name(element.name().as_ref()) {
                b"dxfs" => in_dxfs = false,
                b"dxf" => {
                    if let Some(style) = style.take() {
                        styles.push(style);
                    }
                }
                b"font" => font = false,
                b"fill" => fill = false,
                b"border" => border = false,
                b"left" | b"right" | b"top" | b"bottom" | b"vertical" | b"horizontal" => {
                    if let (Some(style), Some((name, edge))) = (style.as_mut(), edge.take()) {
                        set_edge(style, &name, edge);
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(styles)
}

fn set_edge(style: &mut DifferentialStyle, side: &str, edge: Option<BorderEdge>) {
    let edge = Some(
        edge.map(DifferentialBorder::Edge)
            .unwrap_or(DifferentialBorder::Clear),
    );
    match side {
        "left" => style.left = edge,
        "right" => style.right = edge,
        "top" => style.top = edge,
        "bottom" => style.bottom = edge,
        "vertical" => style.inside_vertical = edge,
        "horizontal" => style.inside_horizontal = edge,
        _ => {}
    }
}
