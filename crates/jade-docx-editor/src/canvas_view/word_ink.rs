//! The browser entry points for Word's 描画 tab: ink kept as Word keeps it
//! (`w14:contentPart` with an InkML part), so Word opens the strokes as
//! ink it can recolour and erase, not as a picture.

use super::*;
use crate::commands::set_ink_drawings::SetInkDrawingsCommand;
use crate::word_document::ink_drawing::{InkDrawingInfo, InkDrawingSpec};
use jade_docx_reader::inkml::InkTrace;

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
        let (start, _) = self.current_selection().normalized();
        let mut specs = Self::keep_ink(&self.ink_drawing_list(), None);
        specs.push(InkDrawingSpec::New {
            paragraph_index: start.paragraph,
            traces,
        });
        self.set_ink(specs)
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
    /// width}`.
    #[wasm_bindgen(js_name = inkStrokes)]
    pub fn ink_strokes_js(&self) -> Result<JsValue, JsValue> {
        use crate::word::ink_tools::WordInkTools;
        let strokes = self.read_word_session(|session| session.ink_strokes())?;
        let value: Vec<serde_json::Value> = strokes
            .iter()
            .enumerate()
            .map(|(index, stroke)| {
                serde_json::json!({
                    "index": index,
                    "page": stroke.page,
                    "points": stroke.trace.points,
                    "color": stroke.trace.brush.color,
                    "width": stroke.trace.brush.width_px,
                })
            })
            .collect();
        Ok(super::word_models::to_js(&value))
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
}
