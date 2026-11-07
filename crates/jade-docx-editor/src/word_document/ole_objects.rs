//! 挿入 › オブジェクト › ファイルから: a file embedded as an OLE package,
//! written as Word writes one — a `w:object` holding the VML picture Word
//! shows for it (the file's icon and name, or its first page) and an
//! `o:OLEObject` with ProgID `Package` pointing at
//! `word/embeddings/oleObjectN.bin`, a compound file whose `\x01Ole10Native`
//! stream carries the file's name and bytes ([MS-OLEDS] §2.3.6 and the
//! Packager's native data, as Office reads it).
//!
//! The same stream is read back for any package Word embedded, so a
//! double-click can give the file to the system to open.

use super::review_tools::local;
use super::*;
use std::io::{Read as _, Write as _};

const VML: &str = "urn:schemas-microsoft-com:vml";
const OFFICE: &str = "urn:schemas-microsoft-com:office:office";
const OLE_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.oleObject";
/// The Packager's class, `{0003000C-0000-0000-C000-000000000046}`.
const PACKAGE_CLSID: uuid::Uuid = uuid::Uuid::from_u128(0x0003000c_0000_0000_c000_000000000046);

/// A file embedded as a package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddedFile {
    /// Its name, as the icon's label shows it.
    pub name: String,
    pub bytes: Vec<u8>,
}

fn ansi(text: &str) -> Vec<u8> {
    let mut out: Vec<u8> = text
        .chars()
        .map(|c| {
            if c.is_ascii() && c != '\0' {
                c as u8
            } else {
                b'_'
            }
        })
        .collect();
    out.push(0);
    out
}

fn utf16(text: &str) -> Vec<u8> {
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut out = (units.len() as u32).to_le_bytes().to_vec();
    for unit in units {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out
}

/// The `\x01Ole10Native` stream for `name` holding `bytes`: the label,
/// the source path and the temporary path in the ANSI code page, the data,
/// then the three strings again in UTF-16 for names ANSI cannot hold.
fn ole10_native(name: &str, bytes: &[u8]) -> Vec<u8> {
    let temp = format!("C:\\Users\\jade\\AppData\\Local\\Temp\\{name}");
    let mut body = Vec::new();
    body.extend_from_slice(&2u16.to_le_bytes());
    body.extend(ansi(name));
    body.extend(ansi(name));
    body.extend_from_slice(&0u16.to_le_bytes());
    body.extend_from_slice(&3u16.to_le_bytes());
    let temp_ansi = ansi(&temp);
    body.extend_from_slice(&(temp_ansi.len() as u32).to_le_bytes());
    body.extend(temp_ansi);
    body.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    body.extend_from_slice(bytes);
    body.extend(utf16(&temp));
    body.extend(utf16(name));
    body.extend(utf16(name));
    let mut out = (body.len() as u32).to_le_bytes().to_vec();
    out.extend(body);
    out
}

/// The `\x01CompObj` stream naming the Packager ([MS-OLEDS] §2.3.8).
fn comp_obj() -> Vec<u8> {
    let mut out = vec![0x01, 0x00, 0xfe, 0xff, 0x03, 0x0a, 0x00, 0x00];
    out.extend_from_slice(&[0xff; 4]);
    out.extend_from_slice(&PACKAGE_CLSID.to_bytes_le());
    let prefixed = |text: &str| {
        let mut bytes = ((text.len() + 1) as u32).to_le_bytes().to_vec();
        bytes.extend_from_slice(text.as_bytes());
        bytes.push(0);
        bytes
    };
    out.extend(prefixed("OLE Package"));
    out.extend(prefixed("Package"));
    out.extend(prefixed("Package"));
    out.extend_from_slice(&0x71b2_39f4u32.to_le_bytes());
    out.extend_from_slice(&[0; 12]);
    out
}

/// The compound file a package object is: `name`'s `bytes` in an
/// `\x01Ole10Native` stream, under the Packager's class.
pub fn ole_package(name: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let cursor = std::io::Cursor::new(Vec::new());
    let mut compound = cfb::CompoundFile::create(cursor).map_err(|error| error.to_string())?;
    compound
        .set_storage_clsid("/", PACKAGE_CLSID)
        .map_err(|error| error.to_string())?;
    for (stream, content) in [
        ("/\u{1}CompObj", comp_obj()),
        ("/\u{1}Ole10Native", ole10_native(name, bytes)),
        // 表示形式: the object is shown as an icon.
        ("/\u{3}ObjInfo", vec![0x00, 0x00, 0x03, 0x00, 0x04, 0x00]),
    ] {
        compound
            .create_stream(stream)
            .and_then(|mut writer| writer.write_all(&content))
            .map_err(|error| error.to_string())?;
    }
    compound.flush().map_err(|error| error.to_string())?;
    Ok(compound.into_inner().into_inner())
}

/// The file a package object holds, from its compound file.
pub fn embedded_file(package: &[u8]) -> Option<EmbeddedFile> {
    let mut compound = cfb::CompoundFile::open(std::io::Cursor::new(package)).ok()?;
    let mut native = Vec::new();
    compound
        .open_stream("/\u{1}Ole10Native")
        .ok()?
        .read_to_end(&mut native)
        .ok()?;
    let mut at = 4 + 2;
    let cstring = |at: &mut usize| -> Option<String> {
        let end = native.get(*at..)?.iter().position(|byte| *byte == 0)?;
        let text = String::from_utf8_lossy(&native[*at..*at + end]).into_owned();
        *at += end + 1;
        Some(text)
    };
    let label = cstring(&mut at)?;
    let _source = cstring(&mut at)?;
    at += 4;
    let u32_at = |at: usize| -> Option<usize> {
        Some(u32::from_le_bytes(native.get(at..at + 4)?.try_into().ok()?) as usize)
    };
    let temp_len = u32_at(at)?;
    at += 4 + temp_len;
    let data_len = u32_at(at)?;
    at += 4;
    let bytes = native.get(at..at + data_len)?.to_vec();
    at += data_len;
    // The UTF-16 temp path, label and source, when the package has them:
    // the label in full where ANSI lost characters.
    let mut wide = Vec::new();
    for _ in 0..3 {
        let Some(count) = u32_at(at) else { break };
        let Some(units) = native.get(at + 4..at + 4 + count * 2) else {
            break;
        };
        let units: Vec<u16> = units
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        wide.push(String::from_utf16_lossy(&units));
        at += 4 + count * 2;
    }
    let name = wide
        .get(1)
        .filter(|name| !name.is_empty())
        .cloned()
        .unwrap_or(label);
    Some(EmbeddedFile { name, bytes })
}

impl WordDocument {
    fn next_embedding_part_path(&self) -> String {
        for index in 1.. {
            let candidate = format!("word/embeddings/oleObject{index}.bin");
            let exists_in_source = self
                .package
                .as_ref()
                .is_some_and(|package| package.files.contains_key(&candidate));
            if !exists_in_source && !self.binary_parts.contains_key(&candidate) {
                return candidate;
            }
        }
        unreachable!("unbounded embedding part search should always return");
    }

    /// Makes the picture run `run_index` of `paragraph_index` the shown
    /// face of an embedded object: `package` (an [`ole_package`], or any
    /// OLE compound file) becomes a part of its own, and the run's
    /// `w:drawing` a `w:object` naming it, shown as an icon or as its
    /// content as `icon` says.
    pub fn make_ole_object(
        &mut self,
        paragraph_index: usize,
        run_index: usize,
        package: &[u8],
        program_id: &str,
        icon: bool,
    ) -> Result<(), String> {
        let (node, picture, width, height) = {
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
                image.width_px.unwrap_or(96.0) * 0.75,
                image.height_px.unwrap_or(96.0) * 0.75,
            )
        };
        // The package, as a part of the main document.
        let path = self.next_embedding_part_path();
        let relationship = {
            let document_part = self
                .document_part
                .as_mut()
                .ok_or("main document part is unavailable")?;
            let relationship = Relationship {
                id: Self::next_relationship_id(&document_part.relationships),
                r#type: RelationshipTypes::OleObject.as_str().to_string(),
                target: path.strip_prefix("word/").unwrap_or(&path).to_string(),
                target_mode: None,
            };
            document_part.relationships.push(relationship.clone());
            relationship
        };
        if let Some(document_part) = self.document_part.as_ref() {
            if let Some(part) = self.parts.get_mut(&document_part.path) {
                part.relationships = document_part.relationships.clone();
            }
        }
        self.ensure_document_relationships_part_has_relationship(&relationship)?;
        self.ensure_content_types_has_image(&path, "bin", OLE_CONTENT_TYPE)?;
        self.removed_parts.remove(&path);
        self.binary_parts.insert(path, package.to_vec());

        let object_number = self.next_ole_object_number();
        let shape_id = format!("_x0000_i{}", 1024 + object_number);
        let raw = self
            .raw_document
            .as_mut()
            .ok_or("the document has no XML")?;
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
            vec![attribute("r:id", &picture), attribute("o:title", "")],
            Vec::new(),
        );
        let shape = raw.create_element(
            "v:shape",
            vec![
                attribute("id", &shape_id),
                attribute("type", "#_x0000_t75"),
                attribute("style", &format!("width:{width:.1}pt;height:{height:.1}pt")),
                attribute("o:ole", ""),
            ],
            vec![XmlNode::Element(imagedata)],
        );
        let ole = raw.create_element(
            "o:OLEObject",
            vec![
                attribute("Type", "Embed"),
                attribute("ProgID", program_id),
                attribute("ShapeID", &shape_id),
                attribute("DrawAspect", if icon { "Icon" } else { "Content" }),
                attribute("ObjectID", &format!("_{}", 1_700_000_000 + object_number)),
                attribute("r:id", &relationship.id),
            ],
            Vec::new(),
        );
        let object = raw.create_element(
            "w:object",
            vec![
                attribute("w:dxaOrig", &format!("{:.0}", width * 20.0)),
                attribute("w:dyaOrig", &format!("{:.0}", height * 20.0)),
            ],
            vec![XmlNode::Element(shape), XmlNode::Element(ole)],
        );
        let run = raw
            .element_mut(node)
            .ok_or("the picture's XML went missing")?;
        let at = run
            .children
            .iter()
            .position(|child| matches!(child, XmlNode::Element(element) if local(&element.name) == "drawing"))
            .ok_or("the picture is not a drawing")?;
        run.children[at] = XmlNode::Element(object);
        raw.rebuild_index();
        if let Some(image) = self
            .paragraph_mut(paragraph_index)
            .and_then(|paragraph| paragraph.runs.get_mut(run_index))
            .and_then(|run| run.image.as_mut())
        {
            image.ole_relationship_id = Some(relationship.id);
            image.ole_program_id = Some(program_id.to_string());
            image.description = Some(program_id.to_string());
        }
        Ok(())
    }

    /// How many embedded objects the body has, to number the next one.
    fn next_ole_object_number(&self) -> usize {
        (0..self.paragraph_count())
            .filter_map(|index| self.paragraph(index))
            .flat_map(|paragraph| paragraph.runs.iter())
            .filter(|run| {
                run.image
                    .as_ref()
                    .is_some_and(|image| image.ole_relationship_id.is_some())
            })
            .count()
            + 1
    }

    /// The bytes of the object run `run_index` of `paragraph_index` embeds,
    /// with its ProgID.
    pub fn ole_object_data(
        &self,
        paragraph_index: usize,
        run_index: usize,
    ) -> Option<(String, Vec<u8>)> {
        let image = self
            .paragraph(paragraph_index)?
            .runs
            .get(run_index)?
            .image
            .as_ref()?;
        let id = image.ole_relationship_id.as_deref()?;
        let document_part = self.document_part.as_ref()?;
        let relationship = document_part
            .relationships
            .iter()
            .find(|relationship| relationship.id == id)
            .filter(|relationship| relationship.target_mode.as_deref() != Some("External"))?;
        let path = Self::resolve_target_path(&document_part.path, &relationship.target);
        let bytes = self.binary_parts.get(&path).cloned().or_else(|| {
            self.package
                .as_ref()
                .and_then(|package| package.get(&path))
                .cloned()
        })?;
        Some((image.ole_program_id.clone().unwrap_or_default(), bytes))
    }

    /// The file a double-click on the object opens: a package's file, or an
    /// embedded Office document (`Word.Document.12`, `Excel.Sheet.12`, …)
    /// as the file it is.
    pub fn embedded_object_file(
        &self,
        paragraph_index: usize,
        run_index: usize,
    ) -> Option<EmbeddedFile> {
        let (program, bytes) = self.ole_object_data(paragraph_index, run_index)?;
        if let Some(file) = embedded_file(&bytes) {
            return Some(file);
        }
        // An Office document embedded in its own format is the file itself.
        let extension = match program.split('.').next().unwrap_or("") {
            "Word" => "docx",
            "Excel" => "xlsx",
            "PowerPoint" => "pptx",
            _ => return None,
        };
        bytes.starts_with(b"PK").then(|| EmbeddedFile {
            name: format!("{}.{extension}", program.replace('.', "_")),
            bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::insert_image::InsertImageCommand;
    use crate::commands::insert_ole_object::InsertOleObjectCommand;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::word_document::review_tools::tests::docx_with_body;
    use futures::executor;

    const PNG_1X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";

    #[test]
    fn a_package_holds_the_file_and_its_name() {
        let package = ole_package("見積書.pdf", b"%PDF-1.7 body").unwrap();
        let file = embedded_file(&package).unwrap();
        assert_eq!(file.name, "見積書.pdf");
        assert_eq!(file.bytes, b"%PDF-1.7 body");
        let compound = cfb::CompoundFile::open(std::io::Cursor::new(&package)).unwrap();
        assert_eq!(compound.root_entry().clsid(), &PACKAGE_CLSID);
    }

    #[test]
    fn an_embedded_file_is_written_as_word_writes_one_and_read_back() {
        let mut document = executor::block_on(WordDocument::load(
            &docx_with_body("<w:p><w:r><w:t>Attached:</w:t></w:r></w:p>"),
            &Options::default(),
        ))
        .unwrap();
        let png = BASE64.decode(PNG_1X1).unwrap();
        let caret = SelectionRange::collapsed(TextPosition::new(0, 9));
        let mut selection = Some(caret);
        let mut executor = CommandExecutor::new();
        executor
            .execute(
                Box::new(InsertOleObjectCommand::new(
                    InsertImageCommand::new(caret, png, "image/png".into(), 96.0, 64.0, None, None),
                    "report.csv".into(),
                    b"a,b\n1,2\n".to_vec(),
                    true,
                )),
                &mut CommandContext::new(&mut document, &mut selection),
            )
            .unwrap();
        let file = document.embedded_object_file(0, 1).unwrap();
        assert_eq!(file.name, "report.csv");
        assert_eq!(file.bytes, b"a,b\n1,2\n");

        let saved = document.export_docx().unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&saved)).unwrap();
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .unwrap()
            .read_to_string(&mut xml)
            .unwrap();
        assert!(xml.contains("<w:object"), "{xml}");
        assert!(xml.contains(r#"ProgID="Package""#), "{xml}");
        assert!(xml.contains(r#"DrawAspect="Icon""#), "{xml}");
        assert!(xml.contains("<v:imagedata"), "{xml}");
        assert!(!xml.contains("<w:drawing"), "{xml}");
        let mut types = String::new();
        archive
            .by_name("[Content_Types].xml")
            .unwrap()
            .read_to_string(&mut types)
            .unwrap();
        assert!(types.contains(OLE_CONTENT_TYPE), "{types}");
        let mut rels = String::new();
        archive
            .by_name("word/_rels/document.xml.rels")
            .unwrap()
            .read_to_string(&mut rels)
            .unwrap();
        assert!(rels.contains("embeddings/oleObject1.bin"), "{rels}");
        assert!(archive.by_name("word/embeddings/oleObject1.bin").is_ok());

        // Read back: the object is still there, its file intact, its
        // picture drawn.
        let reopened = executor::block_on(WordDocument::load(&saved, &Options::default())).unwrap();
        let (at, run) = reopened
            .paragraph(0)
            .unwrap()
            .runs
            .iter()
            .enumerate()
            .find(|(_, run)| run.image.is_some())
            .unwrap();
        let image = run.image.as_ref().unwrap();
        assert_eq!(image.ole_program_id.as_deref(), Some("Package"));
        assert!(image.relationship_id.is_some());
        assert_eq!(
            reopened.embedded_object_file(0, at).unwrap().bytes,
            b"a,b\n1,2\n"
        );

        // Undo takes the object and its part away.
        executor
            .undo(&mut CommandContext::new(&mut document, &mut selection))
            .unwrap();
        assert!(document.embedded_object_file(0, 1).is_none());
    }
}
