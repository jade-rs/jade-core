//! 図の圧縮 (ファイル サイズの圧縮): the document's pictures brought down
//! to the resolution they are shown at — 330, 220, 150 or 96 ppi of their
//! size on the page — and the parts a crop hides discarded, as Word does.
//!
//! A picture is resampled only when it has more pixels than the
//! resolution asks for its displayed size; a JPEG stays a JPEG and a PNG a
//! PNG. A crop (`a:srcRect`) is cut out of the pixels and the crop taken
//! off, so the picture looks as it did. A picture used more than once is
//! sized for its largest use, and keeps its crop unless every use has the
//! same one.

use std::collections::HashMap;
use std::io::Cursor;

use image::{DynamicImage, GenericImageView, ImageFormat};
use jade_opc::OpcPackage;

/// The resolutions Word offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Resolution {
    /// HD (330 ppi).
    Hd,
    /// 印刷用 (220 ppi).
    Print,
    /// Web (150 ppi).
    Web,
    /// 電子メール用 (96 ppi).
    Email,
}

impl Resolution {
    pub const ALL: [Resolution; 4] = [
        Resolution::Hd,
        Resolution::Print,
        Resolution::Web,
        Resolution::Email,
    ];

    pub fn ppi(self) -> u32 {
        match self {
            Resolution::Hd => 330,
            Resolution::Print => 220,
            Resolution::Web => 150,
            Resolution::Email => 96,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Resolution::Hd => "compress.hd",
            Resolution::Print => "compress.print",
            Resolution::Web => "compress.web",
            Resolution::Email => "compress.email",
        }
    }
}

/// What the compression did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Compressed {
    /// Pictures made smaller.
    pub resampled: usize,
    /// Crops cut out of their pictures.
    pub uncropped: usize,
    /// The pictures' bytes before and after.
    pub bytes_before: usize,
    pub bytes_after: usize,
}

/// One use of a picture: its relationship's part and id, its displayed
/// size in EMU, and its crop (left, top, right, bottom in thousandths of a
/// percent).
#[derive(Clone, Debug)]
struct Use {
    part: String,
    id: String,
    width: u64,
    height: u64,
    crop: Option<[i64; 4]>,
}

fn attribute(element: &str, name: &str) -> Option<String> {
    let marker = format!(" {name}=\"");
    let start = element.find(&marker)? + marker.len();
    Some(element[start..start + element[start..].find('"')?].to_string())
}

/// Every picture use in `xml`: each `a:blip` with the extent and crop of
/// the drawing it is in.
fn uses_in(part: &str, xml: &str) -> Vec<Use> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(found) = xml[at..].find("<w:drawing>") {
        let start = at + found;
        let end = xml[start..]
            .find("</w:drawing>")
            .map(|end| start + end)
            .unwrap_or(xml.len());
        let drawing = &xml[start..end];
        let extent = drawing
            .find("<wp:extent ")
            .map(|at| &drawing[at..at + drawing[at..].find('>').unwrap_or(0)]);
        let (width, height) = extent
            .map(|extent| {
                (
                    attribute(extent, "cx")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0),
                    attribute(extent, "cy")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0),
                )
            })
            .unwrap_or((0, 0));
        if let Some(blip_at) = drawing.find("<a:blip ") {
            let blip = &drawing[blip_at..blip_at + drawing[blip_at..].find('>').unwrap_or(0)];
            if let Some(id) = attribute(blip, "r:embed") {
                let crop = drawing.find("<a:srcRect").and_then(|at| {
                    let rect = &drawing[at..at + drawing[at..].find('>')?];
                    let value = |name: &str| {
                        attribute(rect, name)
                            .and_then(|v| v.parse::<i64>().ok())
                            .unwrap_or(0)
                    };
                    let crop = [value("l"), value("t"), value("r"), value("b")];
                    crop.iter().any(|side| *side != 0).then_some(crop)
                });
                out.push(Use {
                    part: part.to_string(),
                    id,
                    width,
                    height,
                    crop,
                });
            }
        }
        at = end.max(start + 1);
    }
    out
}

fn image_target(package: &OpcPackage, part: &str, id: &str) -> Option<String> {
    let rels = package.part_str(&jade_opc::rels_part_name(part)).ok()??;
    let relationships = jade_opc::parse_relationships(&rels).ok()?;
    let target = relationships.target(id)?;
    Some(jade_opc::resolve_target(part, target))
}

/// `docx` with its pictures at `resolution`, and their crops cut out when
/// `delete_crops`.
pub fn compress_pictures(
    docx: &[u8],
    resolution: Resolution,
    delete_crops: bool,
) -> Result<(Vec<u8>, Compressed), String> {
    let mut package = OpcPackage::open(docx).map_err(|error| error.to_string())?;
    let parts: Vec<String> = package
        .part_names()
        .iter()
        .filter(|name| {
            name.starts_with("word/")
                && name.ends_with(".xml")
                && (name.contains("document")
                    || name.contains("header")
                    || name.contains("footer")
                    || name.contains("notes"))
        })
        .cloned()
        .collect();
    let mut uses: HashMap<String, Vec<Use>> = HashMap::new();
    for part in &parts {
        let Some(xml) = package.part_str(part).ok().flatten() else {
            continue;
        };
        for picture in uses_in(part, &xml) {
            if let Some(target) = image_target(&package, part, &picture.id) {
                uses.entry(target).or_default().push(picture);
            }
        }
    }
    let mut report = Compressed::default();
    let mut uncropped_parts: Vec<(String, String)> = Vec::new();
    let ppi = resolution.ppi() as f64;
    for (target, list) in &uses {
        let Some(bytes) = package.part(target).map(<[u8]>::to_vec) else {
            continue;
        };
        let Ok(format) = image::guess_format(&bytes) else {
            continue;
        };
        if !matches!(format, ImageFormat::Png | ImageFormat::Jpeg) {
            continue;
        }
        let Ok(mut picture) = image::load_from_memory_with_format(&bytes, format) else {
            continue;
        };
        // A crop comes out only when every use has the same one.
        let crop = list[0].crop;
        let same_crop = list.iter().all(|other| other.crop == crop);
        let mut cropped = false;
        if delete_crops && same_crop {
            if let Some([left, top, right, bottom]) = crop {
                let (width, height) = picture.dimensions();
                let x = (width as i64 * left.max(0) / 100_000) as u32;
                let y = (height as i64 * top.max(0) / 100_000) as u32;
                let keep_width =
                    (width as i64 * (100_000 - left.max(0) - right.max(0)) / 100_000).max(1) as u32;
                let keep_height = (height as i64 * (100_000 - top.max(0) - bottom.max(0)) / 100_000)
                    .max(1) as u32;
                if x + keep_width <= width && y + keep_height <= height {
                    picture = picture.crop_imm(x, y, keep_width, keep_height);
                    cropped = true;
                }
            }
        }
        // The largest use decides how many pixels are needed.
        let shown_width =
            list.iter().map(|picture| picture.width).max().unwrap_or(0) as f64 / 914_400.0;
        let shown_height =
            list.iter().map(|picture| picture.height).max().unwrap_or(0) as f64 / 914_400.0;
        let (width, height) = picture.dimensions();
        let wanted_width = (shown_width * ppi).ceil() as u32;
        let wanted_height = (shown_height * ppi).ceil() as u32;
        let resample = wanted_width > 0
            && wanted_height > 0
            && (width > wanted_width + 1 || height > wanted_height + 1);
        if resample {
            let scale = (wanted_width as f64 / width as f64)
                .max(wanted_height as f64 / height as f64)
                .min(1.0);
            let new_width = ((width as f64 * scale).round() as u32).max(1);
            let new_height = ((height as f64 * scale).round() as u32).max(1);
            picture =
                picture.resize_exact(new_width, new_height, image::imageops::FilterType::Lanczos3);
        }
        if !resample && !cropped {
            continue;
        }
        let encoded = encode(&picture, format)?;
        // Smaller, or the crop went: otherwise the original stays.
        if encoded.len() >= bytes.len() && !cropped {
            continue;
        }
        report.bytes_before += bytes.len();
        report.bytes_after += encoded.len();
        if resample {
            report.resampled += 1;
        }
        if cropped {
            report.uncropped += 1;
            for picture in list {
                uncropped_parts.push((picture.part.clone(), picture.id.clone()));
            }
        }
        package.replace(target, encoded);
    }
    // Crops that came out of the pixels come off the drawings.
    for part in &parts {
        let ids: Vec<&String> = uncropped_parts
            .iter()
            .filter(|(used_in, _)| used_in == part)
            .map(|(_, id)| id)
            .collect();
        if ids.is_empty() {
            continue;
        }
        let Some(xml) = package.part_str(part).ok().flatten() else {
            continue;
        };
        let mut out = String::with_capacity(xml.len());
        let mut at = 0;
        while let Some(found) = xml[at..].find("<w:drawing>") {
            let start = at + found;
            let end = xml[start..]
                .find("</w:drawing>")
                .map(|end| start + end)
                .unwrap_or(xml.len());
            let drawing = &xml[start..end];
            out.push_str(&xml[at..start]);
            let ours = ids
                .iter()
                .any(|id| drawing.contains(&format!("r:embed=\"{id}\"")));
            if ours {
                out.push_str(&strip_crop(drawing));
            } else {
                out.push_str(drawing);
            }
            at = end;
        }
        out.push_str(&xml[at..]);
        package.replace(part, out.into_bytes());
    }
    let written = package.write().map_err(|error| error.to_string())?;
    Ok((written, report))
}

/// A drawing with its `a:srcRect` made empty.
fn strip_crop(drawing: &str) -> String {
    let Some(start) = drawing.find("<a:srcRect") else {
        return drawing.to_string();
    };
    let Some(end) = drawing[start..].find('>') else {
        return drawing.to_string();
    };
    let element = &drawing[start..start + end + 1];
    let replacement = if element.ends_with("/>") {
        "<a:srcRect/>".to_string()
    } else {
        "<a:srcRect>".to_string()
    };
    format!(
        "{}{replacement}{}",
        &drawing[..start],
        &drawing[start + end + 1..]
    )
}

fn encode(picture: &DynamicImage, format: ImageFormat) -> Result<Vec<u8>, String> {
    let mut out = Cursor::new(Vec::new());
    match format {
        ImageFormat::Jpeg => {
            let rgb = DynamicImage::ImageRgb8(picture.to_rgb8());
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85);
            rgb.write_with_encoder(encoder)
                .map_err(|error| error.to_string())?;
        }
        _ => {
            picture
                .write_to(&mut out, ImageFormat::Png)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A document with one picture of `pixels` square shown `inches` square.
    fn document(pixels: u32, inches: f64, crop: Option<&str>) -> Vec<u8> {
        let picture = image::RgbImage::from_fn(pixels, pixels, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        });
        let mut png = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(picture)
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        let emu = (inches * 914_400.0) as u64;
        let mut paragraph = jade_docx_convert::flow::Paragraph::default();
        paragraph
            .inlines
            .push(jade_docx_convert::flow::Inline::Image(
                jade_docx_convert::flow::Image {
                    data: png.into_inner(),
                    format: jade_docx_convert::flow::ImageFormat::Png,
                    width: emu,
                    height: emu,
                    description: None,
                },
            ));
        let docx = jade_docx_convert::write_docx(&jade_docx_convert::FlowDocument::single(vec![
            jade_docx_convert::flow::Block::Paragraph(paragraph),
        ]))
        .unwrap();
        match crop {
            None => docx,
            Some(crop) => {
                let mut package = OpcPackage::open(&docx).unwrap();
                let xml = package.part_str("word/document.xml").unwrap().unwrap();
                package.replace(
                    "word/document.xml",
                    xml.replace("<a:stretch>", &format!("{crop}<a:stretch>"))
                        .into_bytes(),
                );
                package.write().unwrap()
            }
        }
    }

    fn picture_size(docx: &[u8]) -> (u32, u32) {
        let package = OpcPackage::open(docx).unwrap();
        let name = package
            .part_names()
            .iter()
            .find(|name| name.starts_with("word/media/"))
            .unwrap()
            .clone();
        image::load_from_memory(package.part(&name).unwrap())
            .unwrap()
            .dimensions()
    }

    #[test]
    fn a_picture_comes_down_to_the_resolution_it_is_shown_at() {
        // 1200 px shown 2 inches across: 600 ppi.
        let docx = document(1200, 2.0, None);
        let (compressed, report) = compress_pictures(&docx, Resolution::Web, false).unwrap();
        assert_eq!(report.resampled, 1);
        assert_eq!(picture_size(&compressed), (300, 300));
        assert!(report.bytes_after < report.bytes_before);
        // Already at 96 ppi: left as it is at 150.
        let small = document(150, 2.0, None);
        let (_, report) = compress_pictures(&small, Resolution::Web, false).unwrap();
        assert_eq!(report, Compressed::default());
    }

    #[test]
    fn a_crop_is_cut_out_and_taken_off() {
        // A quarter cropped from the left and the top.
        let docx = document(400, 4.0, Some(r#"<a:srcRect l="25000" t="25000"/>"#));
        let (compressed, report) = compress_pictures(&docx, Resolution::Hd, true).unwrap();
        assert_eq!(report.uncropped, 1);
        assert_eq!(picture_size(&compressed), (300, 300));
        let xml = OpcPackage::open(&compressed)
            .unwrap()
            .part_str("word/document.xml")
            .unwrap()
            .unwrap();
        assert!(xml.contains("<a:srcRect/>"));
        assert!(!xml.contains(r#"l="25000""#));
        // Without deleting crops, the crop stays.
        let (kept, _) = compress_pictures(&docx, Resolution::Hd, false).unwrap();
        let xml = OpcPackage::open(&kept)
            .unwrap()
            .part_str("word/document.xml")
            .unwrap()
            .unwrap();
        assert!(xml.contains(r#"l="25000""#));
    }
}
