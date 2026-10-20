//! Objects copied and pasted as PowerPoint pastes them: onto another slide
//! where they were, their relationships with them — a picture's media
//! shared, a chart's and a diagram's parts copied for the copy alone — and
//! onto their own slide offset, as Duplicate puts them. What Duplicate
//! refuses — a video, whose timing the slide holds — is refused here too.

use jade_opc::OpcPackage;
use jade_pptx_writer::{copy_shapes, editable_shapes, EditSession, ShapeTransformInfo};

fn poi(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../public/fixtures/poi/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// `bytes` with an empty slide after the first.
fn with_blank_slide(bytes: &[u8]) -> Vec<u8> {
    let mut session = EditSession::open(bytes).unwrap();
    assert!(session.insert_slide(1).unwrap());
    session.bytes().to_vec()
}

fn copyable(bytes: &[u8], slide: usize) -> Vec<ShapeTransformInfo> {
    editable_shapes(bytes, slide)
        .unwrap()
        .into_iter()
        .filter(|shape| shape.can_duplicate)
        .collect()
}

fn parts(bytes: &[u8], kind: &str) -> usize {
    OpcPackage::open(bytes)
        .unwrap()
        .part_names()
        .iter()
        .filter(|name| name.contains(kind) && !name.ends_with(".rels"))
        .count()
}

fn slide_part(bytes: &[u8], slide: usize) -> String {
    let part = jade_pptx_reader::read_pptx(bytes).unwrap().slides[slide]
        .part_name
        .clone();
    OpcPackage::open(bytes)
        .unwrap()
        .required_part_str(&part)
        .unwrap()
}

#[test]
fn objects_pasted_on_another_slide_keep_their_place_and_bring_their_parts() {
    for (name, copied_parts, shared_parts) in [
        ("bar-chart.pptx", "/charts/chart", ""),
        ("smartart-simple.pptx", "/diagrams/data", ""),
        ("picture-transparency.pptx", "", "/media/"),
    ] {
        let bytes = with_blank_slide(&poi(name));
        let originals = copyable(&bytes, 0);
        assert!(!originals.is_empty(), "{name}");
        let ids: Vec<u32> = originals.iter().map(|shape| shape.id).collect();
        let before = slide_part(&bytes, 0);

        let (pasted, copies) = copy_shapes(&bytes, 0, &ids, 1).unwrap();
        assert_eq!(copies.len(), ids.len(), "{name}");
        let on_target = editable_shapes(&pasted, 1).unwrap();
        for (original, copy) in originals.iter().zip(&copies) {
            let copy = on_target.iter().find(|shape| shape.id == *copy).unwrap();
            assert_eq!(copy.kind, original.kind, "{name}");
            assert_eq!(copy.transform, original.transform, "{name}");
            assert_eq!(copy.name, original.name, "{name}");
        }
        // The slide copied from is as it was.
        assert_eq!(slide_part(&pasted, 0), before, "{name}");
        if !copied_parts.is_empty() {
            assert!(
                parts(&pasted, copied_parts) > parts(&bytes, copied_parts),
                "{name}: the copy has its own {copied_parts}"
            );
        }
        if !shared_parts.is_empty() {
            assert_eq!(
                parts(&pasted, shared_parts),
                parts(&bytes, shared_parts),
                "{name}: the media is shared"
            );
        }
        // And it opens as a deck.
        jade_pptx_reader::read_deck(&pasted).unwrap();
    }
}

#[test]
fn a_paste_onto_a_slide_takes_ids_and_names_unused_there() {
    let bytes = poi("shapes.pptx");
    let mut session = EditSession::open(&bytes).unwrap();
    assert!(session.duplicate_slide(0).unwrap());
    let bytes = session.bytes().to_vec();
    // Slide 2 is a copy of slide 1: every id and name is taken there.
    let originals = copyable(&bytes, 0);
    let ids: Vec<u32> = originals.iter().map(|shape| shape.id).take(2).collect();
    let (pasted, copies) = copy_shapes(&bytes, 0, &ids, 1).unwrap();
    let on_target = editable_shapes(&pasted, 1).unwrap();
    let mut seen = std::collections::HashSet::new();
    for shape in &on_target {
        assert!(seen.insert(shape.id), "id {} twice", shape.id);
    }
    for copy in &copies {
        assert!(!ids.contains(copy));
        let name = &on_target.iter().find(|s| s.id == *copy).unwrap().name;
        assert!(name.contains("(copy"), "{name}");
    }
}

#[test]
fn a_paste_onto_its_own_slide_is_offset_as_duplicate_does() {
    let bytes = poi("shapes.pptx");
    let original = copyable(&bytes, 0)
        .into_iter()
        .find(|shape| shape.can_move && shape.transform.is_some())
        .unwrap();
    let (pasted, copies) = copy_shapes(&bytes, 0, &[original.id], 0).unwrap();
    let copy = editable_shapes(&pasted, 0)
        .unwrap()
        .into_iter()
        .find(|shape| shape.id == copies[0])
        .unwrap();
    let (from, to) = (
        original.transform.unwrap().offset_emu,
        copy.transform.unwrap().offset_emu,
    );
    assert_eq!((to.0 - from.0, to.1 - from.1), (127000, 127000));
}

#[test]
fn a_paste_is_one_step_of_undo() {
    let bytes = with_blank_slide(&poi("bar-chart.pptx"));
    let ids: Vec<u32> = copyable(&bytes, 0).iter().map(|shape| shape.id).collect();
    let mut session = EditSession::open(&bytes).unwrap();
    let copies = session.copy_shapes(0, &ids, 1).unwrap();
    assert_eq!(copies.len(), ids.len());
    assert!(session.undo());
    assert_eq!(session.bytes(), bytes.as_slice());
    assert!(session.copy_shapes(0, &[u32::MAX - 7], 1).is_err());
}

#[test]
fn what_duplicate_refuses_is_not_pasted() {
    let bytes = with_blank_slide(&poi("EmbeddedVideo.pptx"));
    let video = editable_shapes(&bytes, 0).unwrap()[0].clone();
    assert!(!video.can_duplicate);
    assert!(copy_shapes(&bytes, 0, &[video.id], 1).is_err());
}

#[test]
fn a_placeholder_placed_by_its_layout_keeps_its_place_on_another_slide() {
    // The sample deck's title slide takes its title's place from its layout;
    // the second slide is on another layout.
    const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");
    let placeholders: Vec<ShapeTransformInfo> = copyable(DECK, 0);
    let ids: Vec<u32> = placeholders.iter().map(|shape| shape.id).collect();
    let (pasted, copies) = copy_shapes(DECK, 0, &ids, 1).unwrap();
    let on_target = editable_shapes(&pasted, 1).unwrap();
    for (original, copy) in placeholders.iter().zip(&copies) {
        let copy = on_target.iter().find(|shape| shape.id == *copy).unwrap();
        assert_eq!(copy.transform, original.transform, "{}", original.name);
    }
}
