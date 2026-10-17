//! A lightweight owned XML tree.
//!
//! Slide parts are small (kilobytes), and the slide grammar nests deeply
//! enough — a colour lives at `spPr/solidFill/srgbClr` and again at
//! `txBody/p/r/rPr/solidFill/srgbClr` — that a streaming parser would need a
//! hand-written state stack for every query. Building the tree once and
//! walking it keeps each query one line.

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XmlElement {
    /// Local name, prefix stripped: `a:off` is stored as `off`.
    pub name: String,
    /// Attributes with local names: `r:embed` is stored as `embed`.
    pub attrs: Vec<(String, String)>,
    pub children: Vec<XmlElement>,
    /// Concatenated character data of direct text children.
    pub text: String,
}

impl XmlElement {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn child(&self, name: &str) -> Option<&XmlElement> {
        self.children.iter().find(|child| child.name == name)
    }

    /// Depth-first search for the first descendant with `name`.
    pub fn descendant(&self, name: &str) -> Option<&XmlElement> {
        for child in &self.children {
            if child.name == name {
                return Some(child);
            }
            if let Some(found) = child.descendant(name) {
                return Some(found);
            }
        }
        None
    }
}

fn local(name: &[u8]) -> String {
    String::from_utf8_lossy(jade_opc::local_name(name)).into_owned()
}

/// Parses `xml` into its root element.
pub(crate) fn parse_tree(xml: &str) -> Result<XmlElement> {
    let mut reader = Reader::from_str(xml);
    // Text is kept verbatim: a run's `<a:t> of many</a:t>` starts with a
    // space that matters. Indentation between elements lands on container
    // elements, whose text nothing reads.
    reader.config_mut().trim_text(false);
    let mut stack: Vec<XmlElement> = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(ref element) => {
                stack.push(read_element(element)?);
            }
            Event::Empty(ref element) => {
                let node = read_element(element)?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => return Ok(node),
                }
            }
            Event::End(_) => {
                let node = stack.pop().expect("balanced XML");
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => return Ok(node),
                }
            }
            Event::Text(ref text) => {
                if let Some(parent) = stack.last_mut() {
                    parent.text.push_str(&text.decode()?);
                }
            }
            Event::CData(ref text) => {
                if let Some(parent) = stack.last_mut() {
                    parent.text.push_str(&text.decode()?);
                }
            }
            // `&amp;` and friends arrive as their own event, not as text.
            Event::GeneralRef(ref reference) => {
                if let Some(parent) = stack.last_mut() {
                    let name = reference.decode()?;
                    if let Ok(decoded) = quick_xml::escape::unescape(&format!("&{name};")) {
                        parent.text.push_str(&decoded);
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Err(crate::PptxError::Malformed("no root element".to_string()))
}

fn read_element(element: &quick_xml::events::BytesStart<'_>) -> Result<XmlElement> {
    let mut attrs = Vec::new();
    for attribute in element.attributes() {
        let attribute = attribute?;
        attrs.push((
            local(attribute.key.as_ref()),
            attribute.unescape_value()?.into_owned(),
        ));
    }
    Ok(XmlElement {
        name: local(element.name().as_ref()),
        attrs,
        children: Vec::new(),
        text: String::new(),
    })
}
