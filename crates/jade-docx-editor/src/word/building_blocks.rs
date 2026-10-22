//! Building blocks (クイック パーツ and 定型句, AutoText): the selection saved
//! under a name into the document's glossary part (`word/glossary/
//! document.xml`, ECMA-376 Part 1 §17.12), listed by gallery, inserted at the
//! caret with its formatting, and removed.
//!
//! An entry keeps each paragraph's style and alignment and each run's
//! character formatting, as a copy does (see `clipboard`); one Word made
//! reads back the same way.

use super::clipboard::{ClipParagraph, ClipRun, RichClip, WordClipboard};
use super::package::{edit_package, Package, PartKind};
use super::xml::{escape, Document, Element, Node};
use super::{Position, WordSession};
use jade_docx_headless_api::HeadlessRunStyle;

/// Which gallery an entry is in (`w:gallery`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gallery {
    /// クイック パーツ (`quickParts`).
    QuickParts,
    /// 定型句 (`autoTxt`).
    AutoText,
}

impl Gallery {
    fn token(self) -> &'static str {
        match self {
            Gallery::QuickParts => "quickParts",
            Gallery::AutoText => "autoTxt",
        }
    }

    fn from_token(token: &str) -> Option<Gallery> {
        match token {
            "quickParts" => Some(Gallery::QuickParts),
            "autoTxt" => Some(Gallery::AutoText),
            _ => None,
        }
    }
}

/// One entry of the glossary.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildingBlock {
    pub name: String,
    pub gallery: Gallery,
    pub category: String,
    pub content: RichClip,
}

fn run_properties(style: &HeadlessRunStyle) -> Option<Element> {
    let mut rpr = Element::new("w:rPr");
    if let Some(family) = style.font_family.as_deref() {
        rpr.children.push(Node::Element(
            Element::new("w:rFonts")
                .attr("w:ascii", family)
                .attr("w:eastAsia", family)
                .attr("w:hAnsi", family),
        ));
    }
    for (on, tag) in [
        (style.bold, "w:b"),
        (style.italic, "w:i"),
        (style.strike, "w:strike"),
    ] {
        if on == Some(true) {
            rpr.children.push(Node::Element(Element::new(tag)));
        }
    }
    if let Some(color) = style
        .color
        .as_deref()
        .map(|color| color.trim_start_matches('#'))
        .filter(|color| color.len() == 6)
    {
        rpr.children
            .push(Node::Element(Element::with_val("w:color", color)));
    }
    if let Some(size) = style.font_size {
        let half_points = ((size * 2.0).round() as i64).to_string();
        rpr.children
            .push(Node::Element(Element::with_val("w:sz", &half_points)));
    }
    if style.underline == Some(true) {
        rpr.children
            .push(Node::Element(Element::with_val("w:u", "single")));
    }
    if let Some(baseline) = style.baseline.as_deref() {
        rpr.children
            .push(Node::Element(Element::with_val("w:vertAlign", baseline)));
    }
    (!rpr.children.is_empty()).then_some(rpr)
}

/// The `w:p` elements of a copied selection.
fn paragraphs_xml(clip: &RichClip) -> Vec<Element> {
    clip.paragraphs
        .iter()
        .map(|paragraph| {
            let mut p = Element::new("w:p");
            let mut ppr = Element::new("w:pPr");
            if let Some(style) = paragraph.style_id.as_deref() {
                ppr.children
                    .push(Node::Element(Element::with_val("w:pStyle", style)));
            }
            if let Some(alignment) = paragraph.alignment.as_deref() {
                ppr.children
                    .push(Node::Element(Element::with_val("w:jc", alignment)));
            }
            if !ppr.children.is_empty() {
                p.children.push(Node::Element(ppr));
            }
            for run in &paragraph.runs {
                let mut r = Element::new("w:r");
                if let Some(rpr) = run_properties(&run.style) {
                    r.children.push(Node::Element(rpr));
                }
                for (index, piece) in run.text.split('\t').enumerate() {
                    if index > 0 {
                        r.children.push(Node::Element(Element::new("w:tab")));
                    }
                    if !piece.is_empty() {
                        let mut t = Element::new("w:t").attr("xml:space", "preserve");
                        t.children.push(Node::Text(escape(piece)));
                        r.children.push(Node::Element(t));
                    }
                }
                p.children.push(Node::Element(r));
            }
            p
        })
        .collect()
}

fn on(element: Option<&Element>) -> Option<bool> {
    element.map(|element| !matches!(element.get("w:val").as_deref(), Some("0" | "false" | "off")))
}

/// A glossary entry's paragraphs back as a copy.
fn clip_of(body: &Element) -> RichClip {
    let mut paragraphs = Vec::new();
    for p in body.elements().filter(|element| element.name == "w:p") {
        let ppr = p.find("w:pPr");
        let mut paragraph = ClipParagraph {
            style_id: ppr.and_then(|ppr| ppr.child_val("w:pStyle")),
            alignment: ppr.and_then(|ppr| ppr.child_val("w:jc")),
            runs: Vec::new(),
        };
        for r in p.elements().filter(|element| element.name == "w:r") {
            let rpr = r.find("w:rPr");
            let style = HeadlessRunStyle {
                bold: rpr.and_then(|rpr| on(rpr.find("w:b"))),
                italic: rpr.and_then(|rpr| on(rpr.find("w:i"))),
                underline: rpr.and_then(|rpr| rpr.child_val("w:u").map(|value| value != "none")),
                strike: rpr.and_then(|rpr| on(rpr.find("w:strike"))),
                baseline: rpr.and_then(|rpr| rpr.child_val("w:vertAlign")),
                font_size: rpr
                    .and_then(|rpr| rpr.child_val("w:sz"))
                    .and_then(|value| value.parse::<f32>().ok())
                    .map(|half_points| half_points / 2.0),
                font_family: rpr
                    .and_then(|rpr| rpr.find("w:rFonts"))
                    .and_then(|fonts| fonts.get("w:eastAsia").or_else(|| fonts.get("w:ascii"))),
                color: rpr
                    .and_then(|rpr| rpr.child_val("w:color"))
                    .filter(|color| !color.eq_ignore_ascii_case("auto"))
                    .map(|color| format!("#{color}")),
            };
            let mut text = String::new();
            for child in r.elements() {
                match child.name.as_str() {
                    "w:t" => {
                        for node in &child.children {
                            if let Node::Text(value) = node {
                                text.push_str(&super::xml::unescape(value));
                            }
                        }
                    }
                    "w:tab" => text.push('\t'),
                    "w:br" | "w:cr" => text.push('\n'),
                    _ => {}
                }
            }
            if text.is_empty() {
                continue;
            }
            match paragraph.runs.last_mut() {
                Some(last) if last.style == style => last.text.push_str(&text),
                _ => paragraph.runs.push(ClipRun { text, style }),
            }
        }
        paragraphs.push(paragraph);
    }
    RichClip { paragraphs }
}

/// The entries of a glossary part.
pub fn blocks_of(glossary_xml: &str) -> Vec<BuildingBlock> {
    let Ok(document) = Document::parse(glossary_xml) else {
        return Vec::new();
    };
    let Some(parts) = document.root.find("w:docParts") else {
        return Vec::new();
    };
    parts
        .elements()
        .filter(|element| element.name == "w:docPart")
        .filter_map(|part| {
            let properties = part.find("w:docPartPr")?;
            let name = properties.child_val("w:name")?;
            let category = properties.find("w:category");
            let gallery = category
                .and_then(|category| category.child_val("w:gallery"))
                .and_then(|token| Gallery::from_token(&token))?;
            Some(BuildingBlock {
                name,
                gallery,
                category: category
                    .and_then(|category| category.child_val("w:name"))
                    .unwrap_or_default(),
                content: part.find("w:docPartBody").map(clip_of).unwrap_or_default(),
            })
        })
        .collect()
}

/// A GUID-shaped id for a new entry, from its name and what is there.
fn guid(name: &str, count: usize) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.bytes().chain(count.to_le_bytes()) {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    let low = hash.rotate_left(17) ^ 0x5bd1_e995;
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:04X}-{:012X}}}",
        hash >> 32,
        (hash >> 16) & 0xffff,
        hash & 0xffff,
        low >> 48,
        low & 0xffff_ffff_ffff
    )
}

/// Adds an entry (replacing one of the same name in the same gallery).
pub fn add_block(glossary: &mut Element, block: &BuildingBlock) {
    let parts = glossary.ensure("w:docParts", &["w:background", "w:docParts"]);
    parts.children.retain(|node| {
        let Node::Element(part) = node else {
            return true;
        };
        let properties = part.find("w:docPartPr");
        let same_name =
            properties.and_then(|p| p.child_val("w:name")).as_deref() == Some(block.name.as_str());
        let same_gallery = properties
            .and_then(|p| p.find("w:category"))
            .and_then(|category| category.child_val("w:gallery"))
            .as_deref()
            == Some(block.gallery.token());
        !(same_name && same_gallery)
    });
    let count = parts.elements().count();
    let properties = Element::new("w:docPartPr")
        .child(Element::with_val("w:name", &block.name))
        .child(
            Element::new("w:category")
                .child(Element::with_val("w:name", &block.category))
                .child(Element::with_val("w:gallery", block.gallery.token())),
        )
        .child(Element::new("w:behaviors").child(Element::with_val("w:behavior", "content")))
        .child(Element::with_val("w:guid", &guid(&block.name, count)));
    let mut body = Element::new("w:docPartBody");
    for paragraph in paragraphs_xml(&block.content) {
        body.children.push(Node::Element(paragraph));
    }
    parts.children.push(Node::Element(
        Element::new("w:docPart").child(properties).child(body),
    ));
}

/// Removes entry `index` (in [`blocks_of`]'s order).
pub fn remove_block(glossary: &mut Element, index: usize) -> bool {
    let Some(parts) = glossary.find_mut("w:docParts") else {
        return false;
    };
    let mut seen = 0usize;
    let mut removed = false;
    parts.children.retain(|node| {
        let Node::Element(part) = node else {
            return true;
        };
        if part.name != "w:docPart" {
            return true;
        }
        let listed = part
            .find("w:docPartPr")
            .and_then(|properties| properties.find("w:category"))
            .and_then(|category| category.child_val("w:gallery"))
            .and_then(|token| Gallery::from_token(&token))
            .is_some()
            && part
                .find("w:docPartPr")
                .and_then(|properties| properties.child_val("w:name"))
                .is_some();
        if !listed {
            return true;
        }
        let keep = seen != index;
        seen += 1;
        removed |= !keep;
        keep
    });
    removed
}

/// The building block commands, for every session.
pub trait WordBuildingBlocks: WordSession {
    fn building_blocks(&self) -> Vec<BuildingBlock> {
        self.word_document()
            .related_part_xml("glossaryDocument")
            .map(|xml| blocks_of(&xml))
            .unwrap_or_default()
    }

    /// 選択範囲をクイック パーツ ギャラリーに保存 / 定型句に保存.
    fn save_building_block(&mut self, name: &str, gallery: Gallery) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("a building block needs a name".to_string());
        }
        let content = WordClipboard::copy_rich(self).ok_or("select what to save")?;
        if content.plain_text().trim().is_empty() {
            return Err("select what to save".to_string());
        }
        let block = BuildingBlock {
            name: name.to_string(),
            gallery,
            category: "全般".to_string(),
            content,
        };
        edit_package(
            self,
            move |package: &mut Package| {
                package.edit(PartKind::Glossary, |glossary| {
                    add_block(glossary, &block);
                    Ok(())
                })
            },
            Vec::new(),
        )
    }

    /// Puts entry `index` in at the caret (over the selection), with its
    /// formatting. Returns where the caret goes.
    fn insert_building_block(&mut self, index: usize) -> Result<Position, String> {
        let block = self
            .building_blocks()
            .into_iter()
            .nth(index)
            .ok_or("no such building block")?;
        WordClipboard::paste_rich(self, &block.content)
    }

    fn delete_building_block(&mut self, index: usize) -> Result<(), String> {
        edit_package(
            self,
            move |package: &mut Package| {
                package.edit(PartKind::Glossary, |glossary| {
                    if remove_block(glossary, index) {
                        Ok(())
                    } else {
                        Err("no such building block".to_string())
                    }
                })
            },
            Vec::new(),
        )
    }
}

impl<T: WordSession + ?Sized> WordBuildingBlocks for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_round_trip_through_the_glossary() {
        let mut glossary = Document::parse(&format!(
            r#"<w:glossaryDocument xmlns:w="{}"><w:docParts/></w:glossaryDocument>"#,
            super::super::package::W_NS
        ))
        .unwrap()
        .root;
        let block = BuildingBlock {
            name: "署名".into(),
            gallery: Gallery::AutoText,
            category: "全般".into(),
            content: RichClip {
                paragraphs: vec![ClipParagraph {
                    style_id: Some("Signature".into()),
                    alignment: Some("right".into()),
                    runs: vec![
                        ClipRun {
                            text: "山田\t太郎".into(),
                            style: HeadlessRunStyle {
                                bold: Some(true),
                                font_size: Some(12.0),
                                color: Some("#C00000".into()),
                                ..HeadlessRunStyle::default()
                            },
                        },
                        ClipRun {
                            text: " & co".into(),
                            style: HeadlessRunStyle::default(),
                        },
                    ],
                }],
            },
        };
        add_block(&mut glossary, &block);
        add_block(&mut glossary, &block);
        let xml = Document {
            prolog: String::new(),
            root: glossary.clone(),
        }
        .to_xml();
        let blocks = blocks_of(&xml);
        assert_eq!(blocks.len(), 1, "a name is saved once per gallery");
        assert_eq!(blocks[0].name, "署名");
        assert_eq!(blocks[0].gallery, Gallery::AutoText);
        let runs = &blocks[0].content.paragraphs[0].runs;
        assert_eq!(runs[0].text, "山田\t太郎");
        assert_eq!(runs[0].style.bold, Some(true));
        assert_eq!(runs[0].style.color.as_deref(), Some("#C00000"));
        assert_eq!(runs[1].text, " & co");
        assert!(remove_block(&mut glossary, 0));
        assert!(blocks_of(
            &Document {
                prolog: String::new(),
                root: glossary
            }
            .to_xml()
        )
        .is_empty());
    }
}
