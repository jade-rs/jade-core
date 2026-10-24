//! The browser entry points for Word's 描画 tab: ink kept as Word keeps it
//! (`w14:contentPart` with an InkML part), so Word opens the strokes as
//! ink it can recolour and erase, not as a picture.

use super::word_models::to_js;
use super::*;
use crate::commands::set_ink_drawings::SetInkDrawingsCommand;
use crate::word_document::ink_drawing::{InkDrawingInfo, InkDrawingSpec};
use jade_docx_reader::inkml::{InkBrush, InkTrace};
use serde_json::json;

impl OffscreenDocumentView {
    fn ink_drawing_list(&self) -> Vec<InkDrawingInfo> {
        self.state
            .borrow()
            .word_doc
            .as_ref()
            .map(WordDocument::ink_drawings)
            .unwrap_or_default()
    }

    fn keep_ink(drawings: &[InkDrawingInfo], except: Option<usize>) -> Vec<InkDrawingSpec> {
        drawings
            .iter()
            .enumerate()
            .filter(|(index, _)| Some(*index) != except)
            .map(|(_, drawing)| InkDrawingSpec::Keep {
                paragraph_index: drawing.paragraph_index,
                run_index: drawing.run_index,
            })
            .collect()
    }

    fn set_ink(&self, specs: Vec<InkDrawingSpec>) -> Result<JsValue, JsValue> {
        self.apply_document_command(move |_| Box::new(SetInkDrawingsCommand::new(specs)))
    }

    fn append_ink(&self, traces: Vec<InkTrace>) -> Result<JsValue, JsValue> {
        let (start, _) = self.current_selection().normalized();
        let mut specs = Self::keep_ink(&self.ink_drawing_list(), None);
        specs.push(InkDrawingSpec::New {
            paragraph_index: start.paragraph,
            traces,
        });
        self.set_ink(specs)
    }
}

#[wasm_bindgen]
impl OffscreenDocumentView {
    /// The body's ink drawings, in document order, with their strokes (px
    /// within each drawing's box).
    #[wasm_bindgen(js_name = inkDrawings)]
    pub fn ink_drawings(&self) -> JsValue {
        self.ink_drawing_list()
            .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
            .unwrap_or(JsValue::NULL)
    }

    /// A new ink drawing of `traces` (`[{ points: [[x, y], …], brush }]`, in
    /// page px from the page's top-left) anchored in the caret's paragraph;
    /// the other drawings stay.
    #[wasm_bindgen(js_name = addInkDrawing)]
    pub fn add_ink_drawing(&self, traces: JsValue) -> Result<JsValue, JsValue> {
        let traces: Vec<InkTrace> = serde_wasm_bindgen::from_value(traces)
            .map_err(|error| JsValue::from_str(&format!("invalid ink: {error}")))?;
        if traces.iter().all(|trace| trace.points.is_empty()) {
            return Err(JsValue::from_str("ink needs at least one stroke"));
        }
        self.append_ink(traces)
    }

    /// Draws a page-local stroke, recognising shapes before committing ink.
    /// A recognised shape or the original brush/trace is one undo step.
    #[wasm_bindgen(js_name = drawInkStroke)]
    pub fn draw_ink_stroke(&self, page: usize, trace: JsValue) -> Result<JsValue, JsValue> {
        use crate::word::ink_tools::{recognize_shape, InkTool, Pen, WordInkTools};
        let trace: InkTrace = serde_wasm_bindgen::from_value(trace)
            .map_err(|error| JsValue::from_str(&format!("invalid ink: {error}")))?;
        if trace.points.is_empty()
            || trace
                .points
                .iter()
                .any(|&(x, y)| !x.is_finite() || !y.is_finite())
        {
            return Err(JsValue::from_str("ink needs finite stroke points"));
        }
        if trace.brush.tool != InkTool::Highlighter {
            if let Some(shape) = recognize_shape(&trace.points) {
                let pen = Pen::new(
                    trace.brush.tool,
                    &trace.brush.color,
                    trace.brush.width_px * 25.4 / 96.0,
                );
                return self.run_word_session("word_draw_ink_shape", move |session| {
                    session.insert_ink_shape(page, shape, &pen)
                });
            }
        }
        self.append_ink(vec![trace])
    }

    /// Erases ink drawing `index` (as `inkDrawings` lists them).
    #[wasm_bindgen(js_name = removeInkDrawing)]
    pub fn remove_ink_drawing(&self, index: usize) -> Result<JsValue, JsValue> {
        let drawings = self.ink_drawing_list();
        if index >= drawings.len() {
            return Err(JsValue::from_str("no such ink drawing"));
        }
        self.set_ink(Self::keep_ink(&drawings, Some(index)))
    }

    /// Erases every ink drawing in the body.
    #[wasm_bindgen(js_name = clearInkDrawings)]
    pub fn clear_ink_drawings(&self) -> Result<JsValue, JsValue> {
        self.set_ink(Vec::new())
    }

    /// The document's ink as strokes on pages (0-based, page px), in the
    /// order the ink tools number them: `{index, page, points, color,
    /// width, bounds}`; bounds includes the native brush footprint.
    #[wasm_bindgen(js_name = inkStrokes)]
    pub fn ink_strokes_js(&self) -> Result<JsValue, JsValue> {
        use crate::word::ink_tools::WordInkTools;
        let strokes = self.read_word_session(|session| session.ink_strokes())?;
        let value: Vec<serde_json::Value> = strokes
            .iter()
            .enumerate()
            .map(|(index, stroke)| {
                Ok(serde_json::json!({
                    "index": index,
                    "page": stroke.page,
                    "points": stroke.trace.points,
                    "color": stroke.trace.brush.color,
                    "width": stroke.trace.brush.width_px,
                    "bounds": stroke.bounds(),
                    "brush": stroke.trace.brush,
                    "preview": crate::canvas_renderer::ink_preview::ink_trace_preview(&stroke.trace)?,
                }))
            })
            .collect::<Result<_, String>>()
            .map_err(|error| JsValue::from_str(&error))?;
        Ok(super::word_models::to_js(&value))
    }

    /// Read-only preview from the document ink painter, for replaceable UI.
    #[wasm_bindgen(js_name = previewInkTrace)]
    pub fn preview_ink_trace(&self, trace: JsValue) -> Result<JsValue, JsValue> {
        let trace: InkTrace = serde_wasm_bindgen::from_value(trace)
            .map_err(|error| JsValue::from_str(&format!("invalid ink: {error}")))?;
        let paths = crate::canvas_renderer::ink_preview::ink_trace_preview(&trace)
            .map_err(|error| JsValue::from_str(&error))?;
        Ok(to_js(&paths))
    }

    /// なげなわ選択: the strokes on `page` mostly inside `polygon`
    /// (`[[x, y], …]`, page px).
    #[wasm_bindgen(js_name = lassoInkStrokes)]
    pub fn lasso_ink_strokes(&self, page: usize, polygon: JsValue) -> Result<Vec<u32>, JsValue> {
        use crate::word::ink_tools::WordInkTools;
        let polygon: Vec<(f64, f64)> = serde_wasm_bindgen::from_value(polygon)
            .map_err(|error| JsValue::from_str(&format!("invalid lasso: {error}")))?;
        let caught = self.read_word_session(|session| session.lasso_strokes(page, &polygon))?;
        Ok(caught.into_iter().map(|index| index as u32).collect())
    }

    /// Erases along a page-local path, splitting strokes for a point eraser.
    #[wasm_bindgen(js_name = eraseInkAlong)]
    pub fn erase_ink_along(
        &self,
        page: usize,
        path: JsValue,
        radius: f64,
        point_eraser: bool,
    ) -> Result<JsValue, JsValue> {
        use crate::word::ink_tools::WordInkTools;
        let path: Vec<(f64, f64)> = serde_wasm_bindgen::from_value(path)
            .map_err(|error| JsValue::from_str(&format!("invalid eraser path: {error}")))?;
        if !radius.is_finite()
            || radius <= 0.0
            || radius > 256.0
            || path.iter().any(|&(x, y)| !x.is_finite() || !y.is_finite())
        {
            return Err(JsValue::from_str(
                "eraser needs finite points and a radius in (0, 256]",
            ));
        }
        // An empty snapshot command would consume undo even when the path misses.
        let touched = self.read_word_session(|session| {
            session
                .ink_strokes()
                .iter()
                .any(|stroke| stroke.page == page && stroke.touches_path(&path, radius))
        })?;
        if !touched {
            return Ok(to_js(&json!({ "result": false })));
        }
        self.run_word_session("word_erase_ink", move |session| {
            if point_eraser {
                session.erase_points_along(page, &path, radius)
            } else {
                session.erase_strokes_along(page, &path, radius)
            }
        })
    }

    /// The selected strokes moved and resized onto `[left, top, right,
    /// bottom]` (page px), one undo step.
    #[wasm_bindgen(js_name = transformInkStrokes)]
    pub fn transform_ink_strokes(
        &self,
        selection: Vec<u32>,
        to: Vec<f64>,
    ) -> Result<JsValue, JsValue> {
        use crate::word::ink_tools::WordInkTools;
        let [left, top, right, bottom] = to[..] else {
            return Err(JsValue::from_str("a box is four numbers"));
        };
        let selection: Vec<usize> = selection.into_iter().map(|index| index as usize).collect();
        self.run_word_session("word_transform_ink", move |session| {
            session.transform_strokes(&selection, (left, top, right, bottom))
        })
    }

    /// The selected strokes deleted, one undo step.
    #[wasm_bindgen(js_name = deleteInkStrokes)]
    pub fn delete_ink_strokes(&self, selection: Vec<u32>) -> Result<JsValue, JsValue> {
        use crate::word::ink_tools::WordInkTools;
        let selection: Vec<usize> = selection.into_iter().map(|index| index as usize).collect();
        self.run_word_session("word_delete_ink", move |session| {
            session.delete_strokes(&selection)
        })
    }

    /// The selected strokes given a pen's `color` (`#RRGGBB`) and
    /// `thickness_mm`, one undo step.
    #[wasm_bindgen(js_name = restyleInkStrokes)]
    pub fn restyle_ink_strokes(
        &self,
        selection: Vec<u32>,
        color: String,
        thickness_mm: f64,
    ) -> Result<JsValue, JsValue> {
        use crate::word::ink_tools::{Pen, WordInkTools};
        use jade_docx_reader::inkml::InkTool;
        let selection: Vec<usize> = selection.into_iter().map(|index| index as usize).collect();
        self.run_word_session("word_restyle_ink", move |session| {
            session.restyle_strokes(&selection, &Pen::new(InkTool::Pen, &color, thickness_mm))
        })
    }

    /// Apply a full brush, retaining the rectangular nib and transparency.
    #[wasm_bindgen(js_name = restyleInkStrokesWithBrush)]
    pub fn restyle_ink_strokes_with_brush(
        &self,
        selection: Vec<u32>,
        brush: JsValue,
    ) -> Result<JsValue, JsValue> {
        use crate::word::ink_tools::WordInkTools;
        let brush: InkBrush = serde_wasm_bindgen::from_value(brush)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let selection: Vec<usize> = selection.into_iter().map(|index| index as usize).collect();
        self.run_word_session("word_restyle_ink", move |session| {
            session.restyle_strokes_with_brush(&selection, &brush)
        })
    }

    /// インクを図形に変換: each selected stroke that draws a shape the
    /// recogniser knows becomes that shape. `result` is how many did.
    #[wasm_bindgen(js_name = convertInkStrokesToShapes)]
    pub fn convert_ink_strokes_to_shapes(&self, selection: Vec<u32>) -> Result<JsValue, JsValue> {
        use crate::word::ink_tools::WordInkTools;
        let selection: Vec<usize> = selection.into_iter().map(|index| index as usize).collect();
        self.run_word_session("word_ink_to_shape", move |session| {
            session.convert_strokes_to_shapes(&selection)
        })
    }

    /// インク エディター: what a pen stroke over the text (`points`, x and y
    /// in turn, as pointer events give them) means — `delete` the words a
    /// line or scribble strikes through, `select` the words a loop goes
    /// round, `insert` (a ^) puts the caret between the words at its tip —
    /// and the body range it applies to, widened to whole words. `null` when
    /// the stroke is none of them or misses the text.
    #[wasm_bindgen(js_name = inkGesture)]
    pub fn ink_gesture(&self, points: Vec<f64>) -> JsValue {
        use crate::word::ink_tools::{recognize_gesture, InkGesture};
        use crate::word::selection::word_at;
        let points: Vec<(f64, f64)> = points
            .chunks_exact(2)
            .map(|pair| (pair[0], pair[1]))
            .collect();
        let Some(gesture) = recognize_gesture(&points) else {
            return JsValue::NULL;
        };
        let top = points
            .iter()
            .map(|point| point.1)
            .fold(f64::INFINITY, f64::min);
        let hit = |x: f64, y: f64| self.renderer.borrow().hit_test_position(x, y);
        let state = self.state.borrow();
        let Some(document) = state.word_doc.as_ref() else {
            return JsValue::NULL;
        };
        let text = |paragraph: usize| -> String {
            document
                .paragraph(paragraph)
                .map(|paragraph| paragraph.runs.iter().map(|run| run.text.as_str()).collect())
                .unwrap_or_default()
        };
        // The boundary before `offset`'s character, for a range's end.
        let before = |text: &str, offset: usize| {
            text.get(..offset)
                .and_then(|head| head.char_indices().next_back())
                .map_or(0, |(at, _)| at)
        };
        let (kind, start, end) = match gesture {
            InkGesture::Delete { from, to } => {
                let (Some(a), Some(b)) = (hit(from.0, from.1), hit(to.0, to.1)) else {
                    return JsValue::NULL;
                };
                let (a, b) = if (a.paragraph, a.offset) <= (b.paragraph, b.offset) {
                    (a, b)
                } else {
                    (b, a)
                };
                let (a_text, b_text) = (text(a.paragraph), text(b.paragraph));
                let start = (a.paragraph, word_at(&a_text, a.offset).0);
                let end = (b.paragraph, word_at(&b_text, before(&b_text, b.offset)).1);
                ("delete", start, end)
            }
            InkGesture::Select((left, top, right, bottom)) => {
                // Inside the loop: it goes round the text, clear of it on every side.
                let inset = ((bottom - top) * 0.25).min(12.0);
                let (Some(a), Some(b)) = (
                    hit(left + inset, top + inset),
                    hit(right - inset, bottom - inset),
                ) else {
                    return JsValue::NULL;
                };
                let (a_text, b_text) = (text(a.paragraph), text(b.paragraph));
                let start = (a.paragraph, word_at(&a_text, a.offset).0);
                let mut end = word_at(&b_text, before(&b_text, b.offset)).1;
                end = b_text[..end]
                    .trim_end()
                    .len()
                    .max(if b.paragraph == a.paragraph {
                        start.1
                    } else {
                        0
                    });
                ("select", start, (b.paragraph, end))
            }
            InkGesture::Insert((x, _)) => {
                let Some(at) = hit(x, top) else {
                    return JsValue::NULL;
                };
                (
                    "insert",
                    (at.paragraph, at.offset),
                    (at.paragraph, at.offset),
                )
            }
        };
        if (start.0, start.1) > (end.0, end.1) || (kind == "delete" && start == end) {
            return JsValue::NULL;
        }
        let body = |surface| document.body_index_for_surface(surface);
        let (Some(start_paragraph), Some(end_paragraph)) = (body(start.0), body(end.0)) else {
            return JsValue::NULL;
        };
        to_js(&json!({
            "kind": kind,
            "start": { "paragraph": start_paragraph, "offset": start.1 },
            "end": { "paragraph": end_paragraph, "offset": end.1 },
        }))
    }
}
