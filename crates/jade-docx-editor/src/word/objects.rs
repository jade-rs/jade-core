//! The drawing-object tools both shells share — 図の形式 and 図形の書式's
//! 配置 and サイズ groups and the レイアウトの詳細設定 dialog — over any
//! [`WordSession`]: where an object is and what it is relative to, how text
//! keeps away from it, its anchor's options, its rotation and flips, its
//! stacking among the others, its name and visibility (the Selection Pane),
//! aligning, distributing and grouping several.
//!
//! An object is changed as Word stores it: the commands edit its
//! `w:drawing` (jade-core's `edit_drawing_command`) and the object is read
//! again from it, so what is drawn is what the file says.

use crate::commands::edit_drawing::edit_drawing_command;
use crate::commands::format_image::{FormatImageCommand, ImageFormatSpec};
use crate::commands::format_text_box::{FormatTextBoxCommand, TextBoxFormatSpec};
use crate::commands::transform_object::TransformObjectCommand;
use crate::commands::Command;
use crate::document::{ImagePosition, Run, RunImageKind, RunImageLayout, TextBoxShape, WrapMode};
use crate::docx_xml::XmlElement;
use crate::drawing_xml::{self, child, child_mut, container_mut, get_attr, set_attr};
use crate::word_document::objects::parse_element;
use crate::word_document::{
    FloatingObjectKind, FloatingObjectTarget, FloatingObjectTransform, WordDocument,
};

use super::page::WordPage;
use super::picture::{PlacedObject, Wrap};
use super::WordSession;

/// CSS px per point.
pub const PX_PER_PT: f64 = 96.0 / 72.0;
/// CSS px per millimetre.
pub const PX_PER_MM: f64 = 96.0 / 25.4;

/// What an object is, for which tools apply to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectSort {
    Picture,
    /// A shape: a preset or freeform, a line or connector.
    Shape,
    /// A shape holding text (`wps:txbx`).
    TextBox,
    /// A text box whose text is warped (`a:prstTxWarp`) — WordArt.
    WordArt,
    Group,
    Chart,
    SmartArt,
}

/// What a position is measured from (`wp:positionH/V@relativeFrom`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelativeTo {
    Margin,
    Page,
    /// Horizontal only: the column the anchor is in.
    Column,
    /// Horizontal only: the character the anchor is at.
    Character,
    LeftMargin,
    RightMargin,
    InsideMargin,
    OutsideMargin,
    /// Vertical only: the anchor's paragraph.
    Paragraph,
    /// Vertical only: the anchor's line.
    Line,
    TopMargin,
    BottomMargin,
}

impl RelativeTo {
    pub const HORIZONTAL: [RelativeTo; 8] = [
        RelativeTo::Margin,
        RelativeTo::Page,
        RelativeTo::Column,
        RelativeTo::Character,
        RelativeTo::LeftMargin,
        RelativeTo::RightMargin,
        RelativeTo::InsideMargin,
        RelativeTo::OutsideMargin,
    ];
    pub const VERTICAL: [RelativeTo; 8] = [
        RelativeTo::Margin,
        RelativeTo::Page,
        RelativeTo::Paragraph,
        RelativeTo::Line,
        RelativeTo::TopMargin,
        RelativeTo::BottomMargin,
        RelativeTo::InsideMargin,
        RelativeTo::OutsideMargin,
    ];

    pub fn ooxml(self) -> &'static str {
        match self {
            RelativeTo::Margin => "margin",
            RelativeTo::Page => "page",
            RelativeTo::Column => "column",
            RelativeTo::Character => "character",
            RelativeTo::LeftMargin => "leftMargin",
            RelativeTo::RightMargin => "rightMargin",
            RelativeTo::InsideMargin => "insideMargin",
            RelativeTo::OutsideMargin => "outsideMargin",
            RelativeTo::Paragraph => "paragraph",
            RelativeTo::Line => "line",
            RelativeTo::TopMargin => "topMargin",
            RelativeTo::BottomMargin => "bottomMargin",
        }
    }

    pub fn from_ooxml(value: &str) -> Option<Self> {
        Some(match value {
            "margin" => RelativeTo::Margin,
            "page" => RelativeTo::Page,
            "column" => RelativeTo::Column,
            "character" => RelativeTo::Character,
            "leftMargin" => RelativeTo::LeftMargin,
            "rightMargin" => RelativeTo::RightMargin,
            "insideMargin" => RelativeTo::InsideMargin,
            "outsideMargin" => RelativeTo::OutsideMargin,
            "paragraph" => RelativeTo::Paragraph,
            "line" => RelativeTo::Line,
            "topMargin" => RelativeTo::TopMargin,
            "bottomMargin" => RelativeTo::BottomMargin,
            _ => return None,
        })
    }
}

/// How an object is placed along one axis: by alignment (`wp:align`) or at
/// an offset (`wp:posOffset`) from what it is relative to.
#[derive(Clone, Debug, PartialEq)]
pub enum Placement {
    /// `left`, `center`, `right`, `inside`, `outside` — or `top`, `center`,
    /// `bottom`, `inside`, `outside` vertically.
    Align(String),
    /// CSS px.
    Offset(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Position {
    pub relative_to: RelativeTo,
    pub placement: Placement,
}

/// What the tools show for an object, read from the document.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectFormat {
    pub target: FloatingObjectTarget,
    pub sort: ObjectSort,
    /// `wp:docPr@name`.
    pub name: String,
    pub id: Option<u32>,
    /// CSS px.
    pub width: f64,
    pub height: f64,
    /// Clockwise, in degrees.
    pub rotation: f64,
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
    pub wrap: Wrap,
    pub horizontal: Option<Position>,
    pub vertical: Option<Position>,
    /// Distance from text: top, bottom, left, right, in CSS px.
    pub distances: [f64; 4],
    /// 文字列と一緒に移動する: the vertical position follows the paragraph.
    pub moves_with_text: bool,
    /// アンカーを段落に固定する.
    pub locked: bool,
    /// 重なりを許可する.
    pub allow_overlap: bool,
    /// 表のセル内に配置する.
    pub layout_in_cell: bool,
    pub hidden: bool,
    /// Stacking among floating objects; higher is in front.
    pub z_order: u32,
    pub behind_text: bool,
}

fn layout_of(run: &Run) -> Option<&RunImageLayout> {
    run.image
        .as_ref()
        .map(|image| &image.layout)
        .or_else(|| run.text_box.as_ref().map(|text_box| &text_box.layout))
}

fn wrap_of(layout: &RunImageLayout) -> Wrap {
    match (layout.kind, layout.wrap) {
        (RunImageKind::Inline, _) => Wrap::Inline,
        (_, WrapMode::Square) => Wrap::Square,
        (_, WrapMode::Tight) => Wrap::Tight,
        (_, WrapMode::Through) => Wrap::Through,
        (_, WrapMode::TopAndBottom) => Wrap::TopAndBottom,
        (_, WrapMode::None) if layout.behind_doc => Wrap::BehindText,
        (_, WrapMode::None) => Wrap::InFrontOfText,
    }
}

fn position_of(position: Option<&ImagePosition>, default: RelativeTo) -> Option<Position> {
    let position = position?;
    let relative_to = position
        .relative_from
        .as_deref()
        .and_then(RelativeTo::from_ooxml)
        .unwrap_or(default);
    let placement = match position.align.as_deref() {
        Some(align) => Placement::Align(align.to_string()),
        None => Placement::Offset(position.offset_px.unwrap_or(0.0)),
    };
    Some(Position {
        relative_to,
        placement,
    })
}

/// The object at `target`'s run.
pub fn object_run(document: &WordDocument, target: FloatingObjectTarget) -> Option<&Run> {
    let run = document
        .paragraph(target.paragraph_index)?
        .runs
        .get(target.run_index)?;
    match target.kind {
        FloatingObjectKind::Image if run.image.is_some() => Some(run),
        FloatingObjectKind::TextBox if run.text_box.is_some() => Some(run),
        _ => None,
    }
}

/// What an object at `target` is and how it is laid out.
pub fn object_format(
    document: &WordDocument,
    target: FloatingObjectTarget,
) -> Option<ObjectFormat> {
    let run = object_run(document, target)?;
    let layout = layout_of(run)?;
    let (sort, width, height, default_name) = match (&run.image, &run.text_box) {
        (Some(image), _) => {
            let sort = if image.chart.is_some() {
                ObjectSort::Chart
            } else if image.diagram_data_relationship_id.is_some() {
                ObjectSort::SmartArt
            } else if image.shape_group.is_some() {
                ObjectSort::Group
            } else {
                ObjectSort::Picture
            };
            let name = match sort {
                ObjectSort::Chart => "グラフ",
                ObjectSort::SmartArt => "図表",
                ObjectSort::Group => "グループ化",
                _ => "図",
            };
            (
                sort,
                image.width_px.unwrap_or(0.0),
                image.height_px.unwrap_or(0.0),
                name,
            )
        }
        (None, Some(text_box)) => {
            let has_text = !text_box.blocks.is_empty() || !text_box.text.is_empty();
            let sort = if text_box.extras().text_warp.is_some() {
                ObjectSort::WordArt
            } else if has_text && !text_box.shape.is_line() {
                ObjectSort::TextBox
            } else {
                ObjectSort::Shape
            };
            (
                sort,
                text_box.width_px.unwrap_or(0.0),
                text_box.height_px.unwrap_or(0.0),
                if sort == ObjectSort::TextBox {
                    "テキスト ボックス"
                } else {
                    "図形"
                },
            )
        }
        (None, None) => return None,
    };
    let vertical = position_of(layout.position_v.as_ref(), RelativeTo::Paragraph);
    let moves_with_text = layout.kind == RunImageKind::Inline
        || matches!(
            vertical.as_ref().map(|position| position.relative_to),
            Some(RelativeTo::Paragraph | RelativeTo::Line)
        );
    let options = layout.options();
    let (id, name) = match (&options.name, options.object_id) {
        (Some(name), id) => (id, name.clone()),
        (None, id) => match package_name(document, run) {
            Some((package_id, name)) => (id.or(package_id), name),
            None => (id, format!("{default_name} {}", id.unwrap_or(1))),
        },
    };
    Some(ObjectFormat {
        target,
        sort,
        name,
        id,
        width,
        height,
        rotation: layout.rotation_deg.unwrap_or(0.0),
        flip_horizontal: layout.flip_horizontal,
        flip_vertical: layout.flip_vertical,
        wrap: wrap_of(layout),
        horizontal: position_of(layout.position_h.as_ref(), RelativeTo::Column),
        vertical,
        distances: [
            layout.dist_top.unwrap_or(0.0),
            layout.dist_bottom.unwrap_or(0.0),
            layout.dist_left.unwrap_or(0.0),
            layout.dist_right.unwrap_or(0.0),
        ],
        moves_with_text,
        locked: options.locked,
        allow_overlap: options.allow_overlap.unwrap_or(true),
        layout_in_cell: options.layout_in_cell.unwrap_or(true),
        hidden: options.hidden,
        z_order: layout.relative_height.unwrap_or(0),
        behind_text: layout.behind_doc,
    })
}

/// Every drawing object in the body, in document order — what the
/// Selection Pane lists (it shows them front first).
pub fn body_objects(document: &WordDocument) -> Vec<ObjectFormat> {
    let mut objects = Vec::new();
    for paragraph_index in 0..document.paragraph_count() {
        let Some(paragraph) = document.paragraph(paragraph_index) else {
            continue;
        };
        for (run_index, run) in paragraph.runs.iter().enumerate() {
            let kind = if run.image.is_some() {
                FloatingObjectKind::Image
            } else if run.text_box.is_some() {
                FloatingObjectKind::TextBox
            } else {
                continue;
            };
            let target = FloatingObjectTarget {
                paragraph_index,
                run_index,
                kind,
            };
            if document
                .text_box_host_for_surface(paragraph_index)
                .is_some()
            {
                // A shape inside a text box is the box's, not the page's.
                continue;
            }
            objects.extend(object_format(document, target));
        }
    }
    objects
}

/// 前面へ移動 / 背面へ移動 and their 最前面 / 最背面 forms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrange {
    Forward,
    Backward,
    ToFront,
    ToBack,
    /// 文字列の前面へ移動.
    InFrontOfText,
    /// テキストの背面へ移動.
    BehindText,
}

/// 配置: align or distribute objects, relative to the page, the margins or
/// the objects themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
    Top,
    Middle,
    Bottom,
    DistributeHorizontally,
    DistributeVertically,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignTo {
    Page,
    Margin,
    /// 選択したオブジェクトを揃える.
    Selection,
}

/// 回転.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rotate {
    Right90,
    Left90,
    FlipVertical,
    FlipHorizontal,
}

/// What a drawing edit does: takes the document (for media) and the
/// object's `w:drawing`.
pub type DrawingEdit =
    Box<dyn FnMut(&mut WordDocument, &mut XmlElement) -> Result<(), String> + 'static>;

/// Parses a DrawingML fragment written with the usual prefixes.
pub fn fragment(xml: &str) -> Result<XmlElement, String> {
    parse_element(xml)
}

/// `wp:anchor` or `wp:inline` of a drawing.
pub fn frame_mut(drawing: &mut XmlElement) -> Result<&mut XmlElement, String> {
    container_mut(drawing).ok_or_else(|| "the drawing has no frame".to_string())
}

fn emu(px: f64) -> i64 {
    drawing_xml::px_to_emu(px)
}

/// The `wp:positionH` / `wp:positionV` element for `position`.
fn position_element(name: &str, position: &Position) -> Result<XmlElement, String> {
    let inner = match &position.placement {
        Placement::Align(align) => format!("<wp:align>{align}</wp:align>"),
        Placement::Offset(offset) => format!("<wp:posOffset>{}</wp:posOffset>", emu(*offset)),
    };
    fragment(&format!(
        r#"<{name} relativeFrom="{}">{inner}</{name}>"#,
        position.relative_to.ooxml()
    ))
}

/// The floating objects back to front, with their `relativeHeight`: ties
/// in document order.
fn stacking(document: &WordDocument) -> Vec<(FloatingObjectTarget, u32)> {
    let mut stack: Vec<(FloatingObjectTarget, u32)> = body_objects(document)
        .into_iter()
        .filter(|object| object.wrap != Wrap::Inline)
        .map(|object| (object.target, object.z_order))
        .collect();
    // A stable sort keeps document order among equals.
    stack.sort_by_key(|(_, height)| *height);
    stack
}

/// Where `target` is among the floating objects, back (0) to front.
pub fn stacking_index(document: &WordDocument, target: FloatingObjectTarget) -> Option<usize> {
    stacking(document)
        .iter()
        .position(|(candidate, _)| *candidate == target)
}

/// The id and name a drawing's `wp:docPr` has in the package, for an object
/// the model has not read them for (one put in this session).
fn package_name(document: &WordDocument, run: &Run) -> Option<(Option<u32>, String)> {
    let raw = document.raw_document.as_ref()?;
    let element = raw.element(run.node_id?)?;
    let doc_pr = drawing_xml::descendant(element, "wp:docPr")?;
    Some((
        get_attr(doc_pr, "id").and_then(|id| id.parse().ok()),
        get_attr(doc_pr, "name")?.to_string(),
    ))
}

/// The drawing objects' tools, for every session.
pub trait WordObjects: WordSession + WordPage {
    /// Runs `edit` on the object's drawing as one undo step.
    fn edit_object(
        &mut self,
        target: FloatingObjectTarget,
        name: &'static str,
        edit: impl FnMut(&mut WordDocument, &mut XmlElement) -> Result<(), String> + 'static,
    ) -> Result<(), String> {
        self.execute_command(Box::new(edit_drawing_command(name, target, edit)), true)
    }

    fn object_format(&self, target: FloatingObjectTarget) -> Option<ObjectFormat> {
        object_format(self.word_document(), target)
    }

    fn body_objects(&self) -> Vec<ObjectFormat> {
        body_objects(self.word_document())
    }

    // ---- 位置 ---------------------------------------------------------------

    /// レイアウトの詳細設定 → 位置: where the object sits along each axis.
    /// An inline object is made to float first (in front of the text, as
    /// Word does when a position is given to one).
    fn set_object_position(
        &mut self,
        target: FloatingObjectTarget,
        horizontal: Position,
        vertical: Position,
    ) -> Result<(), String> {
        let inline = self
            .object_format(target)
            .map(|format| format.wrap == Wrap::Inline)
            .unwrap_or(false);
        if inline {
            self.set_object_wrap(target, Wrap::Square)?;
        }
        self.edit_object(target, "object_position", move |_, drawing| {
            let frame = frame_mut(drawing)?;
            if frame.name != "wp:anchor" {
                return Err("an object in line with the text has no position".into());
            }
            let h = position_element("wp:positionH", &horizontal)?;
            let v = position_element("wp:positionV", &vertical)?;
            drawing_xml::put_child(frame, h, &["wp:positionH"], &["wp:positionV", "wp:extent"]);
            drawing_xml::put_child(frame, v, &["wp:positionV"], &["wp:extent"]);
            set_attr(frame, "simplePos", Some("0".into()));
            Ok(())
        })
    }

    /// 位置 gallery: one of Word's nine page positions with square wrapping
    /// (文字列の折り返し: 四角), relative to the margins.
    fn place_object_on_page(
        &mut self,
        target: FloatingObjectTarget,
        horizontal: &str,
        vertical: &str,
    ) -> Result<(), String> {
        let wrap = self.object_format(target).map(|format| format.wrap);
        if matches!(wrap, Some(Wrap::Inline) | None) {
            self.set_object_wrap(target, Wrap::Square)?;
        }
        self.set_object_position(
            target,
            Position {
                relative_to: RelativeTo::Margin,
                placement: Placement::Align(horizontal.to_string()),
            },
            Position {
                relative_to: RelativeTo::Margin,
                placement: Placement::Align(vertical.to_string()),
            },
        )
    }

    /// 文字列の折り返し, for a picture or a shape.
    fn set_object_wrap(&mut self, target: FloatingObjectTarget, wrap: Wrap) -> Result<(), String> {
        let command: Box<dyn Command> = match target.kind {
            FloatingObjectKind::Image => Box::new(FormatImageCommand::new(
                target,
                ImageFormatSpec {
                    layout: Some(wrap),
                    ..Default::default()
                },
            )),
            FloatingObjectKind::TextBox => Box::new(FormatTextBoxCommand::new(
                target,
                TextBoxFormatSpec {
                    layout: Some(wrap),
                    ..Default::default()
                },
            )),
        };
        self.execute_command(command, true)
    }

    /// 文字列との間隔: top, bottom, left, right in CSS px.
    fn set_object_distances(
        &mut self,
        target: FloatingObjectTarget,
        [top, bottom, left, right]: [f64; 4],
    ) -> Result<(), String> {
        self.edit_object(target, "object_distances", move |_, drawing| {
            let frame = frame_mut(drawing)?;
            for (name, value) in [
                ("distT", top),
                ("distB", bottom),
                ("distL", left),
                ("distR", right),
            ] {
                set_attr(frame, name, Some(emu(value.max(0.0)).to_string()));
            }
            // Word repeats them on the wrap element.
            for wrap in [
                "wp:wrapSquare",
                "wp:wrapTight",
                "wp:wrapThrough",
                "wp:wrapTopAndBottom",
            ] {
                if let Some(element) = child_mut(frame, wrap) {
                    let sides: &[(&str, f64)] = match wrap {
                        "wp:wrapSquare" => &[
                            ("distT", top),
                            ("distB", bottom),
                            ("distL", left),
                            ("distR", right),
                        ],
                        "wp:wrapTopAndBottom" => &[("distT", top), ("distB", bottom)],
                        _ => &[("distL", left), ("distR", right)],
                    };
                    for (name, value) in sides {
                        set_attr(element, name, Some(emu(value.max(0.0)).to_string()));
                    }
                }
            }
            Ok(())
        })
    }

    /// レイアウトの詳細設定 → 位置 → オプション. `moves_with_text` makes the
    /// vertical position relative to the paragraph (文字列と一緒に移動する),
    /// or to the page (ページ上で位置を固定), keeping where the object is.
    fn set_object_options(
        &mut self,
        target: FloatingObjectTarget,
        options: ObjectOptions,
    ) -> Result<(), String> {
        let page_y = options.page_y;
        let paragraph_y = options.paragraph_y;
        self.edit_object(target, "object_options", move |_, drawing| {
            let frame = frame_mut(drawing)?;
            if frame.name != "wp:anchor" {
                return Err("an object in line with the text has no anchor options".into());
            }
            if let Some(locked) = options.locked {
                set_attr(frame, "locked", Some(u8::from(locked).to_string()));
            }
            if let Some(allow) = options.allow_overlap {
                set_attr(frame, "allowOverlap", Some(u8::from(allow).to_string()));
            }
            if let Some(in_cell) = options.layout_in_cell {
                set_attr(frame, "layoutInCell", Some(u8::from(in_cell).to_string()));
            }
            if let Some(moves) = options.moves_with_text {
                let current = child(frame, "wp:positionV")
                    .and_then(|position| get_attr(position, "relativeFrom"))
                    .unwrap_or("paragraph")
                    .to_string();
                let follows = matches!(current.as_str(), "paragraph" | "line");
                if moves != follows {
                    let (relative_to, offset) = if moves {
                        (RelativeTo::Paragraph, paragraph_y)
                    } else {
                        (RelativeTo::Page, page_y)
                    };
                    let element = position_element(
                        "wp:positionV",
                        &Position {
                            relative_to,
                            placement: Placement::Offset(offset.unwrap_or(0.0)),
                        },
                    )?;
                    drawing_xml::put_child(frame, element, &["wp:positionV"], &["wp:extent"]);
                }
            }
            Ok(())
        })
    }

    // ---- サイズ・回転 ------------------------------------------------------------

    /// サイズ: the object's width and height in CSS px.
    fn resize_object(
        &mut self,
        target: FloatingObjectTarget,
        width: f64,
        height: f64,
    ) -> Result<(), String> {
        let (width, height) = (width.clamp(1.0, 100_000.0), height.clamp(1.0, 100_000.0));
        let command: Box<dyn Command> = match target.kind {
            FloatingObjectKind::Image => Box::new(FormatImageCommand::new(
                target,
                ImageFormatSpec {
                    width_px: Some(width),
                    height_px: Some(height),
                    ..Default::default()
                },
            )),
            FloatingObjectKind::TextBox => Box::new(FormatTextBoxCommand::new(
                target,
                TextBoxFormatSpec {
                    width_px: Some(width),
                    height_px: Some(height),
                    ..Default::default()
                },
            )),
        };
        self.execute_command(command, true)
    }

    /// 回転: the angle, clockwise in degrees, normalised to 0–360.
    fn set_object_rotation(
        &mut self,
        target: FloatingObjectTarget,
        degrees: f64,
    ) -> Result<(), String> {
        let degrees = degrees.rem_euclid(360.0);
        self.edit_object(target, "object_rotation", move |_, drawing| {
            let sp_pr =
                drawing_xml::shape_properties_mut(drawing).ok_or("the object cannot be rotated")?;
            let xfrm = child_mut(sp_pr, "a:xfrm").ok_or("the object has no transform")?;
            let value = (degrees * 60_000.0).round() as i64;
            set_attr(xfrm, "rot", (value != 0).then(|| value.to_string()));
            Ok(())
        })
    }

    /// 回転 menu: 右へ 90 度回転, 左へ 90 度回転, 上下反転, 左右反転.
    fn rotate_object(
        &mut self,
        target: FloatingObjectTarget,
        rotate: Rotate,
    ) -> Result<(), String> {
        let format = self.object_format(target).ok_or("the object is gone")?;
        match rotate {
            Rotate::Right90 => self.set_object_rotation(target, format.rotation + 90.0),
            Rotate::Left90 => self.set_object_rotation(target, format.rotation - 90.0),
            Rotate::FlipVertical | Rotate::FlipHorizontal => {
                let horizontal = rotate == Rotate::FlipHorizontal;
                self.edit_object(target, "object_flip", move |_, drawing| {
                    let sp_pr = drawing_xml::shape_properties_mut(drawing)
                        .ok_or("the object cannot be flipped")?;
                    let xfrm = child_mut(sp_pr, "a:xfrm").ok_or("the object has no transform")?;
                    let name = if horizontal { "flipH" } else { "flipV" };
                    let on = matches!(get_attr(xfrm, name), Some("1" | "true"));
                    set_attr(xfrm, name, (!on).then(|| "1".to_string()));
                    Ok(())
                })
            }
        }
    }

    // ---- 前面へ・背面へ --------------------------------------------------------

    fn arrange_object(
        &mut self,
        target: FloatingObjectTarget,
        arrange: Arrange,
    ) -> Result<(), String> {
        if matches!(arrange, Arrange::InFrontOfText | Arrange::BehindText) {
            let wrap = if arrange == Arrange::BehindText {
                Wrap::BehindText
            } else {
                Wrap::InFrontOfText
            };
            return self.set_object_wrap(target, wrap);
        }
        // The floating objects back to front: by `relativeHeight`, ties in
        // document order (the later one in front, as Word draws them).
        let mut stack = stacking(self.word_document());
        let position = stack
            .iter()
            .position(|(candidate, _)| *candidate == target)
            .ok_or("an object in line with the text has no stacking order")?;
        let own = stack.remove(position);
        let at = match arrange {
            Arrange::ToFront => stack.len(),
            Arrange::ToBack => 0,
            Arrange::Forward => (position + 1).min(stack.len()),
            Arrange::Backward => position.saturating_sub(1),
            Arrange::InFrontOfText | Arrange::BehindText => unreachable!(),
        };
        if at == position {
            return Ok(());
        }
        stack.insert(at, own);
        // Every object gets a height in the new order, spaced as Word
        // spaces them, where it does not have one in order already.
        const BASE: u32 = 251_659_264;
        const STEP: u32 = 1_024;
        let commands: Vec<Box<dyn Command>> = stack
            .iter()
            .enumerate()
            .filter_map(|(index, (target, height))| {
                let wanted = BASE + STEP * index as u32;
                (*height != wanted).then(|| {
                    Box::new(edit_drawing_command(
                        "object_arrange",
                        *target,
                        move |_, drawing| {
                            let frame = frame_mut(drawing)?;
                            set_attr(frame, "relativeHeight", Some(wanted.to_string()));
                            Ok(())
                        },
                    )) as Box<dyn Command>
                })
            })
            .collect();
        self.execute_command(Box::new(Sequence::new("object_arrange", commands)), true)
    }

    // ---- オブジェクトの選択と表示 ------------------------------------------------

    /// The Selection Pane's rename: `wp:docPr@name`.
    fn rename_object(&mut self, target: FloatingObjectTarget, name: &str) -> Result<(), String> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err("an object needs a name".into());
        }
        self.edit_object(target, "object_name", move |_, drawing| {
            let frame = frame_mut(drawing)?;
            let doc_pr = child_mut(frame, "wp:docPr").ok_or("the object has no properties")?;
            set_attr(doc_pr, "name", Some(name.clone()));
            Ok(())
        })
    }

    /// The Selection Pane's eye: `wp:docPr@hidden`.
    fn set_object_hidden(
        &mut self,
        target: FloatingObjectTarget,
        hidden: bool,
    ) -> Result<(), String> {
        self.edit_object(target, "object_visibility", move |_, drawing| {
            let frame = frame_mut(drawing)?;
            let doc_pr = child_mut(frame, "wp:docPr").ok_or("the object has no properties")?;
            set_attr(doc_pr, "hidden", hidden.then(|| "1".to_string()));
            Ok(())
        })
    }

    /// 代替テキスト: `wp:docPr@descr`.
    fn set_object_description(
        &mut self,
        target: FloatingObjectTarget,
        description: &str,
    ) -> Result<(), String> {
        let description = description.to_string();
        self.edit_object(target, "object_description", move |_, drawing| {
            let frame = frame_mut(drawing)?;
            let doc_pr = child_mut(frame, "wp:docPr").ok_or("the object has no properties")?;
            set_attr(
                doc_pr,
                "descr",
                (!description.is_empty()).then(|| description.clone()),
            );
            Ok(())
        })
    }

    // ---- 配置 (揃え) -----------------------------------------------------------

    /// 配置: lines `objects` up, or spreads them evenly, by their boxes on
    /// the page. Each becomes positioned on the page at its new place.
    fn align_objects(
        &mut self,
        objects: &[PlacedObject],
        align: Align,
        to: AlignTo,
    ) -> Result<(), String> {
        let floating: Vec<PlacedObject> = objects
            .iter()
            .copied()
            .filter(|object| {
                self.object_format(object.target)
                    .is_some_and(|format| format.wrap != Wrap::Inline)
            })
            .collect();
        if floating.is_empty() {
            return Err("only floating objects can be aligned".into());
        }
        let page = self.page_setup();
        let (page_width, page_height) = (
            f64::from(page.width) * PX_PER_PT,
            f64::from(page.height) * PX_PER_PT,
        );
        let margin = (
            f64::from(page.margins.left) * PX_PER_PT,
            f64::from(page.margins.top) * PX_PER_PT,
            page_width - f64::from(page.margins.right) * PX_PER_PT,
            page_height - f64::from(page.margins.bottom) * PX_PER_PT,
        );
        let selection = (
            floating.iter().map(|o| o.x).fold(f64::INFINITY, f64::min),
            floating.iter().map(|o| o.y).fold(f64::INFINITY, f64::min),
            floating
                .iter()
                .map(|o| o.x + o.width)
                .fold(f64::NEG_INFINITY, f64::max),
            floating
                .iter()
                .map(|o| o.y + o.height)
                .fold(f64::NEG_INFINITY, f64::max),
        );
        // A single object aligns to the page or margins whatever is asked.
        let (left, top, right, bottom) = match to {
            AlignTo::Page => (0.0, 0.0, page_width, page_height),
            AlignTo::Margin => margin,
            AlignTo::Selection if floating.len() < 2 => margin,
            AlignTo::Selection => selection,
        };
        let mut placed: Vec<(PlacedObject, f64, f64)> = Vec::new();
        match align {
            Align::DistributeHorizontally | Align::DistributeVertically => {
                let horizontal = align == Align::DistributeHorizontally;
                let mut sorted = floating.clone();
                sorted.sort_by(|a, b| {
                    let (ka, kb) = if horizontal { (a.x, b.x) } else { (a.y, b.y) };
                    ka.total_cmp(&kb)
                });
                let total: f64 = sorted
                    .iter()
                    .map(|o| if horizontal { o.width } else { o.height })
                    .sum();
                let (start, end) = if to == AlignTo::Selection && sorted.len() >= 2 {
                    if horizontal {
                        (selection.0, selection.2)
                    } else {
                        (selection.1, selection.3)
                    }
                } else if horizontal {
                    (left, right)
                } else {
                    (top, bottom)
                };
                let gap = if sorted.len() > 1 {
                    ((end - start) - total) / (sorted.len() - 1) as f64
                } else {
                    0.0
                };
                let mut cursor = start;
                for object in sorted {
                    if horizontal {
                        placed.push((object, cursor, object.y));
                        cursor += object.width + gap;
                    } else {
                        placed.push((object, object.x, cursor));
                        cursor += object.height + gap;
                    }
                }
            }
            _ => {
                for object in floating {
                    let (x, y) = match align {
                        Align::Left => (left, object.y),
                        Align::Center => ((left + right - object.width) / 2.0, object.y),
                        Align::Right => (right - object.width, object.y),
                        Align::Top => (object.x, top),
                        Align::Middle => (object.x, (top + bottom - object.height) / 2.0),
                        Align::Bottom => (object.x, bottom - object.height),
                        _ => unreachable!(),
                    };
                    placed.push((object, x, y));
                }
            }
        }
        let commands: Vec<Box<dyn Command>> = placed
            .into_iter()
            .filter(|(object, x, y)| (object.x - x).abs() > 0.01 || (object.y - y).abs() > 0.01)
            .map(|(object, x, y)| {
                let rotation = self
                    .object_format(object.target)
                    .map(|format| format.rotation)
                    .unwrap_or(0.0);
                Box::new(TransformObjectCommand::new(
                    object.target,
                    FloatingObjectTransform {
                        x,
                        y,
                        width: object.width,
                        height: object.height,
                        rotation_deg: rotation,
                    },
                )) as Box<dyn Command>
            })
            .collect();
        if commands.is_empty() {
            return Ok(());
        }
        self.execute_command(Box::new(Sequence::new("align_objects", commands)), true)
    }

    /// Moves and resizes an object to `rect` (x, y, width, height on its
    /// page, CSS px), keeping its rotation — a drag's end.
    fn move_object(
        &mut self,
        object: PlacedObject,
        (x, y, width, height): (f64, f64, f64, f64),
    ) -> Result<(), String> {
        let rotation = self
            .object_format(object.target)
            .map(|format| format.rotation)
            .unwrap_or(0.0);
        self.execute_command(
            Box::new(TransformObjectCommand::new(
                object.target,
                FloatingObjectTransform {
                    x,
                    y,
                    width: width.max(1.0),
                    height: height.max(1.0),
                    rotation_deg: rotation,
                },
            )),
            true,
        )
    }
}

impl<T: WordSession + ?Sized> WordObjects for T {}

/// レイアウトの詳細設定's OK: everything its three tabs say, as one undo
/// step. `None` leaves that part as it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutSettings {
    pub wrap: Option<Wrap>,
    /// 文字列の折り返し → 配置: `bothSides`, `left`, `right`, `largest`.
    pub wrap_side: Option<&'static str>,
    pub horizontal: Option<Position>,
    pub vertical: Option<Position>,
    pub locked: Option<bool>,
    pub allow_overlap: Option<bool>,
    pub layout_in_cell: Option<bool>,
    /// Top, bottom, left, right, in CSS px.
    pub distances: Option<[f64; 4]>,
    /// Width and height in CSS px.
    pub size: Option<(f64, f64)>,
    /// Clockwise degrees.
    pub rotation: Option<f64>,
}

/// The wrap polygon of a tight or through wrap, in DrawingML's
/// 21600-square space; `None` when the object has none written.
pub fn wrap_polygon(
    document: &WordDocument,
    target: FloatingObjectTarget,
) -> Option<Vec<(i64, i64)>> {
    let run = object_run(document, target)?;
    let layout = layout_of(run)?;
    layout.wrap_polygon.as_ref().map(|polygon| {
        polygon
            .points
            .iter()
            .map(|point| (point.x, point.y))
            .collect()
    })
}

/// What the Selection Pane and the ribbon need beside the session.
pub trait WordObjectLayout: WordSession + WordObjects {
    fn apply_layout(
        &mut self,
        target: FloatingObjectTarget,
        settings: LayoutSettings,
    ) -> Result<(), String> {
        let mut commands: Vec<Box<dyn Command>> = Vec::new();
        let current = self.object_format(target).ok_or("the object is gone")?;
        if let Some(wrap) = settings.wrap.filter(|wrap| *wrap != current.wrap) {
            commands.push(wrap_command(target, wrap));
        }
        let floating = settings.wrap.unwrap_or(current.wrap) != Wrap::Inline;
        commands.push(Box::new(edit_drawing_command(
            "object_layout",
            target,
            move |_, drawing| {
                let frame = frame_mut(drawing)?;
                let anchored = frame.name == "wp:anchor";
                if anchored && floating {
                    if let Some(horizontal) = settings.horizontal.as_ref() {
                        let element = position_element("wp:positionH", horizontal)?;
                        drawing_xml::put_child(
                            frame,
                            element,
                            &["wp:positionH"],
                            &["wp:positionV", "wp:extent"],
                        );
                    }
                    if let Some(vertical) = settings.vertical.as_ref() {
                        let element = position_element("wp:positionV", vertical)?;
                        drawing_xml::put_child(frame, element, &["wp:positionV"], &["wp:extent"]);
                    }
                    if settings.horizontal.is_some() || settings.vertical.is_some() {
                        set_attr(frame, "simplePos", Some("0".into()));
                    }
                    for (name, value) in [
                        ("locked", settings.locked),
                        ("allowOverlap", settings.allow_overlap),
                        ("layoutInCell", settings.layout_in_cell),
                    ] {
                        if let Some(value) = value {
                            set_attr(frame, name, Some(u8::from(value).to_string()));
                        }
                    }
                    if let Some(side) = settings.wrap_side {
                        for wrap in ["wp:wrapSquare", "wp:wrapTight", "wp:wrapThrough"] {
                            if let Some(element) = child_mut(frame, wrap) {
                                set_attr(element, "wrapText", Some(side.to_string()));
                            }
                        }
                    }
                }
                if let Some([top, bottom, left, right]) = settings.distances {
                    for (name, value) in [
                        ("distT", top),
                        ("distB", bottom),
                        ("distL", left),
                        ("distR", right),
                    ] {
                        set_attr(frame, name, Some(emu(value.max(0.0)).to_string()));
                    }
                    for wrap in [
                        "wp:wrapSquare",
                        "wp:wrapTight",
                        "wp:wrapThrough",
                        "wp:wrapTopAndBottom",
                    ] {
                        if let Some(element) = child_mut(frame, wrap) {
                            for (name, value) in [
                                ("distT", top),
                                ("distB", bottom),
                                ("distL", left),
                                ("distR", right),
                            ] {
                                if get_attr(element, name).is_some() {
                                    set_attr(element, name, Some(emu(value.max(0.0)).to_string()));
                                }
                            }
                        }
                    }
                }
                if let Some((width, height)) = settings.size {
                    super::picture_format::resize_drawing(drawing, width, height)?;
                }
                if let Some(degrees) = settings.rotation {
                    if let Some(sp_pr) = drawing_xml::shape_properties_mut(drawing) {
                        if let Some(xfrm) = child_mut(sp_pr, "a:xfrm") {
                            let value = (degrees.rem_euclid(360.0) * 60_000.0).round() as i64;
                            set_attr(xfrm, "rot", (value != 0).then(|| value.to_string()));
                        }
                    }
                }
                Ok(())
            },
        )));
        self.execute_command(Box::new(Sequence::new("object_layout", commands)), true)
    }

    /// 折り返し点の編集: the tight or through wrap's polygon, in the
    /// 21600-square space, closed (Word repeats the start at the end).
    fn set_wrap_polygon(
        &mut self,
        target: FloatingObjectTarget,
        points: Vec<(i64, i64)>,
    ) -> Result<(), String> {
        if points.len() < 3 {
            return Err("a wrap polygon needs three points".into());
        }
        self.edit_object(target, "wrap_points", move |_, drawing| {
            let frame = frame_mut(drawing)?;
            let wrap = drawing_xml::child_index(frame, &["wp:wrapTight", "wp:wrapThrough"])
                .ok_or("only tight and through wrapping have wrap points")?;
            let crate::docx_xml::XmlNode::Element(wrap) = &mut frame.children[wrap] else {
                unreachable!()
            };
            let mut path = format!(r#"<wp:start x="{}" y="{}"/>"#, points[0].0, points[0].1);
            for (x, y) in points.iter().skip(1).chain(std::iter::once(&points[0])) {
                path.push_str(&format!(r#"<wp:lineTo x="{x}" y="{y}"/>"#));
            }
            let polygon = fragment(&format!(
                r#"<wp:wrapPolygon edited="1">{path}</wp:wrapPolygon>"#
            ))?;
            drawing_xml::put_child(wrap, polygon, &["wp:wrapPolygon"], &[]);
            Ok(())
        })
    }
}

impl<T: WordSession + ?Sized> WordObjectLayout for T {}

/// 文字列の折り返し as a command, for a picture or a shape.
pub fn wrap_command(target: FloatingObjectTarget, wrap: Wrap) -> Box<dyn Command> {
    match target.kind {
        FloatingObjectKind::Image => Box::new(FormatImageCommand::new(
            target,
            ImageFormatSpec {
                layout: Some(wrap),
                ..Default::default()
            },
        )),
        FloatingObjectKind::TextBox => Box::new(FormatTextBoxCommand::new(
            target,
            TextBoxFormatSpec {
                layout: Some(wrap),
                ..Default::default()
            },
        )),
    }
}

/// レイアウトの詳細設定's オプション; `None` leaves one as it is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ObjectOptions {
    pub moves_with_text: Option<bool>,
    pub locked: Option<bool>,
    pub allow_overlap: Option<bool>,
    pub layout_in_cell: Option<bool>,
    /// Where the object is from the page's top and from its paragraph's,
    /// in CSS px, so switching what it moves with keeps it in place.
    pub page_y: Option<f64>,
    pub paragraph_y: Option<f64>,
}

/// The shape a picture is cropped to, from a TextBoxShape preset name.
pub fn shape_named(name: &str) -> Option<TextBoxShape> {
    TextBoxShape::EXACT_PRESETS
        .into_iter()
        .find(|shape| shape.preset_name() == name)
}

/// Several commands applied in order as one undo step.
pub struct Sequence {
    name: &'static str,
    children: Vec<Box<dyn Command>>,
    applied: usize,
}

impl Sequence {
    pub fn new(name: &'static str, children: Vec<Box<dyn Command>>) -> Self {
        Self {
            name,
            children,
            applied: 0,
        }
    }
}

impl Command for Sequence {
    fn name(&self) -> &'static str {
        self.name
    }

    fn apply(
        &mut self,
        ctx: &mut crate::commands::CommandContext<'_>,
    ) -> crate::commands::CommandResult<()> {
        self.applied = 0;
        for index in 0..self.children.len() {
            if let Err(error) = self.children[index].apply(ctx) {
                for child in self.children[..index].iter_mut().rev() {
                    let _ = child.undo(ctx);
                }
                return Err(error);
            }
            self.applied = index + 1;
        }
        Ok(())
    }

    fn undo(
        &mut self,
        ctx: &mut crate::commands::CommandContext<'_>,
    ) -> crate::commands::CommandResult<()> {
        for child in self.children[..self.applied].iter_mut().rev() {
            child.undo(ctx)?;
        }
        Ok(())
    }

    fn redo(
        &mut self,
        ctx: &mut crate::commands::CommandContext<'_>,
    ) -> crate::commands::CommandResult<()> {
        for child in self.children.iter_mut() {
            child.redo(ctx)?;
        }
        self.applied = self.children.len();
        Ok(())
    }
}
