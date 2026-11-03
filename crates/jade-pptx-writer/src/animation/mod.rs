//! Editing a slide's animations and transition, as PowerPoint's アニメーション
//! and 画面切り替え tabs do.
//!
//! A slide's effects are edited as the animation pane lists them — the main
//! sequence, then each trigger's — and addressed by their place in that
//! list. Every edit reads the slide's `<p:timing>`, changes the effect
//! list, and writes the timing back the way PowerPoint lays it out
//! ([`serialize`]); an effect's markup is kept as it was apart from what the
//! edit changes. Transitions are written by [`transition`].

pub mod presets;
pub mod serialize;
pub mod transition;
pub mod zoom;

use std::collections::HashMap;

use jade_pptx_reader::animation::{
    parse_effect, read_slide_animation, AfterEffect, AnimationEffect, AnimationTarget, EffectStart,
    InteractiveSequence, IterateUnit, Repeat, SlideAnimation, SlideTiming, TargetPart, Trigger,
    XmlChild, XmlNode,
};
use serde::{Deserialize, Serialize};

use crate::{EditSession, Result, WriteError};
pub use presets::{build_effect_node, preset, EffectParams, EffectPreset, PresetClass, PRESETS};
pub use serialize::ShapeKind;

/// One edit of a slide's effects. Effects are addressed by their index in
/// [`pane_effects`].
#[derive(Debug, Clone, PartialEq)]
pub enum AnimationEdit {
    /// アニメーションの追加: a preset on each target, after `after` (or at
    /// the end). The first starts `start` (on click by default), the rest
    /// with it.
    Add {
        class: PresetClass,
        preset_id: u32,
        params: EffectParams,
        targets: Vec<AnimationTarget>,
        start: Option<EffectStart>,
        after: Option<usize>,
    },
    /// Choosing another effect from the gallery for the selected effects:
    /// their timing stays.
    Change {
        indices: Vec<usize>,
        class: PresetClass,
        preset_id: u32,
        params: EffectParams,
    },
    /// 効果のオプション: a new subtype or parameter for the same preset.
    SetOptions {
        indices: Vec<usize>,
        params: EffectParams,
    },
    Remove {
        indices: Vec<usize>,
    },
    /// Every effect of a shape (or of the slide when `None`).
    RemoveAll {
        shape_id: Option<u32>,
    },
    /// 順番を前にする / 後にする: moves effect `index` to `to`.
    Move {
        index: usize,
        to: usize,
    },
    SetStart {
        indices: Vec<usize>,
        start: EffectStart,
    },
    SetDuration {
        indices: Vec<usize>,
        ms: u32,
    },
    SetDelay {
        indices: Vec<usize>,
        ms: u32,
    },
    /// 滑らかに開始 / 終了 / 弾んで終了 as accel and decel fractions.
    SetSmoothing {
        indices: Vec<usize>,
        accel: f64,
        decel: f64,
    },
    SetRepeat {
        indices: Vec<usize>,
        repeat: Repeat,
    },
    SetRewind {
        indices: Vec<usize>,
        rewind: bool,
    },
    SetAutoReverse {
        indices: Vec<usize>,
        auto_reverse: bool,
    },
    SetAfter {
        indices: Vec<usize>,
        after: AfterEffect,
    },
    /// テキストの動作: all at once, by word or by letter (with the delay
    /// between units as a fraction of the duration).
    SetTextUnit {
        indices: Vec<usize>,
        unit: Option<(IterateUnit, f64)>,
    },
    /// トリガー: on a click of a shape, at a bookmark, or (None) back in
    /// the main sequence.
    SetTrigger {
        indices: Vec<usize>,
        trigger: Option<Trigger>,
    },
    /// 効果のオプション › シーケンス for a text shape's effect: as one
    /// object, all at once, or by paragraph — one effect per paragraph.
    SetSequence {
        index: usize,
        sequence: TextSequence,
        paragraphs: u32,
    },
    /// グラフ アニメーション: as one object, or by series or category
    /// (one effect per element).
    SetChartSequence {
        index: usize,
        sequence: ChartSequence,
        series: u32,
        categories: u32,
    },
    /// アニメーションのコピー/貼り付け: the effects of `from` (a shape on
    /// this slide or `source` given) appended for each target shape.
    Paint {
        source: Vec<AnimationEffect>,
        targets: Vec<u32>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextSequence {
    AsOneObject,
    AllAtOnce,
    ByParagraph,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChartSequence {
    AsOneObject,
    BySeries,
    ByCategory,
    BySeriesElement,
    ByCategoryElement,
}

/// An effect as the pane lists it: the effect and the trigger it belongs to.
#[derive(Debug, Clone, PartialEq)]
pub struct PaneEffect {
    pub effect: AnimationEffect,
    pub trigger: Option<Trigger>,
}

/// The effects in pane order: main sequence, then each trigger's.
pub fn pane_effects(timing: &SlideTiming) -> Vec<PaneEffect> {
    let mut out: Vec<PaneEffect> = timing
        .main_sequence
        .iter()
        .map(|effect| PaneEffect {
            effect: effect.clone(),
            trigger: None,
        })
        .collect();
    for sequence in &timing.interactive_sequences {
        out.extend(sequence.effects.iter().map(|effect| PaneEffect {
            effect: effect.clone(),
            trigger: Some(sequence.trigger.clone()),
        }));
    }
    out
}

/// Puts a pane list back into sequences; triggers keep their first order.
fn regroup(timing: &mut SlideTiming, list: Vec<PaneEffect>) {
    timing.main_sequence.clear();
    let mut order: Vec<Trigger> = timing
        .interactive_sequences
        .iter()
        .map(|s| s.trigger.clone())
        .collect();
    let mut sequences: HashMap<usize, Vec<AnimationEffect>> = HashMap::new();
    for item in list {
        match item.trigger {
            None => timing.main_sequence.push(item.effect),
            Some(trigger) => {
                let index = match order.iter().position(|t| *t == trigger) {
                    Some(index) => index,
                    None => {
                        order.push(trigger);
                        order.len() - 1
                    }
                };
                sequences.entry(index).or_default().push(item.effect);
            }
        }
    }
    timing.interactive_sequences = order
        .into_iter()
        .enumerate()
        .filter_map(|(index, trigger)| {
            let mut effects = sequences.remove(&index)?;
            // A trigger's first effect is what its click starts.
            if let Some(first) = effects.first_mut() {
                first.start = EffectStart::OnClick;
            }
            Some(InteractiveSequence { trigger, effects })
        })
        .collect();
}

fn ctn_mut(effect: &mut AnimationEffect) -> Option<&mut XmlNode> {
    effect.node.child_mut("cTn")
}

/// Re-reads an effect's fields from its markup.
fn refresh(effect: &mut AnimationEffect) {
    if let Some(fresh) = parse_effect(&effect.node, Some(effect.start)) {
        let start = effect.start;
        let group = effect.group_id;
        *effect = fresh;
        effect.start = start;
        effect.group_id = group;
    }
}

/// Sets the effect's own start delay.
fn set_delay(effect: &mut AnimationEffect, ms: u32) {
    if let Some(ctn) = ctn_mut(effect) {
        let list = match ctn.child_mut("stCondLst") {
            Some(list) => list,
            None => {
                ctn.children
                    .insert(0, XmlChild::Element(XmlNode::new("p:stCondLst")));
                ctn.child_mut("stCondLst").expect("just inserted")
            }
        };
        match list.child_mut("cond") {
            Some(cond) => cond.set_attr("delay", ms.to_string()),
            None => list.children.push(XmlChild::Element(
                XmlNode::new("p:cond").with_attr("delay", ms.to_string()),
            )),
        }
    }
}

/// Scales every behaviour's length and offset so the effect lasts `ms`.
fn set_duration(effect: &mut AnimationEffect, ms: u32) {
    let current = effect.duration_ms;
    let ms = ms.max(10);
    if current <= 1 {
        // Appear, Disappear: nothing to stretch.
        return;
    }
    let ratio = f64::from(ms) / f64::from(current);
    let Some(ctn) = ctn_mut(effect) else {
        return;
    };
    if let Some(children) = ctn.child_mut("childTnLst") {
        children.visit_mut(&mut |node| {
            if node.local_name() != "cTn" {
                return;
            }
            if let Some(dur) = node.attr("dur").and_then(|d| d.parse::<f64>().ok()) {
                if dur > 1.0 {
                    node.set_attr("dur", ((dur * ratio).round().max(1.0) as u64).to_string());
                }
            }
            if let Some(cond) = node.path_mut(&["stCondLst", "cond"]) {
                if let Some(delay) = cond.attr("delay").and_then(|d| d.parse::<f64>().ok()) {
                    if delay > 0.0 {
                        cond.set_attr("delay", ((delay * ratio).round() as u64).to_string());
                    }
                }
            }
        });
    }
}

fn set_fraction(node: &mut XmlNode, name: &str, value: f64) {
    let value = (value.clamp(0.0, 1.0) * 100_000.0).round() as u32;
    if value == 0 {
        node.remove_attr(name);
    } else {
        // accel/decel sit before fill in PowerPoint's attribute order.
        let at = node
            .attrs
            .iter()
            .position(|(key, _)| key == "fill" || key == "nodeType" || key == "grpId");
        if node.attr(name).is_some() {
            node.set_attr(name, value.to_string());
        } else if let Some(at) = at {
            node.attrs.insert(at, (name.to_string(), value.to_string()));
        } else {
            node.set_attr(name, value.to_string());
        }
    }
}

fn set_repeat(effect: &mut AnimationEffect, repeat: Repeat) {
    let Some(ctn) = ctn_mut(effect) else {
        return;
    };
    ctn.children
        .retain(|child| !matches!(child, XmlChild::Element(n) if n.local_name() == "endCondLst"));
    match repeat {
        Repeat::None => ctn.remove_attr("repeatCount"),
        Repeat::Count(count) => {
            ctn.set_attr("repeatCount", ((count * 1000.0).round() as u64).to_string())
        }
        Repeat::UntilEndOfSlide => ctn.set_attr("repeatCount", "indefinite"),
        Repeat::UntilNextClick => {
            ctn.set_attr("repeatCount", "indefinite");
            let end = XmlNode::parse(
                r#"<p:endCondLst><p:cond evt="onNext" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:endCondLst>"#,
            )
            .expect("static markup");
            let at = ctn
                .children
                .iter()
                .position(|c| matches!(c, XmlChild::Element(n) if n.local_name() == "stCondLst"))
                .map_or(0, |i| i + 1);
            ctn.children.insert(at, XmlChild::Element(end));
        }
    }
}

/// Replaces the effect's after-effect behaviours.
fn set_after(effect: &mut AnimationEffect, after: &AfterEffect) {
    let duration = effect.duration_ms;
    let target = presets::target_xml(&effect.target);
    let Some(children) = ctn_mut(effect).and_then(|ctn| ctn.child_mut("childTnLst")) else {
        return;
    };
    children.children.retain(|child| match child {
        XmlChild::Element(node) => !node.descendants("cTn").iter().any(|ctn| {
            ctn.attr("afterEffect")
                .is_some_and(|v| v == "1" || v == "true")
        }),
        XmlChild::Text(_) => true,
    });
    let delay = duration.saturating_sub(1);
    let markup = match after {
        AfterEffect::None => return,
        AfterEffect::Hide => format!(
            r#"<p:set><p:cBhvr><p:cTn id="0" dur="1" fill="hold" afterEffect="1"><p:stCondLst><p:cond delay="{delay}"/></p:stCondLst></p:cTn>{target}<p:attrNameLst><p:attrName>style.visibility</p:attrName></p:attrNameLst></p:cBhvr><p:to><p:strVal val="hidden"/></p:to></p:set>"#
        ),
        AfterEffect::HideOnNextClick => format!(
            r#"<p:set><p:cBhvr><p:cTn id="0" dur="1" fill="hold" afterEffect="1"><p:stCondLst><p:cond evt="onNext" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:stCondLst></p:cTn>{target}<p:attrNameLst><p:attrName>style.visibility</p:attrName></p:attrNameLst></p:cBhvr><p:to><p:strVal val="hidden"/></p:to></p:set>"#
        ),
        AfterEffect::Dim(color) => format!(
            r#"<p:animClr clrSpc="rgb" dir="cw"><p:cBhvr override="childStyle"><p:cTn id="0" dur="1" fill="hold" afterEffect="1"><p:stCondLst><p:cond delay="{delay}"/></p:stCondLst></p:cTn>{target}<p:attrNameLst><p:attrName>style.color</p:attrName></p:attrNameLst></p:cBhvr><p:to><a:srgbClr val="{color}"/></p:to></p:animClr>"#
        ),
    };
    let wrapped = format!(
        r#"<w xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">{markup}</w>"#
    );
    if let Ok(parsed) = XmlNode::parse(&wrapped) {
        children.children.extend(parsed.children);
    }
}

fn set_iterate(effect: &mut AnimationEffect, unit: Option<(IterateUnit, f64)>) {
    let Some(ctn) = ctn_mut(effect) else {
        return;
    };
    ctn.children
        .retain(|child| !matches!(child, XmlChild::Element(n) if n.local_name() == "iterate"));
    let Some((unit, interval)) = unit else {
        return;
    };
    let kind = match unit {
        IterateUnit::Element => "el",
        IterateUnit::Word => "wd",
        IterateUnit::Letter => "lt",
    };
    let node = XmlNode::new("p:iterate")
        .with_attr("type", kind)
        .with_child(XmlNode::new("p:tmPct").with_attr(
            "val",
            ((interval.clamp(0.0, 1.0)) * 100_000.0).round().to_string(),
        ));
    let at = ctn
        .children
        .iter()
        .position(|c| matches!(c, XmlChild::Element(n) if n.local_name() == "childTnLst"))
        .unwrap_or(ctn.children.len());
    ctn.children.insert(at, XmlChild::Element(node));
}

/// The next unused build group id for a shape.
fn next_group(list: &[PaneEffect], shape_id: u32) -> u32 {
    list.iter()
        .filter(|item| item.effect.target.shape_id == shape_id)
        .filter_map(|item| item.effect.group_id)
        .map(|group| group + 1)
        .max()
        .unwrap_or(0)
}

/// A new effect of `preset` on `target`.
pub fn new_effect(
    preset: &EffectPreset,
    params: &EffectParams,
    target: &AnimationTarget,
    start: EffectStart,
    duration_ms: Option<u32>,
) -> AnimationEffect {
    let node = build_effect_node(preset, params, target, start, duration_ms);
    let mut effect = parse_effect(&node, Some(start)).expect("a built effect parses");
    effect.start = start;
    effect.target = target.clone();
    effect
}

/// Carries an old effect's timing over to a rebuilt one.
fn carry_timing(new: &mut AnimationEffect, old: &AnimationEffect) {
    new.start = old.start;
    new.group_id = old.group_id;
    set_delay(new, old.delay_ms);
    if old.duration_ms > 1 && new.duration_ms > 1 {
        set_duration(new, old.duration_ms);
    }
    if let Some(ctn) = ctn_mut(new) {
        if old.accel > 0.0 || old.decel > 0.0 {
            set_fraction(ctn, "accel", old.accel);
            set_fraction(ctn, "decel", old.decel);
        }
        if old.auto_reverse {
            ctn.set_attr("autoRev", "1");
        }
        if old.rewind {
            ctn.set_attr("fill", "remove");
        }
    }
    set_repeat(new, old.repeat);
    refresh(new);
    if old.after != AfterEffect::None {
        set_after(new, &old.after);
    }
    if let Some(iterate) = &old.iterate {
        let interval = match iterate.interval {
            jade_pptx_reader::animation::IterateInterval::Percent(p) => p,
            jade_pptx_reader::animation::IterateInterval::Milliseconds(ms) => {
                f64::from(ms) / f64::from(new.duration_ms.max(1))
            }
        };
        set_iterate(new, Some((iterate.unit, interval)));
    }
    refresh(new);
}

fn preset_of(effect: &AnimationEffect) -> Option<&'static EffectPreset> {
    let class = PresetClass::from_class(&effect.class)?;
    presets::preset(class, effect.preset_id)
}

fn sorted_desc(indices: &[usize]) -> Vec<usize> {
    let mut indices = indices.to_vec();
    indices.sort_unstable();
    indices.dedup();
    indices.reverse();
    indices
}

/// Applies an edit to a slide's timing.
pub fn apply_edit(timing: &mut SlideTiming, edit: &AnimationEdit) -> Result<()> {
    let mut list = pane_effects(timing);
    let check = |list: &[PaneEffect], indices: &[usize]| -> Result<()> {
        if indices.iter().any(|&i| i >= list.len()) {
            Err(WriteError::Xml("no such animation effect".into()))
        } else {
            Ok(())
        }
    };
    match edit {
        AnimationEdit::Add {
            class,
            preset_id,
            params,
            targets,
            start,
            after,
        } => {
            let preset = presets::preset(*class, *preset_id)
                .ok_or_else(|| WriteError::Xml("unknown animation preset".into()))?;
            let mut at = after.map_or(list.len(), |a| (a + 1).min(list.len()));
            let trigger = after
                .and_then(|a| list.get(a))
                .and_then(|i| i.trigger.clone());
            for (n, target) in targets.iter().enumerate() {
                let start = if n == 0 {
                    start.unwrap_or(EffectStart::OnClick)
                } else {
                    EffectStart::WithPrevious
                };
                let mut effect = new_effect(preset, params, target, start, None);
                effect.group_id = Some(next_group(&list, target.shape_id));
                list.insert(
                    at,
                    PaneEffect {
                        effect,
                        trigger: trigger.clone(),
                    },
                );
                at += 1;
            }
        }
        AnimationEdit::Change {
            indices,
            class,
            preset_id,
            params,
        } => {
            check(&list, indices)?;
            let preset = presets::preset(*class, *preset_id)
                .ok_or_else(|| WriteError::Xml("unknown animation preset".into()))?;
            for &index in indices {
                let old = list[index].effect.clone();
                let mut new = new_effect(preset, params, &old.target, old.start, None);
                carry_timing(&mut new, &old);
                // Entrance to exit is a new build of the shape.
                if old.class != new.class {
                    new.group_id = Some(next_group(&list, old.target.shape_id));
                }
                list[index].effect = new;
            }
        }
        AnimationEdit::SetOptions { indices, params } => {
            check(&list, indices)?;
            for &index in indices {
                let old = list[index].effect.clone();
                let Some(preset) = preset_of(&old) else {
                    continue;
                };
                let mut new = new_effect(preset, params, &old.target, old.start, None);
                carry_timing(&mut new, &old);
                list[index].effect = new;
            }
        }
        AnimationEdit::Remove { indices } => {
            check(&list, indices)?;
            for index in sorted_desc(indices) {
                list.remove(index);
            }
        }
        AnimationEdit::RemoveAll { shape_id } => {
            list.retain(|item| shape_id.is_some_and(|id| item.effect.target.shape_id != id));
        }
        AnimationEdit::Move { index, to } => {
            check(&list, &[*index])?;
            let mut item = list.remove(*index);
            let to = (*to).min(list.len());
            // Dropped among another sequence's effects, it joins that
            // sequence: the one it lands before, or the last one.
            let neighbour = list.get(to).or_else(|| list.last());
            if let Some(neighbour) = neighbour {
                item.trigger = neighbour.trigger.clone();
            }
            list.insert(to, item);
        }
        AnimationEdit::SetStart { indices, start } => {
            check(&list, indices)?;
            for &index in indices {
                list[index].effect.start = *start;
            }
        }
        AnimationEdit::SetDuration { indices, ms } => {
            check(&list, indices)?;
            for &index in indices {
                let effect = &mut list[index].effect;
                set_duration(effect, *ms);
                let after = effect.after.clone();
                refresh(effect);
                if after != AfterEffect::None {
                    set_after(effect, &after);
                    refresh(effect);
                }
            }
        }
        AnimationEdit::SetDelay { indices, ms } => {
            check(&list, indices)?;
            for &index in indices {
                set_delay(&mut list[index].effect, *ms);
                refresh(&mut list[index].effect);
            }
        }
        AnimationEdit::SetSmoothing {
            indices,
            accel,
            decel,
        } => {
            check(&list, indices)?;
            for &index in indices {
                if let Some(ctn) = ctn_mut(&mut list[index].effect) {
                    set_fraction(ctn, "accel", *accel);
                    set_fraction(ctn, "decel", *decel);
                }
                refresh(&mut list[index].effect);
            }
        }
        AnimationEdit::SetRepeat { indices, repeat } => {
            check(&list, indices)?;
            for &index in indices {
                set_repeat(&mut list[index].effect, *repeat);
                refresh(&mut list[index].effect);
            }
        }
        AnimationEdit::SetRewind { indices, rewind } => {
            check(&list, indices)?;
            for &index in indices {
                if let Some(ctn) = ctn_mut(&mut list[index].effect) {
                    ctn.set_attr("fill", if *rewind { "remove" } else { "hold" });
                }
                refresh(&mut list[index].effect);
            }
        }
        AnimationEdit::SetAutoReverse {
            indices,
            auto_reverse,
        } => {
            check(&list, indices)?;
            for &index in indices {
                if let Some(ctn) = ctn_mut(&mut list[index].effect) {
                    if *auto_reverse {
                        ctn.set_attr("autoRev", "1");
                    } else {
                        ctn.remove_attr("autoRev");
                    }
                }
                refresh(&mut list[index].effect);
            }
        }
        AnimationEdit::SetAfter { indices, after } => {
            check(&list, indices)?;
            for &index in indices {
                set_after(&mut list[index].effect, after);
                refresh(&mut list[index].effect);
            }
        }
        AnimationEdit::SetTextUnit { indices, unit } => {
            check(&list, indices)?;
            for &index in indices {
                set_iterate(&mut list[index].effect, *unit);
                refresh(&mut list[index].effect);
            }
        }
        AnimationEdit::SetTrigger { indices, trigger } => {
            check(&list, indices)?;
            let mut moved = Vec::new();
            for index in sorted_desc(indices) {
                moved.push(list.remove(index));
            }
            moved.reverse();
            for (n, mut item) in moved.into_iter().enumerate() {
                item.trigger = trigger.clone();
                if n == 0 {
                    item.effect.start = EffectStart::OnClick;
                }
                list.push(item);
            }
        }
        AnimationEdit::SetSequence {
            index,
            sequence,
            paragraphs,
        } => {
            check(&list, &[*index])?;
            let shape_id = list[*index].effect.target.shape_id;
            let group = list[*index].effect.group_id;
            // The effect and its paragraph siblings (same shape and build).
            let members: Vec<usize> = (0..list.len())
                .filter(|&i| {
                    list[i].effect.target.shape_id == shape_id && list[i].effect.group_id == group
                })
                .collect();
            let first = members[0];
            let template = list[first].clone();
            for i in members.iter().rev() {
                list.remove(*i);
            }
            let mut inserted = Vec::new();
            let make = |part: TargetPart, start: EffectStart| {
                let mut item = template.clone();
                let target = AnimationTarget { shape_id, part };
                retarget(&mut item.effect, &target);
                item.effect.start = start;
                item
            };
            match sequence {
                TextSequence::AsOneObject => {
                    inserted.push(make(TargetPart::Shape, template.effect.start));
                }
                TextSequence::AllAtOnce | TextSequence::ByParagraph => {
                    for p in 0..(*paragraphs).max(1) {
                        let start = if p == 0 {
                            template.effect.start
                        } else if *sequence == TextSequence::AllAtOnce {
                            EffectStart::WithPrevious
                        } else {
                            EffectStart::OnClick
                        };
                        inserted.push(make(TargetPart::Paragraphs { start: p, end: p }, start));
                    }
                }
            }
            for (n, item) in inserted.into_iter().enumerate() {
                list.insert(first + n, item);
            }
        }
        AnimationEdit::SetChartSequence {
            index,
            sequence,
            series,
            categories,
        } => {
            check(&list, &[*index])?;
            let shape_id = list[*index].effect.target.shape_id;
            let group = list[*index].effect.group_id;
            let members: Vec<usize> = (0..list.len())
                .filter(|&i| {
                    list[i].effect.target.shape_id == shape_id && list[i].effect.group_id == group
                })
                .collect();
            let first = members[0];
            let template = list[first].clone();
            for i in members.iter().rev() {
                list.remove(*i);
            }
            let part = |series: i32, category: i32, step: &str| TargetPart::Chart {
                series,
                category,
                step: step.to_string(),
            };
            // The chart's background first, as PowerPoint builds it.
            let mut parts = Vec::new();
            match sequence {
                ChartSequence::AsOneObject => parts.push(TargetPart::Shape),
                ChartSequence::BySeries => {
                    parts.push(part(-3, -3, "bg"));
                    for s in 0..*series as i32 {
                        parts.push(part(s, -4, "series"));
                    }
                }
                ChartSequence::ByCategory => {
                    parts.push(part(-3, -3, "bg"));
                    for c in 0..*categories as i32 {
                        parts.push(part(-4, c, "category"));
                    }
                }
                ChartSequence::BySeriesElement => {
                    parts.push(part(-3, -3, "bg"));
                    for s in 0..*series as i32 {
                        for c in 0..*categories as i32 {
                            parts.push(part(s, c, "ptInSeries"));
                        }
                    }
                }
                ChartSequence::ByCategoryElement => {
                    parts.push(part(-3, -3, "bg"));
                    for c in 0..*categories as i32 {
                        for s in 0..*series as i32 {
                            parts.push(part(s, c, "ptInCategory"));
                        }
                    }
                }
            }
            for (n, part) in parts.into_iter().enumerate() {
                let mut item = template.clone();
                retarget(&mut item.effect, &AnimationTarget { shape_id, part });
                item.effect.start = if n == 0 {
                    template.effect.start
                } else {
                    EffectStart::OnClick
                };
                list.insert(first + n, item);
            }
        }
        AnimationEdit::Paint { source, targets } => {
            for &shape_id in targets {
                let group = next_group(&list, shape_id);
                for (n, effect) in source.iter().enumerate() {
                    let mut effect = effect.clone();
                    let part = match &effect.target.part {
                        TargetPart::Paragraphs { .. } | TargetPart::Shape => {
                            effect.target.part.clone()
                        }
                        _ => TargetPart::Shape,
                    };
                    retarget(&mut effect, &AnimationTarget { shape_id, part });
                    effect.group_id = Some(group);
                    if n == 0 && effect.start != EffectStart::OnClick && list.is_empty() {
                        effect.start = EffectStart::OnClick;
                    }
                    list.push(PaneEffect {
                        effect,
                        trigger: None,
                    });
                }
            }
        }
    }
    regroup(timing, list);
    Ok(())
}

/// Points every behaviour of an effect at a new target.
pub fn retarget(effect: &mut AnimationEffect, target: &AnimationTarget) {
    let markup = presets::target_xml(target);
    let wrapped = format!(
        r#"<w xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">{markup}</w>"#
    );
    let Some(new_target) = XmlNode::parse(&wrapped)
        .ok()
        .and_then(|w| w.elements().next().cloned())
    else {
        return;
    };
    effect.node.visit_mut(&mut |node| {
        if node.local_name() == "cBhvr" {
            for child in node.elements_mut() {
                if child.local_name() == "tgtEl" {
                    *child = new_target.clone();
                }
            }
        }
    });
    effect.target = target.clone();
    refresh(effect);
    effect.target = target.clone();
}

/// The slide's animation as the editor sees it.
pub fn slide_animation(bytes: &[u8], slide: usize) -> Result<SlideAnimation> {
    let (_, _, xml) = crate::slide_xml(bytes, slide)?;
    Ok(read_slide_animation(&xml)?)
}

/// What kind of object each top-level (and grouped) shape id is.
pub fn shape_kinds(slide_xml: &str) -> HashMap<u32, ShapeKind> {
    let mut kinds = HashMap::new();
    let Ok(root) = XmlNode::parse(slide_xml) else {
        return kinds;
    };
    fn walk(node: &XmlNode, kinds: &mut HashMap<u32, ShapeKind>) {
        for child in node.elements() {
            let nv = match child.local_name() {
                "sp" => Some(("nvSpPr", ShapeKind::Shape)),
                "pic" => Some(("nvPicPr", ShapeKind::Other)),
                "cxnSp" => Some(("nvCxnSpPr", ShapeKind::Shape)),
                "grpSp" => Some(("nvGrpSpPr", ShapeKind::Other)),
                "graphicFrame" => {
                    let uri = child
                        .path(&["graphic", "graphicData"])
                        .and_then(|d| d.attr("uri"))
                        .unwrap_or("");
                    let kind = if uri.ends_with("/chart") || uri.contains("chartex") {
                        ShapeKind::Chart
                    } else if uri.ends_with("/diagram") {
                        ShapeKind::Diagram
                    } else {
                        ShapeKind::Other
                    };
                    Some(("nvGraphicFramePr", kind))
                }
                _ => None,
            };
            if let Some((nv, kind)) = nv {
                if let Some(id) = child
                    .path(&[nv, "cNvPr"])
                    .and_then(|c| c.attr("id"))
                    .and_then(|v| v.parse().ok())
                {
                    kinds.insert(id, kind);
                }
            }
            if matches!(
                child.local_name(),
                "grpSp" | "AlternateContent" | "Choice" | "Fallback"
            ) {
                walk(child, kinds);
            }
        }
    }
    if let Some(tree) = root.path(&["cSld", "spTree"]) {
        walk(tree, &mut kinds);
    }
    kinds
}

/// Writes `timing` into a slide part's markup.
pub fn write_timing(slide_xml: &str, timing: &SlideTiming) -> Result<String> {
    let kinds = shape_kinds(slide_xml);
    let markup = serialize::timing_markup(timing, &kinds);
    crate::xml::animation::replace_timing(slide_xml, markup.as_deref())
}

/// Applies `edit` to slide `slide`'s animations.
pub fn edit_animation(bytes: &[u8], slide: usize, edit: &AnimationEdit) -> Result<Vec<u8>> {
    let (mut package, part, xml) = crate::slide_xml(bytes, slide)?;
    let mut timing = read_slide_animation(&xml)?.timing;
    apply_edit(&mut timing, edit)?;
    let changed = write_timing(&xml, &timing)?;
    if changed == xml {
        return Ok(bytes.to_vec());
    }
    package.replace(&part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}

impl EditSession {
    /// One animation edit as one step of Undo.
    pub fn edit_animation(&mut self, slide: usize, edit: &AnimationEdit) -> Result<bool> {
        let next = edit_animation(self.bytes(), slide, edit)?;
        Ok(self.commit(next))
    }

    /// One transition edit as one step of Undo.
    pub fn set_transition(
        &mut self,
        slides: &[usize],
        edit: &transition::TransitionEdit,
    ) -> Result<bool> {
        let next = transition::set_transitions(self.bytes(), slides, edit)?;
        Ok(self.commit(next))
    }
}

#[cfg(test)]
mod tests;
