//! Namespace/hierarchy ownership for pivot parts. Foreign subtrees never
//! contribute owned descendants; MCE is reported, not guessed.
use crate::{Result, XlsxError};
use quick_xml::{
    events::{BytesStart, Event},
    name::ResolveResult,
    NsReader,
};
use std::collections::BTreeMap;
pub(crate) const MAIN: &[&str] = &[
    "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
    "http://purl.oclc.org/ooxml/spreadsheetml/main",
];
pub(crate) const RELS: &[&str] = &[
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
    "http://purl.oclc.org/ooxml/officeDocument/relationships",
];
pub(crate) const PKG: &[&str] = &[
    "http://schemas.openxmlformats.org/package/2006/relationships",
    "http://purl.oclc.org/ooxml/package/relationships",
];
pub(crate) fn bad(message: impl Into<String>) -> XlsxError {
    XlsxError::BadPivot(message.into())
}
pub(crate) fn namespace(reader: &NsReader<&[u8]>, e: &BytesStart<'_>) -> Option<String> {
    match reader.resolver().resolve_element(e.name()).0 {
        ResolveResult::Unbound => Some(String::new()),
        ResolveResult::Bound(n) => std::str::from_utf8(n.as_ref()).ok().map(str::to_string),
        _ => None,
    }
}
pub(crate) fn attributes(
    reader: &NsReader<&[u8]>,
    e: &BytesStart<'_>,
) -> Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    for a in e.attributes() {
        let a = a?;
        let key = if !a.key.as_ref().contains(&b':') {
            std::str::from_utf8(a.key.as_ref()).ok().map(str::to_string)
        } else {
            let (ns, local) = reader.resolver().resolve_attribute(a.key);
            match ns {
                ResolveResult::Bound(n)
                    if RELS.iter().any(|s| s.as_bytes() == n.as_ref())
                        && local.as_ref() == b"id" =>
                {
                    Some("@relationship".into())
                }
                _ => None,
            }
        };
        if let Some(key) = key {
            if key == "xmlns" {
                continue;
            }
            if result
                .insert(key, a.unescape_value()?.into_owned())
                .is_some()
            {
                return Err(bad("duplicate expanded attribute"));
            }
        }
    }
    Ok(result)
}
#[derive(Debug)]
pub(crate) struct Node {
    pub name: String,
    pub own: bool,
    pub attrs: BTreeMap<String, String>,
    pub children: Vec<Node>,
}
impl Node {
    pub fn children<'a>(&'a self, name: &str) -> impl Iterator<Item = &'a Node> + 'a {
        let name = name.to_string();
        self.children
            .iter()
            .filter(move |n| n.own && n.name == name)
    }
    pub fn optional(&self, name: &str) -> Result<Option<&Node>> {
        let mut all = self.children(name);
        let one = all.next();
        if all.next().is_some() {
            return Err(bad(format!("duplicate {name}")));
        }
        Ok(one)
    }
    pub fn required(&self, name: &str) -> Result<&Node> {
        self.optional(name)?
            .ok_or_else(|| bad(format!("missing {name}")))
    }
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(String::as_str)
    }
    pub fn text(&self, name: &str) -> Result<String> {
        self.attr(name)
            .map(str::to_string)
            .ok_or_else(|| bad(format!("missing {name}")))
    }
    pub fn u32(&self, name: &str) -> Result<Option<u32>> {
        number(self.attr(name), name)
    }
    pub fn i32(&self, name: &str) -> Result<Option<i32>> {
        number(self.attr(name), name)
    }
    pub fn required_u32(&self, name: &str) -> Result<u32> {
        self.u32(name)?
            .ok_or_else(|| bad(format!("missing {name}")))
    }
    pub fn boolean(&self, name: &str, default: bool) -> Result<bool> {
        boolean(self.attr(name), name, default)
    }
}
pub(crate) fn number<T: std::str::FromStr>(value: Option<&str>, name: &str) -> Result<Option<T>> {
    value
        .map(|v| v.trim().parse().map_err(|_| bad(format!("invalid {name}"))))
        .transpose()
}
pub(crate) fn boolean(value: Option<&str>, name: &str, default: bool) -> Result<bool> {
    match value.map(str::trim) {
        None => Ok(default),
        Some("1" | "true") => Ok(true),
        Some("0" | "false") => Ok(false),
        _ => Err(bad(format!("invalid {name}"))),
    }
}
pub(crate) fn document(xml: &str, root: &str, namespaces: &[&str]) -> Result<Node> {
    let mut reader = NsReader::from_str(xml);
    let mut stack: Vec<Node> = Vec::new();
    let mut output = None;
    let mut root_ns = None;
    let mut nodes = 0usize;
    loop {
        let event = reader.read_event()?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let ns = namespace(&reader, &e);
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if stack.is_empty() {
                    if output.is_some()
                        || name != root
                        || !ns
                            .as_ref()
                            .is_some_and(|n| n.is_empty() || namespaces.contains(&n.as_str()))
                    {
                        return Err(bad(format!("invalid {root} root")));
                    }
                    root_ns = ns.clone();
                }
                let own = ns.is_some() && ns == root_ns && stack.last().is_none_or(|p| p.own);
                let node = Node {
                    name,
                    own,
                    attrs: attributes(&reader, &e)?,
                    children: Vec::new(),
                };
                nodes += 1;
                if nodes > 1_000_000 || stack.len() > 128 {
                    return Err(bad("pivot metadata resource limit"));
                }
                if empty {
                    if let Some(parent) = stack.last_mut() {
                        parent.children.push(node);
                    } else {
                        output = Some(node);
                    }
                } else {
                    stack.push(node);
                }
            }
            Event::End(_) => {
                let node = stack.pop().ok_or_else(|| bad("unexpected pivot end"))?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                } else {
                    output = Some(node);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err(bad("unclosed pivot element"));
    }
    output.ok_or_else(|| bad("empty pivot part"))
}
/// Only collect declarations at their semantic owner path. MCE wrappers select
/// one understood branch; foreign parents never contribute owned descendants.
/// Worksheet cell grids are skipped without building another document tree.
pub(crate) fn references(
    xml: &str,
    root: &str,
    container: &str,
    leaf: &str,
) -> Result<Vec<BTreeMap<String, String>>> {
    enum Frame {
        Element,
        Alternate { chosen: bool },
        Branch,
    }
    const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
    let mut reader = NsReader::from_str(xml);
    let mut frames = Vec::new();
    let mut path = Vec::new();
    let mut root_ns = None;
    let mut out = Vec::new();
    let mut containers = 0;
    let mut root_seen = false;
    let mut declared_count: Option<u32> = None;
    loop {
        let event = reader.read_event()?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                let ns = namespace(&reader, &e);
                if frames.len() > 128 {
                    return Err(bad("pivot declaration depth limit"));
                }
                if ns.as_deref() == Some(MC) && !path.is_empty() {
                    if name == "AlternateContent"
                        && !matches!(frames.last(), Some(Frame::Alternate { .. }))
                    {
                        if !empty {
                            frames.push(Frame::Alternate { chosen: false });
                        }
                        continue;
                    }
                    if matches!(name.as_str(), "Choice" | "Fallback") {
                        if let Some(Frame::Alternate { chosen }) = frames.last_mut() {
                            let supported = name == "Fallback"
                                || crate::data_validations::choice_supported(&reader, &e)?;
                            let selected = !*chosen && supported;
                            *chosen |= selected;
                            if !empty {
                                if selected {
                                    frames.push(Frame::Branch);
                                } else {
                                    reader.read_to_end(e.name())?;
                                }
                            }
                            continue;
                        }
                    }
                }
                if path.is_empty() {
                    if root_seen
                        || name != root
                        || !ns
                            .as_ref()
                            .is_some_and(|n| n.is_empty() || MAIN.contains(&n.as_str()))
                    {
                        return Err(bad(format!("invalid {root} root")));
                    }
                    root_seen = true;
                    root_ns = ns.clone();
                }
                let owned = ns.is_some()
                    && ns == root_ns
                    && !matches!(frames.last(), Some(Frame::Alternate { .. }))
                    && match path.as_slice() {
                        [] => name == root,
                        [_] => name == container,
                        [_, parent] => parent == container && name == leaf,
                        _ => false,
                    };
                if !owned {
                    if !empty {
                        reader.read_to_end(e.name())?;
                    }
                    continue;
                }
                if path.len() == 1 {
                    declared_count = number(
                        attributes(&reader, &e)?.get("count").map(String::as_str),
                        "count",
                    )?;
                    containers += 1;
                    if containers > 1 {
                        return Err(bad(format!("duplicate {container}")));
                    }
                }
                if path.len() == 2 {
                    out.push(attributes(&reader, &e)?);
                    if out.len() > 4096 {
                        return Err(bad("too many pivot declarations"));
                    }
                }
                if !empty {
                    path.push(name);
                    frames.push(Frame::Element);
                }
            }
            Event::End(_) => match frames.pop() {
                Some(Frame::Element) => {
                    path.pop();
                }
                Some(_) => {}
                None => return Err(bad("unexpected pivot declaration end")),
            },
            Event::Eof => break,
            _ => {}
        }
    }
    if !root_seen || !frames.is_empty() {
        return Err(bad("incomplete pivot declarations"));
    }
    if declared_count.is_some_and(|n| n as usize != out.len()) {
        return Err(bad(format!(
            "{container} count disagrees with declarations"
        )));
    }
    Ok(out)
}
