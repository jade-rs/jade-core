//! ページの色 › 塗りつぶし効果: a gradient, a texture, a pattern or a
//! picture behind every page, kept as Word keeps it — VML's
//! `v:background/v:fill` under `w:background`, its picture a part of the
//! document, and `w:displayBackgroundShape` in the settings so Word shows it.

use super::*;
use crate::document::PageBackgroundFill;
use jade_docx_headless_api::HeadlessPageFill;

const VML: &str = "urn:schemas-microsoft-com:vml";
const OFFICE: &str = "urn:schemas-microsoft-com:office:office";
const RELATIONSHIPS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn rgb(hex: Option<&str>, fallback: [u8; 3]) -> [u8; 3] {
    let Some(hex) = hex.map(|hex| hex.trim_start_matches('#')) else {
        return fallback;
    };
    if hex.len() != 6 {
        return fallback;
    }
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
    match (channel(0), channel(2), channel(4)) {
        (Some(r), Some(g), Some(b)) => [r, g, b],
        _ => fallback,
    }
}

fn hex_color(color: &str) -> Option<String> {
    let hex = color.trim().trim_start_matches('#');
    (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit())).then(|| hex.to_ascii_uppercase())
}

/// Puts `fill`'s picture in the form the renderer draws: a texture or a
/// picture as it is, a pattern's bits in its foreground and background,
/// repeated to a tile of about 64 px.
pub(crate) fn prepare_page_fill_picture(
    fill: &mut PageBackgroundFill,
    bytes: &[u8],
    foreground: Option<&str>,
) {
    let Ok(picture) = image::load_from_memory(bytes) else {
        return;
    };
    let (width, height) = (picture.width().max(1), picture.height().max(1));
    if fill.kind == "pattern" {
        let foreground = rgb(foreground, [0, 0, 0]);
        let background = rgb(fill.color2.as_deref(), [255, 255, 255]);
        let bits = picture.to_luma8();
        let (across, down) = ((64 / width).max(1), (64 / height).max(1));
        let tile = image::RgbImage::from_fn(width * across, height * down, |x, y| {
            let dark = bits.get_pixel(x % width, y % height).0[0] < 128;
            image::Rgb(if dark { foreground } else { background })
        });
        let mut png = Vec::new();
        if image::DynamicImage::ImageRgb8(tile)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .is_err()
        {
            return;
        }
        fill.data_uri = Some(format!("data:image/png;base64,{}", BASE64.encode(&png)));
        fill.tile_size = Some(((width * across) as f64, (height * down) as f64));
        return;
    }
    let mime = match image::guess_format(bytes) {
        Ok(image::ImageFormat::Jpeg) => "image/jpeg",
        _ => "image/png",
    };
    fill.data_uri = Some(format!("data:{mime};base64,{}", BASE64.encode(bytes)));
    fill.tile_size = Some((width as f64, height as f64));
}

impl WordDocument {
    /// Reads the fill effect's picture from its part, at load.
    pub(super) fn resolve_page_fill(
        document: &mut Document,
        doc_part: &DocumentPart,
        package: &OpenXmlPackage,
    ) {
        if document
            .background_fill
            .as_ref()
            .is_some_and(|fill| fill.kind.is_empty() || fill.kind == "solid")
        {
            document.background_fill = None;
        }
        let foreground = document.background_color.clone();
        let Some(fill) = document.background_fill.as_mut() else {
            return;
        };
        let Some(id) = fill.relationship_id.as_deref() else {
            return;
        };
        let Some(relationship) = doc_part
            .relationships
            .iter()
            .find(|relationship| relationship.id == id)
        else {
            return;
        };
        let path = Self::resolve_target_path(&doc_part.path, &relationship.target);
        if let Some(bytes) = package.get(&path) {
            let foreground = fill.color.clone().or(foreground);
            prepare_page_fill_picture(fill, bytes, foreground.as_deref());
        }
    }

    /// The page's fill effect, as drawn.
    pub fn page_fill(&self) -> Option<&PageBackgroundFill> {
        self.document.as_ref()?.background_fill.as_ref()
    }

    /// 塗りつぶし効果: puts `fill` behind every page — its colour becomes the
    /// page colour, its picture a part of the document — or, with `None`,
    /// takes the effect off and leaves the plain page colour.
    pub fn set_page_fill(&mut self, fill: Option<&HeadlessPageFill>) -> Result<(), String> {
        let Some(fill) = fill else {
            if let Some(document) = self.document_mut() {
                document.background_fill = None;
            }
            return self.put_page_fill_vml(None);
        };
        let kind = match fill.kind.as_str() {
            "gradient" => "gradient",
            "texture" => "tile",
            "pattern" => "pattern",
            "picture" => "frame",
            other => return Err(format!("unknown page fill {other}")),
        };
        // The colour the fill starts from is the page colour; a texture or
        // a picture keeps the one there is.
        let color = fill
            .color
            .as_deref()
            .and_then(hex_color)
            .or_else(|| {
                self.document
                    .as_ref()
                    .and_then(|document| document.background_color.as_deref())
                    .and_then(hex_color)
            })
            .unwrap_or_else(|| "FFFFFF".to_string());
        self.set_page_background_color(Some(format!("#{color}")))?;
        let color2 = fill.color2.as_deref().and_then(hex_color);
        let picture = match (kind, fill.image_base64.as_deref()) {
            ("gradient", _) => None,
            (_, Some(encoded)) => Some(
                BASE64
                    .decode(encoded.trim())
                    .map_err(|error| format!("the fill's picture is not base64: {error}"))?,
            ),
            _ => return Err("a texture, pattern or picture fill needs its picture".to_string()),
        };
        let relationship_id = match picture.as_deref() {
            Some(bytes) => Some(self.add_page_fill_picture(bytes)?),
            None => None,
        };

        let mut fill_attributes = vec![];
        let mut attribute = |name: &str, value: String| {
            fill_attributes.push(XmlAttribute {
                name: name.to_string(),
                value,
            })
        };
        if let Some(id) = &relationship_id {
            attribute("r:id", id.clone());
            attribute(
                "o:title",
                fill.title.clone().unwrap_or_else(|| "picture".to_string()),
            );
        }
        if let Some(color2) = &color2 {
            if kind == "gradient" || kind == "pattern" {
                attribute("color2", format!("#{}", color2.to_ascii_lowercase()));
            }
        }
        if kind == "tile" || kind == "frame" {
            attribute("recolor", "t".to_string());
        }
        if kind == "gradient" {
            if fill.angle != 0.0 {
                attribute("angle", format!("{}", fill.angle.round() as i64));
            }
            if fill.focus != 0.0 {
                attribute("focus", format!("{}%", fill.focus.round() as i64));
            }
        }
        attribute("type", kind.to_string());
        self.put_page_fill_vml(Some((color.clone(), fill_attributes)))?;

        let mut model = PageBackgroundFill {
            kind: kind.to_string(),
            color: Some(format!("#{color}")),
            color2: color2.map(|color2| format!("#{color2}")),
            angle: fill.angle,
            focus: fill.focus,
            relationship_id,
            title: fill.title.clone(),
            data_uri: None,
            tile_size: None,
        };
        if let Some(bytes) = picture.as_deref() {
            prepare_page_fill_picture(&mut model, bytes, Some(&format!("#{color}")));
        }
        if let Some(document) = self.document_mut() {
            document.background_fill = Some(model);
        }
        Ok(())
    }

    /// The picture as a part of the main document; its relationship id.
    fn add_page_fill_picture(&mut self, bytes: &[u8]) -> Result<String, String> {
        let (extension, mime) = match image::guess_format(bytes) {
            Ok(image::ImageFormat::Jpeg) => ("jpeg", "image/jpeg"),
            Ok(image::ImageFormat::Png) => ("png", "image/png"),
            _ => return Err("the fill's picture is neither PNG nor JPEG".to_string()),
        };
        let path = self.next_image_part_path("image", extension);
        let relationship = {
            let document_part = self
                .document_part
                .as_mut()
                .ok_or("main document part is unavailable")?;
            let relationship = Relationship {
                id: Self::next_relationship_id(&document_part.relationships),
                r#type: RelationshipTypes::Image.as_str().to_string(),
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
        self.ensure_content_types_has_image(&path, extension, mime)?;
        self.removed_parts.remove(&path);
        self.binary_parts.insert(path, bytes.to_vec());
        Ok(relationship.id)
    }

    /// `w:background`'s VML: `v:background` with the colour and `fill` as
    /// its `v:fill`, or none.
    fn put_page_fill_vml(
        &mut self,
        fill: Option<(String, Vec<XmlAttribute>)>,
    ) -> Result<(), String> {
        let shown = fill.is_some();
        let Some(raw) = self.raw_document.as_mut() else {
            return Ok(());
        };
        let root_id = raw.root().id;
        let background_id = raw.element(root_id).and_then(|root| {
            root.children.iter().find_map(|node| match node {
                XmlNode::Element(element) if element.name == "w:background" => Some(element.id),
                _ => None,
            })
        });
        let Some(background_id) = background_id else {
            return Ok(());
        };
        let vml = fill.map(|(color, attributes)| {
            let fill = raw.create_element("v:fill", attributes, Vec::new());
            raw.create_element(
                "v:background",
                vec![
                    XmlAttribute {
                        name: "id".to_string(),
                        value: "_x0000_s1025".to_string(),
                    },
                    XmlAttribute {
                        name: "o:bwmode".to_string(),
                        value: "white".to_string(),
                    },
                    XmlAttribute {
                        name: "fillcolor".to_string(),
                        value: format!("#{}", color.to_ascii_lowercase()),
                    },
                    XmlAttribute {
                        name: "o:targetscreensize".to_string(),
                        value: "1024,768".to_string(),
                    },
                ],
                vec![XmlNode::Element(fill)],
            )
        });
        if let Some(root) = raw.element_mut(root_id) {
            for (prefix, uri) in [
                ("xmlns:v", VML),
                ("xmlns:o", OFFICE),
                ("xmlns:r", RELATIONSHIPS),
            ] {
                if vml.is_some()
                    && !root
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
        let background = raw
            .element_mut(background_id)
            .ok_or("the page background went missing")?;
        background.children.retain(
            |node| !matches!(node, XmlNode::Element(element) if element.name == "v:background"),
        );
        if let Some(vml) = vml {
            background.children.push(XmlNode::Element(vml));
        }
        raw.rebuild_index_for_subtree(root_id);
        // Word shows the background shape in print layout only with this.
        if shown && self.settings_xml_content().is_some() {
            self.set_settings_flag("w:displayBackgroundShape", true)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::word_document::review_tools::tests::docx_with_body;
    use futures::executor;
    use std::io::Read;

    const PNG_1X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";

    fn load(bytes: &[u8]) -> WordDocument {
        executor::block_on(WordDocument::load(bytes, &Options::default())).unwrap()
    }

    fn part(saved: &[u8], name: &str) -> String {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(saved)).unwrap();
        let mut xml = String::new();
        archive
            .by_name(name)
            .unwrap()
            .read_to_string(&mut xml)
            .unwrap();
        xml
    }

    #[test]
    fn a_gradient_is_written_as_word_writes_it_and_read_back() {
        let mut document = load(&docx_with_body("<w:p/>"));
        document
            .set_page_fill(Some(&HeadlessPageFill {
                kind: "gradient".to_string(),
                color: Some("#FFC000".to_string()),
                color2: Some("#5B9BD5".to_string()),
                angle: -90.0,
                focus: 50.0,
                ..Default::default()
            }))
            .unwrap();
        let saved = document.export_docx().unwrap();
        let xml = part(&saved, "word/document.xml");
        assert!(xml.contains(r#"<w:background w:color="FFC000">"#), "{xml}");
        assert!(xml.contains(r##"fillcolor="#ffc000""##), "{xml}");
        assert!(
            xml.contains(r##"<v:fill color2="#5b9bd5" angle="-90" focus="50%" type="gradient"/>"##),
            "{xml}"
        );
        let reopened = load(&saved);
        let fill = reopened.page_fill().expect("the fill is read");
        assert_eq!(fill.kind, "gradient");
        assert_eq!(fill.color2.as_deref(), Some("#5B9BD5"));
        assert_eq!((fill.angle, fill.focus), (-90.0, 50.0));
    }

    #[test]
    fn a_picture_fill_becomes_a_part_and_is_drawn_from_it() {
        // A package with word/settings.xml, as Word's always have.
        let hello = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../public/fixtures/hello.docx");
        let mut document = load(&std::fs::read(hello).unwrap());
        document
            .set_page_fill(Some(&HeadlessPageFill {
                kind: "pattern".to_string(),
                color: Some("#000080".to_string()),
                color2: Some("#FFFFFF".to_string()),
                image_base64: Some(PNG_1X1.to_string()),
                title: Some("10%".to_string()),
                ..Default::default()
            }))
            .unwrap();
        let drawn = document.page_fill().unwrap();
        assert_eq!(drawn.tile_size, Some((64.0, 64.0)), "repeated to a tile");
        assert!(drawn.data_uri.is_some());
        let saved = document.export_docx().unwrap();
        let xml = part(&saved, "word/document.xml");
        assert!(xml.contains(r#"type="pattern""#), "{xml}");
        assert!(xml.contains(r#"o:title="10%""#), "{xml}");
        let rels = part(&saved, "word/_rels/document.xml.rels");
        assert!(rels.contains("media/image"), "{rels}");
        let settings = part(&saved, "word/settings.xml");
        assert!(
            settings.contains("<w:displayBackgroundShape/>"),
            "{settings}"
        );

        let mut reopened = load(&saved);
        let fill = reopened.page_fill().expect("the fill is read");
        assert_eq!(fill.kind, "pattern");
        assert_eq!(fill.title.as_deref(), Some("10%"));
        assert!(fill.data_uri.is_some(), "its picture is resolved");
        // Taken off, the colour stays.
        reopened.set_page_fill(None).unwrap();
        assert!(reopened.page_fill().is_none());
        let xml = part(&reopened.export_docx().unwrap(), "word/document.xml");
        assert!(!xml.contains("v:background"), "{xml}");
        assert!(xml.contains(r#"<w:background w:color="000080"/>"#), "{xml}");
    }
}
