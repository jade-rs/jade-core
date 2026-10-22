//! The browser entry points for Word's 差し込み文書 tab: a main document
//! (letters, envelopes, labels, directory, e-mail), merge fields — columns,
//! 住所ブロック, あいさつ文 and ルール — 結果のプレビュー, エラーのチェック, the
//! merged document, and back to a normal document.
//!
//! The recipients stay with the caller: each call that needs them takes the
//! data source as rows (the first the header) and the records chosen.

use super::*;
use crate::word_document::mail_merge::address::{AddressBlockOptions, GreetingLineOptions};
use crate::word_document::mail_merge::fields::MergeProblem;
use crate::word_document::mail_merge::layouts::{EnvelopeSpec, ENVELOPE_SIZES, LABEL_PRODUCTS};
use crate::word_document::mail_merge::{
    MailMergeAction, MainDocumentType, MergeData, MergeFieldSpec, MergeRequest, MergeRule,
};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct MailMergeSettingsJs {
    kind: Option<&'static str>,
    data_source: Option<String>,
    preview: bool,
    active_record: usize,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EnvelopeJs {
    /// A name from `envelopeSizes`.
    size: String,
    #[serde(default)]
    delivery: String,
    #[serde(default)]
    return_address: String,
    #[serde(default)]
    omit_return: bool,
    #[serde(default)]
    merge: bool,
    #[serde(default)]
    delivery_left_mm: Option<f32>,
    #[serde(default)]
    delivery_top_mm: Option<f32>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct EnvelopeSizeJs {
    name: &'static str,
    width_mm: f32,
    height_mm: f32,
    vertical: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LabelProductJs {
    vendor: &'static str,
    number: &'static str,
    description: &'static str,
    columns: usize,
    rows: usize,
    label_width_mm: f32,
    label_height_mm: f32,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecipientsJs {
    /// The data source, the first row its header.
    rows: Vec<Vec<String>>,
    /// The records chosen, as indexes into the rows after the header; all
    /// of them when left out.
    #[serde(default)]
    records: Option<Vec<usize>>,
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum MergeFieldJs {
    Column {
        name: String,
    },
    AddressBlock,
    GreetingLine,
    #[serde(rename_all = "camelCase")]
    If {
        field: String,
        operator: String,
        value: String,
        then_text: String,
        #[serde(default)]
        else_text: String,
    },
    Ask {
        bookmark: String,
        prompt: String,
        #[serde(default)]
        default: String,
        #[serde(default)]
        once: bool,
    },
    FillIn {
        prompt: String,
        #[serde(default)]
        default: String,
        #[serde(default)]
        once: bool,
    },
    MergeRecord,
    MergeSequence,
    Next,
}

fn kind_name(kind: MainDocumentType) -> &'static str {
    match kind {
        MainDocumentType::Letters => "letters",
        MainDocumentType::Envelopes => "envelopes",
        MainDocumentType::Labels => "labels",
        MainDocumentType::Directory => "directory",
        MainDocumentType::Email => "email",
    }
}

fn kind_of(name: &str) -> Result<MainDocumentType, JsValue> {
    Ok(match name {
        "letters" => MainDocumentType::Letters,
        "envelopes" => MainDocumentType::Envelopes,
        "labels" => MainDocumentType::Labels,
        "directory" => MainDocumentType::Directory,
        "email" => MainDocumentType::Email,
        _ => {
            return Err(JsValue::from_str(
                "the main document is letters, envelopes, labels, directory or email",
            ))
        }
    })
}

fn problem_text(problem: &MergeProblem) -> String {
    match problem {
        MergeProblem::EmptyDataSource => "データ ソースに列がありません。".to_string(),
        MergeProblem::NoRecipients => "宛先が選択されていません。".to_string(),
        MergeProblem::MissingColumn(column) => {
            format!("差し込みフィールド「{column}」はデータ ソースにありません。")
        }
        MergeProblem::UnmatchedAddress => {
            "住所ブロックまたはあいさつ文に使う列がありません。フィールドの対応を確認してください。"
                .to_string()
        }
    }
}

impl OffscreenDocumentView {
    /// The merge over `recipients`, mapped as the document's settings say.
    fn merge_request(&self, recipients: JsValue) -> Result<MergeRequest, JsValue> {
        let recipients: RecipientsJs = serde_wasm_bindgen::from_value(recipients)
            .map_err(|error| JsValue::from_str(&format!("invalid recipients: {error}")))?;
        let data = MergeData::from_rows(recipients.rows);
        let records = match recipients.records {
            Some(records) => {
                if records.iter().any(|record| *record >= data.records.len()) {
                    return Err(JsValue::from_str(
                        "a chosen record is not in the data source",
                    ));
                }
                records
            }
            None => (0..data.records.len()).collect(),
        };
        let mapping = self
            .state
            .borrow()
            .word_doc
            .as_ref()
            .map(|document| document.mail_merge_settings().field_mapping(&data))
            .ok_or_else(|| JsValue::from_str("No document loaded"))?;
        Ok(MergeRequest {
            data,
            records,
            mapping,
        })
    }

    fn apply_mail_merge(&self, action: MailMergeAction) -> Result<JsValue, JsValue> {
        self.apply_document_command(move |selection| Box::new(action.command(selection)))
    }
}

#[wasm_bindgen]
impl OffscreenDocumentView {
    /// The merge's settings: `{ kind, dataSource, preview, activeRecord }`,
    /// `kind` null for a document that is not a main document.
    #[wasm_bindgen(js_name = mailMergeSettings)]
    pub fn mail_merge_settings(&self) -> JsValue {
        let state = self.state.borrow();
        let Some(document) = state.word_doc.as_ref() else {
            return JsValue::NULL;
        };
        let settings = document.mail_merge_settings();
        let value = MailMergeSettingsJs {
            kind: settings.kind.map(kind_name),
            data_source: settings.data_source,
            preview: settings.preview,
            active_record: settings.active_record,
        };
        value
            .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
            .unwrap_or(JsValue::NULL)
    }

    /// 差し込み印刷の開始: makes the document a main document of `kind`.
    #[wasm_bindgen(js_name = startMailMerge)]
    pub fn start_mail_merge(
        &self,
        kind: String,
        data_source: Option<String>,
    ) -> Result<JsValue, JsValue> {
        let kind = kind_of(&kind)?;
        let mut settings = self
            .state
            .borrow()
            .word_doc
            .as_ref()
            .map(WordDocument::mail_merge_settings)
            .unwrap_or_default();
        settings.kind = Some(kind);
        if data_source.is_some() {
            settings.data_source = data_source;
        }
        self.apply_mail_merge(MailMergeAction::Settings(settings))
    }

    /// 標準の Word 文書: the document is no longer a main document.
    #[wasm_bindgen(js_name = stopMailMerge)]
    pub fn stop_mail_merge(&self) -> Result<JsValue, JsValue> {
        self.apply_mail_merge(MailMergeAction::Normal)
    }

    /// A merge field at the caret: `{ type: "column", name }`,
    /// `addressBlock`, `greetingLine`, or a ルール (`if`, `ask`, `fillIn`,
    /// `mergeRecord`, `mergeSequence`, `next`). With `recipients` and
    /// `record` it shows that record's values.
    #[wasm_bindgen(js_name = insertMergeField)]
    pub fn insert_merge_field(
        &self,
        field: JsValue,
        recipients: JsValue,
        record: Option<usize>,
        japanese: bool,
    ) -> Result<JsValue, JsValue> {
        let field: MergeFieldJs = serde_wasm_bindgen::from_value(field)
            .map_err(|error| JsValue::from_str(&format!("invalid merge field: {error}")))?;
        let field = match field {
            MergeFieldJs::Column { name } => MergeFieldSpec::Column(name),
            MergeFieldJs::AddressBlock => {
                MergeFieldSpec::AddressBlock(AddressBlockOptions::new(japanese))
            }
            MergeFieldJs::GreetingLine => {
                MergeFieldSpec::GreetingLine(GreetingLineOptions::new(japanese))
            }
            MergeFieldJs::If {
                field,
                operator,
                value,
                then_text,
                else_text,
            } => MergeFieldSpec::Rule(MergeRule::If {
                field,
                operator,
                value,
                then_text,
                else_text,
            }),
            MergeFieldJs::Ask {
                bookmark,
                prompt,
                default,
                once,
            } => MergeFieldSpec::Rule(MergeRule::Ask {
                bookmark,
                prompt,
                default,
                once,
            }),
            MergeFieldJs::FillIn {
                prompt,
                default,
                once,
            } => MergeFieldSpec::Rule(MergeRule::FillIn {
                prompt,
                default,
                once,
            }),
            MergeFieldJs::MergeRecord => MergeFieldSpec::Rule(MergeRule::MergeRecord),
            MergeFieldJs::MergeSequence => MergeFieldSpec::Rule(MergeRule::MergeSequence),
            MergeFieldJs::Next => MergeFieldSpec::Rule(MergeRule::Next),
        };
        let preview = match (recipients.is_null() || recipients.is_undefined(), record) {
            (false, Some(record)) => Some((self.merge_request(recipients)?, record)),
            _ => None,
        };
        self.apply_mail_merge(MailMergeAction::InsertField { field, preview })
    }

    /// 結果のプレビュー: every merge field shows record `record` (an index
    /// into the chosen records) of `recipients`.
    #[wasm_bindgen(js_name = previewMailMerge)]
    pub fn preview_mail_merge(
        &self,
        recipients: JsValue,
        record: usize,
    ) -> Result<JsValue, JsValue> {
        let request = self.merge_request(recipients)?;
        if record >= request.records.len() {
            return Err(JsValue::from_str(
                "the record previewed is not among those chosen",
            ));
        }
        self.apply_mail_merge(MailMergeAction::Preview(request, record))
    }

    /// Preview off: the merge fields show their «names».
    #[wasm_bindgen(js_name = showMergeFieldCodes)]
    pub fn show_merge_field_codes(&self) -> Result<JsValue, JsValue> {
        self.apply_mail_merge(MailMergeAction::FieldCodes)
    }

    /// エラーのチェック: what would stop or spoil a merge over `recipients`,
    /// as messages.
    #[wasm_bindgen(js_name = mailMergeProblems)]
    pub fn mail_merge_problems(&self, recipients: JsValue) -> Result<JsValue, JsValue> {
        let request = self.merge_request(recipients)?;
        let state = self.state.borrow();
        let document = state
            .word_doc
            .as_ref()
            .ok_or_else(|| JsValue::from_str("No document loaded"))?;
        let problems: Vec<String> = document
            .merge_problems(&request)
            .map_err(|error| JsValue::from_str(&error))?
            .iter()
            .map(problem_text)
            .collect();
        serde_wasm_bindgen::to_value(&problems)
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// 完了と差し込み ▸ 個々のドキュメントの編集: the merged document as
    /// `.docx` bytes, one copy of the main document per chosen record.
    #[wasm_bindgen(js_name = mergedDocument)]
    pub fn merged_document(&self, recipients: JsValue) -> Result<Vec<u8>, JsValue> {
        let request = self.merge_request(recipients)?;
        let state = self.state.borrow();
        let document = state
            .word_doc
            .as_ref()
            .ok_or_else(|| JsValue::from_str("No document loaded"))?;
        document
            .merged_document(&request)
            .map_err(|error| JsValue::from_str(&error))
    }

    /// The merge fields the document uses, by name.
    #[wasm_bindgen(js_name = mergeFieldNames)]
    pub fn merge_field_names(&self) -> JsValue {
        let state = self.state.borrow();
        let names = state
            .word_doc
            .as_ref()
            .map(WordDocument::merge_field_names)
            .unwrap_or_default();
        serde_wasm_bindgen::to_value(&names).unwrap_or(JsValue::NULL)
    }

    /// The sizes 封筒オプション offers.
    #[wasm_bindgen(js_name = envelopeSizes)]
    pub fn envelope_sizes(&self) -> JsValue {
        let sizes: Vec<EnvelopeSizeJs> = ENVELOPE_SIZES
            .iter()
            .map(|size| EnvelopeSizeJs {
                name: size.name,
                width_mm: size.width_mm,
                height_mm: size.height_mm,
                vertical: size.vertical,
            })
            .collect();
        serde_wasm_bindgen::to_value(&sizes).unwrap_or(JsValue::NULL)
    }

    /// The label sheets ラベル オプション offers.
    #[wasm_bindgen(js_name = labelProducts)]
    pub fn label_products(&self) -> JsValue {
        let products: Vec<LabelProductJs> = LABEL_PRODUCTS
            .iter()
            .map(|product| LabelProductJs {
                vendor: product.vendor,
                number: product.number,
                description: product.description,
                columns: product.columns,
                rows: product.rows,
                label_width_mm: product.label_width_mm,
                label_height_mm: product.label_height_mm,
            })
            .collect();
        serde_wasm_bindgen::to_value(&products).unwrap_or(JsValue::NULL)
    }

    /// 封筒: the document made an envelope of a size, with the delivery
    /// address (or, with `merge`, the address block) and the return address.
    #[wasm_bindgen(js_name = makeEnvelope)]
    pub fn make_envelope(&self, envelope: JsValue, japanese: bool) -> Result<JsValue, JsValue> {
        let envelope: EnvelopeJs = serde_wasm_bindgen::from_value(envelope)
            .map_err(|error| JsValue::from_str(&format!("invalid envelope: {error}")))?;
        let size = ENVELOPE_SIZES
            .iter()
            .copied()
            .find(|size| size.name == envelope.size)
            .ok_or_else(|| JsValue::from_str("unknown envelope size"))?;
        self.apply_mail_merge(MailMergeAction::MakeEnvelope(EnvelopeSpec {
            size,
            delivery: envelope.delivery,
            return_address: envelope.return_address,
            omit_return: envelope.omit_return,
            merge: envelope.merge,
            delivery_left_mm: envelope.delivery_left_mm,
            delivery_top_mm: envelope.delivery_top_mm,
            japanese,
        }))
    }

    /// ラベル: the document made a sheet of `vendor` `number` labels, each
    /// with `text` — or, with `merge`, the address block in the first and
    /// Next Record in the rest.
    #[wasm_bindgen(js_name = makeLabels)]
    pub fn make_labels(
        &self,
        vendor: String,
        number: String,
        text: Option<String>,
        merge: bool,
        japanese: bool,
    ) -> Result<JsValue, JsValue> {
        let product = LABEL_PRODUCTS
            .iter()
            .copied()
            .find(|product| product.vendor == vendor && product.number == number)
            .ok_or_else(|| JsValue::from_str("unknown label product"))?;
        self.apply_mail_merge(MailMergeAction::MakeLabels {
            product,
            text,
            merge,
            japanese,
        })
    }

    /// ラベルの更新: the first label copied to every other one.
    #[wasm_bindgen(js_name = propagateLabels)]
    pub fn propagate_labels(&self) -> Result<JsValue, JsValue> {
        self.apply_mail_merge(MailMergeAction::PropagateLabels)
    }
}
