//! Smaller browser entry points: 表スタイルのオプション in full and 前の
//! コメント / 次のコメント.

use super::word_models::{from_js, to_js};
use super::*;
use crate::word::references::navigation::WordNavigation;
use crate::word::WordSession;
use serde_json::json;

/// 表スタイルのオプション; a field left out stays.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
struct TableLookJs {
    first_row: Option<bool>,
    last_row: Option<bool>,
    banded_rows: Option<bool>,
    first_column: Option<bool>,
    last_column: Option<bool>,
    banded_columns: Option<bool>,
}

#[wasm_bindgen]
impl OffscreenDocumentView {
    /// 表スタイルのオプション for the caret's table: タイトル行, 集計行,
    /// 縞模様 (行), 最初の列, 最後の列, 縞模様 (列).
    #[wasm_bindgen(js_name = setTableLookOptions)]
    pub fn set_table_look_options(&self, look: JsValue) -> Result<JsValue, JsValue> {
        let look: TableLookJs = from_js(look, "table look")?;
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        self.apply_paragraph_style_headless(move |_, _| HeadlessCommand::SetTableLook {
            table_order_index: cell.table_order_index,
            first_row: look.first_row,
            banded_rows: look.banded_rows,
            last_row: look.last_row,
            first_column: look.first_column,
            last_column: look.last_column,
            banded_columns: look.banded_columns,
        })
    }

    /// 前のコメント / 次のコメント from the caret: the comment's id and its
    /// anchor as story places, or `null` when there is none that way.
    #[wasm_bindgen(js_name = stepComment)]
    pub fn step_comment(&self, forwards: bool) -> Result<JsValue, JsValue> {
        let caret = self.current_selection().focus;
        let found = self.read_word_session(|session| {
            let document = session.word_document();
            session
                .step_comment((caret.paragraph, caret.offset), forwards)
                .and_then(|comment| {
                    let (start, end) = comment.anchor?;
                    let place = |(paragraph, offset): (usize, usize)| {
                        document.story_position_for_surface(paragraph).map(|place| {
                            json!({
                                "story": place.story,
                                "paragraph": place.index,
                                "offset": offset,
                                "anchorParagraph": place.anchor_paragraph,
                                "noteId": place.note_id,
                            })
                        })
                    };
                    Some(json!({
                        "id": comment.id,
                        "author": comment.author,
                        "text": comment.text,
                        "start": place(start)?,
                        "end": place(end)?,
                    }))
                })
        })?;
        Ok(found.map_or(JsValue::NULL, |value| to_js(&value)))
    }
}
