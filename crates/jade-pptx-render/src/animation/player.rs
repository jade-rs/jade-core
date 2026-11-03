//! The show's clock against a slide's `<p:timing>`: click groups, their
//! steps and the trigger sequences, the way PowerPoint schedules them.

use std::collections::HashMap;

use jade_pptx_reader::animation::{
    AnimationEffect, EffectClass, EffectStart, IterateInterval, IterateUnit, Repeat, SlideTiming,
    TargetPart, Trigger,
};

use crate::animation::evaluate::{evaluate, KeyPart, SlideFrame, TargetBase, TargetKey};

/// The text of each shape on the slide, paragraph by paragraph, so the
/// player can count paragraphs, words and letters.
pub type SlideText = HashMap<u32, Vec<String>>;

/// What a click (or the space bar) did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Advance {
    /// It started or finished animations on this slide.
    Animated,
    /// Nothing is left on this slide: go to the next one.
    NextSlide,
    /// Going back went past the first group: go to the previous slide.
    PreviousSlide,
}

/// One effect placed on a group's clock.
#[derive(Debug, Clone)]
pub(crate) struct Placed {
    pub effect: usize,
    /// From the group's start, ms.
    pub offset: f64,
}

/// One click's worth of effects.
#[derive(Debug, Clone, Default)]
pub(crate) struct Group {
    pub effects: Vec<Placed>,
    /// When its last effect ends; infinite while one repeats to the end.
    pub span: f64,
}

#[derive(Debug, Clone)]
struct TriggerSequence {
    trigger: Trigger,
    groups: Vec<Group>,
    /// Starts of the groups fired so far, in order; the next click fires
    /// the next group, wrapping around.
    starts: Vec<f64>,
}

/// An effect that has started (or is due), on the slide's clock.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Instance {
    pub effect: usize,
    pub start: f64,
    /// When a later click cut it short (`until next click`).
    pub stop: Option<f64>,
}

/// A slide's animations being played.
#[derive(Debug, Clone)]
pub struct SlidePlayer {
    pub(crate) effects: Vec<AnimationEffect>,
    groups: Vec<Group>,
    /// The first group starts with the slide when its first effect is not
    /// On Click.
    auto_first: bool,
    starts: Vec<f64>,
    triggers: Vec<TriggerSequence>,
    pub(crate) text: SlideText,
    /// Each target's state before anything plays: hidden when its first
    /// effect is an entrance.
    pub(crate) initially_hidden: Vec<TargetKey>,
}

impl SlidePlayer {
    pub fn new(timing: &SlideTiming, text: SlideText) -> Self {
        let mut effects: Vec<AnimationEffect> = timing.main_sequence.clone();
        let main_count = effects.len();
        for sequence in &timing.interactive_sequences {
            effects.extend(sequence.effects.iter().cloned());
        }
        let mut player = Self {
            effects,
            groups: Vec::new(),
            auto_first: false,
            starts: Vec::new(),
            triggers: Vec::new(),
            text,
            initially_hidden: Vec::new(),
        };
        let main: Vec<usize> = (0..main_count).collect();
        player.auto_first = timing
            .main_sequence
            .first()
            .is_some_and(|effect| effect.start != EffectStart::OnClick);
        player.groups = player.group(&main);
        let mut next = main_count;
        for sequence in &timing.interactive_sequences {
            let indices: Vec<usize> = (next..next + sequence.effects.len()).collect();
            next += sequence.effects.len();
            let groups = player.group(&indices);
            player.triggers.push(TriggerSequence {
                trigger: sequence.trigger.clone(),
                groups,
                starts: Vec::new(),
            });
        }
        player.initially_hidden = player.initial_hidden();
        player
    }

    /// Starts the slide at `now`: the automatic first group begins.
    pub fn start(&mut self, now: f64) {
        self.starts.clear();
        for trigger in &mut self.triggers {
            trigger.starts.clear();
        }
        if self.auto_first && !self.groups.is_empty() {
            self.starts.push(now);
        }
    }

    /// Every group played to its end, as the slide stands when the show
    /// comes back to it.
    pub fn finish_all(&mut self, now: f64) {
        self.starts = self
            .groups
            .iter()
            .map(|group| now - finite(group.span) - 1.0)
            .collect();
        // Stagger so each group still begins after the one before.
        let count = self.starts.len();
        for (index, start) in self.starts.iter_mut().enumerate() {
            *start -= (count - index) as f64;
        }
    }

    /// A click: finishes what is still playing, else starts the next group.
    pub fn next(&mut self, now: f64) -> Advance {
        if let Some(index) = self.running_group(now) {
            let span = self.groups[index].span;
            if span.is_finite() {
                self.starts[index] = now - span;
                return Advance::Animated;
            }
        }
        if self.starts.len() < self.groups.len() {
            self.starts.push(now);
            Advance::Animated
        } else {
            Advance::NextSlide
        }
    }

    /// Back one click: the last started group is undone.
    pub fn previous(&mut self) -> Advance {
        let minimum = usize::from(self.auto_first);
        if self.starts.len() > minimum {
            self.starts.pop();
            Advance::Animated
        } else {
            Advance::PreviousSlide
        }
    }

    /// Whether a click on `shape_id` fires a trigger.
    pub fn is_trigger(&self, shape_id: u32) -> bool {
        self.triggers
            .iter()
            .any(|sequence| matches!(sequence.trigger, Trigger::ShapeClick { shape_id: id } if id == shape_id))
    }

    /// A click on a shape during the show: plays its trigger sequence.
    pub fn click_shape(&mut self, shape_id: u32, now: f64) -> bool {
        self.fire(
            now,
            |trigger| matches!(trigger, Trigger::ShapeClick { shape_id: id } if *id == shape_id),
        )
    }

    /// A media bookmark reached: plays the sequences it triggers.
    pub fn bookmark_reached(&mut self, shape_id: u32, bookmark: &str, now: f64) -> bool {
        self.fire(now, |trigger| {
            matches!(trigger, Trigger::Bookmark { shape_id: id, bookmark: name } if *id == shape_id && name == bookmark)
        })
    }

    fn fire(&mut self, now: f64, matches: impl Fn(&Trigger) -> bool) -> bool {
        let mut fired = false;
        for sequence in &mut self.triggers {
            if !matches(&sequence.trigger) || sequence.groups.is_empty() {
                continue;
            }
            if sequence.starts.len() >= sequence.groups.len() {
                sequence.starts.clear();
            }
            sequence.starts.push(now);
            fired = true;
        }
        fired
    }

    /// Whether anything is moving at `now`, so the show keeps repainting.
    pub fn is_animating(&self, now: f64) -> bool {
        self.instances()
            .iter()
            .any(|instance| self.is_running(instance, now))
    }

    /// How many click groups are left on the slide.
    pub fn remaining_clicks(&self) -> usize {
        self.groups.len() - self.starts.len().min(self.groups.len())
    }

    /// The number of click groups (the animation pane's click numbers).
    pub fn click_count(&self) -> usize {
        self.groups.len() - usize::from(self.auto_first)
    }

    /// When everything started so far will have ended; `None` while
    /// something repeats without end.
    pub fn settled_at(&self) -> Option<f64> {
        let mut end: f64 = 0.0;
        for instance in self.instances() {
            let span = self.effect_span(instance.effect);
            if let Some(stop) = instance.stop {
                end = end.max(stop);
            } else if span.is_finite() {
                end = end.max(instance.start + span);
            } else {
                return None;
            }
        }
        Some(end)
    }

    /// The state of every animated target at `now`.
    pub fn frame(&self, now: f64, base: &dyn Fn(&TargetKey) -> TargetBase) -> SlideFrame {
        evaluate(self, &self.instances(), now, base)
    }

    fn running_group(&self, now: f64) -> Option<usize> {
        let index = self.starts.len().checked_sub(1)?;
        let span = self.groups[index].span;
        (now < self.starts[index] + span).then_some(index)
    }

    fn is_running(&self, instance: &Instance, now: f64) -> bool {
        if now < instance.start {
            return true;
        }
        let end = instance.start + self.effect_span(instance.effect);
        let end = instance.stop.map_or(end, |stop| end.min(stop));
        now < end
    }

    /// Every effect whose group has started, with its start time.
    pub(crate) fn instances(&self) -> Vec<Instance> {
        let mut out = Vec::new();
        push_instances(&mut out, &self.groups, &self.starts);
        for sequence in &self.triggers {
            push_instances(&mut out, &sequence.groups, &sequence.starts);
        }
        out.sort_by(|a, b| a.start.total_cmp(&b.start));
        out
    }

    /// Splits effects into click groups, each into steps.
    fn group(&self, indices: &[usize]) -> Vec<Group> {
        let mut groups: Vec<Group> = Vec::new();
        let mut step_start = 0.0;
        let mut step_end: f64 = 0.0;
        for &index in indices {
            let effect = &self.effects[index];
            let span = self.effect_span(index);
            match effect.start {
                EffectStart::OnClick => {
                    groups.push(Group::default());
                    step_start = 0.0;
                    step_end = 0.0;
                }
                EffectStart::WithPrevious => {}
                EffectStart::AfterPrevious => step_start = step_end,
            }
            if groups.is_empty() {
                groups.push(Group::default());
            }
            let group = groups.last_mut().expect("a group");
            let offset = step_start + f64::from(effect.delay_ms);
            let end = offset + span;
            step_end = step_end.max(end);
            group.span = group.span.max(end);
            group.effects.push(Placed {
                effect: index,
                offset,
            });
        }
        groups
    }

    /// One effect's whole length from its own start: duration, repeats,
    /// reversals and the stagger of its letters or words.
    pub(crate) fn effect_span(&self, index: usize) -> f64 {
        let effect = &self.effects[index];
        let simple = self.simple_span(index);
        let repeat = match effect.repeat {
            Repeat::None => 1.0,
            Repeat::Count(count) => count.max(0.0),
            Repeat::UntilNextClick | Repeat::UntilEndOfSlide => return f64::INFINITY,
        };
        simple * repeat
    }

    /// One run through the effect, reversal included, letters staggered.
    pub(crate) fn simple_span(&self, index: usize) -> f64 {
        let effect = &self.effects[index];
        let duration = f64::from(effect.duration_ms) * if effect.auto_reverse { 2.0 } else { 1.0 };
        duration + self.unit_delay(index) * (self.unit_count(index).saturating_sub(1)) as f64
    }

    /// The delay between one letter or word and the next.
    pub(crate) fn unit_delay(&self, index: usize) -> f64 {
        let effect = &self.effects[index];
        match effect.iterate.as_ref().map(|iterate| iterate.interval) {
            Some(IterateInterval::Percent(fraction)) => fraction * f64::from(effect.duration_ms),
            Some(IterateInterval::Milliseconds(ms)) => f64::from(ms),
            None => 0.0,
        }
    }

    /// How many letters or words the effect walks; 1 without iteration.
    pub(crate) fn unit_count(&self, index: usize) -> usize {
        self.units(index).map_or(1, |(_, range)| range.len().max(1))
    }

    /// The letters or words an iterating effect walks, as unit indices in
    /// its shape's text.
    pub(crate) fn units(&self, index: usize) -> Option<(IterateUnit, std::ops::Range<usize>)> {
        let effect = &self.effects[index];
        let iterate = effect.iterate.as_ref()?;
        if iterate.unit == IterateUnit::Element {
            return None;
        }
        let paragraphs = self.text.get(&effect.target.shape_id)?;
        let (first, last) = match effect.target.part {
            TargetPart::Paragraphs { start, end } => (start as usize, end as usize),
            TargetPart::Shape => (0, paragraphs.len().saturating_sub(1)),
            _ => return None,
        };
        let counts: Vec<usize> = paragraphs
            .iter()
            .map(|text| unit_count(text, iterate.unit))
            .collect();
        let start: usize = counts.iter().take(first).sum();
        let end: usize = counts.iter().take(last + 1).sum();
        (end > start).then_some((iterate.unit, start..end))
    }

    /// Targets whose first effect is an entrance start hidden.
    fn initial_hidden(&self) -> Vec<TargetKey> {
        let mut seen: Vec<TargetKey> = Vec::new();
        let mut hidden = Vec::new();
        for effect in &self.effects {
            for key in self.keys(effect) {
                if seen.contains(&key) {
                    continue;
                }
                // A whole shape seen before covers its paragraphs.
                let covered = seen
                    .iter()
                    .any(|other| other.shape_id == key.shape_id && other.part == KeyPart::Shape);
                seen.push(key.clone());
                if !covered && effect.class == EffectClass::Entrance {
                    hidden.push(key);
                }
            }
        }
        hidden
    }

    /// The keys an effect's own target spans.
    pub(crate) fn keys(&self, effect: &AnimationEffect) -> Vec<TargetKey> {
        target_keys(effect.target.shape_id, &effect.target.part)
    }
}

pub(crate) fn target_keys(shape_id: u32, part: &TargetPart) -> Vec<TargetKey> {
    match part {
        TargetPart::Paragraphs { start, end } => (*start..=*end)
            .map(|paragraph| TargetKey {
                shape_id,
                part: KeyPart::Paragraph(paragraph),
            })
            .collect(),
        TargetPart::Background => vec![TargetKey {
            shape_id,
            part: KeyPart::Body,
        }],
        _ => vec![TargetKey {
            shape_id,
            part: KeyPart::Shape,
        }],
    }
}

fn push_instances(out: &mut Vec<Instance>, groups: &[Group], starts: &[f64]) {
    for (index, (group, &start)) in groups.iter().zip(starts).enumerate() {
        // `Until next click` ends where the next group begins.
        let stop = starts.get(index + 1).copied();
        for placed in &group.effects {
            out.push(Instance {
                effect: placed.effect,
                start: start + placed.offset,
                stop,
            });
        }
    }
}

fn finite(span: f64) -> f64 {
    if span.is_finite() {
        span
    } else {
        0.0
    }
}

/// Letters (characters) or words in a paragraph's text.
pub(crate) fn unit_count(text: &str, unit: IterateUnit) -> usize {
    match unit {
        IterateUnit::Letter => text.chars().count(),
        IterateUnit::Word => text.split_whitespace().count(),
        IterateUnit::Element => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jade_pptx_reader::animation::read_slide_animation;

    fn slide(timing: &str) -> String {
        format!(
            r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree/></p:cSld>{timing}</p:sld>"#
        )
    }

    pub(crate) fn fade(id: u32, spid: u32, node_type: &str, delay: u32) -> String {
        format!(
            r#"<p:par><p:cTn id="{id}" presetID="10" presetClass="entr" presetSubtype="0" fill="hold" nodeType="{node_type}"><p:stCondLst><p:cond delay="{delay}"/></p:stCondLst><p:childTnLst><p:set><p:cBhvr><p:cTn id="{a}" dur="1" fill="hold"><p:stCondLst><p:cond delay="0"/></p:stCondLst></p:cTn><p:tgtEl><p:spTgt spid="{spid}"/></p:tgtEl><p:attrNameLst><p:attrName>style.visibility</p:attrName></p:attrNameLst></p:cBhvr><p:to><p:strVal val="visible"/></p:to></p:set><p:animEffect transition="in" filter="fade"><p:cBhvr><p:cTn id="{b}" dur="500"/><p:tgtEl><p:spTgt spid="{spid}"/></p:tgtEl></p:cBhvr></p:animEffect></p:childTnLst></p:cTn></p:par>"#,
            a = id + 100,
            b = id + 200
        )
    }

    pub(crate) fn timing(clicks: &[Vec<Vec<String>>]) -> String {
        let mut body = String::new();
        for click in clicks {
            body.push_str(r#"<p:par><p:cTn id="3" fill="hold"><p:stCondLst><p:cond delay="indefinite"/></p:stCondLst><p:childTnLst>"#);
            for step in click {
                body.push_str(r#"<p:par><p:cTn id="4" fill="hold"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst>"#);
                for effect in step {
                    body.push_str(effect);
                }
                body.push_str("</p:childTnLst></p:cTn></p:par>");
            }
            body.push_str("</p:childTnLst></p:cTn></p:par>");
        }
        format!(
            r#"<p:timing><p:tnLst><p:par><p:cTn id="1" dur="indefinite" restart="never" nodeType="tmRoot"><p:childTnLst><p:seq concurrent="1" nextAc="seek"><p:cTn id="2" dur="indefinite" nodeType="mainSeq"><p:childTnLst>{body}</p:childTnLst></p:cTn><p:prevCondLst><p:cond evt="onPrev" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:prevCondLst><p:nextCondLst><p:cond evt="onNext" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:nextCondLst></p:seq></p:childTnLst></p:cTn></p:par></p:tnLst></p:timing>"#
        )
    }

    pub(crate) fn player(timing_xml: &str) -> SlidePlayer {
        let animation = read_slide_animation(&slide(timing_xml)).expect("reads");
        SlidePlayer::new(&animation.timing, SlideText::new())
    }

    #[test]
    fn clicks_start_groups_and_steps_follow() {
        let xml = timing(&[
            vec![vec![fade(5, 2, "clickEffect", 0)]],
            vec![
                vec![fade(6, 3, "clickEffect", 0)],
                vec![fade(7, 4, "afterEffect", 0)],
            ],
        ]);
        let mut player = player(&xml);
        player.start(0.0);
        assert_eq!(player.click_count(), 2);
        let key = |id| TargetKey {
            shape_id: id,
            part: KeyPart::Shape,
        };
        let frame = player.frame(0.0, &|_| TargetBase::default());
        assert!(!frame.state(&key(2)).visible);
        assert_eq!(player.next(100.0), Advance::Animated);
        let frame = player.frame(350.0, &|_| TargetBase::default());
        let state = frame.state(&key(2));
        assert!(state.visible && (state.opacity - 0.5).abs() < 1e-9);
        assert!(!player.is_animating(700.0));
        assert_eq!(player.next(1000.0), Advance::Animated);
        // The after-previous step waits for the first.
        assert!(
            !player
                .frame(1200.0, &|_| TargetBase::default())
                .state(&key(4))
                .visible
        );
        assert!(
            player
                .frame(1600.0, &|_| TargetBase::default())
                .state(&key(4))
                .visible
        );
        // A click mid-animation finishes it.
        assert_eq!(player.next(1100.0), Advance::Animated);
        assert!(
            player
                .frame(1100.0, &|_| TargetBase::default())
                .state(&key(4))
                .opacity
                > 0.999
        );
        assert_eq!(player.next(3000.0), Advance::NextSlide);
        assert_eq!(player.previous(), Advance::Animated);
        assert!(
            !player
                .frame(3000.0, &|_| TargetBase::default())
                .state(&key(3))
                .visible
        );
        assert_eq!(player.previous(), Advance::Animated);
        assert_eq!(player.previous(), Advance::PreviousSlide);
    }

    #[test]
    fn with_previous_first_effect_starts_with_the_slide() {
        let xml = timing(&[vec![vec![fade(5, 2, "withEffect", 250)]]]);
        let mut player = player(&xml);
        player.start(0.0);
        let key = TargetKey {
            shape_id: 2,
            part: KeyPart::Shape,
        };
        assert!(
            !player
                .frame(100.0, &|_| TargetBase::default())
                .state(&key)
                .visible
        );
        assert!(
            player
                .frame(1000.0, &|_| TargetBase::default())
                .state(&key)
                .visible
        );
        assert_eq!(player.next(2000.0), Advance::NextSlide);
    }
}
