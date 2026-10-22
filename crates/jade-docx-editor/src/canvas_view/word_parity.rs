//! The browser entry points for the Word dialogs the headless API gained
//! with the Office parity work: フォント, 段落 (改ページと改行, 体裁, タブと罫線),
//! 文字体裁, ルビ, 囲い文字, 日付と時刻, ページ設定 (sections), ハイフネーション,
//! 前と同じヘッダー/フッター, and the table tools (並べ替え, 計算式, 表の解除,
//! 文字列を表にする, 表の位置, セルの間隔, 文字列の方向).
//!
//! Each method resolves the selection or the caret's table and section the
//! way the other ribbon methods do and applies one headless command, so the
//! edit is one undo step and repaints like the rest of the ribbon.

use super::*;
use jade_docx_headless_api::{
    HeadlessHyphenationPatch, HeadlessJapaneseTypography, HeadlessParagraphBorderBox,
    HeadlessParagraphShading, HeadlessParagraphTypography, HeadlessRuby, HeadlessRunFormat,
    HeadlessSectionLayoutPatch, HeadlessSectionScope, HeadlessTabStop, HeadlessTableSortKey,
};

/// The 段落 dialog's タブとリーダー, 罫線と網かけ and options. A field left
/// out stays as it is; `clearShading` takes the shading off.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ParagraphOptionsJs {
    #[serde(default)]
    tabs: Option<Vec<HeadlessTabStop>>,
    #[serde(default)]
    contextual_spacing: Option<bool>,
    #[serde(default)]
    suppress_line_numbers: Option<bool>,
    #[serde(default)]
    suppress_auto_hyphens: Option<bool>,
    #[serde(default)]
    borders: Option<HeadlessParagraphBorderBox>,
    #[serde(default)]
    shading: Option<HeadlessParagraphShading>,
    #[serde(default)]
    clear_shading: bool,
}

/// The 段落 dialog's 改ページと改行 options.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ParagraphPaginationJs {
    #[serde(default)]
    keep_next: Option<bool>,
    #[serde(default)]
    keep_lines: Option<bool>,
    #[serde(default)]
    page_break_before: Option<bool>,
    #[serde(default)]
    widow_control: Option<bool>,
}

/// 表のプロパティ ▸ 表の位置.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TablePositionJs {
    #[serde(default)]
    horizontal_align: Option<String>,
    #[serde(default)]
    horizontal_offset: Option<f32>,
    horizontal_anchor: String,
    #[serde(default)]
    vertical_align: Option<String>,
    #[serde(default)]
    vertical_offset: Option<f32>,
    vertical_anchor: String,
    #[serde(default)]
    distance_top: f32,
    #[serde(default)]
    distance_bottom: f32,
    #[serde(default)]
    distance_left: f32,
    #[serde(default)]
    distance_right: f32,
    #[serde(default = "allow_overlap_default")]
    allow_overlap: bool,
}

fn allow_overlap_default() -> bool {
    true
}

fn from_js<T: serde::de::DeserializeOwned>(value: JsValue, what: &str) -> Result<T, JsValue> {
    serde_wasm_bindgen::from_value(value)
        .map_err(|error| JsValue::from_str(&format!("invalid {what}: {error}")))
}

/// [`from_js`] for the dialog payloads whose types tolerate unknown fields:
/// a key the type does not know (a typo, a field from another dialog) is an
/// error instead of an edit that silently does less.
fn from_js_strict<T>(value: JsValue, what: &str) -> Result<T, JsValue>
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let json: serde_json::Value = from_js(value, what)?;
    let parsed: T = serde_json::from_value(json.clone())
        .map_err(|error| JsValue::from_str(&format!("invalid {what}: {error}")))?;
    let known = serde_json::to_value(&parsed).unwrap_or_default();
    if let (Some(given), Some(known)) = (json.as_object(), known.as_object()) {
        if let Some(key) = given
            .iter()
            .find(|(key, value)| !value.is_null() && !known.contains_key(*key))
            .map(|(key, _)| key)
        {
            return Err(JsValue::from_str(&format!(
                "invalid {what}: unknown field `{key}`"
            )));
        }
    }
    Ok(parsed)
}

fn to_js<T: serde::Serialize>(value: &T) -> JsValue {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .unwrap_or(JsValue::NULL)
}

pub(super) fn snake_case(key: &str) -> String {
    let mut snake = String::with_capacity(key.len() + 4);
    for ch in key.chars() {
        if ch.is_ascii_uppercase() {
            snake.push('_');
            snake.push(ch.to_ascii_lowercase());
        } else {
            snake.push(ch);
        }
    }
    snake
}

/// `marginTop` → `margin_top`, recursively over object keys. The section
/// layout types speak snake_case (as the CLI and MCP tools write them); the
/// browser API speaks camelCase like the rest of the runtime.
pub(super) fn snake_keys(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .map(|(key, value)| (snake_case(&key), snake_keys(value)))
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(snake_keys).collect())
        }
        other => other,
    }
}

/// The reverse of [`snake_keys`], for what the browser reads back.
pub(super) fn camel_keys(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    let mut camel = String::with_capacity(key.len());
                    let mut upper = false;
                    for ch in key.chars() {
                        if ch == '_' {
                            upper = true;
                        } else if upper {
                            camel.push(ch.to_ascii_uppercase());
                            upper = false;
                        } else {
                            camel.push(ch);
                        }
                    }
                    (camel, camel_keys(value))
                })
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(camel_keys).collect())
        }
        other => other,
    }
}

fn from_camel_js<T: serde::de::DeserializeOwned>(value: JsValue, what: &str) -> Result<T, JsValue> {
    let json: serde_json::Value = from_js(value, what)?;
    serde_json::from_value(snake_keys(json))
        .map_err(|error| JsValue::from_str(&format!("invalid {what}: {error}")))
}

fn to_camel_js<T: serde::Serialize>(value: &T) -> JsValue {
    serde_json::to_value(value)
        .map(camel_keys)
        .map(|json| to_js(&json))
        .unwrap_or(JsValue::NULL)
}

/// Re-lay-out the whole document: these change every page, not only the
/// paragraphs under the selection.
fn whole_document() -> HeadlessApplyOverrides {
    HeadlessApplyOverrides {
        dirty_from: Some(0),
        ..Default::default()
    }
}

impl OffscreenDocumentView {
    pub(super) fn current_selection(&self) -> SelectionRange {
        let state = self.state.borrow();
        state.selection.unwrap_or_else(|| {
            self.renderer
                .borrow()
                .selection_range()
                .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
        })
    }

    /// The selection, or with a bare body caret the word around it — what
    /// Word's フォント dialog, ルビ and 囲い文字 act on. `None` when there is
    /// no text to act on.
    fn character_span(&self) -> Option<SelectionRange> {
        let selection = self.current_selection();
        if !selection.is_collapsed() {
            let (start, end) = selection.normalized();
            return Some(SelectionRange {
                anchor: start,
                focus: end,
            });
        }
        let (start, end) = self
            .renderer
            .borrow()
            .selection_manager()
            .word_range_at(selection.focus)?;
        (start < end).then(|| SelectionRange {
            anchor: TextPosition::new(selection.focus.paragraph, start),
            focus: TextPosition::new(selection.focus.paragraph, end),
        })
    }

    fn apply_body_headless(
        &self,
        commands: Vec<HeadlessCommand>,
        overrides: HeadlessApplyOverrides,
    ) -> Result<JsValue, JsValue> {
        if !self.body_commands_allowed() {
            return Ok(JsValue::NULL);
        }
        self.apply_paragraph_style_headless_many_with(move |_, _| commands, overrides)
    }
}

#[wasm_bindgen]
impl OffscreenDocumentView {
    /// Word's フォント dialog: applies every field `format` sets to the
    /// selection, or to the word at a bare caret, as one step. Fields left
    /// out stay as they are.
    #[wasm_bindgen(js_name = setRunFormat)]
    pub fn set_run_format(&self, format: JsValue) -> Result<JsValue, JsValue> {
        let format: HeadlessRunFormat = from_js_strict(format, "run format")?;
        if format == HeadlessRunFormat::default() {
            return Ok(JsValue::NULL);
        }
        let Some(span) = self.character_span() else {
            return Ok(JsValue::NULL);
        };
        self.apply_body_headless(
            vec![HeadlessCommand::SetRunFormat {
                start_paragraph: span.anchor.paragraph,
                start_offset: span.anchor.offset,
                end_paragraph: span.focus.paragraph,
                end_offset: span.focus.offset,
                format: Box::new(format),
            }],
            HeadlessApplyOverrides::default(),
        )
    }

    /// What the フォント dialog opens on: the direct formatting of the run
    /// at the start of the selection.
    #[wasm_bindgen(js_name = runFormatAtSelection)]
    pub fn run_format_at_selection(&self) -> JsValue {
        let (start, _) = self.current_selection().normalized();
        let state = self.state.borrow();
        state.word_doc.as_ref().map_or(JsValue::NULL, |document| {
            to_js(&document.run_format_at(start.paragraph, start.offset))
        })
    }

    /// ルビ over the selection (or the word at the caret) in one paragraph;
    /// `null` takes the ruby there off.
    #[wasm_bindgen(js_name = setRuby)]
    pub fn set_ruby(&self, ruby: JsValue) -> Result<JsValue, JsValue> {
        let ruby: Option<HeadlessRuby> = if ruby.is_null() || ruby.is_undefined() {
            None
        } else {
            Some(from_js_strict(ruby, "ruby")?)
        };
        let Some(span) = self.character_span() else {
            return Ok(JsValue::NULL);
        };
        if span.anchor.paragraph != span.focus.paragraph {
            return Err(JsValue::from_str("ruby must stay within one paragraph"));
        }
        self.apply_body_headless(
            vec![HeadlessCommand::SetRuby {
                paragraph: span.anchor.paragraph,
                start_offset: span.anchor.offset,
                end_offset: span.focus.offset,
                ruby,
            }],
            HeadlessApplyOverrides::default(),
        )
    }

    /// 囲い文字: the selection (or the word at the caret) replaced by `text`
    /// in an enclosure — `○`, `□`, `△` or `◇`.
    #[wasm_bindgen(js_name = insertEnclosedCharacter)]
    pub fn insert_enclosed_character(
        &self,
        text: String,
        enclosure: String,
    ) -> Result<JsValue, JsValue> {
        if text.trim().is_empty() {
            return Err(JsValue::from_str("enclosed text must not be empty"));
        }
        let span = self
            .character_span()
            .unwrap_or_else(|| self.current_selection());
        self.apply_body_headless(
            vec![HeadlessCommand::InsertEnclosedCharacter {
                start_paragraph: span.anchor.paragraph,
                start_offset: span.anchor.offset,
                end_paragraph: span.focus.paragraph,
                end_offset: span.focus.offset,
                text,
                enclosure,
            }],
            HeadlessApplyOverrides::default(),
        )
    }

    /// 日付と時刻 with 自動的に更新する: a `DATE` (or `TIME`) field with the
    /// `\@` picture `format` in place of the selection, showing `cachedText`.
    #[wasm_bindgen(js_name = insertDateTimeField)]
    pub fn insert_date_time_field(
        &self,
        format: String,
        cached_text: String,
        time: bool,
    ) -> Result<JsValue, JsValue> {
        let (start, end) = self.current_selection().normalized();
        self.apply_body_headless(
            vec![HeadlessCommand::InsertDateTimeField {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                time,
                format,
                cached_text,
            }],
            HeadlessApplyOverrides::default(),
        )
    }

    /// The 段落 dialog's タブとリーダー, 罫線と網かけ and the options beside
    /// them, on the selected paragraphs.
    #[wasm_bindgen(js_name = setParagraphOptions)]
    pub fn set_paragraph_options(&self, options: JsValue) -> Result<JsValue, JsValue> {
        let options: ParagraphOptionsJs = from_js(options, "paragraph options")?;
        let shading = if options.clear_shading {
            Some(None)
        } else {
            options.shading.map(Some)
        };
        self.apply_paragraph_style_headless(move |start, end| {
            HeadlessCommand::SetParagraphOptions {
                start_paragraph: start,
                end_paragraph: end,
                tabs: options.tabs,
                contextual_spacing: options.contextual_spacing,
                suppress_line_numbers: options.suppress_line_numbers,
                suppress_auto_hyphens: options.suppress_auto_hyphens,
                borders: options.borders,
                shading,
            }
        })
    }

    /// The 段落 dialog's 改ページと改行 options on the selected paragraphs.
    #[wasm_bindgen(js_name = setParagraphPagination)]
    pub fn set_paragraph_pagination(&self, options: JsValue) -> Result<JsValue, JsValue> {
        let options: ParagraphPaginationJs = from_js(options, "paragraph pagination")?;
        self.apply_paragraph_style_headless(move |start, end| {
            HeadlessCommand::SetParagraphPagination {
                start_paragraph: start,
                end_paragraph: end,
                keep_next: options.keep_next,
                keep_lines: options.keep_lines,
                page_break_before: options.page_break_before,
                widow_control: options.widow_control,
            }
        })
    }

    /// What the 段落 dialog's 改ページと改行 tab opens on, for the first
    /// selected paragraph.
    #[wasm_bindgen(js_name = paragraphPagination)]
    pub fn paragraph_pagination(&self) -> JsValue {
        let (start, _) = self.current_selection().normalized();
        let state = self.state.borrow();
        state.word_doc.as_ref().map_or(JsValue::NULL, |document| {
            to_js(&document.paragraph_pagination(start.paragraph))
        })
    }

    /// The 段落 dialog's 体裁 tab on the selected paragraphs.
    #[wasm_bindgen(js_name = setParagraphTypography)]
    pub fn set_paragraph_typography(&self, typography: JsValue) -> Result<JsValue, JsValue> {
        let typography: HeadlessParagraphTypography = from_js_strict(typography, "typography")?;
        self.apply_paragraph_style_headless(move |start, end| {
            HeadlessCommand::SetParagraphTypography {
                start_paragraph: start,
                end_paragraph: end,
                typography,
            }
        })
    }

    #[wasm_bindgen(js_name = paragraphTypography)]
    pub fn paragraph_typography(&self) -> JsValue {
        let (start, _) = self.current_selection().normalized();
        let state = self.state.borrow();
        state.word_doc.as_ref().map_or(JsValue::NULL, |document| {
            to_js(&document.paragraph_typography(start.paragraph))
        })
    }

    /// 1 ページの行数を指定時に文字を行グリッド線に合わせる on the selected
    /// paragraphs.
    #[wasm_bindgen(js_name = setParagraphSnapToGrid)]
    pub fn set_paragraph_snap_to_grid(&self, snap: bool) -> Result<JsValue, JsValue> {
        self.apply_paragraph_style_headless(move |start, end| {
            HeadlessCommand::SetParagraphSnapToGrid {
                start_paragraph: start,
                end_paragraph: end,
                snap,
            }
        })
    }

    /// The document's 文字体裁: kinsoku level and sets, and compression.
    #[wasm_bindgen(js_name = setJapaneseTypography)]
    pub fn set_japanese_typography(&self, typography: JsValue) -> Result<JsValue, JsValue> {
        let typography: HeadlessJapaneseTypography = from_js_strict(typography, "typography")?;
        self.apply_body_headless(
            vec![HeadlessCommand::SetJapaneseTypography { typography }],
            whole_document(),
        )
    }

    #[wasm_bindgen(js_name = japaneseTypography)]
    pub fn japanese_typography(&self) -> JsValue {
        let state = self.state.borrow();
        state.word_doc.as_ref().map_or(JsValue::NULL, |document| {
            to_js(&document.japanese_typography())
        })
    }

    /// Word's ハイフネーション dialog: `{ auto, zone, consecutiveLimit,
    /// hyphenateCaps }`, each optional.
    #[wasm_bindgen(js_name = setHyphenation)]
    pub fn set_hyphenation(&self, patch: JsValue) -> Result<JsValue, JsValue> {
        let patch: HeadlessHyphenationPatch = from_camel_js(patch, "hyphenation")?;
        self.apply_body_headless(
            vec![HeadlessCommand::SetHyphenation { patch }],
            whole_document(),
        )
    }

    /// The document's automatic hyphenation, in the dialog's terms.
    #[wasm_bindgen(js_name = hyphenation)]
    pub fn hyphenation(&self) -> JsValue {
        let state = self.state.borrow();
        state.word_doc.as_ref().map_or(JsValue::NULL, |document| {
            let settings = document.hyphenation();
            to_camel_js(&HeadlessHyphenationPatch {
                auto: Some(settings.auto),
                zone: settings.zone_twips.map(|twips| twips as f32 / 20.0),
                consecutive_limit: Some(settings.consecutive_limit.unwrap_or(0)),
                hyphenate_caps: Some(!settings.do_not_hyphenate_caps),
            })
        })
    }

    /// Word's ページ設定 dialog: applies what `patch` names to the sections
    /// `scope` reaches (`{ kind: "caret" | "caretToEnd" | "all" }`,
    /// `{ kind: "index", index }` or `{ kind: "range", first, last }`).
    #[wasm_bindgen(js_name = setSectionLayout)]
    pub fn set_section_layout(&self, scope: JsValue, patch: JsValue) -> Result<JsValue, JsValue> {
        let scope: HeadlessSectionScope = if scope.is_null() || scope.is_undefined() {
            HeadlessSectionScope::Caret
        } else {
            let mut json = snake_keys(from_js(scope, "section scope")?);
            // The tag's values are snake_case too (`caretToEnd` → `caret_to_end`).
            if let Some(kind) = json.get_mut("kind") {
                if let Some(text) = kind.as_str() {
                    *kind = serde_json::Value::String(snake_case(text));
                }
            }
            serde_json::from_value(json)
                .map_err(|error| JsValue::from_str(&format!("invalid section scope: {error}")))?
        };
        let patch: HeadlessSectionLayoutPatch = from_camel_js(patch, "section layout")?;
        if patch.is_empty() {
            return Ok(JsValue::NULL);
        }
        self.apply_body_headless(
            vec![HeadlessCommand::SetSectionLayout {
                scope,
                patch: Box::new(patch),
            }],
            whole_document(),
        )
    }

    /// The caret's section layout (or the one being edited in a header or
    /// footer) — what the ページ設定 dialog opens on.
    #[wasm_bindgen(js_name = sectionLayout)]
    pub fn section_layout(&self) -> JsValue {
        let section = self.ribbon_target_section();
        let state = self.state.borrow();
        state.word_doc.as_ref().map_or(JsValue::NULL, |document| {
            to_camel_js(&document.section_layout(section))
        })
    }

    /// Word's 前と同じヘッダー/フッター for the caret's section: `role` is
    /// `header` or `footer`, `variant` `default`, `first` or `even`.
    #[wasm_bindgen(js_name = setHeaderFooterLink)]
    pub fn set_header_footer_link(
        &self,
        role: String,
        variant: String,
        linked: bool,
    ) -> Result<JsValue, JsValue> {
        let Some(section) = self.ribbon_target_section() else {
            return Ok(JsValue::NULL);
        };
        let command = HeadlessCommand::SetHeaderFooterLink {
            section,
            role,
            variant,
            linked,
        };
        self.apply_body_headless(vec![command], whole_document())
    }

    /// Whether the caret section's story is linked to the previous section's
    /// (always `false` for the first section).
    #[wasm_bindgen(js_name = headerFooterLinked)]
    pub fn header_footer_linked(&self, role: String, variant: String) -> Result<bool, JsValue> {
        let Some(section) = self.ribbon_target_section() else {
            return Ok(false);
        };
        let state = self.state.borrow();
        let document = state
            .word_doc
            .as_ref()
            .ok_or_else(|| JsValue::from_str("No document loaded"))?;
        document
            .header_footer_linked(section, &role, &variant)
            .map_err(|error| JsValue::from_str(&error))
    }

    /// The caret section's header or footer story as a `.docx` of its own —
    /// its blocks, pictures and tables, with this document's styles — for a
    /// second editor to open: ヘッダーの編集 with everything Word allows in it.
    #[wasm_bindgen(js_name = headerFooterStoryDocx)]
    pub fn header_footer_story_docx(
        &self,
        role: String,
        variant: String,
    ) -> Result<Vec<u8>, JsValue> {
        let section = self.ribbon_target_section().unwrap_or(0);
        let mut state = self.state.borrow_mut();
        let document = state
            .word_doc
            .as_mut()
            .ok_or_else(|| JsValue::from_str("No document loaded"))?;
        // A document opened lazily has no round-trip XML until it is first
        // exported; the story is cut from the export.
        document
            .ensure_roundtrip_xml()
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        document
            .story_docx(section, &role, &variant, 1)
            .map_err(|error| JsValue::from_str(&error))
    }

    /// Writes a story edited as `headerFooterStoryDocx`'s document back into
    /// the caret section's header or footer, as one undo step.
    #[wasm_bindgen(js_name = replaceHeaderFooterStory)]
    pub fn replace_header_footer_story(
        &self,
        role: String,
        variant: String,
        bytes: Vec<u8>,
    ) -> Result<JsValue, JsValue> {
        let section = self.ribbon_target_section().unwrap_or(0);
        self.apply_document_command(move |_| {
            Box::new(crate::commands::snapshot_edit::SnapshotEditCommand::new(
                "word_replace_header_footer_story",
                move |document| {
                    document
                        .ensure_roundtrip_xml()
                        .map_err(|error| error.to_string())?;
                    document
                        .replace_story_from_docx(section, &role, &variant, &bytes)
                        .map(|_| ())
                },
            ))
        })
    }

    /// 表のプロパティ ▸ オプション ▸ セルの間隔 in points; `null` removes it.
    #[wasm_bindgen(js_name = setTableCellSpacing)]
    pub fn set_table_cell_spacing(&self, spacing: Option<f32>) -> Result<JsValue, JsValue> {
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        self.apply_body_headless(
            vec![HeadlessCommand::SetTableCellSpacing {
                table_order_index: cell.table_order_index,
                spacing,
            }],
            HeadlessApplyOverrides::default(),
        )
    }

    /// 文字列の方向 for every selected cell (`lrTb`, `tbRl`, `btLr`, …);
    /// `null` goes back to horizontal text.
    #[wasm_bindgen(js_name = setTableCellTextDirection)]
    pub fn set_table_cell_text_direction(
        &self,
        direction: Option<String>,
    ) -> Result<JsValue, JsValue> {
        let Some(range) = self.caret_table_range() else {
            return Ok(JsValue::NULL);
        };
        let commands = range
            .cells
            .iter()
            .map(|cell| HeadlessCommand::SetTableCellTextDirection {
                table_order_index: range.table_order_index,
                row: cell.row,
                column: cell.column,
                direction: direction.clone(),
            })
            .collect();
        self.apply_body_headless(commands, HeadlessApplyOverrides::default())
    }

    /// 左端からのインデント of the caret's table, in points.
    #[wasm_bindgen(js_name = setTableIndent)]
    pub fn set_table_indent(&self, indent: f32) -> Result<JsValue, JsValue> {
        if !indent.is_finite() {
            return Err(JsValue::from_str(
                "table indent must be a finite point value",
            ));
        }
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        self.apply_body_headless(
            vec![HeadlessCommand::SetTableIndent {
                table_order_index: cell.table_order_index,
                indent,
            }],
            HeadlessApplyOverrides::default(),
        )
    }

    /// 表の位置: floats the caret's table where `spec` says.
    #[wasm_bindgen(js_name = setTablePosition)]
    pub fn set_table_position(&self, spec: JsValue) -> Result<JsValue, JsValue> {
        let spec: TablePositionJs = from_js(spec, "table position")?;
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        self.apply_body_headless(
            vec![HeadlessCommand::SetTablePosition {
                table_order_index: cell.table_order_index,
                horizontal_align: spec.horizontal_align,
                horizontal_offset: spec.horizontal_offset,
                horizontal_anchor: spec.horizontal_anchor,
                vertical_align: spec.vertical_align,
                vertical_offset: spec.vertical_offset,
                vertical_anchor: spec.vertical_anchor,
                distance_top: spec.distance_top,
                distance_bottom: spec.distance_bottom,
                distance_left: spec.distance_left,
                distance_right: spec.distance_right,
                allow_overlap: spec.allow_overlap,
            }],
            HeadlessApplyOverrides::default(),
        )
    }

    /// 文字列を表にする: the selected paragraphs become one table, split into
    /// cells at `separator` (`tab`, `comma`, `paragraph` or one character).
    #[wasm_bindgen(js_name = convertTextToTable)]
    pub fn convert_text_to_table(
        &self,
        separator: String,
        columns: Option<usize>,
    ) -> Result<JsValue, JsValue> {
        if self.caret_table_cell().is_some() {
            return Err(JsValue::from_str("the selection is already in a table"));
        }
        let (start, end) = self.current_selection().normalized();
        self.apply_body_headless(
            vec![HeadlessCommand::ConvertTextToTable {
                start_paragraph: start.paragraph,
                end_paragraph: end.paragraph,
                separator,
                columns,
            }],
            HeadlessApplyOverrides::default(),
        )
    }

    /// 表の解除: the caret's table becomes paragraphs, cells joined by
    /// `separator`.
    #[wasm_bindgen(js_name = convertTableToText)]
    pub fn convert_table_to_text(&self, separator: String) -> Result<JsValue, JsValue> {
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        self.apply_body_headless(
            vec![HeadlessCommand::ConvertTableToText {
                table_order_index: cell.table_order_index,
                separator,
            }],
            HeadlessApplyOverrides::default(),
        )
    }

    /// 並べ替え: the caret table's rows after `headerRows`, by `keys`
    /// (`[{ column, kind: "text" | "number" | "date", descending }]`).
    #[wasm_bindgen(js_name = sortTable)]
    pub fn sort_table(&self, keys: JsValue, header_rows: usize) -> Result<JsValue, JsValue> {
        let keys: Vec<HeadlessTableSortKey> = from_js(keys, "sort keys")?;
        if keys.is_empty() {
            return Err(JsValue::from_str("sorting needs at least one key"));
        }
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        self.apply_body_headless(
            vec![HeadlessCommand::SortTable {
                table_order_index: cell.table_order_index,
                keys,
                header_rows,
            }],
            HeadlessApplyOverrides::default(),
        )
    }

    /// 計算式: an `=` field at the caret in a table cell, computed now —
    /// `=SUM(ABOVE)`, `=AVERAGE(LEFT)`, `=B2*C2`, … with an optional `\#`
    /// picture.
    #[wasm_bindgen(js_name = insertTableFormula)]
    pub fn insert_table_formula(
        &self,
        formula: String,
        number_format: Option<String>,
    ) -> Result<JsValue, JsValue> {
        if self.caret_table_cell().is_none() {
            return Ok(JsValue::NULL);
        }
        let selection = self.current_selection();
        let (start, _) = selection.normalized();
        self.apply_body_headless(
            vec![HeadlessCommand::InsertTableFormula {
                paragraph: start.paragraph,
                offset: start.offset,
                formula,
                number_format,
            }],
            HeadlessApplyOverrides::default(),
        )
    }

    /// フィールド更新 for the `=` fields of every table.
    #[wasm_bindgen(js_name = updateTableFormulas)]
    pub fn update_table_formulas(&self) -> Result<JsValue, JsValue> {
        self.apply_body_headless(vec![HeadlessCommand::UpdateTableFormulas], whole_document())
    }
}

#[cfg(test)]
mod tests {
    use super::{camel_keys, snake_keys};
    use serde_json::json;

    #[test]
    fn section_layout_keys_cross_the_boundary_in_camel_case() {
        let browser = json!({
            "marginTop": 72.0,
            "lineNumbering": { "enabled": true, "startAt": 1, "countBy": 5, "restart": "newPage" },
            "columns": { "count": 2, "space": 18.0, "equalWidth": true, "separator": false,
                         "columns": [{ "width": 100.0, "space": 10.0 }] }
        });
        let rust = snake_keys(browser.clone());
        assert_eq!(rust["margin_top"], 72.0);
        assert_eq!(rust["line_numbering"]["count_by"], 5);
        assert_eq!(rust["columns"]["equal_width"], true);
        assert_eq!(camel_keys(rust), browser);
    }
}
