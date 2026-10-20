//! Preserving slide text/object edits and insertion of ordinary text boxes.
//! Fields, tables, diagrams, inherited content and unrelated parts remain intact.

pub use xml::layouts::{
    reset_slide, reset_slides, set_slide_layout, set_slide_layouts, slide_layouts,
    validate_slide_layout, validate_slide_layouts, validate_slide_reset, validate_slide_resets,
    SlideLayoutInfo,
};
mod shape_insertion;
mod xml;
pub use shape_insertion::{insert_shape, validate_shape_insert, ShapeInsert, ShapePreset};
pub use xml::shape_style::{
    set_shape_style, shape_style, validate_shape_style, ShapePaintInfo, ShapeStyleEdit,
    ShapeStyleInfo,
};

use jade_opc::{ArchiveSnapshot, OpcPackage, PreservingOpcPackage, SnapshotStore};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Error)]
pub enum WriteError {
    #[error(transparent)]
    Package(#[from] jade_opc::OpcError),
    #[error(transparent)]
    Presentation(#[from] jade_pptx_reader::PptxError),
    #[error("invalid slide XML: {0}")]
    Xml(String),
    #[error("the deck has no slide {0}")]
    NoSlide(usize),
    #[error("the slide has no uniquely identified writable run at {0:?}")]
    NoRun(RunAddress),
    #[error("run text contains an unsupported character U+{0:04X}")]
    InvalidCharacter(u32),
    #[error("the slide has no uniquely identified editable shape {0}")]
    NoShape(u32),
    #[error(
        "shape {0} has unsupported references or alternate-content structure for this operation"
    )]
    UnsupportedObjectOperation(u32),
    #[error("shape {0} locks this transform operation")]
    LockedShape(u32),
    #[error("shape coordinates or extents are outside PowerPoint's supported range")]
    InvalidTransform,
    #[error("invalid shape fill, outline color or outline width")]
    InvalidShapeStyle,
    #[error("text selection is outside the shape or splits a UTF-16 surrogate pair")]
    InvalidTextRange,
    #[error("text changed since the input event started")]
    StaleTextInput,
    #[error("shape {0} has no editable text body")]
    NoTextBody(u32),
    #[error("shape {0} locks text editing")]
    TextLocked(u32),
    #[error("shape {0} contains fields or unsupported inline content")]
    UnsupportedTextContent(u32),
    #[error("font size must be finite and between 1 and 4000 points")]
    InvalidTextFormat,
    #[error("typeface must be nonempty and contain valid XML attribute characters")]
    InvalidTypeface,
    #[error("text color must be #RRGGBB")]
    InvalidTextColor,
    #[error("paragraph format is outside DrawingML bounds or has an invalid bullet")]
    InvalidParagraphFormat,
    #[error("invalid slide order or insertion position")]
    InvalidSlideOrder,
    #[error("invalid or unsupported picture: {0}")]
    InvalidPicture(String),
    #[error("crop must retain a nonempty region of the picture")]
    InvalidPictureCrop,
    #[error("grouping requires at least two distinct, groupable top-level objects")]
    InvalidGroupSelection,
}

pub use xml::crop::{picture_crop, set_picture_crop, validate_picture_crop_edit, PictureCropInfo};
pub use xml::pictures::{
    insert_picture, picture_info, picture_replacement, replace_picture, PictureInfo,
    PictureInsertResult, PictureReplaceResult, PictureReplacementInfo,
};

mod paragraph;
pub use paragraph::{
    ParagraphAlignment, ParagraphBullet, ParagraphFormat, ParagraphFormatEdit, ParagraphSpacing,
};

mod text_input;
pub use text_input::{NotesInputEdit, TextInputEdit, TextInputResult, TypingKind};

pub type Result<T> = std::result::Result<T, WriteError>;

/// Zero-based paragraph and ordinary-run indexes, scoped to a shape's id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunAddress {
    pub shape_id: u32,
    pub paragraph: usize,
    pub run: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableRun {
    pub address: RunAddress,
    pub shape_name: String,
    pub text: String,
    pub locked: bool,
}

/// A table cell's text — paragraphs joined by `\n` — and whether it belongs
/// to a merge another cell starts, which only that cell edits.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableCell {
    pub text: String,
    pub merged: bool,
    /// How many rows and columns the cell spans: 1 each unless it starts a
    /// merge.
    pub row_span: usize,
    pub column_span: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextSnapshot {
    pub text: String,
    pub can_edit: bool,
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextRangeEdit {
    pub shape_id: u32,
    pub start: usize,
    pub end: usize,
    pub text: String,
    #[serde(default)]
    pub soft_break: bool,
}

/// Omitted properties retain their existing value. Size is in authored points.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextFormat {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub size_pt: Option<f64>,
    pub font: Option<String>,
    pub east_asian_font: Option<String>,
    /// Opaque RGB in #RRGGBB notation.
    pub color: Option<String>,
}

impl TextFormat {
    pub fn validate(&self) -> Result<()> {
        if self
            .size_pt
            .is_some_and(|size| !size.is_finite() || !(1.0..=4000.0).contains(&size))
        {
            return Err(WriteError::InvalidTextFormat);
        }
        for font in [&self.font, &self.east_asian_font].into_iter().flatten() {
            if font.trim().is_empty()
                || font
                    .chars()
                    .any(|c| c < ' ' || matches!(c, '\u{fffe}' | '\u{ffff}'))
            {
                return Err(WriteError::InvalidTypeface);
            }
        }
        if self.color.as_ref().is_some_and(|c| {
            c.len() != 7 || !c.starts_with('#') || !c[1..].bytes().all(|b| b.is_ascii_hexdigit())
        }) {
            return Err(WriteError::InvalidTextColor);
        }
        Ok(())
    }
    pub fn is_empty(&self) -> bool {
        self.bold.is_none()
            && self.italic.is_none()
            && self.underline.is_none()
            && self.size_pt.is_none()
            && self.font.is_none()
            && self.east_asian_font.is_none()
            && self.color.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextFormatEdit {
    pub shape_id: u32,
    pub start: usize,
    pub end: usize,
    pub format: TextFormat,
}

/// Formats selected characters/paragraph marks without replacing their text.
/// A collapsed range changes only an empty paragraph's insertion properties.
pub fn format_text_range(bytes: &[u8], slide: usize, edit: &TextFormatEdit) -> Result<Vec<u8>> {
    edit.format.validate()?;
    let (mut package, part, xml) = slide_xml(bytes, slide)?;
    let changed = xml::format_text_range(&xml, edit)?;
    if changed == xml {
        return Ok(bytes.to_vec());
    }
    package.replace(&part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}

/// Applies to paragraphs touched by a UTF-16 selection, or the caret's paragraph.
pub fn format_paragraphs(
    bytes: &[u8],
    slide: usize,
    edit: &ParagraphFormatEdit,
) -> Result<Vec<u8>> {
    edit.format.validate()?;
    let (mut package, part, xml) = slide_xml(bytes, slide)?;
    let changed = xml::format_paragraphs(&xml, edit)?;
    if changed == xml {
        return Ok(bytes.to_vec());
    }
    package.replace(&part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}

/// Finds one grapheme-safe splice between two browser text values. Formatting
/// outside that splice remains attached to its existing runs and paragraphs.
pub fn text_edit_between(
    shape_id: u32,
    before: &str,
    after: &str,
    soft_break: bool,
) -> TextRangeEdit {
    let a: Vec<_> = before.graphemes(true).collect();
    let b: Vec<_> = after.graphemes(true).collect();
    let prefix = a.iter().zip(&b).take_while(|(a, b)| a == b).count();
    let limit = a.len().min(b.len()) - prefix;
    let suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(limit)
        .take_while(|(a, b)| a == b)
        .count();
    let start = a[..prefix].iter().map(|g| g.encode_utf16().count()).sum();
    let end = before.encode_utf16().count()
        - a[a.len() - suffix..]
            .iter()
            .map(|g| g.encode_utf16().count())
            .sum::<usize>();
    let text = b[prefix..b.len() - suffix].concat();
    TextRangeEdit {
        shape_id,
        start,
        end,
        text,
        soft_break,
    }
}

pub fn shape_text(bytes: &[u8], slide: usize, id: u32) -> Result<TextSnapshot> {
    let (_, _, xml) = slide_xml(bytes, slide)?;
    xml::text_snapshot(&xml, id)
}
pub fn notes_text(bytes: &[u8], slide: usize) -> Result<TextSnapshot> {
    xml::notes_snapshot(bytes, slide)
}
pub fn set_slide_notes(bytes: &[u8], slide: usize, text: &str) -> Result<Vec<u8>> {
    set_slide_notes_with_breaks(bytes, slide, text, false)
}
pub fn set_slide_notes_with_breaks(
    bytes: &[u8],
    slide: usize,
    text: &str,
    soft_break: bool,
) -> Result<Vec<u8>> {
    validate_text(text, true)?;
    xml::set_notes(bytes, slide, text, soft_break)
}
fn validate_text(text: &str, newline: bool) -> Result<()> {
    for ch in text.chars() {
        if !((newline && ch == '\n')
            || matches!(ch, '\t' | ' '..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}'))
        {
            return Err(WriteError::InvalidCharacter(ch as u32));
        }
    }
    Ok(())
}

/// Offsets are UTF-16 code units, matching browser selection APIs. Newlines
/// insert paragraphs by default, or explicit line breaks when `soft_break`.
pub fn replace_text_range(
    bytes: &[u8],
    slide: usize,
    id: u32,
    start: usize,
    end: usize,
    text: &str,
    soft_break: bool,
) -> Result<Vec<u8>> {
    validate_text(text, true)?;
    let (mut package, part, xml) = slide_xml(bytes, slide)?;
    let changed = xml::replace_text_range(&xml, id, start, end, text, soft_break)?;
    if changed == xml {
        return Ok(bytes.to_vec());
    }
    package.replace(&part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}

/// Direct slide shapes, pictures and groups. Group children retain local coordinates.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeTransformInfo {
    pub id: u32,
    pub name: String,
    pub kind: String,
    pub transform: Option<jade_pptx_reader::Transform>,
    pub can_move: bool,
    pub can_resize: bool,
    pub can_rotate: bool,
    pub can_select: bool,
    pub lock_aspect_ratio: bool,
    pub can_duplicate: bool,
    pub can_delete: bool,
    pub can_arrange: bool,
    pub can_group: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObjectArrangement {
    Front,
    Back,
    Forward,
    Backward,
}

pub fn validate_text_box(
    transform: jade_pptx_reader::Transform,
    text: &str,
    format: &TextFormat,
) -> Result<()> {
    validate_transform(transform)?;
    if transform.extent_emu.0 == 0 || transform.extent_emu.1 == 0 {
        return Err(WriteError::InvalidTransform);
    }
    validate_text(text, true)?;
    format.validate()
}

/// Adds an ordinary, fixed-bounds text box at the front of the slide's object stack.
pub fn insert_text_box(
    bytes: &[u8],
    slide: usize,
    transform: jade_pptx_reader::Transform,
    text: &str,
    format: &TextFormat,
) -> Result<(Vec<u8>, u32)> {
    validate_text_box(transform, text, format)?;
    let (mut package, part, xml) = slide_xml(bytes, slide)?;
    let (changed, id) = xml::insert_text_box(&xml, transform, text, format)?;
    package.replace(&part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok((result, id))
}

/// Clones a complete top-level object, preserving its media relationships and styles.
pub fn duplicate_shape(bytes: &[u8], slide: usize, id: u32) -> Result<(Vec<u8>, u32)> {
    let (mut package, part, xml) = slide_xml(bytes, slide)?;
    let deck = jade_pptx_reader::read_deck(bytes)?;
    let (changed, new_id) =
        xml::duplicate_shape(&xml, id, inherited_shape_transform(&deck, slide, id))?;
    package.replace(&part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok((result, new_id))
}
/// Copies top-level objects `ids` of slide `from` onto slide `to`, in front
/// of its own objects, as PowerPoint pastes them: onto another slide where
/// they were, onto their own slide offset as [`duplicate_shape`] does. Each
/// object's relationships go with it — media shared, a chart's or diagram's
/// parts copied. Returns the copies' ids, in order.
pub fn copy_shapes(
    bytes: &[u8],
    from: usize,
    ids: &[u32],
    to: usize,
) -> Result<(Vec<u8>, Vec<u32>)> {
    if from == to {
        let mut current = bytes.to_vec();
        let mut copies = Vec::with_capacity(ids.len());
        for &id in ids {
            let (next, copy) = duplicate_shape(&current, from, id)?;
            current = next;
            copies.push(copy);
        }
        return Ok((current, copies));
    }
    let (_, from_part, source) = slide_xml(bytes, from)?;
    let (mut package, to_part, into) = slide_xml(bytes, to)?;
    let (changed, copies) =
        xml::copy_objects(&mut package, &from_part, &source, ids, &to_part, &into)?;
    package.replace(&to_part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok((result, copies))
}

/// Moves and sizes several objects of one slide, as one edit: what moving a
/// selection of them does.
pub fn set_shape_transforms(
    bytes: &[u8],
    slide: usize,
    changes: &[(u32, jade_pptx_reader::Transform)],
) -> Result<Vec<u8>> {
    let mut current = bytes.to_vec();
    for &(id, transform) in changes {
        current = set_shape_transform(&current, slide, id, transform)?;
    }
    Ok(current)
}

/// Deletes several objects of one slide, as one edit.
pub fn delete_shapes(bytes: &[u8], slide: usize, ids: &[u32]) -> Result<Vec<u8>> {
    let mut current = bytes.to_vec();
    for &id in ids {
        current = delete_shape(&current, slide, id)?;
    }
    Ok(current)
}

pub fn delete_shape(bytes: &[u8], slide: usize, id: u32) -> Result<Vec<u8>> {
    shape_operation(bytes, slide, |xml| xml::delete_shape(xml, id))
}
pub fn arrange_shape(
    bytes: &[u8],
    slide: usize,
    id: u32,
    direction: ObjectArrangement,
) -> Result<Vec<u8>> {
    shape_operation(bytes, slide, |xml| xml::arrange_shape(xml, id, direction))
}
fn shape_operation(
    bytes: &[u8],
    slide: usize,
    edit: impl FnOnce(&str) -> Result<String>,
) -> Result<Vec<u8>> {
    let (mut package, part, xml) = slide_xml(bytes, slide)?;
    let changed = edit(&xml)?;
    if changed == xml {
        return Ok(bytes.to_vec());
    }
    package.replace(&part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}

pub fn editable_shapes(bytes: &[u8], slide: usize) -> Result<Vec<ShapeTransformInfo>> {
    let (_, _, xml) = slide_xml(bytes, slide)?;
    let mut infos = xml::shapes(&xml)?;
    if infos.iter().any(|i| i.transform.is_none()) {
        let deck = jade_pptx_reader::read_deck(bytes)?;
        resolve_shape_infos(&mut infos, &deck, slide);
    }
    Ok(infos)
}
fn shape_infos_in_deck(
    xml: &str,
    deck: &jade_pptx_reader::Deck,
    slide: usize,
) -> Result<Vec<ShapeTransformInfo>> {
    let mut infos = xml::shapes(xml)?;
    resolve_shape_infos(&mut infos, deck, slide);
    Ok(infos)
}
fn resolve_shape_infos(
    infos: &mut [ShapeTransformInfo],
    deck: &jade_pptx_reader::Deck,
    slide: usize,
) {
    for info in infos {
        info.transform = info
            .transform
            .or_else(|| inherited_shape_transform(deck, slide, info.id));
    }
}

fn grouping_bounds(bytes: &[u8], slide: usize, ids: &[u32]) -> Result<jade_pptx_reader::Transform> {
    let (_, _, xml) = slide_xml(bytes, slide)?;
    xml::grouping::validate_selection(&xml, ids)?;
    let infos = editable_shapes(bytes, slide)?;
    let transforms = ids
        .iter()
        .map(|id| {
            let t = infos
                .iter()
                .find(|info| info.id == *id)
                .and_then(|info| info.transform)
                .ok_or(WriteError::NoShape(*id))?;
            Ok(jade_pptx_layout::ObjectTransform {
                rect: jade_pptx_layout::Rect {
                    x: t.offset_emu.0 as f64 / 9525.0,
                    y: t.offset_emu.1 as f64 / 9525.0,
                    width: t.extent_emu.0 as f64 / 9525.0,
                    height: t.extent_emu.1 as f64 / 9525.0,
                },
                rotation_degrees: f64::from(t.rotation_60000ths) / 60000.0,
                flip_h: t.flip_h,
                flip_v: t.flip_v,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    jade_pptx_layout::grouping_transform(&transforms).ok_or(WriteError::InvalidTransform)
}
pub fn validate_group_objects(bytes: &[u8], slide: usize, ids: &[u32]) -> Result<()> {
    grouping_bounds(bytes, slide, ids).map(|_| ())
}
pub fn group_objects(bytes: &[u8], slide: usize, ids: &[u32]) -> Result<(Vec<u8>, u32)> {
    let bounds = grouping_bounds(bytes, slide, ids)?;
    let (mut package, part, xml) = slide_xml(bytes, slide)?;
    let (changed, id) = xml::grouping::group(&xml, ids, bounds)?;
    package.replace(&part, changed.into_bytes());
    let bytes = package.write()?;
    jade_pptx_reader::read_deck(&bytes)?;
    Ok((bytes, id))
}

fn inherited_shape_transform(
    deck: &jade_pptx_reader::Deck,
    index: usize,
    id: u32,
) -> Option<jade_pptx_reader::Transform> {
    let slide = deck.slides.get(index)?;
    let inherited = jade_pptx_layout::InheritedContent {
        layout: slide.layout_content.as_ref(),
        master: slide.master_content.as_ref(),
        ..Default::default()
    };
    slide.content.shapes.iter().find_map(|item| match item {
        jade_pptx_reader::ShapeItem::Shape(shape) if shape.id == Some(id) => {
            jade_pptx_layout::resolve_transform(
                shape.transform,
                shape.placeholder.as_ref(),
                &inherited,
            )
        }
        jade_pptx_reader::ShapeItem::Picture(picture) if picture.id == Some(id) => {
            jade_pptx_layout::resolve_transform(
                picture.transform,
                picture.placeholder.as_ref(),
                &inherited,
            )
        }
        _ => None,
    })
}

/// PowerPoint limits offsets and positive extents to signed 32-bit EMUs.
pub fn validate_transform(transform: jade_pptx_reader::Transform) -> Result<()> {
    let (x, y) = transform.offset_emu;
    let (width, height) = transform.extent_emu;
    if [x, y, width, height]
        .iter()
        .any(|&value| i32::try_from(value).is_err())
        || width < 0
        || height < 0
    {
        return Err(WriteError::InvalidTransform);
    }
    Ok(())
}

pub fn set_shape_transform(
    bytes: &[u8],
    slide: usize,
    id: u32,
    transform: jade_pptx_reader::Transform,
) -> Result<Vec<u8>> {
    validate_transform(transform)?;
    let (mut package, part, xml) = slide_xml(bytes, slide)?;
    let deck = jade_pptx_reader::read_deck(bytes)?;
    let baseline = inherited_shape_transform(&deck, slide, id);
    let before = xml::shapes(&xml)?.into_iter().find(|shape| shape.id == id);
    let changed = xml::set_transform(&xml, id, transform, baseline)?;
    if changed == xml {
        return Ok(bytes.to_vec());
    }
    package.replace(&part, changed.into_bytes());
    // A SmartArt diagram shows the drawing PowerPoint cached for it, in the
    // frame's own coordinates: resized, the drawing scales with the frame.
    let resized_diagram = before
        .filter(|shape| shape.kind == "diagram")
        .and_then(|shape| shape.transform)
        .filter(|old| old.extent_emu != transform.extent_emu);
    if let Some(old) = resized_diagram {
        if let Some(drawing) = diagram_drawing_part(&package, &part, &xml, id)? {
            let scaled = xml::diagrams::scale_drawing(
                &package.required_part_str(&drawing)?,
                old.extent_emu,
                transform.extent_emu,
            )?;
            package.replace(&drawing, scaled.into_bytes());
        }
    }
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}

/// The drawing PowerPoint caches for the SmartArt frame `id`: the one its
/// data part's `dataModelExt` names, or the slide's one diagram drawing.
fn diagram_drawing_part(
    package: &OpcPackage,
    slide_part: &str,
    slide_xml: &str,
    id: u32,
) -> Result<Option<String>> {
    let Some(rels) = package.part_str(&jade_opc::rels_part_name(slide_part))? else {
        return Ok(None);
    };
    let rels = jade_opc::parse_relationships(&rels)?;
    let data = xml::diagrams::data_relationship(slide_xml, id)?
        .and_then(|rel| rels.target(&rel))
        .map(|target| jade_opc::resolve_target(slide_part, target));
    let drawing_rel = match data {
        Some(data) => match package.part_str(&data)? {
            Some(data_xml) => jade_pptx_reader::diagram_drawing_rel_id(&data_xml)?,
            None => None,
        },
        None => None,
    };
    Ok(drawing_rel
        .as_deref()
        .and_then(|rel| rels.target(rel))
        .or_else(|| rels.target_by_type_suffix("/diagramDrawing"))
        .map(|target| jade_opc::resolve_target(slide_part, target)))
}

/// The cells of the table in frame `id` on `slide`, row by row.
pub fn table_cells(bytes: &[u8], slide: usize, id: u32) -> Result<Vec<Vec<TableCell>>> {
    let (_, _, xml) = slide_xml(bytes, slide)?;
    xml::tables::cells(&xml, id)
}

/// `bytes` with cell (`row`, `column`) of the table in frame `id` holding
/// `text`, a line each paragraph, keeping the cell's formatting.
pub fn set_table_cell_text(
    bytes: &[u8],
    slide: usize,
    id: u32,
    row: usize,
    column: usize,
    text: &str,
) -> Result<Vec<u8>> {
    let (mut package, part, xml) = slide_xml(bytes, slide)?;
    let changed = xml::tables::set_cell_text(&xml, id, row, column, text)?;
    if changed == xml {
        return Ok(bytes.to_vec());
    }
    package.replace(&part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}

/// A change to a table's layout: a row or column added at the index given
/// (the end when it is the count) or removed from it; cells merged into one,
/// or a merged cell split back into its cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableChange {
    InsertRow(usize),
    DeleteRow(usize),
    InsertColumn(usize),
    DeleteColumn(usize),
    /// The `rows` × `columns` cells from (`row`, `column`) become one, as
    /// PowerPoint's Merge Cells makes them: the first spans the rest, which
    /// give it their text as paragraphs.
    MergeCells {
        row: usize,
        column: usize,
        rows: usize,
        columns: usize,
    },
    /// The merged cell at (`row`, `column`) becomes its cells again, its text
    /// staying in the first.
    SplitCell {
        row: usize,
        column: usize,
    },
}

/// `bytes` with a row or column of the table in frame `id` added or removed,
/// as PowerPoint does: a new one is empty, formatted as the one it pushes
/// over, and the frame grows or shrinks by it. Cutting a merged cell, or
/// removing the last row or column, is refused.
pub fn change_table(bytes: &[u8], slide: usize, id: u32, change: TableChange) -> Result<Vec<u8>> {
    let (mut package, part, xml) = slide_xml(bytes, slide)?;
    let changed = xml::tables::change(&xml, id, change)?;
    package.replace(&part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}

pub fn validate_shape_transform(
    info: &ShapeTransformInfo,
    transform: jade_pptx_reader::Transform,
) -> Result<()> {
    validate_transform(transform)?;
    if info.kind == "group" && (transform.extent_emu.0 <= 0 || transform.extent_emu.1 <= 0) {
        return Err(WriteError::InvalidTransform);
    }
    let locked = if let Some(before) = info.transform {
        let width = i128::from(before.extent_emu.0);
        let height = i128::from(before.extent_emu.1);
        let aspect_changed = info.lock_aspect_ratio
            && (width * i128::from(transform.extent_emu.1)
                - height * i128::from(transform.extent_emu.0))
            .abs()
                > width.max(0) + height.max(0);
        (!info.can_move && before.offset_emu != transform.offset_emu)
            || aspect_changed
            || (!info.can_resize && before.extent_emu != transform.extent_emu)
            || (!info.can_rotate
                && (before.rotation_60000ths != transform.rotation_60000ths
                    || before.flip_h != transform.flip_h
                    || before.flip_v != transform.flip_v))
    } else {
        !info.can_move || !info.can_resize || !info.can_rotate
    };
    if !info.can_select || locked {
        return Err(WriteError::LockedShape(info.id));
    }
    Ok(())
}

fn slide_xml(bytes: &[u8], index: usize) -> Result<(PreservingOpcPackage<'_>, String, String)> {
    let presentation = jade_pptx_reader::read_pptx(bytes)?;
    let slide = presentation
        .slides
        .get(index)
        .ok_or(WriteError::NoSlide(index))?;
    let package = OpcPackage::open_preserving(bytes)?;
    let xml = package.required_part_str(&slide.part_name)?;
    Ok((package, slide.part_name.clone(), xml))
}

pub fn editable_runs(bytes: &[u8], slide: usize) -> Result<Vec<EditableRun>> {
    let (_, _, xml) = slide_xml(bytes, slide)?;
    Ok(xml::runs(&xml)?.into_iter().map(|run| run.value).collect())
}

/// Replaces just the selected `<a:t>` payload. Everything outside its byte
/// range, including unknown extensions and formatting, is left verbatim.
pub fn replace_run_text(
    bytes: &[u8],
    slide: usize,
    address: RunAddress,
    text: &str,
) -> Result<Vec<u8>> {
    for ch in text.chars() {
        if !matches!(ch, '\t' | ' '..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}')
        {
            return Err(WriteError::InvalidCharacter(ch as u32));
        }
    }
    let (mut package, part, mut xml) = slide_xml(bytes, slide)?;
    let anchors = xml::runs(&xml)?;
    if xml::text_snapshot(&xml, address.shape_id)?.locked {
        return Err(WriteError::TextLocked(address.shape_id));
    }
    let mut matches = anchors.iter().filter(|run| run.value.address == address);
    let anchor = matches.next().ok_or(WriteError::NoRun(address))?;
    if matches.next().is_some() {
        return Err(WriteError::NoRun(address));
    }
    if anchor.value.text == text {
        return Ok(bytes.to_vec());
    }
    let escaped = quick_xml::escape::escape(text);
    let replacement = if let Some(name) = &anchor.empty_name {
        format!(
            "{}{escaped}</{name}>",
            anchor.empty_open.as_deref().unwrap_or_default()
        )
    } else {
        escaped.into_owned()
    };
    xml.replace_range(anchor.range.clone(), &replacement);
    package.replace(&part, xml.into_bytes());
    let result = package.write()?;
    // A failed edit must never replace the current, openable document.
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}

/// Bounded exact archive history, sharing unchanged ZIP payloads across edits.
pub struct EditSession {
    current: Arc<Vec<u8>>,
    current_snapshot: ArchiveSnapshot,
    saved: ArchiveSnapshot,
    undo: Vec<ArchiveSnapshot>,
    redo: Vec<ArchiveSnapshot>,
    snapshots: SnapshotStore,
    typing: Option<text_input::TypingGroup>,
}

impl EditSession {
    pub fn set_slide_notes(&mut self, slide: usize, text: &str) -> Result<bool> {
        self.set_slide_notes_with_breaks(slide, text, false)
    }
    pub fn set_slide_notes_with_breaks(
        &mut self,
        slide: usize,
        text: &str,
        soft_break: bool,
    ) -> Result<bool> {
        let next = set_slide_notes_with_breaks(self.bytes(), slide, text, soft_break)?;
        Ok(self.commit(next))
    }
    pub fn duplicate_slide(&mut self, index: usize) -> Result<bool> {
        self.duplicate_slides(&[index])
    }
    /// Duplicate targets in presentation order after the last original target, as one action.
    pub fn duplicate_slides(&mut self, indices: &[usize]) -> Result<bool> {
        let next = duplicate_slides(self.bytes(), indices)?;
        Ok(self.commit(next))
    }
    pub fn delete_slide(&mut self, index: usize) -> Result<bool> {
        self.delete_slides(&[index])
    }
    /// Delete distinct original slide indexes atomically in one history action.
    pub fn delete_slides(&mut self, indices: &[usize]) -> Result<bool> {
        let next = delete_slides(self.bytes(), indices)?;
        Ok(self.commit(next))
    }
    pub fn move_slide(&mut self, from: usize, to: usize) -> Result<bool> {
        self.move_slides(&[from], to)
    }
    /// Move targets in original presentation order to a final contiguous start index.
    pub fn move_slides(&mut self, indices: &[usize], to: usize) -> Result<bool> {
        let next = move_slides(self.bytes(), indices, to)?;
        Ok(self.commit(next))
    }
    pub fn set_slide_hidden(&mut self, index: usize, hidden: bool) -> Result<bool> {
        self.set_slides_hidden(&[index], hidden)
    }
    /// Set visibility for distinct slide indexes in one history action.
    pub fn set_slides_hidden(&mut self, indices: &[usize], hidden: bool) -> Result<bool> {
        let next = set_slides_hidden(self.bytes(), indices, hidden)?;
        Ok(self.commit(next))
    }
    pub fn set_slide_layout(&mut self, index: usize, layout: &str) -> Result<bool> {
        let next = set_slide_layout(self.bytes(), index, layout)?;
        Ok(self.commit(next))
    }
    pub fn set_slide_layouts(&mut self, indices: &[usize], layout: &str) -> Result<bool> {
        let next = set_slide_layouts(self.bytes(), indices, layout)?;
        Ok(self.commit(next))
    }
    pub fn reset_slides(&mut self, indices: &[usize]) -> Result<bool> {
        let next = reset_slides(self.bytes(), indices)?;
        Ok(self.commit(next))
    }
    pub fn reset_slide(&mut self, index: usize) -> Result<bool> {
        let next = reset_slide(self.bytes(), index)?;
        Ok(self.commit(next))
    }
    pub fn insert_slide(&mut self, index: usize) -> Result<bool> {
        self.edit_slides(xml::SlideEdit::Insert(index))
    }
    fn edit_slides(&mut self, edit: xml::SlideEdit) -> Result<bool> {
        let next = xml::edit_slides(self.bytes(), edit)?;
        Ok(self.commit(next))
    }
    pub fn open(bytes: &[u8]) -> Result<Self> {
        jade_pptx_reader::read_deck(bytes)?;
        let current = Arc::new(bytes.to_vec());
        let mut snapshots = SnapshotStore::default();
        let current_snapshot = snapshots.capture(bytes);
        Ok(Self {
            saved: current_snapshot.clone(),
            current,
            current_snapshot,
            snapshots,
            undo: Vec::new(),
            redo: Vec::new(),
            typing: None,
        })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.current
    }
    pub fn is_dirty(&self) -> bool {
        self.current_snapshot != self.saved
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn history_depth(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
    pub fn mark_saved(&mut self) {
        self.end_text_history();
        self.saved = self.current_snapshot.clone();
    }

    pub fn set_run_text(&mut self, slide: usize, address: RunAddress, text: &str) -> Result<bool> {
        let next = replace_run_text(self.bytes(), slide, address, text)?;
        Ok(self.commit(next))
    }

    pub fn replace_text_range(&mut self, slide: usize, edit: &TextRangeEdit) -> Result<bool> {
        let next = replace_text_range(
            self.bytes(),
            slide,
            edit.shape_id,
            edit.start,
            edit.end,
            &edit.text,
            edit.soft_break,
        )?;
        Ok(self.commit(next))
    }

    pub fn set_shape_text(
        &mut self,
        slide: usize,
        id: u32,
        text: &str,
        soft_break: bool,
    ) -> Result<bool> {
        let before = shape_text(self.bytes(), slide, id)?;
        let edit = text_edit_between(id, &before.text, text, soft_break);
        self.replace_text_range(slide, &edit)
    }

    pub fn format_text_range(&mut self, slide: usize, edit: &TextFormatEdit) -> Result<bool> {
        let next = format_text_range(self.bytes(), slide, edit)?;
        Ok(self.commit(next))
    }

    pub fn format_paragraphs(&mut self, slide: usize, edit: &ParagraphFormatEdit) -> Result<bool> {
        let next = format_paragraphs(self.bytes(), slide, edit)?;
        Ok(self.commit(next))
    }

    /// Applies typing properties to the inserted splice in the same Undo entry.
    pub fn set_shape_text_formatted(
        &mut self,
        slide: usize,
        id: u32,
        text: &str,
        soft_break: bool,
        format: &TextFormat,
    ) -> Result<bool> {
        format.validate()?;
        let before = shape_text(self.bytes(), slide, id)?;
        let edit = text_edit_between(id, &before.text, text, soft_break);
        let next = replace_text_range(
            self.bytes(),
            slide,
            id,
            edit.start,
            edit.end,
            &edit.text,
            soft_break,
        )?;
        let inserted = edit.text.encode_utf16().count();
        let next = if !format.is_empty() && inserted > 0 {
            format_text_range(
                &next,
                slide,
                &TextFormatEdit {
                    shape_id: id,
                    start: edit.start,
                    end: edit.start + inserted,
                    format: format.clone(),
                },
            )?
        } else {
            next
        };
        Ok(self.commit(next))
    }

    pub fn duplicate_shape(&mut self, slide: usize, id: u32) -> Result<u32> {
        let (next, new_id) = duplicate_shape(self.bytes(), slide, id)?;
        self.commit(next);
        Ok(new_id)
    }
    /// Pastes objects copied from slide `from` onto slide `to`, as one step
    /// of Undo: see [`copy_shapes`].
    pub fn copy_shapes(&mut self, from: usize, ids: &[u32], to: usize) -> Result<Vec<u32>> {
        let (next, copies) = copy_shapes(self.bytes(), from, ids, to)?;
        self.commit(next);
        Ok(copies)
    }
    pub fn group_objects(&mut self, slide: usize, ids: &[u32]) -> Result<u32> {
        let (next, id) = group_objects(self.bytes(), slide, ids)?;
        self.commit(next);
        Ok(id)
    }
    pub fn set_shape_style(
        &mut self,
        slide: usize,
        id: u32,
        value: &ShapeStyleEdit,
    ) -> Result<bool> {
        let next = set_shape_style(self.bytes(), slide, id, value)?;
        Ok(self.commit(next))
    }
    pub fn insert_text_box(
        &mut self,
        slide: usize,
        transform: jade_pptx_reader::Transform,
        text: &str,
        format: &TextFormat,
    ) -> Result<u32> {
        let (next, id) = insert_text_box(self.bytes(), slide, transform, text, format)?;
        self.commit(next);
        Ok(id)
    }
    pub fn insert_shape(
        &mut self,
        slide: usize,
        transform: jade_pptx_reader::Transform,
        value: &ShapeInsert,
    ) -> Result<u32> {
        let (next, id) = insert_shape(self.bytes(), slide, transform, value)?;
        self.commit(next);
        Ok(id)
    }
    pub fn insert_picture(
        &mut self,
        slide: usize,
        transform: jade_pptx_reader::Transform,
        image: &[u8],
    ) -> Result<PictureInsertResult> {
        let (next, inserted) = insert_picture(self.bytes(), slide, transform, image)?;
        self.commit(next);
        Ok(inserted)
    }
    pub fn set_picture_crop(
        &mut self,
        slide: usize,
        id: u32,
        crop: jade_pptx_reader::SourceRect,
        keep_frame: bool,
    ) -> Result<bool> {
        let next = set_picture_crop(self.bytes(), slide, id, crop, keep_frame)?;
        Ok(self.commit(next))
    }
    pub fn replace_picture(
        &mut self,
        slide: usize,
        id: u32,
        image: &[u8],
        expected_fingerprint: &str,
    ) -> Result<PictureReplaceResult> {
        let (next, mut replaced) =
            replace_picture(self.bytes(), slide, id, image, expected_fingerprint)?;
        replaced.changed = self.commit(next);
        Ok(replaced)
    }
    pub fn delete_shape(&mut self, slide: usize, id: u32) -> Result<bool> {
        let next = delete_shape(self.bytes(), slide, id)?;
        Ok(self.commit(next))
    }
    pub fn arrange_shape(
        &mut self,
        slide: usize,
        id: u32,
        direction: ObjectArrangement,
    ) -> Result<bool> {
        let next = arrange_shape(self.bytes(), slide, id, direction)?;
        Ok(self.commit(next))
    }
    pub fn set_shape_transform(
        &mut self,
        slide: usize,
        id: u32,
        transform: jade_pptx_reader::Transform,
    ) -> Result<bool> {
        let next = set_shape_transform(self.bytes(), slide, id, transform)?;
        Ok(self.commit(next))
    }

    /// Moves and sizes several objects of one slide, as one step of Undo.
    pub fn set_shape_transforms(
        &mut self,
        slide: usize,
        changes: &[(u32, jade_pptx_reader::Transform)],
    ) -> Result<bool> {
        let next = set_shape_transforms(self.bytes(), slide, changes)?;
        Ok(self.commit(next))
    }

    /// Deletes several objects of one slide, as one step of Undo.
    pub fn delete_shapes(&mut self, slide: usize, ids: &[u32]) -> Result<bool> {
        let next = delete_shapes(self.bytes(), slide, ids)?;
        Ok(self.commit(next))
    }

    /// Types `text` into a table cell, as one undoable edit.
    pub fn set_table_cell_text(
        &mut self,
        slide: usize,
        id: u32,
        row: usize,
        column: usize,
        text: &str,
    ) -> Result<bool> {
        let next = set_table_cell_text(self.bytes(), slide, id, row, column, text)?;
        Ok(self.commit(next))
    }

    pub fn change_table(&mut self, slide: usize, id: u32, change: TableChange) -> Result<bool> {
        let next = change_table(self.bytes(), slide, id, change)?;
        Ok(self.commit(next))
    }

    fn commit(&mut self, next: Vec<u8>) -> bool {
        self.end_text_history();
        if next == *self.current {
            return false;
        }
        self.undo.push(self.current_snapshot.clone());
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.replace_current(next);
        true
    }

    fn replace_current(&mut self, next: Vec<u8>) {
        self.current_snapshot = self.snapshots.capture(&next);
        self.current = Arc::new(next);
    }

    /// Archive byte buffers retained by current/saved/Undo/Redo snapshots.
    /// Excludes parsed documents, bitmaps, snapshot descriptors and hash metadata.
    pub fn retained_archive_bytes(&self) -> usize {
        self.current.len() + self.snapshots.retained_bytes()
    }

    pub fn undo(&mut self) -> bool {
        self.end_text_history();
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.current = Arc::new(previous.restore());
        self.redo
            .push(std::mem::replace(&mut self.current_snapshot, previous));
        true
    }

    pub fn redo(&mut self) -> bool {
        self.end_text_history();
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.current = Arc::new(next.restore());
        self.undo
            .push(std::mem::replace(&mut self.current_snapshot, next));
        true
    }
}

/// Delete distinct original slide indexes and their unreferenced owned parts.
pub fn delete_slides(bytes: &[u8], indices: &[usize]) -> Result<Vec<u8>> {
    xml::edit_slides(bytes, xml::SlideEdit::Delete(indices.to_vec()))
}
/// Check batch deletion without changing document or history.
pub fn validate_slide_deletions(bytes: &[u8], indices: &[usize]) -> Result<()> {
    delete_slides(bytes, indices).map(|_| ())
}

/// Set visibility atomically without changing content, identities or references.
pub fn set_slides_hidden(bytes: &[u8], indices: &[usize], hidden: bool) -> Result<Vec<u8>> {
    xml::edit_slides(
        bytes,
        xml::SlideEdit::Hidden {
            indices: indices.to_vec(),
            hidden,
        },
    )
}
/// Validate all targets before changing document, selection or previews.
pub fn validate_slides_hidden(bytes: &[u8], indices: &[usize], hidden: bool) -> Result<()> {
    set_slides_hidden(bytes, indices, hidden).map(|_| ())
}

/// Move distinct original indexes as one block; `to` is its final start index.
pub fn move_slides(bytes: &[u8], indices: &[usize], to: usize) -> Result<Vec<u8>> {
    xml::edit_slides(
        bytes,
        xml::SlideEdit::Move {
            indices: indices.to_vec(),
            to,
        },
    )
}
pub fn validate_slide_moves(bytes: &[u8], indices: &[usize], to: usize) -> Result<()> {
    move_slides(bytes, indices, to).map(|_| ())
}

/// Duplicate distinct indexes after the last target; copies have independent owned graphs.
pub fn duplicate_slides(bytes: &[u8], indices: &[usize]) -> Result<Vec<u8>> {
    xml::edit_slides(bytes, xml::SlideEdit::Duplicate(indices.to_vec()))
}
pub fn validate_slide_duplications(bytes: &[u8], indices: &[usize]) -> Result<()> {
    duplicate_slides(bytes, indices).map(|_| ())
}
