//! External edits must refresh all numbering data without changing saved replays.
use super::*;
use jade_docx_reader::numbering::{
    AbstractNum, LevelOverride, MultilevelListPreset, NumberingIndent, NumberingInstance,
    NumberingLevel, PictureBullet,
};
use proptest::prelude::*;

fn level(seed: u32, mask: u16) -> NumberingLevel {
    let present = |bit: u32| mask & (1u16 << bit) != 0;
    let text = |bit| present(bit).then(|| format!("{seed}:日本語:{bit}"));
    let run = || RunProperties {
        style: Some(format!("run-{seed}").into()),
        font_size: Some(f32::from_bits(seed)),
        color: Some(format!("color-{seed}").into()),
        bold: Some(seed.is_multiple_of(2)),
        preserved_rpr: jade_docx_reader::document::PreservedRunProps(vec![
            jade_docx_reader::docx_xml::XmlNode::Comment(format!("preserved-{seed}")),
        ]),
        direct_font_properties: jade_docx_reader::document::DirectRunFontProperties {
            known: true,
            font_size: Some(f32::from_bits(seed)),
            ..Default::default()
        },
        ..Default::default()
    };
    NumberingLevel {
        ilvl: seed % 12,
        start: present(0).then_some(seed),
        restart: present(1).then_some(seed.wrapping_add(1)),
        num_fmt: text(2),
        lvl_text: text(3),
        lvl_jc: text(4),
        suffix: text(5),
        indent: present(6).then_some(NumberingIndent {
            left: Some(f32::from_bits(seed)),
            hanging: Some(f32::from_bits(seed ^ 0x8000_0000)),
        }),
        paragraph_style: text(7),
        paragraph_props: present(8).then(|| ParagraphProperties {
            style: Some(format!("paragraph-{seed}").into()),
            alignment: Some(format!("align-{seed}").into()),
            keep_next: Some(seed.is_multiple_of(2)),
            mark_run_properties: Some(run()),
            ..Default::default()
        }),
        run_props: present(9).then(run),
        language: text(10),
        picture_bullet: text(11),
        is_legal: present(12),
    }
}

// JSON observes every serialized field. Debug also observes run-property
// preservation/provenance, which serialization and PartialEq intentionally omit.
// Float bits separately distinguish signed zero and NaN payloads.
#[derive(Debug, PartialEq, Clone)]
struct Observation {
    json: serde_json::Value,
    debug: String,
    float_bits: Vec<u32>,
}

fn observe(part: &NumberingPart) -> Observation {
    let mut float_bits = Vec::new();
    for abstract_num in part.abstract_nums.values() {
        for level in &abstract_num.levels {
            if let Some(indent) = &level.indent {
                float_bits.extend(indent.left.map(f32::to_bits));
                float_bits.extend(indent.hanging.map(f32::to_bits));
            }
            let mark = level
                .paragraph_props
                .as_ref()
                .and_then(|props| props.mark_run_properties.as_ref());
            for props in [level.run_props.as_ref(), mark].into_iter().flatten() {
                float_bits.extend(props.font_size.map(f32::to_bits));
                float_bits.extend(props.direct_font_properties.font_size.map(f32::to_bits));
            }
        }
    }
    for picture in part.picture_bullets.values() {
        float_bits.extend(picture.width_pt.map(f32::to_bits));
        float_bits.extend(picture.height_pt.map(f32::to_bits));
    }
    Observation {
        json: serde_json::to_value(part).unwrap(),
        debug: format!("{part:?}"),
        float_bits,
    }
}

fn edit(part: &mut NumberingPart, kind: u8, seed: u32, mask: u16) {
    let key = (seed % 3).to_string();
    match kind % 16 {
        0..=2 => {
            part.abstract_nums.insert(
                key,
                AbstractNum {
                    id: format!("abstract-{seed}"),
                    levels: (0..seed % 11)
                        .map(|n| level(seed.wrapping_add(n), mask.rotate_left(n)))
                        .collect(),
                    preset: match seed % 4 {
                        0 => None,
                        1 => Some(MultilevelListPreset::DecimalHierarchy),
                        2 => Some(MultilevelListPreset::ChineseOfficial),
                        _ => Some(MultilevelListPreset::BulletHierarchy),
                    },
                },
            );
        }
        3 => {
            part.abstract_nums.remove(&key);
        }
        4 => {
            part.numbering_instances.insert(
                key,
                NumberingInstance {
                    id: format!("instance-{seed}"),
                    abstract_num_id: (seed % 3).to_string(),
                    overrides: (0..seed % 4)
                        .map(|n| {
                            (
                                n,
                                LevelOverride {
                                    start: Some(seed),
                                    level: None,
                                },
                            )
                        })
                        .collect(),
                },
            );
        }
        5 => {
            part.numbering_instances.remove(&key);
        }
        6 => {
            part.picture_bullets.insert(
                key,
                PictureBullet {
                    relationship_id: Some(format!("relationship-{seed}")),
                    width_pt: Some(f32::from_bits(seed)),
                    height_pt: Some(f32::from_bits(seed ^ 0x8000_0000)),
                    data_uri: Some(format!("data:image/png;base64,{seed}")),
                },
            );
        }
        7 => {
            part.picture_bullets.remove(&key);
        }
        8 => *part = NumberingPart::new(),
        9 => {
            // New hash seed / insertion order, including unchanged keys.
            part.abstract_nums = part
                .abstract_nums
                .iter()
                .map(|(id, definition)| (id.clone(), definition.clone()))
                .collect();
        }
        10..=14 => {
            let instance =
                part.numbering_instances
                    .entry(key)
                    .or_insert_with(|| NumberingInstance {
                        id: "instance".into(),
                        abstract_num_id: "abstract".into(),
                        overrides: Default::default(),
                    });
            match kind % 16 {
                10 => {
                    instance.id.clear();
                    instance.id.push_str(&format!("id-{seed}"));
                }
                11 => {
                    instance.abstract_num_id.clear();
                    instance.abstract_num_id.push_str(&format!("ref-{seed}"));
                }
                12 => {
                    instance.overrides.insert(
                        seed % 12,
                        LevelOverride {
                            start: (mask & 1 != 0).then_some(seed),
                            level: None,
                        },
                    );
                }
                13 => {
                    instance.overrides.remove(&(seed % 12));
                }
                _ => instance.overrides.clear(),
            }
        }
        _ => {
            part.numbering_instances = part
                .numbering_instances
                .iter()
                .map(|(id, instance)| (id.clone(), instance.clone()))
                .collect();
        }
    }
}

fn text_field(level: &mut NumberingLevel, field: usize) -> &mut Option<String> {
    match field {
        0 => &mut level.num_fmt,
        1 => &mut level.lvl_text,
        2 => &mut level.lvl_jc,
        3 => &mut level.suffix,
        4 => &mut level.paragraph_style,
        5 => &mut level.language,
        6 => &mut level.picture_bullet,
        _ => unreachable!(),
    }
}

#[test]
fn numbering_snapshot_refreshes_each_text_field_after_in_place_edits() {
    let mut source = NumberingPart::new();
    edit(&mut source, 0, 1, u16::MAX);
    let mut cached = source.clone();
    let long = "税😀".repeat(256);
    for field in 0..7 {
        *text_field(
            &mut source.abstract_nums.get_mut("1").unwrap().levels[0],
            field,
        ) = Some(String::with_capacity(4096));
        for value in [
            Some("abc"),
            Some("abc"),
            Some("xbc"),
            Some("xby"),
            Some("日本語"),
            Some("日本税"),
            Some("a\0b"),
            Some(long.as_str()),
            Some(""),
            None,
            None,
            Some(""),
            Some("restored"),
        ] {
            let text = text_field(
                &mut source.abstract_nums.get_mut("1").unwrap().levels[0],
                field,
            );
            if let Some(value) = value {
                let text = text.get_or_insert_with(|| String::with_capacity(4096));
                let address = text.as_ptr();
                text.clear();
                text.push_str(value);
                assert_eq!(text.as_ptr(), address);
            } else {
                *text = None;
            }
            let expected = observe(&source);
            super::super::numbering_snapshot::refresh(&mut cached, &source);
            assert_eq!(observe(&cached), expected, "field {field}, value {value:?}");
        }
    }
}

#[test]
fn numbering_snapshot_refreshes_instance_fields_independently_of_map_keys() {
    let mut source = NumberingPart::new();
    edit(&mut source, 4, 1, 0);
    let mut cached = source.clone();
    for (kind, seed, mask) in [
        (10, 4, 0),
        (11, 7, 0),
        (12, 10, 1),
        (12, 10, 0),
        (12, 13, 1),
        (13, 10, 0),
        (14, 4, 0),
        (12, 16, 1),
        (15, 0, 0),
    ] {
        edit(&mut source, kind, seed, mask);
        let expected = observe(&source);
        super::super::numbering_snapshot::refresh(&mut cached, &source);
        assert_eq!(observe(&cached), expected, "kind {kind}, seed {seed}");
    }
}

#[test]
fn numbering_snapshot_refresh_preserves_every_level_field_and_float_bits() {
    let mut source = NumberingPart::new();
    edit(&mut source, 0, 1, u16::MAX);
    let mut cached = source.clone();
    for bits in [0, 0x8000_0000, 1, 0x7f80_0000, 0xff80_0000, 0x7fc1_2345] {
        for mask in [u16::MAX, 0, u16::MAX, 1 << 8, 1 << 9, 0] {
            source.abstract_nums.get_mut("1").unwrap().levels = vec![level(bits, mask)];
            let expected = observe(&source);
            super::super::numbering_snapshot::refresh(&mut cached, &source);
            assert_eq!(observe(&cached), expected);
        }
    }
}

#[test]
fn numbering_snapshot_reuses_only_exclusive_storage_and_refreshes_the_measurer() {
    let mut renderer = pageless_test_renderer();
    let mut source = NumberingPart::new();
    edit(&mut source, 0, 1, 0);
    renderer.store_numbering_part(Some(&source));
    let original = Rc::as_ptr(renderer.last_numbering_part.as_ref().unwrap());
    let mut helper = pageless_test_renderer();
    helper.last_numbering_part = renderer.last_numbering_part.clone();
    renderer.keep_next_measurer = Some(Box::new(helper));
    edit(&mut source, 0, 1, u16::MAX);
    renderer.store_numbering_part(Some(&source));
    assert_eq!(
        Rc::as_ptr(renderer.last_numbering_part.as_ref().unwrap()),
        original
    );
    assert_eq!(
        observe(renderer.last_numbering_part.as_deref().unwrap()),
        observe(&source)
    );
    assert!(Rc::ptr_eq(
        renderer.last_numbering_part.as_ref().unwrap(),
        renderer
            .keep_next_measurer
            .as_ref()
            .unwrap()
            .last_numbering_part
            .as_ref()
            .unwrap(),
    ));
    let saved = renderer.last_numbering_part.clone().unwrap();
    let expected_saved = observe(&saved);
    edit(&mut source, 0, 1, 0);
    renderer.store_numbering_part(Some(&source));
    assert!(!Rc::ptr_eq(
        &saved,
        renderer.last_numbering_part.as_ref().unwrap()
    ));
    assert_eq!(observe(&saved), expected_saved);
    assert_eq!(
        observe(renderer.last_numbering_part.as_deref().unwrap()),
        observe(&source)
    );
}

#[test]
fn numbering_snapshot_respects_weak_owners_and_unrelated_measurer_snapshots() {
    let mut renderer = pageless_test_renderer();
    let mut source = NumberingPart::new();
    edit(&mut source, 0, 1, u16::MAX);
    renderer.store_numbering_part(Some(&source));
    let weak = Rc::downgrade(renderer.last_numbering_part.as_ref().unwrap());
    let mut helper = pageless_test_renderer();
    let unrelated = Rc::new(NumberingPart::new());
    helper.last_numbering_part = Some(unrelated.clone());
    renderer.keep_next_measurer = Some(Box::new(helper));

    edit(&mut source, 0, 1, 0);
    renderer.store_numbering_part(Some(&source));
    assert!(weak.upgrade().is_none());
    assert_eq!(
        observe(renderer.last_numbering_part.as_deref().unwrap()),
        observe(&source)
    );
    assert!(Rc::ptr_eq(
        renderer
            .keep_next_measurer
            .as_ref()
            .unwrap()
            .last_numbering_part
            .as_ref()
            .unwrap(),
        &unrelated,
    ));
    renderer.store_numbering_part(None);
    assert!(renderer.last_numbering_part.is_none());
    assert!(Rc::ptr_eq(
        renderer
            .keep_next_measurer
            .as_ref()
            .unwrap()
            .last_numbering_part
            .as_ref()
            .unwrap(),
        &unrelated,
    ));
}

#[derive(Clone, Debug)]
enum Action {
    Edit(u8, u32, u16),
    EditWithoutRefresh(u8, u32, u16),
    Refresh,
    Clear,
    Save,
    Release,
    Replay,
    ShareMeasurer,
    DropMeasurer,
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        8 => (0u8..16, any::<u32>(), any::<u16>()).prop_map(|(kind, seed, mask)| Action::Edit(kind, seed, mask)),
        2 => (0u8..16, any::<u32>(), any::<u16>()).prop_map(|(kind, seed, mask)| Action::EditWithoutRefresh(kind, seed, mask)),
        3 => Just(Action::Refresh),
        1 => Just(Action::Clear),
        2 => Just(Action::Save),
        2 => Just(Action::Release),
        2 => Just(Action::Replay),
        2 => Just(Action::ShareMeasurer),
        1 => Just(Action::DropMeasurer),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]
    #[test]
    fn numbering_snapshot_refresh_histories_match_complete_input_snapshots(
        actions in proptest::collection::vec((0u8..16, any::<u32>(), any::<u16>()), 1..80),
    ) {
        let mut source = NumberingPart::new();
        let mut cached = source.clone();
        for (kind, seed, mask) in actions {
            edit(&mut source, kind, seed, mask);
            let expected = observe(&source);
            super::super::numbering_snapshot::refresh(&mut cached, &source);
            prop_assert_eq!(observe(&cached), expected);
        }
    }

    #[test]
    fn numbering_snapshot_histories_preserve_external_edits_and_saved_replays(
        actions in proptest::collection::vec(action(), 1..80),
    ) {
        let mut renderer = pageless_test_renderer();
        let mut source = NumberingPart::new();
        let mut expected = None;
        let mut saved = Vec::new();
        for action in actions {
            match action {
                Action::Edit(kind, seed, mask) => {
                    edit(&mut source, kind, seed, mask);
                    expected = Some(observe(&source));
                    renderer.store_numbering_part(Some(&source));
                }
                Action::EditWithoutRefresh(kind, seed, mask) => edit(&mut source, kind, seed, mask),
                Action::Refresh => {
                    expected = Some(observe(&source));
                    renderer.store_numbering_part(Some(&source));
                }
                Action::Clear => {
                    expected = None;
                    renderer.store_numbering_part(None);
                }
                Action::Save => {
                    if let Some(snapshot) = &renderer.last_numbering_part {
                        saved.push((snapshot.clone(), expected.clone().unwrap()));
                    }
                }
                Action::Release => { saved.pop(); }
                Action::Replay => {
                    if let Some(snapshot) = renderer.last_numbering_part.clone() {
                        renderer.store_numbering_part(Some(&snapshot));
                        prop_assert!(Rc::ptr_eq(&snapshot, renderer.last_numbering_part.as_ref().unwrap()));
                    }
                }
                Action::ShareMeasurer => {
                    let mut helper = pageless_test_renderer();
                    helper.last_numbering_part = renderer.last_numbering_part.clone();
                    renderer.keep_next_measurer = Some(Box::new(helper));
                }
                Action::DropMeasurer => renderer.keep_next_measurer = None,
            }
            prop_assert_eq!(renderer.last_numbering_part.as_deref().map(observe), expected.clone());
            for (snapshot, expected) in &saved {
                prop_assert_eq!(&observe(snapshot), expected);
            }
        }
    }
}
