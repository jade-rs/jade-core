//! INCLUDEPICTURE's result: a local image file embedded as an inline
//! picture — its media part, the story's relationship to it, its content
//! type and the `w:drawing` run.

use super::write::{find_element, fresh_copy};
use crate::docx_xml::{DocxXmlDocument, XmlElement, XmlNode};
use std::collections::HashMap;

/// The pixel size and content type of a PNG, JPEG or GIF.
pub(crate) fn image_info(bytes: &[u8]) -> Option<(u32, u32, &'static str, &'static str)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 24 {
        let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
        let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
        return Some((width, height, "png", "image/png"));
    }
    if bytes.starts_with(b"GIF8") && bytes.len() >= 10 {
        let width = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
        let height = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
        return Some((width, height, "gif", "image/gif"));
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        let mut at = 2;
        while at + 9 < bytes.len() {
            if bytes[at] != 0xFF {
                at += 1;
                continue;
            }
            let marker = bytes[at + 1];
            let length = u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]) as usize;
            if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
                let height = u16::from_be_bytes([bytes[at + 5], bytes[at + 6]]) as u32;
                let width = u16::from_be_bytes([bytes[at + 7], bytes[at + 8]]) as u32;
                return Some((width, height, "jpeg", "image/jpeg"));
            }
            at += 2 + length;
        }
    }
    None
}

fn rels_path(part: &str) -> String {
    let (folder, name) = part.rsplit_once('/').unwrap_or(("", part));
    format!("{folder}/_rels/{name}.rels")
}

fn max_docpr_id(root: &XmlElement) -> u32 {
    let own = (root.name == "wp:docPr")
        .then(|| {
            root.attributes
                .iter()
                .find(|attribute| attribute.name == "id")
                .and_then(|attribute| attribute.value.parse().ok())
        })
        .flatten()
        .unwrap_or(0);
    root.children
        .iter()
        .filter_map(|child| match child {
            XmlNode::Element(child) => Some(max_docpr_id(child)),
            _ => None,
        })
        .fold(own, u32::max)
}

/// Embeds `bytes` for story `part` and returns the run that shows it, with
/// properties `rpr`. `root` is the story, for a fresh drawing id.
pub(crate) fn picture_run(
    files: &mut HashMap<String, Vec<u8>>,
    part: &str,
    root: &XmlElement,
    bytes: &[u8],
    name: &str,
    rpr: Option<&XmlElement>,
) -> Result<XmlElement, String> {
    let (width, height, extension, content_type) =
        image_info(bytes).ok_or("the file is not a PNG, JPEG or GIF picture")?;
    let mut number = 1;
    while files.contains_key(&format!("word/media/field{number}.{extension}")) {
        number += 1;
    }
    let media = format!("word/media/field{number}.{extension}");
    files.insert(media.clone(), bytes.to_vec());

    // The relationship from the story to the picture.
    let rels_path = rels_path(part);
    let rels = files
        .get(&rels_path)
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_else(|| {
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>".to_string()
        });
    let mut id = 1;
    while rels.contains(&format!("\"rId{id}\"")) {
        id += 1;
    }
    let relationship = format!("rId{id}");
    let entry = format!(
        "<Relationship Id=\"{relationship}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"media/field{number}.{extension}\"/>"
    );
    let rels = match rels.rfind("</Relationships>") {
        Some(at) => format!("{}{entry}{}", &rels[..at], &rels[at..]),
        None => rels.replacen("/>", &format!(">{entry}</Relationships>"), 1),
    };
    files.insert(rels_path, rels.into_bytes());

    if let Some(types) = files.get("[Content_Types].xml") {
        let types = String::from_utf8_lossy(types).into_owned();
        if !types.contains(&format!("Extension=\"{extension}\"")) {
            if let Some(at) = types.rfind("</Types>") {
                let entry =
                    format!("<Default Extension=\"{extension}\" ContentType=\"{content_type}\"/>");
                files.insert(
                    "[Content_Types].xml".to_string(),
                    format!("{}{entry}{}", &types[..at], &types[at..]).into_bytes(),
                );
            }
        }
    }

    // 96 dpi, at most 6 inches wide.
    const EMU_PER_PIXEL: u64 = 9525;
    const MAX_WIDTH: u64 = 6 * 914_400;
    let mut cx = width.max(1) as u64 * EMU_PER_PIXEL;
    let mut cy = height.max(1) as u64 * EMU_PER_PIXEL;
    if cx > MAX_WIDTH {
        cy = cy * MAX_WIDTH / cx;
        cx = MAX_WIDTH;
    }
    let docpr = max_docpr_id(root) + 1;
    let name = name
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;");
    let xml = format!(
        concat!(
            "<w:r xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">",
            "<w:drawing>",
            "<wp:inline xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\">",
            "<wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:docPr id=\"{docpr}\" name=\"{name}\"/>",
            "<wp:cNvGraphicFramePr><a:graphicFrameLocks xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" noChangeAspect=\"1\"/></wp:cNvGraphicFramePr>",
            "<a:graphic xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\">",
            "<a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">",
            "<pic:pic xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">",
            "<pic:nvPicPr><pic:cNvPr id=\"0\" name=\"{name}\"/><pic:cNvPicPr/></pic:nvPicPr>",
            "<pic:blipFill><a:blip xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" r:embed=\"{relationship}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>",
            "<pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr>",
            "</pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"
        ),
        cx = cx,
        cy = cy,
        docpr = docpr,
        name = name,
        relationship = relationship
    );
    let parsed = DocxXmlDocument::parse(&xml).map_err(|error| format!("{error:?}"))?;
    let mut run = fresh_copy(parsed.root());
    run.attributes.clear();
    if let Some(rpr) = rpr {
        run.children.insert(0, XmlNode::Element(fresh_copy(rpr)));
    }
    let _ = find_element;
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_picture_sizes() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&120u32.to_be_bytes());
        png.extend_from_slice(&80u32.to_be_bytes());
        assert_eq!(image_info(&png), Some((120, 80, "png", "image/png")));
        let gif = b"GIF89a\x10\x00\x20\x00".to_vec();
        assert_eq!(image_info(&gif), Some((16, 32, "gif", "image/gif")));
        let jpeg = [
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00,
            0x30, 0x00, 0x40, 0x03,
        ];
        assert_eq!(image_info(&jpeg), Some((64, 48, "jpeg", "image/jpeg")));
        assert_eq!(image_info(b"text"), None);
    }

    #[test]
    fn embeds_the_picture_with_its_relationship_and_type() {
        let mut files = HashMap::new();
        files.insert(
            "[Content_Types].xml".to_string(),
            b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"></Types>".to_vec(),
        );
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&96u32.to_be_bytes());
        png.extend_from_slice(&48u32.to_be_bytes());
        let root = XmlElement {
            id: crate::docx_xml::XmlNodeId::new(1),
            name: "w:body".into(),
            attributes: Vec::new(),
            children: Vec::new(),
        };
        let run = picture_run(
            &mut files,
            "word/document.xml",
            &root,
            &png,
            "logo.png",
            None,
        )
        .unwrap();
        assert_eq!(run.name, "w:r");
        assert!(files.contains_key("word/media/field1.png"));
        let rels = String::from_utf8(files["word/_rels/document.xml.rels"].clone()).unwrap();
        assert!(rels.contains("Target=\"media/field1.png\""));
        let types = String::from_utf8(files["[Content_Types].xml"].clone()).unwrap();
        assert!(types.contains("Extension=\"png\""));
    }
}
