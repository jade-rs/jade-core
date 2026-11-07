//! Writes a [`SlideTiming`] back as `<p:timing>`, the way PowerPoint lays
//! it out: a `tmRoot` over the `mainSeq`, one `<p:par>` per click, one per
//! "after previous" step inside it (its delay the time the steps before it
//! take), and the effects themselves; then each trigger's
//! `interactiveSeq`; then `<p:bldLst>`. Time-node ids are numbered in
//! document order, as PowerPoint numbers them.

use std::collections::HashMap;

use jade_pptx_reader::animation::{
    AnimationEffect, BuildEntry, BuildKind, EffectStart, Repeat, SlideTiming, TargetPart, Trigger,
    XmlNode,
};

/// What the build list needs to know about a shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeKind {
    /// `p:sp` (with or without text).
    Shape,
    /// A chart frame.
    Chart,
    /// A SmartArt frame.
    Diagram,
    /// Pictures, groups, tables, media: no build entry.
    Other,
}

/// How long an effect occupies its step: delay plus its plays.
pub fn effect_span_ms(effect: &AnimationEffect) -> u32 {
    let once = effect.duration_ms.max(1);
    let plays = match effect.repeat {
        Repeat::Count(count) => count.max(1.0),
        _ => 1.0,
    };
    let plays = if effect.auto_reverse {
        plays * 2.0
    } else {
        plays
    };
    effect.delay_ms + (f64::from(once) * plays).round() as u32
}

fn condition(delay: &str) -> String {
    format!(r#"<p:cond delay="{delay}"/>"#)
}

/// Groups effects into clicks of steps of effects.
fn groups(effects: &[AnimationEffect]) -> Vec<(bool, Vec<Vec<&AnimationEffect>>)> {
    let mut groups: Vec<(bool, Vec<Vec<&AnimationEffect>>)> = Vec::new();
    for effect in effects {
        match effect.start {
            EffectStart::OnClick => groups.push((false, vec![vec![effect]])),
            EffectStart::WithPrevious => match groups.last_mut() {
                Some((_, steps)) => steps
                    .last_mut()
                    .expect("a click group has a step")
                    .push(effect),
                None => groups.push((true, vec![vec![effect]])),
            },
            EffectStart::AfterPrevious => match groups.last_mut() {
                Some((_, steps)) => steps.push(vec![effect]),
                None => groups.push((true, vec![vec![effect]])),
            },
        }
    }
    groups
}

/// An effect's `<p:par>` with its start written into it.
fn effect_markup(effect: &AnimationEffect) -> String {
    let mut node = effect.node.clone();
    if let Some(ctn) = node.child_mut("cTn") {
        ctn.remove_attr("grpId");
        ctn.set_attr("nodeType", effect.start.node_type());
        // PowerPoint writes grpId just before nodeType.
        if let Some(group) = effect.group_id {
            let at = ctn
                .attrs
                .iter()
                .position(|(key, _)| key == "nodeType")
                .unwrap_or(ctn.attrs.len());
            ctn.attrs
                .insert(at, ("grpId".to_string(), group.to_string()));
        }
    }
    node.to_xml()
}

fn steps_markup(steps: &[Vec<&AnimationEffect>]) -> String {
    let mut out = String::new();
    let mut at = 0u32;
    for step in steps {
        out.push_str(&format!(
            r#"<p:par><p:cTn id="0" fill="hold"><p:stCondLst>{}</p:stCondLst><p:childTnLst>"#,
            condition(&at.to_string())
        ));
        for effect in step {
            out.push_str(&effect_markup(effect));
        }
        out.push_str("</p:childTnLst></p:cTn></p:par>");
        at += step.iter().map(|e| effect_span_ms(e)).max().unwrap_or(0);
    }
    out
}

const MAIN_SEQ_ID: &str = "{MAIN_SEQ_ID}";

fn main_sequence(effects: &[AnimationEffect]) -> String {
    let mut clicks = String::new();
    for (automatic, steps) in groups(effects) {
        let start = if automatic {
            format!(
                r#"{}<p:cond evt="onBegin" delay="0"><p:tn val="{MAIN_SEQ_ID}"/></p:cond>"#,
                condition("indefinite")
            )
        } else {
            condition("indefinite")
        };
        clicks.push_str(&format!(
            r#"<p:par><p:cTn id="0" fill="hold"><p:stCondLst>{start}</p:stCondLst><p:childTnLst>{}</p:childTnLst></p:cTn></p:par>"#,
            steps_markup(&steps)
        ));
    }
    format!(
        r#"<p:seq concurrent="1" nextAc="seek"><p:cTn id="0" dur="indefinite" nodeType="mainSeq"><p:childTnLst>{clicks}</p:childTnLst></p:cTn><p:prevCondLst><p:cond evt="onPrev" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:prevCondLst><p:nextCondLst><p:cond evt="onNext" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:nextCondLst></p:seq>"#
    )
}

fn trigger_condition(trigger: &Trigger, event: &str) -> String {
    match trigger {
        Trigger::ShapeClick { shape_id } => format!(
            r#"<p:cond evt="{event}" delay="0"><p:tgtEl><p:spTgt spid="{shape_id}"/></p:tgtEl></p:cond>"#
        ),
        Trigger::Bookmark { shape_id, bookmark } => format!(
            r#"<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main" Requires="p14"><p:cond evt="onMediaBookmark" delay="0"><p:tgtEl><p14:bmkTgt spid="{shape_id}" bmkName="{}"/></p:tgtEl></p:cond></mc:Choice><mc:Fallback><p:cond evt="{event}" delay="0"><p:tgtEl><p:spTgt spid="{shape_id}"/></p:tgtEl></p:cond></mc:Fallback></mc:AlternateContent>"#,
            jade_pptx_reader::animation::node::escape(bookmark, true)
        ),
    }
}

fn interactive_sequence(trigger: &Trigger, effects: &[AnimationEffect]) -> String {
    let mut clicks = String::new();
    for (index, (_, steps)) in groups(effects).into_iter().enumerate() {
        clicks.push_str(&format!(
            r#"<p:par><p:cTn id="0" fill="hold"><p:stCondLst>{}</p:stCondLst><p:childTnLst>{}</p:childTnLst></p:cTn></p:par>"#,
            condition(if index == 0 { "0" } else { "indefinite" }),
            steps_markup(&steps)
        ));
    }
    let shape_id = match trigger {
        Trigger::ShapeClick { shape_id } | Trigger::Bookmark { shape_id, .. } => *shape_id,
    };
    format!(
        r#"<p:seq concurrent="1" nextAc="seek"><p:cTn id="0" restart="whenNotActive" fill="hold" evtFilter="cancelBubble" nodeType="interactiveSeq"><p:stCondLst>{}</p:stCondLst><p:endSync evt="end" delay="0"><p:rtn val="all"/></p:endSync><p:childTnLst>{clicks}</p:childTnLst></p:cTn><p:nextCondLst><p:cond evt="onClick" delay="0"><p:tgtEl><p:spTgt spid="{shape_id}"/></p:tgtEl></p:cond></p:nextCondLst></p:seq>"#,
        trigger_condition(trigger, "onClick")
    )
}

/// The build list for the effects: entries already there are kept when
/// their shape and group are still animated; new ones are written as
/// PowerPoint writes them.
pub fn build_list(timing: &SlideTiming, kinds: &HashMap<u32, ShapeKind>) -> Vec<BuildEntry> {
    let mut wanted: Vec<(u32, u32, bool)> = Vec::new();
    for effect in timing.all_effects() {
        let Some(group) = effect.group_id else {
            continue;
        };
        let by_paragraph = matches!(effect.target.part, TargetPart::Paragraphs { .. });
        match wanted
            .iter_mut()
            .find(|(shape, g, _)| *shape == effect.target.shape_id && *g == group)
        {
            Some(entry) => entry.2 |= by_paragraph,
            None => wanted.push((effect.target.shape_id, group, by_paragraph)),
        }
    }
    let mut out = Vec::new();
    for (shape_id, group_id, by_paragraph) in wanted {
        let existing = timing
            .builds
            .iter()
            .find(|b| b.shape_id == shape_id && b.group_id == group_id);
        let kind = kinds.get(&shape_id).copied().unwrap_or(ShapeKind::Other);
        let entry = match (existing, kind) {
            (Some(existing), _) => {
                let mut existing = existing.clone();
                if let BuildKind::Paragraph { build, .. } = &existing.kind {
                    if by_paragraph && build == "whole" {
                        existing.node.set_attr("build", "p");
                        existing.node.remove_attr("animBg");
                    }
                }
                Some(existing)
            }
            (None, ShapeKind::Shape) => {
                let markup = if by_paragraph {
                    format!(r#"<p:bldP spid="{shape_id}" grpId="{group_id}" build="p"/>"#)
                } else {
                    format!(r#"<p:bldP spid="{shape_id}" grpId="{group_id}" animBg="1"/>"#)
                };
                Some(markup)
                    .and_then(|m| XmlNode::parse(&m).ok())
                    .map(|node| BuildEntry {
                        shape_id,
                        group_id,
                        kind: BuildKind::Paragraph {
                            build: if by_paragraph { "p" } else { "whole" }.to_string(),
                            level: None,
                            animate_background: !by_paragraph,
                            reverse: false,
                        },
                        node,
                    })
            }
            (None, ShapeKind::Chart | ShapeKind::Diagram) => {
                let chart_steps = timing.all_effects().find_map(|e| match &e.target.part {
                    TargetPart::Chart { step, .. }
                        if e.target.shape_id == shape_id && e.group_id == Some(group_id) =>
                    {
                        Some(step.clone())
                    }
                    _ => None,
                });
                let sub = match (kind, chart_steps.as_deref()) {
                    (ShapeKind::Chart, Some("series")) => Some("series"),
                    (ShapeKind::Chart, Some("category")) => Some("category"),
                    (ShapeKind::Chart, Some("ptInSeries")) => Some("seriesEl"),
                    (ShapeKind::Chart, Some("ptInCategory")) => Some("categoryEl"),
                    _ => None,
                };
                let body = match (kind, sub) {
                    (ShapeKind::Chart, Some(sub)) => {
                        format!(r#"<p:bldSub><a:bldChart bld="{sub}"/></p:bldSub>"#)
                    }
                    (ShapeKind::Diagram, _) if diagram_by_element(timing, shape_id) => {
                        r#"<p:bldSub><a:bldDgm bld="one"/></p:bldSub>"#.to_string()
                    }
                    _ => "<p:bldAsOne/>".to_string(),
                };
                let markup = format!(
                    r#"<p:bldGraphic xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" spid="{shape_id}" grpId="{group_id}">{body}</p:bldGraphic>"#
                );
                XmlNode::parse(&markup).ok().map(|mut node| {
                    node.attrs.retain(|(key, _)| !key.starts_with("xmlns"));
                    BuildEntry {
                        shape_id,
                        group_id,
                        kind: BuildKind::Graphic {
                            as_one: sub.is_none(),
                            build: sub.map(str::to_string),
                        },
                        node,
                    }
                })
            }
            (None, ShapeKind::Other) => None,
        };
        out.extend(entry);
    }
    out
}

fn diagram_by_element(timing: &SlideTiming, shape_id: u32) -> bool {
    timing.all_effects().any(|e| {
        e.target.shape_id == shape_id && matches!(e.target.part, TargetPart::Diagram { .. })
    })
}

/// `<p:timing>` for `timing`, or `None` when nothing is left to write.
pub fn timing_markup(timing: &SlideTiming, kinds: &HashMap<u32, ShapeKind>) -> Option<String> {
    if timing.is_empty() {
        return None;
    }
    let mut children = String::new();
    if !timing.main_sequence.is_empty() {
        children.push_str(&main_sequence(&timing.main_sequence));
    }
    let others_at = |position: usize, out: &mut String| {
        for (node, _) in timing
            .other_nodes
            .iter()
            .zip(&timing.other_positions)
            .filter(|(_, at)| **at == position)
        {
            out.push_str(&node.to_xml());
        }
    };
    for (index, sequence) in timing.interactive_sequences.iter().enumerate() {
        others_at(index, &mut children);
        children.push_str(&interactive_sequence(&sequence.trigger, &sequence.effects));
    }
    let after = timing.interactive_sequences.len();
    for (node, at) in timing.other_nodes.iter().zip(
        timing
            .other_positions
            .iter()
            .copied()
            .chain(std::iter::repeat(after)),
    ) {
        if at >= after {
            children.push_str(&node.to_xml());
        }
    }
    let builds = build_list(timing, kinds);
    let build_list = if builds.is_empty() {
        String::new()
    } else {
        format!(
            "<p:bldLst>{}</p:bldLst>",
            builds.iter().map(|b| b.node.to_xml()).collect::<String>()
        )
    };
    let markup = format!(
        r#"<p:timing><p:tnLst><p:par><p:cTn id="0" dur="indefinite" restart="never" nodeType="tmRoot"><p:childTnLst>{children}</p:childTnLst></p:cTn></p:par></p:tnLst>{build_list}</p:timing>"#
    );
    Some(number_time_nodes(&markup))
}

/// Numbers every `<p:cTn id>` in document order from 1, and points the
/// automatic first click at the main sequence's node.
fn number_time_nodes(markup: &str) -> String {
    let mut out = String::with_capacity(markup.len() + 64);
    let mut next = 1u32;
    let mut main_id = None;
    let mut rest = markup;
    while let Some(at) = rest.find("<p:cTn ") {
        out.push_str(&rest[..at]);
        let tag_end = rest[at..].find('>').map_or(rest.len(), |end| at + end);
        let tag = &rest[at..tag_end];
        let renumbered = match tag.find(" id=\"") {
            Some(id_at) => {
                let value_start = id_at + 5;
                let value_end = tag[value_start..]
                    .find('"')
                    .map_or(tag.len(), |end| value_start + end);
                if tag.contains("nodeType=\"mainSeq\"") {
                    main_id = Some(next);
                }
                let replaced = format!("{}{}{}", &tag[..value_start], next, &tag[value_end..]);
                next += 1;
                replaced
            }
            None => tag.to_string(),
        };
        out.push_str(&renumbered);
        rest = &rest[tag_end..];
    }
    out.push_str(rest);
    match main_id {
        Some(id) => out.replace(MAIN_SEQ_ID, &id.to_string()),
        None => out,
    }
}
