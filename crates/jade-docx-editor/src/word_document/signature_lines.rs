//! 署名欄: a picture made a VML shape that asks for a signature
//! (`o:signatureline`, ECMA-376 Part 4 §19.1 / the Office VML extensions),
//! as Word writes one.

use super::review_tools::local;
use super::*;
use jade_docx_reader::SignatureLine;
use sha2::Digest;

const VML: &str = "urn:schemas-microsoft-com:vml";
const OFFICE: &str = "urn:schemas-microsoft-com:office:office";

impl WordDocument {
    /// Makes the picture run `run_index` of `paragraph_index` a signature
    /// line asking `line`'s signer to sign: its `w:drawing` becomes a
    /// `w:pict` with the same picture and an `o:signatureline`.
    pub fn make_signature_line(
        &mut self,
        paragraph_index: usize,
        run_index: usize,
        line: &SignatureLine,
    ) -> Result<(), String> {
        let (node, relationship, width, height) = {
            let run = self
                .paragraph(paragraph_index)
                .and_then(|paragraph| paragraph.runs.get(run_index))
                .ok_or("the picture was not found")?;
            let image = run.image.as_ref().ok_or("the run is not a picture")?;
            (
                run.node_id.ok_or("the picture has no XML")?,
                image
                    .relationship_id
                    .clone()
                    .ok_or("the picture has no part")?,
                image.width_px.unwrap_or(192.0) * 0.75,
                image.height_px.unwrap_or(96.0) * 0.75,
            )
        };
        let id = line.id.clone().unwrap_or_else(|| {
            let hash = sha2::Sha256::digest(
                format!(
                    "{relationship}|{:?}|{:?}|{paragraph_index}|{run_index}",
                    line.signer, line.title
                )
                .as_bytes(),
            );
            let hex: String = hash
                .iter()
                .take(16)
                .map(|byte| format!("{byte:02X}"))
                .collect();
            format!(
                "{{{}-{}-{}-{}-{}}}",
                &hex[0..8],
                &hex[8..12],
                &hex[12..16],
                &hex[16..20],
                &hex[20..32]
            )
        });
        let raw = self
            .raw_document
            .as_mut()
            .ok_or("the document has no XML")?;
        // The VML and Office namespaces, declared where Word declares them.
        let root_id = raw.root().id;
        if let Some(root) = raw.element_mut(root_id) {
            for (prefix, uri) in [("xmlns:v", VML), ("xmlns:o", OFFICE)] {
                if !root
                    .attributes
                    .iter()
                    .any(|attribute| attribute.name == prefix)
                {
                    root.attributes.push(XmlAttribute {
                        name: prefix.into(),
                        value: uri.into(),
                    });
                }
            }
        }
        let attribute = |name: &str, value: &str| XmlAttribute {
            name: name.into(),
            value: value.into(),
        };
        let imagedata = raw.create_element(
            "v:imagedata",
            vec![attribute("r:id", &relationship), attribute("o:title", "")],
            Vec::new(),
        );
        let lock = raw.create_element(
            "o:lock",
            vec![
                attribute("v:ext", "edit"),
                attribute("ungrouping", "t"),
                attribute("rotation", "t"),
                attribute("cropping", "t"),
                attribute("verticies", "t"),
                attribute("text", "t"),
                attribute("grouping", "t"),
            ],
            Vec::new(),
        );
        let mut line_attributes = vec![
            attribute("v:ext", "edit"),
            attribute("id", &id),
            attribute("provid", "{00000000-0000-0000-0000-000000000000}"),
        ];
        for (name, value) in [
            ("o:suggestedsigner", &line.signer),
            ("o:suggestedsigner2", &line.title),
            ("o:suggestedsigneremail", &line.email),
            ("o:signinginstructions", &line.instructions),
        ] {
            if let Some(value) = value.as_deref().filter(|value| !value.is_empty()) {
                line_attributes.push(attribute(name, value));
            }
        }
        line_attributes.push(attribute("issignatureline", "t"));
        let signature_line = raw.create_element("o:signatureline", line_attributes, Vec::new());
        let shape = raw.create_element(
            "v:shape",
            vec![
                attribute("id", &format!("SignatureLine{}", &id[1..9])),
                attribute("type", "#_x0000_t75"),
                attribute("alt", "Microsoft Office Signature Line..."),
                attribute("style", &format!("width:{width:.1}pt;height:{height:.1}pt")),
            ],
            vec![
                XmlNode::Element(imagedata),
                XmlNode::Element(lock),
                XmlNode::Element(signature_line),
            ],
        );
        let pict = raw.create_element("w:pict", Vec::new(), vec![XmlNode::Element(shape)]);
        let run = raw
            .element_mut(node)
            .ok_or("the picture's XML went missing")?;
        let at = run
            .children
            .iter()
            .position(|child| matches!(child, XmlNode::Element(element) if local(&element.name) == "drawing"))
            .ok_or("the picture is not a drawing")?;
        run.children[at] = XmlNode::Element(pict);
        raw.rebuild_index();
        let mut line = line.clone();
        line.id = Some(id);
        if let Some(image) = self
            .paragraph_mut(paragraph_index)
            .and_then(|paragraph| paragraph.runs.get_mut(run_index))
            .and_then(|run| run.image.as_mut())
        {
            image.signature_line = Some(Box::new(line));
        }
        Ok(())
    }

    /// Every signature line in the document: where, and who is to sign.
    pub fn signature_lines(&self) -> Vec<(usize, usize, SignatureLine)> {
        (0..self.paragraph_count())
            .filter_map(|index| Some((index, self.paragraph(index)?)))
            .flat_map(|(index, paragraph)| {
                paragraph
                    .runs
                    .iter()
                    .enumerate()
                    .filter_map(move |(run_index, run)| {
                        let line = run.image.as_ref()?.signature_line.as_deref()?;
                        Some((index, run_index, line.clone()))
                    })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::insert_image::InsertImageCommand;
    use crate::commands::insert_signature_line::InsertSignatureLineCommand;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::word_document::review_tools::tests::docx_with_body;
    use futures::executor;

    const PNG_1X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";

    #[test]
    fn a_signature_line_is_written_as_word_writes_one_and_read_back() {
        let mut document = executor::block_on(WordDocument::load(
            &docx_with_body("<w:p><w:r><w:t>Sign here:</w:t></w:r></w:p>"),
            &Options::default(),
        ))
        .unwrap();
        let png = BASE64.decode(PNG_1X1).unwrap();
        let caret = SelectionRange::collapsed(TextPosition::new(0, 10));
        let mut selection = Some(caret);
        let mut executor = CommandExecutor::new();
        let line = SignatureLine {
            id: None,
            signer: Some("Ann Lee".into()),
            title: Some("Director".into()),
            email: Some("ann@example.com".into()),
            instructions: None,
        };
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(InsertSignatureLineCommand::new(
                        InsertImageCommand::new(
                            caret,
                            png,
                            "image/png".into(),
                            256.0,
                            128.0,
                            None,
                            None,
                        ),
                        line,
                    )),
                    &mut ctx,
                )
                .unwrap();
        }
        let lines = document.signature_lines();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].2.signer.as_deref(), Some("Ann Lee"));
        let saved = document.export_docx().unwrap();
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
        assert!(xml.contains("<o:signatureline"), "{xml}");
        assert!(xml.contains("o:suggestedsigner=\"Ann Lee\""));
        assert!(xml.contains("xmlns:v=\"urn:schemas-microsoft-com:vml\""));
        assert!(!xml.contains("<w:drawing"));
        let reloaded = executor::block_on(WordDocument::load(&saved, &Options::default())).unwrap();
        let lines = reloaded.signature_lines();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].2.title.as_deref(), Some("Director"));
        assert_eq!(lines[0].2.email.as_deref(), Some("ann@example.com"));
        assert!(reloaded.paragraph(0).unwrap().runs[lines[0].1]
            .image
            .as_ref()
            .unwrap()
            .relationship_id
            .is_some());
        // Undo takes it out again.
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor.undo(&mut ctx).unwrap();
        assert!(document.signature_lines().is_empty());
    }
}
