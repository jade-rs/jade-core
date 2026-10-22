//! The browser entry points for Word's フィールド tools and 編集の制限: update,
//! lock, unlink, insert and edit field codes, the document's プロパティ, and
//! the restriction kinds, their exceptions (ranges everyone may edit) and
//! the 書き込みパスワード / 読み取り専用を推奨 pair.
//!
//! Each edit is one undo step and re-lays the whole document out: a field's
//! result or a restriction can change anything after it.

use super::word_parity::{camel_keys, snake_case};
use super::*;
use crate::commands::insert_image::InsertImageCommand;
use crate::commands::insert_signature_line::InsertSignatureLineCommand;
use crate::commands::snapshot_edit::{EditScope, SnapshotEditCommand};
use crate::fields::{self, DocumentProperties, FieldCommand, FieldEnvironment, FieldScope};
use crate::word_document::{
    AltText, EditingKind, EditingRestriction, PasswordHash, WriteProtection,
};
use std::cell::RefCell;
use std::collections::HashMap;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FieldInfoJs {
    code: String,
    name: String,
    result: String,
    locked: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FieldPromptJs {
    key: String,
    field: String,
    prompt: String,
    default: String,
}

/// What the restriction pane shows; the password hash never leaves the
/// worker.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct EditingRestrictionJs {
    kind: Option<&'static str>,
    formatting: bool,
    enforced: bool,
    has_password: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct EditablePositionJs {
    paragraph: usize,
    offset: usize,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct EditableRangeJs {
    id: String,
    start: EditablePositionJs,
    end: EditablePositionJs,
    editors: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AltTextJs {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    decorative: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SignatureLineJs {
    paragraph: usize,
    run: usize,
    #[serde(flatten)]
    line: jade_docx_reader::SignatureLine,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct WriteProtectionJs {
    recommended: bool,
    has_password: bool,
}

fn json_js<T: serde::Serialize>(value: &T) -> JsValue {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .unwrap_or(JsValue::NULL)
}

fn field_scope(scope: &str, selection: SelectionRange) -> Result<FieldScope, JsValue> {
    match scope {
        "selection" => Ok(FieldScope::Selection(selection)),
        "document" => Ok(FieldScope::Document),
        _ => Err(JsValue::from_str(
            "field scope must be selection or document",
        )),
    }
}

/// A 16-byte salt the worker drew from `crypto.getRandomValues`.
fn salt_of(salt: &[u8]) -> Result<[u8; 16], JsValue> {
    salt.try_into()
        .map_err(|_| JsValue::from_str("a password salt must be 16 random bytes"))
}

fn editing_kind(kind: Option<String>) -> Result<Option<EditingKind>, JsValue> {
    kind.filter(|kind| !kind.is_empty())
        .map(|kind| {
            EditingKind::from_attribute(&kind).ok_or_else(|| {
                JsValue::from_str(
                    "restriction kind must be readOnly, comments, trackedChanges or forms",
                )
            })
        })
        .transpose()
}

impl OffscreenDocumentView {
    /// What field evaluation needs from the browser: the clock, the author
    /// name the editor records revisions under, and the laid-out pages.
    fn field_environment(&self, answers: HashMap<String, String>) -> FieldEnvironment {
        let now = js_sys::Date::new_0();
        let state = self.state.borrow();
        let renderer = self.renderer.borrow();
        let paragraph_count = state
            .word_doc
            .as_ref()
            .map_or(0, WordDocument::paragraph_count);
        let paragraph_pages = (0..paragraph_count)
            .filter_map(|index| {
                renderer
                    .cached_paragraph_start_page(index)
                    .map(|page| (index, page))
            })
            .collect();
        FieldEnvironment {
            now: {
                let clock = current_field_datetime();
                jade_docx_reader::field_datetime::FieldDateTime {
                    year: clock.year,
                    month: clock.month,
                    day: clock.day,
                    hour: clock.hour,
                    minute: clock.minute,
                    second: clock.second,
                    weekday: clock.weekday,
                }
            },
            // getTimezoneOffset is minutes *west* of UTC.
            utc_offset_seconds: -(now.get_timezone_offset() as i64) * 60,
            user_name: state.options.author_name_or_default(),
            page_count: Some(renderer.page_count()).filter(|count| *count > 0),
            paragraph_pages,
            prompt_answers: answers,
            ..FieldEnvironment::default()
        }
    }

    /// Runs one field or protection command over the whole document.
    fn apply_document_command(
        &self,
        command: impl FnOnce(SelectionRange) -> Box<dyn Command>,
    ) -> Result<JsValue, JsValue> {
        let command = RefCell::new(Some(command));
        self.apply_inline_style_with_scope(
            |_| {},
            |selection| {
                let make = command
                    .borrow_mut()
                    .take()
                    .expect("the command factory runs once");
                make(selection)
            },
            true,
        )
    }

    fn restriction_command(
        name: &'static str,
        restriction: EditingRestriction,
    ) -> Box<dyn Command> {
        Box::new(SnapshotEditCommand::with_scope(
            name,
            EditScope::Paragraphs(None),
            move |document| document.set_editing_restriction(Some(&restriction)),
        ))
    }
}

#[wasm_bindgen]
impl OffscreenDocumentView {
    /// The fields the selection touches (the one at a caret), as フィールドの
    ///編集 shows them.
    #[wasm_bindgen(js_name = fieldsAtSelection)]
    pub fn fields_at_selection(&self) -> JsValue {
        let selection = self.current_selection();
        let state = self.state.borrow();
        let Some(document) = state.word_doc.as_ref() else {
            return JsValue::NULL;
        };
        let fields: Vec<FieldInfoJs> = fields::fields_at(document, selection)
            .into_iter()
            .map(|field| FieldInfoJs {
                code: field.code,
                name: field.name,
                result: field.result,
                locked: field.locked,
            })
            .collect();
        json_js(&fields)
    }

    /// The ASK / FILLIN prompts an update of `scope` (`selection` or
    /// `document`) would show. Pass their answers to `updateFields` by key.
    #[wasm_bindgen(js_name = fieldPrompts)]
    pub fn field_prompts(&self, scope: String) -> Result<JsValue, JsValue> {
        let scope = field_scope(&scope, self.current_selection())?;
        let state = self.state.borrow();
        let Some(document) = state.word_doc.as_ref() else {
            return Ok(JsValue::NULL);
        };
        let prompts: Vec<FieldPromptJs> = fields::field_prompts(document, scope)
            .into_iter()
            .map(|prompt| FieldPromptJs {
                key: prompt.key,
                field: prompt.field,
                prompt: prompt.prompt,
                default: prompt.default,
            })
            .collect();
        Ok(json_js(&prompts))
    }

    /// F9: updates the fields of `scope` (`selection` or `document`), with
    /// `answers` (`{ key: text }`) for its ASK and FILLIN prompts.
    #[wasm_bindgen(js_name = updateFields)]
    pub fn update_fields(&self, scope: String, answers: JsValue) -> Result<JsValue, JsValue> {
        let scope = field_scope(&scope, self.current_selection())?;
        let answers: HashMap<String, String> = if answers.is_null() || answers.is_undefined() {
            HashMap::new()
        } else {
            serde_wasm_bindgen::from_value(answers)
                .map_err(|error| JsValue::from_str(&format!("invalid field answers: {error}")))?
        };
        let env = self.field_environment(answers);
        self.apply_document_command(move |_| Box::new(FieldCommand::update(scope, env)))
    }

    /// ⌘F11 / ⌘⇧F11: locks or unlocks the fields of `scope`.
    #[wasm_bindgen(js_name = setFieldsLocked)]
    pub fn set_fields_locked(&self, scope: String, locked: bool) -> Result<JsValue, JsValue> {
        let scope = field_scope(&scope, self.current_selection())?;
        self.apply_document_command(move |_| Box::new(FieldCommand::lock(scope, locked)))
    }

    /// ⌘⇧F9: replaces the fields of `scope` by their results.
    #[wasm_bindgen(js_name = unlinkFields)]
    pub fn unlink_fields(&self, scope: String) -> Result<JsValue, JsValue> {
        let scope = field_scope(&scope, self.current_selection())?;
        self.apply_document_command(move |_| Box::new(FieldCommand::unlink(scope)))
    }

    /// 挿入 › フィールド: a field with `code` (`PAGE \* roman`, `AUTHOR`, …) in
    /// place of the selection, evaluated now.
    #[wasm_bindgen(js_name = insertFieldCode)]
    pub fn insert_field_code(&self, code: String) -> Result<JsValue, JsValue> {
        if code.trim().is_empty() {
            return Err(JsValue::from_str("a field code must not be empty"));
        }
        let env = self.field_environment(HashMap::new());
        self.apply_document_command(move |_| Box::new(FieldCommand::insert(code, env)))
    }

    /// フィールドの編集: gives the field at the selection `code`, evaluated now.
    #[wasm_bindgen(js_name = editFieldCode)]
    pub fn edit_field_code(&self, code: String) -> Result<JsValue, JsValue> {
        if code.trim().is_empty() {
            return Err(JsValue::from_str("a field code must not be empty"));
        }
        let env = self.field_environment(HashMap::new());
        self.apply_document_command(move |selection| {
            Box::new(FieldCommand::edit(selection, code, env))
        })
    }

    /// The document's プロパティ (core, app and custom), in camelCase.
    #[wasm_bindgen(js_name = documentProperties)]
    pub fn document_properties(&self) -> JsValue {
        let state = self.state.borrow();
        let Some(document) = state.word_doc.as_ref() else {
            return JsValue::NULL;
        };
        serde_json::to_value(fields::document_properties(document))
            .map(camel_keys)
            .map(|json| json_js(&json))
            .unwrap_or(JsValue::NULL)
    }

    /// Sets the properties `patch` names; the rest stay as they are.
    #[wasm_bindgen(js_name = setDocumentProperties)]
    pub fn set_document_properties(&self, patch: JsValue) -> Result<JsValue, JsValue> {
        let patch: serde_json::Value = serde_wasm_bindgen::from_value(patch)
            .map_err(|error| JsValue::from_str(&format!("invalid document properties: {error}")))?;
        let current = {
            let state = self.state.borrow();
            let document = state
                .word_doc
                .as_ref()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;
            serde_json::to_value(fields::document_properties(document))
                .map_err(|error| JsValue::from_str(&error.to_string()))?
        };
        // Only the property names are camelCase; a custom property's value
        // keeps its own tags.
        let (Some(patch), serde_json::Value::Object(mut merged)) = (
            patch.as_object().map(|patch| {
                patch
                    .iter()
                    .map(|(key, value)| (snake_case(key), value.clone()))
                    .collect::<serde_json::Map<_, _>>()
            }),
            current,
        ) else {
            return Err(JsValue::from_str("document properties must be an object"));
        };
        for (key, value) in patch {
            if !merged.contains_key(&key) {
                return Err(JsValue::from_str(&format!(
                    "invalid document properties: unknown field `{key}`"
                )));
            }
            merged.insert(key, value);
        }
        let properties: DocumentProperties =
            serde_json::from_value(serde_json::Value::Object(merged)).map_err(|error| {
                JsValue::from_str(&format!("invalid document properties: {error}"))
            })?;
        self.apply_document_command(move |_| Box::new(FieldCommand::properties(properties)))
    }

    /// The document's 編集の制限, enforced or not; `null` when it has none.
    #[wasm_bindgen(js_name = editingRestriction)]
    pub fn editing_restriction(&self) -> JsValue {
        let state = self.state.borrow();
        state
            .word_doc
            .as_ref()
            .and_then(WordDocument::editing_restriction)
            .map_or(JsValue::NULL, |restriction| {
                json_js(&EditingRestrictionJs {
                    kind: restriction.kind.map(EditingKind::attribute),
                    formatting: restriction.formatting,
                    enforced: restriction.enforced,
                    has_password: restriction.password.is_some(),
                })
            })
    }

    /// 保護の開始: restricts editing to `kind` (`readOnly`, `comments`,
    /// `trackedChanges`, `forms`, or `null` with `formatting` alone), with
    /// `password` hashed over `salt` when one is given.
    #[wasm_bindgen(js_name = startEditingRestriction)]
    pub fn start_editing_restriction(
        &self,
        kind: Option<String>,
        formatting: bool,
        password: Option<String>,
        salt: &[u8],
    ) -> Result<JsValue, JsValue> {
        let kind = editing_kind(kind)?;
        if kind.is_none() && !formatting {
            return Err(JsValue::from_str("choose what the restriction allows"));
        }
        let salt = salt_of(salt)?;
        let restriction = EditingRestriction {
            kind,
            formatting,
            enforced: true,
            password: password
                .filter(|password| !password.is_empty())
                .map(|password| PasswordHash::new(&password, &salt)),
        };
        self.apply_document_command(move |_| {
            Self::restriction_command("set_editing_restriction", restriction)
        })
    }

    /// 保護の中止: with the restriction's password when it has one. A
    /// protection Word wrote with its older hash cannot be checked here and
    /// is lifted as Word lifts one without a password. The choices stay for
    /// the next 保護の開始.
    #[wasm_bindgen(js_name = stopEditingRestriction)]
    pub fn stop_editing_restriction(&self, password: String) -> Result<JsValue, JsValue> {
        let Some(restriction) = self
            .state
            .borrow()
            .word_doc
            .as_ref()
            .and_then(WordDocument::editing_restriction)
        else {
            return Ok(JsValue::NULL);
        };
        if let Some(hash) = &restriction.password {
            if hash.verifies(&password) == Some(false) {
                return Err(JsValue::from_str("the password is wrong"));
            }
        }
        let stopped = EditingRestriction {
            enforced: false,
            password: None,
            ..restriction
        };
        self.apply_document_command(move |_| {
            Self::restriction_command("set_editing_restriction", stopped)
        })
    }

    /// The exceptions of a restriction: the body ranges everyone (or an
    /// editor) may change.
    #[wasm_bindgen(js_name = editableRanges)]
    pub fn editable_ranges(&self) -> JsValue {
        let state = self.state.borrow();
        let ranges: Vec<EditableRangeJs> = state
            .word_doc
            .as_ref()
            .map(WordDocument::editable_ranges)
            .unwrap_or_default()
            .into_iter()
            .map(|range| EditableRangeJs {
                id: range.id,
                start: EditablePositionJs {
                    paragraph: range.start.0,
                    offset: range.start.1,
                },
                end: EditablePositionJs {
                    paragraph: range.end.0,
                    offset: range.end.1,
                },
                editors: range.editors,
            })
            .collect();
        json_js(&ranges)
    }

    /// 例外処理 › すべてのユーザー for the selected paragraphs.
    #[wasm_bindgen(js_name = addEditableRange)]
    pub fn add_editable_range(&self) -> Result<JsValue, JsValue> {
        let (start, end) = self.current_selection().normalized();
        let (first, last) = (start.paragraph, end.paragraph);
        self.apply_document_command(move |_| {
            Box::new(SnapshotEditCommand::with_scope(
                "add_editable_range_restriction",
                EditScope::Paragraphs(Some((first, last))),
                move |document| {
                    document
                        .add_editable_paragraphs(first, last, "everyone")
                        .map(|_| ())
                },
            ))
        })
    }

    /// Takes an exception out by its id.
    #[wasm_bindgen(js_name = removeEditableRange)]
    pub fn remove_editable_range(&self, id: String) -> Result<JsValue, JsValue> {
        self.apply_document_command(move |_| {
            Box::new(SnapshotEditCommand::new(
                "remove_editable_range_restriction",
                move |document| document.remove_editable_range(&id),
            ))
        })
    }

    /// 書き込みパスワード and 読み取り専用を推奨; `null` when neither is set.
    #[wasm_bindgen(js_name = writeProtection)]
    pub fn write_protection(&self) -> JsValue {
        let state = self.state.borrow();
        state
            .word_doc
            .as_ref()
            .and_then(WordDocument::write_protection)
            .map_or(JsValue::NULL, |protection| {
                json_js(&WriteProtectionJs {
                    recommended: protection.recommended,
                    has_password: protection.password.is_some(),
                })
            })
    }

    /// Sets 読み取り専用を推奨 and the 書き込みパスワード together: `password`
    /// `undefined` keeps the current one, `""` removes it.
    #[wasm_bindgen(js_name = setWriteProtection)]
    pub fn set_write_protection(
        &self,
        recommended: bool,
        password: Option<String>,
        salt: &[u8],
    ) -> Result<JsValue, JsValue> {
        let salt = salt_of(salt)?;
        let current = self
            .state
            .borrow()
            .word_doc
            .as_ref()
            .and_then(WordDocument::write_protection);
        let password = match password.as_deref() {
            Some("") => None,
            Some(password) => Some(PasswordHash::new(password, &salt)),
            None => current.and_then(|protection| protection.password),
        };
        let protection = WriteProtection {
            recommended,
            password,
        };
        self.apply_document_command(move |_| {
            Box::new(SnapshotEditCommand::with_scope(
                "set_write_protection",
                EditScope::Paragraphs(None),
                move |document| document.set_write_protection(Some(&protection)),
            ))
        })
    }

    /// The selected picture's or shape's 代替テキスト; `null` with no object
    /// selected.
    #[wasm_bindgen(js_name = objectAltText)]
    pub fn object_alt_text(&self) -> JsValue {
        let state = self.state.borrow();
        let (Some(target), Some(document)) = (state.object_selection, state.word_doc.as_ref())
        else {
            return JsValue::NULL;
        };
        document
            .object_alt_text(target)
            .map_or(JsValue::NULL, |alt| {
                json_js(&AltTextJs {
                    description: alt.description,
                    decorative: alt.decorative,
                })
            })
    }

    /// Sets the selected object's 代替テキスト (`{ description, decorative }`);
    /// a decorative object keeps no description, as in Word.
    #[wasm_bindgen(js_name = setObjectAltText)]
    pub fn set_object_alt_text(&self, alt: JsValue) -> Result<JsValue, JsValue> {
        let alt: AltTextJs = serde_wasm_bindgen::from_value(alt)
            .map_err(|error| JsValue::from_str(&format!("invalid alt text: {error}")))?;
        let Some(target) = self.state.borrow().object_selection else {
            return Ok(JsValue::NULL);
        };
        let paragraph = target.paragraph_index;
        let alt = AltText {
            description: alt.description,
            decorative: alt.decorative,
        };
        self.apply_document_command(move |_| {
            Box::new(SnapshotEditCommand::with_scope(
                "set_object_alt_text",
                EditScope::Paragraphs(Some((paragraph, paragraph))),
                move |document| document.set_object_alt_text(target, &alt),
            ))
        })
    }

    /// 挿入 ▸ 署名欄: `image` (the line's picture, drawn by the caller) at
    /// the caret, made a signature line asking `line`'s signer to sign.
    #[wasm_bindgen(js_name = insertSignatureLine)]
    pub fn insert_signature_line(
        &self,
        image: Vec<u8>,
        mime_type: String,
        width_px: f64,
        height_px: f64,
        line: JsValue,
    ) -> Result<JsValue, JsValue> {
        let line: jade_docx_reader::SignatureLine = serde_wasm_bindgen::from_value(line)
            .map_err(|error| JsValue::from_str(&format!("invalid signature line: {error}")))?;
        // The page images are cached by page; the new picture joins them.
        self.state.borrow_mut().paged_images.clear();
        self.apply_document_command(move |selection| {
            Box::new(InsertSignatureLineCommand::new(
                InsertImageCommand::new(
                    selection, image, mime_type, width_px, height_px, None, None,
                ),
                line,
            ))
        })
    }

    /// The document's signature lines, with where their pictures are.
    #[wasm_bindgen(js_name = signatureLines)]
    pub fn signature_lines(&self) -> JsValue {
        let state = self.state.borrow();
        let lines: Vec<SignatureLineJs> = state
            .word_doc
            .as_ref()
            .map(WordDocument::signature_lines)
            .unwrap_or_default()
            .into_iter()
            .map(|(paragraph, run, line)| SignatureLineJs {
                paragraph,
                run,
                line,
            })
            .collect();
        json_js(&lines)
    }
}
