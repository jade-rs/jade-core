//! The browser entry points for Word's 開発 tab: content controls (rich and
//! plain text, picture, check box, combo box, drop-down list, date picker,
//! building block gallery, repeating section, group), their プロパティ,
//! legacy form fields, フォームの保護 and デザイン モード.
//!
//! Every edit is a `ContentControlAction`, one undo step, sent as JSON with
//! a `type` tag (`insert`, `setChecked`, `chooseEntry`, …).

use super::*;
use crate::word_document::content_controls::edit::FormFieldOptions;
use crate::word_document::content_controls::{
    CheckSymbol, ContentControl, ContentControlAction, ContentControlKind,
    ContentControlProperties, ControlAppearance, FormFieldKind, LegacyFormField, ListEntry,
};

#[derive(serde::Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ListEntryJs {
    display: String,
    value: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PositionJs {
    paragraph: usize,
    offset: usize,
}

impl From<TextPosition> for PositionJs {
    fn from(position: TextPosition) -> Self {
        Self {
            paragraph: position.paragraph,
            offset: position.offset,
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ContentControlJs {
    index: usize,
    id: Option<i64>,
    kind: String,
    title: Option<String>,
    tag: Option<String>,
    cannot_delete: bool,
    cannot_edit: bool,
    showing_placeholder: bool,
    temporary: bool,
    multi_line: bool,
    appearance: &'static str,
    color: Option<String>,
    style: Option<String>,
    entries: Vec<ListEntryJs>,
    date_format: Option<String>,
    date_language: Option<String>,
    full_date: Option<String>,
    checked: bool,
    gallery: Option<String>,
    category: Option<String>,
    section_title: Option<String>,
    fixed_items: bool,
    block: bool,
    start: PositionJs,
    end: PositionJs,
    text: String,
    depth: usize,
    parent: Option<usize>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FormFieldJs {
    index: usize,
    kind: &'static str,
    name: String,
    enabled: bool,
    default_text: String,
    max_length: usize,
    checked: bool,
    default_checked: bool,
    entries: Vec<String>,
    selected: usize,
    status_text: Option<String>,
    result: String,
    start: PositionJs,
    end: PositionJs,
}

/// プロパティ changes; each field left out stays as the control has it.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PropertiesPatchJs {
    title: Option<String>,
    tag: Option<String>,
    appearance: Option<String>,
    color: Option<Option<String>>,
    style: Option<Option<String>>,
    temporary: Option<bool>,
    cannot_delete: Option<bool>,
    cannot_edit: Option<bool>,
    multi_line: Option<bool>,
    entries: Option<Vec<ListEntryJs>>,
    date_format: Option<String>,
    date_language: Option<String>,
    checked_symbol: Option<String>,
    unchecked_symbol: Option<String>,
    section_title: Option<String>,
    fixed_items: Option<bool>,
    gallery: Option<String>,
    category: Option<String>,
}

/// A form field's オプション changes.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FormOptionsPatchJs {
    name: Option<String>,
    enabled: Option<bool>,
    default_text: Option<String>,
    max_length: Option<usize>,
    default_checked: Option<bool>,
    entries: Option<Vec<String>>,
    status_text: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum ActionJs {
    #[serde(rename_all = "camelCase")]
    Insert {
        kind: String,
        #[serde(default = "japanese_default")]
        japanese: bool,
    },
    SetChecked {
        control: usize,
        checked: bool,
    },
    ChooseEntry {
        control: usize,
        entry: usize,
    },
    SetText {
        control: usize,
        text: String,
    },
    ShowPlaceholder {
        control: usize,
    },
    SetDate {
        control: usize,
        date: String,
    },
    SetPicture {
        control: usize,
        bytes: Vec<u8>,
        mime: String,
    },
    InsertBuildingBlock {
        control: usize,
        name: String,
    },
    AddRepeatingItem {
        control: usize,
    },
    RemoveRepeatingItem {
        control: usize,
    },
    Remove {
        control: usize,
    },
    SetProperties {
        control: usize,
        properties: PropertiesPatchJs,
    },
    InsertFormField {
        kind: String,
    },
    SetFormCheckBox {
        field: usize,
        checked: bool,
    },
    ChooseFormEntry {
        field: usize,
        entry: usize,
    },
    SetFormText {
        field: usize,
        text: String,
    },
    SetFormFieldOptions {
        field: usize,
        options: FormOptionsPatchJs,
    },
    SaveBuildingBlock {
        name: String,
        gallery: String,
    },
    ProtectForms {
        enabled: bool,
    },
    DesignMode {
        enabled: bool,
    },
}

fn japanese_default() -> bool {
    true
}

fn kind_name(kind: &ContentControlKind) -> String {
    match kind {
        ContentControlKind::RichText => "richText".into(),
        ContentControlKind::PlainText => "plainText".into(),
        ContentControlKind::Picture => "picture".into(),
        ContentControlKind::CheckBox => "checkBox".into(),
        ContentControlKind::ComboBox => "comboBox".into(),
        ContentControlKind::DropDownList => "dropDownList".into(),
        ContentControlKind::Date => "date".into(),
        ContentControlKind::BuildingBlockGallery => "buildingBlockGallery".into(),
        ContentControlKind::RepeatingSection => "repeatingSection".into(),
        ContentControlKind::RepeatingSectionItem => "repeatingSectionItem".into(),
        ContentControlKind::Group => "group".into(),
        ContentControlKind::Other(name) => name.clone(),
    }
}

fn kind_of(name: &str) -> Result<ContentControlKind, JsValue> {
    Ok(match name {
        "richText" => ContentControlKind::RichText,
        "plainText" => ContentControlKind::PlainText,
        "picture" => ContentControlKind::Picture,
        "checkBox" => ContentControlKind::CheckBox,
        "comboBox" => ContentControlKind::ComboBox,
        "dropDownList" => ContentControlKind::DropDownList,
        "date" => ContentControlKind::Date,
        "buildingBlockGallery" => ContentControlKind::BuildingBlockGallery,
        "repeatingSection" => ContentControlKind::RepeatingSection,
        "group" => ContentControlKind::Group,
        _ => {
            return Err(JsValue::from_str(&format!(
                "unknown content control kind: {name}"
            )))
        }
    })
}

fn form_kind_name(kind: FormFieldKind) -> &'static str {
    match kind {
        FormFieldKind::Text => "text",
        FormFieldKind::CheckBox => "checkBox",
        FormFieldKind::DropDown => "dropDown",
    }
}

fn form_kind_of(name: &str) -> Result<FormFieldKind, JsValue> {
    Ok(match name {
        "text" => FormFieldKind::Text,
        "checkBox" => FormFieldKind::CheckBox,
        "dropDown" => FormFieldKind::DropDown,
        _ => {
            return Err(JsValue::from_str(&format!(
                "unknown form field kind: {name}"
            )))
        }
    })
}

fn control_js(control: ContentControl) -> ContentControlJs {
    ContentControlJs {
        index: control.index,
        id: control.id,
        kind: kind_name(&control.kind),
        title: control.title,
        tag: control.tag,
        cannot_delete: control.cannot_delete,
        cannot_edit: control.cannot_edit,
        showing_placeholder: control.showing_placeholder,
        temporary: control.temporary,
        multi_line: control.multi_line,
        appearance: control.appearance.as_ooxml(),
        color: control.color,
        style: control.style,
        entries: control
            .entries
            .into_iter()
            .map(|entry| ListEntryJs {
                display: entry.display,
                value: entry.value,
            })
            .collect(),
        date_format: control.date_format,
        date_language: control.date_language,
        full_date: control.full_date,
        checked: control.checked,
        gallery: control.gallery,
        category: control.category,
        section_title: control.section_title,
        fixed_items: control.fixed_items,
        block: control.block,
        start: control.start.into(),
        end: control.end.into(),
        text: control.text,
        depth: control.depth,
        parent: control.parent,
    }
}

fn form_field_js(field: LegacyFormField) -> FormFieldJs {
    FormFieldJs {
        index: field.index,
        kind: form_kind_name(field.kind),
        name: field.name,
        enabled: field.enabled,
        default_text: field.default_text,
        max_length: field.max_length,
        checked: field.checked,
        default_checked: field.default_checked,
        entries: field.entries,
        selected: field.selected,
        status_text: field.status_text,
        result: field.result,
        start: field.result_start.into(),
        end: field.result_end.into(),
    }
}

fn json_js<T: serde::Serialize>(value: &T) -> JsValue {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .unwrap_or(JsValue::NULL)
}

fn symbol_of(text: &str, font: &str) -> Result<CheckSymbol, JsValue> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(symbol), None) => Ok(CheckSymbol {
            symbol,
            font: font.to_string(),
        }),
        _ => Err(JsValue::from_str("a check box symbol is one character")),
    }
}

impl OffscreenDocumentView {
    /// The action for `action`, reading the control's or field's present
    /// properties for a patch.
    fn control_action(&self, action: ActionJs) -> Result<ContentControlAction, JsValue> {
        Ok(match action {
            ActionJs::Insert { kind, japanese } => ContentControlAction::Insert {
                kind: kind_of(&kind)?,
                japanese,
            },
            ActionJs::SetChecked { control, checked } => {
                ContentControlAction::SetChecked { control, checked }
            }
            ActionJs::ChooseEntry { control, entry } => {
                ContentControlAction::ChooseEntry { control, entry }
            }
            ActionJs::SetText { control, text } => ContentControlAction::SetText { control, text },
            ActionJs::ShowPlaceholder { control } => {
                ContentControlAction::ShowPlaceholder { control }
            }
            ActionJs::SetDate { control, date } => ContentControlAction::SetDate { control, date },
            ActionJs::SetPicture {
                control,
                bytes,
                mime,
            } => ContentControlAction::SetPicture {
                control,
                bytes,
                mime,
            },
            ActionJs::InsertBuildingBlock { control, name } => {
                ContentControlAction::InsertBuildingBlock { control, name }
            }
            ActionJs::AddRepeatingItem { control } => {
                ContentControlAction::AddRepeatingItem { control }
            }
            ActionJs::RemoveRepeatingItem { control } => {
                ContentControlAction::RemoveRepeatingItem { control }
            }
            ActionJs::Remove { control } => ContentControlAction::Remove { control },
            ActionJs::SetProperties {
                control,
                properties: patch,
            } => {
                let current = {
                    let state = self.state.borrow();
                    let document = state
                        .word_doc
                        .as_ref()
                        .ok_or_else(|| JsValue::from_str("No document loaded"))?;
                    let found = document
                        .content_controls()
                        .into_iter()
                        .find(|candidate| candidate.index == control)
                        .ok_or_else(|| JsValue::from_str("no such content control"))?;
                    ContentControlProperties::of(&found)
                };
                let mut properties = current;
                if let Some(title) = patch.title {
                    properties.title = title;
                }
                if let Some(tag) = patch.tag {
                    properties.tag = tag;
                }
                if let Some(appearance) = patch.appearance {
                    properties.appearance = match appearance.as_str() {
                        "boundingBox" => ControlAppearance::BoundingBox,
                        "tags" => ControlAppearance::Tags,
                        "hidden" => ControlAppearance::Hidden,
                        _ => {
                            return Err(JsValue::from_str(
                                "appearance must be boundingBox, tags or hidden",
                            ))
                        }
                    };
                }
                if let Some(color) = patch.color {
                    properties.color =
                        color.map(|color| color.trim_start_matches('#').to_uppercase());
                }
                if let Some(style) = patch.style {
                    properties.style = style;
                }
                if let Some(value) = patch.temporary {
                    properties.temporary = value;
                }
                if let Some(value) = patch.cannot_delete {
                    properties.cannot_delete = value;
                }
                if let Some(value) = patch.cannot_edit {
                    properties.cannot_edit = value;
                }
                if let Some(value) = patch.multi_line {
                    properties.multi_line = value;
                }
                if let Some(entries) = patch.entries {
                    properties.entries = entries
                        .into_iter()
                        .map(|entry| ListEntry {
                            display: entry.display,
                            value: entry.value,
                        })
                        .collect();
                }
                if let Some(value) = patch.date_format {
                    properties.date_format = value;
                }
                if let Some(value) = patch.date_language {
                    properties.date_language = value;
                }
                if let Some(symbol) = patch.checked_symbol {
                    let font = properties.checked_symbol.font.clone();
                    properties.checked_symbol = symbol_of(&symbol, &font)?;
                }
                if let Some(symbol) = patch.unchecked_symbol {
                    let font = properties.unchecked_symbol.font.clone();
                    properties.unchecked_symbol = symbol_of(&symbol, &font)?;
                }
                if let Some(value) = patch.section_title {
                    properties.section_title = value;
                }
                if let Some(value) = patch.fixed_items {
                    properties.fixed_items = value;
                }
                if let Some(value) = patch.gallery {
                    properties.gallery = value;
                }
                if let Some(value) = patch.category {
                    properties.category = value;
                }
                ContentControlAction::SetProperties {
                    control,
                    properties,
                }
            }
            ActionJs::InsertFormField { kind } => ContentControlAction::InsertFormField {
                kind: form_kind_of(&kind)?,
            },
            ActionJs::SetFormCheckBox { field, checked } => {
                ContentControlAction::SetFormCheckBox { field, checked }
            }
            ActionJs::ChooseFormEntry { field, entry } => {
                ContentControlAction::ChooseFormEntry { field, entry }
            }
            ActionJs::SetFormText { field, text } => {
                ContentControlAction::SetFormText { field, text }
            }
            ActionJs::SetFormFieldOptions {
                field,
                options: patch,
            } => {
                let mut options = {
                    let state = self.state.borrow();
                    let document = state
                        .word_doc
                        .as_ref()
                        .ok_or_else(|| JsValue::from_str("No document loaded"))?;
                    let found = document
                        .legacy_form_fields()
                        .into_iter()
                        .find(|candidate| candidate.index == field)
                        .ok_or_else(|| JsValue::from_str("no such form field"))?;
                    FormFieldOptions::of(&found)
                };
                if let Some(value) = patch.name {
                    options.name = value;
                }
                if let Some(value) = patch.enabled {
                    options.enabled = value;
                }
                if let Some(value) = patch.default_text {
                    options.default_text = value;
                }
                if let Some(value) = patch.max_length {
                    options.max_length = value;
                }
                if let Some(value) = patch.default_checked {
                    options.default_checked = value;
                }
                if let Some(value) = patch.entries {
                    options.entries = value;
                }
                if let Some(value) = patch.status_text {
                    options.status_text = value;
                }
                ContentControlAction::SetFormFieldOptions { field, options }
            }
            ActionJs::SaveBuildingBlock { name, gallery } => {
                ContentControlAction::SaveBuildingBlock { name, gallery }
            }
            ActionJs::ProtectForms { enabled } => ContentControlAction::ProtectForms(enabled),
            ActionJs::DesignMode { enabled } => ContentControlAction::DesignMode(enabled),
        })
    }
}

#[wasm_bindgen]
impl OffscreenDocumentView {
    /// The document's content controls, in document order.
    #[wasm_bindgen(js_name = contentControls)]
    pub fn content_controls(&self) -> JsValue {
        let state = self.state.borrow();
        let controls: Vec<ContentControlJs> = state
            .word_doc
            .as_ref()
            .map(WordDocument::content_controls)
            .unwrap_or_default()
            .into_iter()
            .map(control_js)
            .collect();
        json_js(&controls)
    }

    /// The innermost content control the caret is in; `null` outside one.
    #[wasm_bindgen(js_name = contentControlAtSelection)]
    pub fn content_control_at_selection(&self) -> JsValue {
        let (start, _) = self.current_selection().normalized();
        let state = self.state.borrow();
        state
            .word_doc
            .as_ref()
            .and_then(|document| document.content_control_at(start))
            .map_or(JsValue::NULL, |control| json_js(&control_js(control)))
    }

    /// The document's legacy form fields, in document order.
    #[wasm_bindgen(js_name = legacyFormFields)]
    pub fn legacy_form_fields(&self) -> JsValue {
        let state = self.state.borrow();
        let fields: Vec<FormFieldJs> = state
            .word_doc
            .as_ref()
            .map(WordDocument::legacy_form_fields)
            .unwrap_or_default()
            .into_iter()
            .map(form_field_js)
            .collect();
        json_js(&fields)
    }

    /// デザイン モード and フォームの保護, as `{ designMode, protectedForForms }`.
    #[wasm_bindgen(js_name = formsState)]
    pub fn forms_state(&self) -> JsValue {
        let state = self.state.borrow();
        let Some(document) = state.word_doc.as_ref() else {
            return JsValue::NULL;
        };
        let result = Object::new();
        let _ = Reflect::set(
            &result,
            &JsValue::from_str("designMode"),
            &JsValue::from_bool(document.forms_design_mode()),
        );
        let _ = Reflect::set(
            &result,
            &JsValue::from_str("protectedForForms"),
            &JsValue::from_bool(document.protected_for_forms()),
        );
        result.into()
    }

    /// Applies one 開発-tab action (`{ type: "insert", kind: "richText" }`,
    /// `{ type: "setChecked", control, checked }`, …) as one undo step.
    #[wasm_bindgen(js_name = applyContentControlAction)]
    pub fn apply_content_control_action(&self, action: JsValue) -> Result<JsValue, JsValue> {
        let action: ActionJs = serde_wasm_bindgen::from_value(action).map_err(|error| {
            JsValue::from_str(&format!("invalid content control action: {error}"))
        })?;
        let action = self.control_action(action)?;
        self.apply_document_command(move |selection| Box::new(action.command(selection)))
    }
}
