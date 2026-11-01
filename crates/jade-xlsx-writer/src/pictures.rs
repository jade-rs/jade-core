//! Insert ▸ Pictures: an image put on a sheet as a new object in its
//! drawing, at its own size from a cell's top-left corner — the image a
//! media part of its own, shared by type through a `Default` content type.

use jade_opc::OpcPackage;
use jade_xlsx_reader::CellRef;

use crate::charts::{append_to_drawing, drawing_for};
use crate::notes::{add_relationship, ensure_default, free_part_name, relative_target};
use crate::patch::escape;
use crate::{Result, WriteError};

const DRAWINGML_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const RELATIONSHIPS_NS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const IMAGE_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
/// English Metric Units in a CSS px at 96 an inch.
const EMU_PER_PX: f64 = 9525.0;

/// The extension and content type of a PNG, JPEG or GIF, by its first
/// bytes; `None` for anything else.
pub fn picture_type(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("png", "image/png"))
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(("jpeg", "image/jpeg"))
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(("gif", "image/gif"))
    } else {
        None
    }
}

/// Puts `image` on the sheet with its top-left at `at`'s, `size` px wide
/// and tall, as a new picture in the sheet's drawing (made when it has
/// none) named `name`; returns the object's id.
pub fn insert_picture(
    package: &mut OpcPackage,
    sheet_part: &str,
    image: &[u8],
    at: CellRef,
    size: (f64, f64),
    name: &str,
) -> Result<u32> {
    let (extension, content_type) =
        picture_type(image).ok_or(WriteError::Picture("a picture must be PNG, JPEG or GIF"))?;
    let drawing = drawing_for(package, sheet_part)?;
    let media = free_part_name(package, "xl/media/image", &format!(".{extension}"));
    package.replace(&media, image.to_vec());
    ensure_default(package, extension, content_type)?;
    let drawing_rels = jade_opc::rels_part_name(&drawing);
    let (rels, relationship) = add_relationship(
        package.part_str(&drawing_rels)?.as_deref(),
        IMAGE_TYPE,
        &relative_target(&drawing, &media),
    )?;
    package.replace(&drawing_rels, rels.into_bytes());
    let emu = |px: f64| (px.max(1.0) * EMU_PER_PX).round() as i64;
    let (cx, cy) = (emu(size.0), emu(size.1));
    let name = escape(name);
    append_to_drawing(package, &drawing, |p, id| {
        format!(
            "<{p}oneCellAnchor><{p}from><{p}col>{}</{p}col><{p}colOff>0</{p}colOff><{p}row>{}</{p}row><{p}rowOff>0</{p}rowOff></{p}from><{p}ext cx=\"{cx}\" cy=\"{cy}\"/><{p}pic xmlns:a=\"{DRAWINGML_NS}\"><{p}nvPicPr><{p}cNvPr id=\"{id}\" name=\"{name}\"/><{p}cNvPicPr><a:picLocks noChangeAspect=\"1\"/></{p}cNvPicPr></{p}nvPicPr><{p}blipFill><a:blip xmlns:r=\"{RELATIONSHIPS_NS}\" r:embed=\"{relationship}\"/><a:stretch><a:fillRect/></a:stretch></{p}blipFill><{p}spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></{p}spPr></{p}pic><{p}clientData/></{p}oneCellAnchor>",
            at.column, at.row
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_png_jpeg_and_gif_by_their_first_bytes() {
        assert_eq!(picture_type(b"\x89PNG\r\n\x1a\n...").unwrap().0, "png");
        assert_eq!(picture_type(&[0xff, 0xd8, 0xff, 0xe0]).unwrap().0, "jpeg");
        assert_eq!(picture_type(b"GIF89a..").unwrap().0, "gif");
        assert!(picture_type(b"<svg").is_none());
    }
}
