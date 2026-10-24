//! Connectors (図形 → 線's コネクタ) and freeforms (フリーフォーム, 頂点の編集)
//! over any [`WordSession`]: a connector drawn from one shape's connection
//! point to another's is glued to them (`a:stCxn` / `a:endCxn`) and follows
//! them when they move; a freeform is drawn point by point and its points
//! are moved afterwards.

use crate::commands::edit_drawing::edit_drawing_command;
use crate::commands::insert_shape::InsertShapeCommand;
use crate::commands::transform_object::TransformObjectCommand;
use crate::commands::Command;
use crate::document::{GeometryPath, PathCommand, TextBoxShape};
use crate::docx_xml::XmlNode;
use crate::drawing_xml::{
    self, child_mut, descendant_mut, put_shape_property, remove_children, set_attr,
};
use crate::word_document::{
    FloatingObjectKind, FloatingObjectTarget, FloatingObjectTransform, ShapeInsertSpec,
};

use super::objects::{fragment, object_run, Sequence, WordObjects};
use super::picture::PlacedObject;
use super::shapes::{apply_style, InsertThen, Inserted, ShapeStyle};
use super::WordSession;

/// The kinds of connector the 線 gallery draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectorKind {
    Straight,
    Elbow,
    Curved,
}

impl ConnectorKind {
    fn preset(self) -> &'static str {
        match self {
            ConnectorKind::Straight => "straightConnector1",
            ConnectorKind::Elbow => "bentConnector3",
            ConnectorKind::Curved => "curvedConnector3",
        }
    }

    fn shape(self) -> TextBoxShape {
        match self {
            ConnectorKind::Straight => TextBoxShape::Line,
            ConnectorKind::Elbow => TextBoxShape::LineBent,
            ConnectorKind::Curved => TextBoxShape::LineCurved,
        }
    }
}

/// A shape's connection points, as DrawingML numbers a rectangle's: top,
/// left, bottom, right (`a:cxnLst`). Other presets' further points map onto
/// these four.
pub fn connection_point(object: &PlacedObject, index: u32) -> (f64, f64) {
    let (x, y, width, height) = (object.x, object.y, object.width, object.height);
    match index % 4 {
        0 => (x + width / 2.0, y),
        1 => (x, y + height / 2.0),
        2 => (x + width / 2.0, y + height),
        _ => (x + width, y + height / 2.0),
    }
}

/// The connection point of `object` nearest `point`.
pub fn nearest_connection(object: &PlacedObject, point: (f64, f64)) -> u32 {
    (0..4)
        .min_by(|a, b| {
            let distance = |index: u32| {
                let (px, py) = connection_point(object, index);
                (px - point.0).powi(2) + (py - point.1).powi(2)
            };
            distance(*a).total_cmp(&distance(*b))
        })
        .unwrap_or(0)
}

/// One end of a connector being drawn: where it is, and the shape and
/// connection point it is glued to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConnectorEnd {
    pub point: (f64, f64),
    pub glued: Option<(FloatingObjectTarget, u32)>,
}

/// A connector's frame from its two ends: the box between them, flipped
/// when it runs right to left or bottom to top.
fn frame((x0, y0): (f64, f64), (x1, y1): (f64, f64)) -> (f64, f64, f64, f64, bool, bool) {
    (
        x0.min(x1),
        y0.min(y1),
        (x1 - x0).abs().max(1.0),
        (y1 - y0).abs().max(1.0),
        x1 < x0,
        y1 < y0,
    )
}

/// The connector edit: its geometry, flips and the shapes it is glued to.
fn connector_drawing_edit(
    kind: ConnectorKind,
    arrow: bool,
    flips: (bool, bool),
    ends: [Option<(u32, u32)>; 2],
) -> impl FnMut(
    &mut crate::word_document::WordDocument,
    &mut crate::docx_xml::XmlElement,
) -> Result<(), String>
       + 'static {
    move |_, drawing| {
        apply_style(drawing, &ShapeStyle::line())?;
        let sp_pr = drawing_xml::shape_properties_mut(drawing).ok_or("no shape")?;
        put_shape_property(
            sp_pr,
            fragment(&format!(
                r#"<a:prstGeom prst="{}"><a:avLst/></a:prstGeom>"#,
                kind.preset()
            ))?,
            &["a:custGeom", "a:prstGeom"],
        );
        if let Some(xfrm) = child_mut(sp_pr, "a:xfrm") {
            set_attr(xfrm, "flipH", flips.0.then(|| "1".into()));
            set_attr(xfrm, "flipV", flips.1.then(|| "1".into()));
        }
        if arrow {
            if let Some(line) = child_mut(sp_pr, "a:ln") {
                remove_children(line, &["a:tailEnd"]);
                line.children.push(XmlNode::Element(fragment(
                    r#"<a:tailEnd type="triangle"/>"#,
                )?));
            }
        }
        let shape = descendant_mut(drawing, "wps:wsp").ok_or("no shape")?;
        // A connector names its glue in `wps:cNvCnPr` and holds no text.
        remove_children(shape, &["wps:cNvSpPr", "wps:cNvCnPr", "wps:txbx"]);
        let mut glue = String::new();
        if let Some((id, index)) = ends[0] {
            glue.push_str(&format!(r#"<a:stCxn id="{id}" idx="{index}"/>"#));
        }
        if let Some((id, index)) = ends[1] {
            glue.push_str(&format!(r#"<a:endCxn id="{id}" idx="{index}"/>"#));
        }
        shape.children.insert(
            0,
            XmlNode::Element(fragment(&format!("<wps:cNvCnPr>{glue}</wps:cNvCnPr>"))?),
        );
        Ok(())
    }
}

/// Connectors and freeforms, for every session.
pub trait WordConnectors: WordSession + WordObjects {
    /// コネクタ: drawn from `start` to `end` (page coordinates), glued to
    /// the shapes the ends are on.
    fn insert_connector(
        &mut self,
        kind: ConnectorKind,
        arrow: bool,
        start: ConnectorEnd,
        end: ConnectorEnd,
    ) -> Result<(), String> {
        let range = self.selection_range().ok_or("no caret")?;
        let (caret, _) = range.normalized();
        let id_of = |session: &Self, glued: Option<(FloatingObjectTarget, u32)>| {
            glued.and_then(|(target, index)| {
                session
                    .object_format(target)
                    .and_then(|format| format.id)
                    .map(|id| (id, index))
            })
        };
        let ends = [id_of(self, start.glued), id_of(self, end.glued)];
        let (x, y, width, height, flip_h, flip_v) = frame(start.point, end.point);
        let insert = InsertShapeCommand::new(
            caret.paragraph,
            caret.offset,
            ShapeInsertSpec {
                shape: kind.shape(),
                width_px: width.clamp(12.0, 4096.0),
                height_px: height.clamp(12.0, 4096.0),
                fill_color: None,
                border_color: Some("000000".into()),
                border_width_px: 1.0,
                text: String::new(),
                text_color: None,
                text_font_size_pt: None,
                text_bold: None,
                text_italic: None,
                text_alignment: None,
            },
        );
        self.execute_command(
            Box::new(InsertThen::new(Inserted::Shape(insert), move |target| {
                vec![
                    Box::new(TransformObjectCommand::new(
                        target,
                        FloatingObjectTransform {
                            x,
                            y,
                            width,
                            height,
                            rotation_deg: 0.0,
                        },
                    )) as Box<dyn Command>,
                    Box::new(edit_drawing_command(
                        "insert_connector",
                        target,
                        connector_drawing_edit(kind, arrow, (flip_h, flip_v), ends),
                    )),
                ]
            })),
            false,
        )
    }

    /// After shapes moved: every connector glued to one is set between its
    /// ends again. `regions` say where the objects are on their pages (the
    /// last paint's). Returns whether any moved.
    fn reroute_connectors(&mut self, regions: &[PlacedObject]) -> Result<bool, String> {
        let region = |target: FloatingObjectTarget| {
            regions
                .iter()
                .find(|placed| placed.target == target)
                .copied()
        };
        let objects = self.body_objects();
        let by_id = |id: u32| {
            objects
                .iter()
                .find(|object| object.id == Some(id))
                .map(|o| o.target)
        };
        let mut commands: Vec<Box<dyn Command>> = Vec::new();
        for object in &objects {
            let Some(text_box) = object_run(self.word_document(), object.target)
                .and_then(|run| run.text_box.as_ref())
            else {
                continue;
            };
            let Some(glue) = text_box.extras().connector else {
                continue;
            };
            let Some(current) = region(object.target) else {
                continue;
            };
            let end_point = |reference: Option<crate::document::ConnectionReference>,
                             fallback: (f64, f64)| {
                reference
                    .and_then(|reference| {
                        by_id(reference.id).map(|target| (target, reference.index))
                    })
                    .and_then(|(target, index)| {
                        region(target).map(|placed| connection_point(&placed, index))
                    })
                    .unwrap_or(fallback)
            };
            // Where the ends are now, flips considered.
            let (left, right) = (current.x, current.x + current.width);
            let (top, bottom) = (current.y, current.y + current.height);
            let start_now = (
                if object.flip_horizontal { right } else { left },
                if object.flip_vertical { bottom } else { top },
            );
            let end_now = (
                if object.flip_horizontal { left } else { right },
                if object.flip_vertical { top } else { bottom },
            );
            let start = end_point(glue.start, start_now);
            let end = end_point(glue.end, end_now);
            let (x, y, width, height, flip_h, flip_v) = frame(start, end);
            let moved = (x - current.x).abs() > 0.5
                || (y - current.y).abs() > 0.5
                || (width - current.width).abs() > 0.5
                || (height - current.height).abs() > 0.5
                || flip_h != object.flip_horizontal
                || flip_v != object.flip_vertical;
            if !moved {
                continue;
            }
            let target = object.target;
            commands.push(Box::new(TransformObjectCommand::new(
                target,
                FloatingObjectTransform {
                    x,
                    y,
                    width,
                    height,
                    rotation_deg: 0.0,
                },
            )));
            commands.push(Box::new(edit_drawing_command(
                "reroute_connector",
                target,
                move |_, drawing| {
                    let sp_pr = drawing_xml::shape_properties_mut(drawing).ok_or("no shape")?;
                    if let Some(xfrm) = child_mut(sp_pr, "a:xfrm") {
                        set_attr(xfrm, "flipH", flip_h.then(|| "1".into()));
                        set_attr(xfrm, "flipV", flip_v.then(|| "1".into()));
                    }
                    Ok(())
                },
            )));
        }
        if commands.is_empty() {
            return Ok(false);
        }
        self.execute_command(
            Box::new(Sequence::new("reroute_connectors", commands)),
            true,
        )?;
        Ok(true)
    }

    /// フリーフォーム: a shape through `points` (page coordinates), closed
    /// or open, styled as a new shape (an open one as a line).
    fn insert_freeform(&mut self, points: &[(f64, f64)], closed: bool) -> Result<(), String> {
        if points.len() < 2 {
            return Err("a freeform needs two points".into());
        }
        let range = self.selection_range().ok_or("no caret")?;
        let (caret, _) = range.normalized();
        let left = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
        let top = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        let right = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
        let bottom = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
        let (width, height) = ((right - left).max(12.0), (bottom - top).max(12.0));
        let emu = |value: f64| (value * 9525.0).round();
        let mut commands = vec![PathCommand::MoveTo(
            emu(points[0].0 - left),
            emu(points[0].1 - top),
        )];
        commands.extend(
            points[1..]
                .iter()
                .map(|(x, y)| PathCommand::LineTo(emu(x - left), emu(y - top))),
        );
        if closed {
            commands.push(PathCommand::Close);
        }
        let path = GeometryPath {
            width: emu(width),
            height: emu(height),
            fill: closed,
            stroke: true,
            commands,
        };
        let insert = InsertShapeCommand::new(
            caret.paragraph,
            caret.offset,
            ShapeInsertSpec {
                shape: TextBoxShape::Rectangle,
                width_px: width.min(4096.0),
                height_px: height.min(4096.0),
                fill_color: None,
                border_color: Some("000000".into()),
                border_width_px: 1.0,
                text: String::new(),
                text_color: None,
                text_font_size_pt: None,
                text_bold: None,
                text_italic: None,
                text_alignment: None,
            },
        );
        self.execute_command(
            Box::new(InsertThen::new(Inserted::Shape(insert), move |target| {
                let path = path.clone();
                vec![
                    Box::new(TransformObjectCommand::new(
                        target,
                        FloatingObjectTransform {
                            x: left,
                            y: top,
                            width,
                            height,
                            rotation_deg: 0.0,
                        },
                    )) as Box<dyn Command>,
                    Box::new(edit_drawing_command(
                        "insert_freeform",
                        target,
                        move |_, drawing| {
                            apply_style(
                                drawing,
                                &if closed {
                                    ShapeStyle::default_shape()
                                } else {
                                    ShapeStyle::line()
                                },
                            )?;
                            set_geometry(drawing, &path)?;
                            if let Some(doc_pr) = descendant_mut(drawing, "wp:docPr") {
                                let id = drawing_xml::get_attr(doc_pr, "id")
                                    .unwrap_or("1")
                                    .to_string();
                                set_attr(doc_pr, "name", Some(format!("フリーフォーム {id}")));
                            }
                            Ok(())
                        },
                    )),
                ]
            })),
            false,
        )
    }

    /// 頂点の編集: the freeform's path becomes `path` (in its own
    /// coordinates).
    fn set_freeform_path(
        &mut self,
        target: FloatingObjectTarget,
        path: GeometryPath,
    ) -> Result<(), String> {
        if target.kind != FloatingObjectKind::TextBox {
            return Err("only a shape has points to edit".into());
        }
        self.edit_object(target, "edit_points", move |_, drawing| {
            set_geometry(drawing, &path)
        })
    }
}

impl<T: WordSession + ?Sized> WordConnectors for T {}

/// Writes `path` as the shape's `a:custGeom`.
pub fn set_geometry(
    drawing: &mut crate::docx_xml::XmlElement,
    path: &GeometryPath,
) -> Result<(), String> {
    let mut commands = String::new();
    for command in &path.commands {
        let point = |x: f64, y: f64| {
            format!(
                r#"<a:pt x="{}" y="{}"/>"#,
                x.round() as i64,
                y.round() as i64
            )
        };
        commands.push_str(&match command {
            PathCommand::MoveTo(x, y) => format!("<a:moveTo>{}</a:moveTo>", point(*x, *y)),
            PathCommand::LineTo(x, y) => format!("<a:lnTo>{}</a:lnTo>", point(*x, *y)),
            PathCommand::CubicTo(x1, y1, x2, y2, x, y) => format!(
                "<a:cubicBezTo>{}{}{}</a:cubicBezTo>",
                point(*x1, *y1),
                point(*x2, *y2),
                point(*x, *y)
            ),
            PathCommand::QuadTo(x1, y1, x, y) => {
                format!(
                    "<a:quadBezTo>{}{}</a:quadBezTo>",
                    point(*x1, *y1),
                    point(*x, *y)
                )
            }
            PathCommand::ArcTo {
                width_radius,
                height_radius,
                start_deg,
                sweep_deg,
            } => format!(
                r#"<a:arcTo wR="{}" hR="{}" stAng="{}" swAng="{}"/>"#,
                width_radius.round() as i64,
                height_radius.round() as i64,
                (start_deg * 60_000.0).round() as i64,
                (sweep_deg * 60_000.0).round() as i64
            ),
            PathCommand::Close => "<a:close/>".to_string(),
        });
    }
    let fill = if path.fill { "" } else { r#" fill="none""# };
    let geometry = fragment(&format!(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:rect l="l" t="t" r="r" b="b"/><a:pathLst><a:path w="{}" h="{}"{fill}>{commands}</a:path></a:pathLst></a:custGeom>"#,
        path.width.round() as i64,
        path.height.round() as i64,
    ))?;
    let sp_pr = drawing_xml::shape_properties_mut(drawing).ok_or("the object is not a shape")?;
    put_shape_property(sp_pr, geometry, &["a:custGeom", "a:prstGeom"]);
    Ok(())
}

/// The freeform's points on the page: each command's end point, mapped
/// from the path's space into the shape's box.
pub fn path_points(path: &GeometryPath, object: &PlacedObject) -> Vec<(f64, f64)> {
    let scale_x = if path.width > 0.0 {
        object.width / path.width
    } else {
        1.0 / 9525.0
    };
    let scale_y = if path.height > 0.0 {
        object.height / path.height
    } else {
        1.0 / 9525.0
    };
    path.commands
        .iter()
        .filter_map(|command| match command {
            PathCommand::MoveTo(x, y)
            | PathCommand::LineTo(x, y)
            | PathCommand::CubicTo(_, _, _, _, x, y)
            | PathCommand::QuadTo(_, _, x, y) => {
                Some((object.x + x * scale_x, object.y + y * scale_y))
            }
            _ => None,
        })
        .collect()
}

/// `path` with its `index`th point (counting as [`path_points`] does) moved
/// to `point` on the page; a curve's control points move with its end.
pub fn move_path_point(
    path: &GeometryPath,
    object: &PlacedObject,
    index: usize,
    point: (f64, f64),
) -> GeometryPath {
    let scale_x = if path.width > 0.0 {
        object.width / path.width
    } else {
        1.0 / 9525.0
    };
    let scale_y = if path.height > 0.0 {
        object.height / path.height
    } else {
        1.0 / 9525.0
    };
    let (x, y) = (
        (point.0 - object.x) / scale_x,
        (point.1 - object.y) / scale_y,
    );
    let mut path = path.clone();
    let mut seen = 0;
    for command in &mut path.commands {
        let is_point = !matches!(command, PathCommand::ArcTo { .. } | PathCommand::Close);
        if !is_point {
            continue;
        }
        if seen == index {
            match command {
                PathCommand::MoveTo(px, py) | PathCommand::LineTo(px, py) => {
                    *px = x;
                    *py = y;
                }
                PathCommand::CubicTo(_, _, x2, y2, px, py) => {
                    let (dx, dy) = (x - *px, y - *py);
                    *x2 += dx;
                    *y2 += dy;
                    *px = x;
                    *py = y;
                }
                PathCommand::QuadTo(x1, y1, px, py) => {
                    let (dx, dy) = (x - *px, y - *py);
                    *x1 += dx / 2.0;
                    *y1 += dy / 2.0;
                    *px = x;
                    *py = y;
                }
                _ => {}
            }
            break;
        }
        seen += 1;
    }
    path
}
