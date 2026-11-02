use super::transition::{
    preset_effect, set_transitions, transition_markup, SoundChoice, TransitionEdit, TRANSITIONS,
};
use super::*;
use jade_pptx_reader::animation::{BehaviorKind, EffectClass, TransitionSchema};

fn deck() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-deck.pptx"
    ))
    .unwrap()
}

fn add(class: PresetClass, id: u32, shape: u32, start: Option<EffectStart>) -> AnimationEdit {
    AnimationEdit::Add {
        class,
        preset_id: id,
        params: EffectParams::default(),
        targets: vec![AnimationTarget::shape(shape)],
        start,
        after: None,
    }
}

fn timing(bytes: &[u8]) -> SlideTiming {
    slide_animation(bytes, 0).unwrap().timing
}

fn slide_xml(bytes: &[u8]) -> String {
    let (_, _, xml) = crate::slide_xml(bytes, 0).unwrap();
    xml
}

#[test]
fn adds_a_fade_entrance_the_way_powerpoint_writes_it() {
    let bytes = edit_animation(&deck(), 0, &add(PresetClass::Entrance, 10, 2, None)).unwrap();
    let xml = slide_xml(&bytes);
    assert!(xml.contains(r#"<p:timing><p:tnLst><p:par><p:cTn id="1" dur="indefinite" restart="never" nodeType="tmRoot"><p:childTnLst><p:seq concurrent="1" nextAc="seek"><p:cTn id="2" dur="indefinite" nodeType="mainSeq"><p:childTnLst><p:par><p:cTn id="3" fill="hold"><p:stCondLst><p:cond delay="indefinite"/></p:stCondLst><p:childTnLst><p:par><p:cTn id="4" fill="hold"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst><p:par><p:cTn id="5" presetID="10" presetClass="entr" presetSubtype="0" fill="hold" grpId="0" nodeType="clickEffect">"#), "{xml}");
    assert!(xml.contains(r#"<p:animEffect transition="in" filter="fade"><p:cBhvr><p:cTn id="7" dur="500"/><p:tgtEl><p:spTgt spid="2"/></p:tgtEl></p:cBhvr></p:animEffect>"#));
    assert!(
        xml.contains(r#"<p:bldLst><p:bldP spid="2" grpId="0" animBg="1"/></p:bldLst></p:timing>"#)
    );
    let timing = timing(&bytes);
    assert_eq!(timing.main_sequence.len(), 1);
    assert_eq!(timing.main_sequence[0].class, EffectClass::Entrance);
}

#[test]
fn groups_with_and_after_previous_and_times_the_after_step() {
    let mut bytes = edit_animation(&deck(), 0, &add(PresetClass::Entrance, 10, 2, None)).unwrap();
    bytes = edit_animation(
        &bytes,
        0,
        &add(
            PresetClass::Entrance,
            2,
            3,
            Some(EffectStart::AfterPrevious),
        ),
    )
    .unwrap();
    bytes = edit_animation(
        &bytes,
        0,
        &AnimationEdit::SetDelay {
            indices: vec![1],
            ms: 250,
        },
    )
    .unwrap();
    let xml = slide_xml(&bytes);
    // The after step starts once the fade (500 ms) is over.
    assert!(
        xml.contains(
            r#"<p:cond delay="500"/></p:stCondLst><p:childTnLst><p:par><p:cTn id="9" presetID="2""#
        ),
        "{xml}"
    );
    let timing = timing(&bytes);
    assert_eq!(timing.main_sequence[1].start, EffectStart::AfterPrevious);
    assert_eq!(timing.main_sequence[1].delay_ms, 250);
    assert_eq!(timing.main_sequence[1].preset_subtype, 4);
}

#[test]
fn a_first_effect_with_previous_starts_with_the_slide() {
    let bytes = edit_animation(
        &deck(),
        0,
        &add(
            PresetClass::Entrance,
            10,
            2,
            Some(EffectStart::WithPrevious),
        ),
    )
    .unwrap();
    let xml = slide_xml(&bytes);
    assert!(xml.contains(r#"<p:cond delay="indefinite"/><p:cond evt="onBegin" delay="0"><p:tn val="2"/></p:cond>"#), "{xml}");
    assert_eq!(
        timing(&bytes).main_sequence[0].start,
        EffectStart::WithPrevious
    );
}

#[test]
fn changes_duration_smoothing_repeat_and_after_effects() {
    let mut bytes = edit_animation(&deck(), 0, &add(PresetClass::Entrance, 2, 2, None)).unwrap();
    for edit in [
        AnimationEdit::SetDuration {
            indices: vec![0],
            ms: 2000,
        },
        AnimationEdit::SetSmoothing {
            indices: vec![0],
            accel: 0.5,
            decel: 0.25,
        },
        AnimationEdit::SetRepeat {
            indices: vec![0],
            repeat: Repeat::Count(3.0),
        },
        AnimationEdit::SetAfter {
            indices: vec![0],
            after: AfterEffect::Dim("7F7F7F".into()),
        },
    ] {
        bytes = edit_animation(&bytes, 0, &edit).unwrap();
    }
    let effect = &timing(&bytes).main_sequence[0];
    assert_eq!(effect.duration_ms, 2000);
    assert_eq!((effect.accel, effect.decel), (0.5, 0.25));
    assert_eq!(effect.repeat, Repeat::Count(3.0));
    assert_eq!(effect.after, AfterEffect::Dim("7F7F7F".into()));
    // Changing the effect keeps the timing.
    bytes = edit_animation(
        &bytes,
        0,
        &AnimationEdit::Change {
            indices: vec![0],
            class: PresetClass::Entrance,
            preset_id: 22,
            params: EffectParams::default(),
        },
    )
    .unwrap();
    let effect = &timing(&bytes).main_sequence[0];
    assert_eq!(effect.preset_id, 22);
    assert_eq!(effect.duration_ms, 2000);
    assert_eq!(effect.repeat, Repeat::Count(3.0));
    assert!(effect
        .behaviors
        .iter()
        .any(|b| matches!(&b.kind, BehaviorKind::Effect { filter, .. } if filter == "wipe(up)")));
}

#[test]
fn moves_removes_and_triggers_effects() {
    let mut bytes = deck();
    for shape in [2, 3] {
        bytes = edit_animation(&bytes, 0, &add(PresetClass::Entrance, 10, shape, None)).unwrap();
    }
    bytes = edit_animation(&bytes, 0, &AnimationEdit::Move { index: 1, to: 0 }).unwrap();
    assert_eq!(timing(&bytes).main_sequence[0].target.shape_id, 3);
    bytes = edit_animation(
        &bytes,
        0,
        &AnimationEdit::SetTrigger {
            indices: vec![1],
            trigger: Some(Trigger::ShapeClick { shape_id: 3 }),
        },
    )
    .unwrap();
    let t = timing(&bytes);
    assert_eq!(t.main_sequence.len(), 1);
    assert_eq!(t.interactive_sequences.len(), 1);
    assert_eq!(
        t.interactive_sequences[0].trigger,
        Trigger::ShapeClick { shape_id: 3 }
    );
    assert!(slide_xml(&bytes).contains(r#"nodeType="interactiveSeq"><p:stCondLst><p:cond evt="onClick" delay="0"><p:tgtEl><p:spTgt spid="3"/>"#));
    bytes = edit_animation(&bytes, 0, &AnimationEdit::RemoveAll { shape_id: None }).unwrap();
    assert!(!slide_xml(&bytes).contains("<p:timing"));
}

#[test]
fn by_paragraph_makes_one_effect_per_paragraph_and_builds_by_paragraph() {
    let mut bytes = edit_animation(&deck(), 0, &add(PresetClass::Entrance, 10, 3, None)).unwrap();
    bytes = edit_animation(
        &bytes,
        0,
        &AnimationEdit::SetSequence {
            index: 0,
            sequence: TextSequence::ByParagraph,
            paragraphs: 2,
        },
    )
    .unwrap();
    let t = timing(&bytes);
    assert_eq!(t.main_sequence.len(), 2);
    assert_eq!(
        t.main_sequence[1].target.part,
        TargetPart::Paragraphs { start: 1, end: 1 }
    );
    assert!(slide_xml(&bytes).contains(r#"<p:bldP spid="3" grpId="0" build="p"/>"#));
}

#[test]
fn every_gallery_preset_builds_and_reads_back() {
    let mut bytes = deck();
    for preset in PRESETS {
        bytes = edit_animation(&bytes, 0, &add(preset.class, preset.id, 2, None)).unwrap();
    }
    let t = timing(&bytes);
    assert_eq!(t.main_sequence.len(), PRESETS.len());
    for (effect, preset) in t.main_sequence.iter().zip(PRESETS) {
        assert_eq!(effect.preset_id, preset.id, "{}", preset.name);
        assert_eq!(effect.class, preset.class.effect_class(), "{}", preset.name);
    }
}

#[test]
fn writes_transitions_plain_and_alternate_with_sound_and_advance() {
    let fade = TRANSITIONS.iter().find(|p| p.key == "push").unwrap();
    let mut bytes = set_transitions(
        &deck(),
        &[0],
        &TransitionEdit::Effect(Some(preset_effect(fade, Some(("dir", "l"))))),
    )
    .unwrap();
    bytes = set_transitions(&bytes, &[0], &TransitionEdit::AdvanceAfter(Some(3000))).unwrap();
    bytes = set_transitions(
        &bytes,
        &[0],
        &TransitionEdit::Sound(SoundChoice::File {
            name: "chime.wav".into(),
            bytes: b"RIFF....WAVE".to_vec(),
        }),
    )
    .unwrap();
    let xml = slide_xml(&bytes);
    assert!(xml.contains(r#"<mc:Choice xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main" Requires="p14"><p:transition spd="slow" p14:dur="1000" advTm="3000"><p:push dir="l"/><p:sndAc><p:stSnd><p:snd r:embed="rIdJadeSound1" name="chime.wav"/></p:stSnd></p:sndAc></p:transition></mc:Choice>"#), "{xml}");
    let transition = slide_animation(&bytes, 0).unwrap().transition.unwrap();
    assert_eq!(transition.effect.unwrap().name, "push");
    assert_eq!(transition.advance_after_ms, Some(3000));
    // すべてに適用 copies it, sound and all, to every slide.
    let all = transition::all_slides(&bytes).unwrap();
    let mut source = jade_pptx_reader::animation::read_deck_animations(&bytes).unwrap()[0]
        .transition
        .clone()
        .unwrap();
    source.advance_on_click = false;
    bytes = set_transitions(&bytes, &all, &TransitionEdit::Replace(source)).unwrap();
    let decks = jade_pptx_reader::animation::read_deck_animations(&bytes).unwrap();
    assert!(decks.iter().all(|slide| {
        let t = slide.transition.as_ref().unwrap();
        t.effect.as_ref().unwrap().name == "push"
            && !t.advance_on_click
            && t.sound.as_ref().unwrap().part.as_deref() == Some("ppt/media/jade-sound1.wav")
    }));
}

#[test]
fn writes_morph_as_p159_with_a_fade_fallback() {
    let morph = TRANSITIONS.iter().find(|p| p.key == "morph").unwrap();
    let mut transition = jade_pptx_reader::animation::SlideTransition {
        effect: Some(preset_effect(morph, None)),
        duration_ms: Some(2000),
        ..Default::default()
    };
    transition.speed = TransitionSpeed::Slow;
    let markup = transition_markup(&transition).unwrap();
    assert!(markup.contains(r#"Requires="p159"><p:transition spd="slow" p14:dur="2000"><p159:morph option="byObject"/></p:transition></mc:Choice><mc:Fallback><p:transition spd="slow"><p:fade/></p:transition></mc:Fallback>"#), "{markup}");
    let bytes = set_transitions(&deck(), &[1], &TransitionEdit::Replace(transition)).unwrap();
    let effect = slide_animation(&bytes, 1)
        .unwrap()
        .transition
        .unwrap()
        .effect
        .unwrap();
    assert_eq!(effect.schema, TransitionSchema::P159);
}

use jade_pptx_reader::animation::TransitionSpeed;
