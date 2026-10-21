//! Content controls (`w:sdt`, ECMA-376 Part 1 §17.5.2) as Word has them:
//! what each one is — rich or plain text, a picture, a check box, a combo
//! box or drop-down list, a date picker, a building block gallery, a
//! repeating section, a group — where its content sits in the text, and
//! what may be done to it.
//!
//! The body model reads a content control's runs as ordinary text and keeps
//! the `w:sdt` wrapper in the round-trip XML, so this module reads the
//! controls from that XML and places them by the runs and paragraphs the
//! model linked to it. Changing a control — putting one in, ticking a box,
//! picking an entry, a date, its properties — rewrites the XML
//! ([`super::rewrite`]); typing in one goes through the ordinary text edits,
//! which [`WordDocument::edit_refusal`] guards: a control whose contents
//! cannot be edited, one that cannot be deleted, a group's fixed text, and a
//! document protected for filling in forms.

use super::rewrite::{xml_escape, xml_fragment, CaretAfter, RewriteCommand};
use super::*;

#[path = "content_controls/edit.rs"]
pub mod edit;
#[path = "content_controls/forms.rs"]
pub mod forms;

pub use edit::{
    ContentControlAction, ContentControlProperties, DateFormat, BUILDING_BLOCK_GALLERIES,
    DATE_FORMATS,
};
pub use forms::{FormFieldKind, LegacyFormField};

/// What a content control holds (§17.5.2.40 and the w14/w15 extensions).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentControlKind {
    /// リッチ テキスト: anything, formatted (no type element, or `w:richText`).
    RichText,
    /// テキスト: one run of unformatted text (`w:text`).
    PlainText,
    /// 画像 (`w:picture`).
    Picture,
    /// チェック ボックス (`w14:checkbox`).
    CheckBox,
    /// コンボ ボックス: a list, or anything typed (`w:comboBox`).
    ComboBox,
    /// ドロップダウン リスト: one of a list (`w:dropDownList`).
    DropDownList,
    /// 日付選択 (`w:date`).
    Date,
    /// 構成要素ギャラリー (`w:docPartList`, or `w:docPartObj` as Word writes
    /// for a cover page or a table of contents).
    BuildingBlockGallery,
    /// 繰り返しセクション (`w15:repeatingSection`) and one of its items
    /// (`w15:repeatingSectionItem`).
    RepeatingSection,
    RepeatingSectionItem,
    /// グループ (`w:group`): its text is fixed, the controls in it are not.
    Group,
    /// An equation, citation or bibliography Word wraps itself.
    Other(String),
}

impl ContentControlKind {
    /// Whether text is typed into it, rather than chosen or ticked.
    pub fn takes_typing(&self) -> bool {
        matches!(
            self,
            ContentControlKind::RichText
                | ContentControlKind::PlainText
                | ContentControlKind::ComboBox
                | ContentControlKind::Date
                | ContentControlKind::BuildingBlockGallery
                | ContentControlKind::RepeatingSectionItem
                | ContentControlKind::Other(_)
        )
    }
}

/// How Word shows a control (`w15:appearance`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ControlAppearance {
    /// 境界ボックス: a box round it when the caret is in it.
    #[default]
    BoundingBox,
    /// 開始/終了タグ.
    Tags,
    /// なし.
    Hidden,
}

impl ControlAppearance {
    pub fn as_ooxml(self) -> &'static str {
        match self {
            ControlAppearance::BoundingBox => "boundingBox",
            ControlAppearance::Tags => "tags",
            ControlAppearance::Hidden => "hidden",
        }
    }

    fn from_ooxml(value: &str) -> Self {
        match value {
            "tags" => ControlAppearance::Tags,
            "hidden" => ControlAppearance::Hidden,
            _ => ControlAppearance::BoundingBox,
        }
    }
}

/// An entry of a combo box or drop-down list (`w:listItem`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListEntry {
    /// 表示名.
    pub display: String,
    /// 値.
    pub value: String,
}

/// A check box's two symbols (`w14:checkedState`, `w14:uncheckedState`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckSymbol {
    pub symbol: char,
    pub font: String,
}

/// One content control, where it is and what it is.
#[derive(Clone, Debug, PartialEq)]
pub struct ContentControl {
    /// Its place among the document's content controls, in document order.
    pub index: usize,
    /// `w:id`.
    pub id: Option<i64>,
    pub kind: ContentControlKind,
    /// タイトル (`w:alias`).
    pub title: Option<String>,
    /// タグ (`w:tag`).
    pub tag: Option<String>,
    /// コンテンツ コントロールの削除不可 (`sdtLocked`).
    pub cannot_delete: bool,
    /// コンテンツの編集不可 (`contentLocked`).
    pub cannot_edit: bool,
    /// Whether its content is the placeholder (`w:showingPlcHdr`).
    pub showing_placeholder: bool,
    /// The placeholder's building block (`w:placeholder/w:docPart`).
    pub placeholder_part: Option<String>,
    /// 内容が編集されたら削除する (`w:temporary`).
    pub temporary: bool,
    /// 複数の段落を許可する (`w:text@w:multiLine`).
    pub multi_line: bool,
    pub appearance: ControlAppearance,
    /// 色 (`w15:color`), `RRGGBB`.
    pub color: Option<String>,
    /// スタイル: the character style its text is in (`w:rPr/w:rStyle`).
    pub style: Option<String>,
    /// A combo box's or drop-down list's entries.
    pub entries: Vec<ListEntry>,
    /// A date picker's 表示形式 (`w:dateFormat`), its language
    /// (`w:lid`) and the date chosen (`w:date@w:fullDate`).
    pub date_format: Option<String>,
    pub date_language: Option<String>,
    pub full_date: Option<String>,
    /// A check box's state and symbols.
    pub checked: bool,
    pub checked_symbol: Option<CheckSymbol>,
    pub unchecked_symbol: Option<CheckSymbol>,
    /// A building block gallery's ギャラリー and 分類.
    pub gallery: Option<String>,
    pub category: Option<String>,
    /// A repeating section's セクションのタイトル and whether its items may
    /// be added and removed.
    pub section_title: Option<String>,
    pub fixed_items: bool,
    /// Whether it wraps paragraphs (or rows, or cells) rather than runs.
    pub block: bool,
    /// Its content, in rendered text positions: the runs it wraps or, for a
    /// block control, the paragraphs from the start of the first to the end
    /// of the last.
    pub start: TextPosition,
    pub end: TextPosition,
    /// The text it holds, paragraphs joined with `\n`.
    pub text: String,
    /// How many controls it is inside, and the innermost of them.
    pub depth: usize,
    pub parent: Option<usize>,
}

impl ContentControl {
    /// Whether `position` is in it: inside its runs or at either end of
    /// them, or in one of its paragraphs.
    pub fn contains(&self, position: TextPosition) -> bool {
        self.start <= position && position <= self.end
    }

    /// What Word shows on its title tab: the title, or the kind's name.
    pub fn label(&self) -> Option<&str> {
        self.title.as_deref().filter(|title| !title.is_empty())
    }

    /// Whether a click on it is a choice — a box ticked, an entry or a date
    /// picked, a picture replaced — rather than a place to type.
    pub fn is_chooser(&self) -> bool {
        matches!(
            self.kind,
            ContentControlKind::CheckBox
                | ContentControlKind::ComboBox
                | ContentControlKind::DropDownList
                | ContentControlKind::Date
                | ContentControlKind::Picture
                | ContentControlKind::BuildingBlockGallery
        )
    }
}

/// Where a run of the body model sits: its paragraph and model offsets.
#[derive(Clone, Copy)]
struct RunPlace {
    paragraph: usize,
    start: usize,
    end: usize,
}

/// The XML ids of the body model's paragraphs and runs, and where each is.
pub(crate) struct ModelPlaces {
    runs: HashMap<XmlNodeId, RunPlace>,
    paragraphs: HashMap<XmlNodeId, usize>,
}

impl WordDocument {
    pub(crate) fn model_places(&self) -> ModelPlaces {
        let mut runs = HashMap::new();
        let mut paragraphs = HashMap::new();
        for index in 0..self.paragraph_count() {
            let Some(paragraph) = self.paragraph(index) else {
                continue;
            };
            if let Some(id) = paragraph.node_id {
                paragraphs.entry(id).or_insert(index);
            }
            let mut offset = 0usize;
            for run in &paragraph.runs {
                let end = offset + run.text.len();
                // A run of the file can be several of the model (a split
                // keeps its element): its place spans them all.
                for id in [run.node_id, run.revision_node_id].into_iter().flatten() {
                    runs.entry(id)
                        .and_modify(|place: &mut RunPlace| {
                            if place.paragraph == index {
                                place.start = place.start.min(offset);
                                place.end = place.end.max(end);
                            }
                        })
                        .or_insert(RunPlace {
                            paragraph: index,
                            start: offset,
                            end,
                        });
                }
                offset = end;
            }
        }
        ModelPlaces { runs, paragraphs }
    }

    /// The `w:sdt` elements of the body, in document order.
    pub(crate) fn body_sdt_ids(xml: &DocxXmlDocument) -> Vec<XmlNodeId> {
        fn walk(element: &XmlElement, out: &mut Vec<XmlNodeId>) {
            if element.name == "w:sdt" {
                out.push(element.id);
            }
            for child in &element.children {
                if let XmlNode::Element(child) = child {
                    walk(child, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(xml.root(), &mut out);
        out
    }

    /// Every content control of the body, in document order.
    pub fn content_controls(&self) -> Vec<ContentControl> {
        let Some(xml) = self.raw_document.as_ref() else {
            return Vec::new();
        };
        let places = self.model_places();
        let mut controls = Vec::new();
        let mut walker = ControlWalker {
            document: self,
            places: &places,
            controls: &mut controls,
            stack: Vec::new(),
            last_run_end: None,
        };
        walker.walk(xml.root(), None);
        controls
    }

    /// The innermost content control at `position` (rendered).
    pub fn content_control_at(&self, position: TextPosition) -> Option<ContentControl> {
        self.content_controls()
            .into_iter()
            .filter(|control| control.contains(position))
            .max_by_key(|control| control.depth)
    }

    /// Why an edit of the text from `start` to `end` (rendered positions;
    /// equal for an insertion) is not allowed, or `None` when it is.
    /// `breaks` is whether it puts in a paragraph break.
    pub fn edit_refusal(
        &self,
        start: TextPosition,
        end: TextPosition,
        breaks: bool,
    ) -> Option<&'static str> {
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let controls = self.content_controls();
        let protected_for_forms = self
            .document_protection()
            .is_some_and(|protection| protection.enforced && protection.edit == "forms");
        let insertion = start == end;
        // The control a position's typing lands in: an insertion at a
        // control's end goes into its last run, one at its start into the
        // run before it unless there is none.
        let lands_in = |control: &ContentControl, position: TextPosition| {
            if control.block {
                return control.contains(position);
            }
            (control.start < position && position <= control.end)
                || (position == control.start && control.start.offset == 0)
        };
        let innermost = |position: TextPosition| {
            controls
                .iter()
                .filter(|control| lands_in(control, position))
                .max_by_key(|control| control.depth)
        };
        for control in &controls {
            let touches = if insertion {
                lands_in(control, start)
            } else {
                start < control.end && end > control.start
            };
            if touches {
                let typed_into = innermost(start).is_some_and(|inner| inner.index == control.index);
                if control.cannot_edit {
                    return Some("このコンテンツ コントロールの内容は編集できません。");
                }
                if typed_into
                    && matches!(
                        control.kind,
                        ContentControlKind::CheckBox
                            | ContentControlKind::DropDownList
                            | ContentControlKind::Picture
                    )
                {
                    return Some("このコンテンツ コントロールには入力できません。");
                }
                if typed_into
                    && breaks
                    && control.kind == ContentControlKind::PlainText
                    && !control.multi_line
                {
                    return Some("このテキスト コンテンツ コントロールでは改行できません。");
                }
            }
            // The whole control and something beside it: the control would
            // go with the text.
            if control.cannot_delete
                && !insertion
                && start <= control.start
                && end >= control.end
                && (start < control.start || end > control.end)
            {
                return Some("このコンテンツ コントロールは削除できません。");
            }
        }
        // A group's own text is fixed; the controls in it are not.
        let in_group_text = |position: TextPosition| {
            let inner = innermost(position)?;
            (inner.kind == ContentControlKind::Group).then_some(())
        };
        if in_group_text(start).is_some() || (!insertion && in_group_text(end).is_some()) {
            return Some("グループ化されたコンテンツは編集できません。");
        }
        if protected_for_forms {
            let in_control = |position: TextPosition| {
                innermost(position).is_some_and(|control| {
                    control.kind != ContentControlKind::Group && !control.cannot_edit
                })
            };
            let fields = self.legacy_form_fields();
            let in_field = |position: TextPosition| {
                fields.iter().any(|field| {
                    field.kind == FormFieldKind::Text
                        && field.result_start <= position
                        && position <= field.result_end
                })
            };
            let allowed = |position: TextPosition| in_control(position) || in_field(position);
            let same_place = innermost(start).map(|control| control.index)
                == innermost(end).map(|control| control.index);
            if !(allowed(start) && allowed(end) && same_place) || breaks && !in_control(start) {
                return Some("文書はフォームへの入力のみ許可するよう保護されています。");
            }
        }
        None
    }

    /// Why a headless text edit may not be made — it would change a locked
    /// control, a group's text, or a document protected for forms — or
    /// `None` when it may. Only the edits typing makes are checked; the
    /// controls are read only when the paragraphs edited hold one, or the
    /// document is protected for forms.
    pub fn headless_edit_refusal(
        &self,
        command: &jade_docx_headless_api::HeadlessCommand,
    ) -> Option<&'static str> {
        use jade_docx_headless_api::{HeadlessCommand, HeadlessDeleteDirection};
        let (start, end, breaks) = match command {
            HeadlessCommand::InsertText {
                paragraph,
                offset,
                text,
            } => {
                let at = TextPosition::new(*paragraph, *offset);
                (at, at, text.contains('\n'))
            }
            HeadlessCommand::ReplaceRange {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                text,
            } => (
                TextPosition::new(*start_paragraph, *start_offset),
                TextPosition::new(*end_paragraph, *end_offset),
                text.contains('\n'),
            ),
            HeadlessCommand::InsertParagraphBreak {
                paragraph, offset, ..
            } => {
                let at = TextPosition::new(*paragraph, *offset);
                (at, at, true)
            }
            HeadlessCommand::DeleteText {
                paragraph,
                offset,
                direction,
                count,
            } => {
                let at = TextPosition::new(*paragraph, *offset);
                let other = self.step_characters(
                    at,
                    *count,
                    matches!(direction, HeadlessDeleteDirection::Backward),
                );
                (at.min(other), at.max(other), false)
            }
            _ => return None,
        };
        let forms = self.settings_mention_forms_protection();
        if !forms && !self.paragraphs_hold_controls(start.paragraph, end.paragraph) {
            return None;
        }
        self.edit_refusal(start, end, breaks)
    }

    /// The rendered position `count` characters before or after `at`,
    /// across a paragraph mark as one character.
    fn step_characters(&self, at: TextPosition, count: usize, backwards: bool) -> TextPosition {
        let mut position = at;
        for _ in 0..count.max(1) {
            let model = self.rendered_to_model_text_position(position);
            let text: String = self
                .paragraph(model.paragraph)
                .map(|paragraph| paragraph.runs.iter().map(|run| run.text.as_str()).collect())
                .unwrap_or_default();
            let offset = model.offset.min(text.len());
            let next = if backwards {
                if offset == 0 {
                    if model.paragraph == 0 {
                        return position;
                    }
                    let previous = model.paragraph - 1;
                    TextPosition::new(previous, self.paragraph_text_len_at(previous).unwrap_or(0))
                } else {
                    let step = text[..offset].chars().next_back().map_or(1, char::len_utf8);
                    TextPosition::new(model.paragraph, offset - step)
                }
            } else if offset >= text.len() {
                if model.paragraph + 1 >= self.paragraph_count() {
                    return position;
                }
                TextPosition::new(model.paragraph + 1, 0)
            } else {
                let step = text[offset..].chars().next().map_or(1, char::len_utf8);
                TextPosition::new(model.paragraph, offset + step)
            };
            position = self.model_to_rendered_text_position(next);
        }
        position
    }

    /// The content-control action a headless text edit amounts to when it
    /// replaces a control's whole placeholder, or a form text box's whole
    /// result (or types into its five spaces): the text goes in as the
    /// control's or field's content rather than beside it.
    pub fn placeholder_replacement(
        &self,
        command: &jade_docx_headless_api::HeadlessCommand,
    ) -> Option<ContentControlAction> {
        use jade_docx_headless_api::HeadlessCommand;
        let (start, end, text) = match command {
            HeadlessCommand::ReplaceRange {
                start_paragraph,
                start_offset,
                end_paragraph,
                end_offset,
                text,
            } => (
                TextPosition::new(*start_paragraph, *start_offset),
                TextPosition::new(*end_paragraph, *end_offset),
                text,
            ),
            HeadlessCommand::InsertText {
                paragraph,
                offset,
                text,
            } => {
                let at = TextPosition::new(*paragraph, *offset);
                (at, at, text)
            }
            _ => return None,
        };
        if text.contains('\n') || !self.paragraph_may_hold_controls(start.paragraph) {
            return None;
        }
        let (start, end) = (start.min(end), start.max(end));
        if let Some(field) = self.legacy_form_fields().into_iter().find(|field| {
            field.kind == FormFieldKind::Text
                && field.enabled
                && field.result_start <= start
                && end <= field.result_end
                && ((start == field.result_start && end == field.result_end && start != end)
                    || field.result == forms::EMPTY_TEXT_FIELD)
        }) {
            return Some(ContentControlAction::SetFormText {
                field: field.index,
                text: text.clone(),
            });
        }
        let control = self.content_control_at(start)?;
        (control.showing_placeholder
            && control.kind.takes_typing()
            && !control.cannot_edit
            && !control.block
            && control.start == start
            && control.end == end
            && start != end)
            .then(|| ContentControlAction::SetText {
                control: control.index,
                text: text.clone(),
            })
    }

    /// Whether a content control or a legacy form field may be in
    /// paragraph `index`: a cheap look before reading them all, for what
    /// runs on every keystroke.
    pub fn paragraph_may_hold_controls(&self, index: usize) -> bool {
        if self.paragraphs_hold_controls(index, index) {
            return true;
        }
        self.paragraph(index).is_some_and(|paragraph| {
            paragraph.runs.iter().any(|run| {
                run.field.as_ref().is_some_and(|field| {
                    matches!(field.kind.base_kind(), RunFieldKind::Unsupported { instruction }
                        if instruction.trim_start().to_ascii_uppercase().starts_with("FORM"))
                })
            })
        })
    }

    /// Whether settings.xml may protect the document for forms: a cheap
    /// look at the text before the settings are parsed.
    fn settings_mention_forms_protection(&self) -> bool {
        const SETTINGS_PATH: &str = "word/settings.xml";
        let mentions = |text: &str| text.contains("documentProtection") && text.contains("forms");
        if let Some(part) = self.parts.get(SETTINGS_PATH) {
            return mentions(&part.content) && self.protected_for_forms();
        }
        self.package
            .as_ref()
            .and_then(|package| package.files.get(SETTINGS_PATH))
            .is_some_and(|bytes| mentions(&String::from_utf8_lossy(bytes)))
            && self.protected_for_forms()
    }

    /// Whether a content control wraps or sits in any of the paragraphs
    /// from `first` to `last`: their XML and the elements around them.
    fn paragraphs_hold_controls(&self, first: usize, last: usize) -> bool {
        let Some(xml) = self.raw_document.as_ref() else {
            return false;
        };
        fn holds(element: &XmlElement) -> bool {
            element.children.iter().any(|node| match node {
                XmlNode::Element(child) => child.name == "w:sdt" || holds(child),
                _ => false,
            })
        }
        (first..=last.max(first)).any(|index| {
            let Some(id) = self
                .paragraph(index)
                .and_then(|paragraph| paragraph.node_id)
            else {
                return false;
            };
            if xml.element(id).is_some_and(holds) {
                return true;
            }
            let mut current = xml.parent_id(id);
            while let Some(ancestor) = current {
                if xml
                    .element(ancestor)
                    .is_some_and(|element| element.name == "w:sdt")
                {
                    return true;
                }
                current = xml.parent_id(ancestor);
            }
            false
        })
    }

    /// The rendered position of a model position, for a paragraph and an
    /// offset in it.
    fn rendered(&self, paragraph: usize, offset: usize) -> TextPosition {
        self.model_to_rendered_text_position(TextPosition::new(paragraph, offset))
    }
}

/// Walks the body XML in document order, reading each `w:sdt` and placing
/// it by the model runs and paragraphs inside it.
struct ControlWalker<'a> {
    document: &'a WordDocument,
    places: &'a ModelPlaces,
    controls: &'a mut Vec<ContentControl>,
    /// The controls the walk is inside, by their index.
    stack: Vec<usize>,
    /// The end of the last run passed in the paragraph being walked, for a
    /// control with no runs: it sits there.
    last_run_end: Option<(usize, usize)>,
}

impl ControlWalker<'_> {
    fn walk(&mut self, element: &XmlElement, paragraph: Option<usize>) {
        match element.name.as_str() {
            "w:p" => {
                let index = self.places.paragraphs.get(&element.id).copied();
                if let Some(index) = index {
                    self.last_run_end = Some((index, 0));
                }
                for child in &element.children {
                    if let XmlNode::Element(child) = child {
                        self.walk(child, index.or(paragraph));
                    }
                }
                return;
            }
            "w:r" => {
                if let Some(place) = self.places.runs.get(&element.id) {
                    self.last_run_end = Some((place.paragraph, place.end));
                }
                // A text box's paragraphs are inside a run.
                for child in &element.children {
                    if let XmlNode::Element(child) = child {
                        if child.name != "w:t" && child.name != "w:rPr" {
                            self.walk(child, paragraph);
                        }
                    }
                }
                return;
            }
            "w:sdt" => {
                self.read_sdt(element, paragraph);
                return;
            }
            _ => {}
        }
        for child in &element.children {
            if let XmlNode::Element(child) = child {
                self.walk(child, paragraph);
            }
        }
    }

    fn read_sdt(&mut self, sdt: &XmlElement, paragraph: Option<usize>) {
        let properties = child(Some(sdt), "w:sdtPr");
        let index = self.controls.len();
        let mut control = read_properties(properties, index);
        control.block = paragraph.is_none();
        control.depth = self.stack.len();
        control.parent = self.stack.last().copied();
        let before = self.last_run_end;
        self.controls.push(control);
        self.stack.push(index);
        if let Some(content) = child(Some(sdt), "w:sdtContent") {
            for node in &content.children {
                if let XmlNode::Element(node) = node {
                    self.walk(node, paragraph);
                }
            }
        }
        self.stack.pop();

        // Where its content is: the runs, or the paragraphs, inside it.
        let mut runs: Vec<RunPlace> = Vec::new();
        let mut paragraphs: Vec<usize> = Vec::new();
        if let Some(content) = child(Some(sdt), "w:sdtContent") {
            collect_places(content, self.places, &mut runs, &mut paragraphs);
        }
        let document = self.document;
        let control = &mut self.controls[index];
        if control.block && !paragraphs.is_empty() {
            let first = *paragraphs.iter().min().expect("not empty");
            let last = *paragraphs.iter().max().expect("not empty");
            let length = document.paragraph_text_len_at(last).unwrap_or(0);
            control.start = document.rendered(first, 0);
            control.end = document.rendered(last, length);
            let mut ordered = paragraphs.clone();
            ordered.sort_unstable();
            ordered.dedup();
            control.text = ordered
                .iter()
                .map(|&index| paragraph_plain_text(document, index))
                .collect::<Vec<_>>()
                .join("\n");
        } else if let (Some(first), Some(last)) = (
            runs.iter()
                .min_by_key(|place| (place.paragraph, place.start)),
            runs.iter().max_by_key(|place| (place.paragraph, place.end)),
        ) {
            control.start = document.rendered(first.paragraph, first.start);
            control.end = document.rendered(last.paragraph, last.end);
            let mut text = String::new();
            let mut ordered = runs.clone();
            ordered.sort_by_key(|place| (place.paragraph, place.start));
            let mut previous: Option<usize> = None;
            for place in ordered {
                if previous.is_some_and(|previous| previous != place.paragraph) {
                    text.push('\n');
                }
                previous = Some(place.paragraph);
                if let Some(slice) = document
                    .paragraph(place.paragraph)
                    .map(|paragraph| paragraph_model_text(paragraph))
                    .and_then(|whole| whole.get(place.start..place.end).map(str::to_string))
                {
                    text.push_str(&slice);
                }
            }
            control.text = text;
        } else {
            let (paragraph, offset) = before.or(paragraph.map(|p| (p, 0))).unwrap_or((0, 0));
            control.start = document.rendered(paragraph, offset);
            control.end = control.start;
        }
    }
}

fn collect_places(
    element: &XmlElement,
    places: &ModelPlaces,
    runs: &mut Vec<RunPlace>,
    paragraphs: &mut Vec<usize>,
) {
    for node in &element.children {
        let XmlNode::Element(node) = node else {
            continue;
        };
        if let Some(place) = places.runs.get(&node.id) {
            runs.push(*place);
        }
        if let Some(&index) = places.paragraphs.get(&node.id) {
            paragraphs.push(index);
        }
        collect_places(node, places, runs, paragraphs);
    }
}

fn paragraph_model_text(paragraph: &Paragraph) -> String {
    paragraph.runs.iter().map(|run| run.text.as_str()).collect()
}

fn paragraph_plain_text(document: &WordDocument, index: usize) -> String {
    document
        .paragraph(index)
        .map(paragraph_model_text)
        .unwrap_or_default()
}

/// The first child element named `name`.
pub(crate) fn child<'a>(element: Option<&'a XmlElement>, name: &str) -> Option<&'a XmlElement> {
    element?.children.iter().find_map(|node| match node {
        XmlNode::Element(child) if child.name == name => Some(child),
        _ => None,
    })
}

/// An attribute's value, by its qualified name.
pub(crate) fn attribute<'a>(element: &'a XmlElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .map(|attribute| attribute.value.as_str())
}

fn val(element: Option<&XmlElement>) -> Option<String> {
    element.and_then(|element| {
        attribute(element, "w:val")
            .or_else(|| attribute(element, "w14:val"))
            .or_else(|| attribute(element, "w15:val"))
            .map(str::to_string)
    })
}

/// An on/off property: present and not switched off.
fn on(element: Option<&XmlElement>) -> bool {
    element.is_some_and(|element| {
        !matches!(
            attribute(element, "w:val").or_else(|| attribute(element, "w14:val")),
            Some("0" | "false" | "off")
        )
    })
}

fn symbol(element: Option<&XmlElement>) -> Option<CheckSymbol> {
    let element = element?;
    let code = attribute(element, "w14:val")?;
    let symbol = u32::from_str_radix(code, 16)
        .ok()
        .and_then(char::from_u32)?;
    Some(CheckSymbol {
        symbol,
        font: attribute(element, "w14:font")
            .unwrap_or_default()
            .to_string(),
    })
}

fn read_properties(properties: Option<&XmlElement>, index: usize) -> ContentControl {
    let get = |name: &str| child(properties, name);
    let kind = if get("w14:checkbox").is_some() {
        ContentControlKind::CheckBox
    } else if get("w15:repeatingSection").is_some() {
        ContentControlKind::RepeatingSection
    } else if get("w15:repeatingSectionItem").is_some() {
        ContentControlKind::RepeatingSectionItem
    } else if get("w:text").is_some() {
        ContentControlKind::PlainText
    } else if get("w:picture").is_some() {
        ContentControlKind::Picture
    } else if get("w:comboBox").is_some() {
        ContentControlKind::ComboBox
    } else if get("w:dropDownList").is_some() {
        ContentControlKind::DropDownList
    } else if get("w:date").is_some() {
        ContentControlKind::Date
    } else if get("w:docPartList").is_some() || get("w:docPartObj").is_some() {
        ContentControlKind::BuildingBlockGallery
    } else if get("w:group").is_some() {
        ContentControlKind::Group
    } else if let Some(other) = ["w:equation", "w:citation", "w:bibliography"]
        .into_iter()
        .find(|name| get(name).is_some())
    {
        ContentControlKind::Other(other.trim_start_matches("w:").to_string())
    } else {
        ContentControlKind::RichText
    };
    let lock = val(get("w:lock")).unwrap_or_default();
    let list = get("w:comboBox").or_else(|| get("w:dropDownList"));
    let entries = list
        .map(|list| {
            list.children
                .iter()
                .filter_map(|node| match node {
                    XmlNode::Element(item) if item.name == "w:listItem" => {
                        let value = attribute(item, "w:value").unwrap_or_default().to_string();
                        let display = attribute(item, "w:displayText")
                            .map(str::to_string)
                            .unwrap_or_else(|| value.clone());
                        Some(ListEntry { display, value })
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let date = get("w:date");
    let checkbox = get("w14:checkbox");
    let gallery = get("w:docPartList").or_else(|| get("w:docPartObj"));
    let repeating = get("w15:repeatingSection");
    ContentControl {
        index,
        id: val(get("w:id")).and_then(|value| value.parse().ok()),
        kind,
        title: val(get("w:alias")),
        tag: val(get("w:tag")),
        cannot_delete: matches!(lock.as_str(), "sdtLocked" | "sdtContentLocked"),
        cannot_edit: matches!(lock.as_str(), "contentLocked" | "sdtContentLocked"),
        showing_placeholder: on(get("w:showingPlcHdr")),
        placeholder_part: val(child(get("w:placeholder"), "w:docPart")),
        temporary: on(get("w:temporary")),
        multi_line: get("w:text")
            .and_then(|text| attribute(text, "w:multiLine"))
            .is_some_and(|value| matches!(value, "1" | "true" | "on")),
        appearance: val(get("w15:appearance"))
            .map(|value| ControlAppearance::from_ooxml(&value))
            .unwrap_or_default(),
        color: val(get("w15:color")),
        style: val(child(get("w:rPr"), "w:rStyle")),
        entries,
        date_format: val(child(date, "w:dateFormat")),
        date_language: val(child(date, "w:lid")),
        full_date: date.and_then(|date| attribute(date, "w:fullDate").map(str::to_string)),
        checked: on(child(checkbox, "w14:checked")),
        checked_symbol: symbol(child(checkbox, "w14:checkedState")),
        unchecked_symbol: symbol(child(checkbox, "w14:uncheckedState")),
        gallery: val(child(gallery, "w:docPartGallery")),
        category: val(child(gallery, "w:docPartCategory")),
        section_title: val(child(repeating, "w15:sectionTitle")),
        fixed_items: on(child(repeating, "w15:doNotAllowInsertDeleteSection")),
        block: false,
        start: TextPosition::new(0, 0),
        end: TextPosition::new(0, 0),
        text: String::new(),
        depth: 0,
        parent: None,
    }
}

#[cfg(test)]
#[path = "content_controls/tests.rs"]
mod tests;
