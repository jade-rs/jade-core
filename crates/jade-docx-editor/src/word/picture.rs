//! 挿入 → 画像, and what the 図の形式 tab does to a picture, over any
//! [`WordSession`]: a picture put in at the caret at its own size (shrunk to
//! fit between the margins, as Word does), moved and resized, and its
//! 文字列の折り返し — in line with the text, or floating with the text
//! around, above or below it.

use crate::commands::format_image::{FormatImageCommand, ImageFormatSpec, ImageLayoutPreset};
use crate::commands::insert_image::InsertImageCommand;
use crate::commands::transform_object::TransformObjectCommand;
use crate::word_document::{FloatingObjectKind, FloatingObjectTarget, FloatingObjectTransform};
use jade_selection::{SelectionRange, TextPosition};

use super::page::WordPage;
use super::WordSession;

pub use crate::commands::format_image::ImageLayoutPreset as Wrap;
pub use crate::word_document::{
    FloatingObjectKind as ObjectKind, FloatingObjectTarget as ObjectTarget,
};

/// CSS px per point.
const PX_PER_PT: f64 = 96.0 / 72.0;

/// The picture formats a document can hold, by their bytes.
pub fn picture_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if is_svg(bytes) {
        Some("image/svg+xml")
    } else {
        None
    }
}

/// Whether `bytes` are an SVG document.
pub fn is_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(1024)];
    let Ok(text) = std::str::from_utf8(head) else {
        return false;
    };
    let text = text.trim_start_matches('\u{feff}').trim_start();
    (text.starts_with("<?xml") || text.starts_with("<svg") || text.starts_with("<!--"))
        && text.contains("<svg")
}

/// A picture ready to go into a document: the bitmap Word shows (an SVG's
/// PNG fallback, as Word writes one) and the SVG itself when it is one.
pub struct PreparedPicture {
    pub bitmap: Vec<u8>,
    pub mime: &'static str,
    pub extension: &'static str,
    pub svg: Option<Vec<u8>>,
    /// Pixels, at 96 an inch.
    pub width: u32,
    pub height: u32,
}

/// Checks a picture's format and size, and draws an SVG's fallback.
pub fn prepare_picture(bytes: Vec<u8>) -> Result<PreparedPicture, String> {
    let mime = picture_type(&bytes).ok_or("not a PNG, JPEG, GIF or SVG picture")?;
    if mime == "image/svg+xml" {
        let (png, width, height) = svg_fallback(&bytes)?;
        return Ok(PreparedPicture {
            bitmap: png,
            mime: "image/png",
            extension: "png",
            svg: Some(bytes),
            width,
            height,
        });
    }
    let (width, height) = picture_size(&bytes).ok_or("the picture has no size")?;
    let extension = match mime {
        "image/png" => "png",
        "image/jpeg" => "jpeg",
        _ => "gif",
    };
    Ok(PreparedPicture {
        bitmap: bytes,
        mime,
        extension,
        svg: None,
        width,
        height,
    })
}

/// An SVG drawn at its own size (twice that, for a sharp fallback), as the
/// PNG Word stores beside it for readers without SVG.
pub fn svg_fallback(svg: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default())
        .map_err(|error| format!("the SVG cannot be read: {error}"))?;
    let size = tree.size();
    let (width, height) = (
        size.width().round().max(1.0) as u32,
        size.height().round().max(1.0) as u32,
    );
    const SCALE: f32 = 2.0;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(
        (width as f32 * SCALE).ceil() as u32,
        (height as f32 * SCALE).ceil() as u32,
    )
    .ok_or("the SVG is too large")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(SCALE, SCALE),
        &mut pixmap.as_mut(),
    );
    let png = pixmap
        .encode_png()
        .map_err(|error| format!("the SVG's fallback cannot be written: {error}"))?;
    Ok((png, width, height))
}

/// A picture's size in pixels, read from its header.
pub fn picture_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let be32 = |at: usize| -> Option<u32> {
        Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    let be16 = |at: usize| -> Option<u32> {
        Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as u32)
    };
    match picture_type(bytes)? {
        "image/png" => Some((be32(16)?, be32(20)?)),
        "image/gif" => {
            let le16 = |at: usize| -> Option<u32> {
                Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as u32)
            };
            Some((le16(6)?, le16(8)?))
        }
        _ => {
            // JPEG: the first start-of-frame marker holds the size.
            let mut at = 2;
            while at + 9 < bytes.len() {
                if bytes[at] != 0xff {
                    at += 1;
                    continue;
                }
                let marker = bytes[at + 1];
                let length = be16(at + 2)? as usize;
                let start_of_frame =
                    matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc);
                if start_of_frame {
                    return Some((be16(at + 7)?, be16(at + 5)?));
                }
                at += 2 + length;
            }
            None
        }
    }
}

/// A picture or shape on a page, as a click found it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedObject {
    pub target: FloatingObjectTarget,
    /// The page it is on, 0-based.
    pub page: usize,
    /// Its box on its page, in CSS px.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl PlacedObject {
    pub fn is_picture(&self) -> bool {
        self.target.kind == FloatingObjectKind::Image
    }
}

/// Pictures for every session.
pub trait WordPicture: WordSession + WordPage {
    /// 画像 → このデバイス: `bytes` (PNG, JPEG or GIF) in line at the caret,
    /// in place of the selection's start. Its own size, at 96 pixels an
    /// inch, unless that is wider than the text: then shrunk to the text's
    /// width, its proportions kept.
    fn insert_picture(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        let picture = prepare_picture(bytes)?;
        let (mime, width, height) = (picture.mime, picture.width, picture.height);
        let page = self.page_setup();
        let text_width =
            f64::from(page.width - page.margins.left - page.margins.right).max(1.0) * PX_PER_PT;
        let (mut width, mut height) = (f64::from(width.max(1)), f64::from(height.max(1)));
        if width > text_width {
            height *= text_width / width;
            width = text_width;
        }
        let range = self.selection_range().ok_or("no caret")?;
        let (start, _) = range.normalized();
        let caret = SelectionRange::collapsed(TextPosition::new(start.paragraph, start.offset));
        self.set_selection_range(Some(caret));
        let insert = InsertImageCommand::new(
            caret,
            picture.bitmap,
            mime.to_string(),
            width,
            height,
            None,
            None,
        );
        match picture.svg {
            // An SVG goes in as Word puts one in: its PNG fallback as the
            // picture, the SVG in the blip's extension.
            Some(svg) => self.execute_command(Box::new(InsertSvgCommand::new(insert, svg)), false),
            None => self.execute_command(Box::new(insert), false),
        }
    }

    /// Moves and resizes a picture or shape to `rect` (x, y, width, height
    /// on its page, in CSS px) — a drag's end — as one undo step.
    fn place_object(
        &mut self,
        object: PlacedObject,
        (x, y, width, height): (f64, f64, f64, f64),
    ) -> Result<(), String> {
        self.execute_command(
            Box::new(TransformObjectCommand::new(
                object.target,
                FloatingObjectTransform {
                    x,
                    y,
                    width: width.max(1.0),
                    height: height.max(1.0),
                    rotation_deg: 0.0,
                },
            )),
            true,
        )
    }

    /// 文字列の折り返し: in line with the text, square, tight, through, top and
    /// bottom, behind or in front of it.
    fn set_picture_wrap(
        &mut self,
        object: PlacedObject,
        wrap: ImageLayoutPreset,
    ) -> Result<(), String> {
        if !object.is_picture() {
            return Err("only a picture wraps".to_string());
        }
        self.execute_command(
            Box::new(FormatImageCommand::new(
                object.target,
                ImageFormatSpec {
                    layout: Some(wrap),
                    ..Default::default()
                },
            )),
            true,
        )
    }
}

impl<T: WordSession + ?Sized> WordPicture for T {}

/// A picture inserted, then given its SVG: one undo step.
struct InsertSvgCommand {
    insert: InsertImageCommand,
    svg: Vec<u8>,
    edit: Option<crate::commands::snapshot_edit::SnapshotEditCommand>,
}

impl InsertSvgCommand {
    fn new(insert: InsertImageCommand, svg: Vec<u8>) -> Self {
        Self {
            insert,
            svg,
            edit: None,
        }
    }
}

impl crate::commands::Command for InsertSvgCommand {
    fn name(&self) -> &'static str {
        "insert_svg"
    }

    fn apply(
        &mut self,
        ctx: &mut crate::commands::CommandContext<'_>,
    ) -> crate::commands::CommandResult<()> {
        use crate::commands::CommandError;
        self.insert.apply(ctx)?;
        let target = self
            .insert
            .inserted_target()
            .ok_or_else(|| CommandError::Custom("the picture did not go in".into()))?;
        let svg = self.svg.clone();
        let mut edit = crate::commands::edit_drawing::edit_drawing_command(
            "insert_svg",
            target,
            move |document, drawing| {
                let relationship = document.add_object_media(&svg, "svg", "image/svg+xml")?;
                let blip = crate::drawing_xml::descendant_mut(drawing, "a:blip")
                    .ok_or("the picture has no image")?;
                super::picture_format::set_svg(blip, Some(&relationship))
            },
        );
        let selection = *ctx.selection;
        if let Err(error) = edit.apply(ctx) {
            let _ = self.insert.undo(ctx);
            return Err(error);
        }
        *ctx.selection = selection;
        self.edit = Some(edit);
        Ok(())
    }

    fn undo(
        &mut self,
        ctx: &mut crate::commands::CommandContext<'_>,
    ) -> crate::commands::CommandResult<()> {
        if let Some(edit) = self.edit.as_mut() {
            edit.undo(ctx)?;
        }
        self.insert.undo(ctx)
    }
}

/// The 文字列の折り返し menu, in Word's order.
pub const WRAPS: [Wrap; 7] = [
    Wrap::Inline,
    Wrap::Square,
    Wrap::Tight,
    Wrap::Through,
    Wrap::TopAndBottom,
    Wrap::BehindText,
    Wrap::InFrontOfText,
];

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3×2 PNG's first bytes: the signature and the IHDR chunk.
    const PNG_3X2: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0,
        0, 3, 0, 0, 0, 2, 8, 6, 0, 0, 0,
    ];

    #[test]
    fn sizes_come_from_the_headers() {
        assert_eq!(picture_type(PNG_3X2), Some("image/png"));
        assert_eq!(picture_size(PNG_3X2), Some((3, 2)));
        let gif = b"GIF89a\x05\x00\x07\x00rest";
        assert_eq!(picture_size(gif), Some((5, 7)));
        // SOI, an APP0 segment, then SOF0 with 8 bits, 2 high, 4 wide.
        let jpeg = [
            0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0, 0xff, 0xc0, 0, 11, 8, 0, 2, 0, 4, 3, 0, 0, 0,
        ];
        assert_eq!(picture_size(&jpeg), Some((4, 2)));
        assert_eq!(picture_type(b"not a picture"), None);
    }
}
