//! An owned XML tree that keeps prefixes, for timing and transition markup.
//!
//! The rest of the reader strips prefixes as it reads; animation markup is
//! also written back — an effect PowerPoint wrote is kept as it was and only
//! the attributes an edit touches change — so here a node remembers its
//! qualified name and every attribute (namespace declarations included) and
//! serialises back to the same markup.

use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

use crate::error::{PptxError, Result};

/// One element: its qualified name, attributes as written and children.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct XmlNode {
    /// `p:cTn`, as written.
    pub name: String,
    /// Attributes as written, `xmlns` declarations included, unescaped.
    pub attrs: Vec<(String, String)>,
    pub children: Vec<XmlChild>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum XmlChild {
    Element(XmlNode),
    /// Character data, unescaped.
    Text(String),
}

fn local(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}

impl XmlNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            attrs: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Builder: an attribute.
    pub fn with_attr(mut self, name: &str, value: impl Into<String>) -> Self {
        self.set_attr(name, value);
        self
    }

    /// Builder: a child element.
    pub fn with_child(mut self, child: XmlNode) -> Self {
        self.children.push(XmlChild::Element(child));
        self
    }

    /// Builder: text content.
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.children.push(XmlChild::Text(text.into()));
        self
    }

    /// The name without its prefix.
    pub fn local_name(&self) -> &str {
        local(&self.name)
    }

    /// The attribute whose local name is `name` (`r:embed` answers `embed`).
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name || (!name.contains(':') && local(key) == name))
            .map(|(_, value)| value.as_str())
    }

    /// Sets an attribute, keeping its place when it is already there.
    pub fn set_attr(&mut self, name: &str, value: impl Into<String>) {
        let value = value.into();
        match self.attrs.iter_mut().find(|(key, _)| key == name) {
            Some((_, existing)) => *existing = value,
            None => self.attrs.push((name.to_string(), value)),
        }
    }

    pub fn remove_attr(&mut self, name: &str) {
        self.attrs.retain(|(key, _)| key != name);
    }

    pub fn elements(&self) -> impl Iterator<Item = &XmlNode> {
        self.children.iter().filter_map(|child| match child {
            XmlChild::Element(node) => Some(node),
            XmlChild::Text(_) => None,
        })
    }

    pub fn elements_mut(&mut self) -> impl Iterator<Item = &mut XmlNode> {
        self.children.iter_mut().filter_map(|child| match child {
            XmlChild::Element(node) => Some(node),
            XmlChild::Text(_) => None,
        })
    }

    /// The first child element with local name `name`.
    pub fn child(&self, name: &str) -> Option<&XmlNode> {
        self.elements().find(|node| node.local_name() == name)
    }

    pub fn child_mut(&mut self, name: &str) -> Option<&mut XmlNode> {
        self.elements_mut().find(|node| node.local_name() == name)
    }

    /// Every child element with local name `name`.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a XmlNode> + 'a {
        self.elements()
            .filter(move |node| node.local_name() == name)
    }

    /// Follows a path of local names through first children.
    pub fn path(&self, names: &[&str]) -> Option<&XmlNode> {
        names.iter().try_fold(self, |node, name| node.child(name))
    }

    pub fn path_mut(&mut self, names: &[&str]) -> Option<&mut XmlNode> {
        let mut node = self;
        for name in names {
            node = node.child_mut(name)?;
        }
        Some(node)
    }

    /// Depth-first: the first descendant with local name `name`.
    pub fn descendant(&self, name: &str) -> Option<&XmlNode> {
        for child in self.elements() {
            if child.local_name() == name {
                return Some(child);
            }
            if let Some(found) = child.descendant(name) {
                return Some(found);
            }
        }
        None
    }

    /// Every descendant (self excluded) with local name `name`, in
    /// document order.
    pub fn descendants<'a>(&'a self, name: &str) -> Vec<&'a XmlNode> {
        let mut found = Vec::new();
        fn walk<'a>(node: &'a XmlNode, name: &str, found: &mut Vec<&'a XmlNode>) {
            for child in node.elements() {
                if child.local_name() == name {
                    found.push(child);
                }
                walk(child, name, found);
            }
        }
        walk(self, name, &mut found);
        found
    }

    /// Visits self and every descendant element, depth first.
    pub fn visit_mut(&mut self, visit: &mut dyn FnMut(&mut XmlNode)) {
        visit(self);
        for child in self.elements_mut() {
            child.visit_mut(visit);
        }
    }

    /// The concatenated text of direct text children.
    pub fn text(&self) -> String {
        self.children
            .iter()
            .filter_map(|child| match child {
                XmlChild::Text(text) => Some(text.as_str()),
                XmlChild::Element(_) => None,
            })
            .collect()
    }

    /// The prefix of the name (`p` for `p:cTn`), when it has one.
    pub fn prefix(&self) -> Option<&str> {
        self.name.split_once(':').map(|(prefix, _)| prefix)
    }

    /// Parses `xml`'s root element.
    pub fn parse(xml: &str) -> Result<XmlNode> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(false);
        let mut stack: Vec<XmlNode> = Vec::new();
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf)? {
                Event::Start(ref element) => stack.push(read_start(element)?),
                Event::Empty(ref element) => {
                    let node = read_start(element)?;
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(XmlChild::Element(node)),
                        None => return Ok(node),
                    }
                }
                Event::End(_) => {
                    let node = stack
                        .pop()
                        .ok_or_else(|| PptxError::Malformed("unbalanced XML".to_string()))?;
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(XmlChild::Element(node)),
                        None => return Ok(node),
                    }
                }
                Event::Text(ref text) => {
                    if let Some(parent) = stack.last_mut() {
                        push_text(parent, &text.decode()?);
                    }
                }
                Event::CData(ref text) => {
                    if let Some(parent) = stack.last_mut() {
                        push_text(parent, &text.decode()?);
                    }
                }
                Event::GeneralRef(ref reference) => {
                    if let Some(parent) = stack.last_mut() {
                        let name = reference.decode()?;
                        if let Ok(decoded) = quick_xml::escape::unescape(&format!("&{name};")) {
                            push_text(parent, &decoded);
                        }
                    }
                }
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        Err(PptxError::Malformed("no root element".to_string()))
    }

    /// Serialises the element back to markup.
    pub fn to_xml(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        out.push('<');
        out.push_str(&self.name);
        for (key, value) in &self.attrs {
            out.push(' ');
            out.push_str(key);
            out.push_str("=\"");
            out.push_str(&escape(value, true));
            out.push('"');
        }
        if self.children.is_empty() {
            out.push_str("/>");
            return;
        }
        out.push('>');
        for child in &self.children {
            match child {
                XmlChild::Element(node) => node.write(out),
                XmlChild::Text(text) => out.push_str(&escape(text, false)),
            }
        }
        out.push_str("</");
        out.push_str(&self.name);
        out.push('>');
    }
}

fn push_text(parent: &mut XmlNode, text: &str) {
    if let Some(XmlChild::Text(existing)) = parent.children.last_mut() {
        existing.push_str(text);
    } else {
        parent.children.push(XmlChild::Text(text.to_string()));
    }
}

fn read_start(element: &quick_xml::events::BytesStart<'_>) -> Result<XmlNode> {
    let mut attrs = Vec::new();
    for attribute in element.attributes() {
        let attribute = attribute?;
        attrs.push((
            String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
            attribute.unescape_value()?.into_owned(),
        ));
    }
    Ok(XmlNode {
        name: String::from_utf8_lossy(element.name().as_ref()).into_owned(),
        attrs,
        children: Vec::new(),
    })
}

/// Escapes text or an attribute value for markup.
pub fn escape(value: &str, attribute: bool) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attribute => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_markup_with_prefixes_and_namespaces() {
        let xml = r#"<p:par xmlns:p="urn:p"><p:cTn id="5" presetID="10"><p:attrName>style.visibility</p:attrName><a:srgbClr val="A&amp;B"/></p:cTn></p:par>"#;
        let node = XmlNode::parse(xml).unwrap();
        assert_eq!(node.to_xml(), xml);
        let ctn = node.child("cTn").unwrap();
        assert_eq!(ctn.attr("presetID"), Some("10"));
        assert_eq!(ctn.child("attrName").unwrap().text(), "style.visibility");
    }
}
