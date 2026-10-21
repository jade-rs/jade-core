//! Numeric conditional visual rules prepared from stored cells, not grid area.
use jade_xlsx_reader::{
    conditional_format::{
        ConditionalRule, ConditionalThreshold, ConditionalVisual, DataBarAxisPosition,
        DataBarDirection, DataBarOptions,
    },
    CellValue, Color,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedDataBar {
    pub origin: f64,
    pub fraction: f64,
    pub gradient: bool,
    pub reverse: bool,
    pub border: Option<[u8; 4]>,
    pub axis: Option<(f64, [u8; 4])>,
    pub color: [u8; 4],
    pub show_value: bool,
}

/// An icon-set icon chosen for one cell: the set it is drawn from and its
/// position there, 0 being the icon for the lowest values.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedIcon {
    pub icon_set: String,
    pub index: u32,
    pub show_value: bool,
}

/// What a visual rule paints for one cell.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct VisualOutcome {
    pub fill: Option<Color>,
    pub bar: Option<ResolvedDataBar>,
    pub icon: Option<ResolvedIcon>,
}

/// How many icons a built-in set has: the digit its name starts with.
pub fn icon_count(icon_set: &str) -> Option<usize> {
    let count = icon_set.chars().next()?.to_digit(10)? as usize;
    (3..=5).contains(&count).then_some(count)
}

pub(super) enum PreparedVisual {
    IconSet {
        points: Vec<f64>,
        gte: Vec<bool>,
        /// The icon for each band, lowest band first; `None` draws nothing.
        icons: Vec<Option<(String, u32)>>,
        show_value: bool,
    },
    ColorScale {
        points: Vec<f64>,
        colors: Vec<[u8; 4]>,
    },
    DataBar {
        points: Vec<f64>,
        color: [u8; 4],
        min: f64,
        max: f64,
        show_value: bool,
        modern: Option<Box<ModernBar>>,
    },
}

pub(super) struct ModernBar {
    options: DataBarOptions,
    has_negative: bool,
}

fn thresholds(visual: &ConditionalVisual) -> &[ConditionalThreshold] {
    match visual {
        ConditionalVisual::ColorScale { thresholds, .. }
        | ConditionalVisual::DataBar { thresholds, .. }
        | ConditionalVisual::IconSet { thresholds, .. } => thresholds,
    }
}
fn number(threshold: &ConditionalThreshold) -> Option<f64> {
    threshold
        .value
        .as_deref()?
        .trim()
        .strip_prefix('=')
        .unwrap_or(threshold.value.as_deref()?.trim())
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite())
}
pub(super) fn issue(rule: &ConditionalRule) -> Option<String> {
    if !rule.issues.is_empty() {
        return Some(rule.issues.join(", "));
    }
    let visual = rule.visual.as_ref()?;
    if let ConditionalVisual::IconSet {
        icon_set,
        thresholds,
        custom,
        ..
    } = visual
    {
        if rule.rule_type != "iconSet" {
            return Some("visual definition does not match rule type".into());
        }
        let expected = if custom.is_empty() {
            icon_count(icon_set)
        } else {
            Some(custom.len())
        };
        if expected != Some(thresholds.len())
            || custom
                .iter()
                .flatten()
                .any(|icon| icon_count(&icon.icon_set).is_none())
        {
            return Some("icon set thresholds do not match its icons".into());
        }
        for threshold in thresholds.iter().skip(1) {
            match threshold.kind.as_str() {
                "num" | "formula" | "percent" | "percentile" if number(threshold).is_some() => {}
                _ => return Some("unsupported or invalid icon set threshold".into()),
            }
        }
        return None;
    }
    let (count, colors) = match visual {
        ConditionalVisual::ColorScale { thresholds, colors } => {
            if !(2..=3).contains(&thresholds.len()) || thresholds.len() != colors.len() {
                return Some("color scales require two or three matching thresholds/colors".into());
            }
            (thresholds.len(), colors)
        }
        ConditionalVisual::DataBar {
            thresholds,
            colors,
            min_length,
            max_length,
            ..
        } => {
            if thresholds.len() != 2
                || colors.len() != 1
                || min_length > max_length
                || *max_length > 100
            {
                return Some("invalid data bar thresholds, color or length limits".into());
            }
            (2, colors)
        }
        ConditionalVisual::IconSet { .. } => unreachable!(),
    };
    if !matches!(
        (&rule.rule_type[..], visual),
        ("colorScale", ConditionalVisual::ColorScale { .. })
            | ("dataBar", ConditionalVisual::DataBar { .. })
    ) {
        return Some("visual definition does not match rule type".into());
    }
    if colors
        .iter()
        .any(|color| crate::color::rgba(*color).is_none())
    {
        return Some("unresolved conditional visual color".into());
    }
    let modern = matches!(
        visual,
        ConditionalVisual::DataBar {
            options: Some(_),
            ..
        }
    );
    if let ConditionalVisual::DataBar {
        options: Some(options),
        ..
    } = visual
    {
        if (!options.negative_same_color && options.negative_color.is_none())
            || (options.border_color.is_some()
                && !options.negative_same_border
                && options.negative_border_color.is_none())
            || (options.axis_position != DataBarAxisPosition::None && options.axis_color.is_none())
        {
            return Some("x14 data bar color is missing".into());
        }
        if [
            options.border_color,
            options.negative_color,
            options.negative_border_color,
            options.axis_color,
        ]
        .into_iter()
        .flatten()
        .any(|color| crate::color::rgba(color).is_none())
        {
            return Some("unresolved x14 data bar color".into());
        }
    }
    for (index, threshold) in thresholds(visual).iter().enumerate() {
        match threshold.kind.as_str() {
            "min" if index == 0 => {}
            "autoMin" if modern && index == 0 && threshold.value.is_none() => {}
            "max" if index == count - 1 => {}
            "autoMax" if modern && index == count - 1 && threshold.value.is_none() => {}
            "num" if number(threshold).is_some() || (modern && threshold.value.is_none()) => {}
            "formula" if !modern && number(threshold).is_some() => {}
            "formula"
                if modern
                    && (threshold.value.is_none()
                        || threshold.value.as_deref().is_some_and(|value| {
                            value
                                .trim()
                                .chars()
                                .next()
                                .is_some_and(|c| c.is_ascii_digit() || c == '.')
                                && number(threshold).is_some()
                        })) => {}
            "percent" | "percentile"
                if number(threshold).is_some_and(|value| (0.0..=100.0).contains(&value))
                    || (modern && threshold.value.is_none()) => {}
            _ => return Some("unsupported or invalid visual threshold".into()),
        }
    }
    None
}

fn interpolate(a: f64, b: f64, t: f64) -> f64 {
    a * (1.0 - t) + b * t
}
fn ratio(value: f64, low: f64, high: f64) -> f64 {
    if value <= low {
        return 0.0;
    }
    if value >= high {
        return 1.0;
    }
    let difference = high - low;
    if difference.is_finite() {
        (value - low) / difference
    } else {
        (value / 2.0 - low / 2.0) / (high / 2.0 - low / 2.0)
    }
}
impl PreparedVisual {
    pub(super) fn new(rule: &ConditionalRule, samples: &[f64]) -> Result<Self, String> {
        if let Some(issue) = issue(rule) {
            return Err(issue);
        }
        let visual = rule.visual.as_ref().ok_or("visual definition is missing")?;
        let modern = matches!(
            visual,
            ConditionalVisual::DataBar {
                options: Some(_),
                ..
            }
        );
        let low = samples.first().copied();
        let high = samples.last().copied();
        let mut points = Vec::new();
        for threshold in thresholds(visual) {
            let point = match threshold.kind.as_str() {
                "min" => low,
                "autoMin" => low.map(|value| value.min(0.0)),
                "max" => high,
                "autoMax" => high.map(|value| value.max(0.0)),
                "formula" if modern => Some(0.0),
                "num" | "formula" => number(threshold).or(modern.then_some(0.0)),
                "percent" => low
                    .zip(high)
                    .map(|(a, b)| interpolate(a, b, number(threshold).unwrap_or(0.0) / 100.0)),
                "percentile" => {
                    if samples.is_empty() {
                        None
                    } else {
                        let index =
                            (samples.len() - 1) as f64 * number(threshold).unwrap_or(0.0) / 100.0;
                        let left = index.floor() as usize;
                        let right = (left + 1).min(samples.len() - 1);
                        Some(interpolate(samples[left], samples[right], index.fract()))
                    }
                }
                _ => return Err("unsupported visual threshold".into()),
            };
            if let Some(point) = point {
                points.push(point);
            } else {
                points.clear();
                break;
            }
        }
        let icons = matches!(visual, ConditionalVisual::IconSet { .. });
        if !icons && points.windows(2).any(|pair| pair[0] > pair[1]) {
            return Err("visual thresholds are not in increasing order".into());
        }
        Ok(match visual {
            ConditionalVisual::IconSet {
                icon_set,
                thresholds,
                reverse,
                show_value,
                custom,
            } => {
                let count = thresholds.len();
                let icons = if custom.is_empty() {
                    (0..count as u32)
                        .map(|band| {
                            let index = if *reverse {
                                count as u32 - 1 - band
                            } else {
                                band
                            };
                            Some((icon_set.clone(), index))
                        })
                        .collect()
                } else {
                    custom
                        .iter()
                        .map(|icon| {
                            icon.as_ref()
                                .map(|icon| (icon.icon_set.clone(), icon.index))
                        })
                        .collect()
                };
                Self::IconSet {
                    points,
                    gte: thresholds.iter().map(|threshold| threshold.gte).collect(),
                    icons,
                    show_value: *show_value,
                }
            }
            ConditionalVisual::ColorScale { colors, .. } => Self::ColorScale {
                points,
                colors: colors
                    .iter()
                    .map(|color| crate::color::rgba(*color).unwrap())
                    .collect(),
            },
            ConditionalVisual::DataBar {
                colors,
                min_length,
                max_length,
                show_value,
                options,
                ..
            } => Self::DataBar {
                points,
                color: crate::color::rgba(colors[0]).unwrap(),
                min: f64::from(*min_length) / 100.0,
                max: f64::from(*max_length) / 100.0,
                show_value: *show_value,
                modern: options.as_ref().map(|options| {
                    Box::new(ModernBar {
                        options: (**options).clone(),
                        has_negative: samples.first().is_some_and(|value| *value < 0.0),
                    })
                }),
            },
        })
    }
    pub(super) fn outcome(&self, value: &CellValue) -> VisualOutcome {
        if let Self::IconSet {
            points,
            gte,
            icons,
            show_value,
        } = self
        {
            let CellValue::Number(value) = value else {
                return VisualOutcome::default();
            };
            if !value.is_finite() || points.len() != icons.len() {
                return VisualOutcome::default();
            }
            let band = (1..points.len())
                .rev()
                .find(|&band| {
                    if gte[band] {
                        *value >= points[band]
                    } else {
                        *value > points[band]
                    }
                })
                .unwrap_or(0);
            return VisualOutcome {
                icon: icons[band].as_ref().map(|(icon_set, index)| ResolvedIcon {
                    icon_set: icon_set.clone(),
                    index: *index,
                    show_value: *show_value,
                }),
                ..VisualOutcome::default()
            };
        }
        let (fill, bar) = self.for_cell(value);
        VisualOutcome {
            fill,
            bar,
            icon: None,
        }
    }

    fn for_cell(&self, value: &CellValue) -> (Option<Color>, Option<ResolvedDataBar>) {
        let CellValue::Number(value) = value else {
            return (None, None);
        };
        if !value.is_finite() {
            return (None, None);
        }
        match self {
            Self::IconSet { .. } => (None, None),
            Self::ColorScale { points, colors } => {
                if points.is_empty() {
                    return (None, None);
                }
                let rgba = if *value <= points[0] {
                    colors[0]
                } else if *value >= points[points.len() - 1] {
                    colors[colors.len() - 1]
                } else {
                    let right = points.iter().position(|point| value <= point).unwrap();
                    let t = ratio(*value, points[right - 1], points[right]);
                    std::array::from_fn(|index| {
                        interpolate(
                            f64::from(colors[right - 1][index]),
                            f64::from(colors[right][index]),
                            t,
                        )
                        .round() as u8
                    })
                };
                (
                    Some(Color::Argb([rgba[3], rgba[0], rgba[1], rgba[2]])),
                    None,
                )
            }
            Self::DataBar {
                points,
                color,
                min,
                max,
                show_value,
                modern,
            } => {
                if points.len() != 2 {
                    return (None, None);
                }
                if let Some(modern) = modern {
                    return (
                        None,
                        Some(modern.resolve(*value, points, *color, *min, *max, *show_value)),
                    );
                }
                let t = if points[0] == points[1] {
                    1.0
                } else {
                    ratio(*value, points[0], points[1])
                };
                (
                    None,
                    Some(ResolvedDataBar {
                        origin: 0.0,
                        fraction: interpolate(*min, *max, t),
                        gradient: true,
                        reverse: false,
                        border: None,
                        axis: None,
                        color: *color,
                        show_value: *show_value,
                    }),
                )
            }
        }
    }
}

impl ModernBar {
    fn resolve(
        &self,
        value: f64,
        points: &[f64],
        color: [u8; 4],
        min: f64,
        max: f64,
        show_value: bool,
    ) -> ResolvedDataBar {
        let options = &self.options;
        let rtl = match options.direction {
            DataBarDirection::Context => options.context_right_to_left,
            DataBarDirection::LeftToRight => false,
            DataBarDirection::RightToLeft => true,
        };
        let negative = value < 0.0;
        let color = if negative && !options.negative_same_color {
            options
                .negative_color
                .and_then(crate::color::rgba)
                .unwrap_or(color)
        } else {
            color
        };
        let border = if negative && !options.negative_same_border {
            options.negative_border_color.and_then(crate::color::rgba)
        } else {
            options.border_color.and_then(crate::color::rgba)
        };
        let mut axis = match options.axis_position {
            DataBarAxisPosition::None => 0.0,
            DataBarAxisPosition::Middle => 0.5,
            DataBarAxisPosition::Automatic => ratio(0.0, points[0].min(0.0), points[1].max(0.0)),
        };
        if options.axis_position == DataBarAxisPosition::Automatic
            && points[0] < 0.0
            && points[1] > 0.0
            && max > 0.0
        {
            let floor = (min / max).min(0.5);
            axis = axis.clamp(floor, 1.0 - floor);
        }
        let (mut origin, length, extends_left) =
            if options.axis_position == DataBarAxisPosition::None {
                let t = if points[0] == points[1] {
                    1.0
                } else {
                    ratio(value, points[0], points[1])
                };
                (0.0, interpolate(min, max, t), false)
            } else {
                let capacity = if options.axis_position == DataBarAxisPosition::Middle {
                    max / 2.0
                } else {
                    max * if negative { axis } else { 1.0 - axis }
                };
                let bottom = if negative {
                    (-points[1]).max(0.0)
                } else {
                    points[0].max(0.0)
                };
                let top = if options.axis_position == DataBarAxisPosition::Middle {
                    (-points[0]).max(points[1]).max(0.0)
                } else if negative {
                    (-points[0]).max(0.0)
                } else {
                    points[1].max(0.0)
                };
                let t = if bottom == top {
                    1.0
                } else {
                    ratio(value.abs(), bottom, top)
                };
                let length = if value == 0.0 {
                    0.0
                } else {
                    interpolate(min.min(capacity), capacity, t)
                };
                (axis, length, negative)
            };
        if rtl {
            origin = 1.0 - origin;
            axis = 1.0 - axis;
        }
        let axis = if self.has_negative && options.axis_position != DataBarAxisPosition::None {
            options
                .axis_color
                .and_then(crate::color::rgba)
                .map(|color| (axis, color))
        } else {
            None
        };
        ResolvedDataBar {
            origin: origin.clamp(0.0, 1.0),
            fraction: length.clamp(0.0, 1.0),
            gradient: options.gradient,
            reverse: extends_left ^ rtl,
            border,
            axis,
            color,
            show_value,
        }
    }
}
