//! Word's legacy form fields (ECMA-376 Part 1 §17.16.17): the text box
//! (`FORMTEXT`), check box (`FORMCHECKBOX`) and drop-down (`FORMDROPDOWN`)
//! fields, whose settings ride on the field's `w:fldChar` as `w:ffData`.
//!
//! A text box's result is ordinary text the model edits in place; a check
//! box has no result at all — Word draws its box — so the loader gives the
//! model one ([`annotate_form_check_boxes`]); a drop-down shows the entry
//! chosen. When the document is protected for filling in forms
//! (`w:documentProtection w:edit="forms"`), these and the content controls
//! are all that can be changed.

use super::*;

/// Which legacy form field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormFieldKind {
    /// テキスト ボックス (`FORMTEXT`).
    Text,
    /// チェック ボックス (`FORMCHECKBOX`).
    CheckBox,
    /// ドロップダウン リスト (`FORMDROPDOWN`).
    DropDown,
}

impl FormFieldKind {
    fn from_instruction(instruction: &str) -> Option<Self> {
        let name = instruction.split_whitespace().next()?.to_ascii_uppercase();
        match name.as_str() {
            "FORMTEXT" => Some(FormFieldKind::Text),
            "FORMCHECKBOX" => Some(FormFieldKind::CheckBox),
            "FORMDROPDOWN" => Some(FormFieldKind::DropDown),
            _ => None,
        }
    }
}

/// One legacy form field.
#[derive(Clone, Debug, PartialEq)]
pub struct LegacyFormField {
    /// Its place among the document's form fields, in document order.
    pub index: usize,
    pub kind: FormFieldKind,
    /// ブックマーク (`w:name`).
    pub name: String,
    /// 入力を有効にする (`w:enabled`).
    pub enabled: bool,
    /// A text box's 既定の文字列, 最大文字数 (0 for no limit), 種類
    /// (`regular`, `number`, `date`, …) and 文字列の書式.
    pub default_text: String,
    pub max_length: usize,
    pub text_type: String,
    pub text_format: String,
    /// A check box's state and 既定値.
    pub checked: bool,
    pub default_checked: bool,
    /// A drop-down's entries and the one chosen.
    pub entries: Vec<String>,
    pub selected: usize,
    /// ヘルプ テキスト: the status bar's (`w:statusText`).
    pub status_text: Option<String>,
    /// The text the field shows: a text box's or drop-down's result.
    pub result: String,
    /// Where its result is, in rendered positions; for a check box, where
    /// the box is drawn, both the same.
    pub result_start: TextPosition,
    pub result_end: TextPosition,
}

/// The five en spaces Word gives a text box with no default text.
pub const EMPTY_TEXT_FIELD: &str = "\u{2002}\u{2002}\u{2002}\u{2002}\u{2002}";

/// The box a check box draws: ☒ ticked, ☐ not.
pub fn check_box_glyph(checked: bool) -> &'static str {
    if checked {
        "\u{2612}"
    } else {
        "\u{2610}"
    }
}

/// A form field's settings, read from its `w:ffData`.
#[derive(Default)]
struct FormData {
    name: String,
    enabled: bool,
    default_text: String,
    max_length: usize,
    text_type: String,
    text_format: String,
    checked: Option<bool>,
    default_checked: bool,
    entries: Vec<String>,
    selected: usize,
    status_text: Option<String>,
}

fn read_form_data(data: &XmlElement) -> FormData {
    let get = |name: &str| child(Some(data), name);
    let value = |element: Option<&XmlElement>| {
        element
            .and_then(|element| attribute(element, "w:val"))
            .map(str::to_string)
    };
    let mut form = FormData {
        name: value(get("w:name")).unwrap_or_default(),
        enabled: get("w:enabled").is_none_or(|enabled| {
            !matches!(attribute(enabled, "w:val"), Some("0" | "false" | "off"))
        }),
        status_text: value(get("w:statusText")),
        ..FormData::default()
    };
    if let Some(input) = get("w:textInput") {
        let part = |name: &str| child(Some(input), name);
        form.default_text = value(part("w:default")).unwrap_or_default();
        form.max_length = value(part("w:maxLength"))
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        form.text_type = value(part("w:type")).unwrap_or_else(|| "regular".to_string());
        form.text_format = value(part("w:format")).unwrap_or_default();
    }
    if let Some(check) = get("w:checkBox") {
        let flag = |name: &str| {
            child(Some(check), name)
                .map(|element| !matches!(attribute(element, "w:val"), Some("0" | "false" | "off")))
        };
        form.default_checked = flag("w:default").unwrap_or(false);
        form.checked = flag("w:checked");
    }
    if let Some(list) = get("w:ddList") {
        form.entries = list
            .children
            .iter()
            .filter_map(|node| match node {
                XmlNode::Element(entry) if entry.name == "w:listEntry" => {
                    attribute(entry, "w:val").map(str::to_string)
                }
                _ => None,
            })
            .collect();
        form.selected = child(Some(list), "w:result")
            .or_else(|| child(Some(list), "w:default"))
            .and_then(|element| attribute(element, "w:val"))
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
    }
    form
}

/// A form field met in the walk, until its `end`.
struct OpenField {
    data: FormData,
    instruction: String,
    /// Whether its `separate` has passed: what follows is its result.
    in_result: bool,
    result_runs: Vec<(usize, usize, usize)>,
    /// Where the field starts, for one with no result.
    anchor: Option<(usize, usize)>,
}

impl WordDocument {
    /// Every legacy form field of the body, in document order.
    pub fn legacy_form_fields(&self) -> Vec<LegacyFormField> {
        let Some(xml) = self.raw_document.as_ref() else {
            return Vec::new();
        };
        let places = self.model_places();
        let mut fields = Vec::new();
        // Fields nest: a form field holds its own, any other a `None`.
        let mut open: Vec<Option<OpenField>> = Vec::new();
        let mut last_run_end: Option<(usize, usize)> = None;
        walk_runs(xml.root(), &mut |run| {
            let place = places.runs.get(&run.id).copied();
            for node in &run.children {
                let XmlNode::Element(element) = node else {
                    continue;
                };
                match element.name.as_str() {
                    "w:fldChar" => match attribute(element, "w:fldCharType") {
                        Some("begin") => {
                            let field = child(Some(element), "w:ffData").map(|data| OpenField {
                                data: read_form_data(data),
                                instruction: String::new(),
                                in_result: false,
                                result_runs: Vec::new(),
                                anchor: last_run_end,
                            });
                            open.push(field);
                        }
                        Some("separate") => {
                            if let Some(Some(field)) = open.last_mut() {
                                field.in_result = true;
                            }
                        }
                        Some("end") => {
                            if let Some(Some(field)) = open.pop() {
                                if let Some(form) = self.finish_form_field(field, fields.len()) {
                                    fields.push(form);
                                }
                            }
                        }
                        _ => {}
                    },
                    "w:instrText" => {
                        if let Some(Some(field)) = open.last_mut() {
                            if !field.in_result {
                                for text in &element.children {
                                    if let XmlNode::Text(text) = text {
                                        field.instruction.push_str(text);
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            if let Some(place) = place {
                if let Some(Some(field)) = open.last_mut() {
                    if field.in_result && place.end > place.start {
                        field
                            .result_runs
                            .push((place.paragraph, place.start, place.end));
                    }
                }
                last_run_end = Some((place.paragraph, place.end));
            }
        });
        fields
    }

    fn finish_form_field(&self, field: OpenField, index: usize) -> Option<LegacyFormField> {
        let kind = FormFieldKind::from_instruction(&field.instruction)?;
        let (result_start, result_end, result) = match (
            field.result_runs.first().copied(),
            field.result_runs.last().copied(),
        ) {
            (Some(first), Some(last)) => {
                let text = self
                    .paragraph(first.0)
                    .map(|paragraph| {
                        let whole: String =
                            paragraph.runs.iter().map(|run| run.text.as_str()).collect();
                        if first.0 == last.0 {
                            whole.get(first.1..last.2).unwrap_or_default().to_string()
                        } else {
                            whole.get(first.1..).unwrap_or_default().to_string()
                        }
                    })
                    .unwrap_or_default();
                (
                    self.rendered(first.0, first.1),
                    self.rendered(last.0, last.2),
                    text,
                )
            }
            _ => {
                let (paragraph, offset) = field.anchor.unwrap_or((0, 0));
                let at = self.rendered(paragraph, offset);
                (at, at, String::new())
            }
        };
        let data = field.data;
        let result = match kind {
            FormFieldKind::DropDown if result.is_empty() => {
                data.entries.get(data.selected).cloned().unwrap_or_default()
            }
            _ => result,
        };
        Some(LegacyFormField {
            index,
            kind,
            name: data.name,
            enabled: data.enabled,
            default_text: data.default_text,
            max_length: data.max_length,
            text_type: data.text_type,
            text_format: data.text_format,
            checked: data.checked.unwrap_or(data.default_checked),
            default_checked: data.default_checked,
            entries: data.entries,
            selected: data.selected,
            status_text: data.status_text,
            result,
            result_start,
            result_end,
        })
    }

    /// Whether the document is protected so that only its forms can be
    /// filled in.
    pub fn protected_for_forms(&self) -> bool {
        self.document_protection()
            .is_some_and(|protection| protection.enforced && protection.edit == "forms")
    }
}

/// Calls `visit` with every `w:r` of the tree, in document order, before
/// walking into it (a text box's runs come after the run that holds it).
fn walk_runs(element: &XmlElement, visit: &mut impl FnMut(&XmlElement)) {
    for node in &element.children {
        let XmlNode::Element(node) = node else {
            continue;
        };
        if node.name == "w:r" {
            visit(node);
        }
        walk_runs(node, visit);
    }
}

/// Gives each `FORMCHECKBOX` the box Word draws for it as its displayed
/// result: Word writes the field with no result, so without one the box
/// would not show.
pub fn annotate_form_check_boxes(document: &mut Document, xml: &DocxXmlDocument) {
    // The state of each check box, in document order.
    let mut states = Vec::new();
    let mut pending: Option<bool> = None;
    walk_runs(xml.root(), &mut |run| {
        for node in &run.children {
            let XmlNode::Element(element) = node else {
                continue;
            };
            match element.name.as_str() {
                "w:fldChar" if attribute(element, "w:fldCharType") == Some("begin") => {
                    pending = child(Some(element), "w:ffData")
                        .and_then(|data| child(Some(data), "w:checkBox"))
                        .map(|check| {
                            let flag = |name: &str| {
                                child(Some(check), name).map(|element| {
                                    !matches!(
                                        attribute(element, "w:val"),
                                        Some("0" | "false" | "off")
                                    )
                                })
                            };
                            flag("w:checked").unwrap_or(flag("w:default").unwrap_or(false))
                        });
                }
                "w:instrText" => {
                    if let Some(state) = pending.take() {
                        let text: String = element
                            .children
                            .iter()
                            .filter_map(|node| match node {
                                XmlNode::Text(text) => Some(text.as_str()),
                                _ => None,
                            })
                            .collect();
                        if FormFieldKind::from_instruction(&text) == Some(FormFieldKind::CheckBox) {
                            states.push(state);
                        }
                    }
                }
                _ => {}
            }
        }
    });
    if states.is_empty() {
        return;
    }
    let mut states = states.into_iter();
    let Some(body) = document.body.as_mut() else {
        return;
    };
    fn visit(elements: &mut [BodyElement], states: &mut impl Iterator<Item = bool>) {
        for element in elements {
            match element {
                BodyElement::Paragraph(paragraph) => {
                    for run in &mut paragraph.as_mut().runs {
                        let Some(field) = run.field.as_mut() else {
                            continue;
                        };
                        let is_check_box = matches!(
                            field.kind.base_kind(),
                            RunFieldKind::Unsupported { instruction }
                                if FormFieldKind::from_instruction(instruction)
                                    == Some(FormFieldKind::CheckBox)
                        );
                        if is_check_box {
                            if let Some(state) = states.next() {
                                field.cached_text = Some(check_box_glyph(state).to_string());
                            }
                        }
                    }
                }
                BodyElement::Table(table) => {
                    for row in &mut table.as_mut().rows {
                        for cell in &mut row.cells {
                            visit(cell.blocks.as_mut_slice(), states);
                        }
                    }
                }
                BodyElement::Math(_) => {}
            }
        }
    }
    visit(body.elements.as_mut_slice(), &mut states);
}
