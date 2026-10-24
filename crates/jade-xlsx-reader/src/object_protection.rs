//! Owned drawing identities, protection flags and preserving edit locations.
use crate::{CellRef, Result, XlsxError};
use quick_xml::{events::Event, name::ResolveResult, NsReader, Reader};
use std::{collections::BTreeMap, ops::Range};

const XDR: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
const STRICT_XDR: &str = "http://purl.oclc.org/ooxml/drawingml/spreadsheetDrawing";
const VML: &str = "urn:schemas-microsoft-com:vml";
const XVML: &str = "urn:schemas-microsoft-com:office:excel";
fn bad(message: &str) -> XlsxError {
    XlsxError::BadDrawingProtection(message.into())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectLocks {
    pub locked: bool,
    pub text_locked: bool,
    pub ambiguous: bool,
}
#[derive(Debug, Clone)]
pub struct DrawingSource {
    pub id: u32,
    pub name: String,
    pub span: Range<usize>,
    pub head_end: usize,
    pub end_start: usize,
    pub qualified: String,
    pub kind: String,
    pub object_span: Range<usize>,
    pub positions: Vec<Range<usize>>,
    pub locks: ObjectLocks,
    pub text: Option<TextSource>,
    pub chart_relationship: Option<String>,
}
#[derive(Debug, Clone)]
pub struct TextSource {
    pub paragraphs: Vec<Range<usize>>,
    pub insertion: usize,
    pub namespace: String,
    pub declarations: String,
    pub paragraph_properties: String,
    pub run_properties: String,
    pub end_properties: String,
}
#[derive(Debug, Clone)]
pub struct NoteSource {
    pub at: CellRef,
    pub span: Range<usize>,
    pub locks: ObjectLocks,
}
pub struct VmlSources {
    pub notes: Vec<NoteSource>,
    pub max_shape_number: u32,
    pub has_note_type: bool,
    pub first_shape: Option<usize>,
    pub root_end: usize,
    pub root_span: Range<usize>,
    pub root_empty: bool,
    pub root_qualified: String,
}
#[derive(Debug, Clone)]
pub struct ElementSource {
    pub span: Range<usize>,
    pub head_end: usize,
    pub end_start: usize,
    pub qualified: String,
    pub empty: bool,
}
#[derive(Debug, Clone)]
pub struct CommentSource {
    pub at: Option<CellRef>,
    pub element: ElementSource,
    pub text: Option<ElementSource>,
    pub author_id: Option<usize>,
    pub value: String,
    pub run_properties: String,
    pub run_declarations: String,
    pub alternatives: Vec<CommentTextSource>,
}
#[derive(Debug, Clone)]
pub struct CommentTextSource {
    pub element: ElementSource,
    pub run_properties: String,
    pub declarations: String,
}
pub struct CommentsSource {
    pub namespace: String,
    pub authors: Vec<String>,
    pub authors_element: Option<ElementSource>,
    pub list: ElementSource,
    pub comments: Vec<CommentSource>,
    pub can_drop_empty: bool,
}
fn element(n: &Node) -> ElementSource {
    ElementSource {
        span: n.span.clone(),
        head_end: n.head_end,
        end_start: n.end_start,
        qualified: n.qualified.clone(),
        empty: n.empty,
    }
}
/// An interpreted relationship view; preserving writers keep the original XML.
pub fn owned_relationships(xml: &str) -> Result<jade_opc::Relationships> {
    let t = tree(xml)?;
    let root = &t.nodes[t.root];
    let ns = root.namespace.as_str();
    if root.local != "Relationships"
        || !matches!(
            ns,
            "" | "http://schemas.openxmlformats.org/package/2006/relationships"
        )
    {
        return Err(bad("not a relationships part"));
    }
    let mut canonical = String::from("<Relationships>");
    let mut ids = std::collections::HashSet::new();
    for i in t.children(t.root, ns, "Relationship") {
        let n = &t.nodes[i];
        let id = n
            .attrs
            .get("Id")
            .ok_or_else(|| bad("missing relationship id"))?;
        if !ids.insert(id) {
            return Err(bad("duplicate owned relationship id"));
        }
        let kind = n
            .attrs
            .get("Type")
            .ok_or_else(|| bad("missing relationship type"))?;
        let target = n
            .attrs
            .get("Target")
            .ok_or_else(|| bad("missing relationship target"))?;
        let mode = n
            .attrs
            .get("TargetMode")
            .map_or("Internal", |s| s.trim_matches([' ', '\t', '\r', '\n']));
        if !matches!(mode, "Internal" | "External") {
            return Err(bad("invalid relationship target mode"));
        }
        canonical.push_str(&format!(
            "<Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\" TargetMode=\"{mode}\"/>",
            escape(id),
            escape(kind),
            escape(target)
        ));
    }
    canonical.push_str("</Relationships>");
    Ok(jade_opc::parse_relationships(&canonical)?)
}
pub fn office_relationship(r: &jade_opc::Relationship, kind: &str) -> bool {
    !r.external
        && (r.relationship_type
            == format!(
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}"
            )
            || r.relationship_type
                == format!("http://purl.oclc.org/ooxml/officeDocument/relationships/{kind}"))
}
/// Empty-part cleanup must not discard opaque package registry metadata.
pub fn plain_package_registry(xml: &str, content_types: bool) -> Result<bool> {
    let t = tree(xml)?;
    let root = &t.nodes[t.root];
    let (name, ns) = if content_types {
        (
            "Types",
            "http://schemas.openxmlformats.org/package/2006/content-types",
        )
    } else {
        (
            "Relationships",
            "http://schemas.openxmlformats.org/package/2006/relationships",
        )
    };
    if root.local != name || !(root.namespace.is_empty() || root.namespace == ns) {
        return Ok(false);
    }
    let namespaces = |k: &String| k == "xmlns" || k.starts_with("xmlns:");
    if root.attrs.keys().any(|k| !namespaces(k)) {
        return Ok(false);
    }
    Ok(root.children.iter().all(|i| {
        let n = &t.nodes[*i];
        let allowed: &[&str] = match (content_types, n.local.as_str()) {
            (true, "Override") => &["PartName", "ContentType"],
            (true, "Default") => &["Extension", "ContentType"],
            (false, "Relationship") => &["Id", "Type", "Target", "TargetMode"],
            _ => return false,
        };
        n.namespace == root.namespace
            && n.children.is_empty()
            && n.attrs
                .keys()
                .all(|k| namespaces(k) || allowed.contains(&k.as_str()))
    }))
}
struct Node {
    namespace: String,
    local: String,
    qualified: String,
    attrs: BTreeMap<String, String>,
    children: Vec<usize>,
    parent: Option<usize>,
    span: Range<usize>,
    head_end: usize,
    end_start: usize,
    empty: bool,
    relationships: Vec<String>,
    mc_supported: bool,
}
struct Tree {
    nodes: Vec<Node>,
    root: usize,
}
fn tree(xml: &str) -> Result<Tree> {
    let mut reader = NsReader::from_str(xml);
    let mut nodes: Vec<Node> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut root = None;
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event()? {
            event @ (Event::Start(_) | Event::Empty(_)) => {
                let empty = matches!(event, Event::Empty(_));
                let e = match event {
                    Event::Start(e) | Event::Empty(e) => e,
                    _ => unreachable!(),
                };
                let namespace = match reader.resolver().resolve_element(e.name()).0 {
                    ResolveResult::Unbound => String::new(),
                    ResolveResult::Bound(n) => String::from_utf8_lossy(n.as_ref()).into_owned(),
                    _ => return Err(bad("unbound drawing prefix")),
                };
                let qualified = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                let local =
                    String::from_utf8_lossy(jade_opc::local_name(e.name().as_ref())).into_owned();
                let mc_supported = namespace
                    == "http://schemas.openxmlformats.org/markup-compatibility/2006"
                    && (local == "Fallback"
                        || (local == "Choice"
                            && crate::data_validations::choice_supported(&reader, &e)?));
                let mut attrs = BTreeMap::new();
                let mut relationships = Vec::new();
                for a in e.attributes() {
                    let a = a?;
                    let value = a.unescape_value()?.into_owned();
                    let (ns, local) = reader.resolver().resolve_attribute(a.key);
                    if local.as_ref() == b"id"
                        && matches!(ns,ResolveResult::Bound(n) if matches!(n.as_ref(),b"http://schemas.openxmlformats.org/officeDocument/2006/relationships"|b"http://purl.oclc.org/ooxml/officeDocument/relationships"))
                    {
                        relationships.push(value.clone());
                    }
                    attrs.insert(String::from_utf8_lossy(a.key.as_ref()).into_owned(), value);
                }
                let head_end = reader.buffer_position() as usize;
                let index = nodes.len();
                nodes.push(Node {
                    namespace,
                    local,
                    qualified,
                    attrs,
                    relationships,
                    mc_supported,
                    children: Vec::new(),
                    parent: stack.last().copied(),
                    span: start..head_end,
                    head_end,
                    end_start: head_end,
                    empty,
                });
                if let Some(parent) = stack.last() {
                    nodes[*parent].children.push(index);
                } else if root.replace(index).is_some() {
                    return Err(bad("multiple drawing roots"));
                }
                if !empty {
                    stack.push(index);
                }
            }
            Event::End(_) => {
                let index = stack.pop().ok_or_else(|| bad("unbalanced drawing"))?;
                nodes[index].end_start = start;
                nodes[index].span.end = reader.buffer_position() as usize;
            }
            Event::Eof => {
                if !stack.is_empty() {
                    return Err(bad("incomplete drawing"));
                }
                return Ok(Tree {
                    nodes,
                    root: root.ok_or_else(|| bad("missing drawing root"))?,
                });
            }
            _ => {}
        }
    }
}
impl Tree {
    fn effective_children(
        &self,
        parent: usize,
        namespace: &str,
        local: &str,
        compatible: bool,
    ) -> Vec<usize> {
        let mut pending: Vec<_> = self.nodes[parent].children.iter().rev().copied().collect();
        let mut result = Vec::new();
        while let Some(i) = pending.pop() {
            let n = &self.nodes[i];
            if n.namespace == namespace && n.local == local {
                result.push(i);
            } else if n.namespace == "http://schemas.openxmlformats.org/markup-compatibility/2006"
                && n.local == "AlternateContent"
            {
                let mut branches: Vec<_> = n
                    .children
                    .iter()
                    .copied()
                    .filter(|b| self.nodes[*b].mc_supported)
                    .collect();
                if !compatible {
                    branches.truncate(1);
                }
                for b in branches.into_iter().rev() {
                    pending.extend(self.nodes[b].children.iter().rev().copied());
                }
            }
        }
        result
    }

    fn children(&self, index: usize, namespace: &str, local: &str) -> Vec<usize> {
        self.nodes[index]
            .children
            .iter()
            .copied()
            .filter(|i| self.nodes[*i].namespace == namespace && self.nodes[*i].local == local)
            .collect()
    }
    fn declaration_scope(&self, mut index: usize) -> String {
        let mut path = vec![index];
        while let Some(p) = self.nodes[index].parent {
            path.push(p);
            index = p;
        }
        let mut attrs = BTreeMap::new();
        for i in path.into_iter().rev() {
            for (k, v) in &self.nodes[i].attrs {
                if k == "xmlns" || k.starts_with("xmlns:") {
                    attrs.insert(k, v);
                }
            }
        }
        attrs
            .into_iter()
            .map(|(k, v)| format!(" {k}=\"{}\"", escape(v)))
            .collect()
    }
}
fn escape(value: &str) -> String {
    quick_xml::escape::escape(value).into_owned()
}
fn xml_bool(value: Option<&String>, default: bool) -> Option<bool> {
    match value.map(|s| s.trim_matches([' ', '\t', '\r', '\n'])) {
        None => Some(default),
        Some("1" | "true") => Some(true),
        Some("0" | "false") => Some(false),
        _ => None,
    }
}
fn renamed(xml: &str, node: &Node, local: &str) -> String {
    let prefix = node.qualified.rsplit_once(':').map_or("", |(p, _)| p);
    let name = if prefix.is_empty() {
        local.to_string()
    } else {
        format!("{prefix}:{local}")
    };
    let tail = &xml[node.span.start + 1 + node.qualified.len()..node.head_end];
    if node.empty {
        format!("<{name}{tail}")
    } else {
        format!(
            "<{name}{tail}{}</{name}>",
            &xml[node.head_end..node.end_start]
        )
    }
}
fn text_source(xml: &str, t: &Tree, object: usize, xdr: &str) -> Option<TextSource> {
    let bodies = t.children(object, xdr, "txBody");
    if bodies.len() != 1 || t.nodes[bodies[0]].empty {
        return None;
    }
    let body = bodies[0];
    let main = if xdr == STRICT_XDR {
        "http://purl.oclc.org/ooxml/drawingml/main"
    } else {
        "http://schemas.openxmlformats.org/drawingml/2006/main"
    };
    let owned = |i: usize, local: &str| {
        t.nodes[i].local == local
            && (t.nodes[i].namespace == main || (xdr.is_empty() && t.nodes[i].namespace.is_empty()))
    };
    let paragraphs: Vec<_> = t.nodes[body]
        .children
        .iter()
        .copied()
        .filter(|i| owned(*i, "p"))
        .collect();
    let scope = paragraphs.first().copied().unwrap_or(body);
    let paragraph_properties = paragraphs
        .first()
        .and_then(|p| {
            t.nodes[*p]
                .children
                .iter()
                .copied()
                .find(|i| owned(*i, "pPr"))
        })
        .map(|i| xml[t.nodes[i].span.clone()].to_string())
        .unwrap_or_default();
    let mut style = None;
    if let Some(p) = paragraphs.first() {
        for r in &t.nodes[*p].children {
            if owned(*r, "r") || owned(*r, "fld") {
                style = t.nodes[*r]
                    .children
                    .iter()
                    .copied()
                    .find(|i| owned(*i, "rPr"));
                if style.is_some() {
                    break;
                }
            }
        }
        if style.is_none() {
            style = t.nodes[*p]
                .children
                .iter()
                .copied()
                .find(|i| owned(*i, "endParaRPr"));
        }
    }
    Some(TextSource {
        insertion: paragraphs
            .first()
            .map_or(t.nodes[body].end_start, |i| t.nodes[*i].span.start),
        paragraphs: paragraphs
            .iter()
            .map(|i| t.nodes[*i].span.clone())
            .collect(),
        namespace: main.into(),
        declarations: t.declaration_scope(scope),
        paragraph_properties,
        run_properties: style
            .map(|i| renamed(xml, &t.nodes[i], "rPr"))
            .unwrap_or_default(),
        end_properties: style
            .map(|i| renamed(xml, &t.nodes[i], "endParaRPr"))
            .unwrap_or_default(),
    })
}
/// Direct, owned anchors and nonvisual IDs only. Missing/ambiguous metadata
/// never becomes an independently unlocked object.
pub fn drawing_sources(xml: &str) -> Result<Vec<DrawingSource>> {
    let t = tree(xml)?;
    let root = &t.nodes[t.root];
    let ns = root.namespace.as_str();
    if root.local != "wsDr" || !matches!(ns, "" | XDR | STRICT_XDR) {
        return Err(bad("not a spreadsheet drawing"));
    }
    let mut result: Vec<DrawingSource> = Vec::new();
    for a in &root.children {
        let anchor = &t.nodes[*a];
        if anchor.namespace != ns
            || !matches!(
                anchor.local.as_str(),
                "twoCellAnchor" | "oneCellAnchor" | "absoluteAnchor"
            )
            || anchor.empty
        {
            continue;
        }
        let objects: Vec<_> = anchor
            .children
            .iter()
            .copied()
            .filter(|i| {
                t.nodes[*i].namespace == ns
                    && matches!(
                        t.nodes[*i].local.as_str(),
                        "sp" | "pic" | "graphicFrame" | "grpSp" | "cxnSp"
                    )
            })
            .collect();
        if objects.len() != 1 {
            continue;
        }
        let object = objects[0];
        let o = &t.nodes[object];
        let nv = match o.local.as_str() {
            "sp" => "nvSpPr",
            "pic" => "nvPicPr",
            "graphicFrame" => "nvGraphicFramePr",
            "grpSp" => "nvGrpSpPr",
            _ => "nvCxnSpPr",
        };
        let nonvisual = t.children(object, ns, nv);
        let ids: Vec<_> = nonvisual
            .iter()
            .flat_map(|n| t.children(*n, ns, "cNvPr"))
            .collect();
        if ids.len() != 1 {
            continue;
        }
        let id_node = &t.nodes[ids[0]];
        let Some(id) = id_node.attrs.get("id").and_then(|v| v.parse::<u32>().ok()) else {
            continue;
        };
        let clients = t.children(*a, ns, "clientData");
        let locked = clients
            .first()
            .and_then(|i| xml_bool(t.nodes[*i].attrs.get("fLocksWithSheet"), true))
            .unwrap_or(true);
        let text_locked = if o.local == "sp" {
            xml_bool(o.attrs.get("fLocksText"), true)
        } else {
            Some(true)
        };
        let ambiguous = nonvisual.len() != 1
            || clients.len() > 1
            || clients.first().is_some_and(|i| {
                !t.nodes[*i].children.is_empty()
                    || xml_bool(t.nodes[*i].attrs.get("fLocksWithSheet"), true).is_none()
            })
            || text_locked.is_none();
        let main = if ns == STRICT_XDR {
            "http://purl.oclc.org/ooxml/drawingml/main"
        } else {
            "http://schemas.openxmlformats.org/drawingml/2006/main"
        };
        let chart_ns = if ns == STRICT_XDR {
            "http://purl.oclc.org/ooxml/drawingml/chart"
        } else {
            "http://schemas.openxmlformats.org/drawingml/2006/chart"
        };
        let charts: Vec<_> = t
            .children(object, main, "graphic")
            .iter()
            .flat_map(|i| t.children(*i, main, "graphicData"))
            .flat_map(|i| t.children(i, chart_ns, "chart"))
            .collect();
        let chart_relationship = if charts.len() == 1 && t.nodes[charts[0]].relationships.len() == 1
        {
            Some(t.nodes[charts[0]].relationships[0].clone())
        } else {
            None
        };
        result.push(DrawingSource {
            id,
            name: id_node.attrs.get("name").cloned().unwrap_or_default(),
            span: anchor.span.clone(),
            head_end: anchor.head_end,
            end_start: anchor.end_start,
            qualified: anchor.qualified.clone(),
            kind: anchor.local.clone(),
            object_span: o.span.clone(),
            positions: anchor
                .children
                .iter()
                .filter(|i| {
                    t.nodes[**i].namespace == ns
                        && matches!(t.nodes[**i].local.as_str(), "from" | "to" | "ext" | "pos")
                })
                .map(|i| t.nodes[*i].span.clone())
                .collect(),
            locks: ObjectLocks {
                locked,
                text_locked: text_locked.unwrap_or(true),
                ambiguous,
            },
            text: text_source(xml, &t, object, ns),
            chart_relationship,
        });
    }
    let mut counts = BTreeMap::new();
    for item in &result {
        *counts.entry(item.id).or_insert(0usize) += 1;
    }
    for item in &mut result {
        item.locks.ambiguous |= counts[&item.id] != 1;
    }
    Ok(result)
}
pub fn drawing_source(xml: &str, id: u32) -> Result<Option<DrawingSource>> {
    let item = drawing_sources(xml)?.into_iter().find(|r| r.id == id);
    if item.as_ref().is_some_and(|r| r.locks.ambiguous) {
        return Err(bad("ambiguous drawing identity or protection"));
    }
    Ok(item)
}
/// The reader-selected owned worksheet drawing relationship. Attribute aliases
/// must resolve to the Office relationships namespace too.
pub fn worksheet_drawing_relationship(xml: &str, legacy: bool) -> Result<Option<String>> {
    crate::cell_content::worksheet_drawing_relationship(xml, legacy)
}
fn leaf_text(xml: &str, n: &Node) -> Result<String> {
    if !n.children.is_empty() {
        return Err(bad("nested note metadata"));
    }
    let mut reader = Reader::from_str(&xml[n.head_end..n.end_start]);
    let mut value = String::new();
    loop {
        match reader.read_event()? {
            Event::Text(t) => value.push_str(&t.xml_content()?),
            Event::CData(t) => value.push_str(&t.decode()?),
            Event::GeneralRef(t) => value.push_str(
                &quick_xml::escape::unescape(&format!("&{};", t.decode()?))
                    .map_err(|_| bad("invalid note entity"))?,
            ),
            Event::Eof => return Ok(value),
            Event::Comment(_) => {}
            _ => return Err(bad("invalid note metadata")),
        }
    }
}
/// VML Note client data is identified by its direct Row/Column leaves.
pub fn note_sources(xml: &str) -> Result<Vec<NoteSource>> {
    Ok(vml_sources(xml)?.notes)
}
pub fn vml_sources(xml: &str) -> Result<VmlSources> {
    let t = tree(xml)?;
    let root = &t.nodes[t.root];
    if root.local != "xml" || !matches!(root.namespace.as_str(), "" | VML) {
        return Err(bad("not a VML note drawing"));
    }
    let mut result = Vec::new();
    for shape in &root.children {
        let n = &t.nodes[*shape];
        if n.namespace != VML || n.local != "shape" {
            continue;
        }
        let clients: Vec<_> = t
            .children(*shape, XVML, "ClientData")
            .into_iter()
            .filter(|i| {
                t.nodes[*i]
                    .attrs
                    .get("ObjectType")
                    .is_some_and(|s| s == "Note")
            })
            .collect();
        if clients.len() != 1 {
            continue;
        }
        let client = clients[0];
        let mut ambiguous = t.children(*shape, XVML, "ClientData").len() != 1;
        let mut value = |name: &str| -> Option<String> {
            let items = t.children(client, XVML, name);
            if items.len() > 1 {
                ambiguous = true;
            }
            items
                .first()
                .and_then(|i| match leaf_text(xml, &t.nodes[*i]) {
                    Ok(s) => Some(s),
                    Err(_) => {
                        ambiguous = true;
                        None
                    }
                })
        };
        let row = value("Row").and_then(|v| v.trim().parse::<u32>().ok());
        let column = value("Column").and_then(|v| v.trim().parse::<u32>().ok());
        let (Some(row), Some(column)) = (row, column) else {
            continue;
        };
        if row >= crate::model::MAX_ROW_COUNT || column >= crate::model::MAX_COLUMN_COUNT {
            continue;
        }
        let locked = value("Locked");
        let text = value("LockText");
        let flag = |v: Option<String>| -> Option<bool> {
            match v
                .as_deref()
                .map(|s| s.trim_matches([' ', '\t', '\r', '\n']))
            {
                None | Some("" | "True" | "t") => Some(true),
                Some("False" | "f") => Some(false),
                _ => None,
            }
        };
        let locked = flag(locked);
        let text = flag(text);
        ambiguous |= locked.is_none() || text.is_none();
        result.push(NoteSource {
            at: CellRef::new(column, row),
            span: n.span.clone(),
            locks: ObjectLocks {
                locked: locked.unwrap_or(true),
                text_locked: text.unwrap_or(true),
                ambiguous,
            },
        });
    }
    let mut counts = BTreeMap::new();
    for n in &result {
        *counts.entry(n.at).or_insert(0usize) += 1;
    }
    for n in &mut result {
        n.locks.ambiguous |= counts[&n.at] != 1;
    }
    let shapes: Vec<_> = root
        .children
        .iter()
        .map(|i| &t.nodes[*i])
        .filter(|n| n.namespace == VML && n.local == "shape")
        .collect();
    let max_shape_number = shapes
        .iter()
        .filter_map(|n| {
            n.attrs
                .get("id")
                .and_then(|s| s.strip_prefix("_x0000_s"))
                .and_then(|s| s.parse::<u32>().ok())
        })
        .max()
        .unwrap_or(1024)
        .max(1024);
    let has_note_type = root.children.iter().map(|i| &t.nodes[*i]).any(|n| {
        n.namespace == VML
            && n.local == "shapetype"
            && n.attrs.get("id").is_some_and(|s| s == "_x0000_t202")
    });
    Ok(VmlSources {
        notes: result,
        max_shape_number,
        has_note_type,
        first_shape: shapes.first().map(|n| n.span.start),
        root_end: root.end_start,
        root_span: root.span.clone(),
        root_empty: root.empty,
        root_qualified: root.qualified.clone(),
    })
}
/// Standard/Strict direct comment ownership, shared with preserving note edits.
pub fn comments_source(xml: &str) -> Result<CommentsSource> {
    let t = tree(xml)?;
    let root = &t.nodes[t.root];
    let ns = root.namespace.as_str();
    if root.local != "comments"
        || !matches!(
            ns,
            "" | "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                | "http://purl.oclc.org/ooxml/spreadsheetml/main"
        )
    {
        return Err(bad("not a comments part"));
    }
    let is_mce = |i: &usize| {
        t.nodes[*i].namespace == "http://schemas.openxmlformats.org/markup-compatibility/2006"
            && t.nodes[*i].local == "AlternateContent"
    };
    if root.children.iter().any(is_mce) {
        return Err(bad("MCE comments containers cannot be edited"));
    }
    let authors = t.children(t.root, ns, "authors");
    let lists = t.children(t.root, ns, "commentList");
    if authors.len() > 1 || lists.len() != 1 {
        return Err(bad("ambiguous comments containers"));
    }
    let list = lists[0];
    if t.nodes[list].children.iter().any(is_mce)
        || authors
            .iter()
            .any(|a| t.nodes[*a].children.iter().any(is_mce))
    {
        return Err(bad("MCE comment/author records cannot be edited"));
    }
    let names: Vec<_> = authors
        .first()
        .map(|i| t.children(*i, ns, "author"))
        .unwrap_or_default();
    let author_names = names
        .iter()
        .map(|i| leaf_text(xml, &t.nodes[*i]).map(|v| crate::xstring::decode_xstring(&v)))
        .collect::<Result<Vec<_>>>()?;
    let mut comments = Vec::new();
    let mut counts = BTreeMap::new();
    for i in t.children(list, ns, "comment") {
        let n = &t.nodes[i];
        let at = n.attrs.get("ref").and_then(|v| CellRef::parse(v).ok());
        if let Some(at) = at {
            *counts.entry(at).or_insert(0usize) += 1;
        }
        let texts = t.effective_children(i, ns, "text", false);
        let all_texts = t.effective_children(i, ns, "text", true);
        if texts.is_empty() && t.nodes[i].children.iter().any(is_mce) {
            return Err(bad("unsupported comment text alternatives"));
        }
        if texts.len() > 1 {
            return Err(bad("ambiguous comment text"));
        }
        let style = texts.first().and_then(|i| {
            t.children(*i, ns, "r")
                .into_iter()
                .flat_map(|r| t.children(r, ns, "rPr"))
                .next()
        });
        comments.push(CommentSource {
            at,
            element: element(n),
            text: texts.first().map(|i| element(&t.nodes[*i])),
            author_id: n.attrs.get("authorId").and_then(|s| s.parse().ok()),
            alternatives: all_texts
                .into_iter()
                .map(|i| {
                    let style = t
                        .children(i, ns, "r")
                        .into_iter()
                        .flat_map(|r| t.children(r, ns, "rPr"))
                        .next();
                    CommentTextSource {
                        element: element(&t.nodes[i]),
                        run_properties: style
                            .map(|j| xml[t.nodes[j].span.clone()].to_string())
                            .unwrap_or_default(),
                        declarations: style.map(|j| t.declaration_scope(j)).unwrap_or_default(),
                    }
                })
                .collect(),
            run_properties: style
                .map(|i| xml[t.nodes[i].span.clone()].to_string())
                .unwrap_or_default(),
            run_declarations: style.map(|i| t.declaration_scope(i)).unwrap_or_default(),
            value: if let Some(i) = texts.first() {
                let mut prefix = "jadeCommentValue".to_string();
                while xml.contains(&prefix) {
                    prefix.push('_');
                }
                let scope = t.declaration_scope(*i);
                let raw = renamed(xml, &t.nodes[*i], "si");
                let wrapped = if ns.is_empty() {
                    format!("<sst{scope}>{raw}</sst>")
                } else {
                    format!(
                        "<{prefix}:sst xmlns:{prefix}=\"{}\"{scope}>{raw}</{prefix}:sst>",
                        escape(ns)
                    )
                };
                crate::shared_strings::parse_shared_strings(&wrapped)?
                    .into_iter()
                    .next()
                    .unwrap_or_default()
            } else {
                String::new()
            },
        });
    }
    if counts.values().any(|n| *n > 1) {
        return Err(bad("duplicate owned comments"));
    }
    let known_attrs = |n: &Node| {
        n.attrs
            .keys()
            .all(|k| k == "xmlns" || k.starts_with("xmlns:"))
    };
    let can_drop_empty = known_attrs(root)
        && root
            .children
            .iter()
            .all(|i| authors.contains(i) || lists.contains(i))
        && known_attrs(&t.nodes[list])
        && t.nodes[list].children.len() == comments.len()
        && authors
            .first()
            .is_none_or(|i| known_attrs(&t.nodes[*i]) && t.nodes[*i].children.len() == names.len());
    Ok(CommentsSource {
        namespace: ns.into(),
        authors: author_names,
        authors_element: authors.first().map(|i| element(&t.nodes[*i])),
        list: element(&t.nodes[list]),
        comments,
        can_drop_empty,
    })
}
