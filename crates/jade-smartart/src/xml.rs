//! A small owned XML tree that writes back what it read.
//!
//! A diagram's parts are edited in place — a point's text changed, a
//! connection added — while everything jade does not understand (extension
//! lists, other namespaces, attributes it never reads) has to survive. The
//! tree keeps qualified names, attribute order and comments, so parsing and
//! writing a part unchanged gives the same markup back.

use quick_xml::events::Event;
use quick_xml::Reader;

/// A node under an element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Element(Element),
    /// Character data, unescaped.
    Text(String),
    /// A comment, processing instruction or CDATA section, as written.
    Raw(String),
}

/// An element with its qualified name (`dgm:pt`) and attributes in order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

/// The local part of a qualified name: `dgm:pt` → `pt`.
pub fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

impl Element {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// Builder: an attribute added.
    pub fn with_attr(mut self, name: &str, value: impl Into<String>) -> Self {
        self.set_attr(name, value);
        self
    }

    /// Builder: a child element added.
    pub fn with_child(mut self, child: Element) -> Self {
        self.children.push(Node::Element(child));
        self
    }

    pub fn local(&self) -> &str {
        local(&self.name)
    }

    /// The value of the attribute whose local name is `name`.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name || local(key) == name)
            .map(|(_, value)| value.as_str())
    }

    /// Sets the attribute `name` (qualified, as it is to be written),
    /// replacing one with the same local name.
    pub fn set_attr(&mut self, name: &str, value: impl Into<String>) {
        let value = value.into();
        match self
            .attrs
            .iter_mut()
            .find(|(key, _)| key == name || local(key) == local(name))
        {
            Some((_, old)) => *old = value,
            None => self.attrs.push((name.to_string(), value)),
        }
    }

    pub fn remove_attr(&mut self, name: &str) {
        self.attrs.retain(|(key, _)| local(key) != name);
    }

    /// The first child element with local name `name`.
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.elements().find(|element| element.local() == name)
    }

    pub fn child_mut(&mut self, name: &str) -> Option<&mut Element> {
        self.elements_mut().find(|element| element.local() == name)
    }

    /// The child `name`, made (as `prefix:name` after the existing children)
    /// when there is none.
    pub fn child_or_insert(&mut self, qualified: &str) -> &mut Element {
        let name = local(qualified).to_string();
        if self.child(&name).is_none() {
            self.children
                .push(Node::Element(Element::new(qualified.to_string())));
        }
        self.child_mut(&name).expect("just made")
    }

    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|node| match node {
            Node::Element(element) => Some(element),
            _ => None,
        })
    }

    pub fn elements_mut(&mut self) -> impl Iterator<Item = &mut Element> {
        self.children.iter_mut().filter_map(|node| match node {
            Node::Element(element) => Some(element),
            _ => None,
        })
    }

    /// Child elements with local name `name`.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> {
        self.elements()
            .filter(move |element| element.local() == name)
    }

    /// The first descendant (depth first) with local name `name`.
    pub fn descendant(&self, name: &str) -> Option<&Element> {
        for child in self.elements() {
            if child.local() == name {
                return Some(child);
            }
            if let Some(found) = child.descendant(name) {
                return Some(found);
            }
        }
        None
    }

    /// Every descendant with local name `name`, depth first.
    pub fn descendants<'a>(&'a self, name: &'a str, out: &mut Vec<&'a Element>) {
        for child in self.elements() {
            if child.local() == name {
                out.push(child);
            }
            child.descendants(name, out);
        }
    }

    /// The character data directly under this element.
    pub fn text(&self) -> String {
        self.children
            .iter()
            .filter_map(|node| match node {
                Node::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Removes the child elements with local name `name`.
    pub fn remove_children(&mut self, name: &str) {
        self.children
            .retain(|node| !matches!(node, Node::Element(element) if element.local() == name));
    }

    /// Writes this element and its subtree.
    pub fn write(&self, out: &mut String) {
        out.push('<');
        out.push_str(&self.name);
        for (key, value) in &self.attrs {
            out.push(' ');
            out.push_str(key);
            out.push_str("=\"");
            out.push_str(&escape(value));
            out.push('"');
        }
        if self.children.is_empty() {
            out.push_str("/>");
            return;
        }
        out.push('>');
        for child in &self.children {
            match child {
                Node::Element(element) => element.write(out),
                Node::Text(text) => out.push_str(&escape_text(text)),
                Node::Raw(raw) => out.push_str(raw),
            }
        }
        out.push_str("</");
        out.push_str(&self.name);
        out.push('>');
    }

    /// This element as markup.
    pub fn to_xml(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    /// This element as a whole part, with the XML declaration Office writes.
    pub fn to_document(&self) -> String {
        let mut out =
            String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
        self.write(&mut out);
        out
    }
}

/// Escapes an attribute value.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Escapes character data.
pub fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

fn read_start(start: &quick_xml::events::BytesStart<'_>) -> Result<Element, String> {
    let mut element = Element::new(String::from_utf8_lossy(start.name().as_ref()).into_owned());
    for attribute in start.attributes() {
        let attribute = attribute.map_err(|error| error.to_string())?;
        element.attrs.push((
            String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
            attribute
                .unescape_value()
                .map_err(|error| error.to_string())?
                .into_owned(),
        ));
    }
    Ok(element)
}

/// Parses `xml` into its root element.
pub fn parse(xml: &str) -> Result<Element, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut stack: Vec<Element> = Vec::new();
    fn push_text(stack: &mut [Element], text: String) {
        if let Some(parent) = stack.last_mut() {
            if let Some(Node::Text(last)) = parent.children.last_mut() {
                last.push_str(&text);
            } else {
                parent.children.push(Node::Text(text));
            }
        }
    }
    loop {
        match reader.read_event().map_err(|error| error.to_string())? {
            Event::Start(start) => stack.push(read_start(&start)?),
            Event::Empty(start) => {
                let element = read_start(&start)?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(Node::Element(element)),
                    None => return Ok(element),
                }
            }
            Event::End(_) => {
                let element = stack.pop().ok_or("unbalanced XML")?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(Node::Element(element)),
                    None => return Ok(element),
                }
            }
            Event::Text(text) => {
                let text = text.decode().map_err(|error| error.to_string())?;
                push_text(&mut stack, text.into_owned());
            }
            Event::GeneralRef(reference) => {
                let name = reference.decode().map_err(|error| error.to_string())?;
                let decoded = quick_xml::escape::unescape(&format!("&{name};"))
                    .map(|text| text.into_owned())
                    .unwrap_or_else(|_| format!("&{name};"));
                push_text(&mut stack, decoded);
            }
            Event::CData(data) => {
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(Node::Raw(format!(
                        "<![CDATA[{}]]>",
                        String::from_utf8_lossy(&data)
                    )));
                }
            }
            Event::Comment(comment) => {
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(Node::Raw(format!(
                        "<!--{}-->",
                        String::from_utf8_lossy(&comment)
                    )));
                }
            }
            Event::Eof => return Err("no root element".to_string()),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_back_what_it_read() {
        let xml = r#"<dgm:dataModel xmlns:dgm="d" xmlns:a="a"><dgm:ptLst><dgm:pt modelId="{1}" type="doc"><dgm:t><a:p><a:r><a:t> A &amp; B &lt;c&gt;</a:t></a:r></a:p></dgm:t></dgm:pt></dgm:ptLst><!-- kept --><x:y q="&quot;"/></dgm:dataModel>"#;
        let root = parse(xml).unwrap();
        assert_eq!(root.to_xml(), xml);
        let t = root.descendant("t").unwrap();
        assert_eq!(t.local(), "t");
    }
}
