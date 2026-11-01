use jade_crdt::{Document, NodeSeed, TextEdit};
use serde_json::json;

fn origin() -> Document {
    Document::create(
        "work",
        "baseline",
        vec![NodeSeed {
            id: "p".into(),
            text: "使用料😀".into(),
        }],
    )
    .unwrap()
}
fn peer(doc: &Document) -> Document {
    Document::open("work", "baseline", &doc.snapshot().unwrap()).unwrap()
}
fn edit(start: usize, end: usize, value: &str) -> TextEdit {
    TextEdit {
        node: "p".into(),
        start,
        end,
        value: value.into(),
    }
}

#[test]
fn text_and_amendment_facts_commit_or_rollback_together() {
    let mut doc = origin();
    let before = doc.version().unwrap();
    let value = json!({"kind":"amendment", "id":"amendment-1"});
    // The second edit fails after the first edit was provisionally applied.
    assert!(doc
        .transact(
            "alice",
            vec![edit(0, 9, "利用料金"), edit(1, 2, "x")],
            vec![value.clone()]
        )
        .is_err());
    assert_eq!(doc.text("p").unwrap(), "使用料😀");
    assert_eq!(doc.version().unwrap(), before);
    assert!(doc.facts().unwrap().is_empty());
    let facts = doc
        .transact("alice", vec![edit(0, 9, "利用料金")], vec![value.clone()])
        .unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].actor, "alice");
    assert_eq!(facts[0].value, value);
    let restored = peer(&doc);
    assert_eq!(restored.text("p").unwrap(), "利用料金😀");
    assert_eq!(restored.facts().unwrap(), facts);
}

#[test]
fn concurrent_metadata_keeps_both_intents_in_all_delivery_orders() {
    let genesis = origin();
    let base = genesis.version().unwrap();
    let mut a = peer(&genesis);
    let mut b = peer(&genesis);
    let fa = a
        .transact("alice", vec![], vec![json!({"date":"2027-05-01"})])
        .unwrap();
    let fb = b
        .transact("bob", vec![], vec![json!({"date":"2027-07-01"})])
        .unwrap();
    assert_ne!(fa[0].id, fb[0].id);
    assert!(fa[0].parents.is_empty() && fb[0].parents.is_empty());
    let updates = [
        a.updates_since(&base).unwrap(),
        b.updates_since(&base).unwrap(),
    ];
    for order in [[0, 1], [1, 0]] {
        let mut receiver = peer(&genesis);
        for i in order.into_iter().chain(order) {
            receiver.import_updates(&updates[i]).unwrap();
        }
        let mut expected = vec![fa[0].clone(), fb[0].clone()];
        expected.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(receiver.facts().unwrap(), expected);
        let resolved = receiver
            .transact("reviewer", vec![], vec![json!({"date":"2027-06-01"})])
            .unwrap();
        assert_eq!(
            resolved[0].parents,
            expected.iter().map(|f| f.id.clone()).collect::<Vec<_>>()
        );
        let reopened = peer(&receiver);
        assert_eq!(reopened.facts().unwrap(), receiver.facts().unwrap());
    }
}

#[test]
fn offline_fact_chains_retry_missing_dependencies_without_partial_application() {
    let mut a = origin();
    let mut b = peer(&a);
    let base = a.version().unwrap();
    let first = a
        .transact("alice", vec![], vec![json!("undetermined")])
        .unwrap();
    let middle = a.version().unwrap();
    let second = a
        .transact("alice", vec![], vec![json!("confirmed")])
        .unwrap();
    assert_eq!(second[0].parents, vec![first[0].id.clone()]);
    assert!(b
        .import_updates(&a.updates_since(&middle).unwrap())
        .is_err());
    assert_eq!(b.version().unwrap(), base);
    assert!(b.facts().unwrap().is_empty());
    b.import_updates(&a.updates_since(&base).unwrap()).unwrap();
    assert_eq!(b.facts().unwrap(), a.facts().unwrap());
}

#[test]
fn actor_validation_does_not_change_text_or_history() {
    let mut doc = origin();
    let before = doc.version().unwrap();
    assert!(doc
        .transact("", vec![edit(0, 0, "新")], vec![json!("x")])
        .is_err());
    assert_eq!(doc.version().unwrap(), before);
    assert_eq!(doc.text("p").unwrap(), "使用料😀");
}

#[test]
fn selective_undo_preserves_remote_text_and_append_only_facts() {
    let genesis = origin();
    let base = genesis.version().unwrap();
    let mut alice = peer(&genesis);
    let mut bob = peer(&genesis);
    alice
        .transact(
            "alice",
            vec![edit(0, 9, "利用料金")],
            vec![json!({"amendment":"a"})],
        )
        .unwrap();
    bob.transact(
        "bob",
        vec![edit(13, 13, "です")],
        vec![json!({"date":"2027-05-01"})],
    )
    .unwrap();
    alice
        .import_updates(&bob.updates_since(&base).unwrap())
        .unwrap();
    let facts = alice.facts().unwrap();
    assert_eq!(alice.text("p").unwrap(), "利用料金😀です");
    assert!(alice.can_undo());
    assert!(alice.undo().unwrap());
    assert_eq!(alice.text("p").unwrap(), "使用料😀です");
    assert_eq!(alice.facts().unwrap(), facts);
    assert!(!alice.can_undo());
    assert!(alice.can_redo());
    let after_undo = alice.version().unwrap();
    // Metadata-only changes do not replace the redo stack with a text step.
    alice
        .transact("reviewer", vec![], vec![json!({"resolution":"skip"})])
        .unwrap();
    assert!(alice.redo().unwrap());
    assert_eq!(alice.text("p").unwrap(), "利用料金😀です");
    bob.import_updates(&alice.updates_since(&base).unwrap())
        .unwrap();
    assert_eq!(bob.text("p").unwrap(), alice.text("p").unwrap());
    assert_eq!(bob.facts().unwrap(), alice.facts().unwrap());
    assert_ne!(alice.version().unwrap(), after_undo);
    let mut reopened = peer(&alice);
    assert!(!reopened.can_undo());
    assert!(!reopened.undo().unwrap());
    assert_eq!(reopened.facts().unwrap(), alice.facts().unwrap());
}

#[test]
fn failed_transaction_does_not_consume_undo_or_redo() {
    let mut doc = origin();
    doc.replace("p", 0, 9, "料金").unwrap();
    doc.replace("p", 0, 0, "新").unwrap();
    assert!(doc.undo().unwrap());
    let version = doc.version().unwrap();
    assert!(doc
        .transact(
            "alice",
            vec![edit(0, 0, "x"), edit(2, 3, "bad")],
            vec![json!("bad")]
        )
        .is_err());
    assert_eq!(doc.version().unwrap(), version);
    assert!(doc.can_redo());
    assert!(doc.redo().unwrap());
    assert_eq!(doc.text("p").unwrap(), "新料金😀");
    assert!(doc.undo().unwrap());
    assert!(doc.undo().unwrap());
    assert_eq!(doc.text("p").unwrap(), "使用料😀");
}

#[test]
fn grouped_native_commands_undo_as_one_without_erasing_remote_text() {
    let mut a = Document::create(
        "work",
        "base",
        vec![NodeSeed {
            id: "p".into(),
            text: "料金".into(),
        }],
    )
    .unwrap();
    let mut b = Document::open("work", "base", &a.snapshot().unwrap()).unwrap();
    let v = a.version().unwrap();
    a.begin_group().unwrap();
    a.replace("p", 0, 0, "利用").unwrap();
    a.replace("p", "利用料金".len(), "利用料金".len(), "！")
        .unwrap();
    a.end_group();
    b.replace("p", "料金".len(), "料金".len(), "です").unwrap();
    a.import_updates(&b.updates_since(&v).unwrap()).unwrap();
    a.undo().unwrap();
    assert_eq!(a.text("p").unwrap(), "料金です");
}
