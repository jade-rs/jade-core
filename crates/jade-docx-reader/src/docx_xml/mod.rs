//! OpenXML の `word/document.xml` を完全に再構築するための汎用 XML ツリー。
//!
//! このモジュールは元の DOCX から読み取った XML 構造を損失なく保持し、編集時に
//! 差分を書き戻すための低レベル API を提供する。段落・表・セルなど任意の要素を
//! 挿入 / 削除できるよう、各要素にはユニークな [`XmlNodeId`] が付与され、親子関係は
//! [`XmlNodePath`] を通じて追跡される。

use quick_xml::escape::unescape;
use quick_xml::events::{attributes::Attribute, BytesDecl, Event};
use quick_xml::Reader;
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

pub use mapping::{
    attach_block_xml_ids, attach_document_xml_ids, attach_text_box_xml_ids, clear_block_xml_ids,
    is_ruby_run, is_transparent_run_container, DocxMappingError,
};

mod mapping;
mod schema;

/// Where an indexed element sits: its parent and its position among the
/// parent's children (text nodes included, as in [`XmlNodePath`]).
///
/// Positions are relative to the parent so that inserting or removing a child
/// only renumbers its later siblings. A full path per element made every
/// paragraph insert re-record the path of every node after it in the body,
/// which dominated an Enter in the middle of a long document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IndexEntry {
    parent: Option<XmlNodeId>,
    position: usize,
}

/// Node ids are sequential integers; SipHash's flooding resistance buys
/// nothing for them, so the index uses a multiplicative hash instead.
#[derive(Default)]
struct NodeIdHasher(u64);

impl Hasher for NodeIdHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.write_u64(u64::from(byte));
        }
    }

    fn write_u32(&mut self, value: u32) {
        self.write_u64(u64::from(value));
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(5) ^ value).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}

type NodeIndex = HashMap<XmlNodeId, IndexEntry, BuildHasherDefault<NodeIdHasher>>;

/// How one edit changed the tree, recorded so the edit can be undone with
/// [`DocxXmlDocument::roll_back`].
///
/// Undo used to keep a deep copy of the whole tree, which on a long document
/// made every paragraph merge spend tens of milliseconds copying
/// `document.xml`. The journal only holds what the edit touched: the earlier
/// contents of each element handed out by [`DocxXmlDocument::element_mut`],
/// and each child inserted or removed.
#[derive(Debug, Default)]
pub struct XmlEditJournal {
    entries: Vec<JournalEntry>,
    modified: HashSet<XmlNodeId, BuildHasherDefault<NodeIdHasher>>,
}

impl XmlEditJournal {
    /// Whether the edit left the tree untouched.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[derive(Debug)]
enum JournalEntry {
    /// The element as it was before the edit first borrowed it mutably.
    Modified {
        id: XmlNodeId,
        before: XmlElement,
    },
    Inserted {
        parent: XmlNodeId,
        index: usize,
    },
    Removed {
        parent: XmlNodeId,
        index: usize,
        node: XmlNode,
    },
}

/// The journal being recorded, if any. A copy of the document (an undo
/// snapshot, say) does not inherit it.
#[derive(Debug, Default)]
struct JournalSlot(Option<XmlEditJournal>);

impl Clone for JournalSlot {
    fn clone(&self) -> Self {
        Self(None)
    }
}

/// DOCX の `word/document.xml` を表現するルート要素。
#[derive(Clone, Debug)]
pub struct DocxXmlDocument {
    root: XmlElement,
    index: NodeIndex,
    next_id: u32,
    declaration: XmlDeclaration,
    /// [`Self::allocate_drawing_object_id`] の採番状態。初回にツリーを走査
    /// した最大値 + 1 から始まり、以後は単調増加する。None は未走査。
    next_drawing_object_id: Option<u32>,
    /// Elements named in [`SELF_CLOSING_TRACKED`] that the source wrote in
    /// empty-element form (`<w:r/>`). The document parser models none of them
    /// as runs, so the model-to-XML run mapping must skip them.
    self_closing_elements: HashSet<XmlNodeId>,
    journal: JournalSlot,
    /// Length of the text the tree was parsed from, which
    /// [`Self::to_string`] reserves up front: a written part is about as long
    /// as its source, and growing the buffer by doubling copied a
    /// multi-megabyte `document.xml` some twenty times per export.
    source_len: usize,
}

/// Elements whose empty-element form the document parser ignores although
/// their start/end form becomes a model run (`<w:fldSimple></w:fldSimple>`
/// is a zero-width field run, `<w:fldSimple/>` is nothing).
const SELF_CLOSING_TRACKED: [&str; 4] = ["w:r", "w:fldSimple", "m:oMath", "w:object"];

impl DocxXmlDocument {
    /// Whether `id` is a `w:r`, `w:fldSimple`, `m:oMath` or `w:object` the
    /// source wrote in empty-element form (see [`Self::self_closing_elements`]).
    /// Elements created after parsing never are.
    pub fn is_self_closing(&self, id: XmlNodeId) -> bool {
        self.self_closing_elements.contains(&id)
    }

    /// 文字列から XML ツリーを構築する。
    pub fn parse(xml: &str) -> Result<Self, DocxXmlError> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(false);

        let mut builder = DocxXmlTreeBuilder::new();
        loop {
            // Events borrow from `xml`; the buffered `read_event_into` copied
            // every event into a scratch vector first.
            let event = reader
                .read_event()
                .map_err(|err| DocxXmlError::Malformed(err.to_string()))?;
            if matches!(event, Event::Eof) {
                break;
            }
            builder.observe(&event)?;
        }
        let mut document = builder.finish()?;
        document.source_len = xml.len();
        Ok(document)
    }

    /// Records a capacity hint after building the tree from a shared event
    /// stream, so serialization can reserve as it does for a standalone parse.
    pub(crate) fn set_source_len(&mut self, source_len: usize) {
        self.source_len = source_len;
    }

    /// Allocates a document-unique DrawingML object id for `wp:docPr@id` /
    /// `pic:cNvPr@id` (ECMA-376 Part 1 §20.4.2.5 requires uniqueness within
    /// the document part). The first call scans the tree for the current
    /// maximum; every call counts up from there, so ids stay unique even
    /// while a rebuild pass holds several freshly built drawings detached
    /// from the tree (a per-call tree scan would hand out the same id to all
    /// of them).
    pub fn allocate_drawing_object_id(&mut self) -> u32 {
        fn scan(element: &XmlElement, maximum: &mut u32) {
            if element.name == "wp:docPr" || element.name == "pic:cNvPr" {
                if let Some(value) = element
                    .attributes
                    .iter()
                    .find(|attribute| attribute.name == "id")
                    .and_then(|attribute| attribute.value.parse::<u32>().ok())
                {
                    *maximum = (*maximum).max(value);
                }
            }
            for child in &element.children {
                if let XmlNode::Element(child) = child {
                    scan(child, maximum);
                }
            }
        }

        let next = self.next_drawing_object_id.unwrap_or_else(|| {
            let mut maximum = 0u32;
            scan(&self.root, &mut maximum);
            maximum.saturating_add(1).max(1)
        });
        self.next_drawing_object_id = Some(next.saturating_add(1));
        next
    }

    /// XML 宣言 (`<?xml ...?>`) を含んだ文字列として書き出す。
    pub fn to_string(&self) -> Result<String, DocxXmlError> {
        let mut buffer = String::with_capacity(self.source_len + self.source_len / 8);
        buffer.push_str(&self.declaration.to_string());
        buffer.push_str("\r\n");
        let plan = schema::write_plan(&self.root);
        self.root.write_with(
            &mut buffer,
            plan.namespaces.as_ref(),
            &plan.renumbered,
            None,
        )?;
        Ok(buffer)
    }

    /// Whether [`Self::to_string`] would write the tree as it is: children in
    /// schema order, every prefix declared, no revision id repeated. A caller
    /// holding a part's original text can keep it byte for byte when it does.
    pub fn follows_schema_rules(&self) -> bool {
        fn ordered(element: &XmlElement, parent: Option<&str>) -> bool {
            schema::schema_order(element, parent).is_none()
                && element.children.iter().all(|child| match child {
                    XmlNode::Element(child) => ordered(child, Some(&element.name)),
                    _ => true,
                })
        }
        schema::write_plan(&self.root).is_empty() && ordered(&self.root, None)
    }

    /// ルート要素 (`<w:document>`) を返す。
    pub fn root(&self) -> &XmlElement {
        &self.root
    }

    /// 指定 ID の要素を取得する。
    pub fn element(&self, id: XmlNodeId) -> Option<&XmlElement> {
        // The walk that checks the index's path finds the element too; going
        // through `path` walked it twice. Loading asks this of every run.
        if let Some(element) = self
            .with_indexed_path(id, |path| Self::element_by_path(&self.root, path))
            .filter(|element| element.id == id)
        {
            return Some(element);
        }
        let mut path = Vec::new();
        Self::find_path(&self.root, id, &mut path)
            .then(|| Self::element_by_path(&self.root, &path))
            .flatten()
    }

    /// 指定 ID の要素を可変参照として取得する。
    ///
    /// While an edit journal is recording, the element's current contents
    /// are kept first so the edit can be rolled back.
    pub fn element_mut(&mut self, id: XmlNodeId) -> Option<&mut XmlElement> {
        let path = self.path(id)?;
        self.record_modification(id, path.as_slice());
        Self::element_by_path_mut(&mut self.root, path.as_slice())
    }

    /// Starts recording how the tree changes, replacing any journal already
    /// being recorded. Finish with [`Self::finish_edit_journal`].
    pub fn begin_edit_journal(&mut self) {
        self.journal.0 = Some(XmlEditJournal::default());
    }

    /// Stops recording and returns what was recorded, or `None` when no
    /// journal was being recorded.
    pub fn finish_edit_journal(&mut self) -> Option<XmlEditJournal> {
        self.journal.0.take()
    }

    /// Undoes a recorded edit: the tree goes back to what it was when the
    /// journal began, provided it is in the state the edit left it in. Node
    /// ids are restored with it; `next_id` keeps counting up.
    pub fn roll_back(&mut self, journal: XmlEditJournal) -> Result<(), DocxXmlError> {
        let recording = self.journal.0.take();
        let result = journal
            .entries
            .into_iter()
            .rev()
            .try_for_each(|entry| match entry {
                JournalEntry::Modified { id, before } => self.restore_element(id, before),
                JournalEntry::Inserted { parent, index } => {
                    self.remove_child_nodes(parent, &[index]).map(|_| ())
                }
                JournalEntry::Removed {
                    parent,
                    index,
                    node,
                } => self.restore_child(parent, index, node),
            });
        self.journal.0 = recording;
        result
    }

    /// Keeps the contents of the element at `path` in the journal being
    /// recorded, unless it or an ancestor was kept already: restoring an
    /// ancestor restores everything below it.
    fn record_modification(&mut self, id: XmlNodeId, path: &[usize]) {
        let Some(journal) = self.journal.0.as_mut() else {
            return;
        };
        let mut current = Some(id);
        while let Some(node) = current {
            if journal.modified.contains(&node) {
                return;
            }
            current = self.index.get(&node).and_then(|entry| entry.parent);
        }
        let Some(element) = Self::element_by_path(&self.root, path) else {
            return;
        };
        journal.modified.insert(id);
        journal.entries.push(JournalEntry::Modified {
            id,
            before: element.clone(),
        });
    }

    fn record(&mut self, entry: impl FnOnce() -> JournalEntry) {
        if let Some(journal) = self.journal.0.as_mut() {
            journal.entries.push(entry());
        }
    }

    fn restore_element(&mut self, id: XmlNodeId, before: XmlElement) -> Result<(), DocxXmlError> {
        let path = self.path(id).ok_or(DocxXmlError::UnknownNode(id))?;
        let Self { root, index, .. } = self;
        let element = Self::element_by_path_mut(root, path.as_slice())
            .ok_or(DocxXmlError::UnknownNode(id))?;
        let current = std::mem::replace(element, before);
        Self::remove_subtree_from_index_recursive(&current, index);
        Self::index_descendants(element, index);
        Ok(())
    }

    /// Puts a removed child back with the ids it had.
    fn restore_child(
        &mut self,
        parent_id: XmlNodeId,
        index: usize,
        node: XmlNode,
    ) -> Result<(), DocxXmlError> {
        let parent_path = self
            .path(parent_id)
            .ok_or(DocxXmlError::UnknownNode(parent_id))?;
        let parent = Self::element_by_path_mut(&mut self.root, parent_path.as_slice())
            .ok_or(DocxXmlError::UnknownNode(parent_id))?;
        if index > parent.children.len() {
            return Err(DocxXmlError::InvalidInsertion(format!(
                "index {index} is out of bounds for parent {}",
                parent.name
            )));
        }
        parent.children.insert(index, node);
        self.reindex_children_from(parent_path.as_slice(), index);
        Ok(())
    }

    /// 指定 ID のノードパスを取得する。
    ///
    /// The path is rebuilt from the parent links and checked against the
    /// tree. When the index has gone stale — a caller rewired `children`
    /// through [`Self::element_mut`] without re-indexing — the element is
    /// searched for instead, so a lookup never returns another element.
    pub fn path(&self, id: XmlNodeId) -> Option<XmlNodePath> {
        if let Some(path) = self.indexed_path(id) {
            if Self::element_by_path(&self.root, path.as_slice())
                .is_some_and(|element| element.id == id)
            {
                return Some(path);
            }
        }
        let mut path = Vec::new();
        Self::find_path(&self.root, id, &mut path).then(|| XmlNodePath::from(path))
    }

    /// 親要素の ID を返す。ルートの場合は `None`。
    pub fn parent_id(&self, id: XmlNodeId) -> Option<XmlNodeId> {
        // Validate the final child while looking up its parent. Going through
        // `path` walked that same prefix once already, then cloned it.
        let parent_at_path = |path: &[usize]| {
            let (&position, parent_path) = path.split_last()?;
            let parent = Self::element_by_path(&self.root, parent_path)?;
            match parent.children.get(position)? {
                XmlNode::Element(child) if child.id == id => Some(parent.id),
                _ => None,
            }
        };
        if let Some(parent) = self.with_indexed_path(id, parent_at_path) {
            return Some(parent);
        }
        // Public child vectors can be changed without rebuilding the index.
        let mut path = Vec::new();
        Self::find_path(&self.root, id, &mut path)
            .then(|| parent_at_path(&path))
            .flatten()
    }

    /// Borrow a short indexed path without allocating. Longer paths keep the
    /// collected positions and continue with the owned-path implementation.
    fn with_indexed_path<T>(
        &self,
        id: XmlNodeId,
        lookup: impl FnOnce(&[usize]) -> Option<T>,
    ) -> Option<T> {
        let mut positions = [0; 8];
        let mut start = positions.len();
        let mut current = id;
        loop {
            let entry = self.index.get(&current)?;
            let Some(parent) = entry.parent else {
                return if current == self.root.id {
                    lookup(&positions[start..])
                } else {
                    None
                };
            };
            if start == 0 {
                let mut overflow = Vec::with_capacity(positions.len() * 2);
                overflow.extend(positions.into_iter().rev());
                let path = self.indexed_path_from(overflow, current)?;
                return lookup(path.as_slice());
            }
            start -= 1;
            positions[start] = entry.position;
            current = parent;
        }
    }

    /// The path recorded by the index, without checking it against the tree.
    fn indexed_path(&self, id: XmlNodeId) -> Option<XmlNodePath> {
        self.indexed_path_from(Vec::new(), id)
    }

    fn indexed_path_from(
        &self,
        mut positions: Vec<usize>,
        mut current: XmlNodeId,
    ) -> Option<XmlNodePath> {
        loop {
            let entry = self.index.get(&current)?;
            let Some(parent) = entry.parent else {
                break;
            };
            // A cycle can only come from a corrupted index; give up on it.
            if positions.len() > self.index.len() {
                return None;
            }
            positions.push(entry.position);
            current = parent;
        }
        if current != self.root.id {
            return None;
        }
        positions.reverse();
        Some(XmlNodePath::from(positions))
    }

    fn find_path(element: &XmlElement, id: XmlNodeId, path: &mut Vec<usize>) -> bool {
        if element.id == id {
            return true;
        }
        for (position, child) in element.children.iter().enumerate() {
            if let XmlNode::Element(child) = child {
                path.push(position);
                if Self::find_path(child, id, path) {
                    return true;
                }
                path.pop();
            }
        }
        false
    }

    /// 新しい要素を生成する。子要素を含む場合は再帰的に ID を割り当てる。
    pub fn create_element(
        &mut self,
        name: impl Into<String>,
        attributes: Vec<XmlAttribute>,
        mut children: Vec<XmlNode>,
    ) -> XmlElement {
        let id = self.allocate_id();
        self.assign_new_ids_for_children(&mut children);
        XmlElement {
            id,
            name: name.into(),
            attributes,
            children,
        }
    }

    /// Reserves a unique element id for callers that must construct a node
    /// while another part of the XML tree is mutably borrowed.
    pub fn allocate_node_id(&mut self) -> XmlNodeId {
        self.allocate_id()
    }

    /// Creates a detached parent while retaining IDs already assigned to its
    /// children by this document. Useful when model bindings were recorded as
    /// the subtree was built bottom-up.
    pub fn create_element_preserving_child_ids(
        &mut self,
        name: impl Into<String>,
        attributes: Vec<XmlAttribute>,
        children: Vec<XmlNode>,
    ) -> XmlElement {
        XmlElement {
            id: self.allocate_id(),
            name: name.into(),
            attributes,
            children,
        }
    }

    /// 親要素の指定した位置にノードを挿入する。
    ///
    /// `XmlNode::Element` を挿入した場合は新しい ID を返す。テキストなどの場合は `None`。
    pub fn insert_node(
        &mut self,
        parent_id: XmlNodeId,
        index: usize,
        node: XmlNode,
    ) -> Result<Option<XmlNodeId>, DocxXmlError> {
        let parent_path = self
            .path(parent_id)
            .ok_or(DocxXmlError::UnknownNode(parent_id))?;

        let mut node = node;
        let inserted_id = if let XmlNode::Element(element) = &mut node {
            self.reassign_ids(element);
            Some(element.id)
        } else {
            None
        };

        let parent = Self::element_by_path_mut(&mut self.root, parent_path.as_slice())
            .ok_or(DocxXmlError::UnknownNode(parent_id))?;

        if index > parent.children.len() {
            return Err(DocxXmlError::InvalidInsertion(format!(
                "index {index} is out of bounds for parent {}",
                parent.name
            )));
        }

        parent.children.insert(index, node);
        self.reindex_children_from(parent_path.as_slice(), index);
        self.record(|| JournalEntry::Inserted {
            parent: parent_id,
            index,
        });
        if let Some(inserted_id) = inserted_id {
            if let Some(max_id) = self.element(inserted_id).and_then(XmlElement::max_node_id) {
                self.next_id = self.next_id.max(max_id.raw().saturating_add(1));
            }
        }
        Ok(inserted_id)
    }

    /// Inserts a detached subtree whose element IDs were allocated by this
    /// document via [`Self::create_element`]. Unlike [`Self::insert_node`], IDs
    /// are preserved so model bindings recorded while constructing the subtree
    /// remain valid.
    pub fn insert_node_preserving_ids(
        &mut self,
        parent_id: XmlNodeId,
        index: usize,
        node: XmlNode,
    ) -> Result<Option<XmlNodeId>, DocxXmlError> {
        fn validate_ids(node: &XmlNode, index: &NodeIndex) -> Result<(), DocxXmlError> {
            if let XmlNode::Element(element) = node {
                if element.id.raw() == 0 || index.contains_key(&element.id) {
                    return Err(DocxXmlError::InvalidInsertion(format!(
                        "element id {} is not available for preserved insertion",
                        element.id.raw()
                    )));
                }
                for child in &element.children {
                    validate_ids(child, index)?;
                }
            }
            Ok(())
        }

        let parent_path = self
            .path(parent_id)
            .ok_or(DocxXmlError::UnknownNode(parent_id))?;
        validate_ids(&node, &self.index)?;
        let inserted_id = match &node {
            XmlNode::Element(element) => Some(element.id),
            _ => None,
        };
        let parent = Self::element_by_path_mut(&mut self.root, parent_path.as_slice())
            .ok_or(DocxXmlError::UnknownNode(parent_id))?;
        if index > parent.children.len() {
            return Err(DocxXmlError::InvalidInsertion(format!(
                "index {index} is out of bounds for parent {}",
                parent.name
            )));
        }
        parent.children.insert(index, node);
        self.reindex_children_from(parent_path.as_slice(), index);
        self.record(|| JournalEntry::Inserted {
            parent: parent_id,
            index,
        });
        Ok(inserted_id)
    }

    /// 指定したノードパスの要素 / ノードを削除し、削除したノードを返す。
    pub fn remove_node(&mut self, path: &XmlNodePath) -> Result<XmlNode, DocxXmlError> {
        if path.is_root() {
            return Err(DocxXmlError::InvalidInsertion(
                "cannot remove root element".into(),
            ));
        }

        let parent_path = path
            .parent()
            .ok_or_else(|| DocxXmlError::InvalidInsertion("invalid path".into()))?;
        let parent = Self::element_by_path_mut(&mut self.root, parent_path.as_slice())
            .ok_or(DocxXmlError::Malformed("parent not found".into()))?;
        let parent_id = parent.id;
        let index = *path
            .as_slice()
            .last()
            .ok_or_else(|| DocxXmlError::InvalidInsertion("empty path".into()))?;

        if index >= parent.children.len() {
            return Err(DocxXmlError::InvalidInsertion(
                "child index out of range".into(),
            ));
        }

        let removed = parent.children.remove(index);

        // インデックスをインクリメンタルに更新する（全ツリー走査の
        // `rebuild_index` を避ける）。削除は 1 ノード（とそのサブツリー）
        // しか触らないので、影響範囲だけ更新すれば O(全ノード) を回避できる。
        //
        // 1) 削除したサブツリーのエントリを index から取り除く。
        if let XmlNode::Element(removed_element) = &removed {
            self.index.remove(&removed_element.id);
            Self::remove_subtree_from_index_recursive(removed_element, &mut self.index);
        }
        // 2) 同じ親の下で、削除位置より後ろにあった兄弟の位置を 1 つ前へ詰める
        //    （子孫は親からの相対位置で持つので触らない）。
        self.reindex_children_from(parent_path.as_slice(), index);
        self.record(|| JournalEntry::Removed {
            parent: parent_id,
            index,
            node: removed.clone(),
        });
        // `next_id` は据え置く: 削除で新しい id が増えることはなく、
        // id は単調増加・再利用しない（最大 id のノードを消したからといって
        // next_id を下げると id の再利用＝衝突を招く）。

        Ok(removed)
    }

    /// Removes multiple direct children under one parent and updates affected
    /// paths once. Indices refer to the parent's child list before removal.
    pub fn remove_child_nodes(
        &mut self,
        parent_id: XmlNodeId,
        child_indices: &[usize],
    ) -> Result<Vec<XmlNode>, DocxXmlError> {
        if child_indices.is_empty() {
            return Ok(Vec::new());
        }

        let parent_path = self
            .path(parent_id)
            .ok_or(DocxXmlError::UnknownNode(parent_id))?;

        let parent = Self::element_by_path_mut(&mut self.root, parent_path.as_slice())
            .ok_or(DocxXmlError::UnknownNode(parent_id))?;

        let mut indices = child_indices.to_vec();
        indices.sort_unstable();
        indices.dedup();
        if indices.len() != child_indices.len() {
            return Err(DocxXmlError::InvalidInsertion(
                "duplicate child index in removal set".into(),
            ));
        }
        if let Some(index) = indices
            .iter()
            .copied()
            .find(|index| *index >= parent.children.len())
        {
            return Err(DocxXmlError::InvalidInsertion(format!(
                "child index {index} is out of range"
            )));
        }

        let start_index = indices[0];
        let mut removed = Vec::with_capacity(indices.len());
        for index in indices.iter().copied().rev() {
            let node = parent.children.remove(index);
            if let XmlNode::Element(element) = &node {
                self.index.remove(&element.id);
                Self::remove_subtree_from_index_recursive(element, &mut self.index);
            }
            removed.push(node);
        }
        // Removed back to front; rolling back re-inserts front to back.
        if let Some(journal) = self.journal.0.as_mut() {
            for (index, node) in indices.iter().rev().zip(&removed) {
                journal.entries.push(JournalEntry::Removed {
                    parent: parent_id,
                    index: *index,
                    node: node.clone(),
                });
            }
        }
        removed.reverse();

        self.reindex_children_from(parent_path.as_slice(), start_index);
        Ok(removed)
    }

    /// インデックスを再構築する。挿入 / 削除後に呼び出す。
    pub fn rebuild_index(&mut self) {
        let max_before = self.root.max_node_id();
        let first_placeholder_id = max_before
            .map(|id| id.raw().saturating_add(1))
            .unwrap_or(self.next_id)
            .max(self.next_id);
        let mut next_id = first_placeholder_id;
        self.index.clear();
        self.index.insert(
            self.root.id,
            IndexEntry {
                parent: None,
                position: 0,
            },
        );
        Self::assign_and_index_descendants(&mut self.root, &mut self.index, &mut next_id);
        self.next_id = next_id;
        if next_id == u32::MAX {
            // Saturation loses whether the last assigned ID was MAX or MAX-1.
            // Keep the original scan and overflow behavior in that case.
            self.next_id = self
                .root
                .max_node_id()
                .map(|id| id.raw() + 1)
                .unwrap_or(self.next_id);
        }
        // Otherwise placeholders ended at next_id - 1, above all existing IDs.
        // `next_id` never goes down, as on a removal: a subtree detached from
        // the tree during the rebuild (a split table's rows, about to be put
        // back under a new table with their ids) still holds ids above the
        // live maximum, and handing those out again gave two elements one id,
        // whose parent links then looped.
    }

    /// 指定した要素のサブツリーのみインデックスを再構築する。
    ///
    /// 段落内のテキスト編集など、子要素のみが変更された場合に使用する。
    /// 全ツリー走査（`rebuild_index`）に比べて大幅に高速。
    pub fn rebuild_index_for_subtree(&mut self, element_id: XmlNodeId) {
        let Some(element_path) = self.path(element_id) else {
            return;
        };
        // `next_id` already covers every unchanged subtree. Only the subtree
        // being rebuilt can contain caller-supplied IDs that it has not seen,
        // so avoid walking the entire document on every local edit.
        let mut next_id = self.next_id;
        if let Some(max_id) = Self::element_by_path(&self.root, element_path.as_slice())
            .and_then(XmlElement::max_node_id)
        {
            next_id = next_id.max(max_id.raw().saturating_add(1));
        }
        if let Some(element) = Self::element_by_path_mut(&mut self.root, element_path.as_slice()) {
            Self::assign_and_index_descendants(element, &mut self.index, &mut next_id);
        }
        self.next_id = next_id;
    }

    /// Assign placeholder ids in preorder and record each child's final id
    /// and position. Inserting replaces its previous entry, so removing those
    /// same entries first would only add another tree walk and hash lookup.
    fn assign_and_index_descendants(
        element: &mut XmlElement,
        index: &mut NodeIndex,
        next_id: &mut u32,
    ) {
        for (position, child) in element.children.iter_mut().enumerate() {
            if let XmlNode::Element(child) = child {
                if child.id.raw() == 0 {
                    child.id = XmlNodeId::new(*next_id);
                    *next_id = (*next_id).saturating_add(1);
                }
                index.insert(
                    child.id,
                    IndexEntry {
                        parent: Some(element.id),
                        position,
                    },
                );
                Self::assign_and_index_descendants(child, index, next_id);
            }
        }
    }

    fn remove_subtree_from_index_recursive(element: &XmlElement, index: &mut NodeIndex) {
        for child in &element.children {
            if let XmlNode::Element(child_element) = child {
                index.remove(&child_element.id);
                Self::remove_subtree_from_index_recursive(child_element, index);
            }
        }
    }

    /// Records every element below `element` under its parent.
    fn index_descendants(element: &XmlElement, index: &mut NodeIndex) {
        for (position, child) in element.children.iter().enumerate() {
            if let XmlNode::Element(child_element) = child {
                index.insert(
                    child_element.id,
                    IndexEntry {
                        parent: Some(element.id),
                        position,
                    },
                );
                Self::index_descendants(child_element, index);
            }
        }
    }

    /// Re-records the children of the element at `parent_path` from
    /// `start_index` on, after an insertion or removal there. A child already
    /// indexed under this parent only moves, so its subtree is left alone; a
    /// child that is new here is indexed together with its subtree.
    fn reindex_children_from(&mut self, parent_path: &[usize], start_index: usize) {
        let Self { root, index, .. } = self;
        let Some(parent) = Self::element_by_path(root, parent_path) else {
            return;
        };
        for (position, child) in parent.children.iter().enumerate().skip(start_index) {
            let XmlNode::Element(child_element) = child else {
                continue;
            };
            match index.get_mut(&child_element.id) {
                Some(entry) if entry.parent == Some(parent.id) => entry.position = position,
                _ => {
                    index.insert(
                        child_element.id,
                        IndexEntry {
                            parent: Some(parent.id),
                            position,
                        },
                    );
                    Self::index_descendants(child_element, index);
                }
            }
        }
    }

    fn element_by_path<'a>(element: &'a XmlElement, path: &[usize]) -> Option<&'a XmlElement> {
        if path.is_empty() {
            return Some(element);
        }
        let idx = path[0];
        match element.children.get(idx)? {
            XmlNode::Element(child) => Self::element_by_path(child, &path[1..]),
            _ => None,
        }
    }

    fn element_by_path_mut<'a>(
        element: &'a mut XmlElement,
        path: &[usize],
    ) -> Option<&'a mut XmlElement> {
        if path.is_empty() {
            return Some(element);
        }
        let idx = path[0];
        match element.children.get_mut(idx)? {
            XmlNode::Element(child) => Self::element_by_path_mut(child, &path[1..]),
            _ => None,
        }
    }

    fn allocate_id(&mut self) -> XmlNodeId {
        let id = XmlNodeId::new(self.next_id);
        self.next_id += 1;
        id
    }

    fn assign_new_ids_for_children(&mut self, children: &mut [XmlNode]) {
        for child in children.iter_mut() {
            if let XmlNode::Element(element) = child {
                self.reassign_ids(element);
            }
        }
    }

    fn reassign_ids(&mut self, element: &mut XmlElement) {
        element.id = self.allocate_id();
        for child in &mut element.children {
            if let XmlNode::Element(child_element) = child {
                self.reassign_ids(child_element);
            }
        }
    }
}

/// Builds a [`DocxXmlDocument`] incrementally from a quick-xml event stream.
///
/// [`DocxXmlDocument::parse`] drives it from its own reader; the semantic
/// parser in `crate::parser` feeds it the very same events it consumes so that
/// `word/document.xml` is tokenized once for both the layout model and the
/// round-trip XML tree. The element ids and the node-path index come out
/// identical to a standalone parse: ids are assigned in document order as each
/// start / empty tag arrives, and every element's path is recorded as soon as
/// its position under the parent is known, so no post-pass over the tree is
/// needed.
#[derive(Debug)]
pub struct DocxXmlTreeBuilder {
    stack: Vec<XmlElement>,
    root: Option<XmlElement>,
    index: NodeIndex,
    next_id: u32,
    declaration: XmlDeclaration,
    self_closing_elements: HashSet<XmlNodeId>,
}

impl Default for DocxXmlTreeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl DocxXmlTreeBuilder {
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            root: None,
            index: NodeIndex::default(),
            next_id: 0,
            declaration: XmlDeclaration::default(),
            self_closing_elements: HashSet::new(),
        }
    }

    /// Feeds one event into the tree. Events must arrive in document order;
    /// `Event::Eof` is ignored so callers can pass every event through.
    pub fn observe(&mut self, event: &Event<'_>) -> Result<(), DocxXmlError> {
        match event {
            Event::Start(e) => {
                let mut element = XmlElement::from_bytes_start(e)?;
                element.id = self.allocate_id();
                let entry = self.next_child_entry()?;
                self.index.insert(element.id, entry);
                self.stack.push(element);
            }
            Event::Empty(e) => {
                let mut element = XmlElement::from_bytes_start(e)?;
                element.id = self.allocate_id();
                if SELF_CLOSING_TRACKED.contains(&element.name.as_str()) {
                    self.self_closing_elements.insert(element.id);
                }
                let entry = self.next_child_entry()?;
                self.index.insert(element.id, entry);
                self.attach_child(XmlNode::Element(element))?;
            }
            Event::End(_) => {
                let element = self
                    .stack
                    .pop()
                    .ok_or_else(|| DocxXmlError::Malformed("unexpected closing tag".into()))?;
                self.attach_child(XmlNode::Element(element))?;
            }
            Event::Decl(e) => {
                self.declaration = XmlDeclaration::from_bytes_decl(e)?;
            }
            Event::Text(e) => {
                if let Some(current) = self.stack.last_mut() {
                    let decoded = e
                        .decode()
                        .map_err(|err| DocxXmlError::Malformed(err.to_string()))?;
                    let text = unescape(decoded.as_ref())
                        .map_err(|err| DocxXmlError::Malformed(err.to_string()))?;
                    Self::push_text(current, &text);
                }
            }
            Event::GeneralRef(e) => {
                // quick-xml reports `&amp;` / `&quot;` / `&#39;` and friends
                // as their own event instead of folding them into the
                // surrounding `Text`. Ignoring it silently deletes the
                // character from the preserved XML tree, so `export_docx`
                // wrote back text with every escaped character missing.
                if let Some(current) = self.stack.last_mut() {
                    let name = e
                        .decode()
                        .map_err(|err| DocxXmlError::Malformed(err.to_string()))?;
                    if let Some(text) = quick_xml::escape::resolve_predefined_entity(&name) {
                        Self::push_text(current, text);
                    } else if let Ok(Some(character)) = e.resolve_char_ref() {
                        Self::push_text(current, character.encode_utf8(&mut [0; 4]));
                    } else {
                        // Keep the original error and partial builder state for
                        // malformed/unknown references, including manual events.
                        let reference = format!("&{name};");
                        let text = unescape(&reference)
                            .map_err(|err| DocxXmlError::Malformed(err.to_string()))?;
                        Self::push_text(current, &text);
                    }
                }
            }
            Event::CData(e) => {
                if let Some(current) = self.stack.last_mut() {
                    current
                        .children
                        .push(XmlNode::CData(String::from_utf8_lossy(e).to_string()));
                }
            }
            Event::Comment(e) => {
                if let Some(current) = self.stack.last_mut() {
                    current
                        .children
                        .push(XmlNode::Comment(String::from_utf8_lossy(e).to_string()));
                }
            }
            Event::Eof | Event::PI(_) | Event::DocType(_) => {}
        }
        Ok(())
    }

    /// Completes the tree. Fails when the stream ended inside an element or
    /// never produced a root.
    pub fn finish(self) -> Result<DocxXmlDocument, DocxXmlError> {
        if !self.stack.is_empty() {
            return Err(DocxXmlError::Malformed(
                "document ended before every element was closed".into(),
            ));
        }
        let root = self
            .root
            .ok_or_else(|| DocxXmlError::Malformed("missing root element".into()))?;
        Ok(DocxXmlDocument {
            root,
            index: self.index,
            next_id: self.next_id,
            declaration: self.declaration,
            next_drawing_object_id: None,
            self_closing_elements: self.self_closing_elements,
            journal: JournalSlot::default(),
            source_len: 0,
        })
    }

    fn allocate_id(&mut self) -> XmlNodeId {
        let id = XmlNodeId::new(self.next_id);
        self.next_id += 1;
        id
    }

    /// Index entry of the element that is about to become the next child of
    /// the open element: its position is the open element's current child
    /// count (text and comment nodes occupy child slots too, and they are
    /// always attached before the next element starts). With nothing open it
    /// is the root.
    fn next_child_entry(&self) -> Result<IndexEntry, DocxXmlError> {
        let Some(parent) = self.stack.last() else {
            if self.root.is_some() {
                return Err(DocxXmlError::Malformed(
                    "multiple root elements encountered".into(),
                ));
            }
            return Ok(IndexEntry {
                parent: None,
                position: 0,
            });
        };
        Ok(IndexEntry {
            parent: Some(parent.id),
            position: parent.children.len(),
        })
    }

    /// Appends character data to `parent`, merging it into a trailing text node
    /// when there is one. Entity references split one logical run of text into
    /// several events; consumers such as the run-splitting code in
    /// `jade-docx-editor` look at the *first* text child only, so the tree must keep
    /// contiguous character data in a single [`XmlNode::Text`].
    fn push_text(parent: &mut XmlElement, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some(XmlNode::Text(existing)) = parent.children.last_mut() {
            existing.push_str(text);
            return;
        }
        let node = XmlNode::Text(text.to_string());
        if parent.children.is_empty()
            && matches!(
                parent.name.as_str(),
                "w:t" | "w:delText" | "w:instrText" | "w:delInstrText" | "m:t"
            )
        {
            // DOCX text elements usually have just one child. Avoid allocating
            // four large XmlNodes, while retaining normal growth for mixed XML.
            parent.children = vec![node];
        } else {
            parent.children.push(node);
        }
    }

    // Keep the large element variant from moving through another call frame
    // before it reaches the parent's child vector.
    #[inline(always)]
    fn attach_child(&mut self, child: XmlNode) -> Result<(), DocxXmlError> {
        if let Some(parent) = self.stack.last_mut() {
            parent.children.push(child);
            Ok(())
        } else if self.root.is_none() {
            match child {
                XmlNode::Element(element) => {
                    self.root = Some(element);
                    Ok(())
                }
                _ => Err(DocxXmlError::Malformed(
                    "root node must be an element".into(),
                )),
            }
        } else {
            Err(DocxXmlError::Malformed(
                "multiple root elements encountered".into(),
            ))
        }
    }
}

/// XML ノード。
#[derive(Clone, Debug)]
pub enum XmlNode {
    Element(XmlElement),
    Text(String),
    CData(String),
    Comment(String),
}

#[derive(Clone, Debug)]
struct XmlDeclaration {
    version: Option<String>,
    encoding: Option<String>,
    standalone: Option<String>,
}

impl Default for XmlDeclaration {
    fn default() -> Self {
        Self {
            version: Some("1.0".to_string()),
            encoding: Some("UTF-8".to_string()),
            standalone: None,
        }
    }
}

impl XmlDeclaration {
    fn from_bytes_decl(decl: &BytesDecl<'_>) -> Result<Self, DocxXmlError> {
        fn decode_bytes(bytes: &[u8]) -> Result<String, DocxXmlError> {
            Ok(std::str::from_utf8(bytes)
                .map_err(|err| DocxXmlError::Malformed(err.to_string()))?
                .to_string())
        }

        let version = decode_bytes(decl.version()?.as_ref())?;

        let encoding = match decl.encoding() {
            Some(value) => Some(decode_bytes(value?.as_ref())?),
            None => None,
        };

        let standalone = match decl.standalone() {
            Some(value) => Some(decode_bytes(value?.as_ref())?),
            None => None,
        };

        Ok(Self {
            version: Some(version),
            encoding,
            standalone,
        })
    }
}

impl std::fmt::Display for XmlDeclaration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<?xml")?;
        if let Some(version) = &self.version {
            write!(f, " version=\"{version}\"")?;
        }
        if let Some(encoding) = &self.encoding {
            write!(f, " encoding=\"{encoding}\"")?;
        }
        if let Some(standalone) = &self.standalone {
            write!(f, " standalone=\"{standalone}\"")?;
        }
        f.write_str("?>")
    }
}

/// 汎用 XML 要素。
#[derive(Clone, Debug)]
pub struct XmlElement {
    pub id: XmlNodeId,
    pub name: String,
    pub attributes: Vec<XmlAttribute>,
    pub children: Vec<XmlNode>,
}

impl XmlElement {
    fn from_bytes_start(e: &quick_xml::events::BytesStart<'_>) -> Result<Self, DocxXmlError> {
        let name = String::from_utf8(e.name().as_ref().to_vec())
            .map_err(|err| DocxXmlError::Malformed(err.to_string()))?;
        let attributes = XmlAttribute::parse_attributes(e)?;
        Ok(Self {
            id: XmlNodeId::new(0),
            name,
            attributes,
            children: Vec::new(),
        })
    }

    /// Writes the element with its children in schema order (see
    /// [`schema::schema_order`]; `parent` is the enclosing element's name),
    /// the namespace declarations `fix` adds (on the root), and the revision
    /// ids `renumbered` replaces.
    fn write_with(
        &self,
        buffer: &mut String,
        fix: Option<&schema::RootNamespaceFix>,
        renumbered: &HashMap<XmlNodeId, String>,
        parent: Option<&str>,
    ) -> Result<(), DocxXmlError> {
        fn push_attribute(buffer: &mut String, name: &str, value: &str) {
            buffer.push(' ');
            buffer.push_str(name);
            buffer.push('=');
            buffer.push('"');
            push_escaped_attribute(buffer, value);
            buffer.push('"');
        }

        buffer.push('<');
        buffer.push_str(&self.name);
        let ignorable = fix.and_then(|fix| fix.ignorable.as_deref());
        // Hashing every element's id for a map that is almost always empty
        // showed up in export profiles.
        let id = if renumbered.is_empty() {
            None
        } else {
            renumbered.get(&self.id).map(String::as_str)
        };
        for attr in &self.attributes {
            let value = match (attr.name.as_str(), ignorable, id) {
                ("mc:Ignorable", Some(value), _) | ("w:id", _, Some(value)) => value,
                _ => &attr.value,
            };
            push_attribute(buffer, &attr.name, value);
        }
        if let Some(fix) = fix {
            for (name, value) in &fix.declarations {
                push_attribute(buffer, name, value);
            }
            if let Some(value) = ignorable {
                if !self
                    .attributes
                    .iter()
                    .any(|attr| attr.name == "mc:Ignorable")
                {
                    push_attribute(buffer, "mc:Ignorable", value);
                }
            }
        }

        if self.children.is_empty() {
            buffer.push_str("/>");
            return Ok(());
        }

        buffer.push('>');

        let order = schema::schema_order(self, parent);
        let children = order
            .iter()
            .flatten()
            .map(|&index| &self.children[index])
            .chain(
                order
                    .is_none()
                    .then_some(self.children.iter())
                    .into_iter()
                    .flatten(),
            );
        for child in children {
            match child {
                XmlNode::Element(element) => {
                    element.write_with(buffer, None, renumbered, Some(&self.name))?
                }
                XmlNode::Text(text) => push_escaped_text(buffer, text),
                XmlNode::CData(data) => {
                    buffer.push_str("<![CDATA[");
                    buffer.push_str(data);
                    buffer.push_str("]]>");
                }
                XmlNode::Comment(comment) => {
                    buffer.push_str("<!--");
                    buffer.push_str(comment);
                    buffer.push_str("-->");
                }
            }
        }

        buffer.push_str("</");
        buffer.push_str(&self.name);
        buffer.push('>');
        Ok(())
    }

    fn max_node_id(&self) -> Option<XmlNodeId> {
        let mut max_id = self.id;
        for child in &self.children {
            if let XmlNode::Element(element) = child {
                if let Some(child_max) = element.max_node_id() {
                    if child_max.raw() > max_id.raw() {
                        max_id = child_max;
                    }
                }
            }
        }
        Some(max_id)
    }
}

/// 汎用 XML 属性。
#[derive(Clone, Debug)]
pub struct XmlAttribute {
    pub name: String,
    pub value: String,
}

impl XmlAttribute {
    fn parse_attributes(e: &quick_xml::events::BytesStart<'_>) -> Result<Vec<Self>, DocxXmlError> {
        // Attribute names already live in the result. Reuse them for duplicate
        // detection instead of allocating quick-xml's separate key-range vector
        // for every element with attributes.
        // Each attribute has an '='. Values may contain it too, so this is
        // only a bounded capacity hint; parsing and validation stay below.
        let capacity = e
            .attributes_raw()
            .iter()
            .filter(|&&byte| byte == b'=')
            .count()
            .min(8);
        let mut attributes = Vec::<Self>::with_capacity(capacity);
        let mut iter = e.attributes();
        iter.with_checks(false);
        for attr in iter {
            let attr = match attr {
                Ok(attr)
                    if !attributes
                        .iter()
                        .any(|previous| previous.name.as_bytes() == attr.key.as_ref()) =>
                {
                    attr
                }
                _ => {
                    // Replay invalid input with the original checks to retain
                    // error offsets and precedence (a duplicate key can take
                    // precedence over a malformed value on that attribute).
                    return e
                        .attributes()
                        .map(|attr| {
                            attr.map_err(DocxXmlError::from)
                                .and_then(Self::from_attribute)
                        })
                        .collect();
                }
            };
            attributes.push(Self::from_attribute(attr)?);
        }
        Ok(attributes)
    }

    fn from_attribute(attr: Attribute<'_>) -> Result<Self, DocxXmlError> {
        let name = String::from_utf8(attr.key.as_ref().to_vec())
            .map_err(|err| DocxXmlError::Malformed(err.to_string()))?;
        let value = attr
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|err| DocxXmlError::Malformed(err.to_string()))?;
        Ok(Self {
            name,
            value: value.into_owned(),
        })
    }
}

/// XML ノードを一意に表す ID。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct XmlNodeId(u32);

impl XmlNodeId {
    pub fn new(value: u32) -> Self {
        Self(value)
    }

    pub fn raw(&self) -> u32 {
        self.0
    }
}

/// XML ツリー内の位置を表すインデックス列。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct XmlNodePath(Vec<usize>);

impl XmlNodePath {
    pub fn root() -> Self {
        Self(Vec::new())
    }

    pub fn with_child(&self, index: usize) -> Self {
        let mut path = self.clone();
        path.0.push(index);
        path
    }

    pub fn parent(&self) -> Option<Self> {
        if self.0.is_empty() {
            return None;
        }
        let mut path = self.clone();
        path.0.pop();
        Some(path)
    }

    pub fn as_slice(&self) -> &[usize] {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Vec<usize>> for XmlNodePath {
    fn from(value: Vec<usize>) -> Self {
        Self(value)
    }
}

impl From<XmlNodePath> for Vec<usize> {
    fn from(value: XmlNodePath) -> Self {
        value.0
    }
}

/// XML パーシング / 操作エラー。
#[derive(Debug)]
pub enum DocxXmlError {
    Malformed(String),
    Io(std::io::Error),
    UnknownNode(XmlNodeId),
    InvalidInsertion(String),
}

impl From<std::io::Error> for DocxXmlError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<quick_xml::Error> for DocxXmlError {
    fn from(err: quick_xml::Error) -> Self {
        Self::Malformed(err.to_string())
    }
}

impl From<quick_xml::events::attributes::AttrError> for DocxXmlError {
    fn from(err: quick_xml::events::attributes::AttrError) -> Self {
        Self::Malformed(err.to_string())
    }
}

impl std::fmt::Display for DocxXmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DocxXmlError::Malformed(err) => write!(f, "malformed xml: {err}"),
            DocxXmlError::Io(err) => write!(f, "io error: {err}"),
            DocxXmlError::UnknownNode(id) => write!(f, "xml node not found: {}", id.raw()),
            DocxXmlError::InvalidInsertion(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for DocxXmlError {}

/// Appends `value` with `&`, `<` and `>` escaped.
///
/// Serialising a part escaped every text node and attribute into a new string
/// grown a character at a time; on a large `document.xml` that was a sizeable
/// share of each export. Unescaped stretches are copied as whole slices.
fn push_escaped_text(buffer: &mut String, value: &str) {
    push_escaped(buffer, value, |byte| match byte {
        b'&' => Some("&amp;"),
        b'<' => Some("&lt;"),
        b'>' => Some("&gt;"),
        _ => None,
    });
}

/// Appends `value` with `&`, `<`, `"` and `'` escaped.
fn push_escaped_attribute(buffer: &mut String, value: &str) {
    push_escaped(buffer, value, |byte| match byte {
        b'&' => Some("&amp;"),
        b'<' => Some("&lt;"),
        b'"' => Some("&quot;"),
        b'\'' => Some("&apos;"),
        // XML 属性の空白正規化で値が変わらないよう、制御文字は文字参照にする。
        b'\n' => Some("&#10;"),
        b'\r' => Some("&#13;"),
        b'\t' => Some("&#9;"),
        _ => None,
    });
}

fn push_escaped(buffer: &mut String, value: &str, entity: impl Fn(u8) -> Option<&'static str>) {
    // Every escaped character is ASCII, and ASCII bytes never occur inside a
    // multi-byte UTF-8 sequence, so each match is also a char boundary.
    let mut start = 0;
    for (index, byte) in value.bytes().enumerate() {
        if let Some(entity) = entity(byte) {
            buffer.push_str(&value[start..index]);
            buffer.push_str(entity);
            start = index + 1;
        }
    }
    buffer.push_str(&value[start..]);
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn attribute_whitespace_survives_roundtrip() {
        let document = DocxXmlDocument::parse(r#"<root value="a&#9;b&#10;c&#13;d"/>"#).unwrap();
        let serialized = document.to_string().unwrap();
        let reopened = DocxXmlDocument::parse(&serialized).unwrap();
        assert_eq!(reopened.root().attributes[0].value, "a\tb\nc\rd");
    }

    use proptest::prelude::*;

    fn checked_attributes(
        element: &quick_xml::events::BytesStart<'_>,
    ) -> Result<Vec<XmlAttribute>, DocxXmlError> {
        element
            .attributes()
            .map(|attr| {
                attr.map_err(DocxXmlError::from)
                    .and_then(XmlAttribute::from_attribute)
            })
            .collect()
    }

    fn attribute_result(
        result: Result<Vec<XmlAttribute>, DocxXmlError>,
    ) -> Result<Vec<(String, String)>, String> {
        result
            .map(|attrs| attrs.into_iter().map(|a| (a.name, a.value)).collect())
            .map_err(|error| format!("{error:?}"))
    }

    #[test]
    fn attribute_errors_keep_checked_iterator_precedence_and_offsets() {
        for content in [
            "r",
            r#"r a="x" b='日本語' xmlns:w="urn:word" w:id="&#49;&amp;""#,
            r#"r a="a=b=c=d=e=f=g=h=i=j" b="" c='日本語=値'"#,
            r#"r a="1" b="2" c="3" d="4" e="5" f="6" g="7" h="8" i="9" j="10""#,
            r#"r a="1" b="2" c="3" d="4" e="5" f="6" g="7" h="8" a=unquoted"#,
            r#"r a="1" a="2""#,
            r#"r a="1" a="&unknown;""#,
            r#"r a="1" a=unquoted"#,
            r#"r a="1" a="unterminated"#,
            r#"r a="1" a="#,
            r#"r a="1" a"#,
            r#"r a="&unknown;" a=unquoted"#,
            r#"r a="&#x110000;" b="&unknown;""#,
            r#"r a="1" 日本語="値" a="2""#,
        ] {
            let element = quick_xml::events::BytesStart::from_content(content, 1);
            assert_eq!(
                attribute_result(XmlAttribute::parse_attributes(&element)),
                attribute_result(checked_attributes(&element)),
                "{content}",
            );
        }
        for xml in [
            b"<r a=\"\xff\"/>".as_slice(),
            b"<r \xff=\"value\"/>",
            b"<r a=\"x\" a=\"\xff\"/>",
            b"<r a=\"\xff\" a=unquoted/>",
        ] {
            let mut reader = Reader::from_reader(xml);
            let Event::Empty(element) = reader.read_event().unwrap() else {
                panic!("expected an empty element");
            };
            assert_eq!(
                attribute_result(XmlAttribute::parse_attributes(&element)),
                attribute_result(checked_attributes(&element)),
                "{xml:?}",
            );
        }
    }

    proptest! {
        #[test]
        fn attribute_parsing_matches_checked_iterator(
            fragments in proptest::collection::vec(
                prop_oneof![
                    ("[a-c:]{1,4}|日本語", "[a-z&;#0-9]{0,12}").prop_map(|(key, value)| format!(" {key}=\"{value}\"")),
                    prop::sample::select(vec![
                        " a='x'", " a=plain", " a=\"", " a=", " a", " =\"x\"",
                        " a=\"&unknown;\"", " a=\"&#0;\"", " a=\"&#x1F600;\"", "\t", "\n",
                    ]).prop_map(str::to_owned),
                ],
                0..32,
            ),
        ) {
            let content = format!("r{}", fragments.concat());
            let element = quick_xml::events::BytesStart::from_content(content, 1);
            prop_assert_eq!(
                attribute_result(XmlAttribute::parse_attributes(&element)),
                attribute_result(checked_attributes(&element)),
            );
        }

        #[test]
        fn attribute_parsing_matches_checked_iterator_for_non_utf8_input(
            bytes in proptest::collection::vec(any::<u8>(), 0..128),
        ) {
            let mut xml = b"<r a=\"valid\" ".to_vec();
            xml.extend(bytes);
            xml.extend_from_slice(b"/>");
            let mut reader = Reader::from_reader(xml.as_slice());
            if let Ok(Event::Start(element) | Event::Empty(element)) = reader.read_event() {
                prop_assert_eq!(
                    attribute_result(XmlAttribute::parse_attributes(&element)),
                    attribute_result(checked_attributes(&element)),
                );
            }
        }
    }

    /// The character-at-a-time escaping the writer used before it escaped
    /// into the output buffer; kept as the reference for byte equality.
    fn reference_escape(value: &str, attribute: bool) -> String {
        let mut escaped = String::new();
        for ch in value.chars() {
            match ch {
                '&' => escaped.push_str("&amp;"),
                '<' => escaped.push_str("&lt;"),
                '>' if !attribute => escaped.push_str("&gt;"),
                '"' if attribute => escaped.push_str("&quot;"),
                '\'' if attribute => escaped.push_str("&apos;"),
                '\n' if attribute => escaped.push_str("&#10;"),
                '\r' if attribute => escaped.push_str("&#13;"),
                '\t' if attribute => escaped.push_str("&#9;"),
                _ => escaped.push(ch),
            }
        }
        escaped
    }

    proptest! {
        #[test]
        fn buffered_escaping_matches_the_reference(
            prefix in ".{0,8}",
            value in "([&<>\"'a-z]|あ|😀|\\PC){0,40}",
        ) {
            let mut text = prefix.clone();
            push_escaped_text(&mut text, &value);
            prop_assert_eq!(text, format!("{prefix}{}", reference_escape(&value, false)));

            let mut attribute = prefix.clone();
            push_escaped_attribute(&mut attribute, &value);
            prop_assert_eq!(attribute, format!("{prefix}{}", reference_escape(&value, true)));
        }
    }

    const SAMPLE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" w:rsidR="0001">
  <w:body>
    <w:p w:rsidR="0002" w14:paraId="12345678">
      <w:pPr>
        <w:pStyle w:val="Heading1"/>
      </w:pPr>
      <w:r w:rsidRPr="0003">
        <w:rPr>
          <w:lang w:val="ja-JP"/>
        </w:rPr>
        <w:t xml:space="preserve"> 見出し</w:t>
      </w:r>
    </w:p>
  </w:body>
</w:document>
"#;

    fn collect_ids(element: &XmlElement, out: &mut Vec<XmlNodeId>) {
        out.push(element.id);
        for child in &element.children {
            if let XmlNode::Element(child) = child {
                collect_ids(child, out);
            }
        }
    }

    /// Every element id with the path the document resolves it to, in
    /// document order. Two documents whose snapshots match address the same
    /// tree with the same ids.
    pub(crate) fn index_snapshot(document: &DocxXmlDocument) -> Vec<(u32, Vec<usize>)> {
        let mut ids = Vec::new();
        collect_ids(document.root(), &mut ids);
        ids.iter()
            .map(|id| {
                let path = document
                    .path(*id)
                    .expect("indexed element")
                    .as_slice()
                    .to_vec();
                assert_eq!(
                    document.element(*id).map(|element| element.id),
                    Some(*id),
                    "path resolves back to the element"
                );
                (id.raw(), path)
            })
            .collect()
    }

    #[test]
    fn builder_index_matches_a_full_rebuild() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <!-- comment before the first block -->
    <w:p><w:r><w:t>a &amp; b</w:t><w:tab/></w:r><w:r><w:t><![CDATA[c]]></w:t></w:r></w:p>
    <w:tbl><w:tr><w:tc><w:p/></w:tc><w:tc><w:p><w:r><w:t>d</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
    <w:sectPr/>
  </w:body>
</w:document>
"#;
        let parsed = DocxXmlDocument::parse(xml).expect("parse");
        let mut rebuilt = parsed.clone();
        rebuilt.rebuild_index();
        assert_eq!(index_snapshot(&parsed), index_snapshot(&rebuilt));
        assert_eq!(parsed.next_id, rebuilt.next_id);
        assert_eq!(
            parsed.to_string().expect("serialize"),
            rebuilt.to_string().expect("serialize")
        );
        // Ids are dense and assigned in document order.
        let ids: Vec<u32> = index_snapshot(&parsed)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(ids, (0..ids.len() as u32).collect::<Vec<_>>());
    }

    #[test]
    fn builder_rejects_unclosed_and_multi_root_input() {
        let unclosed = "<w:document><w:body><w:p/></w:body>";
        assert!(matches!(
            DocxXmlDocument::parse(unclosed),
            Err(DocxXmlError::Malformed(_))
        ));
        let two_roots = "<w:document/><w:document/>";
        assert!(matches!(
            DocxXmlDocument::parse(two_roots),
            Err(DocxXmlError::Malformed(_))
        ));
        assert!(matches!(
            DocxXmlDocument::parse(""),
            Err(DocxXmlError::Malformed(_))
        ));
    }

    #[test]
    fn parse_and_reserialize_preserves_attributes() {
        let document = DocxXmlDocument::parse(SAMPLE_XML).expect("parse");
        let xml = document.to_string().expect("serialize");

        assert!(
            xml.contains(r#"w:rsidR="0001""#),
            "document level attribute should be preserved"
        );
        assert!(
            xml.contains(r#"w14:paraId="12345678""#),
            "paragraph attribute with prefix should be retained"
        );
        assert!(
            xml.contains(r#"xml:space="preserve""#),
            "xml namespace attribute must survive roundtrip"
        );
        assert!(
            xml.contains("見出し"),
            "unicode content should remain intact"
        );
    }

    /// Regression for #16: quick-xml delivers entity references as standalone
    /// `GeneralRef` events, so the preserved-XML tree used to drop them and
    /// `export_docx` wrote text with the escaped characters missing.
    #[test]
    fn entity_references_survive_roundtrip_as_one_text_node() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:t xml:space="preserve">A &amp; B &quot;X&quot; &lt;Y&gt; &apos;Z&apos; &#65;</w:t>
"#;
        let document = DocxXmlDocument::parse(xml).expect("parse");

        // Character data must stay in a single text node: consumers read the
        // first text child of `w:t` when splitting runs.
        let children = &document.root().children;
        assert_eq!(children.len(), 1, "expected one text child: {children:?}");
        let XmlNode::Text(text) = &children[0] else {
            panic!("expected a text node, got {:?}", children[0]);
        };
        assert_eq!(text, "A & B \"X\" <Y> 'Z' A");

        let output = document.to_string().expect("serialize");
        assert!(
            output.contains(r#">A &amp; B "X" &lt;Y&gt; 'Z' A</w:t>"#),
            "escaped characters must be written back: {output}"
        );

        // Re-parsing the serialized form yields the same text.
        let reparsed = DocxXmlDocument::parse(&output).expect("reparse");
        let XmlNode::Text(reparsed_text) = &reparsed.root().children[0] else {
            panic!("expected a text node after reparse");
        };
        assert_eq!(reparsed_text, text);
    }

    #[test]
    fn reference_events_preserve_text_and_error_recovery() {
        use quick_xml::events::{BytesEnd, BytesRef, BytesStart, BytesText};

        // Compare against the previous unescape path, including manually
        // supplied events whose payload contains more than one reference.
        for name in [
            "amp",
            "lt",
            "gt",
            "quot",
            "apos",
            "AMP",
            "nbsp",
            "#1",
            "#38",
            "#x41",
            "#xD7FF",
            "#xE000",
            "#xFFFF",
            "#x10000",
            "#x1F600",
            "#x10FFFF",
            "#0",
            "#xD800",
            "#xDFFF",
            "#1114112",
            "#4294967296",
            "#99999999999999999999999",
            "#X41",
            "#-1",
            "#+1",
            "#x+41",
            "#",
            "#x",
            "",
            "unknown",
            "amp;tail",
            "#38;tail",
            "amp;&lt",
            "日本語",
        ] {
            let mut builder = DocxXmlTreeBuilder::new();
            builder
                .observe(&Event::Start(BytesStart::new("root")))
                .unwrap();
            builder
                .observe(&Event::Text(BytesText::new("prefix")))
                .unwrap();
            let reference = format!("&{name};");
            let expected = unescape(&reference);
            let actual = builder.observe(&Event::GeneralRef(BytesRef::new(name)));
            assert_eq!(
                actual.map_err(|error| error.to_string()),
                expected
                    .as_ref()
                    .map(|_| ())
                    .map_err(|error| { DocxXmlError::Malformed(error.to_string()).to_string() }),
                "{name}",
            );
            // An invalid reference must leave the preceding text intact, and
            // subsequent valid events must still merge into the same node.
            builder
                .observe(&Event::GeneralRef(BytesRef::new("amp")))
                .unwrap();
            builder.observe(&Event::End(BytesEnd::new("root"))).unwrap();
            let document = builder.finish().unwrap();
            let [XmlNode::Text(text)] = document.root().children.as_slice() else {
                panic!("expected one text node for {name}");
            };
            assert_eq!(
                text,
                &format!("prefix{}&", expected.unwrap_or_default()),
                "{name}"
            );
        }
    }

    #[test]
    fn handles_comments_and_cdata() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<root>
  <!-- comment -->
  <w:t><![CDATA[<raw>xml</raw>]]></w:t>
</root>
"#;
        let document = DocxXmlDocument::parse(xml).expect("parse");
        let output = document.to_string().expect("serialize");

        assert!(
            output.contains("<!-- comment -->"),
            "comments should be preserved"
        );
        assert!(
            output.contains("<![CDATA[<raw>xml</raw>]]>"),
            "CDATA sections should roundtrip"
        );
    }

    #[test]
    fn insert_node_assigns_unique_ids() {
        let mut document = DocxXmlDocument::parse(SAMPLE_XML).expect("parse");
        let body = document
            .root()
            .children
            .iter()
            .find_map(|node| match node {
                XmlNode::Element(element) if element.name == "w:body" => Some(element.id),
                _ => None,
            })
            .expect("body id");

        let text = document.create_element("w:t", Vec::new(), vec![XmlNode::Text("Hello".into())]);
        let run = document.create_element("w:r", Vec::new(), vec![XmlNode::Element(text)]);
        let paragraph = document.create_element("w:p", Vec::new(), vec![XmlNode::Element(run)]);

        let parent_children_len = document.element(body).unwrap().children.len();
        let new_id = document
            .insert_node(body, parent_children_len, XmlNode::Element(paragraph))
            .expect("insert")
            .expect("element id");

        let element = document.element(new_id).expect("lookup inserted element");
        assert_eq!(element.name, "w:p");
    }

    #[test]
    fn insert_node_updates_paths_for_following_siblings_without_full_rebuild() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:r><w:t>First</w:t></w:r>
    </w:p>
    <w:p>
      <w:r><w:t>Second</w:t></w:r>
    </w:p>
  </w:body>
</w:document>
"#;
        let mut document = DocxXmlDocument::parse(xml).expect("parse");
        let body = document
            .root()
            .children
            .iter()
            .find_map(|node| match node {
                XmlNode::Element(element) if element.name == "w:body" => Some(element.id),
                _ => None,
            })
            .expect("body id");
        let second_paragraph_id = document
            .root()
            .children
            .iter()
            .find_map(|node| match node {
                XmlNode::Element(body) if body.name == "w:body" => body
                    .children
                    .iter()
                    .filter_map(|child| match child {
                        XmlNode::Element(element) if element.name == "w:p" => Some(element.id),
                        _ => None,
                    })
                    .nth(1),
                _ => None,
            })
            .expect("second paragraph id");
        let second_original_path = document
            .path(second_paragraph_id)
            .expect("second path before insert")
            .as_slice()
            .to_vec();

        let text =
            document.create_element("w:t", Vec::new(), vec![XmlNode::Text("Inserted".into())]);
        let run = document.create_element("w:r", Vec::new(), vec![XmlNode::Element(text)]);
        let paragraph = document.create_element("w:p", Vec::new(), vec![XmlNode::Element(run)]);
        let inserted_id = document
            .insert_node(body, 1, XmlNode::Element(paragraph))
            .expect("insert")
            .expect("inserted element");

        let second_path = document
            .path(second_paragraph_id)
            .expect("second path after insert")
            .as_slice()
            .to_vec();
        assert_eq!(
            second_original_path.len(),
            second_path.len(),
            "inserting a sibling should not change path depth"
        );
        assert_eq!(
            &second_original_path[..second_original_path.len() - 1],
            &second_path[..second_path.len() - 1],
            "only the sibling slot should shift after insertion"
        );
        assert_eq!(
            second_path[second_path.len() - 1],
            second_original_path[second_original_path.len() - 1] + 1,
            "following sibling path should shift by one"
        );

        let inserted = document.element(inserted_id).expect("lookup inserted");
        assert_eq!(inserted.name, "w:p");
        let inserted_run = inserted
            .children
            .iter()
            .find_map(|child| match child {
                XmlNode::Element(element) if element.name == "w:r" => Some(element.id),
                _ => None,
            })
            .expect("inserted run id");
        assert!(
            document.element(inserted_run).is_some(),
            "inserted subtree should be indexed"
        );
    }

    /// Checks every element's *indexed* path against its real position. The
    /// public [`DocxXmlDocument::path`] would hide a stale entry behind its
    /// search fallback, so this reads the index directly.
    fn assert_index_matches_tree(document: &DocxXmlDocument) {
        fn walk(
            document: &DocxXmlDocument,
            element: &XmlElement,
            path: &mut Vec<usize>,
            parent: Option<XmlNodeId>,
            live: &mut usize,
        ) {
            assert_eq!(
                document.indexed_path(element.id).map(Vec::from),
                Some(path.clone()),
                "indexed path of element {}",
                element.id.raw()
            );
            assert!(std::ptr::eq(document.element(element.id).unwrap(), element));
            assert_eq!(document.parent_id(element.id), parent);
            *live += 1;
            for (position, child) in element.children.iter().enumerate() {
                if let XmlNode::Element(child) = child {
                    path.push(position);
                    walk(document, child, path, Some(element.id), live);
                    path.pop();
                }
            }
        }
        let mut live = 0;
        walk(document, document.root(), &mut Vec::new(), None, &mut live);
        assert_eq!(live, document.index.len(), "index holds only live elements");
    }

    #[test]
    fn parent_lookup_follows_direct_child_edits_when_index_is_stale() {
        let mut document =
            DocxXmlDocument::parse("<root><a><target/>text<other/></a><b><end/></b></root>")
                .unwrap();
        let root = document.root.id;
        fn id(node: &XmlNode) -> XmlNodeId {
            match node {
                XmlNode::Element(element) => element.id,
                _ => panic!("expected element"),
            }
        }
        let a = id(&document.root.children[0]);
        let b = id(&document.root.children[1]);
        let target = id(&document.element(a).unwrap().children[0]);
        assert_index_matches_tree(&document);
        assert_eq!(document.parent_id(root), None);
        assert_eq!(document.parent_id(XmlNodeId::new(u32::MAX)), None);

        // Leave text at the indexed position, then move the element to another
        // parent without updating the index, as public child vectors allow.
        let moved = document.element_mut(a).unwrap().children.remove(0);
        document.element_mut(b).unwrap().children.push(moved);
        assert_eq!(document.parent_id(target), Some(b));
        assert_eq!(document.parent_id(a), Some(root));

        // The indexed position can also contain a different element.
        document.element_mut(a).unwrap().children.swap(0, 1);
        assert_eq!(document.parent_id(target), Some(b));
        document.root.children.swap(0, 1);
        assert_eq!(document.parent_id(target), Some(b));
        assert_eq!(document.parent_id(a), Some(root));
        assert_eq!(document.parent_id(b), Some(root));

        document.element_mut(b).unwrap().children.pop();
        assert_eq!(document.parent_id(target), None);
    }

    #[test]
    fn lookups_preserve_deep_paths_and_stale_index_fallbacks() {
        fn check(
            document: &DocxXmlDocument,
            element: &XmlElement,
            parent: Option<XmlNodeId>,
            path: &mut Vec<usize>,
        ) {
            assert!(std::ptr::eq(document.element(element.id).unwrap(), element));
            assert_eq!(document.parent_id(element.id), parent);
            assert_eq!(
                document.path(element.id).unwrap().as_slice(),
                path.as_slice()
            );
            for (position, child) in element.children.iter().enumerate() {
                if let XmlNode::Element(child) = child {
                    path.push(position);
                    check(document, child, Some(element.id), path);
                    path.pop();
                }
            }
        }
        for depth in [0, 7, 8, 9, 16, 64] {
            let mut xml = "<leaf/>".to_string();
            for _ in 0..depth {
                xml = format!("<node>text<!--gap-->{xml}tail</node>");
            }
            let mut document = DocxXmlDocument::parse(&xml).unwrap();
            assert_index_matches_tree(&document);
            check(&document, document.root(), None, &mut Vec::new());

            // Shift every descendant's path and attach an unindexed element.
            document
                .root
                .children
                .insert(0, XmlNode::Text("new".into()));
            let added = document.create_element("added", Vec::new(), Vec::new());
            document.root.children.push(XmlNode::Element(added));
            check(&document, document.root(), None, &mut Vec::new());
            assert!(document.element(XmlNodeId::new(u32::MAX)).is_none());
            assert!(document.parent_id(XmlNodeId::new(u32::MAX)).is_none());

            // A corrupt chain must terminate and use the tree search, including
            // when the cycle crosses the stack-buffer boundary.
            let mut ids = Vec::new();
            element_ids(document.root(), &mut ids);
            let leaf = ids[depth];
            document.index.get_mut(&leaf).unwrap().parent = Some(leaf);
            check(&document, document.root(), None, &mut Vec::new());
        }
    }

    fn body_id(document: &DocxXmlDocument) -> XmlNodeId {
        document
            .root()
            .children
            .iter()
            .find_map(|node| match node {
                XmlNode::Element(body) if body.name == "w:body" => Some(body.id),
                _ => None,
            })
            .expect("body id")
    }

    fn element_ids(element: &XmlElement, out: &mut Vec<XmlNodeId>) {
        out.push(element.id);
        for child in &element.children {
            if let XmlNode::Element(child) = child {
                element_ids(child, out);
            }
        }
    }

    #[test]
    fn inserting_before_a_paragraph_leaves_its_subtree_entries_alone() {
        let xml = r#"<w:document xmlns:w="w"><w:body><w:p><w:r><w:t>One</w:t></w:r></w:p> <w:p><w:r><w:t>Two</w:t></w:r></w:p></w:body></w:document>"#;
        let mut document = DocxXmlDocument::parse(xml).expect("parse");
        let body = body_id(&document);
        let second = document.element(body).expect("body").children[2].clone();
        let XmlNode::Element(second) = second else {
            panic!("second paragraph");
        };
        let XmlNode::Element(second_run) = &second.children[0] else {
            panic!("run");
        };
        let run_entry = document.index[&second_run.id];
        let paragraph_entry = document.index[&second.id];

        let paragraph = document.create_element("w:p", Vec::new(), Vec::new());
        document
            .insert_node(body, 0, XmlNode::Element(paragraph))
            .expect("insert");

        // Only the moved sibling is renumbered; its subtree keeps its entries,
        // which is what keeps an insert proportional to the sibling count.
        assert_eq!(document.index[&second_run.id], run_entry);
        assert_eq!(
            document.index[&second.id].position,
            paragraph_entry.position + 1
        );
        assert_index_matches_tree(&document);
    }

    #[derive(Clone, Debug)]
    enum IndexOp {
        Insert {
            parent: usize,
            index: usize,
            depth: usize,
        },
        InsertPreserving {
            parent: usize,
            index: usize,
        },
        Remove {
            element: usize,
        },
        RemoveChildren {
            parent: usize,
            mask: u8,
        },
        RebuildSubtree {
            element: usize,
        },
    }

    fn index_op() -> impl Strategy<Value = IndexOp> {
        prop_oneof![
            (any::<usize>(), any::<usize>(), 0usize..3).prop_map(|(parent, index, depth)| {
                IndexOp::Insert {
                    parent,
                    index,
                    depth,
                }
            }),
            (any::<usize>(), any::<usize>())
                .prop_map(|(parent, index)| IndexOp::InsertPreserving { parent, index }),
            any::<usize>().prop_map(|element| IndexOp::Remove { element }),
            (any::<usize>(), any::<u8>())
                .prop_map(|(parent, mask)| IndexOp::RemoveChildren { parent, mask }),
            any::<usize>().prop_map(|element| IndexOp::RebuildSubtree { element }),
        ]
    }

    proptest! {
        #[test]
        fn incremental_index_edits_keep_every_indexed_path_exact(
            ops in proptest::collection::vec(index_op(), 1..24),
        ) {
            let xml = r#"<w:document xmlns:w="w"><w:body>text<w:p><w:r><w:t>A</w:t></w:r>tail<w:r/></w:p><w:tbl><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl><w:p><w:r><w:t>B</w:t></w:r></w:p></w:body></w:document>"#;
            let mut document = DocxXmlDocument::parse(xml).expect("parse");
            assert_index_matches_tree(&document);
            for op in ops {
                apply_index_op(&mut document, op);
                assert_index_matches_tree(&document);
            }
        }
    }

    #[derive(Clone, Debug)]
    enum JournalOp {
        Index(IndexOp),
        /// Borrows an element mutably and rewrites it the way edits do:
        /// adds an attribute, appends a placeholder child and re-indexes.
        Rewrite {
            element: usize,
        },
    }

    fn journal_op() -> impl Strategy<Value = JournalOp> {
        prop_oneof![
            index_op().prop_map(JournalOp::Index),
            any::<usize>().prop_map(|element| JournalOp::Rewrite { element }),
        ]
    }

    fn apply_index_op(document: &mut DocxXmlDocument, op: IndexOp) {
        let mut ids = Vec::new();
        element_ids(document.root(), &mut ids);
        let pick = |choice: usize| ids[choice % ids.len()];
        match op {
            IndexOp::Insert {
                parent,
                index,
                depth,
            } => {
                let parent = pick(parent);
                let len = document.element(parent).expect("parent").children.len();
                let mut node = XmlNode::Text("t".into());
                for level in 0..=depth {
                    let name = if level == 0 { "w:t" } else { "w:r" };
                    node = XmlNode::Element(document.create_element(
                        name,
                        Vec::new(),
                        vec![node, XmlNode::Text("x".into())],
                    ));
                }
                document
                    .insert_node(parent, index % (len + 1), node)
                    .expect("insert");
            }
            IndexOp::InsertPreserving { parent, index } => {
                let parent = pick(parent);
                let len = document.element(parent).expect("parent").children.len();
                let child = document.create_element("w:t", Vec::new(), Vec::new());
                let node = document.create_element_preserving_child_ids(
                    "w:r",
                    Vec::new(),
                    vec![XmlNode::Element(child)],
                );
                document
                    .insert_node_preserving_ids(parent, index % (len + 1), XmlNode::Element(node))
                    .expect("insert preserving ids");
            }
            IndexOp::Remove { element } => {
                let element = pick(element);
                if element != document.root().id {
                    let path = document.path(element).expect("path");
                    document.remove_node(&path).expect("remove");
                }
            }
            IndexOp::RemoveChildren { parent, mask } => {
                let parent = pick(parent);
                let len = document.element(parent).expect("parent").children.len();
                let indices: Vec<usize> = (0..len.min(8))
                    .filter(|bit| mask & (1 << bit) != 0)
                    .collect();
                document
                    .remove_child_nodes(parent, &indices)
                    .expect("remove children");
            }
            IndexOp::RebuildSubtree { element } => {
                document.rebuild_index_for_subtree(pick(element));
            }
        }
    }

    proptest! {
        #[test]
        fn rolling_back_a_journal_restores_the_tree_and_its_ids(
            ops in proptest::collection::vec(journal_op(), 1..16),
        ) {
            let xml = r#"<w:document xmlns:w="w"><w:body>text<w:p w:a="1"><w:r><w:t>A</w:t></w:r>tail<w:r/></w:p><w:tbl><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl><w:p><w:r><w:t>B</w:t></w:r></w:p></w:body></w:document>"#;
            let mut document = DocxXmlDocument::parse(xml).expect("parse");
            let original_xml = document.to_string().expect("serialize");
            let mut original_ids = Vec::new();
            element_ids(document.root(), &mut original_ids);

            document.begin_edit_journal();
            for op in ops {
                match op {
                    JournalOp::Index(op) => apply_index_op(&mut document, op),
                    JournalOp::Rewrite { element } => {
                        let mut ids = Vec::new();
                        element_ids(document.root(), &mut ids);
                        let id = ids[element % ids.len()];
                        let element = document.element_mut(id).expect("element");
                        element.attributes.push(XmlAttribute {
                            name: "w:edited".into(),
                            value: "1".into(),
                        });
                        element.children.push(XmlNode::Element(XmlElement {
                            id: XmlNodeId::new(0),
                            name: "w:new".into(),
                            attributes: Vec::new(),
                            children: Vec::new(),
                        }));
                        document.rebuild_index_for_subtree(id);
                    }
                }
            }
            let journal = document.finish_edit_journal().expect("journal");
            document.roll_back(journal).expect("roll back");

            prop_assert_eq!(document.to_string().expect("serialize"), original_xml);
            let mut ids = Vec::new();
            element_ids(document.root(), &mut ids);
            prop_assert_eq!(ids, original_ids);
            assert_index_matches_tree(&document);
        }
    }

    #[test]
    fn a_copied_document_does_not_inherit_the_journal_being_recorded() {
        let mut document =
            DocxXmlDocument::parse(r#"<w:document xmlns:w="w"><w:body/></w:document>"#)
                .expect("parse");
        document.begin_edit_journal();
        let mut copy = document.clone();
        assert!(copy.finish_edit_journal().is_none());
        assert!(document
            .finish_edit_journal()
            .is_some_and(|journal| journal.is_empty()));
    }

    #[test]
    fn insert_node_incrementally_matches_full_rebuild() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>First</w:t></w:r></w:p>
    <w:p><w:r><w:t>Second</w:t></w:r></w:p>
    <w:p><w:r><w:t>Third</w:t></w:r></w:p>
  </w:body>
</w:document>
"#;
        let mut document = DocxXmlDocument::parse(xml).expect("parse");
        let body_id = document
            .root()
            .children
            .iter()
            .find_map(|node| match node {
                XmlNode::Element(body) if body.name == "w:body" => Some(body.id),
                _ => None,
            })
            .expect("body id");

        let text =
            document.create_element("w:t", Vec::new(), vec![XmlNode::Text("Inserted".into())]);
        let run = document.create_element("w:r", Vec::new(), vec![XmlNode::Element(text)]);
        let paragraph = document.create_element("w:p", Vec::new(), vec![XmlNode::Element(run)]);
        document
            .insert_node(body_id, 1, XmlNode::Element(paragraph))
            .expect("insert paragraph");

        fn collect_element_ids(element: &XmlElement, out: &mut Vec<XmlNodeId>) {
            out.push(element.id);
            for child in &element.children {
                if let XmlNode::Element(child_element) = child {
                    collect_element_ids(child_element, out);
                }
            }
        }

        let mut ids = Vec::new();
        collect_element_ids(document.root(), &mut ids);
        let incremental: Vec<(XmlNodeId, XmlNodePath)> = ids
            .iter()
            .map(|id| (*id, document.path(*id).expect("incremental path").clone()))
            .collect();

        document.rebuild_index();
        let full: Vec<(XmlNodeId, XmlNodePath)> = ids
            .iter()
            .map(|id| (*id, document.path(*id).expect("rebuilt path").clone()))
            .collect();

        assert_eq!(
            incremental, full,
            "incremental insert_node must produce the same index as a full rebuild"
        );
    }

    #[test]
    fn remove_node_incrementally_matches_full_rebuild() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>First</w:t></w:r></w:p>
    <w:p><w:r><w:t>Second</w:t></w:r></w:p>
    <w:p><w:r><w:t>Third</w:t></w:r></w:p>
  </w:body>
</w:document>
"#;
        let mut document = DocxXmlDocument::parse(xml).expect("parse");

        fn collect_element_ids(element: &XmlElement, out: &mut Vec<XmlNodeId>) {
            out.push(element.id);
            for child in &element.children {
                if let XmlNode::Element(child_element) = child {
                    collect_element_ids(child_element, out);
                }
            }
        }

        let body_id = document
            .root()
            .children
            .iter()
            .find_map(|node| match node {
                XmlNode::Element(body) if body.name == "w:body" => Some(body.id),
                _ => None,
            })
            .expect("body id");
        let paragraph_ids: Vec<XmlNodeId> = document
            .element(body_id)
            .expect("body element")
            .children
            .iter()
            .filter_map(|child| match child {
                XmlNode::Element(element) if element.name == "w:p" => Some(element.id),
                _ => None,
            })
            .collect();
        assert_eq!(paragraph_ids.len(), 3);

        // 削除する段落（最初の段落）とそのサブツリーの id を控えておく。
        let removed_paragraph = document
            .element(paragraph_ids[0])
            .expect("first paragraph")
            .clone();
        let mut removed_ids = Vec::new();
        collect_element_ids(&removed_paragraph, &mut removed_ids);

        // 後続兄弟の削除前の子インデックス（body 直下にはインデント由来の
        // 空白テキストノードも居るので 0/1 のような詰まった値とは限らない）。
        let p2_index_before = *document
            .path(paragraph_ids[1])
            .unwrap()
            .as_slice()
            .last()
            .unwrap();
        let p3_index_before = *document
            .path(paragraph_ids[2])
            .unwrap()
            .as_slice()
            .last()
            .unwrap();

        let first_path = document.path(paragraph_ids[0]).expect("first path").clone();
        document
            .remove_node(&first_path)
            .expect("remove first paragraph");

        // 1) 削除したサブツリーの全 id が index から消えていること。
        for id in &removed_ids {
            assert!(
                document.path(*id).is_none(),
                "removed subtree id {id:?} should be gone from the index"
            );
        }

        // 2) 後続の兄弟（とその子孫）のパスがインクリメンタル更新で
        //    全走査リビルドと完全一致すること（= インクリメンタル更新の正しさ）。
        let mut surviving = Vec::new();
        collect_element_ids(document.root(), &mut surviving);
        let incremental: Vec<(XmlNodeId, XmlNodePath)> = surviving
            .iter()
            .map(|id| (*id, document.path(*id).expect("incremental path").clone()))
            .collect();

        document.rebuild_index();
        let full: Vec<(XmlNodeId, XmlNodePath)> = surviving
            .iter()
            .map(|id| (*id, document.path(*id).expect("rebuilt path").clone()))
            .collect();
        assert_eq!(
            incremental, full,
            "incremental remove_node must produce the same index as a full rebuild"
        );

        // 3) 削除位置より後ろの兄弟は、子インデックスが 1 つ前へ詰まること。
        assert_eq!(
            *document
                .path(paragraph_ids[1])
                .unwrap()
                .as_slice()
                .last()
                .unwrap(),
            p2_index_before - 1
        );
        assert_eq!(
            *document
                .path(paragraph_ids[2])
                .unwrap()
                .as_slice()
                .last()
                .unwrap(),
            p3_index_before - 1
        );
    }

    #[test]
    fn remove_child_nodes_incrementally_matches_full_rebuild() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>First</w:t></w:r></w:p>
    <w:p><w:r><w:t>Second</w:t></w:r></w:p>
    <w:p><w:r><w:t>Third</w:t></w:r></w:p>
    <w:p><w:r><w:t>Fourth</w:t></w:r></w:p>
  </w:body>
</w:document>
"#;
        let mut document = DocxXmlDocument::parse(xml).expect("parse");

        fn collect_element_ids(element: &XmlElement, out: &mut Vec<XmlNodeId>) {
            out.push(element.id);
            for child in &element.children {
                if let XmlNode::Element(child_element) = child {
                    collect_element_ids(child_element, out);
                }
            }
        }

        let body_id = document
            .root()
            .children
            .iter()
            .find_map(|node| match node {
                XmlNode::Element(body) if body.name == "w:body" => Some(body.id),
                _ => None,
            })
            .expect("body id");
        let paragraph_ids: Vec<XmlNodeId> = document
            .element(body_id)
            .expect("body element")
            .children
            .iter()
            .filter_map(|child| match child {
                XmlNode::Element(element) if element.name == "w:p" => Some(element.id),
                _ => None,
            })
            .collect();
        assert_eq!(paragraph_ids.len(), 4);

        let removed_ids = [paragraph_ids[1], paragraph_ids[2]];
        let mut removed_subtree_ids = Vec::new();
        for id in removed_ids {
            collect_element_ids(
                document.element(id).expect("removed paragraph"),
                &mut removed_subtree_ids,
            );
        }
        let remove_indices: Vec<usize> = removed_ids
            .iter()
            .map(|id| {
                *document
                    .path(*id)
                    .expect("paragraph path")
                    .as_slice()
                    .last()
                    .expect("child index")
            })
            .collect();

        document
            .remove_child_nodes(body_id, &remove_indices)
            .expect("remove child nodes");

        for id in &removed_subtree_ids {
            assert!(
                document.path(*id).is_none(),
                "removed subtree id {id:?} should be gone from the index"
            );
        }

        let mut surviving = Vec::new();
        collect_element_ids(document.root(), &mut surviving);
        let incremental: Vec<(XmlNodeId, XmlNodePath)> = surviving
            .iter()
            .map(|id| (*id, document.path(*id).expect("incremental path").clone()))
            .collect();

        document.rebuild_index();
        let full: Vec<(XmlNodeId, XmlNodePath)> = surviving
            .iter()
            .map(|id| (*id, document.path(*id).expect("rebuilt path").clone()))
            .collect();

        assert_eq!(
            incremental, full,
            "batched remove_child_nodes must produce the same index as a full rebuild"
        );
        assert!(document.element(paragraph_ids[0]).is_some());
        assert!(document.element(paragraph_ids[3]).is_some());
    }

    #[test]
    fn remove_node_keeps_next_id_monotonic() {
        // 最大 id のノードを消しても、その後の挿入で id が再利用されないこと。
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>Keep</w:t></w:r></w:p>
    <w:p><w:r><w:t>Drop</w:t></w:r></w:p>
  </w:body>
</w:document>
"#;
        let mut document = DocxXmlDocument::parse(xml).expect("parse");
        let body_id = document
            .root()
            .children
            .iter()
            .find_map(|node| match node {
                XmlNode::Element(body) if body.name == "w:body" => Some(body.id),
                _ => None,
            })
            .expect("body id");
        let last_paragraph_id = document
            .element(body_id)
            .expect("body")
            .children
            .iter()
            .filter_map(|child| match child {
                XmlNode::Element(element) if element.name == "w:p" => Some(element.id),
                _ => None,
            })
            .next_back()
            .expect("last paragraph id");
        let max_before = document.root().max_node_id().expect("max id").raw();

        let path = document.path(last_paragraph_id).expect("path").clone();
        document.remove_node(&path).expect("remove last paragraph");

        // 削除後に新規ノードを挿入すると、消えた最大 id より大きい id が振られる。
        let inserted = document.create_element("w:p", Vec::new(), Vec::new());
        let inserted_id = document
            .insert_node(body_id, 0, XmlNode::Element(inserted))
            .expect("insert")
            .expect("inserted element");
        assert!(
            inserted_id.raw() > max_before,
            "id {} must exceed the pre-removal max {max_before} (no reuse)",
            inserted_id.raw()
        );
    }

    #[test]
    fn preserves_xml_declaration_attributes() {
        let xml = r#"<?xml version="1.1" encoding="UTF-16" standalone="yes"?>
<root/>
"#;
        let document = DocxXmlDocument::parse(xml).expect("parse");
        let serialized = document.to_string().expect("serialize");
        assert!(
            serialized.starts_with(r#"<?xml version="1.1" encoding="UTF-16" standalone="yes"?>"#),
            "declaration should be preserved, got: {serialized}"
        );
    }

    #[test]
    fn rebuild_index_for_subtree_updates_only_children() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:r><w:t>First</w:t></w:r>
    </w:p>
    <w:p>
      <w:r><w:t>Second</w:t></w:r>
    </w:p>
  </w:body>
</w:document>
"#;
        let mut document = DocxXmlDocument::parse(xml).expect("parse");

        // 最初の段落を取得
        let first_paragraph_id = document
            .root()
            .children
            .iter()
            .find_map(|node| match node {
                XmlNode::Element(body) if body.name == "w:body" => {
                    body.children.iter().find_map(|child| match child {
                        XmlNode::Element(p) if p.name == "w:p" => Some(p.id),
                        _ => None,
                    })
                }
                _ => None,
            })
            .expect("first paragraph id");

        // 段落に新しい run を追加
        let new_text =
            document.create_element("w:t", Vec::new(), vec![XmlNode::Text("New".into())]);
        let new_run = document.create_element("w:r", Vec::new(), vec![XmlNode::Element(new_text)]);
        let new_run_id = new_run.id;

        // 段落の children を直接変更
        let first_paragraph = document.element_mut(first_paragraph_id).unwrap();
        first_paragraph.children.push(XmlNode::Element(new_run));

        // サブツリーのインデックスのみ再構築
        document.rebuild_index_for_subtree(first_paragraph_id);

        // 新しく追加した要素がインデックスにあることを確認
        assert!(
            document.element(new_run_id).is_some(),
            "new run should be indexed after rebuild_index_for_subtree"
        );

        // 2番目の段落もまだインデックスにあることを確認
        let second_paragraph_exists = document
            .root()
            .children
            .iter()
            .find_map(|node| match node {
                XmlNode::Element(body) if body.name == "w:body" => body
                    .children
                    .iter()
                    .filter_map(|child| match child {
                        XmlNode::Element(p) if p.name == "w:p" => Some(p.id),
                        _ => None,
                    })
                    .nth(1),
                _ => None,
            })
            .map(|id| document.element(id).is_some())
            .unwrap_or(false);

        assert!(
            second_paragraph_exists,
            "second paragraph should still be indexed"
        );
    }

    #[test]
    fn subtree_rebuild_assigns_ids_in_preorder_and_preserves_rollback() {
        let mut document =
            DocxXmlDocument::parse(r#"<root><p><a/>text<b/></p><p><untouched/></p></root>"#)
                .unwrap();
        let original = document.to_string().unwrap();
        let root = document.root().id;
        let paragraph = match &document.root().children[0] {
            XmlNode::Element(element) => element.id,
            _ => unreachable!(),
        };
        document.begin_edit_journal();
        let element = document.element_mut(paragraph).unwrap();
        element.children.reverse();
        element.children.push(XmlNode::Element(XmlElement {
            id: XmlNodeId::new(0),
            name: "new".into(),
            attributes: Vec::new(),
            children: vec![XmlNode::Element(XmlElement {
                id: XmlNodeId::new(5000),
                name: "explicit".into(),
                attributes: Vec::new(),
                children: vec![XmlNode::Element(XmlElement {
                    id: XmlNodeId::new(0),
                    name: "nested".into(),
                    attributes: Vec::new(),
                    children: Vec::new(),
                })],
            })],
        }));

        document.rebuild_index_for_subtree(paragraph);
        assert_index_matches_tree(&document);
        assert_eq!(document.parent_id(paragraph), Some(root));
        assert_eq!(document.parent_id(XmlNodeId::new(5001)), Some(paragraph));
        assert_eq!(
            document.parent_id(XmlNodeId::new(5000)),
            Some(XmlNodeId::new(5001))
        );
        assert_eq!(
            document.parent_id(XmlNodeId::new(5002)),
            Some(XmlNodeId::new(5000))
        );
        assert_eq!(document.allocate_node_id().raw(), 5003);

        let journal = document.finish_edit_journal().unwrap();
        document.roll_back(journal).unwrap();
        assert_eq!(document.to_string().unwrap(), original);
        assert_index_matches_tree(&document);
        assert_eq!(document.allocate_node_id().raw(), 5004);
    }

    #[test]
    fn rebuild_index_for_subtree_handles_replaced_children() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:r><w:t>Original</w:t></w:r>
    </w:p>
  </w:body>
</w:document>
"#;
        let mut document = DocxXmlDocument::parse(xml).expect("parse");

        // 段落を取得
        let paragraph_id = document
            .root()
            .children
            .iter()
            .find_map(|node| match node {
                XmlNode::Element(body) if body.name == "w:body" => {
                    body.children.iter().find_map(|child| match child {
                        XmlNode::Element(p) if p.name == "w:p" => Some(p.id),
                        _ => None,
                    })
                }
                _ => None,
            })
            .expect("paragraph id");

        // 古いrunのIDを取得
        let old_run_id = document
            .element(paragraph_id)
            .unwrap()
            .children
            .iter()
            .find_map(|child| match child {
                XmlNode::Element(r) if r.name == "w:r" => Some(r.id),
                _ => None,
            })
            .expect("old run id");

        // children を完全に置き換え
        let new_text =
            document.create_element("w:t", Vec::new(), vec![XmlNode::Text("Replaced".into())]);
        let new_run = document.create_element("w:r", Vec::new(), vec![XmlNode::Element(new_text)]);
        let new_run_id = new_run.id;

        let paragraph = document.element_mut(paragraph_id).unwrap();
        paragraph.children = vec![XmlNode::Element(new_run)];

        // サブツリーのインデックスのみ再構築
        document.rebuild_index_for_subtree(paragraph_id);

        // 新しい run がインデックスにあることを確認
        assert!(
            document.element(new_run_id).is_some(),
            "new run should be indexed"
        );

        // 古い run はインデックスから削除されていることを確認
        assert!(
            document.element(old_run_id).is_none(),
            "old run should be removed from index"
        );
    }

    #[test]
    fn rebuild_index_for_subtree_updates_next_id() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:r><w:t>Text</w:t></w:r>
    </w:p>
  </w:body>
</w:document>
"#;
        let mut document = DocxXmlDocument::parse(xml).expect("parse");

        let paragraph_id = document
            .root()
            .children
            .iter()
            .find_map(|node| match node {
                XmlNode::Element(body) if body.name == "w:body" => {
                    body.children.iter().find_map(|child| match child {
                        XmlNode::Element(p) if p.name == "w:p" => Some(p.id),
                        _ => None,
                    })
                }
                _ => None,
            })
            .expect("paragraph id");

        // 高い ID を持つ要素を手動で作成して追加
        let new_text = XmlElement {
            id: XmlNodeId::new(9999),
            name: "w:t".to_string(),
            attributes: Vec::new(),
            children: vec![XmlNode::Text("High ID".into())],
        };
        let new_run = XmlElement {
            id: XmlNodeId::new(9998),
            name: "w:r".to_string(),
            attributes: Vec::new(),
            children: vec![XmlNode::Element(new_text)],
        };

        let paragraph = document.element_mut(paragraph_id).unwrap();
        paragraph.children.push(XmlNode::Element(new_run));

        // サブツリーのインデックスのみ再構築
        document.rebuild_index_for_subtree(paragraph_id);

        // next_id が更新されていることを確認
        let next_element = document.create_element("test", Vec::new(), Vec::new());
        assert!(
            next_element.id.raw() > 9999,
            "next_id should be updated to be greater than max id in subtree, got: {}",
            next_element.id.raw()
        );
    }

    #[test]
    fn full_rebuild_keeps_next_id_at_saturation_boundaries() {
        for (existing_id, seed, placeholder, expected_next) in [
            (u32::MAX - 2, 0, true, u32::MAX),
            (10, u32::MAX - 1, true, u32::MAX),
            (10, u32::MAX, false, 11),
            (10, u32::MAX - 2, true, u32::MAX - 1),
            (u32::MAX - 1, 0, false, u32::MAX),
            (10, u32::MAX - 1, false, u32::MAX - 1),
        ] {
            let mut document = DocxXmlDocument::parse("<root><existing/></root>").unwrap();
            let XmlNode::Element(existing) = &mut document.root.children[0] else {
                unreachable!();
            };
            existing.id = XmlNodeId::new(existing_id);
            document.next_id = seed;
            if placeholder {
                document.root.children.push(XmlNode::Element(XmlElement {
                    id: XmlNodeId::new(0),
                    name: "new".into(),
                    attributes: Vec::new(),
                    children: Vec::new(),
                }));
            }

            document.rebuild_index();

            assert_eq!(document.next_id, expected_next);
            assert_index_matches_tree(&document);
            if placeholder {
                let XmlNode::Element(assigned) = &document.root.children[1] else {
                    unreachable!();
                };
                assert_eq!(assigned.id.raw(), expected_next - 1);
            }
        }
    }

    #[test]
    fn full_rebuild_preserves_preorder_ids_and_never_reuses_ids() {
        let mut document = DocxXmlDocument::parse("<root><a/>text<b/></root>").unwrap();
        let root = document.root().id;
        let old_max = document.root().max_node_id().unwrap().raw();
        for _ in 0..10 {
            document.allocate_node_id();
        }
        document.rebuild_index();
        assert_eq!(document.allocate_node_id().raw(), old_max + 11);

        let removed_id = match &document.root().children[0] {
            XmlNode::Element(element) => element.id,
            _ => unreachable!(),
        };
        let element = document.element_mut(root).unwrap();
        element.children.remove(0);
        element.children.reverse();
        element.children.push(XmlNode::Element(XmlElement {
            id: XmlNodeId::new(0),
            name: "new".into(),
            attributes: Vec::new(),
            children: vec![XmlNode::Element(XmlElement {
                id: XmlNodeId::new(5000),
                name: "explicit".into(),
                attributes: Vec::new(),
                children: vec![XmlNode::Element(XmlElement {
                    id: XmlNodeId::new(0),
                    name: "nested".into(),
                    attributes: Vec::new(),
                    children: Vec::new(),
                })],
            })],
        }));

        document.rebuild_index();
        assert_index_matches_tree(&document);
        assert!(!document.index.contains_key(&removed_id));
        assert_eq!(document.parent_id(XmlNodeId::new(5001)), Some(root));
        assert_eq!(
            document.parent_id(XmlNodeId::new(5000)),
            Some(XmlNodeId::new(5001))
        );
        assert_eq!(
            document.parent_id(XmlNodeId::new(5002)),
            Some(XmlNodeId::new(5000))
        );
        assert_eq!(document.allocate_node_id().raw(), 5003);
        let xml = document.to_string().unwrap();
        document.rebuild_index();
        assert_eq!(document.allocate_node_id().raw(), 5004);
        assert_eq!(document.to_string().unwrap(), xml);
    }

    #[test]
    fn a_rebuild_does_not_hand_out_the_ids_of_a_detached_subtree() {
        // Splitting a table detaches its later rows, rebuilds the index and
        // puts the rows back under a new table with their ids. The rebuild
        // lowered the next id to the live tree's maximum, so the new table's
        // properties took ids the detached rows still held.
        let mut document =
            DocxXmlDocument::parse("<root><tbl><tblPr/><tr><tc/></tr></tbl></root>").unwrap();
        let root = document.root().id;
        for _ in 0..3 {
            document.allocate_node_id();
        }
        let added = document.create_element("tr", Vec::new(), Vec::new());
        let table = document.root().children[0].clone();
        let XmlNode::Element(mut table) = table else {
            unreachable!();
        };
        table.children.push(XmlNode::Element(added));
        document.element_mut(root).unwrap().children = vec![XmlNode::Element(table)];
        document.rebuild_index();
        let detached = {
            let XmlNode::Element(table) = &mut document.element_mut(root).unwrap().children[0]
            else {
                unreachable!();
            };
            table.children.split_off(1)
        };
        document.rebuild_index();
        let copy = document.create_element("tblPr", Vec::new(), Vec::new());
        let mut held = Vec::new();
        for node in &detached {
            if let XmlNode::Element(element) = node {
                held.push(element.id);
                if let Some(max) = element.max_node_id() {
                    held.push(max);
                }
            }
        }
        assert!(
            !held.contains(&copy.id),
            "{:?} reused from {held:?}",
            copy.id
        );
        assert_index_matches_tree(&document);
    }

    #[test]
    fn rebuild_index_reassigns_non_root_placeholder_ids() {
        let xml = r#"<w:document xmlns:w="urn:w"><w:body><w:p/></w:body></w:document>"#;
        let mut document = DocxXmlDocument::parse(xml).expect("parse");
        let root_id = document.root().id;
        let body_id = document
            .root()
            .children
            .iter()
            .find_map(|node| match node {
                XmlNode::Element(element) if element.name == "w:body" => Some(element.id),
                _ => None,
            })
            .expect("body");
        document
            .element_mut(body_id)
            .expect("body")
            .children
            .push(XmlNode::Element(XmlElement {
                id: XmlNodeId::new(0),
                name: "w:sectPr".to_string(),
                attributes: Vec::new(),
                children: vec![XmlNode::Element(XmlElement {
                    id: XmlNodeId::new(0),
                    name: "w:pgSz".to_string(),
                    attributes: Vec::new(),
                    children: Vec::new(),
                })],
            }));

        document.rebuild_index();

        assert_eq!(document.root().id, root_id);
        assert_eq!(
            document
                .element(root_id)
                .map(|element| element.name.as_str()),
            Some("w:document")
        );
        let body = document.element(body_id).expect("body remains indexed");
        let sect_pr = body
            .children
            .iter()
            .find_map(|node| match node {
                XmlNode::Element(element) if element.name == "w:sectPr" => Some(element),
                _ => None,
            })
            .expect("section properties");
        assert_ne!(sect_pr.id, root_id);
        let pg_sz = sect_pr
            .children
            .iter()
            .find_map(|node| match node {
                XmlNode::Element(element) => Some(element),
                _ => None,
            })
            .expect("page size");
        assert_ne!(pg_sz.id, root_id);
        assert_ne!(pg_sz.id, sect_pr.id);
    }
}
