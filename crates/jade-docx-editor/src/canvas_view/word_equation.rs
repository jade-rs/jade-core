//! Browser entry points for editing an equation through
//! [`crate::word::equation`]: the equation at the caret read out in its
//! linear format (UnicodeMath or LaTeX), and written back from it, built up
//! into 2 次元形式 as Word's equation editor builds what is typed.

use super::word_models::to_js;
use super::*;
use crate::canvas_renderer::MathZone;
use crate::document::{MathElement, MathElementKind};
use crate::word::equation::{EquationEditor, EquationTarget, LinearFormat};
use serde_json::json;

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

#[wasm_bindgen]
impl OffscreenDocumentView {
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
