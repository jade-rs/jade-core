//! Several ranges selected at once, as Word allows: ⌘/Ctrl-drag adds a
//! range to the ones already selected (複数範囲の選択), ⌥/Alt-drag selects a
//! rectangle of text (矩形選択). The shell keeps the ranges beside the
//! document's own selection; these entry points find them, draw them, copy
//! them, and let a command run on each of them as one undo step.

use super::word_models::{from_js, to_js};
use super::*;
use crate::word::selection::{block_ranges, range_rects, ranges_text};
use crate::word::Position;
use serde::{Deserialize, Serialize};

/// A place in the body, as script gives and takes it.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct BodyPlace {
    paragraph: usize,
    offset: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct BodyRange {
    start: BodyPlace,
    end: BodyPlace,
}

impl OffscreenDocumentView {
    /// `ranges` in the surface positions the layout keeps, ordered; the ones
    /// a paragraph of which is gone are left out.
    fn surface_ranges(&self, ranges: JsValue) -> Result<Vec<(Position, Position)>, JsValue> {
        let ranges: Vec<BodyRange> = from_js(ranges, "ranges")?;
        let state = self.state.borrow();
        let document = state
            .word_doc
            .as_ref()
            .ok_or_else(|| JsValue::from_str("No document loaded"))?;
        let place = |place: BodyPlace| {
            document
                .surface_index_for_body(place.paragraph)
                .map(|paragraph| (paragraph, place.offset))
        };
        Ok(ranges
            .into_iter()
            .filter_map(|range| {
                let (start, end) = (place(range.start)?, place(range.end)?);
                Some(if start <= end {
                    (start, end)
                } else {
                    (end, start)
                })
            })
            .collect())
    }
}

#[wasm_bindgen]
impl OffscreenDocumentView {
    /// 矩形選択: the ranges an ⌥-drag from (`from_x`, `from_y`) to (`to_x`,
    /// `to_y`) takes — on every line the rectangle crosses, the text between
    /// its left and right edges — as body ranges, top to bottom.
    #[wasm_bindgen(js_name = blockRanges)]
    pub fn block_ranges_between(&self, from_x: f64, from_y: f64, to_x: f64, to_y: f64) -> JsValue {
        let surface = block_ranges(
            self.renderer.borrow().selection_manager(),
            (from_x, from_y),
            (to_x, to_y),
        );
        let state = self.state.borrow();
        let Some(document) = state.word_doc.as_ref() else {
            return JsValue::NULL;
        };
        let body = |(paragraph, offset): Position| {
            Some(BodyPlace {
                paragraph: document.body_index_for_surface(paragraph)?,
                offset,
            })
        };
        let ranges: Vec<BodyRange> = surface
            .into_iter()
            .filter_map(|(start, end)| {
                Some(BodyRange {
                    start: body(start)?,
                    end: body(end)?,
                })
            })
            .collect();
        to_js(&ranges)
    }

    /// The highlight rectangles of body `ranges` (`[{start, end}]`), in the
    /// document's coordinates as the caret's `absolute_top` gives them.
    #[wasm_bindgen(js_name = rangeRects)]
    pub fn range_rects_of(&self, ranges: JsValue) -> Result<JsValue, JsValue> {
        let ranges = self.surface_ranges(ranges)?;
        let renderer = self.renderer.borrow();
        let rects: Vec<serde_json::Value> = ranges
            .into_iter()
            .flat_map(|(start, end)| range_rects(renderer.selection_manager(), start, end))
            .map(|(x, y, width, height)| {
                serde_json::json!({ "x": x, "y": y, "width": width, "height": height })
            })
            .collect();
        Ok(to_js(&rects))
    }

    /// The text of body `ranges`, as copying them gives it: in document
    /// order, a line each.
    #[wasm_bindgen(js_name = rangesText)]
    pub fn ranges_text_of(&self, ranges: JsValue) -> Result<String, JsValue> {
        let mut ranges = self.surface_ranges(ranges)?;
        ranges.sort();
        let state = self.state.borrow();
        let document = state
            .word_doc
            .as_ref()
            .ok_or_else(|| JsValue::from_str("No document loaded"))?;
        Ok(ranges_text(
            |index| {
                document
                    .paragraph(index)
                    .map(|paragraph| paragraph.runs.iter().map(|run| run.text.as_str()).collect())
            },
            &ranges,
        ))
    }

    /// Opens an undo step that the edits until [`Self::end_edit_group`] all
    /// go into — a format put on each of several ranges undoes at once.
    #[wasm_bindgen(js_name = beginEditGroup)]
    pub fn begin_edit_group(&self) {
        self.state.borrow_mut().command_executor.begin_group();
    }

    /// Closes the undo step [`Self::begin_edit_group`] opened.
    #[wasm_bindgen(js_name = endEditGroup)]
    pub fn end_edit_group(&self) {
        self.state.borrow_mut().command_executor.end_group();
    }
}
