//! Browser entry points for the Word models moved from jade-office into
//! [`crate::word`], run on the view as a [`super::word_session`] session.

use super::*;
use crate::word::styles::{NewStyle, StyleChange, StyleFormat, StyleInfo, WordStyles};

/// スタイルの変更: what changes; a field left out stays. `basedOn: null`
/// takes the base off (スタイルなし).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StyleChangeJs {
    #[serde(default)]
    name: Option<String>,
    #[serde(default, deserialize_with = "present")]
    based_on: Option<Option<String>>,
    #[serde(default)]
    next: Option<String>,
    #[serde(default)]
    format: StyleFormat,
    #[serde(default)]
    quick: Option<bool>,
    #[serde(default)]
    auto_update: Option<bool>,
    #[serde(default)]
    priority: Option<i32>,
}

/// A field that is present — even as `null` — is `Some`.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    serde::Deserialize::deserialize(deserializer).map(Some)
}

pub(super) fn from_js<T: serde::de::DeserializeOwned>(
    value: JsValue,
    what: &str,
) -> Result<T, JsValue> {
    serde_wasm_bindgen::from_value(value)
        .map_err(|error| JsValue::from_str(&format!("invalid {what}: {error}")))
}

pub(super) fn to_js<T: serde::Serialize>(value: &T) -> JsValue {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .unwrap_or(JsValue::NULL)
}

#[wasm_bindgen]
impl OffscreenDocumentView {
    /// Every style of the document, in its own order, with its format: the
    /// スタイル pane.
    #[wasm_bindgen(js_name = styleCatalog)]
    pub fn style_catalog(&self) -> Result<JsValue, JsValue> {
        let styles: Vec<StyleInfo> = self.read_word_session(|session| session.style_catalog())?;
        Ok(to_js(&styles))
    }

    /// 新しいスタイル: returns the new style's id in `result`.
    #[wasm_bindgen(js_name = createStyle)]
    pub fn create_style(&self, style: JsValue) -> Result<JsValue, JsValue> {
        let style: NewStyle = from_js(style, "style")?;
        if style.name.trim().is_empty() {
            return Err(JsValue::from_str("a style needs a name"));
        }
        self.run_word_session("word_create_style", move |session| {
            session.create_style(&style)
        })
    }

    /// 新しい表スタイル: added and put on the caret's table; `result` is its id.
    #[wasm_bindgen(js_name = createTableStyle)]
    pub fn create_table_style(&self, spec: JsValue) -> Result<JsValue, JsValue> {
        let spec: crate::word::styles::TableStyleSpec = from_js(spec, "table style")?;
        self.run_word_session("word_create_table_style", move |session| {
            session.create_table_style(&spec)
        })
    }

    /// 表スタイルの変更 for table style `id`.
    #[wasm_bindgen(js_name = modifyTableStyle)]
    pub fn modify_table_style(&self, id: String, spec: JsValue) -> Result<JsValue, JsValue> {
        let spec: crate::word::styles::TableStyleSpec = from_js(spec, "table style")?;
        self.run_word_session("word_modify_table_style", move |session| {
            session.modify_table_style(&id, &spec)
        })
    }

    /// What table style `id` draws, as 表スタイルの変更 opens on it; `null`
    /// when it is not a table style of the document.
    #[wasm_bindgen(js_name = tableStyleSpec)]
    pub fn table_style_spec(&self, id: String) -> Result<JsValue, JsValue> {
        let spec = self.read_word_session(|session| session.table_style_spec(&id))?;
        Ok(spec.map_or(JsValue::NULL, |spec| to_js(&spec)))
    }

    /// スタイルの変更 for style `id`.
    #[wasm_bindgen(js_name = modifyStyle)]
    pub fn modify_style(&self, id: String, change: JsValue) -> Result<JsValue, JsValue> {
        let change: StyleChangeJs = from_js(change, "style change")?;
        let change = StyleChange {
            name: change.name,
            based_on: change.based_on,
            next: change.next,
            format: change.format,
            quick: change.quick,
            auto_update: change.auto_update,
            priority: change.priority,
        };
        self.run_word_session("word_modify_style", move |session| {
            session.modify_style(&id, &change)
        })
    }

    /// 選択個所と一致するように更新: style `id` takes the selection's format.
    #[wasm_bindgen(js_name = updateStyleToMatch)]
    pub fn update_style_to_match(&self, id: String) -> Result<JsValue, JsValue> {
        self.run_word_session("word_update_style_to_match", move |session| {
            session.update_style_to_match(&id)
        })
    }
}
