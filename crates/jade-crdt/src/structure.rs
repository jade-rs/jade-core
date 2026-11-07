//! Ordered XML-like nodes. Text and metadata use permanent application IDs;
//! deleting an existing node changes visibility, never its container identity.
use super::{error, Document, Result};
use loro::{LoroDoc, LoroTree, ToJson, TreeID, TreeParentId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

const TREE: &str = "jade:tree:v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredNode {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub attributes: BTreeMap<String, String>,
    pub children: Vec<StructuredNode>,
    pub deleted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum NodeOp {
    Insert {
        parent: Option<String>,
        index: usize,
        node: StructuredNode,
    },
    Move {
        id: String,
        parent: Option<String>,
        index: usize,
    },
    SetAttribute {
        id: String,
        name: String,
        value: Option<String>,
    },
    SetDeleted {
        id: String,
        deleted: bool,
    },
    SetKind {
        id: String,
        value: String,
    },
}

fn tree(doc: &LoroDoc) -> LoroTree {
    let tree = doc.get_tree(TREE);
    tree.enable_fractional_index(0);
    tree
}

fn node_id(tree: &LoroTree, id: TreeID) -> Result<String> {
    tree.get_meta(id)
        .map_err(error)?
        .get_deep_value()
        .to_json_value()
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "invalid tree node identity".into())
}

// Resolve permanent IDs once per transaction, including logical tombstones.
// Looking up each operation by scanning every tree node made typing quadratic.
pub(super) fn index(tree: &LoroTree) -> Result<HashMap<String, TreeID>> {
    let mut ids = HashMap::new();
    for node in tree.get_nodes(false) {
        if ids.insert(node_id(tree, node.id)?, node.id).is_some() {
            return Err("duplicate structured node ID".into());
        }
    }
    Ok(ids)
}
fn find(ids: &HashMap<String, TreeID>, id: &str) -> Result<TreeID> {
    ids.get(id)
        .copied()
        .ok_or_else(|| "unknown structured node ID".into())
}

fn parent(ids: &HashMap<String, TreeID>, id: &Option<String>) -> Result<TreeParentId> {
    Ok(match id {
        Some(id) => TreeParentId::Node(find(ids, id)?),
        None => TreeParentId::Root,
    })
}

fn insert(
    doc: &LoroDoc,
    tree: &LoroTree,
    ids: &mut HashMap<String, TreeID>,
    parent: TreeParentId,
    index: usize,
    node: &StructuredNode,
) -> Result<()> {
    if node.id.is_empty() || node.kind.is_empty() || ids.contains_key(&node.id) {
        return Err("unique nonempty node ID and kind are required".into());
    }
    let id = tree.create_at(parent, index).map_err(error)?;
    ids.insert(node.id.clone(), id);
    tree.get_meta(id)
        .map_err(error)?
        .insert("id", node.id.as_str())
        .map_err(error)?;
    let meta = doc.get_map(format!("node-meta:{}", node.id));
    meta.insert("kind", node.kind.as_str()).map_err(error)?;
    meta.insert("deleted", node.deleted).map_err(error)?;
    for (key, value) in &node.attributes {
        if key.is_empty() {
            return Err("attribute name is required".into());
        }
        meta.insert(&format!("attr:{key}"), value.as_str())
            .map_err(error)?;
    }
    doc.get_text(format!("node:{}", node.id))
        .insert_utf8(0, &node.text)
        .map_err(error)?;
    for (index, child) in node.children.iter().enumerate() {
        insert(doc, tree, ids, TreeParentId::Node(id), index, child)?;
    }
    Ok(())
}

pub(super) fn apply(
    doc: &LoroDoc,
    ops: &[NodeOp],
    ids: &mut HashMap<String, TreeID>,
) -> Result<()> {
    if ops.is_empty() {
        return Ok(());
    }
    let tree = tree(doc);
    for op in ops {
        match op {
            NodeOp::Insert {
                parent: p,
                index,
                node,
            } => {
                let p = parent(ids, p)?;
                insert(doc, &tree, ids, p, *index, node)?;
            }
            NodeOp::Move {
                id,
                parent: p,
                index,
            } => {
                tree.mov_to(find(ids, id)?, parent(ids, p)?, *index)
                    .map_err(error)?;
            }
            NodeOp::SetAttribute { id, name, value } => {
                find(ids, id)?;
                if name.is_empty() {
                    return Err("attribute name is required".into());
                }
                let meta = doc.get_map(format!("node-meta:{id}"));
                let key = format!("attr:{name}");
                match value {
                    Some(value) => meta.insert(&key, value.as_str()).map_err(error)?,
                    None => meta.delete(&key).map_err(error)?,
                }
            }
            NodeOp::SetDeleted { id, deleted } => {
                find(ids, id)?;
                doc.get_map(format!("node-meta:{id}"))
                    .insert("deleted", *deleted)
                    .map_err(error)?;
            }
            NodeOp::SetKind { id, value } => {
                find(ids, id)?;
                if value.is_empty() {
                    return Err("node kind is required".into());
                }
                doc.get_map(format!("node-meta:{id}"))
                    .insert("kind", value.as_str())
                    .map_err(error)?;
            }
        }
    }
    Ok(())
}

fn visit(
    doc: &LoroDoc,
    tree: &LoroTree,
    key: TreeID,
    seen: &mut HashSet<String>,
) -> Result<StructuredNode> {
    let id = node_id(tree, key)?;
    if !seen.insert(id.clone()) {
        return Err("duplicate structured node ID".into());
    }
    let value = doc
        .get_map(format!("node-meta:{id}"))
        .get_deep_value()
        .to_json_value();
    let meta = value.as_object().ok_or("invalid node metadata")?;
    let kind = meta
        .get("kind")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or("invalid node kind")?
        .to_owned();
    let deleted = meta
        .get("deleted")
        .and_then(|v| v.as_bool())
        .ok_or("invalid node tombstone")?;
    let mut attributes = BTreeMap::new();
    for (key, value) in meta {
        if let Some(key) = key.strip_prefix("attr:") {
            if key.is_empty() {
                return Err("invalid attribute name".into());
            }
            attributes.insert(
                key.to_owned(),
                value.as_str().ok_or("invalid node attribute")?.to_owned(),
            );
        } else if key != "kind" && key != "deleted" {
            return Err("unknown node metadata".into());
        }
    }
    let children = tree
        .children(key)
        .unwrap_or_default()
        .into_iter()
        .map(|child| visit(doc, tree, child, seen))
        .collect::<Result<_>>()?;
    Ok(StructuredNode {
        text: doc.get_text(format!("node:{id}")).to_string(),
        id,
        kind,
        attributes,
        children,
        deleted,
    })
}
pub(super) fn read(doc: &LoroDoc) -> Result<Vec<StructuredNode>> {
    let tree = tree(doc);
    let mut seen = HashSet::new();
    tree.roots()
        .into_iter()
        .map(|id| visit(doc, &tree, id, &mut seen))
        .collect()
}

impl Document {
    pub(super) fn structure_ids(&self) -> Result<std::rc::Rc<HashMap<String, TreeID>>> {
        let mut cached = self.structure_index.borrow_mut();
        if cached.is_none() {
            *cached = Some(std::rc::Rc::new(index(&tree(&self.doc))?));
        }
        Ok(cached.as_ref().unwrap().clone())
    }

    /// Read a single subtree without materializing unrelated XML/package parts.
    pub fn subtree(&self, id: &str) -> Result<StructuredNode> {
        let key = {
            let mut cached = self.structure_index.borrow_mut();
            if cached.is_none() {
                *cached = Some(std::rc::Rc::new(index(&tree(&self.doc))?));
            }
            find(cached.as_ref().unwrap(), id)?
        };
        visit(&self.doc, &tree(&self.doc), key, &mut HashSet::new())
    }
    pub fn create_structured(
        document: &str,
        baseline: &str,
        nodes: Vec<StructuredNode>,
    ) -> Result<Self> {
        let mut result = Self::create(document, baseline, vec![])?;
        result.transact_structured(
            "genesis",
            nodes
                .into_iter()
                .enumerate()
                .map(|(index, node)| NodeOp::Insert {
                    parent: None,
                    index,
                    node,
                })
                .collect(),
            vec![],
            vec![],
        )?;
        result.undo.clear();
        Ok(result)
    }

    /// Includes logical tombstones. A renderer omits a deleted node's subtree.
    pub fn structure(&self) -> Result<Vec<StructuredNode>> {
        read(&self.doc)
    }
}
