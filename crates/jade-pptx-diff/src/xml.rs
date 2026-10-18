//! OOXML を扱うための最小の DOM。
//!
//! スライド 1 枚の XML は小さいので、イベントを追いながら状態機械を書くより
//! いったん木にしてから引くほうが読みやすく間違えにくい。名前空間は
//! prefix を落としたローカル名だけで扱う (`p:sp` → `sp`)。pptx の要素名は
//! 名前空間をまたいでも衝突しないので、これで十分。

use anyhow::{Context, Result};
use quick_xml::events::Event;
use quick_xml::Reader;

#[derive(Debug, Clone, Default)]
pub struct Node {
    /// prefix を除いたローカル名。
    pub name: String,
    /// 属性。キーは prefix を含んだまま持つ (`r:id` と `id` は別物なので)。
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
    /// 直下のテキスト。
    pub text: String,
}

impl Node {
    /// 属性を引く。完全一致を優先し、無ければ prefix を無視して探す。
    pub fn attr(&self, name: &str) -> Option<&str> {
        if let Some((_, value)) = self.attrs.iter().find(|(key, _)| key == name) {
            return Some(value.as_str());
        }
        let wanted = name.rsplit(':').next().unwrap_or(name);
        self.attrs
            .iter()
            .find(|(key, _)| key.rsplit(':').next().unwrap_or(key) == wanted)
            .map(|(_, value)| value.as_str())
    }

    pub fn attr_i64(&self, name: &str) -> Option<i64> {
        self.attr(name).and_then(|value| value.parse().ok())
    }

    /// 直下の子から最初の一致を返す。
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|node| node.name == name)
    }

    /// 直下の子のうち一致するものすべて。
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |node| node.name == name)
    }

    /// パスで潜る (`shape.path(&["spPr", "xfrm", "off"])`)。
    pub fn path(&self, names: &[&str]) -> Option<&Node> {
        let mut current = self;
        for name in names {
            current = current.child(name)?;
        }
        Some(current)
    }

    /// 子孫を先行順に走査する。
    pub fn descendants(&self) -> Vec<&Node> {
        let mut out = Vec::new();
        let mut stack: Vec<&Node> = vec![self];
        while let Some(node) = stack.pop() {
            out.push(node);
            for child in node.children.iter().rev() {
                stack.push(child);
            }
        }
        out
    }

    /// 子孫すべてのテキストを連結する。
    pub fn text_content(&self) -> String {
        let mut out = String::new();
        for node in self.descendants() {
            out.push_str(&node.text);
        }
        out
    }
}

fn local_name(raw: &[u8]) -> String {
    let name = String::from_utf8_lossy(raw);
    match name.split_once(':') {
        Some((_, local)) => local.to_string(),
        None => name.to_string(),
    }
}

/// XML を読んで木にする。ルート要素を返す。
pub fn parse(bytes: &[u8]) -> Result<Node> {
    let text = String::from_utf8_lossy(bytes);
    let mut reader = Reader::from_str(&text);
    reader.config_mut().trim_text(false);

    let mut stack: Vec<Node> = vec![Node::default()];
    loop {
        match reader.read_event().context("failed to read xml event")? {
            Event::Start(start) => {
                let mut node = Node {
                    name: local_name(start.name().as_ref()),
                    ..Default::default()
                };
                for attr in start.attributes().flatten() {
                    node.attrs.push((
                        String::from_utf8_lossy(attr.key.as_ref()).into_owned(),
                        attr.unescape_value().unwrap_or_default().into_owned(),
                    ));
                }
                stack.push(node);
            }
            Event::Empty(start) => {
                let mut node = Node {
                    name: local_name(start.name().as_ref()),
                    ..Default::default()
                };
                for attr in start.attributes().flatten() {
                    node.attrs.push((
                        String::from_utf8_lossy(attr.key.as_ref()).into_owned(),
                        attr.unescape_value().unwrap_or_default().into_owned(),
                    ));
                }
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                }
            }
            Event::End(_) => {
                if stack.len() > 1 {
                    let node = stack.pop().expect("stack depth checked");
                    if let Some(parent) = stack.last_mut() {
                        parent.children.push(node);
                    }
                }
            }
            Event::Text(text) => {
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&text.xml_content().unwrap_or_default());
                }
            }
            Event::CData(data) => {
                if let Some(node) = stack.last_mut() {
                    node.text
                        .push_str(&String::from_utf8_lossy(data.into_inner().as_ref()));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    let root = stack.remove(0);
    root.children
        .into_iter()
        .next()
        .context("xml document has no root element")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_tree_with_attributes_and_text() {
        let node =
            parse(br#"<p:sld xmlns:p="x"><p:sp id="2"><a:t>hello</a:t></p:sp></p:sld>"#).unwrap();
        assert_eq!(node.name, "sld");
        let sp = node.child("sp").unwrap();
        assert_eq!(sp.attr("id"), Some("2"));
        assert_eq!(sp.text_content(), "hello");
    }

    #[test]
    fn keeps_empty_elements() {
        let node = parse(br#"<root><off x="1" y="2"/><ext cx="3" cy="4"/></root>"#).unwrap();
        assert_eq!(node.child("off").unwrap().attr_i64("x"), Some(1));
        assert_eq!(node.child("ext").unwrap().attr_i64("cy"), Some(4));
    }

    #[test]
    fn path_walks_nested_elements() {
        let node = parse(br#"<sp><spPr><xfrm><off x="5"/></xfrm></spPr></sp>"#).unwrap();
        assert_eq!(
            node.path(&["spPr", "xfrm", "off"]).unwrap().attr_i64("x"),
            Some(5)
        );
    }
}
