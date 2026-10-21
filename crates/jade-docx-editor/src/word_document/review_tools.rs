//! Review and accessibility edits that touch a drawing's own XML: Word's
//! 代替テキスト — a picture's or shape's description (`wp:docPr@descr`) and
//! its 装飾用にする flag (the `adec:decorative` extension Word 2019 writes).

use super::*;

/// The `a:ext` Word writes 装飾用 under.
pub const DECORATIVE_EXT_URI: &str = "{C183D7F6-B498-43B3-948B-1728B52AA6E4}";
const DECORATIVE_NS: &str = "http://schemas.microsoft.com/office/drawing/2017/decorative";
const DRAWING_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

/// A drawing's 代替テキスト as Word's pane shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AltText {
    pub description: Option<String>,
    pub decorative: bool,
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// The first `wp:docPr` under `element`.
fn find_doc_pr(element: &XmlElement) -> Option<XmlNodeId> {
    if local(&element.name) == "docPr" {
        return Some(element.id);
    }
    element.children.iter().find_map(|child| match child {
        XmlNode::Element(child) => find_doc_pr(child),
        _ => None,
    })
}

/// Whether a `wp:docPr` carries Word's decorative flag.
pub fn doc_pr_is_decorative(doc_pr: &XmlElement) -> bool {
    fn walk(element: &XmlElement) -> bool {
        if local(&element.name) == "decorative" {
            return element
                .attributes
                .iter()
                .find(|attribute| attribute.name == "val")
                .is_none_or(|attribute| matches!(attribute.value.as_str(), "1" | "true"));
        }
        element.children.iter().any(|child| match child {
            XmlNode::Element(child) => walk(child),
            _ => false,
        })
    }
    walk(doc_pr)
}

impl WordDocument {
    /// The 代替テキスト of the picture or shape at `target`.
    pub fn object_alt_text(&self, target: FloatingObjectTarget) -> Option<AltText> {
        let run = self
            .paragraph(target.paragraph_index)?
            .runs
            .get(target.run_index)?;
        let doc_pr = run
            .node_id
            .and_then(|id| self.raw_document.as_ref()?.element(id))
            .and_then(|element| {
                let id = find_doc_pr(element)?;
                self.raw_document.as_ref()?.element(id)
            });
        if let Some(image) = run.image.as_ref() {
            return Some(AltText {
                description: image.description.clone().filter(|text| !text.is_empty()),
                decorative: image.decorative || doc_pr.is_some_and(doc_pr_is_decorative),
            });
        }
        let doc_pr = doc_pr?;
        Some(AltText {
            description: doc_pr
                .attributes
                .iter()
                .find(|attribute| attribute.name == "descr")
                .map(|attribute| attribute.value.clone())
                .filter(|text| !text.is_empty()),
            decorative: doc_pr_is_decorative(doc_pr),
        })
    }

    /// Sets the 代替テキスト of the picture or shape at `target`. Decorative
    /// drawings have no description, as Word clears it when the box is
    /// ticked.
    pub fn set_object_alt_text(
        &mut self,
        target: FloatingObjectTarget,
        alt: &AltText,
    ) -> Result<(), String> {
        let description = alt
            .description
            .clone()
            .filter(|text| !text.trim().is_empty() && !alt.decorative);
        let run = self
            .paragraph_mut(target.paragraph_index)
            .and_then(|paragraph| paragraph.runs.get_mut(target.run_index))
            .ok_or("no picture or shape there")?;
        if let Some(image) = run.image.as_mut() {
            image.description = description.clone();
            image.decorative = alt.decorative;
        } else if run.text_box.is_none() {
            return Err("no picture or shape there".into());
        }
        let Some(node) = run.node_id else {
            // A drawing written from the model: the serializer writes both.
            return Ok(());
        };
        let raw = self
            .raw_document
            .as_mut()
            .ok_or("the document has no XML to edit")?;
        let doc_pr = raw
            .element(node)
            .and_then(find_doc_pr)
            .ok_or("the drawing has no non-visual properties")?;
        // The decorative extension, built before the element is borrowed.
        let decorative_ext = alt.decorative.then(|| {
            let flag = raw.create_element(
                "adec:decorative",
                vec![
                    XmlAttribute {
                        name: "xmlns:adec".into(),
                        value: DECORATIVE_NS.into(),
                    },
                    XmlAttribute {
                        name: "val".into(),
                        value: "1".into(),
                    },
                ],
                Vec::new(),
            );
            raw.create_element(
                "a:ext",
                vec![XmlAttribute {
                    name: "uri".into(),
                    value: DECORATIVE_EXT_URI.into(),
                }],
                vec![XmlNode::Element(flag)],
            )
        });
        let ext_list = raw.create_element(
            "a:extLst",
            vec![XmlAttribute {
                name: "xmlns:a".into(),
                value: DRAWING_NS.into(),
            }],
            Vec::new(),
        );
        let element = raw.element_mut(doc_pr).ok_or("the drawing went missing")?;
        element
            .attributes
            .retain(|attribute| attribute.name != "descr");
        if let Some(text) = description {
            element.attributes.push(XmlAttribute {
                name: "descr".into(),
                value: text,
            });
        }
        // Out with any decorative extension, and with an extension list it
        // leaves empty; in with a fresh one when asked.
        let is_decorative_ext = |child: &XmlNode| {
            matches!(child, XmlNode::Element(ext)
            if local(&ext.name) == "ext"
                && ext.attributes.iter().any(|attribute| {
                    attribute.name == "uri" && attribute.value == DECORATIVE_EXT_URI
                }))
        };
        for child in element.children.iter_mut() {
            if let XmlNode::Element(list) = child {
                if local(&list.name) == "extLst" {
                    list.children.retain(|ext| !is_decorative_ext(ext));
                }
            }
        }
        element.children.retain(|child| {
            !matches!(child, XmlNode::Element(list)
                if local(&list.name) == "extLst" && list.children.is_empty())
        });
        if let Some(ext) = decorative_ext {
            let list = element.children.iter_mut().find_map(|child| match child {
                XmlNode::Element(list) if local(&list.name) == "extLst" => Some(list),
                _ => None,
            });
            match list {
                Some(list) => list.children.push(XmlNode::Element(ext)),
                None => {
                    let mut list = ext_list;
                    list.children.push(XmlNode::Element(ext));
                    element.children.push(XmlNode::Element(list));
                }
            }
        }
        Ok(())
    }
}

/// The commands that change paragraph formatting: with changes tracked,
/// what each one changes is recorded as a `w:pPrChange`.
pub(crate) const PARAGRAPH_FORMAT_COMMANDS: [&str; 16] = [
    "adjust_paragraph_indent",
    "apply_multilevel_list",
    "reset_paragraph_formatting",
    "set_drop_cap",
    "set_numbering",
    "set_paragraph_alignment",
    "set_paragraph_direction",
    "set_paragraph_indent",
    "set_paragraph_line_spacing",
    "set_paragraph_pagination",
    "set_paragraph_style",
    "set_paragraph_visuals",
    "set_paragraph_typography",
    "set_japanese_typography",
    "set_paragraph_metrics",
    "toggle_paragraph_space_before",
];

/// A paragraph's `w:pPr`, as it was before a formatting command.
pub(crate) type ParagraphFormats = Vec<(XmlNodeId, Option<XmlElement>)>;

/// Who a paragraph formatting change is by.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParagraphFormatChange {
    pub id: Option<String>,
    pub author: Option<String>,
    pub date: Option<String>,
}

fn child<'a>(element: &'a XmlElement, name: &str) -> Option<&'a XmlElement> {
    element.children.iter().find_map(|node| match node {
        XmlNode::Element(child) if local(&child.name) == name => Some(child),
        _ => None,
    })
}

fn escape(text: &str, attribute: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
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

/// `element` written out as XML.
pub(crate) fn element_xml(element: &XmlElement) -> String {
    let mut out = format!("<{}", element.name);
    for attribute in &element.attributes {
        out.push_str(&format!(
            " {}=\"{}\"",
            attribute.name,
            escape(&attribute.value, true)
        ));
    }
    if element.children.is_empty() {
        out.push_str("/>");
        return out;
    }
    out.push('>');
    for node in &element.children {
        match node {
            XmlNode::Element(child) => out.push_str(&element_xml(child)),
            XmlNode::Text(text) => out.push_str(&escape(text, false)),
            XmlNode::CData(text) => out.push_str(&format!("<![CDATA[{text}]]>")),
            XmlNode::Comment(text) => out.push_str(&format!("<!--{text}-->")),
        }
    }
    out.push_str(&format!("</{}>", element.name));
    out
}

/// A `w:pPr`'s formatting, the record of a change to it left out.
fn formatting_of(p_pr: Option<&XmlElement>) -> String {
    p_pr.map(|p_pr| {
        p_pr.children
            .iter()
            .filter(|node| !matches!(node, XmlNode::Element(child) if local(&child.name) == "pPrChange"))
            .map(|node| match node {
                XmlNode::Element(child) => element_xml(child),
                _ => String::new(),
            })
            .collect()
    })
    .unwrap_or_default()
}

impl WordDocument {
    fn paragraph_element(&self, index: usize) -> Option<&XmlElement> {
        let id = self.paragraph(index)?.node_id?;
        self.raw_document
            .as_ref()?
            .element(id)
            .filter(|element| local(&element.name) == "p")
    }

    /// Every body paragraph's `w:pPr`, before a formatting command runs.
    pub(crate) fn paragraph_formats(&self) -> ParagraphFormats {
        (0..self.paragraph_count())
            .filter_map(|index| {
                let element = self.paragraph_element(index)?;
                Some((element.id, child(element, "pPr").cloned()))
            })
            .collect()
    }

    /// After a formatting command with changes tracked: each paragraph whose
    /// `w:pPr` it changed keeps what it was in a `w:pPrChange` (one already
    /// there keeps the first). Returns the records put in, for undo.
    pub(crate) fn track_paragraph_format_changes(
        &mut self,
        before: ParagraphFormats,
        author: Option<&str>,
        date: Option<&str>,
    ) -> Vec<XmlNodeId> {
        let mut inserted = Vec::new();
        for (paragraph_id, old) in before {
            let Some(raw) = self.raw_document.as_ref() else {
                break;
            };
            let Some(paragraph) = raw.element(paragraph_id) else {
                continue;
            };
            let new = child(paragraph, "pPr");
            if formatting_of(new) == formatting_of(old.as_ref())
                || new.is_some_and(|p_pr| child(p_pr, "pPrChange").is_some())
            {
                continue;
            }
            // The formatting before: the old `w:pPr` without the paragraph
            // mark's run properties, a section break or an older record.
            let previous: Vec<XmlNode> = old
                .map(|old| {
                    old.children
                        .into_iter()
                        .filter(|node| {
                            !matches!(node, XmlNode::Element(child)
                                if matches!(local(&child.name), "rPr" | "sectPr" | "pPrChange"))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let id = self.allocate_revision_id().to_string();
            let raw = self.raw_document.as_mut().expect("checked above");
            let previous_p_pr = raw.create_element("w:pPr", Vec::new(), previous);
            let mut attributes = vec![XmlAttribute {
                name: "w:id".into(),
                value: id,
            }];
            for (name, value) in [("w:author", author), ("w:date", date)] {
                if let Some(value) = value {
                    attributes.push(XmlAttribute {
                        name: name.into(),
                        value: value.into(),
                    });
                }
            }
            let record = raw.create_element(
                "w:pPrChange",
                attributes,
                vec![XmlNode::Element(previous_p_pr)],
            );
            let record_id = record.id;
            let has_p_pr = raw
                .element(paragraph_id)
                .is_some_and(|paragraph| child(paragraph, "pPr").is_some());
            if !has_p_pr {
                let p_pr = raw.create_element("w:pPr", Vec::new(), Vec::new());
                if let Some(paragraph) = raw.element_mut(paragraph_id) {
                    paragraph.children.insert(0, XmlNode::Element(p_pr));
                }
            }
            let Some(paragraph) = raw.element_mut(paragraph_id) else {
                continue;
            };
            if let Some(XmlNode::Element(p_pr)) = paragraph
                .children
                .iter_mut()
                .find(|node| matches!(node, XmlNode::Element(child) if local(&child.name) == "pPr"))
            {
                p_pr.children.push(XmlNode::Element(record));
                inserted.push(record_id);
                // The layout's paragraph shows the change bar.
                if let Some(index) = (0..self.paragraph_count()).find(|&index| {
                    self.paragraph(index)
                        .is_some_and(|paragraph| paragraph.node_id == Some(paragraph_id))
                }) {
                    if let Some(paragraph) = self.paragraph_mut(index) {
                        let properties = paragraph.properties.get_or_insert_with(Default::default);
                        properties.has_format_change = true;
                        properties.format_change_author = author.map(Into::into);
                    }
                }
            }
        }
        inserted
    }

    /// Takes the records `track_paragraph_format_changes` put in back out.
    pub(crate) fn remove_paragraph_format_records(&mut self, records: &[XmlNodeId]) {
        let Some(raw) = self.raw_document.as_mut() else {
            return;
        };
        for &id in records {
            if let Some(path) = raw.path(id) {
                let _ = raw.remove_node(&path);
            }
        }
    }

    /// The tracked change to paragraph `index`'s formatting, if it has one.
    pub fn paragraph_format_change(&self, index: usize) -> Option<ParagraphFormatChange> {
        let record = child(child(self.paragraph_element(index)?, "pPr")?, "pPrChange")?;
        let attribute = |name: &str| {
            record
                .attributes
                .iter()
                .find(|attribute| attribute.name == name)
                .map(|attribute| attribute.value.clone())
        };
        Some(ParagraphFormatChange {
            id: attribute("w:id"),
            author: attribute("w:author"),
            date: attribute("w:date"),
        })
    }

    /// 承諾 (`accept`) or 元に戻す for paragraph `index`'s formatting change:
    /// accepted, the record goes; rejected, the formatting in the record comes
    /// back — in the XML and in the paragraph the layout reads.
    pub(crate) fn decide_paragraph_format_change(
        &mut self,
        index: usize,
        accept: bool,
    ) -> Result<(), String> {
        let paragraph_id = self
            .paragraph_element(index)
            .map(|element| element.id)
            .ok_or("the paragraph has no XML")?;
        let raw = self
            .raw_document
            .as_mut()
            .ok_or("the document has no XML")?;
        let paragraph = raw
            .element_mut(paragraph_id)
            .ok_or("the paragraph went missing")?;
        let Some(XmlNode::Element(p_pr)) = paragraph
            .children
            .iter_mut()
            .find(|node| matches!(node, XmlNode::Element(child) if local(&child.name) == "pPr"))
        else {
            return Ok(());
        };
        let Some(at) = p_pr.children.iter().position(
            |node| matches!(node, XmlNode::Element(child) if local(&child.name) == "pPrChange"),
        ) else {
            return Ok(());
        };
        let XmlNode::Element(record) = p_pr.children.remove(at) else {
            unreachable!("matched an element");
        };
        if accept {
            if let Some(properties) = self
                .paragraph_mut(index)
                .and_then(|paragraph| paragraph.properties.as_mut())
            {
                properties.has_format_change = false;
                properties.format_change_author = None;
            }
            return Ok(());
        }
        // The formatting in the record, with the paragraph mark's run
        // properties and any section break the paragraph carries.
        let previous: Vec<XmlNode> = record
            .children
            .into_iter()
            .find_map(|node| match node {
                XmlNode::Element(inner) if local(&inner.name) == "pPr" => Some(inner.children),
                _ => None,
            })
            .unwrap_or_default();
        let kept: Vec<XmlNode> = p_pr
            .children
            .drain(..)
            .filter(|node| {
                matches!(node, XmlNode::Element(child) if matches!(local(&child.name), "rPr" | "sectPr"))
            })
            .collect();
        p_pr.children = previous;
        p_pr.children.extend(kept);
        let restored = element_xml(p_pr);
        // The paragraph the layout reads, from the restored `w:pPr` and what
        // its style gives, as a load would make it.
        let wrapper = format!(
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p>{restored}<w:r><w:t>x</w:t></w:r></w:p></w:body></w:document>"#
        );
        let parsed = crate::parser::DocumentParser::new()
            .parse_document_xml(&wrapper)
            .map_err(|error| error.to_string())?;
        let mut properties = parsed
            .body
            .and_then(|body| {
                body.elements.into_iter().find_map(|element| match element {
                    BodyElement::Paragraph(paragraph) => Some(paragraph.properties),
                    _ => None,
                })
            })
            .flatten()
            .unwrap_or_default();
        let style = properties.style.as_deref().map(str::to_string);
        if let Some(inherited) = self.inherited_paragraph_properties(index, style.as_deref()) {
            Self::fill_missing_paragraph_props(&mut properties, &inherited);
        }
        if let Some(paragraph) = self.paragraph_mut(index) {
            // What the paragraph mark carries stays the paragraph's own.
            if let Some(old) = paragraph.properties.as_ref() {
                properties.mark_revision = old.mark_revision.clone();
            }
            paragraph.properties = Some(properties);
        }
        Ok(())
    }
}

impl WordDocument {
    /// The names of the package's parts, as a save would write them: for
    /// ドキュメント検査, which counts custom XML among them.
    pub fn package_part_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .package
            .as_ref()
            .map(|package| package.files.keys().cloned().collect())
            .unwrap_or_default();
        names.extend(self.parts.keys().cloned());
        names.extend(self.binary_parts.keys().cloned());
        names.retain(|name| !self.removed_parts.contains(name));
        names.sort();
        names.dedup();
        names
    }
}

/// The formatting 文書の比較 compares: what a reader sees of a run.
fn visible_formatting(properties: Option<&RunProperties>) -> RunProperties {
    let Some(properties) = properties else {
        return RunProperties::default();
    };
    RunProperties {
        style: properties.style.clone(),
        bold: properties.bold,
        italic: properties.italic,
        underline: properties.underline,
        underline_kind: properties.underline_kind.clone(),
        strike: properties.strike,
        double_strike: properties.double_strike,
        caps: properties.caps,
        small_caps: properties.small_caps,
        font_size: properties.font_size,
        font_family: properties.font_family.clone(),
        color: properties.color.clone(),
        highlight: properties.highlight.clone(),
        baseline: properties.baseline.clone(),
        ..RunProperties::default()
    }
}

impl WordDocument {
    /// 文書の比較 › 書式: in each paragraph whose text `original` has too
    /// and that carries no tracked change, every run formatted otherwise in
    /// `original` gets a formatting change (`w:rPrChange`) holding the
    /// original's formatting, put down to `author` at `date`. Returns how many
    /// runs were marked.
    pub fn mark_formatting_differences(
        &mut self,
        original: &WordDocument,
        author: &str,
        date: &str,
    ) -> usize {
        let text_of = |document: &WordDocument, index: usize| -> Option<String> {
            Some(
                document
                    .paragraph(index)?
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect(),
            )
        };
        let mut used = vec![false; original.paragraph_count()];
        let mut from = 0usize;
        let mut marked = 0usize;
        for index in 0..self.paragraph_count() {
            let Some(paragraph) = self.paragraph(index) else {
                continue;
            };
            if paragraph.runs.iter().any(|run| run.revision.is_some()) {
                continue;
            }
            let Some(text) = text_of(self, index).filter(|text| !text.trim().is_empty()) else {
                continue;
            };
            // The same text in the original, the nearest after the last match.
            let Some(matched) =
                (from..original.paragraph_count())
                    .chain(0..from)
                    .find(|&candidate| {
                        !used[candidate] && text_of(original, candidate).as_deref() == Some(&text)
                    })
            else {
                continue;
            };
            used[matched] = true;
            from = matched + 1;
            let Some(old) = original.paragraph(matched) else {
                continue;
            };
            let mut old_runs = Vec::new();
            let mut offset = 0usize;
            for run in &old.runs {
                old_runs.push((offset, offset + run.text.len(), run.properties.as_ref()));
                offset += run.text.len();
            }
            let mut changes = Vec::new();
            let mut offset = 0usize;
            for (run_index, run) in paragraph.runs.iter().enumerate() {
                let start = offset;
                offset += run.text.len();
                if run.text.is_empty() {
                    continue;
                }
                let before = old_runs
                    .iter()
                    .find(|(from, to, _)| *from <= start && start < *to)
                    .map(|(_, _, properties)| visible_formatting(*properties))
                    .unwrap_or_default();
                if before != visible_formatting(run.properties.as_ref()) {
                    changes.push((run_index, before));
                }
            }
            if changes.is_empty() {
                continue;
            }
            let mut restyled = Vec::new();
            for (run_index, before) in changes {
                let id = self.allocate_revision_id().to_string();
                let Some(run) = self
                    .paragraph_mut(index)
                    .and_then(|paragraph| paragraph.runs.get_mut(run_index))
                else {
                    continue;
                };
                // The original's formatting where this run's differs.
                let mut previous = run.properties.clone().unwrap_or_default();
                let visible = visible_formatting(Some(&previous));
                if visible.bold != before.bold {
                    previous.bold = before.bold;
                }
                if visible.italic != before.italic {
                    previous.italic = before.italic;
                }
                if visible.underline != before.underline
                    || visible.underline_kind != before.underline_kind
                {
                    previous.underline = before.underline;
                    previous.underline_kind = before.underline_kind.clone();
                }
                if visible.strike != before.strike || visible.double_strike != before.double_strike
                {
                    previous.strike = before.strike;
                    previous.double_strike = before.double_strike;
                }
                if visible.caps != before.caps || visible.small_caps != before.small_caps {
                    previous.caps = before.caps;
                    previous.small_caps = before.small_caps;
                }
                if visible.font_size != before.font_size {
                    previous.font_size = before.font_size;
                }
                if visible.font_family != before.font_family {
                    previous.font_family = before.font_family.clone();
                }
                if visible.color != before.color {
                    previous.color = before.color.clone();
                    previous.theme_color = None;
                }
                if visible.highlight != before.highlight {
                    previous.highlight = before.highlight.clone();
                }
                if visible.baseline != before.baseline {
                    previous.baseline = before.baseline.clone();
                }
                if visible.style != before.style {
                    previous.style = before.style.clone();
                }
                previous.change = None;
                run.properties
                    .get_or_insert_with(RunProperties::default)
                    .change = Some(Box::new(crate::document::RunPropertiesChange {
                    id: Some(id),
                    author: Some(author.to_string()),
                    date: Some(date.to_string()),
                    previous,
                }));
                restyled.extend(run.node_id);
                marked += 1;
            }
            self.sync_paragraph_run_properties(index, &restyled);
        }
        marked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor;

    const DOCUMENT: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body><w:p><w:r><w:drawing><wp:inline><wp:extent cx="952500" cy="952500"/><wp:docPr id="1" name="Picture 1" descr="old"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="0" name="p.png"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rId9"/></pic:blipFill><pic:spPr/></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p><w:sectPr/></w:body></w:document>"#;

    fn docx() -> Vec<u8> {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("[Content_Types].xml", options).unwrap();
        zip.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#).unwrap();
        zip.start_file("_rels/.rels", options).unwrap();
        zip.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#).unwrap();
        zip.start_file("word/_rels/document.xml.rels", options)
            .unwrap();
        zip.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/p.png"/></Relationships>"#).unwrap();
        zip.start_file("word/media/p.png", options).unwrap();
        zip.write_all(&[0x89, b'P', b'N', b'G']).unwrap();
        zip.start_file("word/document.xml", options).unwrap();
        zip.write_all(DOCUMENT.as_bytes()).unwrap();
        zip.finish().unwrap().into_inner()
    }

    fn docx_with_body(body: &str) -> Vec<u8> {
        let document = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}<w:sectPr/></w:body></w:document>"#
        );
        let mut parts = vec![("word/document.xml".to_string(), document)];
        parts.push((
            "[Content_Types].xml".into(),
            r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.into(),
        ));
        parts.push((
            "_rels/.rels".into(),
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.into(),
        ));
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, content) in parts {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn document_xml(saved: &[u8]) -> String {
        use std::io::Read;
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(saved)).unwrap();
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .unwrap()
            .read_to_string(&mut xml)
            .unwrap();
        xml
    }

    fn revisions(document: &WordDocument) -> Vec<jade_docx_headless_api::HeadlessRevisionSnapshot> {
        use jade_docx_headless_api::{HeadlessQuery, HeadlessQueryRequest, HeadlessQueryResult};
        let result = crate::headless_api::execute_headless_queries_on_document(
            document,
            &HeadlessQueryRequest {
                queries: vec![HeadlessQuery::Revisions {
                    start_revision: 0,
                    max_revisions: 100,
                    kind: None,
                    author: None,
                    date_from: None,
                    date_to: None,
                }],
            },
        )
        .unwrap();
        match result.results.into_iter().next() {
            Some(HeadlessQueryResult::Revisions { revisions, .. }) => revisions,
            other => panic!("{other:?}"),
        }
    }

    fn edit(document: &mut WordDocument, commands: Vec<jade_docx_headless_api::HeadlessCommand>) {
        crate::headless_api::apply_headless_commands_to_document(
            document,
            &jade_docx_headless_api::HeadlessEditRequest { commands },
        )
        .unwrap();
    }

    /// With changes tracked, making text bold is a formatting change Word
    /// lists and saves as `w:rPrChange`: accepted, the bold stays; rejected,
    /// the text is as it was.
    #[test]
    fn tracked_formatting_is_recorded_listed_and_decided() {
        use jade_docx_headless_api::HeadlessCommand;
        let bytes = docx_with_body(r#"<w:p><w:r><w:t>hello world</w:t></w:r></w:p>"#);
        for accept in [true, false] {
            let mut document =
                executor::block_on(WordDocument::load(&bytes, &Options::default())).unwrap();
            edit(
                &mut document,
                vec![
                    HeadlessCommand::StartChangeTracking {
                        author: Some("Ann".into()),
                        initials: None,
                        date: Some("2026-10-22T00:00:00Z".into()),
                        date_utc: None,
                    },
                    HeadlessCommand::ApplyBold {
                        start_paragraph: 0,
                        start_offset: 0,
                        end_paragraph: 0,
                        end_offset: 5,
                        enabled: true,
                    },
                ],
            );
            let listed = revisions(&document);
            assert_eq!(listed.len(), 1, "{listed:?}");
            assert_eq!(listed[0].kind, "formatted");
            assert_eq!(listed[0].author.as_deref(), Some("Ann"));
            assert_eq!((listed[0].start_offset, listed[0].end_offset), (0, 5));
            let xml = document_xml(&document.export_docx().unwrap());
            assert!(xml.contains("<w:rPrChange"), "{xml}");
            assert!(xml.contains(r#"w:author="Ann""#), "{xml}");

            // Saved and opened again, the change is still the same change.
            let mut reopened = executor::block_on(WordDocument::load(
                &document.export_docx().unwrap(),
                &Options::default(),
            ))
            .unwrap();
            assert_eq!(revisions(&reopened).len(), 1);
            let decide = if accept {
                HeadlessCommand::AcceptRevisions {
                    kind: None,
                    start_paragraph: None,
                    end_paragraph: None,
                    revision_id: None,
                    revision_ids: None,
                    revision_targets: None,
                }
            } else {
                HeadlessCommand::RejectRevisions {
                    kind: None,
                    start_paragraph: None,
                    end_paragraph: None,
                    revision_id: None,
                    revision_ids: None,
                    revision_targets: None,
                }
            };
            edit(&mut reopened, vec![decide]);
            assert!(revisions(&reopened).is_empty());
            let bold = reopened.paragraph(0).unwrap().runs[0]
                .properties
                .as_ref()
                .and_then(|properties| properties.bold);
            assert_eq!(bold.unwrap_or(false), accept, "accept={accept}");
            let xml = document_xml(&reopened.export_docx().unwrap());
            assert!(!xml.contains("rPrChange"), "{xml}");
        }
    }

    /// Word's own `w:rPrChange` — italic taken off — is read with what the
    /// text was, and rejecting it puts the italic back.
    #[test]
    fn a_change_word_recorded_is_read_and_rejected() {
        use jade_docx_headless_api::HeadlessCommand;
        let bytes = docx_with_body(
            r#"<w:p><w:r><w:rPr><w:b/><w:rPrChange w:id="7" w:author="Bob" w:date="2025-01-01T00:00:00Z"><w:rPr><w:i/></w:rPr></w:rPrChange></w:rPr><w:t>changed</w:t></w:r></w:p>"#,
        );
        let mut document =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).unwrap();
        let change = document.paragraph(0).unwrap().runs[0]
            .properties
            .as_ref()
            .and_then(|properties| properties.change.clone())
            .expect("the change is read");
        assert_eq!(change.author.as_deref(), Some("Bob"));
        assert_eq!(change.previous.italic, Some(true));
        assert_eq!(change.previous.bold, None);
        // Untouched, it is saved as it was.
        assert!(document_xml(&document.export_docx().unwrap()).contains(r#"w:id="7""#));
        edit(
            &mut document,
            vec![HeadlessCommand::RejectRevisions {
                kind: None,
                start_paragraph: None,
                end_paragraph: None,
                revision_id: Some("7".into()),
                revision_ids: None,
                revision_targets: None,
            }],
        );
        let properties = document.paragraph(0).unwrap().runs[0]
            .properties
            .clone()
            .unwrap_or_default();
        assert_eq!(properties.italic, Some(true));
        assert_ne!(properties.bold, Some(true));
        assert!(properties.change.is_none());
    }

    /// With changes tracked, centring a paragraph records what its
    /// formatting was (`w:pPrChange`): listed, undone with the command, and
    /// rejected — the alignment back as it was, in the XML and the layout.
    #[test]
    fn tracked_paragraph_formatting_is_recorded_and_rejected() {
        use jade_docx_headless_api::HeadlessCommand;
        let bytes = docx_with_body(
            r#"<w:p><w:pPr><w:jc w:val="right"/></w:pPr><w:r><w:t>a paragraph</w:t></w:r></w:p>"#,
        );
        let mut document =
            executor::block_on(WordDocument::load(&bytes, &Options::default())).unwrap();
        edit(
            &mut document,
            vec![
                HeadlessCommand::StartChangeTracking {
                    author: Some("Ann".into()),
                    initials: None,
                    date: None,
                    date_utc: None,
                },
                HeadlessCommand::SetParagraphAlignment {
                    start_paragraph: 0,
                    end_paragraph: 0,
                    alignment: "center".into(),
                },
            ],
        );
        let listed = revisions(&document);
        assert_eq!(listed.len(), 1, "{listed:?}");
        assert_eq!(listed[0].kind, "formatted");
        assert!(document.paragraph_format_change(0).is_some());
        assert!(
            document
                .paragraph(0)
                .unwrap()
                .properties
                .as_ref()
                .unwrap()
                .has_format_change
        );
        let xml = document_xml(&document.export_docx().unwrap());
        assert!(xml.contains("<w:pPrChange"), "{xml}");
        assert!(xml.contains(r#"<w:pPrChange w:id"#), "{xml}");

        // Saved and opened again, then rejected: right-aligned again.
        let mut reopened = executor::block_on(WordDocument::load(
            &document.export_docx().unwrap(),
            &Options::default(),
        ))
        .unwrap();
        assert!(reopened.paragraph_format_change(0).is_some());
        edit(
            &mut reopened,
            vec![HeadlessCommand::RejectRevisions {
                kind: None,
                start_paragraph: None,
                end_paragraph: None,
                revision_id: None,
                revision_ids: None,
                revision_targets: None,
            }],
        );
        assert!(revisions(&reopened).is_empty());
        let alignment = reopened
            .paragraph(0)
            .unwrap()
            .properties
            .as_ref()
            .unwrap()
            .alignment
            .clone();
        assert_eq!(alignment.as_deref(), Some("right"));
        let xml = document_xml(&reopened.export_docx().unwrap());
        assert!(!xml.contains("pPrChange"), "{xml}");
        assert!(xml.contains(r#"w:val="right""#), "{xml}");
    }

    /// 文書の比較 › 書式: a run bold now and not in the original is marked
    /// as a formatting change holding the original's formatting.
    #[test]
    fn formatting_differences_from_an_original_are_marked() {
        let original = executor::block_on(WordDocument::load(
            &docx_with_body(r#"<w:p><w:r><w:t>same text</w:t></w:r></w:p>"#),
            &Options::default(),
        ))
        .unwrap();
        let mut revised = executor::block_on(WordDocument::load(
            &docx_with_body(
                r#"<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>same</w:t></w:r><w:r><w:t xml:space="preserve"> text</w:t></w:r></w:p>"#,
            ),
            &Options::default(),
        ))
        .unwrap();
        assert_eq!(
            revised.mark_formatting_differences(&original, "Ann", "2026-10-22T00:00:00Z"),
            1
        );
        let change = revised.paragraph(0).unwrap().runs[0]
            .properties
            .as_ref()
            .and_then(|properties| properties.change.clone())
            .unwrap();
        assert_eq!(change.author.as_deref(), Some("Ann"));
        assert_ne!(change.previous.bold, Some(true));
        assert!(document_xml(&revised.export_docx().unwrap()).contains("<w:rPrChange"));
        // The same formatting both sides: nothing to mark.
        let mut same = executor::block_on(WordDocument::load(
            &docx_with_body(r#"<w:p><w:r><w:t>same text</w:t></w:r></w:p>"#),
            &Options::default(),
        ))
        .unwrap();
        assert_eq!(same.mark_formatting_differences(&original, "Ann", "x"), 0);
    }

    #[test]
    fn package_part_names_list_what_a_save_writes() {
        let document = executor::block_on(WordDocument::load(
            &docx_with_body("<w:p/>"),
            &Options::default(),
        ))
        .unwrap();
        let names = document.package_part_names();
        assert!(
            names.contains(&"word/document.xml".to_string()),
            "{names:?}"
        );
        assert!(names.contains(&"_rels/.rels".to_string()), "{names:?}");
    }

    fn picture(document: &WordDocument) -> FloatingObjectTarget {
        let run_index = document
            .paragraph(0)
            .unwrap()
            .runs
            .iter()
            .position(|run| run.image.is_some())
            .expect("a picture");
        FloatingObjectTarget {
            paragraph_index: 0,
            run_index,
            kind: FloatingObjectKind::Image,
        }
    }

    /// 代替テキスト is read, written into the drawing's own XML, and marked
    /// decorative as Word marks it; each survives a save and reopen.
    #[test]
    fn alt_text_and_the_decorative_flag_round_trip() {
        let mut document =
            executor::block_on(WordDocument::load(&docx(), &Options::default())).unwrap();
        let target = picture(&document);
        assert_eq!(
            document.object_alt_text(target),
            Some(AltText {
                description: Some("old".into()),
                decorative: false
            })
        );
        document
            .set_object_alt_text(
                target,
                &AltText {
                    description: Some("A red <square> & more".into()),
                    decorative: false,
                },
            )
            .unwrap();
        let saved = document.export_docx().unwrap();
        let mut reopened =
            executor::block_on(WordDocument::load(&saved, &Options::default())).unwrap();
        let target = picture(&reopened);
        assert_eq!(
            reopened
                .object_alt_text(target)
                .unwrap()
                .description
                .as_deref(),
            Some("A red <square> & more")
        );

        reopened
            .set_object_alt_text(
                target,
                &AltText {
                    description: Some("ignored".into()),
                    decorative: true,
                },
            )
            .unwrap();
        let saved = reopened.export_docx().unwrap();
        let xml = {
            use std::io::Read;
            let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&saved)).unwrap();
            let mut xml = String::new();
            archive
                .by_name("word/document.xml")
                .unwrap()
                .read_to_string(&mut xml)
                .unwrap();
            xml
        };
        assert!(xml.contains(DECORATIVE_EXT_URI), "{xml}");
        assert!(!xml.contains("descr="), "{xml}");
        let again = executor::block_on(WordDocument::load(&saved, &Options::default())).unwrap();
        assert_eq!(
            again.object_alt_text(picture(&again)),
            Some(AltText {
                description: None,
                decorative: true
            })
        );
    }
}
