//! Evaluates started effects at a moment into per-target state: SMIL
//! timing (delay, duration, accelerate/decelerate, auto-reverse, repeat,
//! hold) over PowerPoint's behaviours.

use std::collections::HashMap;

use jade_pptx_reader::animation::{
    AfterEffect, AnimValue, AnimationEffect, Behavior, BehaviorKind, EffectClass, IterateUnit,
    Keyframe,
};

use crate::animation::formula::{evaluate as evaluate_formula, FormulaScope};
use crate::animation::path::MotionPath;
use crate::animation::player::{target_keys, Instance, SlidePlayer};

/// What part of a shape a state applies to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeyPart {
    /// The whole shape, text and all.
    Shape,
    /// The shape without its text (`<p:bg/>`, 背景を含む).
    Body,
    /// One paragraph of its text.
    Paragraph(u32),
    /// One letter of its text, counted across paragraphs.
    Letter(usize),
    /// One word of its text, counted across paragraphs.
    Word(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TargetKey {
    pub shape_id: u32,
    pub part: KeyPart,
}

impl TargetKey {
    pub fn shape(shape_id: u32) -> Self {
        Self {
            shape_id,
            part: KeyPart::Shape,
        }
    }
}

/// A reveal mask partway: `filter` as `<p:animEffect>` names it
/// (`wipe(down)`, `circle(in)`), `progress` the shown fraction.
#[derive(Debug, Clone, PartialEq)]
pub struct Reveal {
    pub filter: String,
    pub progress: f64,
}

/// One target's look at a moment.
#[derive(Debug, Clone, PartialEq)]
pub struct TargetState {
    pub visible: bool,
    pub opacity: f64,
    /// Moved by, in fractions of the slide.
    pub offset: (f64, f64),
    pub scale: (f64, f64),
    pub rotation_deg: f64,
    pub reveal: Option<Reveal>,
    /// `#RRGGBB` overrides.
    pub fill: Option<String>,
    pub line: Option<String>,
    pub text_color: Option<String>,
    pub fill_on: Option<bool>,
    pub line_on: Option<bool>,
    pub bold: Option<bool>,
    pub underline: Option<bool>,
}

impl Default for TargetState {
    fn default() -> Self {
        Self {
            visible: true,
            opacity: 1.0,
            offset: (0.0, 0.0),
            scale: (1.0, 1.0),
            rotation_deg: 0.0,
            reveal: None,
            fill: None,
            line: None,
            text_color: None,
            fill_on: None,
            line_on: None,
            bold: None,
            underline: None,
        }
    }
}

impl TargetState {
    /// Whether it draws exactly as the slide does.
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }

    /// Whether anything of it shows.
    pub fn shows(&self) -> bool {
        self.visible && self.opacity > 0.0 && self.reveal.as_ref().is_none_or(|r| r.progress > 0.0)
    }
}

/// A target's untouched geometry and colours, as formulas read them:
/// centre and size in fractions of the slide, rotation in degrees.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TargetBase {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub r: f64,
    pub fill: Option<String>,
    pub line: Option<String>,
    pub text: Option<String>,
}

/// Every animated target's state at one moment.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SlideFrame {
    pub states: HashMap<TargetKey, TargetState>,
}

impl SlideFrame {
    pub fn state(&self, key: &TargetKey) -> TargetState {
        self.states.get(key).cloned().unwrap_or_default()
    }

    /// Whether the shape has letter or word states of its own.
    pub fn has_units(&self, shape_id: u32) -> bool {
        self.states.keys().any(|key| {
            key.shape_id == shape_id && matches!(key.part, KeyPart::Letter(_) | KeyPart::Word(_))
        })
    }

    pub fn is_empty(&self) -> bool {
        self.states.values().all(TargetState::is_identity)
    }
}

pub(crate) fn evaluate(
    player: &SlidePlayer,
    instances: &[Instance],
    now: f64,
    base: &dyn Fn(&TargetKey) -> TargetBase,
) -> SlideFrame {
    let mut frame = SlideFrame::default();
    for key in &player.initially_hidden {
        frame.states.entry(key.clone()).or_default().visible = false;
    }
    for instance in instances {
        let effect = &player.effects[instance.effect];
        let local = instance.stop.map_or(now, |stop| now.min(stop)) - instance.start;
        if local < 0.0 {
            continue;
        }
        let span = player.effect_span(instance.effect);
        let ended = local >= span || instance.stop.is_some_and(|stop| now >= stop);
        if ended && effect.rewind {
            // 再生が終了したら巻き戻す: the effect leaves no trace.
            continue;
        }
        match player.units(instance.effect) {
            Some((unit, range)) => {
                for key in player.keys(effect) {
                    frame.states.entry(key).or_default().visible = true;
                }
                let delay = player.unit_delay(instance.effect);
                let order: Vec<usize> = if effect.iterate.as_ref().is_some_and(|i| i.backwards) {
                    range.clone().rev().collect()
                } else {
                    range.clone().collect()
                };
                for (position, index) in order.into_iter().enumerate() {
                    let key = TargetKey {
                        shape_id: effect.target.shape_id,
                        part: match unit {
                            IterateUnit::Word => KeyPart::Word(index),
                            _ => KeyPart::Letter(index),
                        },
                    };
                    let state = frame
                        .states
                        .entry(key.clone())
                        .or_insert_with(|| TargetState {
                            visible: effect.class != EffectClass::Entrance,
                            ..TargetState::default()
                        });
                    let unit_local = local - position as f64 * delay;
                    if unit_local >= 0.0 {
                        apply_effect(effect, unit_local, &base(&key), state);
                    }
                }
            }
            None => {
                for key in player.keys(effect) {
                    let state = frame.states.entry(key.clone()).or_default();
                    apply_effect(effect, local, &base(&key), state);
                }
                // Behaviours aimed elsewhere than the effect's own target.
                for behavior in &effect.behaviors {
                    if behavior.target == effect.target || behavior.timing.after_effect {
                        continue;
                    }
                    for key in target_keys(behavior.target.shape_id, &behavior.target.part) {
                        let state = frame.states.entry(key.clone()).or_default();
                        let time = effect_time(effect, local);
                        apply_behavior(behavior, time, &base(&key), state);
                    }
                }
            }
        }
        if ended {
            let keys = player.keys(effect);
            for key in keys {
                let state = frame.states.entry(key).or_default();
                match &effect.after {
                    AfterEffect::None => {}
                    AfterEffect::Dim(color) => {
                        let color = hex(color);
                        state.text_color = Some(color.clone());
                        if state.fill.is_some()
                            || base(&TargetKey::shape(effect.target.shape_id))
                                .fill
                                .is_some()
                        {
                            state.fill = Some(color);
                        }
                    }
                    AfterEffect::Hide => state.visible = false,
                    AfterEffect::HideOnNextClick => {
                        if instance.stop.is_some_and(|stop| now >= stop) {
                            state.visible = false;
                        }
                    }
                }
            }
        }
    }
    frame
}

/// The effect's own clock at `local` ms after it began, through its
/// repeats, reversal and smoothing, in ms within one simple duration.
fn effect_time(effect: &AnimationEffect, local: f64) -> f64 {
    let duration = f64::from(effect.duration_ms);
    let repeat = match effect.repeat {
        jade_pptx_reader::animation::Repeat::Count(count) => count,
        jade_pptx_reader::animation::Repeat::None => 1.0,
        _ => f64::INFINITY,
    };
    progress(
        local,
        duration,
        effect.auto_reverse,
        repeat,
        effect.accel,
        effect.decel,
    ) * duration
}

fn apply_effect(effect: &AnimationEffect, local: f64, base: &TargetBase, state: &mut TargetState) {
    let time = effect_time(effect, local);
    for behavior in &effect.behaviors {
        if behavior.timing.after_effect {
            continue;
        }
        if behavior.target.shape_id != effect.target.shape_id {
            continue;
        }
        apply_behavior(behavior, time, base, state);
    }
}

/// Where a timed span stands, 0..=1 of its simple duration, after `local`
/// ms: repeats wrap it, auto-reverse runs it back, accelerate and
/// decelerate shape it (SMIL's time manipulations).
pub(crate) fn progress(
    local: f64,
    duration: f64,
    auto_reverse: bool,
    repeat: f64,
    accel: f64,
    decel: f64,
) -> f64 {
    if duration <= 0.0 {
        return 1.0;
    }
    let cycle = duration * if auto_reverse { 2.0 } else { 1.0 };
    let repeat = if repeat <= 0.0 { 1.0 } else { repeat };
    let total = cycle * repeat;
    let at = local.clamp(0.0, total);
    let cycles = at / cycle;
    let mut fraction = cycles - cycles.floor();
    if at >= total && fraction == 0.0 {
        fraction = 1.0;
    }
    let mut simple = if auto_reverse {
        let doubled = fraction * 2.0;
        if doubled > 1.0 {
            2.0 - doubled
        } else {
            doubled
        }
    } else {
        fraction
    };
    simple = ease(simple, accel, decel);
    simple.clamp(0.0, 1.0)
}

/// SMIL accelerate/decelerate: speed ramps up over the first `accel` and
/// down over the last `decel` of the span.
pub(crate) fn ease(t: f64, accel: f64, decel: f64) -> f64 {
    let (a, b) = (accel.clamp(0.0, 1.0), decel.clamp(0.0, 1.0));
    if a + b <= 0.0 || a + b > 1.0 {
        return t;
    }
    let rate = 1.0 / (1.0 - a / 2.0 - b / 2.0);
    if t < a {
        rate * t * t / (2.0 * a)
    } else if t <= 1.0 - b {
        rate * (t - a / 2.0)
    } else {
        let tail = t - (1.0 - b);
        rate * (t - a / 2.0 - tail * tail / (2.0 * b))
    }
}

fn apply_behavior(behavior: &Behavior, effect_ms: f64, base: &TargetBase, state: &mut TargetState) {
    let timing = &behavior.timing;
    let local = effect_ms - f64::from(timing.begin_ms);
    if local < 0.0 {
        return;
    }
    let duration = f64::from(timing.duration_ms.unwrap_or(1).max(1));
    let p = progress(
        local,
        duration,
        timing.auto_reverse,
        timing.repeat,
        timing.accel,
        timing.decel,
    );
    let attribute = behavior
        .attributes
        .first()
        .map(String::as_str)
        .unwrap_or("");
    let scope = FormulaScope {
        x: base.x,
        y: base.y,
        w: base.w,
        h: base.h,
        r: base.r,
        value: 0.0,
    };
    match &behavior.kind {
        BehaviorKind::Set { to } => {
            // A reversing set (Blink) undoes itself when it runs back.
            let repeat = if timing.repeat > 0.0 {
                timing.repeat
            } else {
                1.0
            };
            if timing.auto_reverse && local >= duration * 2.0 * repeat {
                return;
            }
            for attribute in &behavior.attributes {
                set_attribute(attribute, &Value::from_anim(to, &scope), base, state);
            }
        }
        BehaviorKind::Anim {
            calc_mode,
            from,
            to,
            by,
            keyframes,
            ..
        } => {
            let value = if keyframes.is_empty() {
                let current = numeric_base(attribute, base);
                let start = from
                    .as_deref()
                    .and_then(|f| evaluate_formula(f, &scope))
                    .unwrap_or(current);
                match (to.as_deref(), by.as_deref()) {
                    (Some(to), _) => {
                        Value::interpolate(&Value::Number(start), &Value::parse(to, &scope), p)
                    }
                    (None, Some(by)) => {
                        let by = evaluate_formula(by, &scope).unwrap_or(0.0);
                        Value::Number(start + by * p)
                    }
                    _ => return,
                }
            } else {
                keyframe_value(keyframes, p, calc_mode == "discrete", &scope)
            };
            set_attribute(attribute, &value, base, state);
        }
        BehaviorKind::Effect {
            transition,
            filter,
            properties,
        } => {
            let shown = match transition.as_str() {
                "out" => 1.0 - p,
                "in" => p,
                _ => 1.0,
            };
            let name = filter.split('(').next().unwrap_or("").trim();
            match name {
                "fade" => state.opacity *= shown,
                "image" | "" => {
                    if let Some(opacity) = properties
                        .as_deref()
                        .and_then(|text| text.split("opacity:").nth(1))
                        .and_then(|text| text.trim().split([';', ' ']).next())
                        .and_then(|text| text.parse::<f64>().ok())
                    {
                        state.opacity = opacity;
                    }
                }
                _ => {
                    state.reveal = (shown < 1.0).then(|| Reveal {
                        filter: filter.clone(),
                        progress: shown,
                    });
                }
            }
        }
        BehaviorKind::Scale { by, from, to, .. } => {
            let (sx, sy) = match (by, from, to) {
                (Some(by), _, _) => (
                    1.0 + (by.0 / 100_000.0 - 1.0) * p,
                    1.0 + (by.1 / 100_000.0 - 1.0) * p,
                ),
                (None, from, Some(to)) => {
                    let from = from.unwrap_or((100_000.0, 100_000.0));
                    (
                        lerp(from.0, to.0, p) / 100_000.0,
                        lerp(from.1, to.1, p) / 100_000.0,
                    )
                }
                _ => (1.0, 1.0),
            };
            state.scale.0 *= sx;
            state.scale.1 *= sy;
        }
        BehaviorKind::Rotate { by, from, to } => {
            let angle = match (by, to) {
                (Some(by), _) => by * p,
                (None, Some(to)) => lerp(from.unwrap_or(0.0), *to, p),
                _ => 0.0,
            };
            state.rotation_deg += angle;
        }
        BehaviorKind::Motion {
            path, by, from, to, ..
        } => {
            let (dx, dy) = if !path.trim().is_empty() {
                MotionPath::parse(path).point_at(p)
            } else if let Some(by) = by {
                (by.0 * p / 100_000.0, by.1 * p / 100_000.0)
            } else if let Some(to) = to {
                let from = from.unwrap_or((0.0, 0.0));
                (
                    lerp(from.0, to.0, p) / 100_000.0,
                    lerp(from.1, to.1, p) / 100_000.0,
                )
            } else {
                (0.0, 0.0)
            };
            state.offset.0 += dx;
            state.offset.1 += dy;
        }
        BehaviorKind::Color {
            color_space,
            from,
            to,
            by_hsl,
            by_rgb,
            ..
        } => {
            let current = color_base(attribute, base, state);
            let start = from.as_deref().and_then(parse_hex).or(current);
            let Some(start) = start else {
                return;
            };
            let color = if let Some(to) = to.as_deref().and_then(parse_hex) {
                if color_space == "hsl" {
                    let (h0, s0, l0) = rgb_to_hsl(start);
                    let (h1, s1, l1) = rgb_to_hsl(to);
                    hsl_to_rgb((lerp(h0, h1, p), lerp(s0, s1, p), lerp(l0, l1, p)))
                } else {
                    mix(start, to, p)
                }
            } else if let Some((dh, ds, dl)) = by_hsl {
                let (h, s, l) = rgb_to_hsl(start);
                hsl_to_rgb((
                    (h + dh * p).rem_euclid(360.0),
                    (s + ds * p).clamp(0.0, 1.0),
                    (l + dl * p).clamp(0.0, 1.0),
                ))
            } else if let Some((dr, dg, db)) = by_rgb {
                (
                    (start.0 + dr * 255.0 * p).clamp(0.0, 255.0),
                    (start.1 + dg * 255.0 * p).clamp(0.0, 255.0),
                    (start.2 + db * 255.0 * p).clamp(0.0, 255.0),
                )
            } else {
                return;
            };
            set_attribute(attribute, &Value::Color(color), base, state);
        }
        BehaviorKind::Command { .. } | BehaviorKind::Other(_) => {}
    }
}

/// An animated value.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Number(f64),
    Text(String),
    Color((f64, f64, f64)),
}

impl Value {
    fn parse(text: &str, scope: &FormulaScope) -> Self {
        if let Some(number) = evaluate_formula(text, scope) {
            return Self::Number(number);
        }
        if let Some(color) = parse_hex(text) {
            return Self::Color(color);
        }
        Self::Text(text.to_string())
    }

    fn from_anim(value: &AnimValue, scope: &FormulaScope) -> Self {
        match value {
            AnimValue::Float(number) => Self::Number(*number),
            AnimValue::Int(number) => Self::Number(*number as f64),
            AnimValue::Bool(flag) => Self::Text(flag.to_string()),
            AnimValue::Color(color) => {
                parse_hex(color).map_or(Self::Text(color.clone()), Self::Color)
            }
            AnimValue::SchemeColor(name) => Self::Text(name.clone()),
            AnimValue::Str(text) => {
                // `visible`, `bold` and `true` are words, not formulas.
                if text.chars().all(|c| c.is_ascii_alphabetic()) && !text.starts_with('#') {
                    Self::Text(text.clone())
                } else {
                    Self::parse(text, scope)
                }
            }
        }
    }

    fn interpolate(from: &Self, to: &Self, p: f64) -> Self {
        match (from, to) {
            (Self::Number(a), Self::Number(b)) => Self::Number(lerp(*a, *b, p)),
            (Self::Color(a), Self::Color(b)) => Self::Color(mix(*a, *b, p)),
            _ => {
                if p < 1.0 {
                    from.clone()
                } else {
                    to.clone()
                }
            }
        }
    }
}

fn keyframe_value(keyframes: &[Keyframe], p: f64, discrete: bool, scope: &FormulaScope) -> Value {
    let value_at = |keyframe: &Keyframe| {
        keyframe
            .value
            .as_ref()
            .map_or(Value::Number(0.0), |value| Value::from_anim(value, scope))
    };
    // The last keyframe closes the interval before it, whose formula
    // still shapes the value it ends on.
    let index = keyframes
        .iter()
        .rposition(|keyframe| keyframe.time <= p)
        .unwrap_or(0)
        .min(keyframes.len().saturating_sub(2));
    let current = &keyframes[index];
    let value = match keyframes.get(index + 1) {
        Some(next) if !discrete && next.time > current.time => {
            let local = ((p - current.time) / (next.time - current.time)).clamp(0.0, 1.0);
            Value::interpolate(&value_at(current), &value_at(next), local)
        }
        _ => value_at(current),
    };
    match (&current.formula, &value) {
        (Some(formula), Value::Number(number)) => {
            let scope = FormulaScope {
                value: *number,
                ..*scope
            };
            evaluate_formula(formula, &scope).map_or(value, Value::Number)
        }
        _ => value,
    }
}

fn numeric_base(attribute: &str, base: &TargetBase) -> f64 {
    match attribute {
        "ppt_x" => base.x,
        "ppt_y" => base.y,
        "ppt_w" => base.w,
        "ppt_h" => base.h,
        "ppt_r" | "r" | "style.rotation" => base.r,
        "style.opacity" => 1.0,
        _ => 0.0,
    }
}

fn color_base(attribute: &str, base: &TargetBase, state: &TargetState) -> Option<(f64, f64, f64)> {
    let (current, original) = match attribute {
        "style.color" => (&state.text_color, &base.text),
        "stroke.color" => (&state.line, &base.line),
        _ => (&state.fill, &base.fill),
    };
    current
        .as_deref()
        .and_then(parse_hex)
        .or_else(|| original.as_deref().and_then(parse_hex))
}

fn set_attribute(attribute: &str, value: &Value, base: &TargetBase, state: &mut TargetState) {
    let number = match value {
        Value::Number(number) => Some(*number),
        _ => None,
    };
    let text = match value {
        Value::Text(text) => text.as_str(),
        _ => "",
    };
    match attribute {
        "style.visibility" => state.visible = text != "hidden",
        "style.opacity" => {
            if let Some(number) = number {
                state.opacity = number.clamp(0.0, 1.0);
            }
        }
        "ppt_x" => {
            if let Some(number) = number {
                state.offset.0 = number - base.x;
            }
        }
        "ppt_y" => {
            if let Some(number) = number {
                state.offset.1 = number - base.y;
            }
        }
        "ppt_w" => {
            if let Some(number) = number {
                state.scale.0 = if base.w > 0.0 { number / base.w } else { 1.0 };
            }
        }
        "ppt_h" => {
            if let Some(number) = number {
                state.scale.1 = if base.h > 0.0 { number / base.h } else { 1.0 };
            }
        }
        "ppt_r" | "r" | "style.rotation" => {
            if let Some(number) = number {
                state.rotation_deg = number - base.r;
            }
        }
        "fillcolor" | "fill.color" => state.fill = color_text(value),
        "style.color" => state.text_color = color_text(value),
        "stroke.color" => state.line = color_text(value),
        "fill.on" => state.fill_on = Some(text != "false"),
        "stroke.on" => state.line_on = Some(text != "false"),
        "style.fontWeight" => state.bold = Some(text == "bold"),
        "style.textDecorationUnderline" => {
            state.underline = Some(text == "true" || text == "single")
        }
        _ => {}
    }
}

fn color_text(value: &Value) -> Option<String> {
    match value {
        Value::Color(color) => Some(to_hex(*color)),
        _ => None,
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn mix(a: (f64, f64, f64), b: (f64, f64, f64), t: f64) -> (f64, f64, f64) {
    (lerp(a.0, b.0, t), lerp(a.1, b.1, t), lerp(a.2, b.2, t))
}

/// `RRGGBB` or `#RRGGBB`.
pub(crate) fn parse_hex(text: &str) -> Option<(f64, f64, f64)> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() < 6 || !hex.is_char_boundary(6) {
        return None;
    }
    let channel =
        |range: std::ops::Range<usize>| u8::from_str_radix(&hex[range], 16).ok().map(f64::from);
    Some((channel(0..2)?, channel(2..4)?, channel(4..6)?))
}

pub(crate) fn to_hex(color: (f64, f64, f64)) -> String {
    let channel = |value: f64| value.round().clamp(0.0, 255.0) as u8;
    format!(
        "#{:02X}{:02X}{:02X}",
        channel(color.0),
        channel(color.1),
        channel(color.2)
    )
}

fn hex(text: &str) -> String {
    parse_hex(text).map_or_else(|| text.to_string(), to_hex)
}

fn rgb_to_hsl((r, g, b): (f64, f64, f64)) -> (f64, f64, f64) {
    let (r, g, b) = (r / 255.0, g / 255.0, b / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn hsl_to_rgb((h, s, l): (f64, f64, f64)) -> (f64, f64, f64) {
    if s == 0.0 {
        return (l * 255.0, l * 255.0, l * 255.0);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let channel = |t: f64| {
        let t = t.rem_euclid(1.0);
        let value = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        value * 255.0
    };
    let h = h / 360.0;
    (channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smil_timing_shapes_progress() {
        assert_eq!(progress(250.0, 500.0, false, 1.0, 0.0, 0.0), 0.5);
        assert_eq!(progress(900.0, 500.0, false, 1.0, 0.0, 0.0), 1.0);
        // Auto-reverse: there and back.
        assert_eq!(progress(500.0, 500.0, true, 1.0, 0.0, 0.0), 1.0);
        assert_eq!(progress(1000.0, 500.0, true, 1.0, 0.0, 0.0), 0.0);
        // Two repeats end at the end.
        assert_eq!(progress(750.0, 500.0, false, 2.0, 0.0, 0.0), 0.5);
        assert_eq!(progress(1000.0, 500.0, false, 2.0, 0.0, 0.0), 1.0);
        // Smooth start and end stay inside 0..1 and are symmetric.
        let middle = ease(0.5, 0.5, 0.5);
        assert!((middle - 0.5).abs() < 1e-9);
        assert!(ease(0.1, 0.5, 0.5) < 0.1);
        assert!((ease(1.0, 0.3, 0.2) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn colours_round_trip_through_hsl() {
        let (h, s, l) = rgb_to_hsl((255.0, 0.0, 0.0));
        assert_eq!((h, s, l), (0.0, 1.0, 0.5));
        assert_eq!(to_hex(hsl_to_rgb((180.0, 1.0, 0.5))), "#00FFFF");
        assert_eq!(parse_hex("4472C4"), Some((68.0, 114.0, 196.0)));
    }
}
