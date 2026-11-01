//! Incremental OOXML package capture, stable native bindings and CRDT projection.
//! Browser scheduling lives in `worker/capture.ts`. Domain semantics are injected
//! through [`FactPolicy`]; this crate does not interpret legal/effective dates.
pub mod nodes;

use jade_crdt::{Document, StructuredNode};
use jade_docx_reader::docx_xml::{DocxXmlDocument, XmlElement, XmlNode};
use std::collections::{BTreeMap, HashMap};

/// Derive opaque application facts from the changed scopes and existing facts.
/// Must be deterministic and side-effect free. Scope slices may contain only
/// dirty paragraphs/structural parents, rather than the complete package.
pub type FactPolicy = fn(
    &[StructuredNode],
    &[StructuredNode],
    &[jade_crdt::Fact],
    &[serde_json::Value],
) -> Result<Vec<serde_json::Value>, String>;
fn no_facts(
    _: &[StructuredNode],
    _: &[StructuredNode],
    _: &[jade_crdt::Fact],
    _: &[serde_json::Value],
) -> Result<Vec<serde_json::Value>, String> {
    Ok(vec![])
}

fn fresh() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn semantic(attributes: &BTreeMap<String, String>) -> Option<(&str, &str)> {
    [
        "w:id",
        "Id",
        "w:styleId",
        "w:abstractNumId",
        "w:numId",
        "name",
    ]
    .iter()
    .find_map(|name| attributes.get(*name).map(|v| (*name, v.as_str())))
}
fn capture_xml(
    root: &XmlElement,
    previous: Option<&StructuredNode>,
    binding: &mut HashMap<u32, String>,
    native: bool,
) -> Result<StructuredNode, String> {
    fn index<'a>(n: &'a StructuredNode, out: &mut HashMap<&'a str, &'a StructuredNode>) {
        out.insert(&n.id, n);
        for c in &n.children {
            index(c, out)
        }
    }
    let mut old = HashMap::new();
    if let Some(p) = previous {
        index(p, &mut old);
    }
    let rebind = binding.is_empty();
    fn visit(
        e: &XmlElement,
        prior: Option<&StructuredNode>,
        old: &HashMap<&str, &StructuredNode>,
        binding: &mut HashMap<u32, String>,
        native: bool,
        rebind: bool,
    ) -> Result<StructuredNode, String> {
        let bound = if native {
            binding.get(&e.id.raw()).cloned()
        } else {
            None
        };
        let prior = bound
            .as_ref()
            .and_then(|id| old.get(id.as_str()).copied())
            .or(prior.filter(|_| !native || rebind));
        let id = bound
            .or_else(|| prior.map(|p| p.id.clone()))
            .map(Ok)
            .unwrap_or_else(fresh)?;
        if native {
            binding.insert(e.id.raw(), id.clone());
        }
        let attributes: BTreeMap<_, _> = e
            .attributes
            .iter()
            .map(|a| (a.name.clone(), a.value.clone()))
            .collect();
        let mut children = vec![];
        let mut used = std::collections::HashSet::new();
        let candidates = prior.map(|p| p.children.as_slice()).unwrap_or(&[]);
        for (i, child) in e.children.iter().enumerate() {
            let (kind, attrs, text) = match child {
                XmlNode::Element(c) => (
                    c.name.as_str(),
                    c.attributes
                        .iter()
                        .map(|a| (a.name.clone(), a.value.clone()))
                        .collect(),
                    None,
                ),
                XmlNode::Text(t) => ("#text", BTreeMap::new(), Some(t)),
                XmlNode::CData(t) => ("#cdata", BTreeMap::new(), Some(t)),
                XmlNode::Comment(t) => ("#comment", BTreeMap::new(), Some(t)),
            };
            let compatible = |p: &&StructuredNode| {
                !used.contains(&p.id)
                    && p.kind == kind
                    && semantic(&p.attributes) == semantic(&attrs)
            };
            let matched = if semantic(&attrs).is_some() {
                candidates.iter().find(compatible)
            } else {
                candidates
                    .get(i)
                    .filter(|p| compatible(p))
                    .or_else(|| candidates.iter().find(compatible))
            };
            if let Some(p) = matched {
                used.insert(p.id.clone());
            }
            children.push(match child {
                XmlNode::Element(c) => visit(c, matched, old, binding, native, rebind)?,
                _ => StructuredNode {
                    id: matched
                        .map(|p| p.id.clone())
                        .map(Ok)
                        .unwrap_or_else(fresh)?,
                    kind: kind.into(),
                    text: text.unwrap().clone(),
                    attributes: attrs,
                    children: vec![],
                    deleted: false,
                },
            });
        }
        Ok(StructuredNode {
            id,
            kind: e.name.clone(),
            text: String::new(),
            attributes,
            children,
            deleted: false,
        })
    }
    visit(root, previous, &old, binding, native, rebind)
}
fn escape(value: &str, attribute: bool) -> String {
    let mut out = String::new();
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attribute => out.push_str("&quot;"),
            '\r' => out.push_str("&#13;"),
            '\n' if attribute => out.push_str("&#10;"),
            '\t' if attribute => out.push_str("&#9;"),
            _ => out.push(c),
        }
    }
    out
}
fn render_xml(n: &StructuredNode) -> Result<String, String> {
    match n.kind.as_str() {
        "#text" => return Ok(escape(&n.text, false)),
        "#cdata" => {
            return Ok(format!(
                "<![CDATA[{}]]>",
                n.text.replace("]]>", "]]]]><![CDATA[>")
            ))
        }
        "#comment" => return Ok(format!("<!--{}-->", n.text)),
        _ => (),
    }
    let mut out = format!("<{}", n.kind);
    for (name, value) in &n.attributes {
        out.push_str(&format!(" {}=\"{}\"", name, escape(value, true)));
    }
    if n.children.is_empty() {
        out.push_str("/>");
        return Ok(out);
    }
    out.push('>');
    for c in &n.children {
        out.push_str(&render_xml(c)?)
    }
    out.push_str(&format!("</{}>", n.kind));
    Ok(out)
}

/// The complete DOCX package is projected from this single CRDT owner.
/// Binary/opaque parts are indivisible registers; XML is granular ordered nodes.
pub struct LiveDocument {
    pub inner: Document,
    fact_policy: FactPolicy,
    document: String,
    baseline: String,
    forest: Vec<StructuredNode>,
    binding: HashMap<u32, String>,
    paths: HashMap<String, Vec<usize>>,
    full_capture: bool,
}
pub struct LivePosition {
    paragraph: String,
    offset: usize,
    // First the containing text, then the nearest surviving boundaries.
    candidates: Vec<(String, Vec<u8>)>,
}
fn find_node<'a>(nodes: &'a [StructuredNode], id: &str) -> Option<&'a StructuredNode> {
    for n in nodes {
        if n.id == id {
            return Some(n);
        }
        if let Some(n) = find_node(&n.children, id) {
            return Some(n);
        }
    }
    None
}
fn forest_paths(forest: &[StructuredNode]) -> HashMap<String, Vec<usize>> {
    fn visit(nodes: &[StructuredNode], prefix: &[usize], out: &mut HashMap<String, Vec<usize>>) {
        for (i, n) in nodes.iter().enumerate() {
            let mut path = prefix.to_vec();
            path.push(i);
            out.insert(n.id.clone(), path.clone());
            visit(&n.children, &path, out);
        }
    }
    let mut out = HashMap::new();
    visit(forest, &[], &mut out);
    out
}
fn at_path<'a>(nodes: &'a [StructuredNode], path: &[usize]) -> &'a StructuredNode {
    let n = &nodes[path[0]];
    if path.len() == 1 {
        n
    } else {
        at_path(&n.children, &path[1..])
    }
}
fn at_path_mut<'a>(nodes: &'a mut [StructuredNode], path: &[usize]) -> &'a mut StructuredNode {
    let n = &mut nodes[path[0]];
    if path.len() == 1 {
        n
    } else {
        at_path_mut(&mut n.children, &path[1..])
    }
}
fn position_chunks<'a>(node: &'a StructuredNode, out: &mut Vec<(&'a StructuredNode, usize)>) {
    if matches!(
        node.kind.as_str(),
        "w:pPr" | "w:rPr" | "w:drawing" | "w:pict"
    ) {
        return;
    }
    if matches!(node.kind.as_str(), "w:t" | "w:delText") {
        for c in &node.children {
            if c.kind == "#text" {
                out.push((c, c.text.len()));
            }
        }
        return;
    }
    if matches!(node.kind.as_str(), "w:tab" | "w:br" | "w:cr") {
        out.push((node, 1));
        return;
    }
    for c in &node.children {
        position_chunks(c, out)
    }
}
fn capture_package(
    bytes: &[u8],
    main: Option<&DocxXmlDocument>,
    previous: &[StructuredNode],
    binding: &mut HashMap<u32, String>,
) -> Result<Vec<StructuredNode>, String> {
    use base64::Engine;
    use std::io::Read;
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut parts = BTreeMap::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        if file.is_dir() {
            continue;
        }
        let path = file.name().map_err(|e| e.to_string())?.into_owned();
        if parts.contains_key(&path) {
            return Err("duplicate package path".into());
        }
        let mut data = vec![];
        file.read_to_end(&mut data).map_err(|e| e.to_string())?;
        parts.insert(path, data);
    }
    if !parts.contains_key("word/document.xml") {
        return Err("main document part is required".into());
    }
    let mut forest = vec![];
    for (path, data) in parts {
        let old = previous
            .iter()
            .find(|p| p.attributes.get("path") == Some(&path));
        let mut attributes = BTreeMap::from([("path".into(), path.clone())]);
        let xml = if path.ends_with(".xml") || path.ends_with(".rels") {
            std::str::from_utf8(&data)
                .ok()
                .and_then(|s| DocxXmlDocument::parse(s).ok())
        } else {
            None
        };
        let children = if let Some(xml) = xml {
            attributes.insert("format".into(), "xml".into());
            let native = path == "word/document.xml" && main.is_some();
            let root = if native {
                main.unwrap().root()
            } else {
                xml.root()
            };
            vec![capture_xml(
                root,
                old.and_then(|p| p.children.first()),
                binding,
                native,
            )?]
        } else {
            attributes.insert("format".into(), "opaque".into());
            attributes.insert(
                "bytes".into(),
                base64::engine::general_purpose::STANDARD.encode(&data),
            );
            vec![]
        };
        forest.push(StructuredNode {
            id: old.map(|p| p.id.clone()).map(Ok).unwrap_or_else(fresh)?,
            kind: "package-part".into(),
            text: String::new(),
            attributes,
            children,
            deleted: false,
        });
    }
    Ok(forest)
}
pub fn render_package(forest: &[StructuredNode]) -> Result<Vec<u8>, String> {
    use base64::Engine;
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let mut paths = std::collections::HashSet::new();
    for part in forest {
        let path = part.attributes.get("path").ok_or("missing package path")?;
        if part.kind != "package-part"
            || path.starts_with('/')
            || path.split('/').any(|p| p == "..")
            || !paths.insert(path)
        {
            return Err("invalid or duplicate package path".into());
        }
        let bytes = match part.attributes.get("format").map(String::as_str) {
            Some("xml") if part.children.len() == 1 => {
                let value = render_xml(&part.children[0])?;
                DocxXmlDocument::parse(&value)
                    .map_err(|e| format!("invalid XML projection {path}: {e}"))?;
                value.into_bytes()
            }
            Some("opaque") if part.children.is_empty() => base64::engine::general_purpose::STANDARD
                .decode(part.attributes.get("bytes").ok_or("missing opaque part")?)
                .map_err(|e| e.to_string())?,
            _ => return Err("invalid package part projection".into()),
        };
        zip.start_file(
            path,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated),
        )
        .map_err(|e| e.to_string())?;
        zip.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    if !paths.contains(&"word/document.xml".to_string()) {
        return Err("main document part is required".into());
    }
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}
impl LiveDocument {
    pub fn create(
        document: &str,
        baseline: &str,
        bytes: &[u8],
        main: Option<&DocxXmlDocument>,
    ) -> Result<Self, String> {
        let mut binding = HashMap::new();
        let forest = capture_package(bytes, main, &[], &mut binding)?;
        Ok(Self {
            inner: Document::create_structured(document, baseline, forest.clone())?,
            document: document.into(),
            baseline: baseline.into(),
            paths: forest_paths(&forest),
            forest,
            binding,
            full_capture: false,
            fact_policy: no_facts,
        })
    }
    pub fn open(
        document: &str,
        baseline: &str,
        bytes: &[u8],
        snapshot: &[u8],
        main: Option<&DocxXmlDocument>,
    ) -> Result<Self, String> {
        let inner = Document::open(document, baseline, snapshot)?;
        let forest = inner.visible_structure()?;
        let mut binding = HashMap::new();
        if capture_package(bytes, main, &forest, &mut binding)? != forest {
            return Err("DOCX and CRDT snapshot do not match".into());
        }
        Ok(Self {
            inner,
            document: document.into(),
            baseline: baseline.into(),
            paths: forest_paths(&forest),
            forest,
            binding,
            full_capture: false,
            fact_policy: no_facts,
        })
    }
    pub fn capture(
        &mut self,
        actor: &str,
        bytes: &[u8],
        main: Option<&DocxXmlDocument>,
        values: Vec<serde_json::Value>,
    ) -> Result<Vec<jade_crdt::Fact>, String> {
        let mut binding = self.binding.clone();
        let next = capture_package(bytes, main, &self.forest, &mut binding)?;
        let mut derived = self.derived_values(&self.forest, &next, &values)?;
        derived.extend(values);
        let facts = self
            .inner
            .reconcile(actor, &self.forest, next.clone(), derived)?;
        self.paths = forest_paths(&next);
        self.forest = next;
        self.binding = binding;
        self.full_capture = false;
        Ok(facts)
    }
    /// No ZIP serialization, package parsing or traversal outside dirty scopes.
    /// False requests the full path for unsupported commands or invalid bindings.
    pub fn capture_changed(&mut self, actor: &str, main: &DocxXmlDocument) -> Result<bool, String> {
        if self.full_capture || self.binding.is_empty() {
            return Ok(false);
        }
        let Some(changed) = main.changed_elements() else {
            return Ok(false);
        };
        let mut roots = vec![];
        for mut id in changed {
            // Capture a complete paragraph for revision semantics. Structural
            // changes already name their parent (body/table/etc.). New elements
            // climb to an existing bound ancestor, never an invented scope root.
            loop {
                let element = main.element(id).ok_or("dirty XML element disappeared")?;
                if self.binding.contains_key(&id.raw())
                    && (matches!(
                        element.name.as_str(),
                        "w:p" | "w:body" | "w:tbl" | "w:tr" | "w:tc"
                    ) || main.parent_id(id).is_none())
                {
                    break;
                }
                let Some(parent) = main.parent_id(id) else {
                    return Ok(false);
                };
                id = parent;
            }
            let Some(path) = self
                .binding
                .get(&id.raw())
                .and_then(|key| self.paths.get(key))
            else {
                return Ok(false);
            };
            roots.push((path.clone(), id));
        }
        roots.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then(a.0.cmp(&b.0)));
        let mut scopes = vec![];
        let mut accepted: Vec<Vec<usize>> = vec![];
        let mut bindings = HashMap::new();
        for (path, id) in roots {
            if accepted.iter().any(|p| path.starts_with(p)) {
                continue;
            }
            let previous = at_path(&self.forest, &path);
            fn bind(e: &XmlElement, old: &HashMap<u32, String>, local: &mut HashMap<u32, String>) {
                if let Some(key) = old.get(&e.id.raw()) {
                    local.insert(e.id.raw(), key.clone());
                }
                for c in &e.children {
                    if let XmlNode::Element(e) = c {
                        bind(e, old, local)
                    }
                }
            }
            let root = main.element(id).ok_or("missing dirty scope")?;
            let mut local = HashMap::new();
            bind(root, &self.binding, &mut local);
            let next = capture_xml(root, Some(previous), &mut local, true)?;
            scopes.push((previous.clone(), next));
            bindings.extend(local);
            accepted.push(path);
        }
        let previous: Vec<_> = scopes.iter().map(|(p, _)| p.clone()).collect();
        let next: Vec<_> = scopes.iter().map(|(_, n)| n.clone()).collect();
        let values = self.derived_values(&previous, &next, &[])?;
        self.inner.reconcile_subtrees(actor, &scopes, values)?;
        fn remove(n: &StructuredNode, paths: &mut HashMap<String, Vec<usize>>) {
            paths.remove(&n.id);
            for c in &n.children {
                remove(c, paths)
            }
        }
        fn add(n: &StructuredNode, path: Vec<usize>, paths: &mut HashMap<String, Vec<usize>>) {
            paths.insert(n.id.clone(), path.clone());
            for (i, c) in n.children.iter().enumerate() {
                let mut p = path.clone();
                p.push(i);
                add(c, p, paths)
            }
        }
        for (path, (previous, next)) in accepted.into_iter().zip(scopes) {
            remove(&previous, &mut self.paths);
            add(&next, path.clone(), &mut self.paths);
            *at_path_mut(&mut self.forest, &path) = next;
        }
        self.binding.extend(bindings);
        Ok(true)
    }
    fn derived_values(
        &self,
        previous: &[StructuredNode],
        next: &[StructuredNode],
        extra: &[serde_json::Value],
    ) -> Result<Vec<serde_json::Value>, String> {
        (self.fact_policy)(previous, next, &self.inner.facts()?, extra)
    }
    /// Install host-specific metadata derivation. It runs before capture commits,
    /// so returned facts and XML changes share the same transaction/undo unit.
    pub fn with_fact_policy(mut self, policy: FactPolicy) -> Self {
        self.fact_policy = policy;
        self
    }
    pub fn document(&self) -> &str {
        &self.document
    }
    pub fn baseline(&self) -> &str {
        &self.baseline
    }
    pub fn append_values(
        &mut self,
        actor: &str,
        values: Vec<serde_json::Value>,
    ) -> Result<Vec<jade_crdt::Fact>, String> {
        let mut derived = self.derived_values(&self.forest, &self.forest, &values)?;
        derived.extend(values);
        self.inner.transact(actor, vec![], derived)
    }
    pub fn history(&mut self, actor: &str, redo: bool) -> Result<bool, String> {
        let previous = self.inner.visible_structure()?;
        let changed = if redo {
            self.inner.redo()?
        } else {
            self.inner.undo()?
        };
        let next = self.inner.visible_structure()?;
        let values = self.derived_values(&previous, &next, &[])?;
        self.inner.transact(actor, vec![], values)?;
        self.forest = next;
        self.reset_binding();
        Ok(changed)
    }
    pub fn native_id(&self, raw: u32) -> Option<&str> {
        self.binding.get(&raw).map(String::as_str)
    }
    pub fn position_anchor(&self, raw: u32, offset: usize) -> Result<LivePosition, String> {
        let paragraph = self
            .native_id(raw)
            .ok_or("unbound native paragraph")?
            .to_string();
        let node = find_node(&self.forest, &paragraph).ok_or("missing paragraph")?;
        let mut chunks = vec![];
        position_chunks(node, &mut chunks);
        let mut start = 0usize;
        let mut candidates = vec![];
        for (node, len) in chunks {
            if node.kind == "#text" {
                let at = offset.saturating_sub(start).min(len);
                let distance = if offset < start {
                    start - offset
                } else {
                    offset.saturating_sub(start + len)
                };
                if node.text.is_char_boundary(at) {
                    candidates.push((
                        distance,
                        node.id.clone(),
                        self.inner
                            .anchor(&node.id, at, jade_crdt::Affinity::Before)?,
                    ));
                }
            }
            start += len;
        }
        candidates.sort_by_key(|c| c.0);
        Ok(LivePosition {
            paragraph,
            offset,
            candidates: candidates.into_iter().map(|(_, id, a)| (id, a)).collect(),
        })
    }
    pub fn resolve_position(&self, position: &LivePosition) -> Result<(String, usize), String> {
        fn locate(nodes: &[StructuredNode], id: &str) -> Option<(String, usize)> {
            for node in nodes {
                if node.kind == "w:p" {
                    let mut chunks = vec![];
                    position_chunks(node, &mut chunks);
                    let mut start = 0;
                    for (n, len) in chunks {
                        if n.id == id {
                            return Some((node.id.clone(), start));
                        }
                        start += len;
                    }
                } else if let Some(found) = locate(&node.children, id) {
                    return Some(found);
                }
            }
            None
        }
        for (id, anchor) in &position.candidates {
            if let Some((paragraph, start)) = locate(&self.forest, id) {
                return Ok((paragraph, start + self.inner.resolve_anchor(anchor)?));
            }
        }
        let node =
            find_node(&self.forest, &position.paragraph).ok_or("selected paragraph was deleted")?;
        let mut chunks = vec![];
        position_chunks(node, &mut chunks);
        Ok((
            position.paragraph.clone(),
            position.offset.min(chunks.iter().map(|(_, len)| len).sum()),
        ))
    }
    /// Package replacement invalidates native XmlNodeIds. Rebind against stable IDs.
    pub fn reset_binding(&mut self) {
        self.binding.clear();
        self.paths = forest_paths(&self.forest);
    }
    pub fn require_full_capture(&mut self) {
        self.full_capture = true;
    }
    pub fn projected_docx(&self) -> Result<Vec<u8>, String> {
        render_package(&self.inner.visible_structure()?)
    }
    pub fn preview_import(&self, updates: &[u8]) -> Result<Vec<u8>, String> {
        let mut candidate =
            Document::open(&self.document, &self.baseline, &self.inner.snapshot()?)?;
        candidate.import_updates(updates)?;
        render_package(&candidate.visible_structure()?)
    }
    /// Import facts/frontiers without rebuilding the native document when its
    /// visible package is unchanged. Preserve native IDs and the live caret.
    pub fn import_metadata(&mut self, updates: &[u8]) -> Result<bool, String> {
        let mut candidate =
            Document::open(&self.document, &self.baseline, &self.inner.snapshot()?)?;
        candidate.import_updates(updates)?;
        if candidate.visible_structure()? != self.forest {
            return Ok(false);
        }
        self.inner.import_updates(updates)?;
        Ok(true)
    }
    pub fn accept_import(&mut self, updates: &[u8]) -> Result<(), String> {
        self.inner.import_updates(updates)?;
        self.forest = self.inner.visible_structure()?;
        self.reset_binding();
        Ok(())
    }
    pub fn rebind(&mut self, bytes: &[u8], main: Option<&DocxXmlDocument>) -> Result<(), String> {
        self.forest = self.inner.visible_structure()?;
        self.reset_binding();
        let mut binding = HashMap::new();
        if capture_package(bytes, main, &self.forest, &mut binding)? != self.forest {
            return Err("native projection differs from CRDT".into());
        }
        self.binding = binding;
        self.full_capture = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jade_docx_reader::docx_xml::XmlNodeId;
    fn xml(value: &str) -> DocxXmlDocument {
        DocxXmlDocument::parse(value).unwrap()
    }
    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(64))]
        #[test]
        fn incremental_capture_matches_full_xml_and_undo(
            edits in proptest::collection::vec((0usize..4, proptest::collection::vec(proptest::sample::select(vec!['a','漢','😀','\u{301}','&','<']),0..12), proptest::bool::ANY),1..18)
        ) {
            let source="<w:document><w:body><w:p><w:r><w:t>A</w:t></w:r></w:p><w:p><w:r><w:t>A</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>A</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>A</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>";
            let mut main=xml(source);let bytes=package(source,&[0,255,7]);
            let mut live=LiveDocument::create("work","base",&bytes,Some(&main)).unwrap();
            fn texts(e:&XmlElement,out:&mut Vec<XmlNodeId>){if e.name=="w:t" {out.push(e.id)}for c in &e.children {if let XmlNode::Element(e)=c {texts(e,out)}}}
            let mut ids=vec![];texts(main.root(),&mut ids);
            for (index,chars,revision) in edits {
                let before=live.inner.visible_structure().unwrap();
                main.track_changes();
                let e=main.element_mut(ids[index]).unwrap();
                e.children=vec![XmlNode::Text(chars.into_iter().collect())];
                e.attributes.retain(|a|a.name!="data-revision");
                if revision {e.attributes.push(jade_docx_reader::docx_xml::XmlAttribute {name:"data-revision".into(),value:"insert".into()});}
                proptest::prop_assert!(live.capture_changed("a",&main).unwrap());
                let mut bindings=live.binding.clone();
                let full=capture_package(&bytes,Some(&main),&live.forest,&mut bindings).unwrap();
                proptest::prop_assert_eq!(&live.forest,&full);
                let after=live.inner.visible_structure().unwrap();
                if before!=after {
                    proptest::prop_assert!(live.inner.undo().unwrap());
                    proptest::prop_assert_eq!(live.inner.visible_structure().unwrap(),before);
                    proptest::prop_assert!(live.inner.redo().unwrap());
                    proptest::prop_assert_eq!(live.inner.visible_structure().unwrap(),after);
                }
                let output=live.projected_docx().unwrap();
                LiveDocument::open("work","base",&output,&live.inner.snapshot().unwrap(),Some(&main)).unwrap();
            }
        }
    }
    #[test]
    fn incremental_structural_scopes_retry_match_full_capture() {
        let source="<w:document><w:body><w:p><w:r><w:t>A</w:t></w:r><w:ins w:author=\"Tester#S00\" w:id=\"7\"><w:r><w:t>B</w:t></w:r></w:ins></w:p><w:p><w:r><w:t>A</w:t></w:r></w:p></w:body></w:document>";
        let mut main = xml(source);
        let bytes = package(source, &[0, 255, 7]);
        let mut live = LiveDocument::create("work", "base", &bytes, Some(&main)).unwrap();
        let body = match &main.root().children[0] {
            XmlNode::Element(e) => e.id,
            _ => unreachable!(),
        };
        let paragraph = match &main.element(body).unwrap().children[0] {
            XmlNode::Element(e) => e.id,
            _ => unreachable!(),
        };
        // Insert, remove and reorder indistinguishable paragraphs. Capture the
        // structural parent so IDs, sibling order and deleted-node facts agree.
        for step in 0..4 {
            main.track_changes();
            if step == 0 {
                let node = main.element(paragraph).unwrap().clone();
                main.insert_node(body, 1, XmlNode::Element(node)).unwrap();
            } else if step == 1 {
                let path = main.path(paragraph).unwrap().clone();
                main.remove_node(&path).unwrap();
            } else if step == 2 {
                let path = main.path(body).unwrap().with_child(0);
                let node = main.remove_node(&path).unwrap();
                main.insert_node_preserving_ids(body, 1, node).unwrap();
            } else {
                // Attribute-only edits retain the same structural scope.
                let e = main.element_mut(body).unwrap();
                e.attributes.push(jade_docx_reader::docx_xml::XmlAttribute {
                    name: "test".into(),
                    value: "retry".into(),
                });
            }
            assert!(live.capture_changed("a", &main).unwrap());
            let mut binding = live.binding.clone();
            let full = capture_package(&bytes, Some(&main), &live.forest, &mut binding).unwrap();
            assert_eq!(live.forest, full);
            let output = live.projected_docx().unwrap();
            LiveDocument::open(
                "work",
                "base",
                &output,
                &live.inner.snapshot().unwrap(),
                Some(&main),
            )
            .unwrap();
        }
        // Stale captures are atomic and can retry after the expected tree is restored.
        main.track_changes();
        main.element_mut(body)
            .unwrap()
            .attributes
            .push(jade_docx_reader::docx_xml::XmlAttribute {
                name: "retry".into(),
                value: "yes".into(),
            });
        let id = live.native_id(body.raw()).unwrap().to_string();
        live.inner
            .transact_structured(
                "external",
                vec![jade_crdt::NodeOp::SetAttribute {
                    id,
                    name: "outside".into(),
                    value: Some("conflict".into()),
                }],
                vec![],
                vec![],
            )
            .unwrap();
        let frontier = live.inner.version().unwrap();
        let dirty = main.changed_elements().unwrap();
        assert!(live.capture_changed("a", &main).is_err());
        assert_eq!(live.inner.version().unwrap(), frontier);
        assert_eq!(main.changed_elements().unwrap(), dirty);
        live.inner.undo().unwrap();
        assert!(live.capture_changed("a", &main).unwrap());
    }
    #[test]
    fn live_native_ids_survive_text_formatting_revision_and_table_edits() {
        let mut before = xml("<w:document xmlns:w=\"urn:test\"><w:body><w:p><w:r><w:t>使用料😀</w:t></w:r></w:p></w:body></w:document>");
        let mut binding = HashMap::new();
        let first = capture_xml(before.root(), None, &mut binding, true).unwrap();
        let old_id = first.children[0].children[0].children[0].children[0]
            .id
            .clone();
        fn edit(e: &mut XmlElement) {
            for c in &mut e.children {
                match c {
                    XmlNode::Element(e) => edit(e),
                    XmlNode::Text(t) => *t = "利用料金😀".into(),
                    _ => (),
                }
            }
        }
        let root_id = before.root().id;
        edit(before.element_mut(root_id).unwrap());
        let next = capture_xml(before.root(), Some(&first), &mut binding, true).unwrap();
        assert_eq!(
            next.children[0].children[0].children[0].children[0].id,
            old_id
        );
        let mut d = Document::create_structured("work", "base", vec![first.clone()]).unwrap();
        d.reconcile(
            "alice",
            std::slice::from_ref(&first),
            vec![next.clone()],
            vec![],
        )
        .unwrap();
        assert_eq!(render_xml(&next).unwrap(), "<w:document xmlns:w=\"urn:test\"><w:body><w:p><w:r><w:t>利用料金😀</w:t></w:r></w:p></w:body></w:document>");
        d.undo().unwrap();
        assert_eq!(d.visible_structure().unwrap(), vec![first]);
    }
    #[test]
    fn live_reparsed_parts_match_semantic_ids_and_escape_losslessly() {
        let old=xml("<w:comments xmlns:w=\"urn:test\"><w:comment w:id=\"1\"><w:p><w:r><w:t>A &amp; B</w:t></w:r></w:p></w:comment></w:comments>");
        let new=xml("<w:comments xmlns:w=\"urn:test\"><w:comment w:id=\"2\"/><w:comment w:id=\"1\"><w:p><w:r><w:t>A &amp; C</w:t></w:r></w:p></w:comment></w:comments>");
        let mut ids = HashMap::new();
        let a = capture_xml(old.root(), None, &mut ids, false).unwrap();
        let b = capture_xml(new.root(), Some(&a), &mut ids, false).unwrap();
        assert_eq!(a.children[0].id, b.children[1].id);
        assert_ne!(a.children[0].id, b.children[0].id);
        let output = render_xml(&b).unwrap();
        assert!(output.contains("A &amp; C"));
        assert!(DocxXmlDocument::parse(&output).is_ok());
    }
    fn package(text: &str, binary: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (path, bytes) in [
            ("[Content_Types].xml",br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.as_slice()),
            ("_rels/.rels",br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.as_slice()),
            ("word/document.xml",text.as_bytes()),("word/media/image.bin",binary)] {
            zip.start_file(path,zip::write::SimpleFileOptions::default()).unwrap();zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }
    #[test]
    fn live_package_snapshot_reopen_and_selective_undo() {
        let bytes = package(
            "<w:document><w:body><w:p><w:r><w:t>料金</w:t></w:r></w:p></w:body></w:document>",
            &[0, 255, 7],
        );
        let mut a = LiveDocument::create("work", "base", &bytes, None).unwrap();
        let mut b =
            LiveDocument::open("work", "base", &bytes, &a.inner.snapshot().unwrap(), None).unwrap();
        let version = a.inner.version().unwrap();
        let local = package(
            "<w:document><w:body><w:p><w:r><w:t>利用料金</w:t></w:r></w:p></w:body></w:document>",
            &[0, 255, 7],
        );
        a.capture("alice", &local, None, vec![]).unwrap();
        let remote = package(
            "<w:document><w:body><w:p><w:r><w:t>料金です</w:t></w:r></w:p></w:body></w:document>",
            &[0, 255, 7],
        );
        b.capture("bob", &remote, None, vec![]).unwrap();
        a.inner
            .import_updates(&b.inner.updates_since(&version).unwrap())
            .unwrap();
        a.inner.undo().unwrap();
        let exported = render_package(&a.inner.visible_structure().unwrap()).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&exported)).unwrap();
        use std::io::Read;
        let mut text = String::new();
        zip.by_name("word/document.xml")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert!(text.contains("料金です"), "{text}");
        assert!(!text.contains("利用"));
        let mut binary = vec![];
        zip.by_name("word/media/image.bin")
            .unwrap()
            .read_to_end(&mut binary)
            .unwrap();
        assert_eq!(binary, [0, 255, 7]);
        LiveDocument::open(
            "work",
            "base",
            &exported,
            &a.inner.snapshot().unwrap(),
            None,
        )
        .unwrap();
        assert!(
            LiveDocument::open("work", "base", &local, &a.inner.snapshot().unwrap(), None).is_err()
        );
    }
    #[test]
    fn live_position_follows_remote_unicode_edit_and_surviving_neighbor() {
        let source="<w:document><w:body><w:p><w:r><w:t>料金😀</w:t></w:r><w:r><w:t>末尾</w:t></w:r></w:p></w:body></w:document>";
        let main = xml(source);
        let bytes = package(source, &[]);
        let mut local = LiveDocument::create("work", "base", &bytes, Some(&main)).unwrap();
        let p = main
            .root()
            .children
            .iter()
            .find_map(|n| {
                if let XmlNode::Element(e) = n {
                    Some(e)
                } else {
                    None
                }
            })
            .unwrap();
        let p = p
            .children
            .iter()
            .find_map(|n| {
                if let XmlNode::Element(e) = n {
                    Some(e)
                } else {
                    None
                }
            })
            .unwrap();
        let position = local.position_anchor(p.id.raw(), "料金".len()).unwrap();
        let mut remote = LiveDocument::open(
            "work",
            "base",
            &bytes,
            &local.inner.snapshot().unwrap(),
            None,
        )
        .unwrap();
        let changed = source.replace("料金😀", "利用料金😀");
        remote
            .capture("remote", &package(&changed, &[]), None, vec![])
            .unwrap();
        local
            .accept_import(&remote.inner.snapshot().unwrap())
            .unwrap();
        assert_eq!(
            local.resolve_position(&position).unwrap().1,
            "利用料金".len()
        );
        let mut next = remote.forest.clone();
        next.iter_mut()
            .find(|n| n.attributes.get("path").map(String::as_str) == Some("word/document.xml"))
            .unwrap()
            .children[0]
            .children[0]
            .children[0]
            .children
            .remove(0);
        remote
            .inner
            .reconcile("remote", &remote.forest, next, vec![])
            .unwrap();
        local
            .accept_import(&remote.inner.snapshot().unwrap())
            .unwrap();
        assert_eq!(local.resolve_position(&position).unwrap().1, 0);
    }
    #[test]
    fn live_metadata_import_preserves_native_bindings() {
        let bytes = package(
            "<w:document><w:body><w:p><w:r><w:t>料金</w:t></w:r></w:p></w:body></w:document>",
            &[],
        );
        let main =
            xml("<w:document><w:body><w:p><w:r><w:t>料金</w:t></w:r></w:p></w:body></w:document>");
        let mut local = LiveDocument::create("work", "base", &bytes, Some(&main)).unwrap();
        let mut remote = LiveDocument::open(
            "work",
            "base",
            &bytes,
            &local.inner.snapshot().unwrap(),
            None,
        )
        .unwrap();
        remote
            .append_values(
                "server",
                vec![serde_json::json!({"kind":"metadata","label":"changed"})],
            )
            .unwrap();
        let binding = local.binding.clone();
        assert!(local
            .import_metadata(&remote.inner.snapshot().unwrap())
            .unwrap());
        assert_eq!(local.binding, binding);
        assert_eq!(local.inner.facts().unwrap().len(), 1);
        let changed = package(
            "<w:document><w:body><w:p><w:r><w:t>利用料金</w:t></w:r></w:p></w:body></w:document>",
            &[],
        );
        remote.capture("remote", &changed, None, vec![]).unwrap();
        let version = local.inner.version().unwrap();
        assert!(!local
            .import_metadata(&remote.inner.snapshot().unwrap())
            .unwrap());
        assert_eq!(local.inner.version().unwrap(), version);
    }
    #[test]
    fn live_rebind_keeps_ids_after_remote_reload() {
        let old =
            xml("<w:document><w:body><w:p><w:r><w:t>旧</w:t></w:r></w:p></w:body></w:document>");
        let mut ids = HashMap::new();
        let a = capture_xml(old.root(), None, &mut ids, true).unwrap();
        ids.clear();
        let reloaded = xml(&render_xml(&a).unwrap());
        let b = capture_xml(reloaded.root(), Some(&a), &mut ids, true).unwrap();
        assert_eq!(a, b);
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;
    use serde_json::{json, Value};
    fn package(text: &str) -> Vec<u8> {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        write!(
            zip,
            "<w:document><w:body><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:body></w:document>"
        )
        .unwrap();
        zip.finish().unwrap().into_inner()
    }
    fn policy(
        previous: &[StructuredNode],
        next: &[StructuredNode],
        _: &[jade_crdt::Fact],
        extra: &[Value],
    ) -> Result<Vec<Value>, String> {
        if extra.iter().any(|v| v["reject"] == true) {
            return Err("host rejected metadata".into());
        }
        if previous == next {
            return Ok(vec![]);
        }
        Ok(vec![
            json!({"kind":"host-change", "scopes": next.iter().map(|n| &n.kind).collect::<Vec<_>>()}),
        ])
    }
    #[test]
    fn facts_share_capture_transaction_and_are_retained_across_history_and_reopen() {
        let before = package("before");
        let mut live = LiveDocument::create("d", "b", &before, None)
            .unwrap()
            .with_fact_policy(policy);
        let facts = live
            .capture(
                "a",
                &package("after"),
                None,
                vec![json!({"kind":"host-explicit"})],
            )
            .unwrap();
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].actor, facts[1].actor);
        assert!(live.history("a", false).unwrap());
        assert!(
            String::from_utf8(extract_main(&live.projected_docx().unwrap()))
                .unwrap()
                .contains("before")
        );
        assert!(live.history("a", true).unwrap());
        assert_eq!(live.inner.facts().unwrap().len(), 4); // Immutable facts: capture, undo and redo derivations.
        let bytes = live.projected_docx().unwrap();
        let opened =
            LiveDocument::open("d", "b", &bytes, &live.inner.snapshot().unwrap(), None).unwrap();
        assert_eq!(live.inner.facts().unwrap(), opened.inner.facts().unwrap());
    }
    fn extract_main(bytes: &[u8]) -> Vec<u8> {
        use std::io::Read;
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut out = vec![];
        zip.by_name("word/document.xml")
            .unwrap()
            .read_to_end(&mut out)
            .unwrap();
        out
    }
    #[test]
    fn rejected_policy_preserves_replica_binding_and_undo_then_allows_retry() {
        let before = package("before");
        let mut live = LiveDocument::create("d", "b", &before, None)
            .unwrap()
            .with_fact_policy(policy);
        let version = live.inner.version().unwrap();
        let binding = live.binding.clone();
        assert!(live
            .capture("a", &package("after"), None, vec![json!({"reject":true})])
            .is_err());
        assert_eq!(live.inner.version().unwrap(), version);
        assert_eq!(live.binding, binding);
        assert!(!live.inner.can_undo());
        assert!(live
            .append_values("a", vec![json!({"reject":true})])
            .is_err());
        assert_eq!(live.inner.version().unwrap(), version);
        live.capture("a", &package("after"), None, vec![]).unwrap();
        assert!(live.inner.can_undo());
    }
}
