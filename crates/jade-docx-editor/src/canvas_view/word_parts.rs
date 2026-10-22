//! Browser entry points for クイック パーツ and 定型句 — the glossary's
//! building blocks — from [`crate::word::building_blocks`], and 音声読み上げ's
//! paragraphs from [`crate::word::read_aloud`], run on the view as a session.

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

    /// 読み上げ from the caret: each paragraph with something to say, as
    /// `{paragraph, offset, story, anchorParagraph, noteId, text}` — the
    /// place its `text` starts, as `setSelectionRange` takes it. The voice
    /// is the browser's; its word boundaries are UTF-16 ranges of `text`.
    #[wasm_bindgen(js_name = readAloudUtterances)]
    pub fn read_aloud_utterances(&self) -> Result<JsValue, JsValue> {
        let caret = self.current_selection().focus;
        let utterances = self.read_word_session(|session| {
            let document = session.word_document();
            crate::word::read_aloud::utterances(session, (caret.paragraph, caret.offset))
                .into_iter()
                .filter_map(|utterance| {
                    let place = document.story_position_for_surface(utterance.paragraph)?;
                    Some(json!({
                        "story": place.story,
                        "paragraph": place.index,
                        "offset": utterance.offset,
                        "anchorParagraph": place.anchor_paragraph,
                        "noteId": place.note_id,
                        "text": utterance.text,
                    }))
                })
                .collect::<Vec<_>>()
        })?;
        Ok(to_js(&utterances))
    }

    /// 文書パーツ オーガナイザー's 削除.
    #[wasm_bindgen(js_name = deleteQuickPart)]
    pub fn delete_quick_part(&self, index: usize) -> Result<JsValue, JsValue> {
        self.run_word_session("word_delete_building_block", move |session| {
            session.delete_building_block(index)
        })
    }
}
