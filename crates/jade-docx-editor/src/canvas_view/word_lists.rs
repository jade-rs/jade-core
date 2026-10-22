//! Browser entry points for 箇条書きと段落番号 beyond the galleries, from
//! [`crate::word::lists`]: the caret's list level in full (番号に続く空白の
//! 扱い, 正規形式の番号, レベルと対応付ける見出しスタイル), 番号の設定 (start
//! again, continue), 図の行頭文字, and list styles.

use super::word_models::{from_js, to_js};
use super::*;
use crate::word::lists::{list_styles, ListLevel, Suffix, WordLists};
use crate::word::WordSession;
use serde_json::{json, Value};

fn suffix_name(suffix: Suffix) -> &'static str {
    match suffix {
        Suffix::Tab => "tab",
        Suffix::Space => "space",
        Suffix::Nothing => "nothing",
    }
}

fn level_js(level: &ListLevel) -> Value {
    json!({
        "format": level.format,
        "text": level.text,
        "start": level.start,
        "alignment": level.alignment,
        "indent": level.indent,
        "hanging": level.hanging,
        "suffix": suffix_name(level.suffix),
        "legal": level.legal,
        "style": level.style,
        "font": level.font,
    })
}

/// The caret's level, each field optional.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
struct LevelPatchJs {
    format: Option<String>,
    text: Option<String>,
    start: Option<u32>,
    alignment: Option<String>,
    indent: Option<f32>,
    hanging: Option<f32>,
    /// `tab`, `space` or `nothing`.
    suffix: Option<String>,
    legal: Option<bool>,
    /// A paragraph style id, or `""` to unlink.
    style: Option<String>,
}

#[wasm_bindgen]
impl OffscreenDocumentView {
    /// The caret's list: `{numId, level, start, levels}`, every level in
    /// full, or `null` outside a list.
    #[wasm_bindgen(js_name = listAtCaret)]
    pub fn list_at_caret(&self) -> Result<JsValue, JsValue> {
        let state = self.read_word_session(|session| session.list_state())?;
        Ok(state.map_or(JsValue::NULL, |state| {
            to_js(&json!({
                "numId": state.num_id,
                "level": state.level,
                "start": state.start,
                "levels": state.definition.levels.iter().map(level_js).collect::<Vec<_>>(),
            }))
        }))
    }

    /// 新しいアウトラインの定義's もっと: the caret's level of its list
    /// rewritten — format, text, start, alignment, positions, what follows
    /// the number, legal numbering, the linked heading style.
    #[wasm_bindgen(js_name = editListLevel)]
    pub fn edit_list_level(&self, patch: JsValue) -> Result<JsValue, JsValue> {
        let patch: LevelPatchJs = from_js(patch, "list level")?;
        let suffix = match patch.suffix.as_deref() {
            None => None,
            Some("tab") => Some(Suffix::Tab),
            Some("space") => Some(Suffix::Space),
            Some("nothing") => Some(Suffix::Nothing),
            Some(other) => return Err(JsValue::from_str(&format!("unknown suffix {other}"))),
        };
        self.run_word_session("word_edit_list_level", move |session| {
            session.edit_list_level(move |level| {
                if let Some(format) = patch.format {
                    level.format = format;
                }
                if let Some(text) = patch.text {
                    level.text = text;
                }
                if let Some(start) = patch.start {
                    level.start = start;
                }
                if let Some(alignment) = patch.alignment {
                    level.alignment = alignment;
                }
                if let Some(indent) = patch.indent {
                    level.indent = indent;
                }
                if let Some(hanging) = patch.hanging {
                    level.hanging = hanging;
                }
                if let Some(suffix) = suffix {
                    level.suffix = suffix;
                }
                if let Some(legal) = patch.legal {
                    level.legal = legal;
                }
                if let Some(style) = patch.style {
                    level.style = (!style.is_empty()).then_some(style);
                }
            })
        })
    }

    /// 番号の設定 ▸ 新しくリストを開始する: from the caret's paragraph the
    /// list starts again at `value`.
    #[wasm_bindgen(js_name = setListStart)]
    pub fn set_list_start(&self, value: u32) -> Result<JsValue, JsValue> {
        self.run_word_session("word_set_list_start", move |session| {
            session.set_list_start(value.max(1))
        })
    }

    /// 番号の設定 ▸ 前のリストに続ける.
    #[wasm_bindgen(js_name = continueList)]
    pub fn continue_list(&self) -> Result<JsValue, JsValue> {
        self.run_word_session("word_continue_list", |session| session.continue_list())
    }

    /// 図の行頭文字: the selection becomes a list bulleted with the picture
    /// `image` (PNG, JPEG or GIF).
    #[wasm_bindgen(js_name = applyPictureBullet)]
    pub fn apply_picture_bullet(
        &self,
        image: Vec<u8>,
        extension: String,
    ) -> Result<JsValue, JsValue> {
        self.run_word_session("word_picture_bullet", move |session| {
            session.apply_picture_bullet(&image, &extension)
        })
    }

    /// The document's list styles: `{id, name, numId}`.
    #[wasm_bindgen(js_name = listStyles)]
    pub fn list_styles_js(&self) -> Result<JsValue, JsValue> {
        let styles = self.read_word_session(|session| {
            list_styles(
                &session
                    .word_document()
                    .related_part_xml("styles")
                    .unwrap_or_default(),
            )
        })?;
        Ok(to_js(
            &styles
                .into_iter()
                .map(|(id, name, num_id)| json!({ "id": id, "name": name, "numId": num_id }))
                .collect::<Vec<_>>(),
        ))
    }

    /// 新しいリスト スタイルの定義: a list style named `name`, defined as the
    /// caret's list (or a decimal outline when the caret is in none), applied
    /// to the selection. `result` is the style's id.
    #[wasm_bindgen(js_name = createListStyle)]
    pub fn create_list_style(&self, name: String) -> Result<JsValue, JsValue> {
        self.run_word_session("word_create_list_style", move |session| {
            let definition = session
                .list_state()
                .map(|state| state.definition)
                .unwrap_or_else(|| {
                    crate::word::lists::MultilevelPreset::ALL[0].definition(&|_| None)
                });
            session.create_list_style(&name, &definition)
        })
    }

    /// 区切り ▸ ページ区切り: `page`, `column` (段区切り) or `textWrapping`
    /// (文字列の折り返し) at the caret, as one undo step.
    #[wasm_bindgen(js_name = insertBreak)]
    pub fn insert_break(&self, kind: String) -> Result<JsValue, JsValue> {
        use crate::word::page_setup::{PageBreak, WordPageSetup};
        let kind = match kind.as_str() {
            "page" => PageBreak::Page,
            "column" => PageBreak::Column,
            "textWrapping" => PageBreak::TextWrapping,
            other => return Err(JsValue::from_str(&format!("unknown break {other}"))),
        };
        self.run_word_session("word_insert_break", move |session| {
            session.insert_page_break(kind)
        })
    }

    /// A list style applied to the selection: it joins that style's list.
    #[wasm_bindgen(js_name = applyListStyle)]
    pub fn apply_list_style(&self, num_id: String) -> Result<JsValue, JsValue> {
        self.run_word_session("word_apply_list_style", move |session| {
            session.apply_list_style(&num_id)
        })
    }
}
