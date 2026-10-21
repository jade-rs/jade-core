//! A small element tree over quick-xml, for the readers: documents are
//! walked as trees, which the streaming reader makes awkward.

use quick_xml::events::Event;
use quick_xml::Reader;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Element {
    /// The qualified name, prefix and all (`w:p`).
    pub name: String,
    pub attributes: Vec<(String, String)>,
    pub children: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Element(Element),
    Text(String),
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

impl Element {
    /// The name without its prefix.
    pub fn local(&self) -> &str {
        local(&self.name)
    }

    pub fn is(&self, local_name: &str) -> bool {
        self.local() == local_name
    }

    /// An attribute by its qualified name, or failing that by its local
    /// name — documents written with other prefixes still read.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .or_else(|| {
                let wanted = local(name);
                self.attributes.iter().find(|(key, _)| local(key) == wanted)
            })
            .map(|(_, value)| value.as_str())
    }

    /// The child elements.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|node| match node {
            Node::Element(element) => Some(element),
            Node::Text(_) => None,
        })
    }

    /// The first child element with this local name.
    pub fn child(&self, local_name: &str) -> Option<&Element> {
        self.elements().find(|element| element.is(local_name))
    }

    pub fn children_named<'a>(
        &'a self,
        local_name: &'a str,
    ) -> impl Iterator<Item = &'a Element> + 'a {
        self.elements()
            .filter(move |element| element.is(local_name))
    }

    /// The first descendant (depth first) with this local name.
    pub fn find(&self, local_name: &str) -> Option<&Element> {
        for element in self.elements() {
            if element.is(local_name) {
                return Some(element);
            }
            if let Some(found) = element.find(local_name) {
                return Some(found);
            }
        }
        None
    }

    /// Every descendant with this local name, in document order.
    pub fn find_all<'a>(&'a self, local_name: &str, out: &mut Vec<&'a Element>) {
        for element in self.elements() {
            if element.is(local_name) {
                out.push(element);
            }
            element.find_all(local_name, out);
        }
    }

    /// The text of the element and everything in it.
    pub fn text(&self) -> String {
        let mut out = String::new();
        self.push_text(&mut out);
        out
    }

    fn push_text(&self, out: &mut String) {
        for node in &self.children {
            match node {
                Node::Text(text) => out.push_str(text),
                Node::Element(element) => element.push_text(out),
            }
        }
    }

    /// `w:val` of the named child, which is how most WordprocessingML
    /// properties say what they are.
    pub fn child_val(&self, local_name: &str) -> Option<&str> {
        self.child(local_name).and_then(|child| child.attr("w:val"))
    }

    /// A WordprocessingML on/off property: present and not `0`/`false`/`off`.
    pub fn flag(&self, local_name: &str) -> Option<bool> {
        let child = self.child(local_name)?;
        Some(!matches!(
            child.attr("w:val"),
            Some("0") | Some("false") | Some("off") | Some("none")
        ))
    }
}

/// Parses `xml` into its root element. Text outside elements, comments and
/// processing instructions are dropped.
pub fn parse(xml: &str) -> Result<Element, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut stack: Vec<Element> = vec![Element::default()];
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => {
                stack.push(element_of(&start));
            }
            Ok(Event::Empty(start)) => {
                let element = element_of(&start);
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(Node::Element(element));
                }
            }
            Ok(Event::End(_)) => {
                if stack.len() > 1 {
                    let element = stack.pop().expect("checked");
                    if let Some(parent) = stack.last_mut() {
                        parent.children.push(Node::Element(element));
                    }
                }
            }
            Ok(Event::Text(text)) => {
                let decoded = text
                    .decode()
                    .map(|text| text.into_owned())
                    .unwrap_or_default();
                let text = quick_xml::escape::unescape(&decoded)
                    .map(|text| text.into_owned())
                    .unwrap_or(decoded);
                push_text(&mut stack, &text);
            }
            Ok(Event::CData(data)) => {
                let text = String::from_utf8_lossy(&data).into_owned();
                push_text(&mut stack, &text);
            }
            Ok(Event::GeneralRef(reference)) => {
                let name = reference
                    .decode()
                    .map(|name| name.into_owned())
                    .unwrap_or_default();
                let text = quick_xml::escape::unescape(&format!("&{name};"))
                    .map(|text| text.into_owned())
                    .unwrap_or_default();
                push_text(&mut stack, &text);
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(format!(
                    "XML error at {}: {error}",
                    reader.buffer_position()
                ))
            }
            _ => {}
        }
    }
    // An unclosed element still counts: what was read is kept.
    while stack.len() > 1 {
        let element = stack.pop().expect("checked");
        if let Some(parent) = stack.last_mut() {
            parent.children.push(Node::Element(element));
        }
    }
    let root = stack.pop().unwrap_or_default();
    root.children
        .into_iter()
        .find_map(|node| match node {
            Node::Element(element) => Some(element),
            Node::Text(_) => None,
        })
        .ok_or_else(|| "the XML has no root element".to_string())
}

fn push_text(stack: &mut [Element], text: &str) {
    if stack.len() < 2 || text.is_empty() {
        return;
    }
    if let Some(parent) = stack.last_mut() {
        if let Some(Node::Text(last)) = parent.children.last_mut() {
            last.push_str(text);
        } else {
            parent.children.push(Node::Text(text.to_string()));
        }
    }
}

fn element_of(start: &quick_xml::events::BytesStart<'_>) -> Element {
    let name = String::from_utf8_lossy(start.name().as_ref()).into_owned();
    let attributes = start
        .attributes()
        .with_checks(false)
        .flatten()
        .map(|attribute| {
            let key = String::from_utf8_lossy(attribute.key.as_ref()).into_owned();
            let raw = String::from_utf8_lossy(&attribute.value).into_owned();
            let value = quick_xml::escape::unescape(&raw)
                .map(|value| value.into_owned())
                .unwrap_or(raw);
            (key, value)
        })
        .collect();
    Element {
        name,
        attributes,
        children: Vec::new(),
    }
}

/// `text` escaped for an XML attribute or element.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Characters XML 1.0 cannot carry at all.
            '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' => {}
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_keeps_names_attributes_and_text() {
        let root = parse(
            r#"<w:document xmlns:w="x"><w:body><w:p><w:r><w:t xml:space="preserve">A &amp; B</w:t></w:r></w:p></w:body></w:document>"#,
        )
        .unwrap();
        assert_eq!(root.local(), "document");
        let t = root.find("t").unwrap();
        assert_eq!(t.text(), "A & B");
        assert_eq!(t.attr("xml:space"), Some("preserve"));
        assert_eq!(t.attr("space"), Some("preserve"));
    }

    #[test]
    fn an_unclosed_tree_keeps_what_was_read() {
        let root = parse("<a><b>one</b><c>two").unwrap();
        assert_eq!(root.text(), "onetwo");
    }

    #[test]
    fn escaping_drops_what_xml_cannot_carry() {
        assert_eq!(escape("a<b>&\u{1}\"'"), "a&lt;b&gt;&amp;&quot;&apos;");
    }
}
