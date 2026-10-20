//! A selection of several objects moved or deleted together, each as one
//! step of Undo, as PowerPoint moves and deletes a selection.

use jade_pptx_writer::{delete_shapes, editable_shapes, set_shape_transforms, EditSession};

const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");

fn movable(bytes: &[u8]) -> Vec<(u32, jade_pptx_reader::Transform)> {
    editable_shapes(bytes, 1)
        .unwrap()
        .into_iter()
        .filter(|shape| shape.can_move && shape.can_delete)
        .filter_map(|shape| Some((shape.id, shape.transform?)))
        .take(2)
        .collect()
}

#[test]
fn several_objects_move_together_as_one_step() {
    let shapes = movable(DECK);
    assert_eq!(shapes.len(), 2);
    let moved: Vec<_> = shapes
        .iter()
        .map(|&(id, mut transform)| {
            transform.offset_emu.0 += 95250;
            transform.offset_emu.1 += 190500;
            (id, transform)
        })
        .collect();
    let mut session = EditSession::open(DECK).unwrap();
    assert!(session.set_shape_transforms(1, &moved).unwrap());
    let now = editable_shapes(session.bytes(), 1).unwrap();
    for (id, transform) in &moved {
        let shape = now.iter().find(|shape| shape.id == *id).unwrap();
        assert_eq!(shape.transform, Some(*transform));
    }
    assert!(session.undo());
    assert_eq!(session.bytes(), DECK);
    assert_eq!(set_shape_transforms(DECK, 1, &[]).unwrap(), DECK.to_vec());
}

#[test]
fn several_objects_are_deleted_as_one_step() {
    let ids: Vec<u32> = movable(DECK).iter().map(|(id, _)| *id).collect();
    let before = editable_shapes(DECK, 1).unwrap().len();
    let mut session = EditSession::open(DECK).unwrap();
    assert!(session.delete_shapes(1, &ids).unwrap());
    assert_eq!(
        editable_shapes(session.bytes(), 1).unwrap().len(),
        before - 2
    );
    assert!(session.undo());
    assert_eq!(session.bytes(), DECK);
    // One that is not there fails it all, and nothing changes.
    assert!(delete_shapes(DECK, 1, &[ids[0], u32::MAX - 3]).is_err());
}
