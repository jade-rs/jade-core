//! The browser entry points for Word's 参考資料 tools beyond inserting a
//! plain TOC: 目次 options (levels, links, page numbers, leaders, styles),
//! 図表目次, 手動作成目次, the 文献目録 style and source manager, document
//! variables, and showing field codes (⌥F9 / ⇧F9).

use super::*;
use crate::commands::snapshot_edit::{EditScope, SnapshotEditCommand};
use crate::fields::{self, FieldCodeView, FieldCommand};
use crate::word_document::CitationSourceInfo;
use jade_docx_reader::document::TocOptions;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TableOfContentsJs {
    index: usize,
    start: usize,
    end: usize,
    instruction: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManualTocLineJs {
    level: u8,
    text: String,
    #[serde(default)]
    page: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DocumentVariableJs {
    name: String,
    value: String,
}

fn json_js<T: serde::Serialize>(value: &T) -> JsValue {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .unwrap_or(JsValue::NULL)
}

/// Word's defaults for a 目次 with `patch` (camelCase `TocOptions` fields)
/// over them. Unknown fields are refused.
fn toc_options(patch: JsValue) -> Result<TocOptions, JsValue> {
    let mut merged = serde_json::to_value(TocOptions {
        outline_min: 1,
        outline_max: 3,
        hyperlinks: true,
        ..TocOptions::default()
    })
    .map_err(|error| JsValue::from_str(&error.to_string()))?;
    if !(patch.is_null() || patch.is_undefined()) {
        let patch: serde_json::Value = serde_wasm_bindgen::from_value(patch)
            .map_err(|error| JsValue::from_str(&format!("invalid TOC options: {error}")))?;
        let (Some(patch), Some(target)) = (patch.as_object(), merged.as_object_mut()) else {
            return Err(JsValue::from_str("TOC options must be an object"));
        };
        for (key, value) in patch {
            target.insert(key.clone(), value.clone());
        }
    }
    serde_json::from_value(merged)
        .map_err(|error| JsValue::from_str(&format!("invalid TOC options: {error}")))
}

/// A leader as Word's 目次 dialog names it: `dot`, `hyphen`, `underscore`,
/// `middleDot` or `none`.
fn leader_of(leader: Option<String>) -> String {
    leader
        .filter(|leader| !leader.is_empty())
        .unwrap_or_else(|| "dot".to_string())
}

impl OffscreenDocumentView {
    /// Repaints with the field codes `view` asks for shown in place of
    /// their results.
    fn show_field_code_view(&self, view: FieldCodeView) -> Result<JsValue, JsValue> {
        self.prepare_field_xml()?;
        let display = {
            let mut state = self.state.borrow_mut();
            state.field_code_view = view;
            let view = state.field_code_view.clone();
            state
                .word_doc
                .as_ref()
                .map(|document| fields::field_code_display(document, &view))
        };
        self.rerender_after_display_change(move |renderer| {
            renderer.set_field_code_display(display);
        })
    }
}

#[wasm_bindgen]
impl OffscreenDocumentView {
    /// 目次 with Word's dialog options, after the caret's paragraph:
    /// `options` names `TocOptions` fields in camelCase (`outlineMin`,
    /// `outlineMax`, `hyperlinks`, `hidePageNumbers`, `customStyleLevels`,
    /// `captionLabel` for a 図表目次, …) over Word's defaults; `leader` is
    /// `dot`, `hyphen`, `underscore`, `middleDot` or `none`.
    #[wasm_bindgen(js_name = insertTableOfContentsWithOptions)]
    pub fn insert_table_of_contents_with_options(
        &self,
        options: JsValue,
        leader: Option<String>,
    ) -> Result<JsValue, JsValue> {
        let options = toc_options(options)?;
        let leader = leader_of(leader);
        let (start, _) = self.current_selection().normalized();
        let after = start.paragraph;
        self.apply_document_command(move |_| {
            Box::new(SnapshotEditCommand::new(
                "insert_table_of_contents",
                move |document| {
                    document
                        .insert_table_of_contents_with_leader(after, &options, &leader)
                        .map(|_| ())
                },
            ))
        })
    }

    /// The document's tables of contents and figures, in order.
    #[wasm_bindgen(js_name = tablesOfContents)]
    pub fn tables_of_contents(&self) -> Result<JsValue, JsValue> {
        self.prepare_field_xml()?;
        let state = self.state.borrow();
        let tables: Vec<TableOfContentsJs> = state
            .word_doc
            .as_ref()
            .map(WordDocument::tables_of_contents)
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(index, block)| TableOfContentsJs {
                index,
                start: block.start,
                end: block.end,
                instruction: block.instruction,
            })
            .collect();
        Ok(json_js(&tables))
    }

    /// Rebuilds table of contents `index` with new options (目次 ▸ 変更).
    #[wasm_bindgen(js_name = replaceTableOfContents)]
    pub fn replace_table_of_contents(
        &self,
        index: usize,
        options: JsValue,
        leader: Option<String>,
    ) -> Result<JsValue, JsValue> {
        let options = toc_options(options)?;
        let leader = leader_of(leader);
        self.apply_document_command(move |_| {
            Box::new(SnapshotEditCommand::new(
                "replace_table_of_contents",
                move |document| {
                    document
                        .replace_table_of_contents(index, &options, &leader)
                        .map(|_| ())
                },
            ))
        })
    }

    /// 手動作成目次: a 目次 heading and `lines` (`{ level, text, page }`) to
    /// type over after the caret's paragraph — set as generated contents
    /// are, but not a field.
    #[wasm_bindgen(js_name = insertManualTableOfContents)]
    pub fn insert_manual_table_of_contents(
        &self,
        heading: String,
        lines: JsValue,
        leader: Option<String>,
    ) -> Result<JsValue, JsValue> {
        let lines: Vec<ManualTocLineJs> = serde_wasm_bindgen::from_value(lines)
            .map_err(|error| JsValue::from_str(&format!("invalid TOC lines: {error}")))?;
        if lines.iter().any(|line| !(1..=9).contains(&line.level)) {
            return Err(JsValue::from_str("TOC levels run from 1 to 9"));
        }
        let lines: Vec<(u8, String, String)> = lines
            .into_iter()
            .map(|line| (line.level, line.text, line.page))
            .collect();
        let leader = leader_of(leader);
        let (start, _) = self.current_selection().normalized();
        let after = start.paragraph;
        self.apply_document_command(move |_| {
            Box::new(SnapshotEditCommand::new(
                "insert_manual_table_of_contents",
                move |document| {
                    document
                        .insert_manual_table_of_contents(after, &heading, &lines, &leader)
                        .map(|_| ())
                },
            ))
        })
    }

    /// The 文献目録 style (`APA`, `MLA`, `SIST02`, …); `null` without sources.
    #[wasm_bindgen(js_name = bibliographyStyle)]
    pub fn bibliography_style(&self) -> Option<String> {
        self.state
            .borrow()
            .word_doc
            .as_ref()
            .and_then(WordDocument::bibliography_style)
    }

    /// Sets the 文献目録 style: `name` as Word lists it and the style file
    /// Word selects (`\APASixthEditionOfficeOnline.xsl`, …).
    #[wasm_bindgen(js_name = setBibliographyStyle)]
    pub fn set_bibliography_style(
        &self,
        name: String,
        selected_style: String,
    ) -> Result<JsValue, JsValue> {
        self.apply_document_command(move |_| {
            Box::new(SnapshotEditCommand::with_scope(
                "set_bibliography_style",
                EditScope::Paragraphs(None),
                move |document| document.set_bibliography_style(&name, &selected_style),
            ))
        })
    }

    /// 資料文献の管理 ▸ 削除: takes the source `tag` out of the catalog.
    #[wasm_bindgen(js_name = removeCitationSource)]
    pub fn remove_citation_source(&self, tag: String) -> Result<JsValue, JsValue> {
        self.apply_document_command(move |_| {
            Box::new(SnapshotEditCommand::with_scope(
                "remove_citation_source",
                EditScope::Paragraphs(None),
                move |document| document.remove_citation_source(&tag),
            ))
        })
    }

    /// 資料文献の管理 ▸ 編集: replaces the source `tag` with `source`.
    #[wasm_bindgen(js_name = replaceCitationSource)]
    pub fn replace_citation_source(
        &self,
        tag: String,
        source: JsValue,
    ) -> Result<JsValue, JsValue> {
        let source: CitationSourceInfo = serde_wasm_bindgen::from_value(source)
            .map_err(|error| JsValue::from_str(&format!("invalid citation source: {error}")))?;
        self.apply_document_command(move |_| {
            Box::new(SnapshotEditCommand::with_scope(
                "replace_citation_source",
                EditScope::Paragraphs(None),
                move |document| document.replace_citation_source(&tag, source.clone()),
            ))
        })
    }

    /// The document variables (`w:docVars`) DOCVARIABLE fields read.
    #[wasm_bindgen(js_name = documentVariables)]
    pub fn document_variables(&self) -> JsValue {
        let state = self.state.borrow();
        let variables: Vec<DocumentVariableJs> = state
            .word_doc
            .as_ref()
            .map(fields::document_variables)
            .unwrap_or_default()
            .into_iter()
            .map(|(name, value)| DocumentVariableJs { name, value })
            .collect();
        json_js(&variables)
    }

    /// Replaces the document variables (`[{ name, value }]`).
    #[wasm_bindgen(js_name = setDocumentVariables)]
    pub fn set_document_variables(&self, variables: JsValue) -> Result<JsValue, JsValue> {
        let variables: Vec<DocumentVariableJs> = serde_wasm_bindgen::from_value(variables)
            .map_err(|error| JsValue::from_str(&format!("invalid document variables: {error}")))?;
        if variables
            .iter()
            .any(|variable| variable.name.trim().is_empty())
        {
            return Err(JsValue::from_str("a document variable needs a name"));
        }
        let variables: Vec<(String, String)> = variables
            .into_iter()
            .map(|variable| (variable.name, variable.value))
            .collect();
        self.apply_document_command(move |_| Box::new(FieldCommand::variables(variables)))
    }

    /// ⌥F9: shows every field's code (`true`) or every result (`false`).
    #[wasm_bindgen(js_name = setFieldCodesShown)]
    pub fn set_field_codes_shown(&self, all: bool) -> Result<JsValue, JsValue> {
        self.show_field_code_view(FieldCodeView {
            all,
            ..FieldCodeView::default()
        })
    }

    /// ⇧F9: toggles the fields the selection touches between code and
    /// result.
    #[wasm_bindgen(js_name = toggleFieldCodesAtSelection)]
    pub fn toggle_field_codes_at_selection(&self) -> Result<JsValue, JsValue> {
        self.prepare_field_xml()?;
        let selection = self.current_selection();
        let view = {
            let state = self.state.borrow();
            let Some(document) = state.word_doc.as_ref() else {
                return Ok(JsValue::NULL);
            };
            let mut view = state.field_code_view.clone();
            for id in fields::field_toggle_ids(document, selection) {
                if !view.toggled.remove(&id) {
                    view.toggled.insert(id);
                }
            }
            view
        };
        self.show_field_code_view(view)
    }

    /// Whether every field shows its code (⌥F9 is on).
    #[wasm_bindgen(js_name = fieldCodesShown)]
    pub fn field_codes_shown(&self) -> bool {
        self.state.borrow().field_code_view.all
    }
}
