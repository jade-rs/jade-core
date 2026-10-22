//! A small XML tree for rewriting a package part — `styles.xml`,
//! `numbering.xml`, the theme, the settings — where an edit reaches past what
//! the engine's commands do.
//!
//! What it does not touch it writes back as it read it: text, attribute
//! values, comments and processing instructions keep their escaped source
//! form, and the declaration is kept, so a part survives a round trip
//! byte for byte apart from how empty elements are spelled.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

/// A node of the tree.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Element(Element),
    /// Text as it was escaped in the source.
    Text(String),
    /// Anything else — a comment, a CDATA section, a processing instruction —
    /// written back verbatim.
    Raw(String),
}

/// An element: its qualified name, its attributes (values escaped as in the
/// source) and its children.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Element {
    pub name: String,
    pub attributes: Vec<(String, String)>,
    pub children: Vec<Node>,
}

/// A parsed part: its prolog (the declaration and anything before the root)
/// and its root element.
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    pub prolog: String,
    pub root: Element,
}

/// `&`, `<`, `>` and `"` escaped, for attribute values and text.
pub fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
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

/// The text an escaped value stands for.
pub fn unescape(value: &str) -> String {
    quick_xml::escape::unescape(value)
        .map(|value| value.into_owned())
        .unwrap_or_else(|_| value.to_string())
}

fn start_element(start: &BytesStart<'_>) -> Result<Element, String> {
    let name = String::from_utf8_lossy(start.name().as_ref()).into_owned();
    let mut attributes = Vec::new();
    for attribute in start.attributes().with_checks(false) {
        let attribute = attribute.map_err(|error| error.to_string())?;
        attributes.push((
            String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
            String::from_utf8_lossy(&attribute.value).into_owned(),
        ));
    }
    Ok(Element {
        name,
        attributes,
        children: Vec::new(),
    })
}

impl Document {
    pub fn parse(xml: &str) -> Result<Self, String> {
        let mut reader = Reader::from_str(xml);
        let config = reader.config_mut();
        config.trim_text(false);
        config.expand_empty_elements = false;
        let mut prolog = String::new();
        let mut stack: Vec<Element> = Vec::new();
        let mut root: Option<Element> = None;
        loop {
            let event = reader.read_event().map_err(|error| error.to_string())?;
            let node = match event {
                Event::Start(start) => {
                    stack.push(start_element(&start)?);
                    continue;
                }
                Event::Empty(start) => Node::Element(start_element(&start)?),
                Event::End(_) => {
                    let element = stack.pop().ok_or("unbalanced XML")?;
                    Node::Element(element)
                }
                Event::Text(text) => {
                    Node::Text(String::from_utf8_lossy(text.as_ref()).into_owned())
                }
                Event::GeneralRef(reference) => {
                    Node::Text(format!("&{};", String::from_utf8_lossy(reference.as_ref())))
                }
                Event::CData(data) => Node::Raw(format!(
                    "<![CDATA[{}]]>",
                    String::from_utf8_lossy(data.as_ref())
                )),
                Event::Comment(comment) => Node::Raw(format!(
                    "<!--{}-->",
                    String::from_utf8_lossy(comment.as_ref())
                )),
                Event::Decl(decl) => {
                    prolog.push_str(&format!("<?{}?>", String::from_utf8_lossy(decl.as_ref())));
                    continue;
                }
                Event::PI(pi) => Node::Raw(format!("<?{}?>", String::from_utf8_lossy(pi.as_ref()))),
                Event::DocType(doctype) => Node::Raw(format!(
                    "<!DOCTYPE {}>",
                    String::from_utf8_lossy(doctype.as_ref())
                )),
                Event::Eof => break,
            };
            match stack.last_mut() {
                Some(parent) => {
                    // Text between elements is kept, so pretty-printed parts
                    // stay pretty-printed.
                    parent.children.push(node);
                }
                None => match node {
                    Node::Element(element) => root = Some(element),
                    Node::Text(text) => prolog.push_str(&text),
                    Node::Raw(raw) => prolog.push_str(&raw),
                },
            }
        }
        Ok(Document {
            prolog,
            root: root.ok_or("the part has no root element")?,
        })
    }

    pub fn to_xml(&self) -> String {
        let mut out = self.prolog.clone();
        if out.is_empty() {
            out.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
        }
        self.root.write(&mut out);
        out
    }
}

impl Element {
    pub fn new(name: &str) -> Self {
        Element {
            name: name.to_string(),
            ..Element::default()
        }
    }

    /// An element with one attribute, `w:val` most often.
    pub fn with_val(name: &str, value: &str) -> Self {
        Element::new(name).attr("w:val", value)
    }

    /// Adds or replaces an attribute (`value` unescaped).
    pub fn attr(mut self, name: &str, value: &str) -> Self {
        self.set_attr(name, value);
        self
    }

    pub fn child(mut self, child: Element) -> Self {
        self.children.push(Node::Element(child));
        self
    }

    pub fn text(mut self, text: &str) -> Self {
        self.children.push(Node::Text(escape(text)));
        self
    }

    /// An attribute's value, unescaped.
    pub fn get(&self, name: &str) -> Option<String> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| unescape(value))
    }

    pub fn set_attr(&mut self, name: &str, value: &str) {
        let value = escape(value);
        match self.attributes.iter_mut().find(|(key, _)| key == name) {
            Some(entry) => entry.1 = value,
            None => self.attributes.push((name.to_string(), value)),
        }
    }

    pub fn remove_attr(&mut self, name: &str) {
        self.attributes.retain(|(key, _)| key != name);
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

    pub fn find(&self, name: &str) -> Option<&Element> {
        self.elements().find(|element| element.name == name)
    }

    pub fn find_mut(&mut self, name: &str) -> Option<&mut Element> {
        self.elements_mut().find(|element| element.name == name)
    }

    /// The `w:val` of child `name`.
    pub fn child_val(&self, name: &str) -> Option<String> {
        self.find(name).and_then(|child| child.get("w:val"))
    }

    /// Child `name`, added (where `order` puts it) when missing.
    pub fn ensure(&mut self, name: &str, order: &[&str]) -> &mut Element {
        if self.find(name).is_none() {
            self.insert_ordered(Element::new(name), order);
        }
        self.find_mut(name).expect("just inserted")
    }

    /// Removes every child `name`.
    pub fn remove(&mut self, name: &str) {
        self.children
            .retain(|node| !matches!(node, Node::Element(element) if element.name == name));
    }

    /// Replaces child `element.name`, or inserts it where `order` (the schema
    /// sequence) puts it: before the first child that comes later.
    pub fn put(&mut self, element: Element, order: &[&str]) {
        let name = element.name.clone();
        if let Some(index) = self
            .children
            .iter()
            .position(|node| matches!(node, Node::Element(child) if child.name == name))
        {
            self.children[index] = Node::Element(element);
            self.children.retain({
                let mut seen = false;
                move |node| match node {
                    Node::Element(child) if child.name == name => {
                        let keep = !seen;
                        seen = true;
                        keep
                    }
                    _ => true,
                }
            });
            return;
        }
        self.insert_ordered(element, order);
    }

    /// Inserts `element` before the first child that `order` ranks after
    /// it; at the end when `order` does not know it or nothing follows.
    pub fn insert_ordered(&mut self, element: Element, order: &[&str]) {
        let rank = |name: &str| order.iter().position(|entry| *entry == name);
        let index = rank(&element.name).and_then(|own| {
            self.children.iter().position(|node| match node {
                Node::Element(child) => rank(&child.name).is_some_and(|other| other > own),
                _ => false,
            })
        });
        match index {
            Some(index) => self.children.insert(index, Node::Element(element)),
            None => self.children.push(Node::Element(element)),
        }
    }

    /// Sets an on/off child: present for `true`, absent for `false`.
    pub fn set_flag(&mut self, name: &str, on: bool, order: &[&str]) {
        if on {
            if self.find(name).is_none() {
                self.insert_ordered(Element::new(name), order);
            }
        } else {
            self.remove(name);
        }
    }

    fn write(&self, out: &mut String) {
        out.push('<');
        out.push_str(&self.name);
        for (key, value) in &self.attributes {
            out.push(' ');
            out.push_str(key);
            out.push_str("=\"");
            out.push_str(value);
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
                Node::Text(text) | Node::Raw(text) => out.push_str(text),
            }
        }
        out.push_str("</");
        out.push_str(&self.name);
        out.push('>');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_round_trips() {
        let source = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="urn:w" xmlns:mc="urn:mc" mc:Ignorable="w14"><!-- c --><w:style w:type="paragraph" w:styleId="A&amp;B"><w:name w:val="a &lt; b"/><w:rPr><w:b/></w:rPr></w:style>
  <w:t xml:space="preserve"> x &amp; y </w:t></w:styles>"#;
        let document = Document::parse(source).unwrap();
        assert_eq!(document.to_xml(), source);
        let style = document.root.find("w:style").unwrap();
        assert_eq!(style.get("w:styleId").as_deref(), Some("A&B"));
        assert_eq!(style.child_val("w:name").as_deref(), Some("a < b"));
    }

    #[test]
    fn children_go_where_the_schema_puts_them() {
        let order = ["w:a", "w:b", "w:c"];
        let mut element = Element::new("w:x").child(Element::new("w:c"));
        element.put(Element::with_val("w:a", "1"), &order);
        element.put(Element::new("w:b"), &order);
        element.put(Element::with_val("w:a", "2"), &order);
        let names: Vec<_> = element.elements().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["w:a", "w:b", "w:c"]);
        assert_eq!(element.child_val("w:a").as_deref(), Some("2"));
    }
}
