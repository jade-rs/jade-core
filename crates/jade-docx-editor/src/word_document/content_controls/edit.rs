//! What changes a content control or a legacy form field: putting one in
//! around the selection (or with its placeholder at the caret), ticking a
//! box, choosing an entry or a date, a picture, a building block, adding a
//! repeating section's item, taking a control away (its content stays),
//! its properties, and the document's protection for filling in forms.
//! Each is one [`RewriteCommand`]: an edit of the XML, read back.

use super::forms::{check_box_glyph, EMPTY_TEXT_FIELD};
use super::*;

/// The date formats Word's 日付選択 offers, as `w:dateFormat` pictures.
pub const DATE_FORMATS: [&str; 10] = [
    "yyyy/MM/dd",
    "yyyy年M月d日",
    "yyyy年M月d日 (aaa)",
    "ggge年M月d日",
    "yyyy-MM-dd",
    "M/d/yyyy",
    "MMMM d, yyyy",
    "dddd, MMMM d, yyyy",
    "d-MMM-yy",
    "yyyy.MM.dd",
];

/// A date format and how it shows a date.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DateFormat {
    pub picture: String,
}

/// The galleries a 構成要素ギャラリー control can draw from.
pub const BUILDING_BLOCK_GALLERIES: [&str; 8] = [
    "Quick Parts",
    "AutoText",
    "Cover Pages",
    "Equations",
    "Tables",
    "Text Boxes",
    "Watermarks",
    "Headers",
];

/// A content control's プロパティ, as the dialog edits them.
#[derive(Clone, Debug, PartialEq)]
pub struct ContentControlProperties {
    pub title: String,
    pub tag: String,
    pub appearance: ControlAppearance,
    /// `RRGGBB`, or none for the automatic colour.
    pub color: Option<String>,
    /// 書式設定に使用するスタイル: a character style id.
    pub style: Option<String>,
    pub temporary: bool,
    pub cannot_delete: bool,
    pub cannot_edit: bool,
    pub multi_line: bool,
    pub entries: Vec<ListEntry>,
    pub date_format: String,
    pub date_language: String,
    pub checked_symbol: CheckSymbol,
    pub unchecked_symbol: CheckSymbol,
    pub section_title: String,
    pub fixed_items: bool,
    pub gallery: String,
    pub category: String,
}

const CHECK_FONT: &str = "MS Gothic";

impl ContentControlProperties {
    /// The properties a control has.
    pub fn of(control: &ContentControl) -> Self {
        Self {
            title: control.title.clone().unwrap_or_default(),
            tag: control.tag.clone().unwrap_or_default(),
            appearance: control.appearance,
            color: control.color.clone(),
            style: control.style.clone(),
            temporary: control.temporary,
            cannot_delete: control.cannot_delete,
            cannot_edit: control.cannot_edit,
            multi_line: control.multi_line,
            entries: control.entries.clone(),
            date_format: control
                .date_format
                .clone()
                .unwrap_or_else(|| DATE_FORMATS[0].to_string()),
            date_language: control
                .date_language
                .clone()
                .unwrap_or_else(|| "ja-JP".to_string()),
            checked_symbol: control.checked_symbol.clone().unwrap_or(CheckSymbol {
                symbol: '\u{2612}',
                font: CHECK_FONT.to_string(),
            }),
            unchecked_symbol: control.unchecked_symbol.clone().unwrap_or(CheckSymbol {
                symbol: '\u{2610}',
                font: CHECK_FONT.to_string(),
            }),
            section_title: control.section_title.clone().unwrap_or_default(),
            fixed_items: control.fixed_items,
            gallery: control
                .gallery
                .clone()
                .unwrap_or_else(|| BUILDING_BLOCK_GALLERIES[0].to_string()),
            category: control.category.clone().unwrap_or_default(),
        }
    }

    /// What a new control of `kind` starts with.
    pub fn new_for(kind: &ContentControlKind, japanese: bool) -> Self {
        let mut properties = Self::of(&ContentControl {
            index: 0,
            id: None,
            kind: kind.clone(),
            title: None,
            tag: None,
            cannot_delete: false,
            cannot_edit: false,
            showing_placeholder: false,
            placeholder_part: None,
            temporary: false,
            multi_line: false,
            appearance: ControlAppearance::BoundingBox,
            color: None,
            style: None,
            entries: Vec::new(),
            date_format: None,
            date_language: None,
            full_date: None,
            checked: false,
            checked_symbol: None,
            unchecked_symbol: None,
            gallery: None,
            category: None,
            section_title: None,
            fixed_items: false,
            block: false,
            start: TextPosition::new(0, 0),
            end: TextPosition::new(0, 0),
            text: String::new(),
            depth: 0,
            parent: None,
        });
        if matches!(
            kind,
            ContentControlKind::ComboBox | ContentControlKind::DropDownList
        ) {
            properties.entries = vec![ListEntry {
                display: choose_item_text(japanese).to_string(),
                value: String::new(),
            }];
        }
        if !japanese {
            properties.date_format = "M/d/yyyy".to_string();
            properties.date_language = "en-US".to_string();
        }
        properties
    }
}

fn choose_item_text(japanese: bool) -> &'static str {
    if japanese {
        "アイテムを選択してください。"
    } else {
        "Choose an item."
    }
}

/// The placeholder Word puts in a new control of `kind`.
pub fn default_placeholder(kind: &ContentControlKind, japanese: bool) -> Option<&'static str> {
    Some(match (kind, japanese) {
        (ContentControlKind::RichText | ContentControlKind::PlainText, true) => {
            "ここをクリックまたはタップしてテキストを入力してください。"
        }
        (ContentControlKind::RichText | ContentControlKind::PlainText, false) => {
            "Click or tap here to enter text."
        }
        (ContentControlKind::ComboBox | ContentControlKind::DropDownList, japanese) => {
            choose_item_text(japanese)
        }
        (ContentControlKind::Date, true) => "クリックまたはタップして日付を入力してください。",
        (ContentControlKind::Date, false) => "Click or tap to enter a date.",
        (ContentControlKind::BuildingBlockGallery, true) => {
            "ここをクリックまたはタップして構成要素を選択してください。"
        }
        (ContentControlKind::BuildingBlockGallery, false) => "Choose a building block.",
        _ => return None,
    })
}

/// What a command does to a content control or a form field.
#[derive(Clone, Debug, PartialEq)]
pub enum ContentControlAction {
    /// 開発 → a control: around the selection, or with its placeholder at
    /// the caret. `japanese` picks the placeholder's language.
    Insert {
        kind: ContentControlKind,
        japanese: bool,
    },
    /// Tick or clear a check box.
    SetChecked { control: usize, checked: bool },
    /// A combo box's or drop-down list's entry.
    ChooseEntry { control: usize, entry: usize },
    /// The control's content, as one run of text: what is typed over a
    /// placeholder, a combo box's own value.
    SetText { control: usize, text: String },
    /// A date picker's date, `yyyy-mm-dd`.
    SetDate { control: usize, date: String },
    /// A picture control's picture: PNG or JPEG bytes.
    SetPicture {
        control: usize,
        bytes: Vec<u8>,
        mime: String,
    },
    /// A building block from the document's glossary, by name.
    InsertBuildingBlock { control: usize, name: String },
    /// A repeating section's item copied after the one at `control`.
    AddRepeatingItem { control: usize },
    /// A repeating section's item taken away.
    RemoveRepeatingItem { control: usize },
    /// The control taken away, its content left as it is.
    Remove { control: usize },
    /// プロパティ.
    SetProperties {
        control: usize,
        properties: ContentControlProperties,
    },
    /// 開発 → レガシ フォーム: a text box, check box or drop-down field at
    /// the caret.
    InsertFormField { kind: FormFieldKind },
    /// A form check box ticked or cleared.
    SetFormCheckBox { field: usize, checked: bool },
    /// A form drop-down's entry.
    ChooseFormEntry { field: usize, entry: usize },
    /// A form text box's result: what is typed over it.
    SetFormText { field: usize, text: String },
    /// A form field's オプション: its bookmark, default, length, entries.
    SetFormFieldOptions {
        field: usize,
        options: FormFieldOptions,
    },
    /// フォームの保護: only the forms may be filled in.
    ProtectForms(bool),
    /// デザイン モード (`w:formsDesign`).
    DesignMode(bool),
}

/// A form field's オプション dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormFieldOptions {
    pub name: String,
    pub enabled: bool,
    pub default_text: String,
    pub max_length: usize,
    pub default_checked: bool,
    pub entries: Vec<String>,
    pub status_text: String,
}

impl FormFieldOptions {
    pub fn of(field: &LegacyFormField) -> Self {
        Self {
            name: field.name.clone(),
            enabled: field.enabled,
            default_text: field.default_text.clone(),
            max_length: field.max_length,
            default_checked: field.default_checked,
            entries: field.entries.clone(),
            status_text: field.status_text.clone().unwrap_or_default(),
        }
    }
}

impl ContentControlAction {
    fn name(&self) -> &'static str {
        match self {
            ContentControlAction::Insert { .. } => "insert_content_control",
            ContentControlAction::SetChecked { .. } => "set_content_control_checked",
            ContentControlAction::ChooseEntry { .. } => "choose_content_control_entry",
            ContentControlAction::SetText { .. } => "set_content_control_text",
            ContentControlAction::SetDate { .. } => "set_content_control_date",
            ContentControlAction::SetPicture { .. } => "set_content_control_picture",
            ContentControlAction::InsertBuildingBlock { .. } => "insert_building_block",
            ContentControlAction::AddRepeatingItem { .. } => "add_repeating_section_item",
            ContentControlAction::RemoveRepeatingItem { .. } => "remove_repeating_section_item",
            ContentControlAction::Remove { .. } => "remove_content_control",
            ContentControlAction::SetProperties { .. } => "set_content_control_properties",
            ContentControlAction::InsertFormField { .. } => "insert_form_field",
            ContentControlAction::SetFormCheckBox { .. } => "set_form_check_box",
            ContentControlAction::ChooseFormEntry { .. } => "choose_form_entry",
            ContentControlAction::SetFormText { .. } => "set_form_text",
            ContentControlAction::SetFormFieldOptions { .. } => "set_form_field_options",
            ContentControlAction::ProtectForms(_) => "protect_forms",
            ContentControlAction::DesignMode(_) => "forms_design_mode",
        }
    }

    /// The action as one undo step, for the selection `range` (rendered).
    pub fn command(self, range: SelectionRange) -> RewriteCommand {
        let name = self.name();
        RewriteCommand::new(name, move |xml, document| {
            document.apply_control_action(xml, &self, range)
        })
    }
}

impl WordDocument {
    /// The design mode switch (`w:formsDesign` in the settings).
    pub fn forms_design_mode(&self) -> bool {
        self.settings_xml_content().is_some_and(|settings| {
            DocxXmlDocument::parse(&settings).is_ok_and(|parsed| {
                parsed.root().children.iter().any(|node| {
                    matches!(node, XmlNode::Element(element)
                        if element.name == "w:formsDesign"
                            && !matches!(attribute(element, "w:val"), Some("0" | "false" | "off")))
                })
            })
        })
    }

    /// The building blocks of the document's glossary in `gallery`, by
    /// name (`w:docPart`, §17.12).
    pub fn building_blocks(&self, gallery: Option<&str>) -> Vec<String> {
        self.glossary_parts()
            .into_iter()
            .filter(|part| {
                gallery.is_none_or(|gallery| {
                    part.gallery.eq_ignore_ascii_case(gallery)
                        || (gallery.eq_ignore_ascii_case("Quick Parts")
                            && part
                                .gallery
                                .eq_ignore_ascii_case("docPartGalleryQuickParts"))
                })
            })
            .map(|part| part.name)
            .collect()
    }

    fn glossary_path(&self) -> Option<String> {
        let document_part = self.document_part.as_ref()?;
        let relationship = document_part
            .relationships
            .iter()
            .find(|relationship| relationship.r#type.ends_with("/glossaryDocument"))?;
        Some(Self::resolve_target_path(
            &document_part.path,
            &relationship.target,
        ))
    }

    fn glossary_parts(&self) -> Vec<GlossaryPart> {
        let Some(xml) = self
            .glossary_path()
            .and_then(|path| self.read_xml_part_content(&path))
        else {
            return Vec::new();
        };
        let Ok(parsed) = DocxXmlDocument::parse(&xml) else {
            return Vec::new();
        };
        let mut parts = Vec::new();
        let Some(doc_parts) = child(Some(parsed.root()), "w:docParts") else {
            return parts;
        };
        for node in &doc_parts.children {
            let XmlNode::Element(part) = node else {
                continue;
            };
            if part.name != "w:docPart" {
                continue;
            }
            let properties = child(Some(part), "w:docPartPr");
            let name = val(child(properties, "w:name")).unwrap_or_default();
            let category = child(properties, "w:category");
            let gallery = val(child(category, "w:gallery")).unwrap_or_default();
            let Some(body) = child(Some(part), "w:docPartBody") else {
                continue;
            };
            parts.push(GlossaryPart {
                name,
                gallery,
                body: body.children.clone(),
            });
        }
        parts
    }

    /// The id of the `Placeholder Text` character style, added to the
    /// styles part when the document has none.
    fn placeholder_style_id(&mut self) -> Option<String> {
        let path = "word/styles.xml";
        let styles = self.read_xml_part_content(path)?;
        let mut parsed = DocxXmlDocument::parse(&styles).ok()?;
        let mut default_character = None;
        for node in &parsed.root().children {
            let XmlNode::Element(style) = node else {
                continue;
            };
            if style.name != "w:style" || attribute(style, "w:type") != Some("character") {
                continue;
            }
            let id = attribute(style, "w:styleId").map(str::to_string);
            if val(child(Some(style), "w:name"))
                .is_some_and(|name| name.eq_ignore_ascii_case("Placeholder Text"))
            {
                return id;
            }
            if attribute(style, "w:default").is_some_and(|value| matches!(value, "1" | "true")) {
                default_character = id;
            }
        }
        let based_on = default_character
            .map(|id| format!("<w:basedOn w:val=\"{}\"/>", xml_escape(&id)))
            .unwrap_or_default();
        let style = xml_fragment(
            &mut parsed,
            &format!(
                "<w:style w:type=\"character\" w:styleId=\"PlaceholderText\"><w:name w:val=\"Placeholder Text\"/>{based_on}<w:uiPriority w:val=\"99\"/><w:semiHidden/><w:rPr><w:color w:val=\"666666\"/></w:rPr></w:style>"
            ),
        );
        let root_id = parsed.root().id;
        let root = parsed.element_mut(root_id)?;
        root.children.extend(style);
        parsed.rebuild_index();
        self.upsert_xml_part_content(path, parsed.to_string().ok()?);
        Some("PlaceholderText".to_string())
    }

    fn write_settings(
        &mut self,
        edit: impl FnOnce(&mut DocxXmlDocument, XmlNodeId),
    ) -> Result<(), String> {
        let previous = self
            .settings_xml_content()
            .ok_or_else(|| "settings.xml is not present in this package".to_string())?;
        let mut parsed = DocxXmlDocument::parse(&previous)
            .map_err(|error| format!("failed to parse settings.xml: {error:?}"))?;
        let root_id = parsed.root().id;
        edit(&mut parsed, root_id);
        parsed.rebuild_index();
        let written = parsed
            .to_string()
            .map_err(|error| format!("failed to write settings.xml: {error:?}"))?;
        self.restore_settings_xml(written);
        Ok(())
    }

    /// Runs one content-control action on `xml`, a copy of the document's.
    fn apply_control_action(
        &mut self,
        xml: &mut DocxXmlDocument,
        action: &ContentControlAction,
        range: SelectionRange,
    ) -> Result<CaretAfter, String> {
        let controls = self.content_controls();
        let control_of = |index: usize| -> Result<ContentControl, String> {
            controls
                .get(index)
                .cloned()
                .ok_or_else(|| "the content control is not in the document".to_string())
        };
        let sdt_id = |xml: &DocxXmlDocument, index: usize| -> Result<XmlNodeId, String> {
            Self::body_sdt_ids(xml)
                .get(index)
                .copied()
                .ok_or_else(|| "the content control is not in the document".to_string())
        };
        match action {
            ContentControlAction::Insert { kind, japanese } => {
                self.insert_control(xml, kind, *japanese, range)
            }
            ContentControlAction::SetChecked { control, checked } => {
                let current = control_of(*control)?;
                let properties = ContentControlProperties::of(&current);
                let symbol = if *checked {
                    properties.checked_symbol
                } else {
                    properties.unchecked_symbol
                };
                let id = sdt_id(xml, *control)?;
                let sdt = xml.element_mut(id).ok_or("missing content control")?;
                if let Some(checkbox) = child_mut(sdt_properties_mut(sdt), "w14:checkbox") {
                    set_child_attribute(
                        checkbox,
                        "w14:checked",
                        "w14:val",
                        if *checked { "1" } else { "0" },
                    );
                }
                set_content_text(sdt, &symbol.symbol.to_string(), Some(&symbol.font));
                Ok(CaretAfter::Keep)
            }
            ContentControlAction::ChooseEntry { control, entry } => {
                let current = control_of(*control)?;
                let entry = current
                    .entries
                    .get(*entry)
                    .cloned()
                    .ok_or("the list has no such entry")?;
                let id = sdt_id(xml, *control)?;
                let sdt = xml.element_mut(id).ok_or("missing content control")?;
                let is_prompt = entry.value.is_empty()
                    && current.entries.first().is_some_and(|first| *first == entry);
                if is_prompt {
                    // The prompt entry puts the placeholder back.
                    set_content_text(sdt, &entry.display, None);
                    set_showing_placeholder(sdt, true);
                } else {
                    set_content_text(sdt, &entry.display, None);
                    set_showing_placeholder(sdt, false);
                    clear_placeholder_style(sdt, current.style.as_deref());
                    if let Some(list) = child_mut(sdt_properties_mut(sdt), "w:comboBox") {
                        set_attribute(list, "w:lastValue", &entry.value);
                    }
                    if let Some(list) = child_mut(sdt_properties_mut(sdt), "w:dropDownList") {
                        set_attribute(list, "w:lastValue", &entry.value);
                    }
                }
                Ok(CaretAfter::AfterControl(*control))
            }
            ContentControlAction::SetText { control, text } => {
                let current = control_of(*control)?;
                let id = sdt_id(xml, *control)?;
                let sdt = xml.element_mut(id).ok_or("missing content control")?;
                if current.block {
                    set_block_text(sdt, text);
                } else {
                    set_content_text(sdt, text, None);
                }
                set_showing_placeholder(sdt, false);
                clear_placeholder_style(sdt, current.style.as_deref());
                if current.temporary {
                    unwrap_sdt(xml, id);
                    let end = current.start.offset + text.len();
                    return Ok(CaretAfter::At(self.rendered_to_model_text_position(
                        TextPosition::new(current.start.paragraph, end),
                    )));
                }
                Ok(CaretAfter::AfterControl(*control))
            }
            ContentControlAction::SetDate { control, date } => {
                let current = control_of(*control)?;
                let format = current
                    .date_format
                    .clone()
                    .unwrap_or_else(|| DATE_FORMATS[0].to_string());
                let stamp = format!("{date}T00:00:00Z");
                let value = jade_docx_reader::field_datetime::parse_core_property_datetime(&stamp)
                    .ok_or("the date is not valid")?;
                let text = jade_docx_reader::field_datetime::format_word_date(&format, &value);
                let id = sdt_id(xml, *control)?;
                let sdt = xml.element_mut(id).ok_or("missing content control")?;
                if let Some(element) = child_mut(sdt_properties_mut(sdt), "w:date") {
                    set_attribute(element, "w:fullDate", &stamp);
                }
                set_content_text(sdt, &text, None);
                set_showing_placeholder(sdt, false);
                clear_placeholder_style(sdt, current.style.as_deref());
                Ok(CaretAfter::AfterControl(*control))
            }
            ContentControlAction::SetPicture {
                control,
                bytes,
                mime,
            } => {
                let mime = InlineImageMime::parse(mime)?;
                if !mime.matches_signature(bytes) {
                    return Err("the picture is not in the format it says".to_string());
                }
                let relationship = self.add_inline_image_media_part(bytes, mime)?;
                let (width, height) = picture_extent(bytes, 1_905_000);
                let id = sdt_id(xml, *control)?;
                let docpr = xml.allocate_drawing_object_id();
                let drawing = picture_run_xml(&relationship.id, docpr, width, height);
                let nodes = xml_fragment(xml, &drawing);
                let sdt = xml.element_mut(id).ok_or("missing content control")?;
                replace_content_runs(sdt, nodes);
                set_showing_placeholder(sdt, false);
                Ok(CaretAfter::AfterControl(*control))
            }
            ContentControlAction::InsertBuildingBlock { control, name } => {
                let part = self
                    .glossary_parts()
                    .into_iter()
                    .find(|part| part.name == *name)
                    .ok_or("the building block is not in the document")?;
                let id = sdt_id(xml, *control)?;
                let blocks: Vec<XmlNode> = part
                    .body
                    .iter()
                    .filter(|node| {
                        matches!(node, XmlNode::Element(element)
                            if element.name == "w:p" || element.name == "w:tbl")
                    })
                    .map(|node| match node {
                        XmlNode::Element(element) => XmlNode::Element(xml.create_element(
                            element.name.clone(),
                            element.attributes.clone(),
                            element.children.clone(),
                        )),
                        other => other.clone(),
                    })
                    .collect();
                let sdt = xml.element_mut(id).ok_or("missing content control")?;
                if let Some(content) = child_mut(Some(sdt), "w:sdtContent") {
                    content.children = blocks;
                }
                set_showing_placeholder(sdt, false);
                Ok(CaretAfter::Keep)
            }
            ContentControlAction::AddRepeatingItem { control } => {
                let current = control_of(*control)?;
                let item = if current.kind == ContentControlKind::RepeatingSectionItem {
                    current.index
                } else {
                    controls
                        .iter()
                        .rev()
                        .find(|candidate| {
                            candidate.parent == Some(current.index)
                                && candidate.kind == ContentControlKind::RepeatingSectionItem
                        })
                        .map(|candidate| candidate.index)
                        .ok_or("the repeating section has no item")?
                };
                let id = sdt_id(xml, item)?;
                let parent = xml.parent_id(id).ok_or("missing repeating section")?;
                let mut copy = xml.element(id).ok_or("missing item")?.clone();
                let mut next_id = next_sdt_id(xml);
                renumber_sdt_ids(&mut copy, &mut next_id);
                let copy = xml.create_element(copy.name, copy.attributes, copy.children);
                let parent = xml.element_mut(parent).ok_or("missing repeating section")?;
                let at = child_index(parent, id).ok_or("missing item")?;
                parent.children.insert(at + 1, XmlNode::Element(copy));
                // The new item comes right after the copied one and its
                // controls, in document order.
                let nested = controls
                    .iter()
                    .filter(|candidate| is_inside(&controls, candidate.index, item))
                    .count();
                Ok(CaretAfter::SelectControl(item + nested + 1))
            }
            ContentControlAction::RemoveRepeatingItem { control } => {
                let current = control_of(*control)?;
                if current.kind != ContentControlKind::RepeatingSectionItem {
                    return Err("the caret is not in a repeating section item".to_string());
                }
                let siblings = controls
                    .iter()
                    .filter(|candidate| {
                        candidate.parent == current.parent
                            && candidate.kind == ContentControlKind::RepeatingSectionItem
                    })
                    .count();
                if siblings <= 1 {
                    return Err("a repeating section keeps one item".to_string());
                }
                let id = sdt_id(xml, *control)?;
                let parent = xml.parent_id(id).ok_or("missing repeating section")?;
                let parent = xml.element_mut(parent).ok_or("missing repeating section")?;
                let at = child_index(parent, id).ok_or("missing item")?;
                parent.children.remove(at);
                Ok(CaretAfter::Keep)
            }
            ContentControlAction::Remove { control } => {
                let current = control_of(*control)?;
                let id = sdt_id(xml, *control)?;
                if current.showing_placeholder {
                    // A placeholder is not content: it goes with the control.
                    let parent = xml.parent_id(id).ok_or("missing content control")?;
                    let parent = xml.element_mut(parent).ok_or("missing content control")?;
                    let at = child_index(parent, id).ok_or("missing content control")?;
                    parent.children.remove(at);
                } else {
                    unwrap_sdt(xml, id);
                }
                Ok(CaretAfter::At(
                    self.rendered_to_model_text_position(current.start),
                ))
            }
            ContentControlAction::SetProperties {
                control,
                properties,
            } => {
                let current = control_of(*control)?;
                let id = sdt_id(xml, *control)?;
                let style = properties.style.clone();
                let sdt_properties = sdt_properties_xml(
                    &current.kind,
                    properties,
                    current.id.unwrap_or_else(|| next_sdt_id(xml)),
                    current.showing_placeholder,
                    current.full_date.as_deref(),
                    current.checked,
                    current.multi_line != properties.multi_line,
                );
                let nodes = xml_fragment(xml, &sdt_properties);
                let sdt = xml.element_mut(id).ok_or("missing content control")?;
                // What the dialog does not know of — a data binding, a
                // placeholder's building block — stays.
                let kept: Vec<XmlNode> = sdt_properties_mut(sdt)
                    .map(|element| {
                        element
                            .children
                            .iter()
                            .filter(|node| {
                                matches!(node, XmlNode::Element(element)
                                    if matches!(element.name.as_str(),
                                        "w:placeholder" | "w:dataBinding" | "w15:dataBinding"
                                        | "w:label" | "w:tabIndex" | "w:equation"
                                        | "w:citation" | "w:bibliography"))
                            })
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                if let (Some(element), Some(XmlNode::Element(new_properties))) =
                    (sdt_properties_mut(sdt), nodes.into_iter().next())
                {
                    element.children = new_properties.children;
                    for node in kept {
                        let XmlNode::Element(kept_element) = &node else {
                            continue;
                        };
                        // The placeholder's building block goes where the
                        // schema puts it: after the lock.
                        let at = element
                            .children
                            .iter()
                            .position(|node| {
                                matches!(node, XmlNode::Element(element)
                                    if !matches!(element.name.as_str(),
                                        "w:rPr" | "w:alias" | "w:tag" | "w:id" | "w:lock"))
                            })
                            .unwrap_or(element.children.len());
                        if kept_element.name == "w:placeholder" {
                            element.children.insert(at, node);
                        } else {
                            element.children.push(node);
                        }
                    }
                }
                if let Some(style) = style {
                    set_content_style(sdt, &style);
                }
                Ok(CaretAfter::Keep)
            }
            ContentControlAction::InsertFormField { kind } => {
                self.insert_form_field(xml, *kind, range)
            }
            ContentControlAction::SetFormCheckBox { field, checked } => {
                let data = form_data_mut(xml, *field).ok_or("the form field is not there")?;
                if let Some(check) = child_mut(Some(data), "w:checkBox") {
                    set_child_attribute(
                        check,
                        "w:checked",
                        "w:val",
                        if *checked { "1" } else { "0" },
                    );
                }
                Ok(CaretAfter::Keep)
            }
            ContentControlAction::ChooseFormEntry { field, entry } => {
                let fields = self.legacy_form_fields();
                let current = fields.get(*field).ok_or("the form field is not there")?;
                let text = current
                    .entries
                    .get(*entry)
                    .cloned()
                    .ok_or("the list has no such entry")?;
                {
                    let data = form_data_mut(xml, *field).ok_or("the form field is not there")?;
                    if let Some(list) = child_mut(Some(data), "w:ddList") {
                        set_child_attribute(list, "w:result", "w:val", &entry.to_string());
                    }
                }
                set_form_result(xml, *field, &text);
                Ok(CaretAfter::Keep)
            }
            ContentControlAction::SetFormText { field, text } => {
                let fields = self.legacy_form_fields();
                let current = fields.get(*field).ok_or("the form field is not there")?;
                if current.kind != FormFieldKind::Text {
                    return Err("the form field is not a text box".to_string());
                }
                let text = if current.max_length > 0 {
                    text.chars().take(current.max_length).collect::<String>()
                } else {
                    text.clone()
                };
                let start = self.rendered_to_model_text_position(current.result_start);
                set_form_result(xml, *field, &text);
                Ok(CaretAfter::At(TextPosition::new(
                    start.paragraph,
                    start.offset + text.len(),
                )))
            }
            ContentControlAction::SetFormFieldOptions { field, options } => {
                let fields = self.legacy_form_fields();
                let current = fields
                    .get(*field)
                    .ok_or("the form field is not there")?
                    .clone();
                let data_xml =
                    form_data_xml(current.kind, options, current.selected, current.checked);
                let nodes = xml_fragment(xml, &data_xml);
                let data = form_data_mut(xml, *field).ok_or("the form field is not there")?;
                if let Some(XmlNode::Element(new_data)) = nodes.into_iter().next() {
                    data.children = new_data.children;
                }
                let shown = match current.kind {
                    FormFieldKind::Text
                        if !options.default_text.is_empty()
                            && (current.result == current.default_text
                                || current.result == EMPTY_TEXT_FIELD) =>
                    {
                        Some(options.default_text.clone())
                    }
                    FormFieldKind::DropDown => options.entries.get(current.selected).cloned(),
                    _ => None,
                };
                if let Some(text) = shown {
                    set_form_result(xml, *field, &text);
                }
                Ok(CaretAfter::Keep)
            }
            ContentControlAction::ProtectForms(on) => {
                let on = *on;
                self.write_settings(|settings, root_id| {
                    let protection = on.then(|| {
                        xml_fragment(
                            settings,
                            "<w:documentProtection w:edit=\"forms\" w:enforcement=\"1\"/>",
                        )
                    });
                    if let Some(root) = settings.element_mut(root_id) {
                        root.children.retain(|node| {
                            !matches!(node, XmlNode::Element(element)
                                if element.name == "w:documentProtection")
                        });
                        if let Some(protection) = protection {
                            root.children.extend(protection);
                        }
                    }
                })?;
                Ok(CaretAfter::Keep)
            }
            ContentControlAction::DesignMode(on) => {
                let on = *on;
                self.write_settings(|settings, root_id| {
                    let element = on.then(|| xml_fragment(settings, "<w:formsDesign/>"));
                    if let Some(root) = settings.element_mut(root_id) {
                        root.children.retain(|node| {
                            !matches!(node, XmlNode::Element(element)
                                if element.name == "w:formsDesign")
                        });
                        if let Some(element) = element {
                            root.children.extend(element);
                        }
                    }
                })?;
                Ok(CaretAfter::Keep)
            }
        }
    }

    fn insert_control(
        &mut self,
        xml: &mut DocxXmlDocument,
        kind: &ContentControlKind,
        japanese: bool,
        range: SelectionRange,
    ) -> Result<CaretAfter, String> {
        let (start, end) = range.normalized();
        let start = self.rendered_to_model_text_position(start);
        let end = self.rendered_to_model_text_position(end);
        let properties = ContentControlProperties::new_for(kind, japanese);
        let id = next_sdt_id(xml);
        let places = self.model_places();
        let spans_paragraphs = start.paragraph != end.paragraph;
        let block = matches!(
            kind,
            ContentControlKind::RepeatingSection | ContentControlKind::BuildingBlockGallery
        ) || (spans_paragraphs
            && matches!(
                kind,
                ContentControlKind::RichText | ContentControlKind::Group
            ));
        let index_before = |xml: &DocxXmlDocument, id: XmlNodeId| {
            Self::body_sdt_ids(xml)
                .iter()
                .position(|candidate| *candidate == id)
        };

        if block {
            let first = self
                .paragraph(start.paragraph)
                .and_then(|paragraph| paragraph.node_id)
                .ok_or("the selection is not in the body")?;
            let last = self
                .paragraph(end.paragraph)
                .and_then(|paragraph| paragraph.node_id)
                .ok_or("the selection is not in the body")?;
            // A repeating section in a table repeats its rows.
            let in_rows = *kind == ContentControlKind::RepeatingSection
                && ancestor_named(xml, first, "w:tr").is_some();
            let (first, last) = if in_rows {
                (
                    ancestor_named(xml, first, "w:tr").ok_or("no row")?,
                    ancestor_named(xml, last, "w:tr").ok_or("no row")?,
                )
            } else {
                (first, last)
            };
            let parent = xml.parent_id(first).ok_or("no parent")?;
            if xml.parent_id(last) != Some(parent) {
                return Err(
                    "コンテンツ コントロールは同じ場所にある段落を囲む必要があります。".to_string(),
                );
            }
            let wrapped_text = start != end || *kind != ContentControlKind::BuildingBlockGallery;
            let placeholder_style = if wrapped_text {
                None
            } else {
                self.placeholder_style_id()
            };
            let mut inner_properties = None;
            let outer_kind = kind.clone();
            if *kind == ContentControlKind::RepeatingSection {
                inner_properties = Some(sdt_properties_xml(
                    &ContentControlKind::RepeatingSectionItem,
                    &ContentControlProperties::new_for(
                        &ContentControlKind::RepeatingSectionItem,
                        japanese,
                    ),
                    id + 1,
                    false,
                    None,
                    false,
                    false,
                ));
            }
            let outer = sdt_properties_xml(
                &outer_kind,
                &properties,
                id,
                !wrapped_text,
                None,
                false,
                false,
            );
            let parent_element = xml.element(parent).ok_or("no parent")?;
            let from = child_index(parent_element, first).ok_or("no paragraph")?;
            let to = child_index(parent_element, last).ok_or("no paragraph")?;
            let mut content: Vec<XmlNode> = {
                let parent_element = xml.element_mut(parent).ok_or("no parent")?;
                parent_element.children.drain(from..=to).collect()
            };
            if !wrapped_text {
                // An empty gallery: one paragraph with the placeholder.
                let text = default_placeholder(kind, japanese).unwrap_or_default();
                content = xml_fragment(
                    xml,
                    &placeholder_paragraph_xml(text, placeholder_style.as_deref()),
                );
            }
            let content_xml = if let Some(inner) = inner_properties {
                let inner_sdt = build_sdt(xml, &inner, content);
                vec![XmlNode::Element(inner_sdt)]
            } else {
                content
            };
            let sdt = build_sdt(xml, &outer, content_xml);
            let sdt_id = sdt.id;
            let parent_element = xml.element_mut(parent).ok_or("no parent")?;
            parent_element.children.insert(from, XmlNode::Element(sdt));
            xml.rebuild_index();
            let index = index_before(xml, sdt_id).ok_or("the control was not placed")?;
            return Ok(if *kind == ContentControlKind::RepeatingSection {
                CaretAfter::Keep
            } else {
                CaretAfter::SelectControl(index)
            });
        }

        if spans_paragraphs {
            return Err(
                "このコンテンツ コントロールは 1 つの段落の中に挿入してください。".to_string(),
            );
        }
        let paragraph_id = self
            .paragraph(start.paragraph)
            .and_then(|paragraph| paragraph.node_id)
            .ok_or("the caret is not in a paragraph of the body")?;
        let collapsed = start == end
            || matches!(
                kind,
                ContentControlKind::CheckBox | ContentControlKind::Picture
            );
        let mut runs = ParagraphRuns::new(xml, &places, paragraph_id, start.paragraph);
        let start_mark = split_at(xml, &mut runs, start.offset)?;
        let end_mark = if collapsed {
            None
        } else {
            Some(split_at(xml, &mut runs, end.offset)?)
        };
        let (parent, from) = start_mark.location(xml).ok_or("no place for the control")?;
        let to = match &end_mark {
            Some(mark) => {
                let (end_parent, to) = mark.location(xml).ok_or("no place for the control")?;
                if end_parent != parent {
                    return Err(
                        "選択範囲の一部だけをコンテンツ コントロールで囲むことはできません。"
                            .to_string(),
                    );
                }
                to
            }
            None => from,
        };
        let placeholder = collapsed
            .then(|| default_placeholder(kind, japanese))
            .flatten();
        let placeholder_style = if placeholder.is_some() {
            self.placeholder_style_id()
        } else {
            None
        };
        let properties_xml = sdt_properties_xml(
            kind,
            &properties,
            id,
            placeholder.is_some(),
            None,
            false,
            false,
        );
        let content: Vec<XmlNode> = match kind {
            ContentControlKind::CheckBox => xml_fragment(
                xml,
                &format!(
                    "<w:r><w:rPr><w:rFonts w:ascii=\"{CHECK_FONT}\" w:eastAsia=\"{CHECK_FONT}\" w:hAnsi=\"{CHECK_FONT}\" w:hint=\"eastAsia\"/></w:rPr><w:t>{}</w:t></w:r>",
                    properties.unchecked_symbol.symbol
                ),
            ),
            ContentControlKind::Picture => {
                let bytes = placeholder_picture_png();
                let relationship = self.add_inline_image_media_part(
                    &bytes,
                    InlineImageMime::parse("image/png")?,
                )?;
                let docpr = xml.allocate_drawing_object_id();
                xml_fragment(
                    xml,
                    &picture_run_xml(&relationship.id, docpr, 1_905_000, 1_905_000),
                )
            }
            _ if collapsed => {
                let text = placeholder.unwrap_or_default();
                let style = placeholder_style
                    .as_deref()
                    .map(|style| format!("<w:rPr><w:rStyle w:val=\"{}\"/></w:rPr>", xml_escape(style)))
                    .unwrap_or_default();
                xml_fragment(
                    xml,
                    &format!(
                        "<w:r>{style}<w:t xml:space=\"preserve\">{}</w:t></w:r>",
                        xml_escape(text)
                    ),
                )
            }
            _ => {
                let parent_element = xml.element_mut(parent).ok_or("no parent")?;
                parent_element.children.drain(from..to).collect()
            }
        };
        let sdt = build_sdt(xml, &properties_xml, content);
        let sdt_id = sdt.id;
        let parent_element = xml.element_mut(parent).ok_or("no parent")?;
        parent_element.children.insert(from, XmlNode::Element(sdt));
        xml.rebuild_index();
        let index = index_before(xml, sdt_id).ok_or("the control was not placed")?;
        Ok(match kind {
            ContentControlKind::CheckBox | ContentControlKind::Picture => {
                CaretAfter::AfterControl(index)
            }
            _ if placeholder.is_some() => CaretAfter::SelectControl(index),
            _ => CaretAfter::AfterControl(index),
        })
    }

    fn insert_form_field(
        &mut self,
        xml: &mut DocxXmlDocument,
        kind: FormFieldKind,
        range: SelectionRange,
    ) -> Result<CaretAfter, String> {
        let (start, _) = range.normalized();
        let start = self.rendered_to_model_text_position(start);
        let places = self.model_places();
        let paragraph_id = self
            .paragraph(start.paragraph)
            .and_then(|paragraph| paragraph.node_id)
            .ok_or("the caret is not in a paragraph of the body")?;
        let mut runs = ParagraphRuns::new(xml, &places, paragraph_id, start.paragraph);
        let mark = split_at(xml, &mut runs, start.offset)?;
        let (parent, at) = mark.location(xml).ok_or("no place for the field")?;
        let existing = self.legacy_form_fields();
        let prefix = match kind {
            FormFieldKind::Text => "Text",
            FormFieldKind::CheckBox => "Check",
            FormFieldKind::DropDown => "Dropdown",
        };
        let number = (1..)
            .find(|number| {
                let name = format!("{prefix}{number}");
                !existing.iter().any(|field| field.name == name)
            })
            .unwrap_or(1);
        let options = FormFieldOptions {
            name: format!("{prefix}{number}"),
            enabled: true,
            default_text: String::new(),
            max_length: 0,
            default_checked: false,
            entries: Vec::new(),
            status_text: String::new(),
        };
        let data = form_data_xml(kind, &options, 0, false);
        let bookmark_id = next_bookmark_id(xml);
        let (instruction, result) = match kind {
            FormFieldKind::Text => ("FORMTEXT", Some(EMPTY_TEXT_FIELD)),
            FormFieldKind::CheckBox => ("FORMCHECKBOX", None),
            FormFieldKind::DropDown => ("FORMDROPDOWN", Some(EMPTY_TEXT_FIELD)),
        };
        let result_xml = result
            .map(|text| {
                format!(
                    "<w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:rPr><w:noProof/></w:rPr><w:t xml:space=\"preserve\">{text}</w:t></w:r>"
                )
            })
            .unwrap_or_default();
        let fragment = format!(
            "<w:bookmarkStart w:id=\"{bookmark_id}\" w:name=\"{name}\"/><w:r><w:fldChar w:fldCharType=\"begin\">{data}</w:fldChar></w:r><w:r><w:instrText xml:space=\"preserve\"> {instruction} </w:instrText></w:r>{result_xml}<w:r><w:fldChar w:fldCharType=\"end\"/></w:r><w:bookmarkEnd w:id=\"{bookmark_id}\"/>",
            name = xml_escape(&options.name),
        );
        let nodes = xml_fragment(xml, &fragment);
        let count = nodes.len();
        let parent_element = xml.element_mut(parent).ok_or("no parent")?;
        for (offset, node) in nodes.into_iter().enumerate() {
            parent_element.children.insert(at + offset, node);
        }
        let _ = count;
        let caret_offset = start.offset
            + match kind {
                FormFieldKind::CheckBox => 0,
                _ => EMPTY_TEXT_FIELD.len(),
            };
        Ok(CaretAfter::At(TextPosition::new(
            start.paragraph,
            caret_offset,
        )))
    }
}

/// Where in the XML a model position of a paragraph is — an element and
/// the index among its children — splitting a run there when it falls in
/// one, and out of a field result or a control at their edges.
pub(crate) fn place_at(
    xml: &mut DocxXmlDocument,
    places: &ModelPlaces,
    paragraph_id: XmlNodeId,
    paragraph: usize,
    offset: usize,
) -> Result<(XmlNodeId, usize), String> {
    let mut runs = ParagraphRuns::new(xml, places, paragraph_id, paragraph);
    let mark = split_at(xml, &mut runs, offset)?;
    mark.location(xml)
        .ok_or_else(|| "no place in the paragraph".to_string())
}

/// The properties of the last run before `index` among `parent`'s
/// children, for new runs to take the format of the text before them.
pub(crate) fn run_properties_before(
    xml: &DocxXmlDocument,
    parent: XmlNodeId,
    index: usize,
) -> Option<XmlElement> {
    let parent = xml.element(parent)?;
    parent.children[..index.min(parent.children.len())]
        .iter()
        .rev()
        .find_map(|node| match node {
            XmlNode::Element(run) if run.name == "w:r" => {
                child(Some(run), "w:rPr").cloned().or(Some(XmlElement {
                    id: XmlNodeId::new(0),
                    name: "w:rPr".to_string(),
                    attributes: Vec::new(),
                    children: Vec::new(),
                }))
            }
            _ => None,
        })
        .filter(|properties| !properties.children.is_empty())
}

/// A glossary document's building block.
struct GlossaryPart {
    name: String,
    gallery: String,
    body: Vec<XmlNode>,
}

/// Where in the XML a text position falls: between two children of an
/// element, recorded by a neighbour so later splits do not move it.
enum Mark {
    /// Right after this element.
    After(XmlNodeId),
    /// Right before this element.
    Before(XmlNodeId),
    /// At the end of this element's children.
    End(XmlNodeId),
}

impl Mark {
    fn location(&self, xml: &DocxXmlDocument) -> Option<(XmlNodeId, usize)> {
        match *self {
            Mark::After(id) => {
                let parent = xml.parent_id(id)?;
                Some((parent, child_index(xml.element(parent)?, id)? + 1))
            }
            Mark::Before(id) => {
                let parent = xml.parent_id(id)?;
                Some((parent, child_index(xml.element(parent)?, id)?))
            }
            Mark::End(id) => Some((id, xml.element(id)?.children.len())),
        }
    }
}

fn child_index(parent: &XmlElement, id: XmlNodeId) -> Option<usize> {
    parent
        .children
        .iter()
        .position(|node| matches!(node, XmlNode::Element(element) if element.id == id))
}

fn ancestor_named(xml: &DocxXmlDocument, id: XmlNodeId, name: &str) -> Option<XmlNodeId> {
    let mut current = xml.parent_id(id);
    while let Some(candidate) = current {
        if xml.element(candidate)?.name == name {
            return Some(candidate);
        }
        current = xml.parent_id(candidate);
    }
    None
}

/// The run-level containers a position at their edge may be lifted out of.
fn liftable(name: &str) -> bool {
    matches!(
        name,
        "w:ins" | "w:del" | "w:moveTo" | "w:moveFrom" | "w:smartTag" | "w:customXml"
    )
}

/// The runs of one paragraph and their model offsets, kept up to date as
/// runs are split.
struct ParagraphRuns {
    paragraph_id: XmlNodeId,
    runs: Vec<(XmlNodeId, usize, usize)>,
}

impl ParagraphRuns {
    fn new(
        xml: &DocxXmlDocument,
        places: &ModelPlaces,
        paragraph_id: XmlNodeId,
        paragraph: usize,
    ) -> Self {
        let mut runs: Vec<(XmlNodeId, usize, usize)> = places
            .runs
            .iter()
            .filter(|(id, place)| {
                place.paragraph == paragraph
                    && xml
                        .element(**id)
                        .is_some_and(|element| element.name == "w:r")
            })
            .map(|(id, place)| (*id, place.start, place.end))
            .collect();
        runs.sort_by_key(|&(_, start, end)| (start, end));
        Self { paragraph_id, runs }
    }
}

/// The mark for a model position of a paragraph, splitting the run it
/// falls inside so the position is between two runs. A position at the
/// edge of a content control or a revision is outside it.
fn split_at(
    xml: &mut DocxXmlDocument,
    runs: &mut ParagraphRuns,
    offset: usize,
) -> Result<Mark, String> {
    let text_runs: Vec<(XmlNodeId, usize, usize)> = runs
        .runs
        .iter()
        .copied()
        .filter(|&(_, start, end)| end > start)
        .collect();
    let mark = if let Some((id, start, end)) = text_runs
        .iter()
        .copied()
        .find(|&(_, start, end)| start < offset && offset < end)
    {
        match split_run(xml, id, offset - start) {
            Some(tail) => {
                let at = runs
                    .runs
                    .iter()
                    .position(|&(candidate, _, _)| candidate == id)
                    .unwrap_or(0);
                runs.runs[at] = (id, start, offset);
                runs.runs.insert(at + 1, (tail, offset, end));
                Mark::After(id)
            }
            None => Mark::Before(id),
        }
    } else if let Some((id, _, _)) = text_runs.iter().copied().find(|&(_, _, end)| end == offset) {
        Mark::After(id)
    } else if let Some((id, _, _)) = text_runs
        .iter()
        .copied()
        .find(|&(_, start, _)| start == offset)
    {
        Mark::Before(id)
    } else {
        // An empty paragraph: after its properties and anything else.
        Mark::End(runs.paragraph_id)
    };
    let mark = lift(xml, mark, runs.paragraph_id);
    Ok(out_of_field_result(xml, mark))
}

/// The field character a run holds, if it is one: `begin`, `separate` or
/// `end`.
fn field_char(node: &XmlNode) -> Option<&str> {
    let XmlNode::Element(run) = node else {
        return None;
    };
    if run.name != "w:r" {
        return None;
    }
    child(Some(run), "w:fldChar").and_then(|element| attribute(element, "w:fldCharType"))
}

/// A mark at either end of a field's result moves out of the field, so
/// what is put there goes beside the field rather than into its result.
fn out_of_field_result(xml: &DocxXmlDocument, mark: Mark) -> Mark {
    let Some((parent, index)) = mark.location(xml) else {
        return mark;
    };
    let Some(element) = xml.element(parent) else {
        return mark;
    };
    let children = &element.children;
    let id_of = |node: &XmlNode| match node {
        XmlNode::Element(element) => Some(element.id),
        _ => None,
    };
    // At the end of a result: after the field's end.
    if children.get(index).and_then(field_char) == Some("end") {
        if let Some(id) = children.get(index).and_then(id_of) {
            return Mark::After(id);
        }
    }
    // At the start of a result: before the field's begin.
    if index > 0 && children.get(index - 1).and_then(field_char) == Some("separate") {
        let mut depth = 0usize;
        for at in (0..index - 1).rev() {
            match children.get(at).and_then(field_char) {
                Some("end") => depth += 1,
                Some("begin") if depth == 0 => {
                    if let Some(id) = children.get(at).and_then(id_of) {
                        return Mark::Before(id);
                    }
                }
                Some("begin") => depth -= 1,
                _ => {}
            }
        }
    }
    mark
}

/// A mark at the edge of a revision, a smart tag or a content control
/// moves out of it.
fn lift(xml: &DocxXmlDocument, mut mark: Mark, paragraph_id: XmlNodeId) -> Mark {
    loop {
        let Some((parent, index)) = mark.location(xml) else {
            return mark;
        };
        if parent == paragraph_id {
            return mark;
        }
        let Some(element) = xml.element(parent) else {
            return mark;
        };
        let at_start = index == 0;
        let at_end = index >= element.children.len();
        if element.name == "w:sdtContent" {
            let Some(sdt) = xml.parent_id(parent) else {
                return mark;
            };
            mark = if at_start {
                Mark::Before(sdt)
            } else if at_end {
                Mark::After(sdt)
            } else {
                return mark;
            };
            continue;
        }
        if !liftable(&element.name) {
            return mark;
        }
        if at_start {
            mark = Mark::Before(parent);
        } else if at_end {
            mark = Mark::After(parent);
        } else {
            return mark;
        }
    }
}

/// Splits a run of plain text at `at` bytes into it: the first part keeps
/// the element, the second is a copy of its properties with the rest; its
/// id is returned. `None` when the run holds more than text.
fn split_run(xml: &mut DocxXmlDocument, id: XmlNodeId, at: usize) -> Option<XmlNodeId> {
    let run = xml.element(id)?.clone();
    let mut properties = None;
    let mut text = String::new();
    for node in &run.children {
        match node {
            XmlNode::Element(element) if element.name == "w:rPr" => {
                properties = Some(element.clone());
            }
            XmlNode::Element(element) if element.name == "w:t" => {
                for node in &element.children {
                    if let XmlNode::Text(part) = node {
                        text.push_str(part);
                    }
                }
            }
            XmlNode::Element(_) => return None,
            _ => {}
        }
    }
    if at == 0 || at >= text.len() || !text.is_char_boundary(at) {
        return None;
    }
    let (head, tail) = text.split_at(at);
    let text_element = |xml: &mut DocxXmlDocument, text: &str| {
        XmlNode::Element(xml.create_element(
            "w:t",
            vec![XmlAttribute {
                name: "xml:space".to_string(),
                value: "preserve".to_string(),
            }],
            vec![XmlNode::Text(text.to_string())],
        ))
    };
    let mut first_children: Vec<XmlNode> = properties
        .clone()
        .map(XmlNode::Element)
        .into_iter()
        .collect();
    first_children.push(text_element(xml, head));
    let mut second_children: Vec<XmlNode> = properties
        .map(|properties| {
            XmlNode::Element(xml.create_element(
                properties.name,
                properties.attributes,
                properties.children,
            ))
        })
        .into_iter()
        .collect();
    second_children.push(text_element(xml, tail));
    let second = xml.create_element("w:r", run.attributes.clone(), second_children);
    let second_id = second.id;
    let parent_id = xml.parent_id(id)?;
    {
        let element = xml.element_mut(id)?;
        element.children = first_children;
    }
    let parent = xml.element_mut(parent_id)?;
    let index = child_index(parent, id)?;
    parent.children.insert(index + 1, XmlNode::Element(second));
    xml.rebuild_index();
    Some(second_id)
}

/// A `w:sdt` with `properties` (an `w:sdtPr` as text) around `content`.
fn build_sdt(xml: &mut DocxXmlDocument, properties: &str, content: Vec<XmlNode>) -> XmlElement {
    let mut children = xml_fragment(xml, properties);
    children.extend(xml_fragment(xml, "<w:sdtEndPr/>"));
    let content = xml.create_element_preserving_child_ids("w:sdtContent", Vec::new(), content);
    children.push(XmlNode::Element(content));
    xml.create_element_preserving_child_ids("w:sdt", Vec::new(), children)
}

/// An unused `w:id` for a content control.
fn next_sdt_id(xml: &DocxXmlDocument) -> i64 {
    fn walk(element: &XmlElement, maximum: &mut i64) {
        if element.name == "w:sdtPr" {
            if let Some(id) = child(Some(element), "w:id")
                .and_then(|id| attribute(id, "w:val"))
                .and_then(|value| value.parse::<i64>().ok())
            {
                *maximum = (*maximum).max(id);
            }
        }
        for node in &element.children {
            if let XmlNode::Element(child) = node {
                walk(child, maximum);
            }
        }
    }
    let mut maximum = 1_000_000_000i64;
    walk(xml.root(), &mut maximum);
    // `w:id` is an `ST_DecimalNumber`, which Word reads as a 32-bit int.
    if maximum >= i32::MAX as i64 - 16 {
        return 1;
    }
    maximum + 1
}

fn next_bookmark_id(xml: &DocxXmlDocument) -> i64 {
    fn walk(element: &XmlElement, maximum: &mut i64) {
        if element.name == "w:bookmarkStart" {
            if let Some(id) = attribute(element, "w:id").and_then(|value| value.parse::<i64>().ok())
            {
                *maximum = (*maximum).max(id);
            }
        }
        for node in &element.children {
            if let XmlNode::Element(child) = node {
                walk(child, maximum);
            }
        }
    }
    let mut maximum = -1;
    walk(xml.root(), &mut maximum);
    maximum + 1
}

fn renumber_sdt_ids(element: &mut XmlElement, next: &mut i64) {
    if element.name == "w:sdtPr" {
        for node in &mut element.children {
            if let XmlNode::Element(id) = node {
                if id.name == "w:id" {
                    set_attribute(id, "w:val", &next.to_string());
                    *next += 1;
                }
            }
        }
    }
    for node in &mut element.children {
        if let XmlNode::Element(child) = node {
            renumber_sdt_ids(child, next);
        }
    }
}

fn is_inside(controls: &[ContentControl], index: usize, ancestor: usize) -> bool {
    let mut current = controls.get(index).and_then(|control| control.parent);
    while let Some(parent) = current {
        if parent == ancestor {
            return true;
        }
        current = controls.get(parent).and_then(|control| control.parent);
    }
    false
}

/// The `w:sdtPr` of a control as Word writes it, in schema order.
fn sdt_properties_xml(
    kind: &ContentControlKind,
    properties: &ContentControlProperties,
    id: i64,
    showing_placeholder: bool,
    full_date: Option<&str>,
    checked: bool,
    _multi_line_changed: bool,
) -> String {
    let mut xml = String::from("<w:sdtPr>");
    if let Some(style) = &properties.style {
        xml.push_str(&format!(
            "<w:rPr><w:rStyle w:val=\"{}\"/></w:rPr>",
            xml_escape(style)
        ));
    }
    if !properties.title.is_empty() {
        xml.push_str(&format!(
            "<w:alias w:val=\"{}\"/>",
            xml_escape(&properties.title)
        ));
    }
    if !properties.tag.is_empty() {
        xml.push_str(&format!(
            "<w:tag w:val=\"{}\"/>",
            xml_escape(&properties.tag)
        ));
    }
    xml.push_str(&format!("<w:id w:val=\"{id}\"/>"));
    let lock = match (properties.cannot_delete, properties.cannot_edit) {
        (true, true) => Some("sdtContentLocked"),
        (true, false) => Some("sdtLocked"),
        (false, true) => Some("contentLocked"),
        (false, false) => None,
    };
    if let Some(lock) = lock {
        xml.push_str(&format!("<w:lock w:val=\"{lock}\"/>"));
    }
    if properties.temporary {
        xml.push_str("<w:temporary/>");
    }
    if showing_placeholder {
        xml.push_str("<w:showingPlcHdr/>");
    }
    let entries = |xml: &mut String| {
        for entry in &properties.entries {
            xml.push_str(&format!(
                "<w:listItem w:displayText=\"{}\" w:value=\"{}\"/>",
                xml_escape(&entry.display),
                xml_escape(&entry.value)
            ));
        }
    };
    match kind {
        ContentControlKind::RichText => {}
        ContentControlKind::PlainText => {
            if properties.multi_line {
                xml.push_str("<w:text w:multiLine=\"1\"/>");
            } else {
                xml.push_str("<w:text/>");
            }
        }
        ContentControlKind::Picture => xml.push_str("<w:picture/>"),
        ContentControlKind::ComboBox => {
            xml.push_str("<w:comboBox>");
            entries(&mut xml);
            xml.push_str("</w:comboBox>");
        }
        ContentControlKind::DropDownList => {
            xml.push_str("<w:dropDownList>");
            entries(&mut xml);
            xml.push_str("</w:dropDownList>");
        }
        ContentControlKind::Date => {
            match full_date {
                Some(date) => {
                    xml.push_str(&format!("<w:date w:fullDate=\"{}\">", xml_escape(date)))
                }
                None => xml.push_str("<w:date>"),
            }
            xml.push_str(&format!(
                "<w:dateFormat w:val=\"{}\"/><w:lid w:val=\"{}\"/><w:storeMappedDataAs w:val=\"dateTime\"/><w:calendar w:val=\"gregorian\"/></w:date>",
                xml_escape(&properties.date_format),
                xml_escape(&properties.date_language)
            ));
        }
        ContentControlKind::BuildingBlockGallery => {
            xml.push_str(&format!(
                "<w:docPartList><w:docPartGallery w:val=\"{}\"/>",
                xml_escape(&properties.gallery)
            ));
            if !properties.category.is_empty() {
                xml.push_str(&format!(
                    "<w:docPartCategory w:val=\"{}\"/>",
                    xml_escape(&properties.category)
                ));
            }
            xml.push_str("</w:docPartList>");
        }
        ContentControlKind::Group => xml.push_str("<w:group/>"),
        ContentControlKind::CheckBox
        | ContentControlKind::RepeatingSection
        | ContentControlKind::RepeatingSectionItem => {}
        ContentControlKind::Other(name) => xml.push_str(&format!("<w:{name}/>")),
    }
    if *kind == ContentControlKind::CheckBox {
        xml.push_str(&format!(
            "<w14:checkbox><w14:checked w14:val=\"{}\"/><w14:checkedState w14:val=\"{:04X}\" w14:font=\"{}\"/><w14:uncheckedState w14:val=\"{:04X}\" w14:font=\"{}\"/></w14:checkbox>",
            if checked { 1 } else { 0 },
            properties.checked_symbol.symbol as u32,
            xml_escape(&properties.checked_symbol.font),
            properties.unchecked_symbol.symbol as u32,
            xml_escape(&properties.unchecked_symbol.font),
        ));
    }
    if properties.appearance != ControlAppearance::BoundingBox {
        xml.push_str(&format!(
            "<w15:appearance w15:val=\"{}\"/>",
            properties.appearance.as_ooxml()
        ));
    }
    if let Some(color) = &properties.color {
        xml.push_str(&format!("<w15:color w:val=\"{}\"/>", xml_escape(color)));
    }
    match kind {
        ContentControlKind::RepeatingSection => {
            xml.push_str("<w15:repeatingSection>");
            if !properties.section_title.is_empty() {
                xml.push_str(&format!(
                    "<w15:sectionTitle w:val=\"{}\"/>",
                    xml_escape(&properties.section_title)
                ));
            }
            if properties.fixed_items {
                xml.push_str("<w15:doNotAllowInsertDeleteSection/>");
            }
            xml.push_str("</w15:repeatingSection>");
        }
        ContentControlKind::RepeatingSectionItem => xml.push_str("<w15:repeatingSectionItem/>"),
        _ => {}
    }
    xml.push_str("</w:sdtPr>");
    xml
}

fn placeholder_paragraph_xml(text: &str, style: Option<&str>) -> String {
    let style = style
        .map(|style| format!("<w:rPr><w:rStyle w:val=\"{}\"/></w:rPr>", xml_escape(style)))
        .unwrap_or_default();
    format!(
        "<w:p><w:r>{style}<w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
        xml_escape(text)
    )
}

fn sdt_properties_mut(sdt: &mut XmlElement) -> Option<&mut XmlElement> {
    child_mut(Some(sdt), "w:sdtPr")
}

fn child_mut<'a>(element: Option<&'a mut XmlElement>, name: &str) -> Option<&'a mut XmlElement> {
    element?.children.iter_mut().find_map(|node| match node {
        XmlNode::Element(child) if child.name == name => Some(child),
        _ => None,
    })
}

fn set_attribute(element: &mut XmlElement, name: &str, value: &str) {
    match element
        .attributes
        .iter_mut()
        .find(|attribute| attribute.name == name)
    {
        Some(attribute) => attribute.value = value.to_string(),
        None => element.attributes.push(XmlAttribute {
            name: name.to_string(),
            value: value.to_string(),
        }),
    }
}

/// Sets `attribute` on `element`'s child `name`, adding the child when it
/// is missing.
fn set_child_attribute(element: &mut XmlElement, name: &str, attribute_name: &str, value: &str) {
    if let Some(child) = child_mut(Some(element), name) {
        set_attribute(child, attribute_name, value);
        return;
    }
    element.children.push(XmlNode::Element(XmlElement {
        id: XmlNodeId::new(0),
        name: name.to_string(),
        attributes: vec![XmlAttribute {
            name: attribute_name.to_string(),
            value: value.to_string(),
        }],
        children: Vec::new(),
    }));
}

fn set_showing_placeholder(sdt: &mut XmlElement, showing: bool) {
    let Some(properties) = sdt_properties_mut(sdt) else {
        return;
    };
    properties.children.retain(
        |node| !matches!(node, XmlNode::Element(element) if element.name == "w:showingPlcHdr"),
    );
    if showing {
        // After the lock, placeholder and temporary; before the type.
        let at = properties
            .children
            .iter()
            .position(|node| {
                matches!(node, XmlNode::Element(element)
                    if !matches!(element.name.as_str(),
                        "w:rPr" | "w:alias" | "w:tag" | "w:id" | "w:lock" | "w:placeholder"
                        | "w:temporary"))
            })
            .unwrap_or(properties.children.len());
        properties.children.insert(
            at,
            XmlNode::Element(XmlElement {
                id: XmlNodeId::new(0),
                name: "w:showingPlcHdr".to_string(),
                attributes: Vec::new(),
                children: Vec::new(),
            }),
        );
    }
}

/// Calls `visit` with each run inside a control's content, wherever they
/// are nested (a nested control's runs too).
fn for_each_run_mut(element: &mut XmlElement, visit: &mut impl FnMut(&mut XmlElement)) {
    for node in &mut element.children {
        if let XmlNode::Element(child) = node {
            if child.name == "w:r" {
                visit(child);
            } else if child.name != "w:sdtPr" {
                for_each_run_mut(child, visit);
            }
        }
    }
}

fn is_placeholder_style(style: &str) -> bool {
    style.eq_ignore_ascii_case("PlaceholderText")
        || style == "a3"
        || style.to_ascii_lowercase().contains("placeholder")
}

/// Takes a placeholder's character style off the control's text.
fn clear_placeholder_style(sdt: &mut XmlElement, keep: Option<&str>) {
    let Some(content) = child_mut(Some(sdt), "w:sdtContent") else {
        return;
    };
    for_each_run_mut(content, &mut |run| {
        if let Some(properties) = child_mut(Some(run), "w:rPr") {
            properties.children.retain(|node| {
                !matches!(node, XmlNode::Element(element)
                    if element.name == "w:rStyle"
                        && attribute(element, "w:val")
                            .is_some_and(|style| Some(style) != keep && is_placeholder_style(style)))
            });
        }
    });
}

/// Gives the control's text the character style `style`.
fn set_content_style(sdt: &mut XmlElement, style: &str) {
    let Some(content) = child_mut(Some(sdt), "w:sdtContent") else {
        return;
    };
    for_each_run_mut(content, &mut |run| {
        if child_mut(Some(&mut *run), "w:rPr").is_none() {
            run.children.insert(0, bare_element("w:rPr", Vec::new()));
        }
        if let Some(properties) = child_mut(Some(run), "w:rPr") {
            properties.children.retain(
                |node| !matches!(node, XmlNode::Element(element) if element.name == "w:rStyle"),
            );
            properties.children.insert(
                0,
                bare_element(
                    "w:rStyle",
                    vec![XmlAttribute {
                        name: "w:val".to_string(),
                        value: style.to_string(),
                    }],
                ),
            );
        }
    });
}

fn bare_element(name: &str, attributes: Vec<XmlAttribute>) -> XmlNode {
    XmlNode::Element(XmlElement {
        id: XmlNodeId::new(0),
        name: name.to_string(),
        attributes,
        children: Vec::new(),
    })
}

/// The name a run to be taken out is given until it is.
const REMOVED_RUN: &str = "w:removed-run";

fn text_node(text: &str) -> XmlNode {
    XmlNode::Element(XmlElement {
        id: XmlNodeId::new(0),
        name: "w:t".to_string(),
        attributes: vec![XmlAttribute {
            name: "xml:space".to_string(),
            value: "preserve".to_string(),
        }],
        children: vec![XmlNode::Text(text.to_string())],
    })
}

/// Makes an inline control's content `text`, in its first run's format
/// (or a run in `font`, for a check box's symbol).
fn set_content_text(sdt: &mut XmlElement, text: &str, font: Option<&str>) {
    let Some(content) = child_mut(Some(sdt), "w:sdtContent") else {
        return;
    };
    let mut first = true;
    for_each_run_mut(content, &mut |run| {
        if first {
            first = false;
            run.children.retain(
                |node| matches!(node, XmlNode::Element(element) if element.name == "w:rPr"),
            );
            if let Some(font) = font {
                set_run_font(run, font);
            }
            run.children.push(text_node(text));
        } else {
            run.name = REMOVED_RUN.to_string();
        }
    });
    if first {
        let mut run = XmlElement {
            id: XmlNodeId::new(0),
            name: "w:r".to_string(),
            attributes: Vec::new(),
            children: vec![text_node(text)],
        };
        if let Some(font) = font {
            set_run_font(&mut run, font);
        }
        content.children.push(XmlNode::Element(run));
    } else {
        remove_marked_runs(content);
    }
}

fn set_run_font(run: &mut XmlElement, font: &str) {
    if child_mut(Some(&mut *run), "w:rPr").is_none() {
        run.children.insert(
            0,
            XmlNode::Element(XmlElement {
                id: XmlNodeId::new(0),
                name: "w:rPr".to_string(),
                attributes: Vec::new(),
                children: Vec::new(),
            }),
        );
    }
    if let Some(properties) = child_mut(Some(run), "w:rPr") {
        properties.children.retain(
            |node| !matches!(node, XmlNode::Element(element) if element.name == "w:rFonts"),
        );
        let attributes = ["w:ascii", "w:eastAsia", "w:hAnsi"]
            .into_iter()
            .map(|name| XmlAttribute {
                name: name.to_string(),
                value: font.to_string(),
            })
            .collect();
        let at = properties
            .children
            .iter()
            .position(
                |node| matches!(node, XmlNode::Element(element) if element.name != "w:rStyle"),
            )
            .unwrap_or(properties.children.len());
        properties.children.insert(
            at,
            XmlNode::Element(XmlElement {
                id: XmlNodeId::new(0),
                name: "w:rFonts".to_string(),
                attributes,
                children: Vec::new(),
            }),
        );
    }
}

/// Removes the runs marked [`REMOVED_RUN`], and the wrappers they leave
/// empty.
fn remove_marked_runs(element: &mut XmlElement) {
    element.children.retain_mut(|node| match node {
        XmlNode::Element(child) if child.name == REMOVED_RUN => false,
        XmlNode::Element(child) if child.name == "w:r" || child.name == "w:sdtPr" => true,
        XmlNode::Element(child) => {
            let had_runs = has_runs(child);
            remove_marked_runs(child);
            !had_runs || has_runs(child)
        }
        _ => true,
    });
}

fn has_runs(element: &XmlElement) -> bool {
    element.children.iter().any(|node| match node {
        XmlNode::Element(child) => child.name == "w:r" || has_runs(child),
        _ => false,
    })
}

/// Replaces an inline control's runs with `nodes`.
fn replace_content_runs(sdt: &mut XmlElement, nodes: Vec<XmlNode>) {
    let Some(content) = child_mut(Some(sdt), "w:sdtContent") else {
        return;
    };
    content
        .children
        .retain(|node| !matches!(node, XmlNode::Element(element) if element.name == "w:r"));
    content.children.extend(nodes);
}

/// Makes a block control's content one paragraph of `text` per line, in
/// the format of its first paragraph.
fn set_block_text(sdt: &mut XmlElement, text: &str) {
    let Some(content) = child_mut(Some(sdt), "w:sdtContent") else {
        return;
    };
    let template = content.children.iter().find_map(|node| match node {
        XmlNode::Element(element) if element.name == "w:p" => Some(element.clone()),
        _ => None,
    });
    let paragraph_properties = template
        .as_ref()
        .and_then(|paragraph| child(Some(paragraph), "w:pPr").cloned());
    let run_properties = template.as_ref().and_then(|paragraph| {
        paragraph.children.iter().find_map(|node| match node {
            XmlNode::Element(run) if run.name == "w:r" => child(Some(run), "w:rPr").cloned(),
            _ => None,
        })
    });
    content.children = text
        .split('\n')
        .map(|line| {
            let mut run_children: Vec<XmlNode> = run_properties
                .clone()
                .map(XmlNode::Element)
                .into_iter()
                .collect();
            run_children.push(text_node(line));
            let mut children: Vec<XmlNode> = paragraph_properties
                .clone()
                .map(XmlNode::Element)
                .into_iter()
                .collect();
            children.push(XmlNode::Element(XmlElement {
                id: XmlNodeId::new(0),
                name: "w:r".to_string(),
                attributes: Vec::new(),
                children: run_children,
            }));
            XmlNode::Element(XmlElement {
                id: XmlNodeId::new(0),
                name: "w:p".to_string(),
                attributes: Vec::new(),
                children,
            })
        })
        .collect();
}

/// Replaces a `w:sdt` with its content.
fn unwrap_sdt(xml: &mut DocxXmlDocument, id: XmlNodeId) {
    let Some(parent_id) = xml.parent_id(id) else {
        return;
    };
    let content = xml
        .element(id)
        .and_then(|sdt| child(Some(sdt), "w:sdtContent"))
        .map(|content| content.children.clone())
        .unwrap_or_default();
    let Some(parent) = xml.element_mut(parent_id) else {
        return;
    };
    let Some(at) = child_index(parent, id) else {
        return;
    };
    parent.children.splice(at..=at, content);
    xml.rebuild_index();
}

/// The `w:ffData` of the form field at `index`, in document order.
fn form_data_mut(xml: &mut DocxXmlDocument, index: usize) -> Option<&mut XmlElement> {
    let ids = form_field_data_ids(xml);
    let id = *ids.get(index)?;
    xml.element_mut(id)
}

/// The `w:ffData` of each legacy form field, in document order.
fn form_field_data_ids(xml: &DocxXmlDocument) -> Vec<XmlNodeId> {
    let mut ids = Vec::new();
    let mut pending: Option<XmlNodeId> = None;
    fn walk(element: &XmlElement, pending: &mut Option<XmlNodeId>, ids: &mut Vec<XmlNodeId>) {
        for node in &element.children {
            let XmlNode::Element(node) = node else {
                continue;
            };
            match node.name.as_str() {
                "w:fldChar" if attribute(node, "w:fldCharType") == Some("begin") => {
                    *pending = child(Some(node), "w:ffData").map(|data| data.id);
                }
                "w:instrText" => {
                    if let Some(id) = pending.take() {
                        let text: String = node
                            .children
                            .iter()
                            .filter_map(|node| match node {
                                XmlNode::Text(text) => Some(text.as_str()),
                                _ => None,
                            })
                            .collect();
                        let name = text.split_whitespace().next().unwrap_or_default();
                        if matches!(
                            name.to_ascii_uppercase().as_str(),
                            "FORMTEXT" | "FORMCHECKBOX" | "FORMDROPDOWN"
                        ) {
                            ids.push(id);
                        }
                    }
                }
                _ => walk(node, pending, ids),
            }
        }
    }
    walk(xml.root(), &mut pending, &mut ids);
    ids
}

/// Writes `text` as the result of the form field at `index`: the runs
/// between its `separate` and its `end`.
fn set_form_result(xml: &mut DocxXmlDocument, index: usize, text: &str) {
    let ids = form_field_data_ids(xml);
    let Some(&data_id) = ids.get(index) else {
        return;
    };
    // The run holding the begin, and its siblings after it.
    let Some(begin_char) = xml.parent_id(data_id) else {
        return;
    };
    let Some(begin_run) = xml.parent_id(begin_char) else {
        return;
    };
    let Some(parent_id) = xml.parent_id(begin_run) else {
        return;
    };
    let Some(parent) = xml.element_mut(parent_id) else {
        return;
    };
    let Some(start) = child_index(parent, begin_run) else {
        return;
    };
    let field_char = |node: &XmlNode| -> Option<String> {
        let XmlNode::Element(run) = node else {
            return None;
        };
        if run.name != "w:r" {
            return None;
        }
        child(Some(run), "w:fldChar")
            .and_then(|element| attribute(element, "w:fldCharType"))
            .map(str::to_string)
    };
    let mut depth = 0usize;
    let mut separate = None;
    let mut end = None;
    for (index, node) in parent.children.iter().enumerate().skip(start) {
        match field_char(node).as_deref() {
            Some("begin") => depth += 1,
            Some("separate") if depth == 1 => separate = Some(index),
            Some("end") => {
                depth -= 1;
                if depth == 0 {
                    end = Some(index);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(end) = end else {
        return;
    };
    let run_properties = separate.and_then(|separate| {
        parent.children[separate + 1..end]
            .iter()
            .find_map(|node| match node {
                XmlNode::Element(run) if run.name == "w:r" => child(Some(run), "w:rPr").cloned(),
                _ => None,
            })
    });
    let mut run_children: Vec<XmlNode> = run_properties.map(XmlNode::Element).into_iter().collect();
    run_children.push(text_node(text));
    let result = XmlNode::Element(XmlElement {
        id: XmlNodeId::new(0),
        name: "w:r".to_string(),
        attributes: Vec::new(),
        children: run_children,
    });
    match separate {
        Some(separate) => {
            parent.children.splice(separate + 1..end, [result]);
        }
        None => {
            let separator = XmlNode::Element(XmlElement {
                id: XmlNodeId::new(0),
                name: "w:r".to_string(),
                attributes: Vec::new(),
                children: vec![XmlNode::Element(XmlElement {
                    id: XmlNodeId::new(0),
                    name: "w:fldChar".to_string(),
                    attributes: vec![XmlAttribute {
                        name: "w:fldCharType".to_string(),
                        value: "separate".to_string(),
                    }],
                    children: Vec::new(),
                })],
            });
            parent.children.splice(end..end, [separator, result]);
        }
    }
}

/// A form field's `w:ffData`, in schema order.
fn form_data_xml(
    kind: FormFieldKind,
    options: &FormFieldOptions,
    selected: usize,
    checked: bool,
) -> String {
    let mut xml = format!(
        "<w:ffData><w:name w:val=\"{}\"/>",
        xml_escape(&options.name)
    );
    if options.enabled {
        xml.push_str("<w:enabled/>");
    } else {
        xml.push_str("<w:enabled w:val=\"0\"/>");
    }
    xml.push_str("<w:calcOnExit w:val=\"0\"/>");
    if !options.status_text.is_empty() {
        xml.push_str(&format!(
            "<w:statusText w:type=\"text\" w:val=\"{}\"/>",
            xml_escape(&options.status_text)
        ));
    }
    match kind {
        FormFieldKind::Text => {
            xml.push_str("<w:textInput>");
            if !options.default_text.is_empty() {
                xml.push_str(&format!(
                    "<w:default w:val=\"{}\"/>",
                    xml_escape(&options.default_text)
                ));
            }
            if options.max_length > 0 {
                xml.push_str(&format!("<w:maxLength w:val=\"{}\"/>", options.max_length));
            }
            xml.push_str("</w:textInput>");
        }
        FormFieldKind::CheckBox => {
            xml.push_str(&format!(
                "<w:checkBox><w:sizeAuto/><w:default w:val=\"{}\"/>{}</w:checkBox>",
                u8::from(options.default_checked),
                if checked != options.default_checked {
                    format!("<w:checked w:val=\"{}\"/>", u8::from(checked))
                } else {
                    String::new()
                }
            ));
        }
        FormFieldKind::DropDown => {
            xml.push_str("<w:ddList>");
            if selected > 0 {
                xml.push_str(&format!("<w:result w:val=\"{selected}\"/>"));
            }
            for entry in &options.entries {
                xml.push_str(&format!("<w:listEntry w:val=\"{}\"/>", xml_escape(entry)));
            }
            xml.push_str("</w:ddList>");
        }
    }
    xml.push_str("</w:ffData>");
    xml
}

/// A run with an inline picture of `width` by `height` EMUs.
fn picture_run_xml(relationship: &str, id: u32, width: i64, height: i64) -> String {
    format!(
        "<w:r><w:rPr><w:noProof/></w:rPr><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{width}\" cy=\"{height}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/><wp:docPr id=\"{id}\" name=\"Picture {id}\"/><wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect=\"1\"/></wp:cNvGraphicFramePr><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:pic><pic:nvPicPr><pic:cNvPr id=\"{id}\" name=\"Picture {id}\"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"{relationship}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{width}\" cy=\"{height}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"
    )
}

/// A picture's size at `width` EMUs wide, by its pixel aspect.
fn picture_extent(bytes: &[u8], width: i64) -> (i64, i64) {
    let pixels = png_size(bytes).or_else(|| jpeg_size(bytes));
    match pixels {
        Some((w, h)) if w > 0 && h > 0 => (width, width * h as i64 / w as i64),
        _ => (width, width),
    }
}

fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[1..4] != b"PNG" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((width, height))
}

fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut at = 2;
    while at + 9 < bytes.len() {
        if bytes[at] != 0xFF {
            at += 1;
            continue;
        }
        let marker = bytes[at + 1];
        let length = u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]) as usize;
        if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
            let height = u16::from_be_bytes([bytes[at + 5], bytes[at + 6]]) as u32;
            let width = u16::from_be_bytes([bytes[at + 7], bytes[at + 8]]) as u32;
            return Some((width, height));
        }
        at += 2 + length;
    }
    None
}

/// The picture a new 画像 control shows until one is chosen: a pale square
/// with a mountain and a sun, as Word's.
pub fn placeholder_picture_png() -> Vec<u8> {
    const SIZE: u32 = 96;
    let mut pixels = vec![0u8; (SIZE * SIZE * 3) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let at = ((y * SIZE + x) * 3) as usize;
            let (fx, fy) = (x as i32, y as i32);
            let border = x < 2 || y < 2 || x >= SIZE - 2 || y >= SIZE - 2;
            let sun = (fx - 66).pow(2) + (fy - 30).pow(2) <= 64;
            // Two hills: the base of the picture rising to peaks.
            let hill =
                fy > 78 - (24 - (fx - 34).abs()).max(0) || fy > 78 - (16 - (fx - 62).abs()).max(0);
            let inside = fy > 18 && fy < 80 && fx > 14 && fx < 82;
            let color: [u8; 3] = if border {
                [0xBF, 0xBF, 0xBF]
            } else if inside && sun {
                [0xA6, 0xA6, 0xA6]
            } else if inside && hill {
                [0x99, 0x99, 0x99]
            } else {
                [0xF2, 0xF2, 0xF2]
            };
            pixels[at..at + 3].copy_from_slice(&color);
        }
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, SIZE, SIZE);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        if let Ok(mut writer) = encoder.write_header() {
            let _ = writer.write_image_data(&pixels);
        }
    }
    bytes
}

/// The text a check box shows for `checked`, in Word's default symbols.
pub fn default_check_symbol(checked: bool) -> &'static str {
    check_box_glyph(checked)
}
