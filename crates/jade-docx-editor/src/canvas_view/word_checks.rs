//! Browser entry points for 文書の検査 ([`crate::word::inspect`]), 校正言語
//! ([`crate::word::language`]) and 類義語辞典 ([`crate::word::thesaurus`]),
//! run on the view as a session.

use super::word_models::to_js;
use super::*;
use crate::word::inspect::{InspectCategory, WordInspect};
use crate::word::language::{LanguageChoice, WordLanguage};
use crate::word::thesaurus::{BuiltinThesaurus, Thesaurus, WordThesaurus};
use serde_json::json;

#[wasm_bindgen]
impl OffscreenDocumentView {
    /// 文書の検査: what the document carries — comments, revisions, personal
    /// information, custom XML, hidden text, header and footer lines.
    #[wasm_bindgen(js_name = inspectDocument)]
    pub fn inspect_document(&self) -> Result<JsValue, JsValue> {
        let report = self.read_word_session(|session| session.inspect())?;
        Ok(to_js(&json!({
            "comments": report.comments,
            "revisions": report.revisions,
            "personal": report.personal,
            "customXmlParts": report.custom_xml_parts,
            "hiddenRuns": report.hidden_runs,
            "headerFooterLines": report.header_footer_lines,
        })))
    }

    /// 文書の検査's すべて削除 for one category held in the body:
    /// `commentsAndRevisions`, `hiddenText` or `headersFooters`. (Personal
    /// information and custom XML are package parts: `cleanPackage`.)
    #[wasm_bindgen(js_name = removeInspected)]
    pub fn remove_inspected(&self, category: String) -> Result<JsValue, JsValue> {
        let category = match category.as_str() {
            "commentsAndRevisions" => InspectCategory::CommentsAndRevisions,
            "hiddenText" => InspectCategory::HiddenText,
            "headersFooters" => InspectCategory::HeadersFooters,
            other => return Err(JsValue::from_str(&format!("unknown inspection {other}"))),
        };
        self.run_word_session("word_remove_inspected", move |session| {
            session.remove_inspected(category)
        })
    }

    /// 言語の設定: the selection marked as `language` (a BCP 47 tag), with
    /// 文章校正を行わない and 言語を自動的に検出する, as one undo step.
    #[wasm_bindgen(js_name = setProofingLanguage)]
    pub fn set_proofing_language(
        &self,
        language: String,
        no_proof: bool,
        detect: bool,
    ) -> Result<JsValue, JsValue> {
        let selection = self.current_selection();
        let (start, end) = selection.normalized();
        self.run_word_session("word_set_language", move |session| {
            session.set_language(
                (start.paragraph, start.offset),
                (end.paragraph, end.offset),
                &LanguageChoice {
                    language,
                    no_proof,
                    detect,
                },
            )
        })
    }

    /// The language the text at the caret is marked as, or `null`.
    #[wasm_bindgen(js_name = languageAtCaret)]
    pub fn language_at_caret(&self) -> Result<JsValue, JsValue> {
        let caret = self.current_selection().focus;
        let language =
            self.read_word_session(|session| session.language_at((caret.paragraph, caret.offset)))?;
        Ok(language.map_or(JsValue::NULL, |language| JsValue::from_str(&language)))
    }

    /// 類義語辞典: the word at the caret and its senses from the built-in
    /// list, or `null` when the caret is at no word.
    #[wasm_bindgen(js_name = thesaurusAtCaret)]
    pub fn thesaurus_at_caret(&self) -> Result<JsValue, JsValue> {
        let found = self.read_word_session(|session| {
            session.word_for_thesaurus().map(|(_, _, word)| {
                let senses = BuiltinThesaurus.lookup(&word);
                json!({
                    "word": word,
                    "senses": senses
                        .iter()
                        .map(|sense| json!({ "label": sense.label, "synonyms": sense.synonyms }))
                        .collect::<Vec<_>>(),
                })
            })
        })?;
        Ok(found.map_or(JsValue::NULL, |value| to_js(&value)))
    }

    /// 類義語辞典's 挿入: `synonym` in place of the word at the caret, with
    /// its capitals.
    #[wasm_bindgen(js_name = replaceWithSynonym)]
    pub fn replace_with_synonym(&self, synonym: String) -> Result<JsValue, JsValue> {
        self.run_word_session("word_replace_synonym", move |session| {
            let word = session
                .word_for_thesaurus()
                .ok_or("カーソル位置に単語がありません")?;
            session.replace_with_synonym(&word, &synonym)
        })
    }
}
