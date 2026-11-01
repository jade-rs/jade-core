use jade_crdt::{Document, NodeOp, StructuredNode, TextEdit};
use serde_json::json;

fn node(id: &str, kind: &str, text: &str, children: Vec<StructuredNode>) -> StructuredNode {
    StructuredNode {
        id: id.into(),
        kind: kind.into(),
        text: text.into(),
        attributes: Default::default(),
        children,
        deleted: false,
    }
}
fn origin() -> Document {
    Document::create_structured(
        "work",
        "base",
        vec![node(
            "body",
            "w:body",
            "",
            vec![
                node("p", "w:p", "", vec![node("t", "w:t", "使用料", vec![])]),
                node("table", "w:tbl", "", vec![node("cell", "w:tc", "", vec![])]),
            ],
        )],
    )
    .unwrap()
}
fn peer(doc: &Document) -> Document {
    Document::open("work", "base", &doc.snapshot().unwrap()).unwrap()
}
fn text(id: &str, start: usize, end: usize, value: &str) -> TextEdit {
    TextEdit {
        node: id.into(),
        start,
        end,
        value: value.into(),
    }
}

#[test]
fn text_structure_attributes_revisions_and_facts_form_one_undo_step() {
    let mut doc = origin();
    let initial = doc.structure().unwrap();
    doc.transact_structured(
        "alice",
        vec![
            NodeOp::Insert {
                parent: Some("p".into()),
                index: 1,
                node: node("ins", "w:ins", "", vec![node("new", "w:t", "新", vec![])]),
            },
            NodeOp::SetAttribute {
                id: "ins".into(),
                name: "w:author".into(),
                value: Some("revision:a".into()),
            },
            NodeOp::Move {
                id: "p".into(),
                parent: Some("cell".into()),
                index: 0,
            },
        ],
        vec![text("t", 0, 9, "利用料金")],
        vec![json!({"amendment":"a"})],
    )
    .unwrap();
    let changed = doc.structure().unwrap();
    assert_eq!(changed[0].children[0].id, "table");
    assert_eq!(changed[0].children[0].children[0].children[0].id, "p");
    assert_eq!(doc.text("new").unwrap(), "新");
    assert!(doc.undo().unwrap());
    assert_eq!(doc.structure().unwrap(), initial);
    assert_eq!(doc.facts().unwrap().len(), 1);
    assert!(doc.redo().unwrap());
    assert_eq!(doc.structure().unwrap(), changed);
    assert_eq!(peer(&doc).structure().unwrap(), changed);
}

#[test]
fn concurrent_text_move_formatting_and_insertion_converge_in_both_orders() {
    let genesis = origin();
    let base = genesis.version().unwrap();
    let mut a = peer(&genesis);
    let mut b = peer(&genesis);
    a.transact_structured(
        "alice",
        vec![
            NodeOp::Move {
                id: "p".into(),
                parent: Some("cell".into()),
                index: 0,
            },
            NodeOp::SetAttribute {
                id: "p".into(),
                name: "bold".into(),
                value: Some("true".into()),
            },
        ],
        vec![text("t", 0, 9, "利用料金")],
        vec![],
    )
    .unwrap();
    b.transact_structured(
        "bob",
        vec![
            NodeOp::SetAttribute {
                id: "p".into(),
                name: "color".into(),
                value: Some("red".into()),
            },
            NodeOp::Insert {
                parent: Some("body".into()),
                index: 1,
                node: node("p2", "w:p", "", vec![node("t2", "w:t", "追加条文", vec![])]),
            },
        ],
        vec![text("t", 9, 9, "です")],
        vec![],
    )
    .unwrap();
    let updates = [
        a.updates_since(&base).unwrap(),
        b.updates_since(&base).unwrap(),
    ];
    let mut projections = vec![];
    for order in [[0, 1], [1, 0]] {
        let mut receiver = peer(&genesis);
        for i in order.into_iter().chain(order) {
            receiver.import_updates(&updates[i]).unwrap();
        }
        assert_eq!(receiver.text("t").unwrap(), "利用料金です");
        projections.push(receiver.structure().unwrap());
    }
    assert_eq!(projections[0], projections[1]);
    a.import_updates(&updates[1]).unwrap();
    assert!(a.undo().unwrap());
    assert_eq!(a.text("t").unwrap(), "使用料です");
    let p = &a.structure().unwrap()[0].children[0];
    assert_eq!(p.id, "p");
    assert_eq!(p.attributes.get("color").map(String::as_str), Some("red"));
    assert!(!p.attributes.contains_key("bold"));
}

#[test]
fn tombstone_undo_restores_identity_with_concurrent_edits() {
    let genesis = origin();
    let base = genesis.version().unwrap();
    let mut a = peer(&genesis);
    let mut b = peer(&genesis);
    a.transact_structured(
        "alice",
        vec![NodeOp::SetDeleted {
            id: "p".into(),
            deleted: true,
        }],
        vec![],
        vec![],
    )
    .unwrap();
    b.replace("t", 0, 9, "料金").unwrap();
    a.import_updates(&b.updates_since(&base).unwrap()).unwrap();
    assert!(a.structure().unwrap()[0].children[0].deleted);
    assert!(a.undo().unwrap());
    let p = &a.structure().unwrap()[0].children[0];
    assert!(!p.deleted);
    assert_eq!(p.id, "p");
    assert_eq!(p.children[0].text, "料金");
}

#[test]
fn invalid_structure_rolls_back_text_facts_history_and_frontier() {
    let mut doc = origin();
    let before = doc.structure().unwrap();
    let version = doc.version().unwrap();
    for op in [
        NodeOp::Move {
            id: "body".into(),
            parent: Some("p".into()),
            index: 0,
        },
        NodeOp::Insert {
            parent: Some("body".into()),
            index: 0,
            node: node("p", "w:p", "duplicate", vec![]),
        },
        NodeOp::Move {
            id: "p".into(),
            parent: Some("absent".into()),
            index: 0,
        },
    ] {
        assert!(doc
            .transact_structured(
                "alice",
                vec![
                    NodeOp::SetAttribute {
                        id: "p".into(),
                        name: "bold".into(),
                        value: Some("true".into())
                    },
                    op
                ],
                vec![text("t", 0, 9, "bad")],
                vec![json!("bad")]
            )
            .is_err());
        assert_eq!(doc.structure().unwrap(), before);
        assert_eq!(doc.version().unwrap(), version);
        assert!(doc.facts().unwrap().is_empty());
        assert!(!doc.can_undo());
    }
}
