//! Mail merge (差し込み印刷), as Word does it: a main document — letters,
//! envelopes, labels or a directory — whose merge fields take each
//! recipient's values from a data source.
//!
//! The merge fields are fields like any other (ECMA-376 Part 1 §17.16.5):
//! `MERGEFIELD`, `ADDRESSBLOCK`, `GREETINGLINE`, and Word's rules `IF`,
//! `NEXT`, `NEXTIF`, `SKIPIF`, `MERGEREC`, `MERGESEQ`, `SET`, `ASK` and
//! `FILLIN`. This module evaluates them for a recipient as one more kind of
//! field ([`fields`]) and writes the results three ways: in place, keeping
//! the fields, to preview a recipient; as the «chevrons» Word shows with
//! preview off; and as plain text, one copy of the body per recipient, for
//! a merged document. It also puts the fields in, keeps the merge's
//! settings (`w:mailMerge`, §17.14), checks a merge for errors, and lays a
//! document out as an envelope or a sheet of labels ([`layouts`]).

use super::rewrite::{xml_escape, xml_fragment, CaretAfter, RewriteCommand};
use super::*;

#[path = "mail_merge/address.rs"]
pub mod address;
#[path = "mail_merge/fields.rs"]
pub mod fields;
#[path = "mail_merge/layouts.rs"]
pub mod layouts;

pub use address::{
    AddressBlockOptions, AddressField, FieldMapping, GreetingLineOptions, NameFormat,
};
pub use fields::{FieldMode, MergeProblem};
pub use layouts::{EnvelopeSize, EnvelopeSpec, LabelProduct, ENVELOPE_SIZES, LABEL_PRODUCTS};

/// A data source's columns and records, every value as text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MergeData {
    pub columns: Vec<String>,
    pub records: Vec<Vec<String>>,
}

/// A merge field's name as Word writes it: spaces become underscores.
pub fn field_name(column: &str) -> String {
    column.trim().replace(' ', "_")
}

fn same_name(a: &str, b: &str) -> bool {
    let normalize = |text: &str| {
        text.trim()
            .chars()
            .map(|ch| if ch == ' ' { '_' } else { ch })
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    normalize(a) == normalize(b)
}

impl MergeData {
    /// The column a merge field names: by name, case and spaces aside.
    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns
            .iter()
            .position(|column| same_name(column, name))
    }

    /// A record's value in a column, empty when the record is short.
    pub fn value(&self, record: usize, column: usize) -> &str {
        self.records
            .get(record)
            .and_then(|values| values.get(column))
            .map(String::as_str)
            .unwrap_or("")
    }

    /// The data from rows: the first is the header.
    pub fn from_rows(mut rows: Vec<Vec<String>>) -> Self {
        // Trailing empty rows are not recipients.
        while rows
            .last()
            .is_some_and(|row| row.iter().all(|value| value.trim().is_empty()))
        {
            rows.pop();
        }
        if rows.is_empty() {
            return Self::default();
        }
        let mut columns = rows.remove(0);
        // Unnamed columns get Word's names for them.
        for (index, column) in columns.iter_mut().enumerate() {
            if column.trim().is_empty() {
                *column = format!("F{}", index + 1);
            }
            *column = column.trim().to_string();
        }
        let width = columns.len();
        let records = rows
            .into_iter()
            .map(|mut row| {
                row.resize(width, String::new());
                row
            })
            .collect();
        Self { columns, records }
    }
}

/// What kind of main document a merge has (`w:mainDocumentType`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MainDocumentType {
    /// レター.
    #[default]
    Letters,
    /// 封筒.
    Envelopes,
    /// ラベル.
    Labels,
    /// 名簿 (Word's catalog).
    Directory,
    /// 電子メール メッセージ.
    Email,
}

impl MainDocumentType {
    pub fn as_ooxml(self) -> &'static str {
        match self {
            MainDocumentType::Letters => "formLetters",
            MainDocumentType::Envelopes => "envelopes",
            MainDocumentType::Labels => "mailingLabels",
            MainDocumentType::Directory => "catalog",
            MainDocumentType::Email => "email",
        }
    }

    pub fn from_ooxml(value: &str) -> Option<Self> {
        Some(match value {
            "formLetters" | "formLetter" => MainDocumentType::Letters,
            "envelopes" => MainDocumentType::Envelopes,
            "mailingLabels" => MainDocumentType::Labels,
            "catalog" => MainDocumentType::Directory,
            "email" => MainDocumentType::Email,
            _ => return None,
        })
    }
}

/// The merge's settings as the document keeps them (`w:mailMerge`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MailMergeSettings {
    /// `None` for a document that is not a main document.
    pub kind: Option<MainDocumentType>,
    /// Where the data source is: a file path.
    pub data_source: Option<String>,
    /// 結果のプレビュー (`w:viewMergedData`).
    pub preview: bool,
    /// The record previewed, 1-based (`w:activeRecord`).
    pub active_record: usize,
    /// フィールドの対応: Word's address fields and the columns they take.
    pub mapping: Vec<(AddressField, String)>,
    /// エラーのチェック's last choice (`w:checkErrors`).
    pub check_errors: Option<u32>,
}

impl MailMergeSettings {
    /// The data-source mapping as one the evaluator reads.
    pub fn field_mapping(&self, data: &MergeData) -> FieldMapping {
        let mut mapping = FieldMapping::matched(data);
        for (field, column) in &self.mapping {
            mapping.set(*field, data.column(column));
        }
        mapping
    }
}

const MAIL_MERGE_SOURCE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/mailMergeSource";
const MERGE_SOURCE_ID: &str = "rIdMergeSource1";

/// What a merge is over: the data, the records chosen (in the order they
/// are merged), and how Word's address fields map to columns.
#[derive(Clone, Debug)]
pub struct MergeRequest {
    pub data: MergeData,
    /// Indices into `data.records`, in merge order.
    pub records: Vec<usize>,
    pub mapping: FieldMapping,
}

impl MergeRequest {
    /// Every record, in the data's order, mapped by name.
    pub fn all(data: MergeData) -> Self {
        let records = (0..data.records.len()).collect();
        let mapping = FieldMapping::matched(&data);
        Self {
            data,
            records,
            mapping,
        }
    }
}

/// A merge field to put in at the caret.
#[derive(Clone, Debug, PartialEq)]
pub enum MergeFieldSpec {
    /// 差し込みフィールドの挿入: a column.
    Column(String),
    /// 住所ブロック.
    AddressBlock(AddressBlockOptions),
    /// あいさつ文.
    GreetingLine(GreetingLineOptions),
    /// A ルール.
    Rule(MergeRule),
}

/// Word's ルール.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeRule {
    /// Ask: a bookmark set from what is typed when the merge runs.
    Ask {
        bookmark: String,
        prompt: String,
        default: String,
        once: bool,
    },
    /// Fill-in: text typed when the merge runs.
    FillIn {
        prompt: String,
        default: String,
        once: bool,
    },
    /// If...Then...Else.
    If {
        field: String,
        operator: String,
        value: String,
        then_text: String,
        else_text: String,
    },
    /// Merge Record #.
    MergeRecord,
    /// Merge Sequence #.
    MergeSequence,
    /// Next Record.
    Next,
    /// Next Record If.
    NextIf {
        field: String,
        operator: String,
        value: String,
    },
    /// Set Bookmark.
    Set { bookmark: String, value: String },
    /// Skip Record If.
    SkipIf {
        field: String,
        operator: String,
        value: String,
    },
}

/// The comparison operators the rules offer, as the field writes them.
pub const RULE_OPERATORS: [&str; 8] = ["=", "<>", "<", ">", "<=", ">=", "isblank", "isnotblank"];

fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\\\""))
}

/// A comparison's right side: blank or not are `""` against `=` / `<>`.
fn comparison(operator: &str, value: &str) -> (String, String) {
    match operator {
        "isblank" => ("=".to_string(), "\"\"".to_string()),
        "isnotblank" => ("<>".to_string(), "\"\"".to_string()),
        other => (other.to_string(), quoted(value)),
    }
}

/// A complex field: `instruction` (text, or a nested field's XML marked
/// `\u{1}`…`\u{2}`) and its result as plain text in a run.
fn complex_field_xml(instruction: &[InstructionPart], result: &str, properties: &str) -> String {
    let mut xml = String::from("<w:r><w:fldChar w:fldCharType=\"begin\"/></w:r>");
    for part in instruction {
        match part {
            InstructionPart::Text(text) => xml.push_str(&format!(
                "<w:r>{properties}<w:instrText xml:space=\"preserve\">{}</w:instrText></w:r>",
                xml_escape(text)
            )),
            InstructionPart::Field(field) => xml.push_str(field),
        }
    }
    xml.push_str("<w:r><w:fldChar w:fldCharType=\"separate\"/></w:r>");
    xml.push_str(&result_runs_xml(result, properties));
    xml.push_str("<w:r><w:fldChar w:fldCharType=\"end\"/></w:r>");
    xml
}

enum InstructionPart {
    Text(String),
    Field(String),
}

/// Text as runs: a line break between lines.
pub(crate) fn result_runs_xml(text: &str, properties: &str) -> String {
    let mut xml = String::new();
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            xml.push_str(&format!("<w:r>{properties}<w:br/></w:r>"));
        }
        if !line.is_empty() {
            xml.push_str(&format!(
                "<w:r>{properties}<w:t xml:space=\"preserve\">{}</w:t></w:r>",
                xml_escape(line)
            ));
        }
    }
    xml
}

impl MergeFieldSpec {
    /// The field's XML, with Word's «chevron» result.
    fn xml(&self, properties: &str) -> String {
        let merge_field = |name: &str| {
            let name = field_name(name);
            complex_field_xml(
                &[InstructionPart::Text(format!(" MERGEFIELD {name} "))],
                &format!("«{name}»"),
                properties,
            )
        };
        match self {
            MergeFieldSpec::Column(name) => merge_field(name),
            MergeFieldSpec::AddressBlock(options) => complex_field_xml(
                &[InstructionPart::Text(format!(
                    " {} ",
                    options.instruction()
                ))],
                "«AddressBlock»",
                properties,
            ),
            MergeFieldSpec::GreetingLine(options) => complex_field_xml(
                &[InstructionPart::Text(format!(
                    " {} ",
                    options.instruction()
                ))],
                "«GreetingLine»",
                properties,
            ),
            MergeFieldSpec::Rule(rule) => match rule {
                MergeRule::Ask {
                    bookmark,
                    prompt,
                    default,
                    once,
                } => complex_field_xml(
                    &[InstructionPart::Text(format!(
                        " ASK {} {} \\d {}{} ",
                        field_name(bookmark),
                        quoted(prompt),
                        quoted(default),
                        if *once { " \\o" } else { "" }
                    ))],
                    "",
                    properties,
                ),
                MergeRule::FillIn {
                    prompt,
                    default,
                    once,
                } => complex_field_xml(
                    &[InstructionPart::Text(format!(
                        " FILLIN {} \\d {}{} ",
                        quoted(prompt),
                        quoted(default),
                        if *once { " \\o" } else { "" }
                    ))],
                    default,
                    properties,
                ),
                MergeRule::If {
                    field,
                    operator,
                    value,
                    then_text,
                    else_text,
                } => {
                    let (operator, value) = comparison(operator, value);
                    complex_field_xml(
                        &[
                            InstructionPart::Text(" IF ".to_string()),
                            InstructionPart::Field(merge_field(field)),
                            InstructionPart::Text(format!(
                                " {operator} {value} {} {} ",
                                quoted(then_text),
                                quoted(else_text)
                            )),
                        ],
                        then_text,
                        properties,
                    )
                }
                MergeRule::MergeRecord => complex_field_xml(
                    &[InstructionPart::Text(" MERGEREC ".to_string())],
                    "«Merge Record #»",
                    properties,
                ),
                MergeRule::MergeSequence => complex_field_xml(
                    &[InstructionPart::Text(" MERGESEQ ".to_string())],
                    "«Merge Sequence #»",
                    properties,
                ),
                MergeRule::Next => complex_field_xml(
                    &[InstructionPart::Text(" NEXT ".to_string())],
                    "«Next Record»",
                    properties,
                ),
                MergeRule::NextIf {
                    field,
                    operator,
                    value,
                } => {
                    let (operator, value) = comparison(operator, value);
                    complex_field_xml(
                        &[
                            InstructionPart::Text(" NEXTIF ".to_string()),
                            InstructionPart::Field(merge_field(field)),
                            InstructionPart::Text(format!(" {operator} {value} ")),
                        ],
                        "«Next Record If»",
                        properties,
                    )
                }
                MergeRule::Set { bookmark, value } => complex_field_xml(
                    &[InstructionPart::Text(format!(
                        " SET {} {} ",
                        field_name(bookmark),
                        quoted(value)
                    ))],
                    "",
                    properties,
                ),
                MergeRule::SkipIf {
                    field,
                    operator,
                    value,
                } => {
                    let (operator, value) = comparison(operator, value);
                    complex_field_xml(
                        &[
                            InstructionPart::Text(" SKIPIF ".to_string()),
                            InstructionPart::Field(merge_field(field)),
                            InstructionPart::Text(format!(" {operator} {value} ")),
                        ],
                        "«Skip Record If»",
                        properties,
                    )
                }
            },
        }
    }
}

/// What a command does to a mail-merge main document.
#[derive(Clone, Debug)]
pub enum MailMergeAction {
    /// The merge's settings written to `word/settings.xml`.
    Settings(MailMergeSettings),
    /// A merge field at the caret, showing `preview`'s values when it is
    /// given.
    InsertField {
        field: MergeFieldSpec,
        preview: Option<(MergeRequest, usize)>,
    },
    /// 結果のプレビュー: every merge field showing this record's values
    /// (an index into the request's records).
    Preview(MergeRequest, usize),
    /// Preview off: the fields show their «names».
    FieldCodes,
    /// The document made an envelope: its size, the return address and the
    /// delivery address (the address block when it is a merge).
    MakeEnvelope(EnvelopeSpec),
    /// The document made a sheet of labels of a product; `merge` puts the
    /// address block in the first and Next Record in the rest.
    MakeLabels {
        product: LabelProduct,
        text: Option<String>,
        merge: bool,
        japanese: bool,
    },
    /// ラベルの更新: the first label's content copied to every other one,
    /// after a Next Record.
    PropagateLabels,
    /// 標準の Word 文書: the document is no longer a main document.
    Normal,
}

impl MailMergeAction {
    fn name(&self) -> &'static str {
        match self {
            MailMergeAction::Settings(_) => "mail_merge_settings",
            MailMergeAction::InsertField { .. } => "insert_merge_field",
            MailMergeAction::Preview(..) => "preview_merge",
            MailMergeAction::FieldCodes => "merge_field_codes",
            MailMergeAction::MakeEnvelope(_) => "make_envelope",
            MailMergeAction::MakeLabels { .. } => "make_labels",
            MailMergeAction::PropagateLabels => "propagate_labels",
            MailMergeAction::Normal => "merge_normal_document",
        }
    }

    /// The action as one undo step, for the selection `range` (rendered).
    pub fn command(self, range: SelectionRange) -> RewriteCommand {
        let name = self.name();
        RewriteCommand::new(name, move |xml, document| {
            document.apply_mail_merge_action(xml, &self, range)
        })
    }
}

impl WordDocument {
    /// The merge's settings, from `word/settings.xml`.
    pub fn mail_merge_settings(&self) -> MailMergeSettings {
        let Some(settings) = self.settings_xml_content() else {
            return MailMergeSettings::default();
        };
        let Ok(parsed) = DocxXmlDocument::parse(&settings) else {
            return MailMergeSettings::default();
        };
        let Some(merge) = parsed.root().children.iter().find_map(|node| match node {
            XmlNode::Element(element) if element.name == "w:mailMerge" => Some(element),
            _ => None,
        }) else {
            return MailMergeSettings::default();
        };
        let child = |name: &str| {
            merge.children.iter().find_map(|node| match node {
                XmlNode::Element(element) if element.name == name => Some(element),
                _ => None,
            })
        };
        let val = |element: Option<&XmlElement>| {
            element.and_then(|element| {
                element
                    .attributes
                    .iter()
                    .find(|attribute| attribute.name == "w:val")
                    .map(|attribute| attribute.value.clone())
            })
        };
        let flag = |element: Option<&XmlElement>| {
            element.is_some_and(|element| {
                !element.attributes.iter().any(|attribute| {
                    attribute.name == "w:val" && matches!(attribute.value.as_str(), "0" | "false")
                })
            })
        };
        let data_source = child("w:dataSource")
            .and_then(|source| {
                source
                    .attributes
                    .iter()
                    .find(|attribute| attribute.name == "r:id")
                    .map(|attribute| attribute.value.clone())
            })
            .and_then(|id| self.settings_relationship_target(&id))
            .or_else(|| {
                // The query names the file when there is no relationship.
                val(child("w:query")).and_then(|query| {
                    let start = query.find('`')?;
                    let end = query[start + 1..].find('`')?;
                    Some(query[start + 1..start + 1 + end].to_string())
                })
            })
            .map(|target| {
                target
                    .strip_prefix("file:///")
                    .map(|path| format!("/{path}"))
                    .unwrap_or(target)
            });
        let mut mapping = Vec::new();
        if let Some(odso) = child("w:odso") {
            for node in &odso.children {
                let XmlNode::Element(map) = node else {
                    continue;
                };
                if map.name != "w:fieldMapData" {
                    continue;
                }
                let get = |name: &str| {
                    map.children.iter().find_map(|node| match node {
                        XmlNode::Element(element) if element.name == name => element
                            .attributes
                            .iter()
                            .find(|attribute| attribute.name == "w:val")
                            .map(|attribute| attribute.value.clone()),
                        _ => None,
                    })
                };
                if let (Some(column), Some(field)) = (
                    get("w:name"),
                    get("w:mappedName").and_then(|name| AddressField::from_odso(&name)),
                ) {
                    mapping.push((field, column));
                }
            }
        }
        MailMergeSettings {
            kind: val(child("w:mainDocumentType"))
                .and_then(|value| MainDocumentType::from_ooxml(&value)),
            data_source,
            preview: flag(child("w:viewMergedData")),
            active_record: val(child("w:activeRecord"))
                .and_then(|value| value.parse().ok())
                .unwrap_or(1),
            mapping,
            check_errors: val(child("w:checkErrors")).and_then(|value| value.parse().ok()),
        }
    }

    fn settings_relationship_target(&self, id: &str) -> Option<String> {
        let rels = self.read_xml_part_content("word/_rels/settings.xml.rels")?;
        let parsed = DocxXmlDocument::parse(&rels).ok()?;
        parsed.root().children.iter().find_map(|node| match node {
            XmlNode::Element(element)
                if element.name == "Relationship"
                    && element
                        .attributes
                        .iter()
                        .any(|attribute| attribute.name == "Id" && attribute.value == id) =>
            {
                element
                    .attributes
                    .iter()
                    .find(|attribute| attribute.name == "Target")
                    .map(|attribute| attribute.value.clone())
            }
            _ => None,
        })
    }

    /// Writes the merge's settings: `w:mailMerge`, and the data source's
    /// relationship from the settings part.
    fn write_mail_merge_settings(&mut self, settings: &MailMergeSettings) -> Result<(), String> {
        let previous = self
            .settings_xml_content()
            .ok_or_else(|| "settings.xml is not present in this package".to_string())?;
        let mut parsed = DocxXmlDocument::parse(&previous)
            .map_err(|error| format!("failed to parse settings.xml: {error:?}"))?;
        let merge = settings.kind.map(|kind| {
            let mut xml = format!(
                "<w:mailMerge><w:mainDocumentType w:val=\"{}\"/><w:linkToQuery/>",
                kind.as_ooxml()
            );
            if let Some(source) = &settings.data_source {
                let lower = source.to_ascii_lowercase();
                let data_type = if lower.ends_with(".xlsx") || lower.ends_with(".xls") {
                    "spreadsheet"
                } else if lower.ends_with(".docx") || lower.ends_with(".doc") {
                    "native"
                } else {
                    "textFile"
                };
                xml.push_str(&format!("<w:dataType w:val=\"{data_type}\"/>"));
                xml.push_str("<w:connectString w:val=\"\"/>");
                let table = if data_type == "spreadsheet" {
                    format!("{source}`.`Sheet1$")
                } else {
                    source.clone()
                };
                xml.push_str(&format!(
                    "<w:query w:val=\"SELECT * FROM `{}`\"/>",
                    xml_escape(&table)
                ));
                xml.push_str(&format!(
                    "<w:dataSource r:id=\"{MERGE_SOURCE_ID}\"/>"
                ));
            }
            if let Some(check) = settings.check_errors {
                xml.push_str(&format!("<w:checkErrors w:val=\"{check}\"/>"));
            }
            if settings.preview {
                xml.push_str("<w:viewMergedData/>");
            }
            if settings.active_record > 1 {
                xml.push_str(&format!(
                    "<w:activeRecord w:val=\"{}\"/>",
                    settings.active_record
                ));
            }
            if !settings.mapping.is_empty() {
                xml.push_str("<w:odso>");
                for (field, column) in &settings.mapping {
                    xml.push_str(&format!(
                        "<w:fieldMapData><w:type w:val=\"dbColumn\"/><w:name w:val=\"{}\"/><w:mappedName w:val=\"{}\"/></w:fieldMapData>",
                        xml_escape(column),
                        field.odso_name()
                    ));
                }
                xml.push_str("</w:odso>");
            }
            xml.push_str("</w:mailMerge>");
            xml
        });
        let nodes = merge.map(|xml| {
            let wrapped = xml.replacen(
                "<w:mailMerge>",
                "<w:mailMerge xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">",
                1,
            );
            xml_fragment(&mut parsed, &wrapped)
        });
        let root_id = parsed.root().id;
        if let Some(root) = parsed.element_mut(root_id) {
            root.children.retain(
                |node| !matches!(node, XmlNode::Element(element) if element.name == "w:mailMerge"),
            );
            if let Some(nodes) = nodes {
                // Strip the namespace declaration the fragment carried: the
                // settings root declares `r` once the writer sees it used.
                for mut node in nodes {
                    if let XmlNode::Element(element) = &mut node {
                        element
                            .attributes
                            .retain(|attribute| attribute.name != "xmlns:r");
                    }
                    root.children.push(node);
                }
            }
        }
        parsed.rebuild_index();
        let written = parsed
            .to_string()
            .map_err(|error| format!("failed to write settings.xml: {error:?}"))?;
        self.restore_settings_xml(written);

        // The data source's relationship.
        let rels_path = "word/_rels/settings.xml.rels";
        let rels = self.read_xml_part_content(rels_path).unwrap_or_else(|| {
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>".to_string()
        });
        let mut parsed = DocxXmlDocument::parse(&rels)
            .map_err(|error| format!("failed to parse the settings relationships: {error:?}"))?;
        let relationship = settings
            .kind
            .and(settings.data_source.as_ref())
            .map(|source| {
                let target = if source.starts_with('/') {
                    format!("file://{source}")
                } else {
                    source.clone()
                };
                parsed.create_element(
                    "Relationship",
                    vec![
                        XmlAttribute {
                            name: "Id".to_string(),
                            value: MERGE_SOURCE_ID.to_string(),
                        },
                        XmlAttribute {
                            name: "Type".to_string(),
                            value: MAIL_MERGE_SOURCE.to_string(),
                        },
                        XmlAttribute {
                            name: "Target".to_string(),
                            value: target,
                        },
                        XmlAttribute {
                            name: "TargetMode".to_string(),
                            value: "External".to_string(),
                        },
                    ],
                    Vec::new(),
                )
            });
        let root_id = parsed.root().id;
        if let Some(root) = parsed.element_mut(root_id) {
            root.children.retain(|node| {
                !matches!(node, XmlNode::Element(element)
                    if element.attributes.iter().any(|attribute| attribute.name == "Type" && attribute.value == MAIL_MERGE_SOURCE))
            });
            if let Some(relationship) = relationship {
                root.children.push(XmlNode::Element(relationship));
            }
        }
        parsed.rebuild_index();
        let written = parsed
            .to_string()
            .map_err(|error| format!("failed to write the settings relationships: {error:?}"))?;
        self.upsert_xml_part_content(rels_path, written);
        Ok(())
    }

    /// The merged document: one copy of the body per recipient (or per
    /// page of labels), the merge fields as plain text, as a package.
    pub fn merged_document(&self, request: &MergeRequest) -> Result<Vec<u8>, String> {
        let kind = self
            .mail_merge_settings()
            .kind
            .unwrap_or(MainDocumentType::Letters);
        let xml = self.raw_document_or_parsed()?;
        let merged = fields::merge_body(&xml, request, kind)?;
        let mut copy = xml;
        let body_id = body_id(&copy).ok_or("the document has no body")?;
        // Every copy gets ids of its own.
        let nodes: Vec<XmlNode> = merged
            .into_iter()
            .map(|node| match node {
                XmlNode::Element(element) => XmlNode::Element(copy.create_element(
                    element.name,
                    element.attributes,
                    element.children,
                )),
                other => other,
            })
            .collect();
        if let Some(body) = copy.element_mut(body_id) {
            body.children = nodes;
        }
        copy.rebuild_index();
        let document_xml = copy
            .to_string()
            .map_err(|error| format!("the merged document could not be written: {error:?}"))?;
        // A merged document is an ordinary one: no merge settings.
        let mut merged = self.clone_for_output();
        merged.write_mail_merge_settings(&MailMergeSettings::default())?;
        merged.package_bytes_with(document_xml)
    }

    /// The round-trip XML, or `document.xml` parsed when the document was
    /// opened for display only.
    fn raw_document_or_parsed(&self) -> Result<DocxXmlDocument, String> {
        if let Some(xml) = self.raw_document.as_ref() {
            return Ok(xml.clone());
        }
        let part = self
            .document_part
            .as_ref()
            .ok_or_else(|| "the document has no main part".to_string())?;
        DocxXmlDocument::parse(&part.content)
            .map_err(|error| format!("the document XML could not be read: {error:?}"))
    }

    /// A copy of this document's package state, to write a new package
    /// from without touching this one.
    fn clone_for_output(&self) -> WordDocument {
        let mut copy = WordDocument::from_document(Document::default());
        copy.package = self.package.clone();
        copy.parts = self.parts.clone();
        copy.binary_parts = self.binary_parts.clone();
        copy.removed_parts = self.removed_parts.clone();
        copy.relationships = self.relationships.clone();
        copy.document_part = self.document_part.clone();
        copy
    }

    /// What is wrong with merging `request` into this document: fields with
    /// no column, an empty data source, a data source with nothing chosen.
    pub fn merge_problems(&self, request: &MergeRequest) -> Result<Vec<MergeProblem>, String> {
        let xml = self.raw_document_or_parsed()?;
        Ok(fields::problems(&xml, request))
    }

    /// The names of the merge fields the document uses, in order, once each.
    pub fn merge_field_names(&self) -> Vec<String> {
        self.raw_document
            .as_ref()
            .map(fields::field_names)
            .unwrap_or_default()
    }

    /// The body of a new document — an envelope or a sheet of labels — in
    /// this document's package: its styles, theme and settings, no merge.
    pub fn document_with_body(&self, body_xml: &str) -> Result<Vec<u8>, String> {
        let mut xml = self.raw_document_or_parsed()?;
        let body_id = body_id(&xml).ok_or("the document has no body")?;
        let nodes = xml_fragment(&mut xml, body_xml);
        if let Some(body) = xml.element_mut(body_id) {
            body.children = nodes;
        }
        xml.rebuild_index();
        let document_xml = xml
            .to_string()
            .map_err(|error| format!("the document could not be written: {error:?}"))?;
        let mut copy = self.clone_for_output();
        copy.write_mail_merge_settings(&MailMergeSettings::default())?;
        copy.package_bytes_with(document_xml)
    }

    fn apply_mail_merge_action(
        &mut self,
        xml: &mut DocxXmlDocument,
        action: &MailMergeAction,
        range: SelectionRange,
    ) -> Result<CaretAfter, String> {
        match action {
            MailMergeAction::Settings(settings) => {
                self.write_mail_merge_settings(settings)?;
                Ok(CaretAfter::Keep)
            }
            MailMergeAction::InsertField { field, preview } => {
                let (start, _) = range.normalized();
                let start = self.rendered_to_model_text_position(start);
                let places = self.model_places();
                let paragraph_id = self
                    .paragraph(start.paragraph)
                    .and_then(|paragraph| paragraph.node_id)
                    .ok_or("the caret is not in a paragraph of the body")?;
                let (parent, at) = super::content_controls::edit::place_at(
                    xml,
                    &places,
                    paragraph_id,
                    start.paragraph,
                    start.offset,
                )?;
                // The field takes the format of the text before it.
                let properties =
                    super::content_controls::edit::run_properties_before(xml, parent, at)
                        .map(|properties| super::rewrite::element_xml(&properties))
                        .unwrap_or_default();
                let nodes = xml_fragment(xml, &field.xml(&properties));
                let parent_element = xml.element_mut(parent).ok_or("no paragraph")?;
                for (offset, node) in nodes.into_iter().enumerate() {
                    parent_element.children.insert(at + offset, node);
                }
                xml.rebuild_index();
                if let Some((request, record)) = preview {
                    fields::rewrite(xml, request, *record, FieldMode::Preview);
                }
                // The caret goes after the field: past its result when the
                // result is text the model keeps (a merge field's).
                let shown = match field {
                    MergeFieldSpec::Column(name) => Some(
                        preview
                            .as_ref()
                            .and_then(|(request, record)| {
                                let column = request.data.column(name)?;
                                let index = *request.records.get(*record)?;
                                Some(request.data.value(index, column).to_string())
                            })
                            .unwrap_or_else(|| format!("«{}»", field_name(name))),
                    ),
                    _ => None,
                };
                Ok(CaretAfter::At(TextPosition::new(
                    start.paragraph,
                    start.offset + shown.map_or(0, |text| text.len()),
                )))
            }
            MailMergeAction::Preview(request, record) => {
                fields::rewrite(xml, request, *record, FieldMode::Preview);
                Ok(CaretAfter::Keep)
            }
            MailMergeAction::FieldCodes => {
                fields::rewrite(
                    xml,
                    &MergeRequest::all(MergeData::default()),
                    0,
                    FieldMode::Codes,
                );
                Ok(CaretAfter::Keep)
            }
            MailMergeAction::MakeEnvelope(spec) => {
                let body = layouts::envelope_body(spec);
                replace_body(xml, &body)?;
                Ok(CaretAfter::At(TextPosition::new(0, 0)))
            }
            MailMergeAction::MakeLabels {
                product,
                text,
                merge,
                japanese,
            } => {
                let body = layouts::labels_body(product, text.as_deref(), *merge, *japanese);
                replace_body(xml, &body)?;
                Ok(CaretAfter::At(TextPosition::new(0, 0)))
            }
            MailMergeAction::PropagateLabels => {
                layouts::propagate_labels(xml)?;
                Ok(CaretAfter::Keep)
            }
            MailMergeAction::Normal => {
                self.write_mail_merge_settings(&MailMergeSettings::default())?;
                Ok(CaretAfter::Keep)
            }
        }
    }
}

fn body_id(xml: &DocxXmlDocument) -> Option<XmlNodeId> {
    xml.root().children.iter().find_map(|node| match node {
        XmlNode::Element(element) if element.name == "w:body" => Some(element.id),
        _ => None,
    })
}

/// Replaces the body's content with `body_xml`, which carries its own
/// section properties.
fn replace_body(xml: &mut DocxXmlDocument, body_xml: &str) -> Result<(), String> {
    let id = body_id(xml).ok_or("the document has no body")?;
    let nodes = xml_fragment(xml, body_xml);
    let body = xml.element_mut(id).ok_or("the document has no body")?;
    body.children = nodes;
    xml.rebuild_index();
    Ok(())
}

#[cfg(test)]
#[path = "mail_merge/tests.rs"]
mod tests;
