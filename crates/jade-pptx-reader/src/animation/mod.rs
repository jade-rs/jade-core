//! A slide's animations and transition: `<p:timing>` and `<p:transition>`.
//!
//! Read on their own, apart from the shape tree, by the slide show and by
//! the animation and transition editors. [`read_slide_animation`] reads one
//! slide part; [`read_deck_animations`] every slide of a package, sounds
//! resolved to their parts.

pub mod node;
pub mod timing;
pub mod transition;
pub mod zoom;

pub use node::{XmlChild, XmlNode};
pub use timing::{
    parse_effect, parse_target, parse_timing, AfterEffect, AnimValue, AnimationEffect,
    AnimationTarget, Behavior, BehaviorKind, BehaviorTiming, BuildEntry, BuildKind, EffectClass,
    EffectStart, InteractiveSequence, Iterate, IterateInterval, IterateUnit, Keyframe, Repeat,
    SlideTiming, TargetPart, Trigger,
};
pub use transition::{
    parse_transition, Namespaces, SlideTransition, TransitionEffect, TransitionSchema,
    TransitionSound, TransitionSpeed,
};
pub use zoom::{read_zooms, ZoomFrame, ZoomKind};

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// What a slide's timing and transition markup says.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SlideAnimation {
    pub timing: SlideTiming,
    pub transition: Option<SlideTransition>,
    /// Summary, slide and section zooms on the slide.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zooms: Vec<ZoomFrame>,
}

/// Reads a slide part's `<p:timing>`, `<p:transition>` and zooms.
pub fn read_slide_animation(slide_xml: &str) -> Result<SlideAnimation> {
    let root = XmlNode::parse(slide_xml)?;
    let mut scope = Namespaces::default();
    scope.declare_from(&root);
    let mut out = SlideAnimation::default();
    for child in root.elements() {
        match child.local_name() {
            "timing" => out.timing = parse_timing(child),
            "transition" => out.transition = Some(parse_transition(child, &scope)),
            "AlternateContent" => {
                if let Some(transition) = alternate_transition(child, &scope) {
                    out.transition = Some(transition);
                }
            }
            _ => {}
        }
    }
    out.zooms = read_zooms(&root, &scope);
    Ok(out)
}

/// The transition inside an `mc:AlternateContent`: the first choice whose
/// requirements are understood, else the fallback.
fn alternate_transition(alternate: &XmlNode, scope: &Namespaces) -> Option<SlideTransition> {
    let mut scope = scope.clone();
    scope.declare_from(alternate);
    for choice in alternate.children_named("Choice") {
        let mut choice_scope = scope.clone();
        choice_scope.declare_from(choice);
        if !transition::requires_understood(choice.attr("Requires").unwrap_or(""), &choice_scope) {
            continue;
        }
        if let Some(node) = choice.child("transition") {
            return Some(parse_transition(node, &choice_scope));
        }
    }
    let fallback = alternate.child("Fallback")?;
    let mut fallback_scope = scope.clone();
    fallback_scope.declare_from(fallback);
    fallback
        .child("transition")
        .map(|node| parse_transition(node, &fallback_scope))
}

/// Every slide's animation, in presentation order, transition sounds
/// resolved to their parts.
pub fn read_deck_animations(bytes: &[u8]) -> Result<Vec<SlideAnimation>> {
    let package = jade_opc::OpcPackage::open(bytes)?;
    let presentation = crate::read_pptx(bytes)?;
    let mut slides = Vec::with_capacity(presentation.slides.len());
    for entry in &presentation.slides {
        let xml = package.required_part_str(&entry.part_name)?;
        let mut animation = read_slide_animation(&xml)?;
        if let Some(sound) = animation
            .transition
            .as_mut()
            .and_then(|transition| transition.sound.as_mut())
        {
            if let Some(rels) = package.part_str(&jade_opc::rels_part_name(&entry.part_name))? {
                let relationships = jade_opc::parse_relationships(&rels)?;
                sound.part = relationships
                    .target(&sound.rel_id)
                    .map(|target| jade_opc::resolve_target(&entry.part_name, target));
            }
        }
        slides.push(animation);
    }
    Ok(slides)
}

#[cfg(test)]
mod tests;
