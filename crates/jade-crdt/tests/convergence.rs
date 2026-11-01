use jade_crdt::{Affinity, Document, NodeSeed};

fn create() -> Document {
    Document::create(
        "work-1",
        "baseline-1",
        vec![NodeSeed {
            id: "paragraph-uuid".into(),
            text: "許可😀".into(),
        }],
    )
    .unwrap()
}
fn peer(doc: &Document) -> Document {
    Document::open("work-1", "baseline-1", &doc.snapshot().unwrap()).unwrap()
}
fn text(doc: &Document) -> String {
    doc.text("paragraph-uuid").unwrap()
}

#[test]
fn concurrent_edits_converge_in_any_delivery_order_and_duplicate_delivery() {
    let origin = create();
    let base = origin.version().unwrap();
    let mut peers = [peer(&origin), peer(&origin), peer(&origin)];
    for (i, p) in peers.iter_mut().enumerate() {
        p.replace("paragraph-uuid", 3, 3, ["甲", "乙", "丙"][i])
            .unwrap();
    }
    let updates: Vec<_> = peers
        .iter()
        .map(|p| p.updates_since(&base).unwrap())
        .collect();
    let mut expected = None;
    for order in [
        [0, 1, 2],
        [2, 1, 0],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [0, 2, 1],
    ] {
        let mut receiver = peer(&origin);
        for i in order.into_iter().chain(order) {
            receiver.import_updates(&updates[i]).unwrap();
        }
        let actual = text(&receiver);
        for s in ["許", "可", "😀", "甲", "乙", "丙"] {
            assert_eq!(actual.matches(s).count(), 1);
        }
        assert_eq!(expected.get_or_insert(actual.clone()), &actual);
    }
}

#[test]
fn full_snapshots_are_idempotent_merge_deliveries_and_preserve_local_undo() {
    let mut a = create();
    let mut b = peer(&a);
    a.replace("paragraph-uuid", 0, 0, "甲").unwrap();
    b.replace("paragraph-uuid", 0, 0, "乙").unwrap();
    let delivery = b.snapshot().unwrap();
    a.import_updates(&delivery).unwrap();
    let merged = a.version().unwrap();
    a.import_updates(&delivery).unwrap();
    assert_eq!(a.version().unwrap(), merged);
    assert!(text(&a).contains('甲') && text(&a).contains('乙'));
    a.undo().unwrap();
    assert_eq!(text(&a), "乙許可😀");
    let unrelated = create();
    assert!(a.import_updates(&unrelated.snapshot().unwrap()).is_err());
}

#[test]
fn concurrent_delete_does_not_erase_an_unseen_insert() {
    let mut a = create();
    let mut b = peer(&a);
    let version = a.version().unwrap();
    a.replace("paragraph-uuid", 0, 3, "").unwrap();
    b.replace("paragraph-uuid", 3, 3, "新").unwrap();
    let ua = a.updates_since(&version).unwrap();
    let ub = b.updates_since(&version).unwrap();
    a.import_updates(&ub).unwrap();
    b.import_updates(&ua).unwrap();
    assert_eq!(text(&a), "新可😀");
    assert_eq!(text(&a), text(&b));
}

#[test]
fn stable_anchor_survives_remote_edit_deletion_and_snapshot_in_utf8_coordinates() {
    let mut a = create();
    let anchor = a.anchor("paragraph-uuid", 6, Affinity::Before).unwrap();
    let mut b = peer(&a);
    let version = a.version().unwrap();
    b.replace("paragraph-uuid", 0, 0, "追記").unwrap();
    a.import_updates(&b.updates_since(&version).unwrap())
        .unwrap();
    assert_eq!(a.resolve_anchor(&anchor).unwrap(), 12);
    a.replace("paragraph-uuid", 12, 16, "").unwrap();
    assert_eq!(a.resolve_anchor(&anchor).unwrap(), 12);
    assert_eq!(peer(&a).resolve_anchor(&anchor).unwrap(), 12);
}

#[test]
fn invalid_utf8_ranges_never_partially_delete_text() {
    let mut a = create();
    let version = a.version().unwrap();
    for (start, end) in [(1, 3), (0, 4), (0, 99), (6, 3), (7, 7)] {
        assert!(a.replace("paragraph-uuid", start, end, "bad").is_err());
        assert!(a.anchor("paragraph-uuid", end, Affinity::Before).is_err() || end == 3);
        assert_eq!(text(&a), "許可😀");
        assert_eq!(a.version().unwrap(), version);
    }
    assert!(a.replace("missing-node", 0, 0, "bad").is_err());
}

#[test]
fn identity_and_genesis_are_checked_before_merging_or_resolving() {
    let mut a = create();
    let unrelated = create(); // same IDs/content, independently initialized: not a replica
    assert!(a
        .import_updates(
            &unrelated
                .updates_since(&unrelated.version().unwrap())
                .unwrap()
        )
        .is_err());
    assert!(a
        .resolve_anchor(
            &unrelated
                .anchor("paragraph-uuid", 0, Affinity::Before)
                .unwrap()
        )
        .is_err());
    assert!(Document::open("another-work", "baseline-1", &a.snapshot().unwrap()).is_err());
    assert!(Document::open("work-1", "another-baseline", &a.snapshot().unwrap()).is_err());
    assert_eq!(text(&a), "許可😀");
}

#[test]
fn missing_causal_dependency_requires_resync_without_partial_mutation() {
    let mut a = create();
    let mut b = peer(&a);
    a.replace("paragraph-uuid", 0, 0, "先").unwrap();
    let after_first = a.version().unwrap();
    a.replace("paragraph-uuid", 3, 3, "後").unwrap();
    let missing_parent = a.updates_since(&after_first).unwrap();
    let version = b.version().unwrap();
    assert!(b
        .import_updates(&missing_parent)
        .unwrap_err()
        .contains("resync"));
    assert_eq!(text(&b), "許可😀");
    assert_eq!(b.version().unwrap(), version);
    b.import_updates(&a.updates_since(&version).unwrap())
        .unwrap();
    assert_eq!(text(&a), text(&b));
}

#[test]
fn malformed_data_and_wrong_payload_kind_are_rejected() {
    let mut a = create();
    let version = a.version().unwrap();
    for bytes in [vec![], b"invalid".to_vec(), version.clone()] {
        assert!(a.import_updates(&bytes).is_err());
        assert_eq!(a.version().unwrap(), version);
    }
    let mut corrupt = a.updates_since(&version).unwrap();
    corrupt.truncate(corrupt.len() - 1);
    assert!(a.import_updates(&corrupt).is_err());
    assert_eq!(text(&a), "許可😀");
}

#[test]
fn empty_nodes_and_end_anchors_round_trip_and_keep_node_identity() {
    let mut a = Document::create(
        "work-1",
        "baseline-1",
        vec![
            NodeSeed {
                id: "a".into(),
                text: "".into(),
            },
            NodeSeed {
                id: "b".into(),
                text: "次".into(),
            },
        ],
    )
    .unwrap();
    let anchor = a.anchor("a", 0, Affinity::After).unwrap();
    a.replace("a", 0, 0, "😀").unwrap();
    let b = peer(&a);
    assert_eq!(b.resolve_anchor(&anchor).unwrap(), 4);
    assert_eq!(b.text("b").unwrap(), "次");
    let end = b.anchor("a", 4, Affinity::After).unwrap();
    assert_eq!(a.resolve_anchor(&end).unwrap(), 4);
}

#[test]
fn duplicate_or_empty_node_ids_cannot_create_ambiguous_documents() {
    for ids in [["same", "same"], ["", "a"]] {
        assert!(Document::create(
            "work-1",
            "baseline-1",
            ids.into_iter()
                .map(|id| NodeSeed {
                    id: id.into(),
                    text: "".into(),
                })
                .collect()
        )
        .is_err());
    }
    assert!(Document::create("", "baseline-1", vec![]).is_err());
}

#[test]
fn anchor_affinity_defines_which_side_of_new_text_the_caret_stays() {
    for (offset, expected_before, expected_after) in [(0, 0, 3), (3, 3, 6), (10, 10, 13)] {
        let mut a = create();
        let before = a
            .anchor("paragraph-uuid", offset, Affinity::Before)
            .unwrap();
        let after = a.anchor("paragraph-uuid", offset, Affinity::After).unwrap();
        assert_eq!(a.resolve_anchor(&before).unwrap(), offset);
        assert_eq!(a.resolve_anchor(&after).unwrap(), offset);
        a.replace("paragraph-uuid", offset, offset, "新").unwrap();
        assert_eq!(a.resolve_anchor(&before).unwrap(), expected_before);
        assert_eq!(a.resolve_anchor(&after).unwrap(), expected_after);
    }
}

#[test]
fn exchanging_versions_with_concurrent_unknown_peers_does_not_drop_updates() {
    let mut a = create();
    let mut b = peer(&a);
    a.replace("paragraph-uuid", 0, 0, "A").unwrap();
    b.replace("paragraph-uuid", 0, 0, "B").unwrap();
    let ua = a.updates_since(&b.version().unwrap()).unwrap();
    let ub = b.updates_since(&a.version().unwrap()).unwrap();
    a.import_updates(&ub).unwrap();
    b.import_updates(&ua).unwrap();
    assert_eq!(text(&a), text(&b));
    a.replace("paragraph-uuid", 0, 0, "次").unwrap();
    b.import_updates(&a.updates_since(&b.version().unwrap()).unwrap())
        .unwrap();
    assert_eq!(text(&a), text(&b));
}

#[test]
fn deleted_anchor_neighbours_collapse_to_the_same_gap() {
    for affinity in [Affinity::Before, Affinity::After] {
        let mut a = create();
        let anchor = a.anchor("paragraph-uuid", 3, affinity).unwrap();
        a.replace("paragraph-uuid", 0, 6, "").unwrap();
        assert_eq!(a.resolve_anchor(&anchor).unwrap(), 0);
        assert_eq!(peer(&a).resolve_anchor(&anchor).unwrap(), 0);
    }
}

#[test]
fn version_tokens_are_canonical_across_reopened_multi_peer_replicas() {
    let mut origin = create();
    for _ in 0..8 {
        let mut branch = peer(&origin);
        branch.replace("paragraph-uuid", 0, 0, "甲").unwrap();
        origin.import_updates(&branch.snapshot().unwrap()).unwrap();
    }
    let version = origin.version().unwrap();
    for _ in 0..64 {
        assert_eq!(peer(&origin).version().unwrap(), version);
    }
}
