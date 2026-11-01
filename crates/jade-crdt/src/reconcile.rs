//! Translate a successful native edit into granular operations, preserving IDs.
//! Captures are local transaction boundaries, never whole-document LWW values.
use super::{Document, Fact, NodeOp, Result, StructuredNode, TextEdit};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

fn visible(nodes: &[StructuredNode]) -> Vec<StructuredNode> {
    nodes
        .iter()
        .filter(|n| !n.deleted)
        .map(|n| StructuredNode {
            children: visible(&n.children),
            ..n.clone()
        })
        .collect()
}
fn flatten<'a>(
    nodes: &'a [StructuredNode],
    map: &mut BTreeMap<String, &'a StructuredNode>,
    order: &mut BTreeMap<Option<String>, Vec<String>>,
    parents: &mut BTreeMap<String, Option<String>>,
    parent: Option<String>,
    reject_deleted: bool,
) -> Result<()> {
    let siblings = order.entry(parent.clone()).or_default();
    siblings.extend(nodes.iter().map(|n| n.id.clone()));
    for node in nodes {
        if node.id.is_empty()
            || node.kind.is_empty()
            || (reject_deleted && node.deleted)
            || map.insert(node.id.clone(), node).is_some()
        {
            return Err("unique nonempty visible node IDs and kinds are required".into());
        }
        parents.insert(node.id.clone(), parent.clone());
        flatten(
            &node.children,
            map,
            order,
            parents,
            Some(node.id.clone()),
            reject_deleted,
        )?;
    }
    Ok(())
}
fn text_delta(id: &str, before: &str, after: &str) -> Option<TextEdit> {
    if before == after {
        return None;
    }
    let prefix = before
        .chars()
        .zip(after.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum::<usize>();
    let suffix = before[prefix..]
        .chars()
        .rev()
        .zip(after[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum::<usize>();
    Some(TextEdit {
        node: id.into(),
        start: prefix,
        end: before.len() - suffix,
        value: after[prefix..after.len() - suffix].into(),
    })
}
impl Document {
    pub fn visible_structure(&self) -> Result<Vec<StructuredNode>> {
        Ok(visible(&self.structure()?))
    }

    /// Reject stale native captures before changing text, facts or Undo history.
    /// Nodes absent from the next projection become tombstones; no IDs are reused.
    pub fn reconcile(
        &mut self,
        actor: &str,
        expected: &[StructuredNode],
        next: Vec<StructuredNode>,
        values: Vec<Value>,
    ) -> Result<Vec<Fact>> {
        let current = self.structure()?;
        if visible(&current) != expected {
            return Err("stale native CRDT capture".into());
        }
        let (ops, edits) = plan(&current, &next)?;
        self.transact_structured(actor, ops, edits, values)
    }

    /// Atomically reconcile disjoint existing subtrees. Roots keep their identity
    /// and position; insertions/removals of roots require capturing their parent.
    /// Stale or overlapping scopes fail before text, facts or Undo is changed.
    pub fn reconcile_subtrees(
        &mut self,
        actor: &str,
        scopes: &[(StructuredNode, StructuredNode)],
        values: Vec<Value>,
    ) -> Result<Vec<Fact>> {
        let mut seen = std::collections::HashSet::new();
        let mut ops = vec![];
        let mut edits = vec![];
        fn unique(
            node: &StructuredNode,
            seen: &mut std::collections::HashSet<String>,
        ) -> Result<()> {
            if !seen.insert(node.id.clone()) {
                return Err("overlapping capture scopes".into());
            }
            for child in &node.children {
                unique(child, seen)?;
            }
            Ok(())
        }
        for (expected, next) in scopes {
            unique(expected, &mut seen)?;
            if expected.id != next.id {
                return Err("capture scope root identity changed".into());
            }
            let current = self.subtree(&expected.id)?;
            if visible(std::slice::from_ref(&current)) != [expected.clone()] {
                return Err("stale native CRDT capture".into());
            }
            let (mut local_ops, mut local_edits) = plan(&[current], std::slice::from_ref(next))?;
            ops.append(&mut local_ops);
            edits.append(&mut local_edits);
        }
        self.transact_structured(actor, ops, edits, values)
    }
}

fn plan(
    current: &[StructuredNode],
    next: &[StructuredNode],
) -> Result<(Vec<NodeOp>, Vec<TextEdit>)> {
    let mut previous = BTreeMap::new();
    let mut order = BTreeMap::new();
    let mut parents = BTreeMap::new();
    flatten(
        current,
        &mut previous,
        &mut order,
        &mut parents,
        None,
        false,
    )?;
    let mut desired = BTreeMap::new();
    let mut desired_order = BTreeMap::new();
    flatten(
        next,
        &mut desired,
        &mut desired_order,
        &mut BTreeMap::new(),
        None,
        true,
    )?;
    let mut ops = vec![];
    let mut edits = vec![];
    fn visit(
        nodes: &[StructuredNode],
        parent: Option<String>,
        previous: &BTreeMap<String, &StructuredNode>,
        order: &mut BTreeMap<Option<String>, Vec<String>>,
        parents: &mut BTreeMap<String, Option<String>>,
        ops: &mut Vec<NodeOp>,
        edits: &mut Vec<TextEdit>,
    ) -> Result<()> {
        for (index, node) in nodes.iter().enumerate() {
            if let Some(old) = previous.get(&node.id) {
                let old_parent = parents.get(&node.id).ok_or("missing node parent")?.clone();
                // Unchanged siblings need neither a global parent search nor
                // a linear position search. Only actual moves update order.
                if old_parent != parent || order[&old_parent].get(index) != Some(&node.id) {
                    let old_index = order[&old_parent]
                        .iter()
                        .position(|id| id == &node.id)
                        .ok_or("missing node position")?;
                    order.get_mut(&old_parent).unwrap().remove(old_index);
                    order
                        .entry(parent.clone())
                        .or_default()
                        .insert(index, node.id.clone());
                    parents.insert(node.id.clone(), parent.clone());
                    ops.push(NodeOp::Move {
                        id: node.id.clone(),
                        parent: parent.clone(),
                        index,
                    });
                }
                if old.deleted {
                    ops.push(NodeOp::SetDeleted {
                        id: node.id.clone(),
                        deleted: false,
                    });
                }
                if old.kind != node.kind {
                    ops.push(NodeOp::SetKind {
                        id: node.id.clone(),
                        value: node.kind.clone(),
                    });
                }
                for name in old
                    .attributes
                    .keys()
                    .chain(node.attributes.keys())
                    .collect::<BTreeSet<_>>()
                {
                    if old.attributes.get(name) != node.attributes.get(name) {
                        ops.push(NodeOp::SetAttribute {
                            id: node.id.clone(),
                            name: name.clone(),
                            value: node.attributes.get(name).cloned(),
                        });
                    }
                }
                if let Some(edit) = text_delta(&node.id, &old.text, &node.text) {
                    edits.push(edit);
                }
            } else {
                parents.insert(node.id.clone(), parent.clone());
                order
                    .entry(parent.clone())
                    .or_default()
                    .insert(index, node.id.clone());
                ops.push(NodeOp::Insert {
                    parent: parent.clone(),
                    index,
                    node: StructuredNode {
                        children: vec![],
                        ..node.clone()
                    },
                });
            }
            visit(
                &node.children,
                Some(node.id.clone()),
                previous,
                order,
                parents,
                ops,
                edits,
            )?;
        }
        Ok(())
    }
    visit(
        next,
        None,
        &previous,
        &mut order,
        &mut parents,
        &mut ops,
        &mut edits,
    )?;
    for old in previous.values() {
        if !old.deleted && !desired.contains_key(&old.id) {
            ops.push(NodeOp::SetDeleted {
                id: old.id.clone(),
                deleted: true,
            });
        }
    }
    Ok((ops, edits))
}
