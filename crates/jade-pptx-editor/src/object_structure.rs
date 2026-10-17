use crate::{js_error, Deck};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TextBoxRequest {
    rect: jade_pptx_layout::Rect,
    #[serde(default)]
    text: String,
    #[serde(default)]
    format: jade_pptx_writer::TextFormat,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PictureRequest {
    #[serde(default)]
    rect: Option<jade_pptx_layout::Rect>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CropRequest {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}
fn crop_request(json: &str) -> Result<jade_pptx_reader::SourceRect, JsValue> {
    let crop: CropRequest = serde_json::from_str(json).map_err(js_error)?;
    let offset = |value: f64| {
        let raw = (value * 1000.0).round();
        if !raw.is_finite() || raw < f64::from(i32::MIN) || raw > f64::from(i32::MAX) {
            Err(js_error("invalid crop percentage"))
        } else {
            Ok(raw as i32)
        }
    };
    Ok(jade_pptx_reader::SourceRect {
        left: offset(crop.left)?,
        top: offset(crop.top)?,
        right: offset(crop.right)?,
        bottom: offset(crop.bottom)?,
    })
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreparedPicture {
    rect: jade_pptx_layout::Rect,
    #[serde(flatten)]
    info: jade_pptx_writer::PictureInfo,
}
fn text_box(json: &str) -> Result<(jade_pptx_reader::Transform, TextBoxRequest), JsValue> {
    let request: TextBoxRequest = serde_json::from_str(json).map_err(js_error)?;
    let transform = crate::object_transform(
        &serde_json::to_string(&jade_pptx_layout::ObjectTransform {
            rect: request.rect,
            rotation_degrees: 0.0,
            flip_h: false,
            flip_v: false,
        })
        .map_err(js_error)?,
    )?;
    jade_pptx_writer::validate_text_box(transform, &request.text, &request.format)
        .map_err(js_error)?;
    Ok((transform, request))
}
fn shape(
    json: &str,
) -> Result<(jade_pptx_reader::Transform, jade_pptx_writer::ShapeInsert), JsValue> {
    let mut value: serde_json::Value = serde_json::from_str(json).map_err(js_error)?;
    let rect = value
        .as_object_mut()
        .and_then(|object| object.remove("rect"))
        .ok_or_else(|| js_error("shape rect is required"))?;
    let rect = serde_json::from_value(rect).map_err(js_error)?;
    let request = serde_json::from_value(value).map_err(js_error)?;
    let transform = crate::object_transform(
        &serde_json::to_string(&jade_pptx_layout::ObjectTransform {
            rect,
            rotation_degrees: 0.0,
            flip_h: false,
            flip_v: false,
        })
        .map_err(js_error)?,
    )?;
    jade_pptx_writer::validate_shape_insert(transform, &request).map_err(js_error)?;
    Ok((transform, request))
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResolvedOutlineState {
    color: String,
    width_pt: f64,
    dash_pt: Vec<f64>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ShapeStyleState {
    #[serde(flatten)]
    authored: jade_pptx_writer::ShapeStyleInfo,
    resolved_outline: Option<ResolvedOutlineState>,
}
#[wasm_bindgen]
impl Deck {
    #[wasm_bindgen(js_name = shapeStyle)]
    pub fn shape_style(&self, index: usize, id: u32) -> Result<String, JsValue> {
        let Some(authored) =
            jade_pptx_writer::shape_style(self.document.bytes(), index, id).map_err(js_error)?
        else {
            return Ok("null".into());
        };
        let layout = self.readable_layout(index)?;
        let resolved_outline = layout
            .shapes
            .iter()
            .find(|s| s.origin == jade_pptx_layout::PlacedOrigin::Slide && s.source_id == Some(id))
            .and_then(|s| s.outline.as_ref())
            .map(|o| ResolvedOutlineState {
                color: o.color.clone(),
                width_pt: o.width_px * (1.0 / jade_pptx_layout::PX_PER_PT),
                dash_pt: o
                    .dash_px
                    .iter()
                    .map(|p| p * (1.0 / jade_pptx_layout::PX_PER_PT))
                    .collect(),
            });
        serde_json::to_string(&ShapeStyleState {
            authored,
            resolved_outline,
        })
        .map_err(js_error)
    }
    #[wasm_bindgen(js_name = validateShapeStyle)]
    pub fn validate_shape_style(&self, index: usize, id: u32, json: &str) -> Result<(), JsValue> {
        let value = serde_json::from_str(json).map_err(js_error)?;
        jade_pptx_writer::validate_shape_style(self.document.bytes(), index, id, &value)
            .map_err(js_error)
    }
    #[wasm_bindgen(js_name = setShapeStyle)]
    pub fn set_shape_style(&mut self, index: usize, id: u32, json: &str) -> Result<bool, JsValue> {
        let value = serde_json::from_str(json).map_err(js_error)?;
        let changed = self
            .document
            .set_shape_style(index, id, &value)
            .map_err(js_error)?;
        if changed {
            self.reload_document()?;
        }
        Ok(changed)
    }
    #[wasm_bindgen(js_name = validateGroupObjects)]
    pub fn validate_group_objects(&self, index: usize, ids: &str) -> Result<(), JsValue> {
        let ids: Vec<u32> = serde_json::from_str(ids).map_err(js_error)?;
        jade_pptx_writer::validate_group_objects(self.document.bytes(), index, &ids)
            .map_err(js_error)
    }
    #[wasm_bindgen(js_name = groupObjects)]
    pub fn group_objects(&mut self, index: usize, ids: &str) -> Result<u32, JsValue> {
        let ids: Vec<u32> = serde_json::from_str(ids).map_err(js_error)?;
        let id = self.document.group_objects(index, &ids).map_err(js_error)?;
        self.reload_document()?;
        Ok(id)
    }
    #[wasm_bindgen(js_name = pictureReplacement)]
    pub fn picture_replacement(&self, index: usize, id: u32) -> Result<String, JsValue> {
        serde_json::to_string(
            &jade_pptx_writer::picture_replacement(self.document.bytes(), index, id)
                .map_err(js_error)?,
        )
        .map_err(js_error)
    }
    #[wasm_bindgen(js_name = validatePictureReplacement)]
    pub fn validate_picture_replacement(
        &self,
        index: usize,
        id: u32,
        image: &[u8],
    ) -> Result<String, JsValue> {
        let state = jade_pptx_writer::picture_replacement(self.document.bytes(), index, id)
            .map_err(js_error)?
            .ok_or_else(|| js_error("not an embedded image"))?;
        if !state.can_replace {
            return Err(js_error("image replacement is locked or unsupported"));
        }
        let info = jade_pptx_writer::picture_info(image).map_err(js_error)?;
        serde_json::to_string(&serde_json::json!({ "fingerprint":state.fingerprint,"width":info.width,"height":info.height,"mimeType":info.mime_type })).map_err(js_error)
    }
    #[wasm_bindgen(js_name = replacePicture)]
    pub fn replace_picture(
        &mut self,
        index: usize,
        id: u32,
        image: &[u8],
        fingerprint: &str,
    ) -> Result<String, JsValue> {
        let replaced = self
            .document
            .replace_picture(index, id, image, fingerprint)
            .map_err(js_error)?;
        if replaced.changed {
            self.reload_document()?;
        }
        serde_json::to_string(&replaced).map_err(js_error)
    }
    #[wasm_bindgen(js_name = pictureCrop)]
    pub fn picture_crop(&self, index: usize, id: u32) -> Result<String, JsValue> {
        let state =
            jade_pptx_writer::picture_crop(self.document.bytes(), index, id).map_err(js_error)?;
        let state = state.map(|state| serde_json::json!({ "crop": { "left": f64::from(state.crop.left)/1000.0, "top": f64::from(state.crop.top)/1000.0, "right": f64::from(state.crop.right)/1000.0, "bottom": f64::from(state.crop.bottom)/1000.0 }, "canEdit": state.can_edit, "canPreserveScale": state.can_preserve_scale, "picture": state.picture }));
        serde_json::to_string(&state).map_err(js_error)
    }
    #[wasm_bindgen(js_name = validatePictureCrop)]
    pub fn validate_picture_crop(
        &self,
        index: usize,
        id: u32,
        json: &str,
        keep_frame: bool,
    ) -> Result<(), JsValue> {
        jade_pptx_writer::validate_picture_crop_edit(
            self.document.bytes(),
            index,
            id,
            crop_request(json)?,
            keep_frame,
        )
        .map_err(js_error)
    }
    #[wasm_bindgen(js_name = setPictureCrop)]
    pub fn set_picture_crop(
        &mut self,
        index: usize,
        id: u32,
        json: &str,
        keep_frame: bool,
    ) -> Result<bool, JsValue> {
        let changed = self
            .document
            .set_picture_crop(index, id, crop_request(json)?, keep_frame)
            .map_err(js_error)?;
        if changed || self.previewing {
            self.reload_document()?;
        }
        Ok(changed)
    }
    fn prepare_picture(
        &self,
        index: usize,
        image: &[u8],
        json: &str,
    ) -> Result<(jade_pptx_reader::Transform, PreparedPicture), JsValue> {
        if index >= self.inner.slides.len() {
            return Err(js_error("slide index out of bounds"));
        }
        let request: PictureRequest = serde_json::from_str(json).map_err(js_error)?;
        let info = jade_pptx_writer::picture_info(image).map_err(js_error)?;
        let rect = request.rect.unwrap_or_else(|| {
            let (w, h) = (
                self.inner.slide_size.width_px(),
                self.inner.slide_size.height_px(),
            );
            let scale = (w * 0.7 / f64::from(info.width))
                .min(h * 0.7 / f64::from(info.height))
                .min(1.0);
            let (width, height) = (
                f64::from(info.width) * scale,
                f64::from(info.height) * scale,
            );
            jade_pptx_layout::Rect {
                x: (w - width) / 2.0,
                y: (h - height) / 2.0,
                width,
                height,
            }
        });
        let transform = crate::object_transform(
            &serde_json::to_string(&jade_pptx_layout::ObjectTransform {
                rect,
                rotation_degrees: 0.0,
                flip_h: false,
                flip_v: false,
            })
            .map_err(js_error)?,
        )?;
        if transform.extent_emu.0 <= 0 || transform.extent_emu.1 <= 0 {
            return Err(js_error("picture dimensions must be positive"));
        }
        Ok((transform, PreparedPicture { rect, info }))
    }
    #[wasm_bindgen(js_name = validatePicture)]
    pub fn validate_picture(
        &self,
        index: usize,
        image: &[u8],
        json: &str,
    ) -> Result<String, JsValue> {
        let (_, prepared) = self.prepare_picture(index, image, json)?;
        serde_json::to_string(&prepared).map_err(js_error)
    }
    #[wasm_bindgen(js_name = insertPicture)]
    pub fn insert_picture(
        &mut self,
        index: usize,
        image: &[u8],
        json: &str,
    ) -> Result<String, JsValue> {
        if index >= self.inner.slides.len() {
            return Err(js_error("slide index out of bounds"));
        }
        let request: PictureRequest = serde_json::from_str(json).map_err(js_error)?;
        let rect = request
            .rect
            .ok_or_else(|| js_error("picture bounds are required after validation"))?;
        let transform = crate::object_transform(
            &serde_json::to_string(&jade_pptx_layout::ObjectTransform {
                rect,
                rotation_degrees: 0.0,
                flip_h: false,
                flip_v: false,
            })
            .map_err(js_error)?,
        )?;
        let inserted = self
            .document
            .insert_picture(index, transform, image)
            .map_err(js_error)?;
        self.reload_document()?;
        serde_json::to_string(&inserted).map_err(js_error)
    }
    #[wasm_bindgen(js_name = textBoxBounds)]
    pub fn text_box_bounds(
        &self,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        constrained: bool,
    ) -> Result<String, JsValue> {
        let rect = jade_pptx_layout::creation_rect((x1, y1), (x2, y2), constrained)
            .ok_or_else(|| js_error("invalid creation coordinates"))?;
        serde_json::to_string(&rect).map_err(js_error)
    }
    #[wasm_bindgen(js_name = validateTextBox)]
    pub fn validate_text_box(&self, index: usize, json: &str) -> Result<(), JsValue> {
        if index >= self.inner.slides.len() {
            return Err(js_error("slide index out of bounds"));
        }
        text_box(json)?;
        Ok(())
    }
    #[wasm_bindgen(js_name = insertTextBox)]
    pub fn insert_text_box(&mut self, index: usize, json: &str) -> Result<u32, JsValue> {
        if index >= self.inner.slides.len() {
            return Err(js_error("slide index out of bounds"));
        }
        let (transform, request) = text_box(json)?;
        self.cancel_object_preview()?;
        let id = self
            .document
            .insert_text_box(index, transform, &request.text, &request.format)
            .map_err(js_error)?;
        self.reload_document()?;
        Ok(id)
    }
    #[wasm_bindgen(js_name = validateShape)]
    pub fn validate_shape(&self, index: usize, json: &str) -> Result<(), JsValue> {
        if index >= self.inner.slides.len() {
            return Err(js_error("slide index out of bounds"));
        }
        shape(json)?;
        Ok(())
    }
    #[wasm_bindgen(js_name = insertShape)]
    pub fn insert_shape(&mut self, index: usize, json: &str) -> Result<u32, JsValue> {
        if index >= self.inner.slides.len() {
            return Err(js_error("slide index out of bounds"));
        }
        let (transform, request) = shape(json)?;
        self.cancel_object_preview()?;
        let id = self
            .document
            .insert_shape(index, transform, &request)
            .map_err(js_error)?;
        self.reload_document()?;
        Ok(id)
    }
    #[wasm_bindgen(js_name = duplicateObject)]
    pub fn duplicate_object(&mut self, index: usize, id: u32) -> Result<u32, JsValue> {
        self.cancel_object_preview()?;
        let new_id = self.document.duplicate_shape(index, id).map_err(js_error)?;
        self.reload_document()?;
        Ok(new_id)
    }
    #[wasm_bindgen(js_name = deleteObject)]
    pub fn delete_object(&mut self, index: usize, id: u32) -> Result<bool, JsValue> {
        self.cancel_object_preview()?;
        let changed = self.document.delete_shape(index, id).map_err(js_error)?;
        if changed {
            self.reload_document()?;
        }
        Ok(changed)
    }
    #[wasm_bindgen(js_name = arrangeObject)]
    pub fn arrange_object(
        &mut self,
        index: usize,
        id: u32,
        direction: &str,
    ) -> Result<bool, JsValue> {
        let direction = serde_json::from_value::<jade_pptx_writer::ObjectArrangement>(
            serde_json::Value::String(direction.into()),
        )
        .map_err(js_error)?;
        self.cancel_object_preview()?;
        let changed = self
            .document
            .arrange_shape(index, id, direction)
            .map_err(js_error)?;
        if changed {
            self.reload_document()?;
        }
        Ok(changed)
    }
}
