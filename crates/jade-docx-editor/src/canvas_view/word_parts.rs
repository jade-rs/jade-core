//! Browser entry points for クイック パーツ and 定型句 — the glossary's
//! building blocks — from [`crate::word::building_blocks`], run on the view
//! as a session.

use super::word_models::to_js;
use super::*;
use crate::word::building_blocks::{Gallery, WordBuildingBlocks};
use crate::word::WordSession;
use serde_json::json;

fn gallery(name: &str) -> Result<Gallery, JsValue> {
    match name {
        "quickParts" => Ok(Gallery::QuickParts),
        "autoText" => Ok(Gallery::AutoText),
        other => Err(JsValue::from_str(&format!("unknown gallery {other}"))),
    }
}

#[wasm_bindgen]
impl OffscreenDocumentView {
    /// The glossary's entries, in order: `index` names one for
    /// `insertQuickPart` and `deleteQuickPart`.
    #[wasm_bindgen(js_name = quickParts)]
    pub fn quick_parts(&self) -> Result<JsValue, JsValue> {
        let parts = self.read_word_session(|session| {
            session
                .building_blocks()
                .iter()
                .enumerate()
                .map(|(index, block)| {
                    json!({
                        "index": index,
                        "name": block.name,
                        "gallery": match block.gallery {
                            Gallery::QuickParts => "quickParts",
                            Gallery::AutoText => "autoText",
                        },
                        "category": block.category,
                    })
                })
                .collect::<Vec<_>>()
        })?;
        Ok(to_js(&parts))
    }

    /// 選択範囲をクイック パーツ ギャラリーに保存 (`quickParts`) / 定型句に
    /// 保存 (`autoText`).
    #[wasm_bindgen(js_name = saveQuickPart)]
    pub fn save_quick_part(&self, name: String, gallery_name: String) -> Result<JsValue, JsValue> {
        let gallery = gallery(&gallery_name)?;
        if name.trim().is_empty() {
            return Err(JsValue::from_str("a building block needs a name"));
        }
        self.run_word_session("word_save_building_block", move |session| {
            session.save_building_block(name.trim(), gallery)
        })
    }

    /// Entry `index` at the caret, over the selection, with its formatting.
    #[wasm_bindgen(js_name = insertQuickPart)]
    pub fn insert_quick_part(&self, index: usize) -> Result<JsValue, JsValue> {
        self.run_word_session("word_insert_building_block", move |session| {
            let (paragraph, offset) = session.insert_building_block(index)?;
            session.set_selection_range(Some(SelectionRange::collapsed(TextPosition::new(
                paragraph, offset,
            ))));
            Ok(())
        })
    }

    /// 文書パーツ オーガナイザー's 削除.
    #[wasm_bindgen(js_name = deleteQuickPart)]
    pub fn delete_quick_part(&self, index: usize) -> Result<JsValue, JsValue> {
        self.run_word_session("word_delete_building_block", move |session| {
            session.delete_building_block(index)
        })
    }
}
