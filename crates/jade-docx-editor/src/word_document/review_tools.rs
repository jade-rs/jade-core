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
