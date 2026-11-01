use jade_crdt::{Document, StructuredNode};
use serde_json::json;
fn n(id: &str, kind: &str, text: &str, children: Vec<StructuredNode>) -> StructuredNode {
    StructuredNode {
        id: id.into(),
        kind: kind.into(),
        text: text.into(),
        attributes: Default::default(),
        children,
        deleted: false,
    }
}
fn original() -> Document {
    Document::create_structured(
        "w",
        "b",
        vec![n(
            "body",
            "w:body",
            "",
            vec![
                n("p", "w:p", "", vec![n("t", "w:t", "使用料😀", vec![])]),
                n("q", "w:p", "", vec![]),
            ],
        )],
    )
    .unwrap()
}
fn peer(d: &Document) -> Document {
    Document::open("w", "b", &d.snapshot().unwrap()).unwrap()
}
#[test]
fn broad_tree_reordering_and_new_siblings_preserve_ids_and_undo() {
    let paragraphs = (0..300)
        .map(|i| {
            n(
                &format!("p{i}"),
                "w:p",
                "",
                vec![n(&format!("t{i}"), "w:t", "本文", vec![])],
            )
        })
        .collect();
    let mut d =
        Document::create_structured("w", "b", vec![n("body", "w:body", "", paragraphs)]).unwrap();
    let before = d.visible_structure().unwrap();
    let mut after = before.clone();
    after[0].children.reverse();
    after[0].children.insert(
        150,
        n("new-p", "w:p", "", vec![n("new-t", "w:t", "新設", vec![])]),
    );
    after[0].children[151].children[0].text = "変更済み".into();
    d.reconcile("alice", &before, after.clone(), vec![])
        .unwrap();
    assert_eq!(d.visible_structure().unwrap(), after);
    assert_eq!(peer(&d).visible_structure().unwrap(), after);
    assert!(d.undo().unwrap());
    assert_eq!(d.visible_structure().unwrap(), before);
    assert!(d.redo().unwrap());
    assert_eq!(d.visible_structure().unwrap(), after);
}
#[test]
fn native_tree_delta_is_atomic_preserves_identity_and_roundtrips_undo() {
    let mut d = original();
    let before = d.visible_structure().unwrap();
    let after = vec![n(
        "body",
        "w:body",
        "",
        vec![n(
            "q",
            "w:p",
            "",
            vec![n(
                "ins",
                "w:ins",
                "",
                vec![n(
                    "p",
                    "w:p",
                    "",
                    vec![n("t", "w:delText", "利用料金😀", vec![])],
                )],
            )],
        )],
    )];
    d.reconcile(
        "alice",
        &before,
        after.clone(),
        vec![json!({"revision":"r"})],
    )
    .unwrap();
    assert_eq!(d.visible_structure().unwrap(), after);
    assert!(d.undo().unwrap());
    assert_eq!(d.visible_structure().unwrap(), before);
    assert_eq!(d.facts().unwrap().len(), 1);
    assert!(d.redo().unwrap());
    assert_eq!(d.visible_structure().unwrap(), after);
    assert_eq!(peer(&d).visible_structure().unwrap(), after);
}
#[test]
fn minimal_text_delta_keeps_remote_insertions_and_local_undo() {
    let g = original();
    let v = g.version().unwrap();
    let mut a = peer(&g);
    let mut b = peer(&g);
    let old = a.visible_structure().unwrap();
    let mut new = old.clone();
    new[0].children[0].children[0].text = "利用料金😀".into();
    a.reconcile("alice", &old, new, vec![]).unwrap();
    b.replace("t", 13, 13, "です").unwrap();
    let ua = a.updates_since(&v).unwrap();
    let ub = b.updates_since(&v).unwrap();
    a.import_updates(&ub).unwrap();
    b.import_updates(&ua).unwrap();
    assert_eq!(
        a.visible_structure().unwrap(),
        b.visible_structure().unwrap()
    );
    assert_eq!(a.text("t").unwrap(), "利用料金😀です");
    assert!(a.undo().unwrap());
    assert_eq!(a.text("t").unwrap(), "使用料😀です");
}
#[test]
fn removal_resurrection_reparenting_and_stale_capture_are_safe() {
    let mut d = original();
    let before = d.visible_structure().unwrap();
    let mut after = before.clone();
    after[0].children.remove(0);
    d.reconcile("alice", &before, after.clone(), vec![])
        .unwrap();
    let version = d.version().unwrap();
    assert!(d
        .reconcile("alice", &before, before.clone(), vec![json!("stale")])
        .is_err());
    assert_eq!(d.version().unwrap(), version);
    d.reconcile("alice", &after, before.clone(), vec![])
        .unwrap();
    assert_eq!(d.visible_structure().unwrap(), before);
    let mut duplicate = before.clone();
    duplicate.push(before[0].clone());
    let v = d.version().unwrap();
    assert!(d
        .reconcile("alice", &before, duplicate, vec![json!("bad")])
        .is_err());
    assert_eq!(d.version().unwrap(), v);
}
#[test]
fn identical_capture_does_not_create_undo_history_or_frontier() {
    let mut d = original();
    let before = d.visible_structure().unwrap();
    let v = d.version().unwrap();
    d.reconcile("alice", &before, before.clone(), vec![])
        .unwrap();
    assert_eq!(d.version().unwrap(), v);
    assert!(!d.can_undo());
}

#[test]
fn scoped_capture_rejects_overlap_staleness_and_partial_failure_atomically() {
    let mut d = original();
    let before = d.visible_structure().unwrap();
    let p = before[0].children[0].clone();
    let q = before[0].children[1].clone();
    let mut changed = p.clone();
    changed.children[0].text = "変更".into();
    let mut invalid = q.clone();
    invalid
        .children
        .push(n("t", "w:t", "duplicate outside scope", vec![]));
    let version = d.version().unwrap();
    let snapshot = d.snapshot().unwrap();
    assert!(d
        .reconcile_subtrees(
            "a",
            &[(p.clone(), changed.clone()), (q.clone(), invalid)],
            vec![json!({"must":"not persist"})]
        )
        .is_err());
    assert_eq!(d.version().unwrap(), version);
    let reopened = Document::open("w", "b", &d.snapshot().unwrap()).unwrap();
    let previous = Document::open("w", "b", &snapshot).unwrap();
    assert_eq!(
        reopened.visible_structure().unwrap(),
        previous.visible_structure().unwrap()
    );
    assert_eq!(reopened.facts().unwrap(), previous.facts().unwrap());
    assert!(!d.can_undo());
    assert!(d.facts().unwrap().is_empty());
    assert!(d
        .reconcile_subtrees(
            "a",
            &[
                (before[0].clone(), before[0].clone()),
                (p.clone(), changed.clone())
            ],
            vec![]
        )
        .is_err());
    d.reconcile_subtrees("a", &[(p.clone(), changed.clone())], vec![])
        .unwrap();
    let version = d.version().unwrap();
    assert!(d
        .reconcile_subtrees("a", &[(p.clone(), p)], vec![])
        .is_err());
    assert_eq!(d.version().unwrap(), version);
    assert!(d.undo().unwrap());
    assert_eq!(d.visible_structure().unwrap(), before);
    assert!(d.redo().unwrap());
    assert_eq!(d.subtree("p").unwrap(), changed);
}

#[test]
fn scoped_capture_survives_import_cache_invalidation_and_selective_history() {
    let g = original();
    let v = g.version().unwrap();
    let mut a = peer(&g);
    let mut b = peer(&g);
    let p = a.subtree("p").unwrap();
    let mut local = p.clone();
    local.children[0].text = "利用料金😀".into();
    a.reconcile_subtrees("a", &[(p, local)], vec![]).unwrap();
    let q = b.subtree("q").unwrap();
    let mut remote = q.clone();
    remote.children.push(n("remote", "w:t", "後から", vec![]));
    b.reconcile_subtrees("b", &[(q, remote.clone())], vec![])
        .unwrap();
    let ua = a.updates_since(&v).unwrap();
    let ub = b.updates_since(&v).unwrap();
    a.import_updates(&ub).unwrap();
    a.import_updates(&ub).unwrap();
    b.import_updates(&ua).unwrap();
    assert_eq!(
        a.visible_structure().unwrap(),
        b.visible_structure().unwrap()
    );
    assert_eq!(a.subtree("q").unwrap(), remote);
    assert!(a.undo().unwrap());
    assert_eq!(a.subtree("p").unwrap(), g.subtree("p").unwrap());
    assert_eq!(a.subtree("q").unwrap(), remote);
    assert!(a.redo().unwrap());
    assert_eq!(
        a.visible_structure().unwrap(),
        b.visible_structure().unwrap()
    );
    let old = a.subtree("q").unwrap();
    let mut new = old.clone();
    new.children[0].text.push('😀');
    a.reconcile_subtrees("a", &[(old, new.clone())], vec![])
        .unwrap();
    assert_eq!(a.subtree("q").unwrap(), new);
}

use proptest::prelude::*;
proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]
    #[test]
    fn scoped_reconciliation_matches_full_reference_with_unicode_structure_and_history(
        commands in proptest::collection::vec((0usize..3,0u8..7,proptest::collection::vec(prop_oneof![Just('a'),Just('漢'),Just('😀'),Just('\u{301}'),Just('\n')],0..9)),1..24)
    ) {
        let tree=vec![n("body","w:body","",(0..3).map(|i|n(&format!("p{i}"),"w:p","",vec![n(&format!("t{i}"),"#text","原文😀",vec![])])).collect())];
        let mut scoped=Document::create_structured("w","b",tree).unwrap();let mut full=peer(&scoped);
        for (step,(index,op,chars)) in commands.into_iter().enumerate() {
            let before=full.visible_structure().unwrap();let mut next=before.clone();let p=&mut next[0].children[index];
            let text:String=chars.into_iter().collect();
            match op {
                0=>p.text.push_str(&text),
                1=>p.children[0].text=text,
                2=>{p.attributes.insert("edited".into(),text);},
                3=>p.children.push(n(&format!("new-{step}"),"w:r",&text,vec![])),
                4=>{if p.children.len()>1 {p.children.remove(1);}},
                5=>p.children.reverse(),
                _=>p.children[0].kind=if step%2==0 {"w:t"} else {"w:delText"}.into(),
            }
            let old=before[0].children[index].clone();let new=next[0].children[index].clone();
            scoped.reconcile_subtrees("local",&[(old,new)],vec![]).unwrap();
            full.reconcile("local",&before,next.clone(),vec![]).unwrap();
            prop_assert_eq!(scoped.visible_structure().unwrap(),next.clone());
            prop_assert_eq!(peer(&scoped).visible_structure().unwrap(),next.clone());
            if before!=next {
                prop_assert!(scoped.undo().unwrap());prop_assert!(full.undo().unwrap());
                prop_assert_eq!(scoped.visible_structure().unwrap(),before.clone());
                prop_assert_eq!(scoped.visible_structure().unwrap(),full.visible_structure().unwrap());
                prop_assert!(scoped.redo().unwrap());prop_assert!(full.redo().unwrap());
                prop_assert_eq!(scoped.visible_structure().unwrap(),next);
            }
        }
    }
}
