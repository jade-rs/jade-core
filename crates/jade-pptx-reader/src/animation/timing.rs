//! `<p:timing>`: a slide's animation effects, as PowerPoint's animation pane
//! lists them.
//!
//! PowerPoint writes the main sequence as three levels of `<p:par>` under
//! the `mainSeq` node — one per click, one per "after previous" step inside
//! that click, and one per effect — and each trigger as its own
//! `interactiveSeq`. The reader flattens that back to the list the pane
//! shows: each effect with how it starts (on click, with or after the
//! previous one), its preset, its target and its behaviours. The effect's
//! own markup is kept so a deck saves back as it came, and so an edit only
//! touches what it changes.

use serde::{Deserialize, Serialize};

use super::node::XmlNode;

/// Everything `<p:timing>` says about a slide.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SlideTiming {
    /// The main sequence: what clicks (and the effects tied to them) play.
    pub main_sequence: Vec<AnimationEffect>,
    /// Sequences that start on a click of a shape or at a media bookmark.
    pub interactive_sequences: Vec<InteractiveSequence>,
    /// `<p:bldLst>`: how each animated shape is built.
    pub builds: Vec<BuildEntry>,
    /// Other children of the root time node (media nodes, sequences this
    /// reader does not interpret), kept verbatim in their order.
    pub other_nodes: Vec<XmlNode>,
    /// Where each of `other_nodes` sat among the root's children: the
    /// number of interactive sequences before it (main sequence first).
    #[serde(default)]
    pub other_positions: Vec<usize>,
}

impl SlideTiming {
    /// No effects and nothing else under the root.
    pub fn is_empty(&self) -> bool {
        self.main_sequence.is_empty()
            && self.interactive_sequences.is_empty()
            && self.other_nodes.is_empty()
    }

    /// Every effect: the main sequence's, then each trigger's.
    pub fn all_effects(&self) -> impl Iterator<Item = &AnimationEffect> {
        self.main_sequence.iter().chain(
            self.interactive_sequences
                .iter()
                .flat_map(|sequence| sequence.effects.iter()),
        )
    }
}

/// Effects that play when something other than the slide's click starts
/// them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InteractiveSequence {
    pub trigger: Trigger,
    pub effects: Vec<AnimationEffect>,
}

/// What starts an interactive sequence: アニメーション › トリガー.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trigger {
    /// クリック時 › a shape on the slide.
    ShapeClick { shape_id: u32 },
    /// ブックマーク時: a bookmark of the media shape.
    Bookmark { shape_id: u32, bookmark: String },
}

/// The kind of an effect: PowerPoint's `presetClass`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectClass {
    Entrance,
    Emphasis,
    Exit,
    MotionPath,
    /// `verb`, `mediacall`: OLE verbs and media play/pause/stop.
    Media,
    /// Anything else, as written (or empty when the effect has no class).
    Other(String),
}

impl EffectClass {
    pub fn parse(value: &str) -> Self {
        match value {
            "entr" => Self::Entrance,
            "emph" => Self::Emphasis,
            "exit" => Self::Exit,
            "path" => Self::MotionPath,
            "mediacall" | "verb" => Self::Media,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Entrance => "entr",
            Self::Emphasis => "emph",
            Self::Exit => "exit",
            Self::MotionPath => "path",
            Self::Media => "mediacall",
            Self::Other(value) => value,
        }
    }
}

/// How an effect starts: 開始 in the timing group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectStart {
    /// クリック時.
    OnClick,
    /// 直前の動作と同時.
    WithPrevious,
    /// 直前の動作の後.
    AfterPrevious,
}

impl EffectStart {
    pub fn node_type(self) -> &'static str {
        match self {
            Self::OnClick => "clickEffect",
            Self::WithPrevious => "withEffect",
            Self::AfterPrevious => "afterEffect",
        }
    }

    pub fn parse(node_type: &str) -> Option<Self> {
        match node_type {
            "clickEffect" => Some(Self::OnClick),
            "withEffect" => Some(Self::WithPrevious),
            "afterEffect" => Some(Self::AfterPrevious),
            _ => None,
        }
    }
}

/// What an effect (or one behaviour of it) animates.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AnimationTarget {
    pub shape_id: u32,
    pub part: TargetPart,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TargetPart {
    /// The whole shape.
    #[default]
    Shape,
    /// `<p:bg/>`: the shape without its text.
    Background,
    /// `<p:txEl><p:pRg>`: paragraphs `start..=end`.
    Paragraphs { start: u32, end: u32 },
    /// `<p:txEl><p:charRg>`: characters `start..end`.
    Characters { start: u32, end: u32 },
    /// `<p:graphicEl><a:chart>`: one step of a chart build. Negative
    /// indices are PowerPoint's markers for the background (-3) and the
    /// whole series or category (-4 = all).
    Chart {
        series: i32,
        category: i32,
        step: String,
    },
    /// `<p:graphicEl><a:dgm>`: one SmartArt shape.
    Diagram { id: String, step: String },
    /// `<p:subSp spid>`: a subshape (ink, Office art).
    Subshape { id: String },
    /// `<p:oleChartEl>`.
    OleChart { kind: String, level: u32 },
}

impl AnimationTarget {
    pub fn shape(shape_id: u32) -> Self {
        Self {
            shape_id,
            part: TargetPart::Shape,
        }
    }
}

/// After the effect: アニメーションの後 in 効果のオプション.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AfterEffect {
    /// 淡色表示しない.
    #[default]
    None,
    /// Dimmed to an RGB colour (`RRGGBB`).
    Dim(String),
    /// アニメーションの後で非表示にする.
    Hide,
    /// 次のクリックで非表示にする.
    HideOnNextClick,
}

/// `<p:iterate>`: テキストの動作 by word or letter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Iterate {
    pub unit: IterateUnit,
    /// The delay between units: a percentage of the effect's duration
    /// (`tmPct`, in thousandths of a percent) or absolute ms (`tmAbs`).
    pub interval: IterateInterval,
    /// `backwards="1"`: 逆順.
    #[serde(default)]
    pub backwards: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IterateUnit {
    Element,
    Word,
    Letter,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum IterateInterval {
    /// Fraction of the effect's duration (0.1 = 10 %).
    Percent(f64),
    Milliseconds(u32),
}

/// 繰り返し.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum Repeat {
    #[default]
    None,
    /// A count, possibly fractional.
    Count(f64),
    /// 次のクリックまで.
    UntilNextClick,
    /// スライドの最後まで.
    UntilEndOfSlide,
}

/// One effect, with what the animation pane shows of it and its markup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationEffect {
    pub class: EffectClass,
    pub preset_id: u32,
    pub preset_subtype: u32,
    pub start: EffectStart,
    pub target: AnimationTarget,
    /// The effect's own delay (遅延), ms.
    pub delay_ms: u32,
    /// How long the effect plays once (継続時間), ms: its longest
    /// behaviour.
    pub duration_ms: u32,
    /// `grpId`: ties the effect to its `<p:bldLst>` entry.
    pub group_id: Option<u32>,
    /// Smooth start / end (滑らかに開始/終了), as fractions 0..=1.
    pub accel: f64,
    pub decel: f64,
    /// 自動的に元に戻す.
    pub auto_reverse: bool,
    pub repeat: Repeat,
    /// 巻き戻し: `fill="remove"` instead of hold, the effect undoes itself.
    pub rewind: bool,
    pub after: AfterEffect,
    pub iterate: Option<Iterate>,
    /// The behaviours, for playing the effect.
    pub behaviors: Vec<Behavior>,
    /// The effect's `<p:par>` as written.
    pub node: XmlNode,
}

impl AnimationEffect {
    /// Re-reads the effect's fields from its markup (after an edit changed
    /// the markup).
    pub fn reread(&self, start: EffectStart) -> Option<Self> {
        parse_effect(&self.node, Some(start))
    }

    /// Every target this effect touches, its own first.
    pub fn targets(&self) -> Vec<AnimationTarget> {
        let mut targets = vec![self.target.clone()];
        for behavior in &self.behaviors {
            if !targets.contains(&behavior.target) {
                targets.push(behavior.target.clone());
            }
        }
        targets
    }
}

/// When a behaviour plays inside its effect, and how its progress runs.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct BehaviorTiming {
    /// Delay from the effect's start, ms.
    pub begin_ms: u32,
    /// One play's length; `None` is `indefinite`.
    pub duration_ms: Option<u32>,
    pub accel: f64,
    pub decel: f64,
    pub auto_reverse: bool,
    /// Repeat count; 1 plays once.
    pub repeat: f64,
    /// `fill="hold"` (or freeze): the end value stays once it has played.
    pub hold: bool,
    /// `afterEffect="1"`: アニメーションの後 (dim, hide).
    pub after_effect: bool,
}

/// One behaviour of an effect: what it changes and how.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Behavior {
    pub target: AnimationTarget,
    pub timing: BehaviorTiming,
    /// The attributes the behaviour names (`style.visibility`, `ppt_x`).
    pub attributes: Vec<String>,
    /// `additive="base|sum|repl|mult"` on the common behaviour.
    pub additive: Option<String>,
    pub kind: BehaviorKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BehaviorKind {
    /// `<p:set>`: the attribute takes `to` for the behaviour's span.
    Set { to: AnimValue },
    /// `<p:anim>`: keyframes over the span.
    Anim {
        /// `calcmode`: `lin`, `discrete`, `fmla`.
        calc_mode: String,
        /// `valueType`: `num`, `str`, `clr`.
        value_type: String,
        from: Option<String>,
        to: Option<String>,
        by: Option<String>,
        keyframes: Vec<Keyframe>,
    },
    /// `<p:animEffect>`: a transition filter.
    Effect {
        /// `in`, `out` or `none`.
        transition: String,
        filter: String,
        /// `prLst`, e.g. `opacity: 0.5`.
        properties: Option<String>,
    },
    /// `<p:animScale>`: percentages, 100000 = 100 %.
    Scale {
        by: Option<(f64, f64)>,
        from: Option<(f64, f64)>,
        to: Option<(f64, f64)>,
        zoom_contents: bool,
    },
    /// `<p:animRot>`: degrees.
    Rotate {
        by: Option<f64>,
        from: Option<f64>,
        to: Option<f64>,
    },
    /// `<p:animMotion>`: the path as written (`M 0 0 L 0.25 0 E`), in
    /// fractions of the slide.
    Motion {
        path: String,
        origin: String,
        path_edit_mode: String,
        points_types: Option<String>,
        /// `rCtr`, the rotation centre, in thousandths of a percent.
        rotation_center: Option<(f64, f64)>,
        by: Option<(f64, f64)>,
        from: Option<(f64, f64)>,
        to: Option<(f64, f64)>,
    },
    /// `<p:animClr>`.
    Color {
        color_space: String,
        direction: String,
        from: Option<String>,
        to: Option<String>,
        /// `by` in HSL: hue (degrees), saturation and lightness (fractions).
        by_hsl: Option<(f64, f64, f64)>,
        by_rgb: Option<(f64, f64, f64)>,
    },
    /// `<p:cmd>`: a media or OLE command.
    Command { kind: String, command: String },
    /// `<p:audio>` and anything else this reader does not play.
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    /// Position in the span, 0..=1.
    pub time: f64,
    pub value: Option<AnimValue>,
    /// `fmla`: a formula over the value (`$`) and the shape.
    pub formula: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AnimValue {
    Str(String),
    Bool(bool),
    Int(i64),
    Float(f64),
    /// An RGB colour `RRGGBB`, resolved where the markup names one.
    Color(String),
    /// A scheme colour name (`accent1`) with no resolution here.
    SchemeColor(String),
}

impl AnimValue {
    pub fn as_text(&self) -> String {
        match self {
            Self::Str(value) | Self::Color(value) | Self::SchemeColor(value) => value.clone(),
            Self::Bool(value) => value.to_string(),
            Self::Int(value) => value.to_string(),
            Self::Float(value) => value.to_string(),
        }
    }
}

/// One `<p:bldLst>` entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildEntry {
    pub shape_id: u32,
    pub group_id: u32,
    pub kind: BuildKind,
    /// The entry as written.
    pub node: XmlNode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuildKind {
    /// `<p:bldP>`: text. `build` is `whole`, `p` (by paragraph level), `cust`
    /// or `allAtOnce`; `level` the paragraph level built by.
    Paragraph {
        build: String,
        level: Option<u32>,
        animate_background: bool,
        reverse: bool,
    },
    /// `<p:bldGraphic>`: a chart or SmartArt; `as_one` or the `bldSub`
    /// build (`series`, `category`, `seriesEl`, `categoryEl`, or a diagram
    /// build such as `one`, `lvlOne`, `lvlAtOnce`).
    Graphic { as_one: bool, build: Option<String> },
    /// `<p:bldDgm>`, `<p:bldOleChart>`.
    Other(String),
}

/// Parses `<p:timing>`.
pub fn parse_timing(timing: &XmlNode) -> SlideTiming {
    let mut out = SlideTiming::default();
    let root = timing
        .path(&["tnLst", "par", "cTn"])
        .filter(|ctn| ctn.attr("nodeType").is_none_or(|kind| kind == "tmRoot"));
    if let Some(root) = root {
        if let Some(children) = root.child("childTnLst") {
            for child in children.elements() {
                let ctn = child.child("cTn");
                let node_type = ctn.and_then(|ctn| ctn.attr("nodeType"));
                match (child.local_name(), node_type) {
                    ("seq", Some("mainSeq")) if out.main_sequence.is_empty() => {
                        out.main_sequence = sequence_effects(child);
                    }
                    ("seq", Some("interactiveSeq")) => match trigger(child) {
                        Some(trigger) => out.interactive_sequences.push(InteractiveSequence {
                            trigger,
                            effects: sequence_effects(child),
                        }),
                        None => {
                            out.other_positions.push(out.interactive_sequences.len());
                            out.other_nodes.push(child.clone());
                        }
                    },
                    _ => {
                        out.other_positions.push(out.interactive_sequences.len());
                        out.other_nodes.push(child.clone());
                    }
                }
            }
        }
    }
    if let Some(builds) = timing.child("bldLst") {
        out.builds = builds.elements().filter_map(parse_build).collect();
    }
    out
}

fn trigger(seq: &XmlNode) -> Option<Trigger> {
    let list = seq.path(&["cTn", "stCondLst"])?;
    // A bookmark trigger is a PowerPoint 2010 condition, written inside
    // `mc:AlternateContent` with a click on the media as its fallback.
    let mut conditions = Vec::new();
    for element in list.elements() {
        match element.local_name() {
            "cond" => conditions.push(element),
            "AlternateContent" => conditions.extend(
                element
                    .children_named("Choice")
                    .chain(element.children_named("Fallback"))
                    .filter_map(|branch| branch.child("cond")),
            ),
            _ => {}
        }
    }
    for cond in conditions {
        let Some(target) = cond.child("tgtEl") else {
            continue;
        };
        if let Some(bookmark) = target.child("bmkTgt") {
            if let Some(shape_id) = bookmark.attr("spid").and_then(|v| v.parse().ok()) {
                return Some(Trigger::Bookmark {
                    shape_id,
                    bookmark: bookmark.attr("bmkName").unwrap_or("").to_string(),
                });
            }
        }
        if let Some(shape_id) = target
            .child("spTgt")
            .and_then(|sp| sp.attr("spid"))
            .and_then(|v| v.parse().ok())
        {
            return Some(Trigger::ShapeClick { shape_id });
        }
    }
    None
}

/// The effects of a sequence, flattened from its click and step groups.
fn sequence_effects(seq: &XmlNode) -> Vec<AnimationEffect> {
    let mut effects = Vec::new();
    let Some(groups) = seq.path(&["cTn", "childTnLst"]) else {
        return effects;
    };
    for click in groups.children_named("par") {
        let Some(steps) = click.path(&["cTn", "childTnLst"]) else {
            continue;
        };
        for (step_index, step) in steps.children_named("par").enumerate() {
            let Some(items) = step.path(&["cTn", "childTnLst"]) else {
                continue;
            };
            for (item_index, item) in items.children_named("par").enumerate() {
                // Without a node type, the position says how it starts.
                let fallback = if step_index == 0 && item_index == 0 {
                    EffectStart::OnClick
                } else if item_index == 0 {
                    EffectStart::AfterPrevious
                } else {
                    EffectStart::WithPrevious
                };
                if let Some(effect) = parse_effect(item, Some(fallback)) {
                    effects.push(effect);
                }
            }
        }
    }
    effects
}

fn ms(value: Option<&str>) -> Option<u32> {
    value
        .and_then(|value| value.parse::<f64>().ok())
        .map(|v| v.max(0.0) as u32)
}

fn fraction(value: Option<&str>) -> f64 {
    value
        .and_then(|value| value.parse::<f64>().ok())
        .map_or(0.0, |value| value / 100_000.0)
}

fn start_delay(ctn: &XmlNode) -> u32 {
    ctn.path(&["stCondLst", "cond"])
        .and_then(|cond| cond.attr("delay"))
        .and_then(|delay| delay.parse::<f64>().ok())
        .map_or(0, |delay| delay.max(0.0) as u32)
}

/// Parses one effect `<p:par>`; `fallback` is how it starts when the
/// markup carries no node type.
pub fn parse_effect(par: &XmlNode, fallback: Option<EffectStart>) -> Option<AnimationEffect> {
    let ctn = par.child("cTn")?;
    let start = ctn
        .attr("nodeType")
        .and_then(EffectStart::parse)
        .or(fallback)?;
    let class = EffectClass::parse(ctn.attr("presetClass").unwrap_or(""));
    let mut behaviors = Vec::new();
    if let Some(children) = ctn.child("childTnLst") {
        collect_behaviors(children, 0, &mut behaviors);
    }
    let target = behaviors
        .iter()
        .find(|behavior| !behavior.timing.after_effect)
        .or(behaviors.first())
        .map(|behavior| behavior.target.clone())
        .unwrap_or_default();
    let duration_ms = behaviors
        .iter()
        .filter(|behavior| !behavior.timing.after_effect)
        .map(|behavior| {
            behavior.timing.begin_ms
                + behavior.timing.duration_ms.unwrap_or(0)
                    * if behavior.timing.auto_reverse { 2 } else { 1 }
        })
        .max()
        .unwrap_or(0);
    let repeat = match ctn.attr("repeatCount") {
        Some("indefinite") => match ctn.path(&["endCondLst", "cond"]) {
            Some(cond) if cond.child("tn").is_some() || cond.attr("evt") == Some("onNext") => {
                Repeat::UntilNextClick
            }
            _ => Repeat::UntilEndOfSlide,
        },
        Some(count) => count
            .parse::<f64>()
            .ok()
            .map(|count| count / 1000.0)
            .filter(|count| (*count - 1.0).abs() > 1e-9)
            .map_or(Repeat::None, Repeat::Count),
        None => Repeat::None,
    };
    let after = after_effect(&behaviors);
    let iterate = ctn.child("iterate").map(|iterate| Iterate {
        unit: match iterate.attr("type") {
            Some("wd") => IterateUnit::Word,
            Some("lt") => IterateUnit::Letter,
            _ => IterateUnit::Element,
        },
        interval: match (iterate.child("tmPct"), iterate.child("tmAbs")) {
            (Some(pct), _) => IterateInterval::Percent(fraction(pct.attr("val"))),
            (None, Some(abs)) => IterateInterval::Milliseconds(ms(abs.attr("val")).unwrap_or(0)),
            (None, None) => IterateInterval::Percent(0.1),
        },
        backwards: iterate.attr("backwards").is_some_and(truthy),
    });
    Some(AnimationEffect {
        class,
        preset_id: ctn
            .attr("presetID")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        preset_subtype: ctn
            .attr("presetSubtype")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        start,
        target,
        delay_ms: start_delay(ctn),
        duration_ms,
        group_id: ctn.attr("grpId").and_then(|v| v.parse().ok()),
        accel: fraction(ctn.attr("accel")),
        decel: fraction(ctn.attr("decel")),
        auto_reverse: ctn.attr("autoRev").is_some_and(truthy),
        repeat,
        rewind: ctn.attr("fill") == Some("remove"),
        after,
        iterate,
        behaviors,
        node: par.clone(),
    })
}

fn after_effect(behaviors: &[Behavior]) -> AfterEffect {
    for behavior in behaviors.iter().filter(|b| b.timing.after_effect) {
        match &behavior.kind {
            BehaviorKind::Set {
                to: AnimValue::Str(value),
            } if value == "hidden" => return AfterEffect::Hide,
            BehaviorKind::Color {
                to: Some(color), ..
            } => return AfterEffect::Dim(color.clone()),
            BehaviorKind::Set {
                to: AnimValue::Color(color),
            } => return AfterEffect::Dim(color.clone()),
            _ => {}
        }
    }
    AfterEffect::None
}

fn truthy(value: &str) -> bool {
    matches!(value, "1" | "true" | "on")
}

/// Walks an effect's `childTnLst`, nested time containers included, for
/// its behaviours. `offset` is the containers' delay so far.
fn collect_behaviors(children: &XmlNode, offset: u32, out: &mut Vec<Behavior>) {
    for child in children.elements() {
        match child.local_name() {
            "par" | "seq" | "excl" => {
                if let Some(ctn) = child.child("cTn") {
                    let delay = start_delay(ctn);
                    if let Some(inner) = ctn.child("childTnLst") {
                        collect_behaviors(inner, offset + delay, out);
                    }
                }
            }
            _ => {
                if let Some(behavior) = parse_behavior(child, offset) {
                    out.push(behavior);
                }
            }
        }
    }
}

fn pair(node: Option<&XmlNode>) -> Option<(f64, f64)> {
    let node = node?;
    Some((node.attr("x")?.parse().ok()?, node.attr("y")?.parse().ok()?))
}

fn parse_behavior(node: &XmlNode, offset: u32) -> Option<Behavior> {
    let cbhvr = node.child("cBhvr");
    let ctn = cbhvr.and_then(|c| c.child("cTn"));
    let timing = ctn.map_or_else(BehaviorTiming::default, |ctn| BehaviorTiming {
        begin_ms: offset + start_delay(ctn),
        duration_ms: match ctn.attr("dur") {
            Some("indefinite") => None,
            other => Some(ms(other).unwrap_or(0)),
        },
        accel: fraction(ctn.attr("accel")),
        decel: fraction(ctn.attr("decel")),
        auto_reverse: ctn.attr("autoRev").is_some_and(truthy),
        repeat: match ctn.attr("repeatCount") {
            Some("indefinite") => f64::INFINITY,
            Some(count) => count.parse::<f64>().map_or(1.0, |c| c / 1000.0),
            None => 1.0,
        },
        hold: matches!(ctn.attr("fill"), Some("hold") | Some("freeze")),
        after_effect: ctn.attr("afterEffect").is_some_and(truthy),
    });
    let target = cbhvr
        .and_then(|c| c.child("tgtEl"))
        .and_then(parse_target)
        .unwrap_or_default();
    let attributes = cbhvr
        .and_then(|c| c.child("attrNameLst"))
        .map(|list| list.children_named("attrName").map(|a| a.text()).collect())
        .unwrap_or_default();
    let additive = cbhvr.and_then(|c| c.attr("additive")).map(str::to_string);
    let kind = match node.local_name() {
        "set" => BehaviorKind::Set {
            to: node
                .child("to")
                .and_then(anim_value)
                .unwrap_or(AnimValue::Str(String::new())),
        },
        "anim" => BehaviorKind::Anim {
            calc_mode: node.attr("calcmode").unwrap_or("lin").to_string(),
            value_type: node.attr("valueType").unwrap_or("num").to_string(),
            from: node.attr("from").map(str::to_string),
            to: node.attr("to").map(str::to_string),
            by: node.attr("by").map(str::to_string),
            keyframes: node
                .child("tavLst")
                .map(|list| {
                    list.children_named("tav")
                        .map(|tav| Keyframe {
                            time: match tav.attr("tm") {
                                Some("indefinite") => 1.0,
                                tm => fraction(tm),
                            },
                            value: tav.child("val").and_then(anim_value),
                            formula: tav.attr("fmla").map(str::to_string),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        },
        "animEffect" => BehaviorKind::Effect {
            transition: node.attr("transition").unwrap_or("in").to_string(),
            filter: node.attr("filter").unwrap_or("").to_string(),
            properties: node.attr("prLst").map(str::to_string),
        },
        "animScale" => BehaviorKind::Scale {
            by: pair(node.child("by")),
            from: pair(node.child("from")),
            to: pair(node.child("to")),
            zoom_contents: node.attr("zoomContents").is_some_and(truthy),
        },
        "animRot" => BehaviorKind::Rotate {
            by: node
                .attr("by")
                .and_then(|v| v.parse::<f64>().ok())
                .map(|v| v / 60_000.0),
            from: node
                .attr("from")
                .and_then(|v| v.parse::<f64>().ok())
                .map(|v| v / 60_000.0),
            to: node
                .attr("to")
                .and_then(|v| v.parse::<f64>().ok())
                .map(|v| v / 60_000.0),
        },
        "animMotion" => BehaviorKind::Motion {
            path: node.attr("path").unwrap_or("").to_string(),
            origin: node.attr("origin").unwrap_or("parent").to_string(),
            path_edit_mode: node.attr("pathEditMode").unwrap_or("relative").to_string(),
            points_types: node.attr("ptsTypes").map(str::to_string),
            rotation_center: pair(node.child("rCtr")),
            by: pair(node.child("by")),
            from: pair(node.child("from")),
            to: pair(node.child("to")),
        },
        "animClr" => BehaviorKind::Color {
            color_space: node.attr("clrSpc").unwrap_or("rgb").to_string(),
            direction: node.attr("dir").unwrap_or("cw").to_string(),
            from: node.child("from").and_then(color_value),
            to: node.child("to").and_then(color_value),
            by_hsl: node.child("by").and_then(|by| by.child("hsl")).map(|hsl| {
                (
                    hsl.attr("h")
                        .and_then(|v| v.parse::<f64>().ok())
                        .unwrap_or(0.0)
                        / 60_000.0,
                    fraction(hsl.attr("s")),
                    fraction(hsl.attr("l")),
                )
            }),
            by_rgb: node.child("by").and_then(|by| by.child("rgb")).map(|rgb| {
                (
                    fraction(rgb.attr("r")),
                    fraction(rgb.attr("g")),
                    fraction(rgb.attr("b")),
                )
            }),
        },
        "cmd" => BehaviorKind::Command {
            kind: node.attr("type").unwrap_or("call").to_string(),
            command: node.attr("cmd").unwrap_or("").to_string(),
        },
        other => BehaviorKind::Other(other.to_string()),
    };
    Some(Behavior {
        target,
        timing,
        attributes,
        additive,
        kind,
    })
}

fn color_value(node: &XmlNode) -> Option<String> {
    let color = node.elements().next()?;
    match color.local_name() {
        "srgbClr" => color.attr("val").map(|v| v.to_ascii_uppercase()),
        "schemeClr" => color.attr("val").map(|v| format!("scheme:{v}")),
        "prstClr" => color.attr("val").map(|v| format!("preset:{v}")),
        "sysClr" => color
            .attr("lastClr")
            .or(color.attr("val"))
            .map(str::to_string),
        _ => None,
    }
}

fn anim_value(node: &XmlNode) -> Option<AnimValue> {
    let value = node.elements().next()?;
    Some(match value.local_name() {
        "strVal" => AnimValue::Str(value.attr("val").unwrap_or("").to_string()),
        "boolVal" => AnimValue::Bool(value.attr("val").is_some_and(truthy)),
        "intVal" => AnimValue::Int(value.attr("val").and_then(|v| v.parse().ok()).unwrap_or(0)),
        "fltVal" => AnimValue::Float(
            value
                .attr("val")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0),
        ),
        "clrVal" => match color_value(value) {
            Some(color) if color.starts_with("scheme:") => {
                AnimValue::SchemeColor(color.trim_start_matches("scheme:").to_string())
            }
            Some(color) => AnimValue::Color(color),
            None => return None,
        },
        _ => return None,
    })
}

/// Parses `<p:tgtEl>`.
pub fn parse_target(tgt: &XmlNode) -> Option<AnimationTarget> {
    let sp = tgt.child("spTgt")?;
    let shape_id = sp.attr("spid")?.parse().ok()?;
    let part = if sp.child("bg").is_some() {
        TargetPart::Background
    } else if let Some(text) = sp.child("txEl") {
        if let Some(range) = text.child("pRg") {
            TargetPart::Paragraphs {
                start: range.attr("st").and_then(|v| v.parse().ok()).unwrap_or(0),
                end: range.attr("end").and_then(|v| v.parse().ok()).unwrap_or(0),
            }
        } else if let Some(range) = text.child("charRg") {
            TargetPart::Characters {
                start: range.attr("st").and_then(|v| v.parse().ok()).unwrap_or(0),
                end: range.attr("end").and_then(|v| v.parse().ok()).unwrap_or(0),
            }
        } else {
            TargetPart::Shape
        }
    } else if let Some(graphic) = sp.child("graphicEl") {
        if let Some(chart) = graphic.child("chart") {
            TargetPart::Chart {
                series: chart
                    .attr("seriesIdx")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(-3),
                category: chart
                    .attr("categoryIdx")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(-3),
                step: chart.attr("bldStep").unwrap_or("").to_string(),
            }
        } else if let Some(diagram) = graphic.child("dgm") {
            TargetPart::Diagram {
                id: diagram.attr("id").unwrap_or("").to_string(),
                step: diagram.attr("bldStep").unwrap_or("").to_string(),
            }
        } else {
            TargetPart::Shape
        }
    } else if let Some(sub) = sp.child("subSp") {
        TargetPart::Subshape {
            id: sub.attr("spid").unwrap_or("").to_string(),
        }
    } else if let Some(ole) = sp.child("oleChartEl") {
        TargetPart::OleChart {
            kind: ole.attr("type").unwrap_or("").to_string(),
            level: ole.attr("lvl").and_then(|v| v.parse().ok()).unwrap_or(0),
        }
    } else {
        TargetPart::Shape
    };
    Some(AnimationTarget { shape_id, part })
}

fn parse_build(node: &XmlNode) -> Option<BuildEntry> {
    let shape_id = node.attr("spid")?.parse().ok()?;
    let group_id = node.attr("grpId").and_then(|v| v.parse().ok()).unwrap_or(0);
    let kind = match node.local_name() {
        "bldP" => BuildKind::Paragraph {
            build: node.attr("build").unwrap_or("whole").to_string(),
            level: node.attr("bldLvl").and_then(|v| v.parse().ok()),
            animate_background: node.attr("animBg").is_some_and(truthy),
            reverse: node.attr("rev").is_some_and(truthy),
        },
        "bldGraphic" => BuildKind::Graphic {
            as_one: node.child("bldAsOne").is_some(),
            build: node.child("bldSub").and_then(|sub| {
                sub.elements()
                    .next()
                    .and_then(|el| el.attr("bld").map(str::to_string))
            }),
        },
        other => BuildKind::Other(other.to_string()),
    };
    Some(BuildEntry {
        shape_id,
        group_id,
        kind,
        node: node.clone(),
    })
}
