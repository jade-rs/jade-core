//! Browser entry points for editing an equation through
//! [`crate::word::equation`]: the equation at the caret read out in its
//! linear format (UnicodeMath or LaTeX), and written back from it, built up
//! into 2 次元形式 as Word's equation editor builds what is typed.

use super::word_models::to_js;
use super::*;
use crate::canvas_renderer::MathZone;
use crate::document::{MathElement, MathElementKind};
use crate::word::equation::{
    EquationEditor, EquationInsert, EquationTarget, LinearFormat, StructureGroup,
};
use serde_json::json;

fn from_js_value<T: serde::de::DeserializeOwned>(value: JsValue) -> Result<T, JsValue> {
    serde_wasm_bindgen::from_value(value)
        .map_err(|error| JsValue::from_str(&format!("invalid equation key: {error}")))
}

fn linear_format(name: &str) -> Result<LinearFormat, JsValue> {
    match name {
        "" | "unicode" | "unicodeMath" => Ok(LinearFormat::UnicodeMath),
        "latex" => Ok(LinearFormat::LaTeX),
        other => Err(JsValue::from_str(&format!("unknown linear format {other}"))),
    }
}

impl OffscreenDocumentView {
    /// The equation the caret is in or beside: a display equation's
    /// paragraph, or the inline equation nearest the caret in its paragraph.
    fn equation_zone_at_selection(&self) -> Option<MathZone> {
        let caret = self.current_selection().focus;
        let renderer = self.renderer.borrow();
        renderer
            .math_zones()
            .iter()
            .filter(|zone| zone.paragraph == caret.paragraph)
            .filter(|zone| zone.display || zone.offset.abs_diff(caret.offset) <= 1)
            .min_by_key(|zone| zone.offset.abs_diff(caret.offset))
            .cloned()
    }
}

/// One key or ribbon command in an equation being edited in place.
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum EquationKeyJs {
    Text {
        text: String,
    },
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    /// Tab: the next argument.
    Next,
    /// A structure from the 数式 tab: its group (`fraction`, `script`…) and
    /// item.
    #[serde(rename_all = "camelCase")]
    Structure {
        group: String,
        index: usize,
    },
}

fn structure_group(name: &str) -> Option<StructureGroup> {
    StructureGroup::ALL
        .into_iter()
        .find(|group| group.key().rsplit('.').next() == Some(name))
}

impl OffscreenDocumentView {
    /// Where the equation editor's caret is drawn, in the payload's `caret`
    /// shape: page-local `top`, and `absolute_top` down the run of pages.
    /// The height of the pages above 1-based `page`, with the gaps between
    /// them: where the page starts down the run of pages.
    fn pages_above(&self, page: usize) -> f64 {
        let renderer = self.renderer.borrow();
        (0..page.saturating_sub(1))
            .filter_map(|index| renderer.page_dimensions(index).map(|(_, height)| height))
            .map(|height| height + 16.0)
            .sum()
    }

    fn equation_caret_js(&self, editor: &EquationEditor) -> JsValue {
        let pages_above_caret = |page| self.pages_above(page);
        let renderer = self.renderer.borrow();
        let Some(zone) = editor.zone(renderer.math_zones()) else {
            return JsValue::NULL;
        };
        let Some(caret) = editor.caret_geometry(zone) else {
            return JsValue::NULL;
        };
        drop(renderer);
        let pages_above = pages_above_caret(caret.page);
        to_js(&json!({
            "x": caret.x,
            "top": caret.top,
            "absolute_top": pages_above + caret.top,
            "width": 1.5,
            "height": caret.height,
            "page": caret.page,
        }))
    }
}

#[wasm_bindgen]
impl OffscreenDocumentView {
    /// Starts editing the equation at the caret in place, its caret at the
    /// end: `{caret}` to draw, or `null` when the caret is at no equation.
    /// With a click point (`x` page-local, `y` down the run of pages), only
    /// a click inside the equation's frame starts it: a click beside an
    /// inline equation types beside it.
    #[wasm_bindgen(js_name = beginEquationEdit)]
    pub fn begin_equation_edit(&self, x: Option<f64>, y: Option<f64>) -> Result<JsValue, JsValue> {
        let collapsed = self.current_selection().is_collapsed();
        let zone = collapsed
            .then(|| self.equation_zone_at_selection())
            .flatten();
        let Some(zone) = zone else {
            self.state.borrow_mut().equation_editor = None;
            return Ok(JsValue::NULL);
        };
        let editor = self.read_word_session(|session| EquationEditor::open(session, &zone))?;
        let Some(editor) = editor else {
            return Ok(JsValue::NULL);
        };
        if let (Some(x), Some(y)) = (x, y) {
            let (frame, _) = editor.frames(&zone);
            let top = self.pages_above(frame.page) + frame.y;
            let inside = x >= frame.x - 2.0
                && x <= frame.x + frame.width + 2.0
                && y >= top - 2.0
                && y <= top + frame.height + 2.0;
            if !inside {
                self.state.borrow_mut().equation_editor = None;
                return Ok(JsValue::NULL);
            }
        }
        let caret = self.equation_caret_js(&editor);
        self.state.borrow_mut().equation_editor = Some(editor);
        let result = js_sys::Object::new();
        let _ = Reflect::set(&result, &JsValue::from_str("caret"), &caret);
        Ok(result.into())
    }

    /// Whether an equation is being edited in place.
    #[wasm_bindgen(js_name = equationEditing)]
    pub fn equation_editing(&self) -> bool {
        self.state.borrow().equation_editor.is_some()
    }

    /// Leaves the equation: typing goes to the text again.
    #[wasm_bindgen(js_name = endEquationEdit)]
    pub fn end_equation_edit(&self) {
        self.state.borrow_mut().equation_editor = None;
    }

    /// One key or 数式 tab command in the equation being edited: typed text
    /// builds up as Word's does, Backspace/Delete, the arrows move through
    /// the structure, Tab to the next argument, a structure inserted at the
    /// caret. An edit is one undo step; the payload's `caret` is the
    /// equation's. Moving out past either end leaves the equation (`null`
    /// `equationCaret`).
    #[wasm_bindgen(js_name = equationEditKey)]
    pub fn equation_edit_key(&self, key: JsValue) -> Result<JsValue, JsValue> {
        let key: EquationKeyJs = from_js_value(key)?;
        let Some(mut editor) = self.state.borrow_mut().equation_editor.take() else {
            return Ok(JsValue::NULL);
        };
        let before = editor.omml();
        let before_caret = editor.caret.clone();
        let zone = editor.zone(self.renderer.borrow().math_zones()).cloned();
        match key {
            EquationKeyJs::Text { text } => editor.type_text(&text),
            EquationKeyJs::Backspace => editor.backspace(),
            EquationKeyJs::Delete => editor.delete_forward(),
            EquationKeyJs::Left => editor.move_left(),
            EquationKeyJs::Right => editor.move_right(),
            EquationKeyJs::Up | EquationKeyJs::Down => {
                if let Some(zone) = zone.as_ref() {
                    editor.move_vertically(zone, matches!(key, EquationKeyJs::Down));
                }
            }
            EquationKeyJs::Home => editor.move_home(),
            EquationKeyJs::End => editor.move_end(),
            EquationKeyJs::Next => editor.next_slot(),
            EquationKeyJs::Structure { group, index } => {
                let group = structure_group(&group).ok_or_else(|| {
                    JsValue::from_str(&format!("unknown structure group {group}"))
                })?;
                editor.insert(&EquationInsert::Structure(group, index));
            }
        }
        let changed = editor.omml() != before;
        let payload = if changed {
            let commit = editor.clone();
            self.run_word_session("word_equation_edit", move |session| commit.commit(session))?
        } else {
            let result = js_sys::Object::new();
            if editor.caret == before_caret {
                // A move that went nowhere at an end leaves the equation.
                let _ = Reflect::set(&result, &JsValue::from_str("equationCaret"), &JsValue::NULL);
                let _ = Reflect::set(&result, &JsValue::from_str("leftEquation"), &JsValue::TRUE);
                return Ok(result.into());
            }
            result.into()
        };
        let caret = self.equation_caret_js(&editor);
        if payload.is_object() {
            let _ = Reflect::set(&payload, &JsValue::from_str("equationCaret"), &caret);
        }
        self.state.borrow_mut().equation_editor = Some(editor);
        Ok(payload)
    }

    /// The equation at the caret as `{linear, display}` in `format`
    /// (`unicode` or `latex`), or `null` when the caret is at none.
    #[wasm_bindgen(js_name = equationAtSelection)]
    pub fn equation_at_selection(&self, format: String) -> Result<JsValue, JsValue> {
        let format = linear_format(&format)?;
        let Some(zone) = self.equation_zone_at_selection() else {
            return Ok(JsValue::NULL);
        };
        let found = self.read_word_session(|session| {
            EquationEditor::open(session, &zone).map(|mut editor| {
                editor.format = format;
                json!({ "linear": editor.linear_text(), "display": zone.display })
            })
        })?;
        Ok(found.map_or(JsValue::NULL, |value| to_js(&value)))
    }

    /// The equation at the caret rewritten from `linear` in `format`, built up
    /// into 2 次元形式 (Math AutoCorrect applied), as one undo step.
    #[wasm_bindgen(js_name = replaceEquationAtSelection)]
    pub fn replace_equation_at_selection(
        &self,
        linear: String,
        format: String,
    ) -> Result<JsValue, JsValue> {
        let format = linear_format(&format)?;
        if linear.trim().is_empty() {
            return Err(JsValue::from_str("数式を入力してください"));
        }
        let zone = self
            .equation_zone_at_selection()
            .ok_or_else(|| JsValue::from_str("数式が選択されていません"))?;
        self.run_word_session("word_replace_equation", move |session| {
            let mut editor = EquationEditor::new(
                EquationTarget {
                    paragraph: zone.paragraph,
                    run_index: zone.run_index,
                    display: zone.display,
                },
                MathElement::new(MathElementKind::Math, Vec::new()),
            );
            editor.format = format;
            editor.type_text(&linear);
            editor.to_professional_form();
            editor.commit(session)
        })
    }
}
