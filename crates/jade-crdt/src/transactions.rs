//! Immutable causal facts share the text document's history and sync envelope.
//! The host owns payload schemas and the meaning of dependencies/effective dates.
use super::{error, structure, Document, NodeOp, Result};
use loro::{LoroDoc, ToJson};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const FACTS: &str = "jade:causal-facts:v1";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TextEdit {
    pub node: String,
    pub start: usize,
    pub end: usize,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Fact {
    pub id: String,
    pub actor: String,
    pub parents: Vec<String>,
    pub value: Value,
}

fn facts(doc: &LoroDoc) -> Result<Vec<Fact>> {
    let values = doc.get_list(FACTS).get_deep_value().to_json_value();
    let mut facts = Vec::new();
    for value in values.as_array().ok_or("invalid causal facts container")? {
        let fact: Fact =
            serde_json::from_str(value.as_str().ok_or("invalid causal fact")?).map_err(error)?;
        if fact.id.is_empty() || fact.actor.trim().is_empty() {
            return Err("fact ID and actor are required".into());
        }
        facts.push(fact);
    }
    facts.sort_by(|a, b| a.id.cmp(&b.id));
    let by_id: BTreeMap<_, _> = facts
        .iter()
        .enumerate()
        .map(|(index, fact)| (fact.id.as_str(), index))
        .collect();
    if by_id.len() != facts.len() {
        return Err("duplicate fact ID".into());
    }
    // Native captures append a causal chain, often with one baseline per
    // paragraph. Rescanning all remaining facts for every generation makes
    // every keypress quadratic in document history. Visit each edge once.
    let mut pending: Vec<_> = facts.iter().map(|fact| fact.parents.len()).collect();
    let mut children = vec![Vec::new(); facts.len()];
    for (index, fact) in facts.iter().enumerate() {
        for parent in &fact.parents {
            let parent = *by_id
                .get(parent.as_str())
                .ok_or("missing or cyclic fact dependency")?;
            children[parent].push(index);
        }
    }
    let mut ready: Vec<_> = pending
        .iter()
        .enumerate()
        .filter_map(|(index, count)| (*count == 0).then_some(index))
        .collect();
    let mut visited = 0;
    while let Some(index) = ready.pop() {
        visited += 1;
        for &child in &children[index] {
            pending[child] -= 1;
            if pending[child] == 0 {
                ready.push(child);
            }
        }
    }
    if visited != facts.len() {
        return Err("missing or cyclic fact dependency".into());
    }
    Ok(facts)
}

pub(super) fn validate_extension(previous: &LoroDoc, candidate: &LoroDoc) -> Result<()> {
    let next: BTreeMap<_, _> = facts(candidate)?
        .into_iter()
        .map(|f| (f.id.clone(), f))
        .collect();
    for old in facts(previous)? {
        if next.get(&old.id) != Some(&old) {
            return Err("causal facts are immutable".into());
        }
    }
    Ok(())
}

impl Document {
    /// Sorted by opaque ID for deterministic interchange, never by legal date.
    pub fn facts(&self) -> Result<Vec<Fact>> {
        facts(&self.doc)
    }

    /// Ranges are evaluated sequentially against provisional text. Publish the
    /// candidate only after every edit and fact succeeds. Failed transactions
    /// cannot leave text, causal facts, counters, or sync frontiers half changed.
    pub fn transact(
        &mut self,
        actor: &str,
        edits: Vec<TextEdit>,
        values: Vec<Value>,
    ) -> Result<Vec<Fact>> {
        self.transact_structured(actor, vec![], edits, values)
    }

    pub fn transact_structured(
        &mut self,
        actor: &str,
        ops: Vec<NodeOp>,
        edits: Vec<TextEdit>,
        values: Vec<Value>,
    ) -> Result<Vec<Fact>> {
        if actor.trim().is_empty() {
            return Err("actor is required".into());
        }
        if ops.is_empty() && edits.is_empty() && values.is_empty() {
            return Ok(vec![]);
        }
        let previous = if values.is_empty() {
            vec![]
        } else {
            self.facts()?
        };
        let mut heads: BTreeSet<_> = previous.iter().map(|f| f.id.clone()).collect();
        for fact in &previous {
            for parent in &fact.parents {
                heads.remove(parent);
            }
        }
        let candidate = self.doc.fork();
        candidate.set_peer_id(self.doc.peer_id()).map_err(error)?;
        let peer = candidate.peer_id();
        let counter = *candidate.oplog_vv().get(&peer).unwrap_or(&0);
        let mut ids = self.structure_ids()?;
        if !ops.is_empty() {
            structure::apply(&candidate, &ops, std::rc::Rc::make_mut(&mut ids))?;
        }
        for edit in &edits {
            // Validate against the fixed manifest without creating a root for
            // an unknown node. All mutation is confined to the candidate.
            if !self.identity.nodes.contains(&edit.node) && !ids.contains_key(&edit.node) {
                return Err("unknown node ID".into());
            }
            let text = candidate.get_text(format!("node:{}", edit.node));
            if edit.start > edit.end || text.to_string().get(edit.start..edit.end).is_none() {
                return Err("invalid UTF-8 range".into());
            }
            text.delete_utf8(edit.start, edit.end - edit.start)
                .map_err(error)?;
            text.insert_utf8(edit.start, &edit.value).map_err(error)?;
        }
        let list = candidate.get_list(FACTS);
        let mut recorded = Vec::new();
        for (index, value) in values.into_iter().enumerate() {
            let fact = Fact {
                id: format!("{peer}:{counter}:{index}"),
                actor: actor.into(),
                parents: heads.into_iter().collect(),
                value,
            };
            list.push(serde_json::to_string(&fact).map_err(error)?)
                .map_err(error)?;
            heads = BTreeSet::from([fact.id.clone()]);
            recorded.push(fact);
        }
        candidate.commit();
        // Operations validate IDs, metadata and tree moves on the candidate.
        // Re-reading every untouched subtree adds no validation to local edits.
        // Validate everything before publishing. Replaying this already checked
        // synchronous sequence on the original document preserves its local
        // undo subscription (importing a fork would be a remote event).
        let serialized: Vec<_> = recorded
            .iter()
            .map(serde_json::to_string)
            .collect::<std::result::Result<_, _>>()
            .map_err(error)?;
        let mut published_ids = self.structure_ids()?;
        if !ops.is_empty() {
            structure::apply(&self.doc, &ops, std::rc::Rc::make_mut(&mut published_ids))?;
        }
        for edit in &edits {
            let text = self.doc.get_text(format!("node:{}", edit.node));
            text.delete_utf8(edit.start, edit.end - edit.start)
                .map_err(error)?;
            text.insert_utf8(edit.start, &edit.value).map_err(error)?;
        }
        self.doc.commit();
        *self.structure_index.borrow_mut() = Some(published_ids);
        self.undo.record_new_checkpoint().map_err(error)?;
        // Facts are audit history, including undo/redo intent recorded by the
        // host; undoing text must never erase an effective-date/resolution fact.
        if !serialized.is_empty() {
            let list = self.doc.get_list(FACTS);
            for value in serialized {
                list.push(value).map_err(error)?;
            }
            self.doc.set_next_commit_origin("jade:fact:append");
            self.doc.commit();
        }
        Ok(recorded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_dependencies(entries: &[(&str, &[&str])]) -> LoroDoc {
        let doc = LoroDoc::new();
        let list = doc.get_list(FACTS);
        for (id, parents) in entries {
            let fact = Fact {
                id: (*id).into(),
                actor: "test".into(),
                parents: parents.iter().map(|p| (*p).into()).collect(),
                value: Value::Null,
            };
            list.push(serde_json::to_string(&fact).unwrap()).unwrap();
        }
        doc.commit();
        doc
    }

    #[test]
    fn validates_long_chain_and_concurrent_join_independent_of_id_order() {
        let ids: Vec<_> = (0..2_000).rev().map(|i| format!("{i:04}")).collect();
        let mut entries: Vec<(&str, &[&str])> = vec![(&ids[0], &[])];
        let parents: Vec<_> = ids.iter().map(|id| [id.as_str()]).collect();
        for index in 1..ids.len() {
            entries.push((&ids[index], &parents[index - 1]));
        }
        let doc = with_dependencies(&entries);
        assert_eq!(facts(&doc).unwrap().len(), ids.len());

        // A diamond and repeated parent references are valid dependencies.
        let joined = with_dependencies(&[
            ("a", &[]),
            ("b", &["a"]),
            ("c", &["a"]),
            ("d", &["b", "c", "c"]),
        ]);
        assert_eq!(facts(&joined).unwrap().len(), 4);
    }

    #[test]
    fn rejects_missing_cyclic_dependencies_and_duplicate_ids() {
        for entries in [
            vec![("a", vec!["missing"])],
            vec![("a", vec!["a"])],
            vec![("root", vec![]), ("a", vec!["b"]), ("b", vec!["a"])],
        ] {
            let borrowed: Vec<_> = entries
                .iter()
                .map(|(id, parents)| (*id, parents.as_slice()))
                .collect();
            assert_eq!(
                facts(&with_dependencies(&borrowed)).unwrap_err(),
                "missing or cyclic fact dependency"
            );
        }
        assert_eq!(
            facts(&with_dependencies(&[("a", &[]), ("a", &[])])).unwrap_err(),
            "duplicate fact ID"
        );
    }
}
