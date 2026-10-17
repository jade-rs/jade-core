//! The browser entry point for presentations.
//!
//! Nothing here decides how a slide looks. It opens the deck with
//! `jade-pptx-reader`, lays a slide out with `jade-pptx-layout`, and hands the
//! result to `jade-pptx-render`, which draws through `jade_painter::Painter` —
//! the same three crates a native shell would use. What this crate adds is
//! only the browser's half: a canvas, a device pixel ratio, and the JS types
//! either side of the boundary.
//!
//! That is the whole point of the `Painter` seam, and it is why the DOCX and
//! XLSX entry points look like this one. A deck renders in the browser and in
//! a native window from one implementation, so a difference between the two
//! has to come from the backend rather than from a second copy of the drawing
//! code.
//!
//! Existing text runs can be edited and exported without regenerating the
//! deck. The format-specific writer also owns Undo/Redo and the saved baseline.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use jade_canvas_painter::Canvas2dPainter;
use jade_pptx_layout::{layout_deck_slide, SlideLayout};
use jade_pptx_render::{render_slide_with, PainterMeasure, RenderOptions};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageBitmap};

mod object_structure;
mod slide_editing;
mod text_editing;

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// A slide's size in CSS px at 96 dpi, for sizing a pane before painting.
#[wasm_bindgen]
pub struct SlideSize {
    width_px: f64,
    height_px: f64,
}

#[wasm_bindgen]
impl SlideSize {
    #[wasm_bindgen(getter, js_name = widthPx)]
    pub fn width_px(&self) -> f64 {
        self.width_px
    }

    #[wasm_bindgen(getter, js_name = heightPx)]
    pub fn height_px(&self) -> f64 {
        self.height_px
    }
}

/// What one `renderSlide` call painted, so the page can position an overlay
/// and a test can assert something other than pixels.
#[wasm_bindgen]
pub struct SlideRender {
    shapes: usize,
    scale: f64,
    offset_x: f64,
    offset_y: f64,
    width_px: f64,
    height_px: f64,
}

#[wasm_bindgen]
impl SlideRender {
    /// Shapes drawn, the master's and layout's included.
    #[wasm_bindgen(getter)]
    pub fn shapes(&self) -> usize {
        self.shapes
    }

    /// Slide px to CSS px. 1.0 means the slide was drawn at its own size.
    #[wasm_bindgen(getter)]
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// The slide's top-left corner in the canvas, in CSS px — the letterbox
    /// margin.
    #[wasm_bindgen(getter, js_name = offsetX)]
    pub fn offset_x(&self) -> f64 {
        self.offset_x
    }

    #[wasm_bindgen(getter, js_name = offsetY)]
    pub fn offset_y(&self) -> f64 {
        self.offset_y
    }

    /// The painted slide's size in CSS px, after scaling.
    #[wasm_bindgen(getter, js_name = widthPx)]
    pub fn width_px(&self) -> f64 {
        self.width_px
    }

    #[wasm_bindgen(getter, js_name = heightPx)]
    pub fn height_px(&self) -> f64 {
        self.height_px
    }
}

/// A deck held open on the JS side.
#[wasm_bindgen]
pub struct Deck {
    inner: jade_pptx_reader::Deck,
    /// The package as opened, so media parts can be read back for decoding.
    document: jade_pptx_writer::EditSession,
    /// Decoded pictures by part name, supplied by the host: the painter has
    /// no image drawing, and decoding is asynchronous on the web, so the
    /// host decodes and hands bitmaps in; slides repaint as they arrive.
    images: RefCell<HashMap<String, ImageBitmap>>,
    /// Laid-out slides, by 0-based index. Layout is in slide px and so does
    /// not depend on the zoom, which means a slide is laid out once however
    /// often the pane is resized or repainted.
    cached: RefCell<BTreeMap<usize, Rc<SlideLayout>>>,
    object_info: RefCell<BTreeMap<usize, Vec<jade_pptx_writer::ShapeTransformInfo>>>,
    previewing: bool,
    preview_base: Option<(usize, u32, jade_pptx_layout::ObjectTransform)>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EditableObject {
    id: u32,
    name: String,
    kind: String,
    transform: jade_pptx_layout::ObjectTransform,
    can_move: bool,
    can_resize: bool,
    can_rotate: bool,
    lock_aspect_ratio: bool,
    can_duplicate: bool,
    can_delete: bool,
    can_arrange: bool,
    can_group: bool,
}

#[derive(Deserialize)]
struct DragRequest {
    before: jade_pptx_layout::ObjectTransform,
    kind: jade_pptx_layout::DragKind,
    start: (f64, f64),
    current: (f64, f64),
    constrained: bool,
}

fn object_transform(json: &str) -> Result<jade_pptx_reader::Transform, JsValue> {
    let value: jade_pptx_layout::ObjectTransform = serde_json::from_str(json).map_err(js_error)?;
    let r = value.rect;
    if [r.x, r.y, r.width, r.height, value.rotation_degrees]
        .iter()
        .any(|v| !v.is_finite())
        || r.width < 0.0
        || r.height < 0.0
    {
        return Err(JsValue::from_str("object transform must be finite"));
    }
    let angle = (value.rotation_degrees * 60000.0).round();
    if angle < f64::from(i32::MIN) || angle > f64::from(i32::MAX) {
        return Err(js_error("invalid rotation"));
    }
    let transform = jade_pptx_reader::Transform {
        offset_emu: ((r.x * 9525.0).round() as i64, (r.y * 9525.0).round() as i64),
        extent_emu: (
            (r.width * 9525.0).round() as i64,
            (r.height * 9525.0).round() as i64,
        ),
        rotation_60000ths: angle as i32,
        flip_h: value.flip_h,
        flip_v: value.flip_v,
    };
    jade_pptx_writer::validate_transform(transform).map_err(js_error)?;
    Ok(transform)
}

#[wasm_bindgen]
impl Deck {
    #[wasm_bindgen(js_name = setShapeText)]
    pub fn set_shape_text(
        &mut self,
        index: usize,
        id: u32,
        text: &str,
        soft_break: bool,
    ) -> Result<bool, JsValue> {
        self.cancel_object_preview()?;
        let changed = self
            .document
            .set_shape_text(index, id, text, soft_break)
            .map_err(js_error)?;
        if changed {
            self.reload_document()?;
        }
        Ok(changed)
    }
    #[wasm_bindgen(js_name = shapeText)]
    pub fn shape_text(&self, index: usize, id: u32) -> Result<String, JsValue> {
        let value =
            jade_pptx_writer::shape_text(self.document.bytes(), index, id).map_err(js_error)?;
        serde_json::to_string(&value).map_err(js_error)
    }

    #[wasm_bindgen(js_name = replaceTextRange)]
    pub fn replace_text_range(&mut self, index: usize, json: &str) -> Result<bool, JsValue> {
        let edit: jade_pptx_writer::TextRangeEdit = serde_json::from_str(json).map_err(js_error)?;
        self.cancel_object_preview()?;
        let changed = self
            .document
            .replace_text_range(index, &edit)
            .map_err(js_error)?;
        if changed {
            self.reload_document()?;
        }
        Ok(changed)
    }
    fn baseline_transform(
        &self,
        index: usize,
        id: u32,
    ) -> Result<jade_pptx_layout::ObjectTransform, JsValue> {
        if let Some((slide, shape, before)) = self.preview_base {
            if slide == index && shape == id {
                return Ok(before);
            }
        }
        let layout = self.readable_layout(index)?;
        if let Some(group) = layout.groups.iter().find(|group| group.id == id) {
            return Ok(group.transform);
        }
        layout
            .shapes
            .iter()
            .find(|shape| {
                shape.origin == jade_pptx_layout::PlacedOrigin::Slide && shape.source_id == Some(id)
            })
            .map(Into::into)
            .ok_or_else(|| js_error("shape has no layout"))
    }

    fn persisted_transform(
        &self,
        index: usize,
        id: u32,
        json: &str,
    ) -> Result<(jade_pptx_reader::Transform, bool), JsValue> {
        let desired: jade_pptx_layout::ObjectTransform =
            serde_json::from_str(json).map_err(js_error)?;
        let baseline = self.baseline_transform(index, id)?;
        let info = self
            .object_infos(index)?
            .into_iter()
            .find(|info| info.id == id)
            .ok_or_else(|| js_error("shape is not editable"))?;
        let mut value = object_transform(json)?;
        // Autofit may enlarge the displayed frame without changing its stored
        // extent. Moving it must preserve that authored extent and rotation.
        let same = |a: f64, b: f64| (a - b).abs() < 1e-8;
        if let Some(raw) = info.transform {
            if same(desired.rect.width, baseline.rect.width) {
                value.extent_emu.0 = raw.extent_emu.0;
            }
            if same(desired.rect.height, baseline.rect.height) {
                value.extent_emu.1 = raw.extent_emu.1;
            }
            if same(desired.rotation_degrees, baseline.rotation_degrees) {
                value.rotation_60000ths = raw.rotation_60000ths;
            }
        }
        let unchanged = same(desired.rect.x, baseline.rect.x)
            && same(desired.rect.y, baseline.rect.y)
            && same(desired.rect.width, baseline.rect.width)
            && same(desired.rect.height, baseline.rect.height)
            && same(desired.rotation_degrees, baseline.rotation_degrees)
            && desired.flip_h == baseline.flip_h
            && desired.flip_v == baseline.flip_v;
        Ok((value, unchanged))
    }
    fn object_infos(
        &self,
        index: usize,
    ) -> Result<Vec<jade_pptx_writer::ShapeTransformInfo>, JsValue> {
        if let Some(info) = self.object_info.borrow().get(&index) {
            return Ok(info.clone());
        }
        let info =
            jade_pptx_writer::editable_shapes(self.document.bytes(), index).map_err(js_error)?;
        self.object_info.borrow_mut().insert(index, info.clone());
        Ok(info)
    }

    fn readable_layout(&self, index: usize) -> Result<Rc<SlideLayout>, JsValue> {
        if let Some(layout) = self.cached.borrow().get(&index) {
            return Ok(layout.clone());
        }
        layout_deck_slide(&self.inner, index, &jade_pptx_layout::HeuristicMeasure)
            .map(Rc::new)
            .ok_or_else(|| js_error("slide index out of bounds"))
    }

    /// Objects with placement supplied by the same layout the canvas renders.
    #[wasm_bindgen(js_name = editableObjects)]
    pub fn editable_objects(&self, index: usize) -> Result<String, JsValue> {
        let info = self.object_infos(index)?;
        let layout = self.readable_layout(index)?;
        let objects: Vec<_> = info
            .into_iter()
            .filter(|info| info.can_select)
            .filter_map(|info| {
                let transform = if info.kind == "group" {
                    layout
                        .groups
                        .iter()
                        .find(|group| group.id == info.id)?
                        .transform
                } else {
                    let shape = layout.shapes.iter().find(|shape| {
                        shape.origin == jade_pptx_layout::PlacedOrigin::Slide
                            && shape.source_id == Some(info.id)
                    })?;
                    shape.into()
                };
                let unlocked_inheritance = info.transform.is_some()
                    || (info.can_move && info.can_resize && info.can_rotate);
                Some(EditableObject {
                    id: info.id,
                    name: info.name,
                    kind: info.kind,
                    transform,
                    can_move: info.can_move && unlocked_inheritance,
                    can_resize: info.can_resize && unlocked_inheritance,
                    can_rotate: info.can_rotate && unlocked_inheritance,
                    lock_aspect_ratio: info.lock_aspect_ratio,
                    can_duplicate: info.can_duplicate,
                    can_delete: info.can_delete,
                    can_arrange: info.can_arrange,
                    can_group: info.can_group,
                })
            })
            .collect();
        serde_json::to_string(&objects).map_err(js_error)
    }

    #[wasm_bindgen(js_name = hitTestObject)]
    pub fn hit_test_object(
        &self,
        index: usize,
        x: f64,
        y: f64,
        tolerance: f64,
    ) -> Result<Option<u32>, JsValue> {
        let info = self.object_infos(index)?;
        let layout = self.readable_layout(index)?;
        Ok(layout
            .shapes
            .iter()
            .rev()
            .find(|shape| {
                shape.origin == jade_pptx_layout::PlacedOrigin::Slide
                    && shape
                        .group_ids
                        .first()
                        .copied()
                        .or(shape.source_id)
                        .is_some_and(|id| info.iter().any(|info| info.id == id && info.can_select))
                    && shape.contains_point(x, y, tolerance)
            })
            .and_then(|shape| shape.group_ids.first().copied().or(shape.source_id)))
    }

    /// Pure layout arithmetic: start/current coordinates are in slide pixels.
    #[wasm_bindgen(js_name = dragObjectTransform)]
    pub fn drag_object_transform(&self, request: &str) -> Result<String, JsValue> {
        let request: DragRequest = serde_json::from_str(request).map_err(js_error)?;
        let after = jade_pptx_layout::drag_transform(
            request.before,
            request.kind,
            request.start,
            request.current,
            request.constrained,
        )
        .ok_or_else(|| js_error("invalid drag coordinates"))?;
        serde_json::to_string(&after).map_err(js_error)
    }

    #[wasm_bindgen(js_name = previewObjectTransform)]
    pub fn preview_object_transform(
        &mut self,
        index: usize,
        id: u32,
        json: &str,
    ) -> Result<(), JsValue> {
        if self
            .preview_base
            .is_some_and(|(slide, shape, _)| slide != index || shape != id)
        {
            self.cancel_object_preview()?;
        }
        let baseline = self.baseline_transform(index, id)?;
        let (transform, unchanged) = self.persisted_transform(index, id, json)?;
        let info = self
            .object_infos(index)?
            .into_iter()
            .find(|info| info.id == id)
            .ok_or_else(|| js_error("shape is not editable"))?;
        jade_pptx_writer::validate_shape_transform(&info, transform).map_err(js_error)?;
        let slide = self
            .inner
            .slides
            .get_mut(index)
            .ok_or_else(|| js_error("slide index out of bounds"))?;
        let target = slide
            .content
            .shapes
            .iter_mut()
            .find(|item| match item {
                jade_pptx_reader::ShapeItem::Shape(shape) => shape.id == Some(id),
                jade_pptx_reader::ShapeItem::Picture(picture) => picture.id == Some(id),
                jade_pptx_reader::ShapeItem::Group(group) => group.id == Some(id),
                _ => false,
            })
            .ok_or_else(|| js_error("shape cannot be previewed"))?;
        let after = if unchanged {
            info.transform
        } else {
            Some(transform)
        };
        match target {
            jade_pptx_reader::ShapeItem::Shape(shape) => shape.transform = after,
            jade_pptx_reader::ShapeItem::Picture(picture) => picture.transform = after,
            jade_pptx_reader::ShapeItem::Group(group) => {
                group
                    .transform
                    .as_mut()
                    .ok_or_else(|| js_error("group has no coordinate space"))?
                    .outer = after.ok_or_else(|| js_error("group has no transform"))?;
            }
            _ => unreachable!(),
        }
        self.cached.get_mut().remove(&index);
        self.previewing = true;
        self.preview_base = Some((index, id, baseline));
        Ok(())
    }

    #[wasm_bindgen(js_name = cancelObjectPreview)]
    pub fn cancel_object_preview(&mut self) -> Result<(), JsValue> {
        if self.previewing {
            self.reload_document()?;
        }
        Ok(())
    }

    #[wasm_bindgen(js_name = setObjectTransform)]
    pub fn set_object_transform(
        &mut self,
        index: usize,
        id: u32,
        json: &str,
    ) -> Result<bool, JsValue> {
        let (transform, unchanged) = self.persisted_transform(index, id, json)?;
        if unchanged {
            self.cancel_object_preview()?;
            return Ok(false);
        }
        let changed = self
            .document
            .set_shape_transform(index, id, transform)
            .map_err(js_error)?;
        if changed || self.previewing {
            self.reload_document()?;
        }
        Ok(changed)
    }
    /// Ordinary slide-owned text runs, as JSON. Fields and inherited shapes
    /// are excluded so editing cannot silently detach their semantics.
    #[wasm_bindgen(js_name = editableRuns)]
    pub fn editable_runs(&self, index: usize) -> Result<String, JsValue> {
        let runs =
            jade_pptx_writer::editable_runs(self.document.bytes(), index).map_err(js_error)?;
        serde_json::to_string(&runs).map_err(js_error)
    }

    #[wasm_bindgen(js_name = setRunText)]
    pub fn set_run_text(
        &mut self,
        index: usize,
        shape_id: u32,
        paragraph: usize,
        run: usize,
        text: &str,
    ) -> Result<bool, JsValue> {
        self.cancel_object_preview()?;
        let changed = self
            .document
            .set_run_text(
                index,
                jade_pptx_writer::RunAddress {
                    shape_id,
                    paragraph,
                    run,
                },
                text,
            )
            .map_err(js_error)?;
        if changed {
            self.reload_document()?;
        }
        Ok(changed)
    }

    pub fn undo(&mut self) -> Result<bool, JsValue> {
        let changed = self.document.undo();
        if changed {
            self.reload_document()?;
        }
        Ok(changed)
    }

    pub fn redo(&mut self) -> Result<bool, JsValue> {
        let changed = self.document.redo();
        if changed {
            self.reload_document()?;
        }
        Ok(changed)
    }

    #[wasm_bindgen(js_name = canUndo)]
    pub fn can_undo(&self) -> bool {
        self.document.can_undo()
    }

    #[wasm_bindgen(js_name = canRedo)]
    pub fn can_redo(&self) -> bool {
        self.document.can_redo()
    }

    #[wasm_bindgen(js_name = isDirty)]
    pub fn is_dirty(&self) -> bool {
        self.document.is_dirty()
    }

    #[wasm_bindgen(js_name = retainedArchiveBytes)]
    pub fn retained_archive_bytes(&self) -> usize {
        self.document.retained_archive_bytes()
    }

    /// Call only after the host successfully persists the exported bytes.
    #[wasm_bindgen(js_name = markSaved)]
    pub fn mark_saved(&mut self) {
        self.document.mark_saved();
    }

    #[wasm_bindgen(js_name = exportPptx)]
    pub fn export_pptx(&self) -> Vec<u8> {
        self.document.bytes().to_vec()
    }

    fn reload_document(&mut self) -> Result<(), JsValue> {
        self.inner = jade_pptx_reader::read_deck(self.document.bytes()).map_err(js_error)?;
        self.cached.get_mut().clear();
        self.object_info.get_mut().clear();
        self.previewing = false;
        self.preview_base = None;
        Ok(())
    }

    /// Opens a `.pptx`. Rejects with the reader's message when the package is
    /// not one.
    #[wasm_bindgen(js_name = fromBytes)]
    pub fn from_bytes(bytes: &[u8]) -> Result<Deck, JsValue> {
        let inner = jade_pptx_reader::read_deck(bytes)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        Ok(Deck {
            inner,
            document: jade_pptx_writer::EditSession::open(bytes).map_err(js_error)?,
            images: RefCell::new(HashMap::new()),
            cached: RefCell::new(BTreeMap::new()),
            object_info: RefCell::new(BTreeMap::new()),
            previewing: false,
            preview_base: None,
        })
    }

    /// Every media part a picture on any slide draws, deduplicated, in the
    /// order first seen. The host decodes these and passes them to
    /// `setImage`.
    #[wasm_bindgen(js_name = mediaParts)]
    pub fn media_parts(&self) -> Vec<String> {
        let mut parts: Vec<String> = Vec::new();
        for slide in &self.inner.slides {
            collect_media_parts(&slide.content.shapes, &mut parts);
            if let Some(layout) = &slide.layout_content {
                collect_media_parts(&layout.shapes, &mut parts);
            }
            if let Some(master) = &slide.master_content {
                collect_media_parts(&master.shapes, &mut parts);
            }
        }
        parts
    }

    /// A media part's bytes, for the host to decode. `None` when the package
    /// has no such part.
    #[wasm_bindgen(js_name = mediaBytes)]
    pub fn media_bytes(&self, part: &str) -> Option<Vec<u8>> {
        jade_pptx_headless_cli::media(self.document.bytes(), part)
            .ok()
            .map(|(bytes, _)| bytes)
    }

    /// The MIME type a media part's name implies, so the host can build a
    /// Blob the browser will decode.
    #[wasm_bindgen(js_name = mediaType)]
    pub fn media_type(&self, part: &str) -> Option<String> {
        jade_pptx_headless_cli::media(self.document.bytes(), part)
            .ok()
            .map(|(_, content_type)| content_type)
    }

    /// Registers a decoded picture for `part`. The next `renderSlide` draws
    /// it where the placeholder was.
    #[wasm_bindgen(js_name = setImage)]
    pub fn set_image(&self, part: &str, image: ImageBitmap) {
        self.images.borrow_mut().insert(part.to_string(), image);
    }

    #[wasm_bindgen(js_name = slideCount)]
    pub fn slide_count(&self) -> usize {
        self.inner.slides.len()
    }

    /// The deck's slide size. Every slide in a deck shares it — PowerPoint
    /// has no per-slide size — so this takes no index.
    #[wasm_bindgen(js_name = slideSize)]
    pub fn slide_size(&self) -> SlideSize {
        SlideSize {
            width_px: self.inner.slide_size.width_px(),
            height_px: self.inner.slide_size.height_px(),
        }
    }

    /// The title placeholder's text, for a filmstrip label. `None` when the
    /// slide has no title — the caller shows the slide number instead rather
    /// than inventing one.
    #[wasm_bindgen(js_name = slideTitle)]
    pub fn slide_title(&self, index: usize) -> Option<String> {
        self.inner
            .slides
            .get(index)
            .and_then(|slide| jade_pptx_headless_cli::slide_title(&slide.content.shapes))
    }

    /// Whether the slide carries speaker notes, so a pane can be hidden
    /// without reading them.
    #[wasm_bindgen(js_name = hasNotes)]
    pub fn has_notes(&self, index: usize) -> bool {
        self.inner
            .slides
            .get(index)
            .map(|slide| slide.notes.is_some())
            .unwrap_or(false)
    }

    /// Speaker notes, paragraphs joined with newlines.
    #[wasm_bindgen(js_name = slideNotes)]
    pub fn slide_notes(&self, index: usize) -> Option<String> {
        self.inner
            .slides
            .get(index)
            .and_then(|slide| slide.notes.clone())
    }

    /// Every run of text on the slide, in z-order, paragraphs on their own
    /// lines. What a search box or an assistant reads.
    #[wasm_bindgen(js_name = slideText)]
    pub fn slide_text(&self, index: usize) -> Option<String> {
        let layout = self.cached.borrow().get(&index).cloned();
        let layout = match layout {
            Some(layout) => layout,
            // Text comes off the laid-out slide rather than the raw shape
            // tree, so what is read is what is drawn: a placeholder that
            // inherits its text from the layout reads the same either way.
            None => Rc::new(layout_deck_slide(
                &self.inner,
                index,
                &jade_pptx_layout::HeuristicMeasure,
            )?),
        };
        let mut text = String::new();
        for shape in &layout.shapes {
            for line in &shape.lines {
                let mut wrote = false;
                for run in &line.runs {
                    text.push_str(&run.text);
                    wrote = true;
                }
                if wrote {
                    text.push('\n');
                }
            }
        }
        Some(text)
    }

    /// Paints one slide, scaled to fit the viewport and centred in it.
    ///
    /// The canvas is sized to the *viewport*, not to the slide. A deck viewer
    /// zooms to fit whatever pane it is in, and a canvas sized to the slide
    /// would either overflow the pane or need the browser to scale a bitmap —
    /// which is how a slide's text ends up soft. Scaling the painting instead
    /// keeps glyphs on the device's own pixel grid at every zoom.
    ///
    /// `device_pixel_ratio` scales the backing store; the canvas keeps its
    /// CSS size in layout px.
    #[wasm_bindgen(js_name = renderSlide)]
    pub fn render_slide(
        &self,
        index: usize,
        canvas: &HtmlCanvasElement,
        device_pixel_ratio: f64,
        viewport_width: f64,
        viewport_height: f64,
    ) -> Result<SlideRender, JsValue> {
        let ratio = if device_pixel_ratio.is_finite() && device_pixel_ratio > 0.0 {
            device_pixel_ratio
        } else {
            1.0
        };
        let width = viewport_width.max(1.0);
        let height = viewport_height.max(1.0);
        size_canvas(canvas, width, height, ratio)?;

        let context = canvas
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("the canvas has no 2d context"))?
            .dyn_into::<CanvasRenderingContext2d>()?;
        let options = RenderOptions::default();

        // A 16:9 slide in a 4:3 pane has to leave a margin, and that margin
        // is the page's chrome rather than part of the deck. Clearing it —
        // instead of painting a colour chosen here — lets the element's own
        // background show through, so the letterbox follows the host's light
        // or dark theme without this crate having to know about themes.
        context.scale(ratio, ratio)?;
        context.clear_rect(0.0, 0.0, width, height);

        let layout = self.layout(index, &context)?;
        let scale = (width / layout.width_px.max(1.0)).min(height / layout.height_px.max(1.0));
        let painted_width = layout.width_px * scale;
        let painted_height = layout.height_px * scale;
        let offset_x = ((width - painted_width) / 2.0).max(0.0);
        let offset_y = ((height - painted_height) / 2.0).max(0.0);

        context.translate(offset_x, offset_y)?;
        context.scale(scale, scale)?;
        let painter = Canvas2dPainter::new(&context);
        let images = self.images.borrow();
        let mut draw_image = |shape: &jade_pptx_layout::PlacedShape| -> bool {
            let Some(bitmap) = shape
                .image_part
                .as_deref()
                .and_then(|part| images.get(part))
            else {
                return false;
            };
            // The painter has already applied the shape's rotation and flips
            // to this same context, so the bitmap lands like the shape would.
            let Some((source, destination)) =
                shape.image_rectangles(f64::from(bitmap.width()), f64::from(bitmap.height()))
            else {
                return false;
            };
            context
                .draw_image_with_image_bitmap_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                    bitmap,
                    source.x,
                    source.y,
                    source.width,
                    source.height,
                    destination.x,
                    destination.y,
                    destination.width,
                    destination.height,
                )
                .is_ok()
        };
        render_slide_with(&painter, &layout, &options, &mut draw_image)?;
        // The transform is the context's state, not the painter's, so it has
        // to come back off or the next frame would compound it.
        context.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0)?;

        Ok(SlideRender {
            shapes: layout.shapes.len(),
            scale,
            offset_x,
            offset_y,
            width_px: painted_width,
            height_px: painted_height,
        })
    }

    /// The laid-out slide, measured against the canvas that will draw it.
    fn layout(
        &self,
        index: usize,
        context: &CanvasRenderingContext2d,
    ) -> Result<Rc<SlideLayout>, JsValue> {
        if let Some(cached) = self.cached.borrow().get(&index) {
            return Ok(cached.clone());
        }
        // Measuring happens in slide px, at the slide's own scale, so the
        // cached layout is reusable at every zoom. The painter is only borrowed
        // for measurement here; the transform it will draw under is set after.
        let options = RenderOptions::default();
        let measure = PainterMeasure::new(Canvas2dPainter::new(context), &options.default_font);
        let layout = layout_deck_slide(&self.inner, index, &measure)
            .ok_or_else(|| JsValue::from_str(&format!("the deck has no slide {}", index + 1)))?;
        let layout = Rc::new(layout);
        self.cached.borrow_mut().insert(index, layout.clone());
        Ok(layout)
    }
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

/// Every `image_part` under `shapes`, depth first, each once.
fn collect_media_parts(shapes: &[jade_pptx_reader::ShapeItem], out: &mut Vec<String>) {
    use jade_pptx_reader::ShapeItem;
    for item in shapes {
        let part = match item {
            ShapeItem::Picture(picture) => picture.image_part.as_deref(),
            ShapeItem::Shape(shape) => shape.image_part.as_deref(),
            ShapeItem::Group(group) => {
                collect_media_parts(&group.children, out);
                None
            }
            ShapeItem::Table(_) | ShapeItem::Chart(_) => None,
        };
        if let Some(part) = part {
            if !out.iter().any(|seen| seen == part) {
                out.push(part.to_string());
            }
        }
    }
}

fn size_canvas(
    canvas: &HtmlCanvasElement,
    viewport_width: f64,
    viewport_height: f64,
    device_pixel_ratio: f64,
) -> Result<(), JsValue> {
    let width = viewport_width.max(1.0);
    let height = viewport_height.max(1.0);
    canvas.set_width((width * device_pixel_ratio).ceil() as u32);
    canvas.set_height((height * device_pixel_ratio).ceil() as u32);
    let style = canvas.style();
    style.set_property("width", &format!("{width}px"))?;
    style.set_property("height", &format!("{height}px"))?;
    Ok(())
}
