use super::*;

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;

fn fade(id: u32, spid: u32, node_type: &str, delay: u32) -> String {
    format!(
        r#"<p:par><p:cTn id="{id}" presetID="10" presetClass="entr" presetSubtype="0" fill="hold" grpId="0" nodeType="{node_type}"><p:stCondLst><p:cond delay="{delay}"/></p:stCondLst><p:childTnLst><p:set><p:cBhvr><p:cTn id="{}" dur="1" fill="hold"><p:stCondLst><p:cond delay="0"/></p:stCondLst></p:cTn><p:tgtEl><p:spTgt spid="{spid}"/></p:tgtEl><p:attrNameLst><p:attrName>style.visibility</p:attrName></p:attrNameLst></p:cBhvr><p:to><p:strVal val="visible"/></p:to></p:set><p:animEffect transition="in" filter="fade"><p:cBhvr><p:cTn id="{}" dur="500"/><p:tgtEl><p:spTgt spid="{spid}"/></p:tgtEl></p:cBhvr></p:animEffect></p:childTnLst></p:cTn></p:par>"#,
        id + 1,
        id + 2
    )
}

fn slide(body: &str) -> String {
    format!(
        r#"<p:sld {NS}><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>{body}</p:sld>"#
    )
}

fn main_seq(clicks: &str) -> String {
    format!(
        r#"<p:timing><p:tnLst><p:par><p:cTn id="1" dur="indefinite" restart="never" nodeType="tmRoot"><p:childTnLst><p:seq concurrent="1" nextAc="seek"><p:cTn id="2" dur="indefinite" nodeType="mainSeq"><p:childTnLst>{clicks}</p:childTnLst></p:cTn><p:prevCondLst><p:cond evt="onPrev" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:prevCondLst><p:nextCondLst><p:cond evt="onNext" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:nextCondLst></p:seq></p:childTnLst></p:cTn></p:par></p:tnLst><p:bldLst><p:bldP spid="2" grpId="0" animBg="1"/></p:bldLst></p:timing>"#
    )
}

#[test]
fn reads_click_with_and_after_effects_in_pane_order() {
    let clicks = format!(
        r#"<p:par><p:cTn id="3" fill="hold"><p:stCondLst><p:cond delay="indefinite"/></p:stCondLst><p:childTnLst><p:par><p:cTn id="4" fill="hold"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst>{}{}</p:childTnLst></p:cTn></p:par><p:par><p:cTn id="11" fill="hold"><p:stCondLst><p:cond delay="500"/></p:stCondLst><p:childTnLst>{}</p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par>"#,
        fade(5, 2, "clickEffect", 0),
        fade(8, 3, "withEffect", 250),
        fade(12, 4, "afterEffect", 0),
    );
    let animation = read_slide_animation(&slide(&main_seq(&clicks))).unwrap();
    let effects = &animation.timing.main_sequence;
    assert_eq!(effects.len(), 3);
    assert_eq!(effects[0].start, EffectStart::OnClick);
    assert_eq!(effects[1].start, EffectStart::WithPrevious);
    assert_eq!(effects[2].start, EffectStart::AfterPrevious);
    assert_eq!(effects[1].delay_ms, 250);
    assert_eq!(effects[0].class, EffectClass::Entrance);
    assert_eq!(effects[0].preset_id, 10);
    assert_eq!(effects[0].duration_ms, 500);
    assert_eq!(effects[1].target, AnimationTarget::shape(3));
    assert_eq!(effects[0].behaviors.len(), 2);
    assert!(matches!(
        &effects[0].behaviors[1].kind,
        BehaviorKind::Effect { filter, transition, .. } if filter == "fade" && transition == "in"
    ));
    assert_eq!(animation.timing.builds.len(), 1);
    assert!(matches!(
        animation.timing.builds[0].kind,
        BuildKind::Paragraph {
            animate_background: true,
            ..
        }
    ));
}

#[test]
fn reads_a_trigger_sequence_and_keeps_media_nodes() {
    let timing = format!(
        r#"<p:timing><p:tnLst><p:par><p:cTn id="1" dur="indefinite" restart="never" nodeType="tmRoot"><p:childTnLst><p:video><p:cMediaNode vol="80000"><p:cTn id="2" fill="hold" display="0"><p:stCondLst><p:cond delay="indefinite"/></p:stCondLst></p:cTn><p:tgtEl><p:spTgt spid="5"/></p:tgtEl></p:cMediaNode></p:video><p:seq concurrent="1" nextAc="seek"><p:cTn id="3" restart="whenNotActive" fill="hold" evtFilter="cancelBubble" nodeType="interactiveSeq"><p:stCondLst><p:cond evt="onClick" delay="0"><p:tgtEl><p:spTgt spid="4"/></p:tgtEl></p:cond></p:stCondLst><p:endSync evt="end" delay="0"><p:rtn val="all"/></p:endSync><p:childTnLst><p:par><p:cTn id="4" fill="hold"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst><p:par><p:cTn id="5" fill="hold"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst>{}</p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn><p:nextCondLst><p:cond evt="onClick" delay="0"><p:tgtEl><p:spTgt spid="4"/></p:tgtEl></p:cond></p:nextCondLst></p:seq></p:childTnLst></p:cTn></p:par></p:tnLst></p:timing>"#,
        fade(6, 2, "clickEffect", 0)
    );
    let animation = read_slide_animation(&slide(&timing)).unwrap();
    assert!(animation.timing.main_sequence.is_empty());
    assert_eq!(animation.timing.interactive_sequences.len(), 1);
    assert_eq!(
        animation.timing.interactive_sequences[0].trigger,
        Trigger::ShapeClick { shape_id: 4 }
    );
    assert_eq!(animation.timing.interactive_sequences[0].effects.len(), 1);
    assert_eq!(animation.timing.other_nodes.len(), 1);
    assert_eq!(animation.timing.other_nodes[0].local_name(), "video");
}

#[test]
fn reads_a_p14_transition_from_its_choice_with_duration_and_advance() {
    let transition = r#"<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main" Requires="p14"><p:transition spd="slow" p14:dur="1600" advClick="0" advTm="3000"><p14:vortex dir="r"/><p:sndAc><p:stSnd><p:snd r:embed="rId3" name="chime.wav"/></p:stSnd></p:sndAc></p:transition></mc:Choice><mc:Fallback><p:transition spd="slow" advClick="0" advTm="3000"><p:fade/></p:transition></mc:Fallback></mc:AlternateContent>"#;
    let animation = read_slide_animation(&slide(transition)).unwrap();
    let transition = animation.transition.unwrap();
    let effect = transition.effect.unwrap();
    assert_eq!(effect.name, "vortex");
    assert_eq!(effect.schema, TransitionSchema::P14);
    assert_eq!(effect.attr("dir"), Some("r"));
    assert_eq!(transition.duration_ms, Some(1600));
    assert!(!transition.advance_on_click);
    assert_eq!(transition.advance_after_ms, Some(3000));
    assert_eq!(transition.sound.unwrap().name, "chime.wav");
}

#[test]
fn reads_a_plain_transition_and_its_default_duration() {
    let animation = read_slide_animation(&slide(
        r#"<p:transition spd="med"><p:push dir="u"/></p:transition>"#,
    ))
    .unwrap();
    let transition = animation.transition.unwrap();
    assert_eq!(transition.effect.as_ref().unwrap().name, "push");
    assert_eq!(transition.effective_duration_ms(), 750);
    assert!(transition.advance_on_click);
}

#[test]
fn reads_a_morph_transition() {
    let transition = r#"<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice xmlns:p159="http://schemas.microsoft.com/office/powerpoint/2015/09/main" Requires="p159"><p:transition spd="slow" p14:dur="2000" xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main"><p159:morph option="byWord"/></p:transition></mc:Choice><mc:Fallback><p:transition spd="slow"><p:fade/></p:transition></mc:Fallback></mc:AlternateContent>"#;
    let transition = read_slide_animation(&slide(transition))
        .unwrap()
        .transition
        .unwrap();
    let effect = transition.effect.unwrap();
    assert_eq!(
        (effect.name.as_str(), effect.schema),
        ("morph", TransitionSchema::P159)
    );
    assert_eq!(effect.attr("option"), Some("byWord"));
    assert_eq!(transition.duration_ms, Some(2000));
}

#[test]
fn reads_paragraph_targets_motion_paths_and_after_effects() {
    let effect = r#"<p:par><p:cTn id="5" presetID="42" presetClass="path" presetSubtype="0" accel="50000" decel="50000" fill="hold" nodeType="clickEffect"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst><p:animMotion origin="layout" path="M 0 0 L 0 0.25 E" pathEditMode="relative" rAng="0" ptsTypes="AA"><p:cBhvr><p:cTn id="6" dur="2000" fill="hold"/><p:tgtEl><p:spTgt spid="3"><p:txEl><p:pRg st="1" end="1"/></p:txEl></p:spTgt></p:tgtEl><p:attrNameLst><p:attrName>ppt_x</p:attrName><p:attrName>ppt_y</p:attrName></p:attrNameLst></p:cBhvr><p:rCtr x="0" y="12500"/></p:animMotion><p:set><p:cBhvr><p:cTn id="7" dur="1" fill="hold" afterEffect="1"><p:stCondLst><p:cond delay="1999"/></p:stCondLst></p:cTn><p:tgtEl><p:spTgt spid="3"><p:txEl><p:pRg st="1" end="1"/></p:txEl></p:spTgt></p:tgtEl><p:attrNameLst><p:attrName>style.visibility</p:attrName></p:attrNameLst></p:cBhvr><p:to><p:strVal val="hidden"/></p:to></p:set></p:childTnLst></p:cTn></p:par>"#;
    let node = XmlNode::parse(effect).unwrap();
    let effect = parse_effect(&node, None).unwrap();
    assert_eq!(effect.class, EffectClass::MotionPath);
    assert_eq!(
        effect.target.part,
        TargetPart::Paragraphs { start: 1, end: 1 }
    );
    assert_eq!(effect.duration_ms, 2000);
    assert_eq!((effect.accel, effect.decel), (0.5, 0.5));
    assert_eq!(effect.after, AfterEffect::Hide);
    assert!(matches!(
        &effect.behaviors[0].kind,
        BehaviorKind::Motion { path, rotation_center: Some((_, y)), .. } if path == "M 0 0 L 0 0.25 E" && *y == 12500.0
    ));
}

#[test]
fn reads_slide_and_summary_zooms() {
    let zoom = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice xmlns:pslz="http://schemas.microsoft.com/office/powerpoint/2016/slidezoom" Requires="pslz"><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="Slide Zoom 3"/><p:cNvGraphicFramePr><a:graphicFrameLocks noGrp="1"/></p:cNvGraphicFramePr><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="100" y="200"/><a:ext cx="300" cy="400"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/powerpoint/2016/slidezoom"><pslz:sldZm><pslz:sldZmObj sldId="258" cId="0"><pslz:zmPr id="{1}" returnToParent="1" transitionDur="1500"/></pslz:sldZmObj></pslz:sldZm></a:graphicData></a:graphic></p:graphicFrame></mc:Choice><mc:Fallback><p:pic/></mc:Fallback></mc:AlternateContent></p:spTree></p:cSld></p:sld>"#;
    let zooms = read_slide_animation(zoom).unwrap().zooms;
    assert_eq!(zooms.len(), 1);
    assert_eq!(zooms[0].kind, ZoomKind::Slide);
    assert_eq!(zooms[0].slide_id, Some(258));
    assert_eq!(zooms[0].rect, (100, 200, 300, 400));
    assert!(zooms[0].return_to_parent);
    assert_eq!(zooms[0].transition_ms, 1500);
}
