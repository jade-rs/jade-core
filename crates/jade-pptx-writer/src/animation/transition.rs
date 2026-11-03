//! Writing slide transitions: the 画面切り替え gallery, its effect options,
//! duration, sound and advance settings, and すべてに適用.
//!
//! A transition the 2007 schema has, at its default length, is written
//! plainly; a newer one — or any with a chosen duration — inside
//! `mc:AlternateContent` with a 2007 fallback, as PowerPoint writes it.

use jade_pptx_reader::animation::{
    read_slide_animation, SlideTransition, TransitionEffect, TransitionSchema, TransitionSound,
    TransitionSpeed,
};

use crate::{Result, WriteError};

/// One transition in the gallery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionPreset {
    /// Key for the UI's name (`transition.<key>`).
    pub key: &'static str,
    pub schema: TransitionSchema,
    /// Element local name (`fade`, `vortex`, `prstTrans`).
    pub element: &'static str,
    /// For `p15:prstTrans`, its `prst`.
    pub preset: Option<&'static str>,
    /// 弱 / はなやか / ダイナミック コンテンツ: 0, 1, 2.
    pub group: u8,
    /// Default duration, ms.
    pub duration_ms: u32,
    /// What 効果のオプション offers: attribute name and values with keys.
    pub options: &'static [(&'static str, &'static str, &'static str)],
}

const DIR_LRUD: &[(&str, &str, &str)] = &[
    ("dir", "u", "from_bottom"),
    ("dir", "l", "from_right"),
    ("dir", "d", "from_top"),
    ("dir", "r", "from_left"),
];
const DIR_8: &[(&str, &str, &str)] = &[
    ("dir", "r", "from_right"),
    ("dir", "ru", "from_top_right"),
    ("dir", "u", "from_bottom"),
    ("dir", "lu", "from_bottom_right"),
    ("dir", "l", "from_left"),
    ("dir", "ld", "from_top_left"),
    ("dir", "d", "from_top"),
    ("dir", "rd", "from_bottom_left"),
];
const DIR_LR: &[(&str, &str, &str)] = &[("dir", "l", "from_right"), ("dir", "r", "from_left")];
const ORIENT: &[(&str, &str, &str)] = &[("dir", "horz", "horizontal"), ("dir", "vert", "vertical")];
const SPLIT: &[(&str, &str, &str)] = &[
    ("split", "vert-out", "vertical_out"),
    ("split", "horz-out", "horizontal_out"),
    ("split", "vert-in", "vertical_in"),
    ("split", "horz-in", "horizontal_in"),
];
const FADE: &[(&str, &str, &str)] = &[
    ("thruBlk", "0", "smoothly"),
    ("thruBlk", "1", "through_black"),
];
const SHAPE: &[(&str, &str, &str)] = &[
    ("shape", "circle", "circle"),
    ("shape", "diamond", "diamond"),
    ("shape", "plus", "plus"),
];
const ZOOM: &[(&str, &str, &str)] = &[("dir", "in", "in"), ("dir", "out", "out")];
const MORPH: &[(&str, &str, &str)] = &[
    ("option", "byObject", "objects"),
    ("option", "byWord", "words"),
    ("option", "byChar", "characters"),
];
const NONE: &[(&str, &str, &str)] = &[];

macro_rules! tr {
    ($key:expr, $schema:ident, $element:expr, $preset:expr, $group:expr, $ms:expr, $options:expr) => {
        TransitionPreset {
            key: $key,
            schema: TransitionSchema::$schema,
            element: $element,
            preset: $preset,
            group: $group,
            duration_ms: $ms,
            options: $options,
        }
    };
}

/// The gallery, in PowerPoint's order.
pub const TRANSITIONS: &[TransitionPreset] = &[
    tr!("morph", P159, "morph", None, 0, 2000, MORPH),
    tr!("fade", Base, "fade", None, 0, 700, FADE),
    tr!("push", Base, "push", None, 0, 1000, DIR_LRUD),
    tr!("wipe", Base, "wipe", None, 0, 1000, DIR_LRUD),
    tr!("split", Base, "split", None, 0, 1500, SPLIT),
    tr!("reveal", P14, "reveal", None, 0, 2000, DIR_LR),
    tr!("cut", Base, "cut", None, 0, 100, FADE),
    tr!("random_bars", Base, "randomBar", None, 0, 1000, ORIENT),
    tr!("shape", Base, "circle", None, 0, 2000, SHAPE),
    tr!("uncover", Base, "pull", None, 0, 1000, DIR_8),
    tr!("cover", Base, "cover", None, 0, 1000, DIR_8),
    tr!("flash", P14, "flash", None, 0, 1000, NONE),
    tr!(
        "fall_over",
        P15,
        "prstTrans",
        Some("fallOver"),
        1,
        2000,
        NONE
    ),
    tr!("drape", P15, "prstTrans", Some("drape"), 1, 2000, NONE),
    tr!(
        "curtains",
        P15,
        "prstTrans",
        Some("curtains"),
        1,
        2000,
        NONE
    ),
    tr!("wind", P15, "prstTrans", Some("wind"), 1, 2000, NONE),
    tr!(
        "prestige",
        P15,
        "prstTrans",
        Some("prestige"),
        1,
        2000,
        NONE
    ),
    tr!(
        "fracture",
        P15,
        "prstTrans",
        Some("fracture"),
        1,
        2000,
        NONE
    ),
    tr!("crush", P15, "prstTrans", Some("crush"), 1, 2000, NONE),
    tr!("peel_off", P15, "prstTrans", Some("peelOff"), 1, 2000, NONE),
    tr!(
        "page_curl",
        P15,
        "prstTrans",
        Some("pageCurlDouble"),
        1,
        2000,
        NONE
    ),
    tr!(
        "airplane",
        P15,
        "prstTrans",
        Some("airplane"),
        1,
        2000,
        NONE
    ),
    tr!("origami", P15, "prstTrans", Some("origami"), 1, 2000, NONE),
    tr!("dissolve", Base, "dissolve", None, 1, 700, NONE),
    tr!("checkerboard", Base, "checker", None, 1, 1500, ORIENT),
    tr!("blinds", Base, "blinds", None, 1, 1600, ORIENT),
    tr!("clock", Base, "wheel", None, 1, 2000, NONE),
    tr!("ripple", P14, "ripple", None, 1, 1400, NONE),
    tr!("honeycomb", P14, "honeycomb", None, 1, 3000, NONE),
    tr!("glitter", P14, "glitter", None, 1, 2500, DIR_LRUD),
    tr!("vortex", P14, "vortex", None, 1, 3000, DIR_LRUD),
    tr!("shred", P14, "shred", None, 1, 2000, NONE),
    tr!("switch", P14, "switch", None, 1, 1250, DIR_LR),
    tr!("flip", P14, "flip", None, 1, 1000, DIR_LR),
    tr!("gallery", P14, "gallery", None, 1, 1600, DIR_LR),
    tr!("cube", P14, "prism", None, 1, 1000, DIR_LRUD),
    tr!("doors", P14, "doors", None, 1, 1400, ORIENT),
    tr!("box", P14, "prism", None, 1, 1000, DIR_LRUD),
    tr!("comb", Base, "comb", None, 1, 700, ORIENT),
    tr!("zoom", Base, "zoom", None, 1, 1000, ZOOM),
    tr!("random", Base, "random", None, 1, 2000, NONE),
    tr!("pan", P14, "pan", None, 2, 1000, DIR_LRUD),
    tr!("ferris_wheel", P14, "ferris", None, 2, 2000, DIR_LR),
    tr!("conveyor", P14, "conveyor", None, 2, 2000, DIR_LR),
    tr!("window", P14, "window", None, 2, 1300, ORIENT),
    tr!("fly_through", P14, "flythrough", None, 2, 1000, ZOOM),
];

/// The preset an effect is an instance of.
pub fn transition_preset(effect: &TransitionEffect) -> Option<&'static TransitionPreset> {
    match effect.name.as_str() {
        "circle" | "diamond" | "plus" => TRANSITIONS.iter().find(|p| p.key == "shape"),
        "prstTrans" => TRANSITIONS
            .iter()
            .find(|p| p.preset.is_some() && p.preset == effect.attr("prst")),
        "prism" if effect.attr("isInverted").is_some() => {
            TRANSITIONS.iter().find(|p| p.key == "box")
        }
        "fade" if effect.attr("thruBlk").is_some() => TRANSITIONS.iter().find(|p| p.key == "fade"),
        name => TRANSITIONS
            .iter()
            .find(|p| p.element == name && p.schema == effect.schema),
    }
}

/// A gallery transition's effect, with an option (attribute, value) set.
pub fn preset_effect(preset: &TransitionPreset, option: Option<(&str, &str)>) -> TransitionEffect {
    let mut effect = TransitionEffect::new(preset.schema, preset.element);
    if let Some(prst) = preset.preset {
        effect = effect.with_attr("prst", prst);
    }
    if preset.key == "box" {
        effect = effect.with_attr("isInverted", "1");
    }
    if preset.key == "clock" {
        effect = effect.with_attr("spokes", "1");
    }
    // No option picked: the first one, PowerPoint's default.
    let option = option.or_else(|| {
        preset
            .options
            .first()
            .map(|(name, value, _)| (*name, *value))
    });
    match option {
        // なめらかに: PowerPoint writes no attribute.
        Some(("thruBlk", "0")) => {}
        Some(("shape", shape)) => effect.name = shape.to_string(),
        Some(("split", value)) => {
            let (orient, dir) = value.split_once('-').unwrap_or(("vert", "out"));
            effect = effect.with_attr("orient", orient).with_attr("dir", dir);
        }
        Some((name, value)) => effect = effect.with_attr(name, value),
        None => {}
    }
    effect
}

fn escape(value: &str) -> String {
    jade_pptx_reader::animation::node::escape(value, true)
}

fn effect_markup(effect: &TransitionEffect) -> String {
    let prefix = effect.schema.prefix();
    let attrs: String = effect
        .attrs
        .iter()
        .map(|(key, value)| format!(r#" {key}="{}""#, escape(value)))
        .collect();
    format!("<{prefix}:{}{attrs}/>", effect.name)
}

/// The 2007 stand-in PowerPoint writes in the fallback.
fn fallback_effect(effect: &TransitionEffect) -> String {
    if effect.schema == TransitionSchema::Base {
        return effect_markup(effect);
    }
    let dir = effect.attr("dir");
    match effect.name.as_str() {
        "reveal" | "pan" | "conveyor" | "ferris" | "gallery" | "switch" | "flip" => {
            let dir = match dir {
                Some("r") => "r",
                Some("u") => "u",
                Some("d") => "d",
                _ => "l",
            };
            format!(r#"<p:push dir="{dir}"/>"#)
        }
        "vortex" | "glitter" | "shred" | "honeycomb" | "ripple" | "flash" => "<p:fade/>".into(),
        "doors" | "window" => r#"<p:split orient="vert"/>"#.into(),
        "prism" => r#"<p:push dir="l"/>"#.into(),
        "flythrough" => r#"<p:zoom/>"#.into(),
        _ => "<p:fade/>".into(),
    }
}

fn sound_markup(transition: &SlideTransition) -> String {
    if transition.stop_previous_sound {
        return "<p:sndAc><p:endSnd/></p:sndAc>".to_string();
    }
    match &transition.sound {
        Some(sound) if !sound.rel_id.is_empty() => format!(
            r#"<p:sndAc><p:stSnd{}><p:snd r:embed="{}" name="{}"{}/></p:stSnd></p:sndAc>"#,
            if sound.loop_until_next {
                r#" loop="1""#
            } else {
                ""
            },
            escape(&sound.rel_id),
            escape(&sound.name),
            if sound.builtin { r#" builtIn="1""# } else { "" }
        ),
        _ => String::new(),
    }
}

/// `<p:transition>` (or its `mc:AlternateContent`) for `transition`.
pub fn transition_markup(transition: &SlideTransition) -> Option<String> {
    if transition.is_default() {
        return None;
    }
    let speed = transition
        .duration_ms
        .map_or(transition.speed, TransitionSpeed::for_duration);
    let mut attrs = format!(r#" spd="{}""#, speed.as_str());
    let mut advance = String::new();
    if !transition.advance_on_click {
        advance.push_str(r#" advClick="0""#);
    }
    if let Some(ms) = transition.advance_after_ms {
        advance.push_str(&format!(r#" advTm="{ms}""#));
    }
    let sound = sound_markup(transition);
    let effect = transition.effect.as_ref();
    let needs_alternate = transition.duration_ms.is_some()
        || effect.is_some_and(|e| e.schema != TransitionSchema::Base);
    if !needs_alternate {
        attrs.push_str(&advance);
        return Some(format!(
            "<p:transition{attrs}>{}{sound}</p:transition>",
            effect.map(effect_markup).unwrap_or_default()
        ));
    }
    let requires = match effect.map(|e| e.schema) {
        Some(TransitionSchema::P15) => "p15",
        Some(TransitionSchema::P159) => "p159",
        _ => "p14",
    };
    let mut declarations =
        r#" xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main""#.to_string();
    if requires == "p15" {
        declarations
            .push_str(r#" xmlns:p15="http://schemas.microsoft.com/office/powerpoint/2012/main""#);
    }
    if requires == "p159" {
        declarations.push_str(
            r#" xmlns:p159="http://schemas.microsoft.com/office/powerpoint/2015/09/main""#,
        );
    }
    let duration = transition
        .duration_ms
        .map(|ms| format!(r#" p14:dur="{ms}""#))
        .unwrap_or_default();
    Some(format!(
        r#"<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice{declarations} Requires="{requires}"><p:transition{attrs}{duration}{advance}>{}{sound}</p:transition></mc:Choice><mc:Fallback><p:transition{attrs}{advance}>{}{sound}</p:transition></mc:Fallback></mc:AlternateContent>"#,
        effect.map(effect_markup).unwrap_or_default(),
        effect.map(fallback_effect).unwrap_or_default(),
    ))
}

/// A sound for 画面切り替え › サウンド.
#[derive(Debug, Clone, PartialEq)]
pub enum SoundChoice {
    /// [サウンドなし].
    None,
    /// [前のサウンドを停止].
    StopPrevious,
    /// その他のサウンド: a file's name and bytes.
    File { name: String, bytes: Vec<u8> },
    /// A sound already in the package (another slide's).
    Part { name: String, part: String },
}

/// One change on the 画面切り替え tab, for one or more slides.
#[derive(Debug, Clone, PartialEq)]
pub enum TransitionEdit {
    /// The gallery: an effect (or なし), at the preset's default length
    /// unless the slide already had a chosen one.
    Effect(Option<TransitionEffect>),
    /// 継続時間, ms.
    Duration(u32),
    /// クリック時.
    AdvanceOnClick(bool),
    /// 自動 after ms, or off.
    AdvanceAfter(Option<u32>),
    Sound(SoundChoice),
    /// サウンドをループ.
    LoopSound(bool),
    /// すべてに適用: the whole transition, as on another slide.
    Replace(SlideTransition),
}

fn apply(current: Option<SlideTransition>, edit: &TransitionEdit) -> SlideTransition {
    let mut transition = current.unwrap_or_default();
    match edit {
        TransitionEdit::Effect(effect) => {
            let had_duration = transition.duration_ms;
            transition.effect = effect.clone();
            // A newly picked effect plays at its own length.
            transition.duration_ms = effect
                .as_ref()
                .and_then(transition_preset)
                .map(|preset| preset.duration_ms)
                .or(had_duration.filter(|_| effect.is_some()));
            if effect.is_none() {
                transition.duration_ms = None;
            }
            transition.speed = transition
                .duration_ms
                .map_or(TransitionSpeed::Fast, TransitionSpeed::for_duration);
        }
        TransitionEdit::Duration(ms) => {
            transition.duration_ms = Some((*ms).clamp(10, 59_000));
            transition.speed = TransitionSpeed::for_duration(*ms);
        }
        TransitionEdit::AdvanceOnClick(on) => transition.advance_on_click = *on,
        TransitionEdit::AdvanceAfter(ms) => transition.advance_after_ms = *ms,
        TransitionEdit::LoopSound(on) => {
            if let Some(sound) = transition.sound.as_mut() {
                sound.loop_until_next = *on;
            }
        }
        TransitionEdit::Sound(_) => {}
        TransitionEdit::Replace(replacement) => transition = replacement.clone(),
    }
    transition
}

/// Applies a transition edit to each of `slides`.
pub fn set_transitions(bytes: &[u8], slides: &[usize], edit: &TransitionEdit) -> Result<Vec<u8>> {
    let mut package = jade_opc::OpcPackage::open(bytes)?;
    let presentation = jade_pptx_reader::read_pptx(bytes)?;
    let mut sound_part: Option<(String, String)> = None;
    let mut changed = false;
    for &index in slides {
        let entry = presentation
            .slides
            .get(index)
            .ok_or(WriteError::NoSlide(index))?;
        let part = entry.part_name.clone();
        let xml = package.required_part_str(&part)?;
        let current = read_slide_animation(&xml)?.transition;
        let mut transition = apply(current, edit);
        // Sounds are relationships of each slide.
        let sound_choice = match edit {
            TransitionEdit::Sound(choice) => Some(choice.clone()),
            TransitionEdit::Replace(replacement) => match &replacement.sound {
                Some(TransitionSound {
                    part: Some(sound), ..
                }) => Some(SoundChoice::Part {
                    name: replacement
                        .sound
                        .as_ref()
                        .map_or(String::new(), |s| s.name.clone()),
                    part: sound.clone(),
                }),
                _ if replacement.stop_previous_sound => Some(SoundChoice::StopPrevious),
                _ => Some(SoundChoice::None),
            },
            _ => None,
        };
        match sound_choice {
            Some(SoundChoice::None) => {
                transition.sound = None;
                transition.stop_previous_sound = false;
            }
            Some(SoundChoice::StopPrevious) => {
                transition.sound = None;
                transition.stop_previous_sound = true;
            }
            Some(SoundChoice::File { name, bytes }) => {
                let target = match &sound_part {
                    Some((_, part)) => part.clone(),
                    None => {
                        let installed =
                            crate::xml::animation::install_sound(&mut package, &name, &bytes)?;
                        sound_part = Some((name.clone(), installed.clone()));
                        installed
                    }
                };
                let rel_id =
                    crate::xml::animation::audio_relationship(&mut package, &part, &target)?;
                let looped = transition.sound.as_ref().is_some_and(|s| s.loop_until_next);
                transition.sound = Some(TransitionSound {
                    rel_id,
                    name: name.clone(),
                    builtin: false,
                    loop_until_next: looped,
                    part: Some(target),
                });
                transition.stop_previous_sound = false;
            }
            Some(SoundChoice::Part { name, part: target }) => {
                let rel_id =
                    crate::xml::animation::audio_relationship(&mut package, &part, &target)?;
                let looped = transition.sound.as_ref().is_some_and(|s| s.loop_until_next);
                transition.sound = Some(TransitionSound {
                    rel_id,
                    name,
                    builtin: false,
                    loop_until_next: looped,
                    part: Some(target),
                });
                transition.stop_previous_sound = false;
            }
            None => {}
        }
        let markup = transition_markup(&transition);
        let updated = crate::xml::animation::replace_transition(&xml, markup.as_deref())?;
        if updated != xml {
            package.replace(&part, updated.into_bytes());
            changed = true;
        }
    }
    if !changed {
        return Ok(bytes.to_vec());
    }
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}

/// Every slide's index, for すべてに適用.
pub fn all_slides(bytes: &[u8]) -> Result<Vec<usize>> {
    Ok((0..jade_pptx_reader::read_pptx(bytes)?.slides.len()).collect())
}
