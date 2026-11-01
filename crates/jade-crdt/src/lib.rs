//! Text identity and synchronization primitives for Jade.
//! Effective dates and legal amendment dependencies belong to the host application.

use loro::cursor::{Cursor, PosType, Side};
use loro::{ContainerTrait, ExportMode, LoroDoc, LoroText, UndoManager, VersionVector};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

mod transactions;
pub use transactions::{Fact, TextEdit};
mod reconcile;
mod structure;
pub use structure::{NodeOp, StructuredNode};

const MAGIC: &[u8; 8] = b"JCRDT\0\0\x01";
type Result<T> = std::result::Result<T, String>;

#[derive(Deserialize, Serialize)]
pub struct NodeSeed {
    pub id: String,
    pub text: String,
}

pub enum Affinity {
    Before,
    After,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Identity {
    document: String,
    baseline: String,
    genesis: String,
    nodes: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Header {
    Snapshot { identity: Identity },
    Updates { identity: Identity },
    Version { identity: Identity },
    Anchor { identity: Identity, node: String },
}

/// A replica must open the same snapshot as its peers; independently creating the
/// same visible content does not create the same CRDT history.
pub struct Document {
    identity: Identity,
    doc: LoroDoc,
    undo: UndoManager,
    structure_index: RefCell<Option<Rc<HashMap<String, loro::TreeID>>>>,
}

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn encode(header: Header, payload: &[u8]) -> Result<Vec<u8>> {
    let metadata = serde_json::to_vec(&header).map_err(error)?;
    let len = u32::try_from(metadata.len()).map_err(error)?;
    let mut bytes = Vec::with_capacity(12 + metadata.len() + payload.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&len.to_le_bytes());
    bytes.extend_from_slice(&metadata);
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> Result<(Header, &[u8])> {
    if bytes.len() < 12 || &bytes[..8] != MAGIC {
        return Err("invalid CRDT envelope/schema".into());
    }
    let len = u32::from_le_bytes(bytes[8..12].try_into().map_err(error)?) as usize;
    let end = 12usize.checked_add(len).ok_or("invalid metadata length")?;
    let header =
        serde_json::from_slice(bytes.get(12..end).ok_or("truncated metadata")?).map_err(error)?;
    Ok((header, &bytes[end..]))
}

fn validate_identity(identity: &Identity) -> Result<()> {
    let unique: HashSet<_> = identity.nodes.iter().collect();
    if identity.document.is_empty()
        || identity.baseline.is_empty()
        || identity.genesis.is_empty()
        || unique.len() != identity.nodes.len()
        || identity.nodes.iter().any(String::is_empty)
    {
        return Err("document, baseline, genesis and unique node IDs are required".into());
    }
    Ok(())
}

impl Document {
    pub fn create(document: &str, baseline: &str, nodes: Vec<NodeSeed>) -> Result<Self> {
        let doc = LoroDoc::new();
        let identity = Identity {
            document: document.into(),
            baseline: baseline.into(),
            genesis: doc.peer_id().to_string(),
            nodes: nodes.iter().map(|n| n.id.clone()).collect(),
        };
        validate_identity(&identity)?;
        for node in nodes {
            doc.get_text(format!("node:{}", node.id))
                .insert_utf8(0, &node.text)
                .map_err(error)?;
        }
        doc.commit();
        Ok(Self::attach(identity, doc))
    }

    pub fn open(document: &str, baseline: &str, bytes: &[u8]) -> Result<Self> {
        let (Header::Snapshot { identity }, payload) = decode(bytes)? else {
            return Err("expected snapshot".into());
        };
        validate_identity(&identity)?;
        if identity.document != document || identity.baseline != baseline {
            return Err("document/baseline mismatch".into());
        }
        let doc = LoroDoc::new();
        if doc.import(payload).map_err(error)?.pending.is_some() {
            return Err("incomplete snapshot; resync required".into());
        }
        let result = Self::attach(identity, doc);
        result.facts()?;
        result.structure()?;
        Ok(result)
    }

    fn attach(identity: Identity, doc: LoroDoc) -> Self {
        // Genesis/imported history is never a local undo step. Keep this exact
        // LoroDoc alive: replacing it with a fork disconnects the manager.
        let mut undo = UndoManager::new(&doc);
        undo.add_exclude_origin_prefix("jade:fact:");
        Self {
            identity,
            doc,
            undo,
            structure_index: RefCell::new(None),
        }
    }

    pub fn begin_group(&mut self) -> Result<()> {
        self.undo.group_start().map_err(error)
    }
    pub fn end_group(&mut self) {
        self.undo.group_end();
    }

    pub fn can_undo(&self) -> bool {
        self.undo.can_undo()
    }
    pub fn can_redo(&self) -> bool {
        self.undo.can_redo()
    }
    pub fn undo(&mut self) -> Result<bool> {
        self.structure_index.take();
        self.undo.undo().map_err(error)
    }
    pub fn redo(&mut self) -> Result<bool> {
        self.structure_index.take();
        self.undo.redo().map_err(error)
    }

    fn check_identity(&self, identity: &Identity) -> Result<()> {
        if &self.identity != identity {
            return Err("CRDT identity mismatch (document/baseline/genesis/nodes)".into());
        }
        Ok(())
    }

    fn node(&self, id: &str) -> Result<LoroText> {
        if !self.identity.nodes.iter().any(|n| n == id) && !self.structure_ids()?.contains_key(id) {
            return Err("unknown node ID".into());
        }
        Ok(self.doc.get_text(format!("node:{id}")))
    }

    pub fn text(&self, node: &str) -> Result<String> {
        Ok(self.node(node)?.to_string())
    }

    /// Jade offsets are UTF-8 bytes. Validate the entire range before mutation.
    pub fn replace(&mut self, node: &str, start: usize, end: usize, value: &str) -> Result<()> {
        self.transact(
            "text",
            vec![TextEdit {
                node: node.into(),
                start,
                end,
                value: value.into(),
            }],
            vec![],
        )?;
        Ok(())
    }

    pub fn snapshot(&self) -> Result<Vec<u8>> {
        // Full history is retained: anchors to deleted characters must survive.
        encode(
            Header::Snapshot {
                identity: self.identity.clone(),
            },
            &self.doc.export(ExportMode::Snapshot).map_err(error)?,
        )
    }

    pub fn version(&self) -> Result<Vec<u8>> {
        encode(
            Header::Version {
                identity: self.identity.clone(),
            },
            // Loro's native encoding iterates a hash map; it is not a stable
            // equality token across reopened replicas. Sort peer IDs first.
            &serde_json::to_vec(
                &self
                    .doc
                    .oplog_vv()
                    .iter()
                    .map(|(peer, counter)| (*peer, *counter))
                    .collect::<BTreeMap<_, _>>()
                    .into_iter()
                    .collect::<Vec<_>>(),
            )
            .map_err(error)?,
        )
    }

    pub fn updates_since(&self, bytes: &[u8]) -> Result<Vec<u8>> {
        let (Header::Version { identity }, payload) = decode(bytes)? else {
            return Err("expected version".into());
        };
        self.check_identity(&identity)?;
        let entries: Vec<(u64, i32)> = serde_json::from_slice(payload).map_err(error)?;
        let version: VersionVector = entries.into_iter().collect();
        encode(
            Header::Updates { identity },
            &self
                .doc
                .export(ExportMode::updates(&version))
                .map_err(error)?,
        )
    }

    /// Reject missing dependencies atomically. The caller retries with updates
    /// since our version (or a full resync), rather than displaying partial text.
    pub fn import_updates(&mut self, bytes: &[u8]) -> Result<()> {
        let (header, payload) = decode(bytes)?;
        let identity = match header {
            Header::Updates { identity } | Header::Snapshot { identity } => identity,
            _ => return Err("expected updates or snapshot".into()),
        };
        self.check_identity(&identity)?;
        let candidate = self.doc.fork();
        if candidate.import(payload).map_err(error)?.pending.is_some() {
            return Err("missing causal dependency; resync required".into());
        }
        transactions::validate_extension(&self.doc, &candidate)?;
        structure::read(&candidate)?;
        // Candidate validation is read-only with respect to the live replica.
        // Import on the original document so local undo transforms over remote
        // operations instead of losing the manager's subscription.
        self.doc.import(payload).map_err(error)?;
        self.structure_index.take();
        Ok(())
    }

    pub fn anchor(&self, node: &str, offset: usize, affinity: Affinity) -> Result<Vec<u8>> {
        let text = self.node(node)?;
        let current = text.to_string();
        let prefix = current.get(..offset).ok_or("invalid UTF-8 anchor offset")?;
        let index = prefix.chars().count();
        let cursor = match affinity {
            Affinity::Before if index == 0 => Cursor::new(None, text.id(), Side::Left, 0),
            Affinity::Before => text
                .get_cursor(index - 1, Side::Right)
                .ok_or("cannot create anchor")?,
            Affinity::After if index == current.chars().count() => {
                Cursor::new(None, text.id(), Side::Right, index)
            }
            Affinity::After => text
                .get_cursor(index, Side::Left)
                .ok_or("cannot create anchor")?,
        };
        encode(
            Header::Anchor {
                identity: self.identity.clone(),
                node: node.into(),
            },
            &cursor.encode(),
        )
    }

    pub fn resolve_anchor(&self, bytes: &[u8]) -> Result<usize> {
        let (Header::Anchor { identity, node }, payload) = decode(bytes)? else {
            return Err("expected anchor".into());
        };
        self.check_identity(&identity)?;
        let text = self.node(&node)?;
        let cursor = Cursor::decode(payload).map_err(error)?;
        if cursor.container != text.id() {
            return Err("anchor node mismatch".into());
        }
        let result = self.doc.get_cursor_pos(&cursor).map_err(error)?;
        // A live character cursor denotes the character's index. A deleted
        // character resolves to a gap already, and must not advance again.
        let position = result.current.pos
            + usize::from(
                result.update.is_none()
                    && cursor.id.is_some()
                    && result.current.side == Side::Right,
            );
        text.convert_pos(position, PosType::Unicode, PosType::Bytes)
            .ok_or("unresolvable anchor".into())
    }
}
