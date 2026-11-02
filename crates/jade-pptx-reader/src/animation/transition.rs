//! `<p:transition>`: how a slide comes in during a slide show and when the
//! show moves past it.
//!
//! PowerPoint 2010 and later write any transition the 2007 schema lacks —
//! and any explicit duration — inside `mc:AlternateContent`: the `Choice`
//! carries the real transition (`p14:vortex`, `p159:morph`, `p14:dur`), the
//! `Fallback` a 2007 stand-in. The reader takes the choice it understands.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::node::XmlNode;

pub const NS_P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
pub const NS_P14: &str = "http://schemas.microsoft.com/office/powerpoint/2010/main";
pub const NS_P15: &str = "http://schemas.microsoft.com/office/powerpoint/2012/main";
pub const NS_P159: &str = "http://schemas.microsoft.com/office/powerpoint/2015/09/main";
pub const NS_MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

/// `spd`: the 2007 speeds, which are also the durations PowerPoint plays
/// a transition at when no `p14:dur` says otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TransitionSpeed {
    Slow,
    Medium,
    #[default]
    Fast,
}

impl TransitionSpeed {
    pub fn parse(value: &str) -> Self {
        match value {
            "slow" => Self::Slow,
            "med" => Self::Medium,
            _ => Self::Fast,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Slow => "slow",
            Self::Medium => "med",
            Self::Fast => "fast",
        }
    }

    pub fn duration_ms(self) -> u32 {
        match self {
            Self::Slow => 1000,
            Self::Medium => 750,
            Self::Fast => 500,
        }
    }

    /// The speed PowerPoint writes in the fallback for a duration.
    pub fn for_duration(ms: u32) -> Self {
        if ms >= 1000 {
            Self::Slow
        } else if ms >= 750 {
            Self::Medium
        } else {
            Self::Fast
        }
    }
}

/// The effect element: `fade`, `push`, `p14:vortex`, `p159:morph`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionEffect {
    /// Local name.
    pub name: String,
    /// Which schema it comes from.
    pub schema: TransitionSchema,
    /// Attributes by local name (`dir`, `thruBlk`, `option`, `pattern`).
    pub attrs: Vec<(String, String)>,
}

impl TransitionEffect {
    pub fn new(schema: TransitionSchema, name: &str) -> Self {
        Self {
            name: name.to_string(),
            schema,
            attrs: Vec::new(),
        }
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn with_attr(mut self, name: &str, value: &str) -> Self {
        match self.attrs.iter_mut().find(|(key, _)| key == name) {
            Some((_, existing)) => *existing = value.to_string(),
            None => self.attrs.push((name.to_string(), value.to_string())),
        }
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionSchema {
    /// ECMA-376 (`p:`).
    Base,
    /// PowerPoint 2010 (`p14:`).
    P14,
    /// PowerPoint 2013 (`p15:`).
    P15,
    /// PowerPoint 2016+ (`p159:`): morph.
    P159,
}

impl TransitionSchema {
    pub fn namespace(self) -> &'static str {
        match self {
            Self::Base => NS_P,
            Self::P14 => NS_P14,
            Self::P15 => NS_P15,
            Self::P159 => NS_P159,
        }
    }

    pub fn prefix(self) -> &'static str {
        match self {
            Self::Base => "p",
            Self::P14 => "p14",
            Self::P15 => "p15",
            Self::P159 => "p159",
        }
    }

    fn from_namespace(uri: &str) -> Option<Self> {
        match uri {
            NS_P | "http://purl.oclc.org/ooxml/presentationml/main" => Some(Self::Base),
            NS_P14 => Some(Self::P14),
            NS_P15 => Some(Self::P15),
            NS_P159 => Some(Self::P159),
            _ => None,
        }
    }
}

/// A sound played as the slide comes in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionSound {
    /// `r:embed` of the sound's relationship.
    pub rel_id: String,
    /// The sound's name as PowerPoint lists it.
    pub name: String,
    /// `builtIn="1"`: one of PowerPoint's own sounds.
    pub builtin: bool,
    /// `loop="1"`: 次のサウンドまで繰り返す.
    pub loop_until_next: bool,
    /// The sound's part, once the relationship is resolved.
    pub part: Option<String>,
}

/// A slide's transition and advance settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlideTransition {
    /// `None`: no effect (なし), only advance or sound settings.
    pub effect: Option<TransitionEffect>,
    pub speed: TransitionSpeed,
    /// `p14:dur`, ms.
    pub duration_ms: Option<u32>,
    /// `advClick`: クリック時 (default on).
    pub advance_on_click: bool,
    /// `advTm`: 自動 after this many ms.
    pub advance_after_ms: Option<u32>,
    pub sound: Option<TransitionSound>,
    /// `<p:sndAc><p:endSnd/>`: 前のサウンドを停止.
    pub stop_previous_sound: bool,
}

impl Default for SlideTransition {
    fn default() -> Self {
        Self {
            effect: None,
            speed: TransitionSpeed::Fast,
            duration_ms: None,
            advance_on_click: true,
            advance_after_ms: None,
            sound: None,
            stop_previous_sound: false,
        }
    }
}

impl SlideTransition {
    /// How long the transition plays, ms.
    pub fn effective_duration_ms(&self) -> u32 {
        self.duration_ms.unwrap_or_else(|| {
            // PowerPoint's own defaults for effects that have a natural
            // length longer than the speed implies.
            match self.effect.as_ref().map(|e| e.name.as_str()) {
                Some("morph") => 2000,
                _ => self.speed.duration_ms(),
            }
        })
    }

    /// Nothing is set: no effect, sound or advance timing.
    pub fn is_default(&self) -> bool {
        self.effect.is_none()
            && self.sound.is_none()
            && !self.stop_previous_sound
            && self.advance_on_click
            && self.advance_after_ms.is_none()
            && self.duration_ms.is_none()
    }
}

/// In-scope namespace declarations, prefix → URI.
#[derive(Debug, Clone, Default)]
pub struct Namespaces(HashMap<String, String>);

impl Namespaces {
    pub fn declare_from(&mut self, node: &XmlNode) {
        for (key, value) in &node.attrs {
            if let Some(prefix) = key.strip_prefix("xmlns:") {
                self.0.insert(prefix.to_string(), value.clone());
            } else if key == "xmlns" {
                self.0.insert(String::new(), value.clone());
            }
        }
    }

    pub fn uri(&self, prefix: Option<&str>) -> Option<&str> {
        self.0.get(prefix.unwrap_or("")).map(String::as_str)
    }
}

/// Parses a `<p:transition>` element; `namespaces` are those in scope.
pub fn parse_transition(node: &XmlNode, namespaces: &Namespaces) -> SlideTransition {
    let mut scope = namespaces.clone();
    scope.declare_from(node);
    let mut transition = SlideTransition {
        speed: TransitionSpeed::parse(node.attr("spd").unwrap_or("fast")),
        duration_ms: node
            .attrs
            .iter()
            .find(|(key, _)| {
                key.ends_with(":dur")
                    && key
                        .split_once(':')
                        .and_then(|(prefix, _)| scope.uri(Some(prefix)))
                        == Some(NS_P14)
            })
            .and_then(|(_, value)| value.parse().ok()),
        advance_on_click: node
            .attr("advClick")
            .is_none_or(|v| v != "0" && v != "false"),
        advance_after_ms: node.attr("advTm").and_then(|v| v.parse().ok()),
        ..SlideTransition::default()
    };
    for child in node.elements() {
        let mut child_scope = scope.clone();
        child_scope.declare_from(child);
        let schema = child_scope
            .uri(child.prefix())
            .and_then(TransitionSchema::from_namespace);
        match (schema, child.local_name()) {
            (Some(TransitionSchema::Base), "sndAc") => {
                if let Some(start) = child.child("stSnd") {
                    if let Some(sound) = start.child("snd") {
                        transition.sound = Some(TransitionSound {
                            rel_id: sound.attr("embed").unwrap_or("").to_string(),
                            name: sound.attr("name").unwrap_or("").to_string(),
                            builtin: sound
                                .attr("builtIn")
                                .is_some_and(|v| v == "1" || v == "true"),
                            loop_until_next: start
                                .attr("loop")
                                .is_some_and(|v| v == "1" || v == "true"),
                            part: None,
                        });
                    }
                }
                transition.stop_previous_sound = child.child("endSnd").is_some();
            }
            (Some(TransitionSchema::Base), "extLst") => {}
            (Some(schema), name) => {
                transition.effect = Some(TransitionEffect {
                    name: name.to_string(),
                    schema,
                    attrs: child
                        .attrs
                        .iter()
                        .filter(|(key, _)| !key.starts_with("xmlns"))
                        .map(|(key, value)| {
                            (
                                key.rsplit_once(':')
                                    .map_or(key.as_str(), |(_, l)| l)
                                    .to_string(),
                                value.clone(),
                            )
                        })
                        .collect(),
                });
            }
            (None, _) => {}
        }
    }
    transition
}

/// Whether a `Requires` list names only namespaces this reader knows.
pub fn requires_understood(requires: &str, scope: &Namespaces) -> bool {
    requires.split_whitespace().all(|prefix| {
        matches!(
            scope.uri(Some(prefix)),
            Some(NS_P14) | Some(NS_P15) | Some(NS_P159) | Some(NS_P)
        )
    })
}
