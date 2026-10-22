//! Browser entry points for 配置 (位置, 文字列の折り返し, 前面へ/背面へ,
//! 配置/整列, グループ化, オブジェクトの選択と表示) and 図形のスタイル, from
//! [`crate::word::objects`], [`crate::word::group`] and
//! [`crate::word::shapes`], run on the view as a session.
//!
//! An object goes out to script as its `target` — `{paragraph, run, kind}`
//! in the model's own paragraph numbering — which comes back as it is.

use super::word_models::{from_js, to_js};
use super::*;
use crate::word::group::WordGroups;
use crate::word::objects::{
    Align, AlignTo, Arrange, ObjectFormat, ObjectOptions, ObjectSort, Placement, Position,
    RelativeTo, WordObjects,
};
use crate::word::picture::{PlacedObject, Wrap};
use crate::word::shapes::{ShapeStyle, WordShapes};
use crate::word::WordSession;
use serde_json::{json, Value};

#[derive(serde::Deserialize, serde::Serialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TargetJs {
    paragraph: usize,
    run: usize,
    kind: String,
}

impl TargetJs {
    fn of(target: FloatingObjectTarget) -> Self {
        TargetJs {
            paragraph: target.paragraph_index,
            run: target.run_index,
            kind: match target.kind {
                FloatingObjectKind::Image => "image",
                FloatingObjectKind::TextBox => "textBox",
            }
            .to_string(),
        }
    }

    fn target(&self) -> Result<FloatingObjectTarget, String> {
        Ok(FloatingObjectTarget {
            paragraph_index: self.paragraph,
            run_index: self.run,
            kind: match self.kind.as_str() {
                "image" => FloatingObjectKind::Image,
                "textBox" => FloatingObjectKind::TextBox,
                other => return Err(format!("unknown object kind {other}")),
            },
        })
    }
}

/// One axis of 位置: what it is relative to, and an alignment or an offset
/// in CSS px.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PositionJs {
    relative_to: String,
    #[serde(default)]
    align: Option<String>,
    #[serde(default)]
    offset: Option<f64>,
}

impl PositionJs {
    fn position(&self) -> Result<Position, String> {
        let relative_to = RelativeTo::from_ooxml(&self.relative_to)
            .ok_or_else(|| format!("unknown position base {}", self.relative_to))?;
        let placement = match (&self.align, self.offset) {
            (Some(align), _) => Placement::Align(align.clone()),
            (None, Some(offset)) => Placement::Offset(offset),
            (None, None) => return Err("a position needs an alignment or an offset".into()),
        };
        Ok(Position {
            relative_to,
            placement,
        })
    }
}

fn position_js(position: &Option<Position>) -> Value {
    match position {
        Some(Position {
            relative_to,
            placement,
        }) => match placement {
            Placement::Align(align) => json!({ "relativeTo": relative_to.ooxml(), "align": align }),
            Placement::Offset(offset) => {
                json!({ "relativeTo": relative_to.ooxml(), "offset": offset })
            }
        },
        None => Value::Null,
    }
}

fn wrap_name(wrap: Wrap) -> &'static str {
    match wrap {
        Wrap::Inline => "inline",
        Wrap::Square => "square",
        Wrap::Tight => "tight",
        Wrap::Through => "through",
        Wrap::TopAndBottom => "topAndBottom",
        Wrap::BehindText => "behindText",
        Wrap::InFrontOfText => "inFrontOfText",
    }
}

fn sort_name(sort: ObjectSort) -> &'static str {
    match sort {
        ObjectSort::Picture => "picture",
        ObjectSort::Shape => "shape",
        ObjectSort::TextBox => "textBox",
        ObjectSort::WordArt => "wordArt",
        ObjectSort::Group => "group",
        ObjectSort::Chart => "chart",
        ObjectSort::SmartArt => "smartArt",
    }
}

fn object_js(format: &ObjectFormat) -> Value {
    json!({
        "target": TargetJs::of(format.target),
        "sort": sort_name(format.sort),
        "name": format.name,
        "width": format.width,
        "height": format.height,
        "rotation": format.rotation,
        "wrap": wrap_name(format.wrap),
        "horizontal": position_js(&format.horizontal),
        "vertical": position_js(&format.vertical),
        "movesWithText": format.moves_with_text,
        "locked": format.locked,
        "allowOverlap": format.allow_overlap,
        "layoutInCell": format.layout_in_cell,
        "hidden": format.hidden,
        "zOrder": format.z_order,
        "behindText": format.behind_text,
    })
}

/// 位置 and 文字列の折り返し's オプション, each field optional.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
struct LayoutJs {
    horizontal: Option<PositionJs>,
    vertical: Option<PositionJs>,
    moves_with_text: Option<bool>,
    locked: Option<bool>,
    allow_overlap: Option<bool>,
    layout_in_cell: Option<bool>,
    hidden: Option<bool>,
    name: Option<String>,
    /// `forward`, `backward`, `toFront`, `toBack`, `inFrontOfText`,
    /// `behindText`.
    arrange: Option<String>,
}

impl OffscreenDocumentView {
    fn object_target(&self, target: JsValue) -> Result<FloatingObjectTarget, JsValue> {
        if target.is_null() || target.is_undefined() {
            return self
                .state
                .borrow()
                .object_selection
                .ok_or_else(|| JsValue::from_str("オブジェクトが選択されていません"));
        }
        let target: TargetJs = from_js(target, "object")?;
        target.target().map_err(|error| JsValue::from_str(&error))
    }

    /// Where each object was drawn at the last render, on its page.
    fn placed_objects(
        &self,
        targets: &[FloatingObjectTarget],
    ) -> Result<Vec<PlacedObject>, JsValue> {
        let regions: Vec<(FloatingObjectTarget, usize, f64, f64, f64, f64)> = {
            let renderer = self.renderer.borrow();
            renderer
                .image_debug_regions()
                .iter()
                .map(|region| {
                    (
                        FloatingObjectTarget {
                            paragraph_index: region.paragraph_index,
                            run_index: region.run_index,
                            kind: FloatingObjectKind::Image,
                        },
                        region.page,
                        region.x,
                        region.y,
                        region.width,
                        region.height,
                    )
                })
                .chain(renderer.text_box_debug_regions().iter().map(|region| {
                    (
                        FloatingObjectTarget {
                            paragraph_index: region.paragraph_index,
                            run_index: region.run_index,
                            kind: FloatingObjectKind::TextBox,
                        },
                        region.page,
                        region.x,
                        region.y,
                        region.width,
                        region.height,
                    )
                }))
                .collect()
        };
        targets
            .iter()
            .map(|target| {
                let (_, page, x, y, width, height) = regions
                    .iter()
                    .find(|region| region.0 == *target)
                    .copied()
                    .ok_or_else(|| JsValue::from_str("the object is not on a drawn page"))?;
                Ok(PlacedObject {
                    target: *target,
                    page: page.saturating_sub(1),
                    x,
                    y: y - self.pages_above(page),
                    width,
                    height,
                })
            })
            .collect()
    }

    fn object_targets(&self, targets: JsValue) -> Result<Vec<FloatingObjectTarget>, JsValue> {
        let targets: Vec<TargetJs> = from_js(targets, "objects")?;
        targets
            .iter()
            .map(|target| target.target().map_err(|error| JsValue::from_str(&error)))
            .collect()
    }
}

#[wasm_bindgen]
impl OffscreenDocumentView {
    /// オブジェクトの選択と表示: every object in the body with its layout, in
    /// stacking order.
    #[wasm_bindgen(js_name = bodyObjects)]
    pub fn body_objects(&self) -> Result<JsValue, JsValue> {
        let objects = self.read_word_session(|session| {
            session
                .body_objects()
                .iter()
                .map(object_js)
                .collect::<Vec<_>>()
        })?;
        Ok(to_js(&objects))
    }

    /// The selected object's layout (or `target`'s), or `null`.
    #[wasm_bindgen(js_name = objectLayout)]
    pub fn object_layout(&self, target: JsValue) -> Result<JsValue, JsValue> {
        let Ok(target) = self.object_target(target) else {
            return Ok(JsValue::NULL);
        };
        let format = self.read_word_session(|session| session.object_format(target))?;
        Ok(format.map_or(JsValue::NULL, |format| to_js(&object_js(&format))))
    }

    /// Selects `target` as a click on it would.
    #[wasm_bindgen(js_name = selectObjectTarget)]
    pub fn select_object_target(&self, target: JsValue) -> Result<(), JsValue> {
        let target = self.object_target(target)?;
        self.state.borrow_mut().object_selection = Some(target);
        Ok(())
    }

    /// 位置, 文字列と一緒に移動する, アンカーを段落に固定する, 重なりを許可する,
    /// 表のセル内に配置する, 表示/非表示, 名前 and 前面へ/背面へ for the
    /// selected object (or `target`), as one undo step.
    #[wasm_bindgen(js_name = setObjectLayout)]
    pub fn set_object_layout(&self, target: JsValue, layout: JsValue) -> Result<JsValue, JsValue> {
        let target = self.object_target(target)?;
        let layout: LayoutJs = from_js(layout, "object layout")?;
        self.run_word_session("word_object_layout", move |session| {
            if layout.horizontal.is_some() || layout.vertical.is_some() {
                let current = session.object_format(target).ok_or("the object is gone")?;
                let horizontal = match &layout.horizontal {
                    Some(position) => position.position()?,
                    None => current.horizontal.clone().unwrap_or(Position {
                        relative_to: RelativeTo::Column,
                        placement: Placement::Offset(0.0),
                    }),
                };
                let vertical = match &layout.vertical {
                    Some(position) => position.position()?,
                    None => current.vertical.clone().unwrap_or(Position {
                        relative_to: RelativeTo::Paragraph,
                        placement: Placement::Offset(0.0),
                    }),
                };
                session.set_object_position(target, horizontal, vertical)?;
            }
            if layout.moves_with_text.is_some()
                || layout.locked.is_some()
                || layout.allow_overlap.is_some()
                || layout.layout_in_cell.is_some()
            {
                session.set_object_options(
                    target,
                    ObjectOptions {
                        moves_with_text: layout.moves_with_text,
                        locked: layout.locked,
                        allow_overlap: layout.allow_overlap,
                        layout_in_cell: layout.layout_in_cell,
                        page_y: None,
                        paragraph_y: None,
                    },
                )?;
            }
            if let Some(hidden) = layout.hidden {
                session.set_object_hidden(target, hidden)?;
            }
            if let Some(name) = &layout.name {
                session.rename_object(target, name)?;
            }
            if let Some(arrange) = &layout.arrange {
                let arrange = match arrange.as_str() {
                    "forward" => Arrange::Forward,
                    "backward" => Arrange::Backward,
                    "toFront" => Arrange::ToFront,
                    "toBack" => Arrange::ToBack,
                    "inFrontOfText" => Arrange::InFrontOfText,
                    "behindText" => Arrange::BehindText,
                    other => return Err(format!("unknown arrangement {other}")),
                };
                session.arrange_object(target, arrange)?;
            }
            Ok(())
        })
    }

    /// 配置: `objects` aligned (`left`, `center`, `right`, `top`, `middle`,
    /// `bottom`) or distributed (`distributeHorizontally`,
    /// `distributeVertically`) to the `page`, `margin` or `selection`.
    #[wasm_bindgen(js_name = alignObjects)]
    pub fn align_objects(
        &self,
        objects: JsValue,
        align: String,
        to: String,
    ) -> Result<JsValue, JsValue> {
        let targets = self.object_targets(objects)?;
        let placed = self.placed_objects(&targets)?;
        let align = match align.as_str() {
            "left" => Align::Left,
            "center" => Align::Center,
            "right" => Align::Right,
            "top" => Align::Top,
            "middle" => Align::Middle,
            "bottom" => Align::Bottom,
            "distributeHorizontally" => Align::DistributeHorizontally,
            "distributeVertically" => Align::DistributeVertically,
            other => return Err(JsValue::from_str(&format!("unknown alignment {other}"))),
        };
        let to = match to.as_str() {
            "page" => AlignTo::Page,
            "margin" => AlignTo::Margin,
            "selection" => AlignTo::Selection,
            other => {
                return Err(JsValue::from_str(&format!(
                    "unknown alignment base {other}"
                )))
            }
        };
        self.run_word_session("word_align_objects", move |session| {
            session.align_objects(&placed, align, to)
        })
    }

    /// グループ化: `objects` (two or more) become one group.
    #[wasm_bindgen(js_name = groupObjects)]
    pub fn group_objects(&self, objects: JsValue) -> Result<JsValue, JsValue> {
        let targets = self.object_targets(objects)?;
        if targets.len() < 2 {
            return Err(JsValue::from_str(
                "グループ化するには 2 つ以上のオブジェクトを選んでください",
            ));
        }
        let placed = self.placed_objects(&targets)?;
        self.run_word_session("word_group_objects", move |session| {
            session.group_objects(&placed)
        })
    }

    /// グループ解除 of the selected group (or `target`).
    #[wasm_bindgen(js_name = ungroupObject)]
    pub fn ungroup_object(&self, target: JsValue) -> Result<JsValue, JsValue> {
        let target = self.object_target(target)?;
        let placed = self
            .placed_objects(&[target])?
            .pop()
            .ok_or_else(|| JsValue::from_str("the group is not on a drawn page"))?;
        self.run_word_session("word_ungroup_object", move |session| {
            session.ungroup_object(placed)
        })
    }

    /// 図形のスタイル: the gallery's `row` (塗りつぶし, 枠線, 光沢…) and
    /// `column` (the theme colour) on the selected shape.
    #[wasm_bindgen(js_name = applyShapeStyle)]
    pub fn apply_shape_style(&self, row: usize, column: usize) -> Result<JsValue, JsValue> {
        let target = self.object_target(JsValue::NULL)?;
        self.run_word_session("word_shape_style", move |session| {
            session.apply_shape_style(target, ShapeStyle::gallery(row, column))
        })
    }

    /// 頂点の編集: the selected shape's (or `target`'s) first geometry path —
    /// `{width, height, fill, stroke, commands}` — or `null` for a shape with
    /// none of its own.
    #[wasm_bindgen(js_name = shapePath)]
    pub fn shape_path(&self, target: JsValue) -> Result<JsValue, JsValue> {
        let target = self.object_target(target)?;
        let path = self.read_word_session(|session| {
            crate::word::objects::object_run(session.word_document(), target)
                .and_then(|run| run.text_box.as_ref())
                .and_then(|text_box| text_box.extras().custom_geometry.as_ref())
                .and_then(|geometry| geometry.paths.first().cloned())
        })?;
        Ok(path.map_or(JsValue::NULL, |path| to_js(&path)))
    }

    /// 頂点の編集's result: the shape's path rewritten, one undo step.
    #[wasm_bindgen(js_name = setShapePath)]
    pub fn set_shape_path(&self, target: JsValue, path: JsValue) -> Result<JsValue, JsValue> {
        use crate::word::connectors::WordConnectors;
        let target = self.object_target(target)?;
        let path: crate::document::GeometryPath = from_js(path, "shape path")?;
        self.run_word_session("word_edit_points", move |session| {
            session.set_freeform_path(target, path)
        })
    }

    /// 折り返し点の編集: the tight or through wrap's polygon, in the
    /// 21600-square space, or `null` when the object has none.
    #[wasm_bindgen(js_name = wrapPoints)]
    pub fn wrap_points(&self, target: JsValue) -> Result<JsValue, JsValue> {
        let target = self.object_target(target)?;
        let points = self.read_word_session(|session| {
            crate::word::objects::wrap_polygon(session.word_document(), target)
        })?;
        Ok(points.map_or(JsValue::NULL, |points| to_js(&points)))
    }

    /// 折り返し点の編集's result: the wrap polygon (21600-square space,
    /// three points or more), one undo step.
    #[wasm_bindgen(js_name = setWrapPoints)]
    pub fn set_wrap_points(&self, target: JsValue, points: JsValue) -> Result<JsValue, JsValue> {
        use crate::word::objects::WordObjectLayout;
        let target = self.object_target(target)?;
        let mut points: Vec<(i64, i64)> = from_js(points, "wrap points")?;
        // The polygon as read is closed (its start repeated at the end); the
        // model closes it again, so the repeat goes.
        if points.len() > 3 && points.first() == points.last() {
            points.pop();
        }
        self.run_word_session("word_wrap_points", move |session| {
            session.set_wrap_polygon(target, points)
        })
    }

    /// 図形 ▸ フリーフォーム: a shape through `points` (page px from the
    /// caret's page), closed or open, at the caret, one undo step.
    #[wasm_bindgen(js_name = insertFreeform)]
    pub fn insert_freeform(&self, points: JsValue, closed: bool) -> Result<JsValue, JsValue> {
        use crate::word::connectors::WordConnectors;
        let points: Vec<(f64, f64)> = from_js(points, "freeform points")?;
        if points.len() < 2 {
            return Err(JsValue::from_str("a freeform needs two points or more"));
        }
        self.run_word_session("word_insert_freeform", move |session| {
            session.insert_freeform(&points, closed)
        })
    }

    /// 文字列の折り返し for the selected object (or `target`): `inline`,
    /// `square`, `tight`, `through`, `topAndBottom`, `behindText` or
    /// `inFrontOfText`, one undo step.
    #[wasm_bindgen(js_name = setObjectWrap)]
    pub fn set_object_wrap(&self, target: JsValue, wrap: String) -> Result<JsValue, JsValue> {
        let target = self.object_target(target)?;
        let wrap = match wrap.as_str() {
            "inline" => Wrap::Inline,
            "square" => Wrap::Square,
            "tight" => Wrap::Tight,
            "through" => Wrap::Through,
            "topAndBottom" => Wrap::TopAndBottom,
            "behindText" => Wrap::BehindText,
            "inFrontOfText" => Wrap::InFrontOfText,
            other => return Err(JsValue::from_str(&format!("unknown wrap {other}"))),
        };
        self.run_word_session("word_object_wrap", move |session| {
            session.set_object_wrap(target, wrap)
        })
    }
}
