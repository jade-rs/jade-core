//! Word's オートコレクト while typing, from [`crate::word::autocorrect`]: after
//! a word ends, the paragraph's corrections — the replacement list,
//! capitals, quotes, ordinals, fractions, dashes, 長音, the spaces between
//! Japanese and Latin text — as an undo step of their own, as Word makes
//! them. Off until a host turns it on; when on, a keystroke that ends no
//! word costs one check of its last character.

use super::word_models::{from_js, to_js};
use super::*;
use crate::word::autocorrect::{
    corrections, flag_mut, is_trigger, AutoCorrectOptions, AUTOCORRECT_KEYS, AUTOFORMAT_KEYS,
};
use serde_json::{json, Map, Value};

/// The settings as script passes them: flags by key and, when given, the
/// whole replacement list.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AutoCorrectOptionsJs {
    #[serde(default)]
    flags: std::collections::HashMap<String, bool>,
    #[serde(default)]
    entries: Option<Vec<(String, String)>>,
}

fn options_js(options: &AutoCorrectOptions) -> Value {
    let mut flags = Map::new();
    let mut copy = options.clone();
    for key in AUTOCORRECT_KEYS.iter().chain(AUTOFORMAT_KEYS.iter()) {
        if let Some(flag) = flag_mut(&mut copy, key) {
            flags.insert((*key).to_string(), Value::Bool(*flag));
        }
    }
    json!({
        "flags": flags,
        "entries": options.entries,
        "autocorrectKeys": AUTOCORRECT_KEYS,
        "autoformatKeys": AUTOFORMAT_KEYS,
    })
}

#[wasm_bindgen]
impl OffscreenDocumentView {
    /// オートコレクトのオプション: the settings and whether it is on.
    #[wasm_bindgen(js_name = autoCorrectOptions)]
    pub fn auto_correct_options(&self) -> JsValue {
        let state = self.state.borrow();
        let enabled = state.word_autocorrect.is_some();
        let options = state.word_autocorrect.clone().unwrap_or_default();
        let mut value = options_js(&options);
        value["enabled"] = Value::Bool(enabled);
        to_js(&value)
    }

    /// Turns オートコレクト on with `options` (`{flags, entries}`, the rest
    /// Word's defaults), or off with `null`.
    #[wasm_bindgen(js_name = setAutoCorrectOptions)]
    pub fn set_auto_correct_options(&self, options: JsValue) -> Result<(), JsValue> {
        let next = if options.is_null() || options.is_undefined() {
            None
        } else {
            let js: AutoCorrectOptionsJs = from_js(options, "autocorrect options")?;
            let mut options = AutoCorrectOptions::default();
            for (key, value) in js.flags {
                *flag_mut(&mut options, &key).ok_or_else(|| {
                    JsValue::from_str(&format!("unknown autocorrect option {key}"))
                })? = value;
            }
            if let Some(entries) = js.entries {
                options.entries = entries;
            }
            Some(options)
        };
        self.state.borrow_mut().word_autocorrect = next;
        Ok(())
    }

    /// After typing: when オートコレクト is on and the character before the
    /// caret ends a word, the paragraph's corrections are made as one undo
    /// step and the caret follows them. `null` when there is nothing to do.
    #[wasm_bindgen(js_name = autoCorrectAtCaret)]
    pub fn auto_correct_at_caret(&self) -> Result<JsValue, JsValue> {
        let (paragraph, caret, fixes) = {
            let state = self.state.borrow();
            let Some(options) = state.word_autocorrect.as_ref() else {
                return Ok(JsValue::NULL);
            };
            let Some(selection) = state.selection.filter(|range| range.is_collapsed()) else {
                return Ok(JsValue::NULL);
            };
            let Some(document) = state.word_doc.as_ref() else {
                return Ok(JsValue::NULL);
            };
            let position = selection.focus;
            let Some(text) = document.paragraph(position.paragraph).map(|paragraph| {
                paragraph
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>()
            }) else {
                return Ok(JsValue::NULL);
            };
            let caret = position.offset.min(text.len());
            if !text.is_char_boundary(caret)
                || !text[..caret].chars().next_back().is_some_and(is_trigger)
            {
                return Ok(JsValue::NULL);
            }
            let fixes = corrections(&text, caret, options);
            if fixes.is_empty() {
                return Ok(JsValue::NULL);
            }
            (position.paragraph, caret, fixes)
        };
        let mut commands = Vec::new();
        let mut sorted = fixes.clone();
        sorted.sort_by(|a, b| b.start.cmp(&a.start));
        for fix in &sorted {
            commands.push(HeadlessCommand::ReplaceRange {
                start_paragraph: paragraph,
                start_offset: fix.start,
                end_paragraph: paragraph,
                end_offset: fix.end,
                text: fix.text.clone(),
            });
            if let Some((from, to)) = fix.superscript {
                commands.push(HeadlessCommand::ApplySuperscript {
                    start_paragraph: paragraph,
                    start_offset: fix.start + from,
                    end_paragraph: paragraph,
                    end_offset: fix.start + to,
                    enabled: true,
                });
            }
        }
        let after = crate::word::autocorrect::position_after((paragraph, caret), paragraph, &fixes);
        self.apply_paragraph_style_headless_many_with(
            move |_, _| commands,
            HeadlessApplyOverrides {
                dirty_from: Some(paragraph),
                selection_after: Some(Box::new(move |_| {
                    Some(SelectionRange::collapsed(TextPosition::new(
                        after.0, after.1,
                    )))
                })),
            },
        )
    }
}
