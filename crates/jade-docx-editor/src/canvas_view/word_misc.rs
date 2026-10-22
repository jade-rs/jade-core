//! Smaller browser entry points: 表スタイルのオプション in full, 前の
//! コメント / 次のコメント, 見出しの折りたたみ, 図表番号 with chapter numbers
//! and the note separators.

use super::word_models::{from_js, to_js};
use super::*;
use crate::word::references::captions::{WordCaptions, CHAPTER_SEPARATORS};
use crate::word::references::navigation::WordNavigation;
use crate::word::references::notes::WordNotes;
use crate::word::references::NoteKind;
use crate::word::WordSession;
use serde_json::json;

/// 図表番号の挿入's options; a field left out takes the dialog's default.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
struct CaptionJs {
    label: String,
    text: String,
    above: Option<bool>,
    exclude_label: Option<bool>,
    /// 章番号を含める.
    chapter: bool,
    /// 章タイトルのスタイル: the heading level that starts a chapter.
    chapter_level: Option<u8>,
    /// 区切り文字: `-`, `.`, `:`, `—` or `–`.
    separator: Option<String>,
}

/// 表スタイルのオプション; a field left out stays.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
struct TableLookJs {
    first_row: Option<bool>,
    last_row: Option<bool>,
    banded_rows: Option<bool>,
    first_column: Option<bool>,
    last_column: Option<bool>,
    banded_columns: Option<bool>,
}

#[wasm_bindgen]
impl OffscreenDocumentView {
    /// 表スタイルのオプション for the caret's table: タイトル行, 集計行,
    /// 縞模様 (行), 最初の列, 最後の列, 縞模様 (列).
    #[wasm_bindgen(js_name = setTableLookOptions)]
    pub fn set_table_look_options(&self, look: JsValue) -> Result<JsValue, JsValue> {
        let look: TableLookJs = from_js(look, "table look")?;
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        self.apply_paragraph_style_headless(move |_, _| HeadlessCommand::SetTableLook {
            table_order_index: cell.table_order_index,
            first_row: look.first_row,
            banded_rows: look.banded_rows,
            last_row: look.last_row,
            first_column: look.first_column,
            last_column: look.last_column,
            banded_columns: look.banded_columns,
        })
    }

    /// 前のコメント / 次のコメント from the caret: the comment's id and its
    /// anchor as story places, or `null` when there is none that way.
    #[wasm_bindgen(js_name = stepComment)]
    pub fn step_comment(&self, forwards: bool) -> Result<JsValue, JsValue> {
        let caret = self.current_selection().focus;
        let found = self.read_word_session(|session| {
            let document = session.word_document();
            session
                .step_comment((caret.paragraph, caret.offset), forwards)
                .and_then(|comment| {
                    let (start, end) = comment.anchor?;
                    let place = |(paragraph, offset): (usize, usize)| {
                        document.story_position_for_surface(paragraph).map(|place| {
                            json!({
                                "story": place.story,
                                "paragraph": place.index,
                                "offset": offset,
                                "anchorParagraph": place.anchor_paragraph,
                                "noteId": place.note_id,
                            })
                        })
                    };
                    Some(json!({
                        "id": comment.id,
                        "author": comment.author,
                        "text": comment.text,
                        "start": place(start)?,
                        "end": place(end)?,
                    }))
                })
        })?;
        Ok(found.map_or(JsValue::NULL, |value| to_js(&value)))
    }

    /// 見出しの折りたたみ: the headings at body paragraphs `headings` shown
    /// collapsed, everything in their sections hidden; empty expands them
    /// all. The document is laid out again.
    #[wasm_bindgen(js_name = setCollapsedHeadings)]
    pub fn set_collapsed_headings(&self, headings: Vec<usize>) -> Result<JsValue, JsValue> {
        let hidden = self.read_word_session(|session| {
            let document = session.word_document();
            let collapsed: std::collections::BTreeSet<usize> = headings
                .iter()
                .filter_map(|body| document.surface_index_for_body(*body))
                .collect();
            session.hidden_by_collapse(&collapsed)
        })?;
        self.state
            .borrow_mut()
            .options
            .set_collapsed_blocks(hidden.clone());
        self.rerender_after_display_change(move |renderer| renderer.set_collapsed_blocks(hidden))
    }

    /// 図表番号の挿入 with its options — above or below, the label left out,
    /// and 章番号を含める (`STYLEREF n \s`, `SEQ \s n`) — at the caret's
    /// paragraph, as one undo step.
    #[wasm_bindgen(js_name = insertCaptionWithOptions)]
    pub fn insert_caption_with_options(&self, caption: JsValue) -> Result<JsValue, JsValue> {
        let caption: CaptionJs = from_js(caption, "caption")?;
        let separator = match caption.separator.as_deref() {
            None => None,
            Some(value) => Some(
                CHAPTER_SEPARATORS
                    .into_iter()
                    .find(|known| *known == value)
                    .ok_or_else(|| JsValue::from_str(&format!("unknown separator {value}")))?,
            ),
        };
        let anchor = self.current_selection().focus.paragraph;
        self.run_word_session("word_insert_caption", move |session| {
            let mut dialog = session.caption_dialog(&[]);
            dialog.label = caption.label;
            if let Some(above) = caption.above {
                dialog.above = above;
            }
            if let Some(exclude) = caption.exclude_label {
                dialog.exclude_label = exclude;
            }
            dialog.chapter = caption.chapter;
            if let Some(level) = caption.chapter_level {
                dialog.chapter_level = level.clamp(1, 9);
            }
            if let Some(separator) = separator {
                dialog.separator = separator;
            }
            session
                .insert_caption(&dialog, anchor, &caption.text)
                .map(|_| ())
        })
    }

    /// 脚注と文末脚注 ▸ 境界線 and 継続境界線 for `kind` (`footnote` or
    /// `endnote`): whether their separator rules are drawn.
    #[wasm_bindgen(js_name = setNoteSeparators)]
    pub fn set_note_separators(
        &self,
        kind: String,
        separator: bool,
        continuation: bool,
    ) -> Result<JsValue, JsValue> {
        let kind = match kind.as_str() {
            "footnote" => NoteKind::Footnote,
            "endnote" => NoteKind::Endnote,
            other => return Err(JsValue::from_str(&format!("unknown note kind {other}"))),
        };
        self.run_word_session("word_note_separators", move |session| {
            let mut dialog = session.note_dialog(kind);
            dialog.separator = separator;
            dialog.continuation_separator = continuation;
            session.apply_note_dialog(&dialog)
        })
    }

    /// Whether `kind`'s separator and continuation separator draw their rules.
    #[wasm_bindgen(js_name = noteSeparators)]
    pub fn note_separators(&self, kind: String) -> Result<JsValue, JsValue> {
        let kind = match kind.as_str() {
            "footnote" => NoteKind::Footnote,
            "endnote" => NoteKind::Endnote,
            other => return Err(JsValue::from_str(&format!("unknown note kind {other}"))),
        };
        let dialog = self.read_word_session(|session| session.note_dialog(kind))?;
        Ok(to_js(&json!({
            "separator": dialog.separator,
            "continuationSeparator": dialog.continuation_separator,
        })))
    }

    /// 段落 ▸ インデント in 字 (Japanese Word's character units, as
    /// `w:leftChars` and friends keep them): `left`, `right` and either
    /// `firstLine` or `hanging`, each in characters (0.5 steps); a field left
    /// out stays.
    #[wasm_bindgen(js_name = setParagraphIndentChars)]
    pub fn set_paragraph_indent_chars(&self, indent: JsValue) -> Result<JsValue, JsValue> {
        #[derive(serde::Deserialize, Default)]
        #[serde(rename_all = "camelCase", deny_unknown_fields, default)]
        struct IndentCharsJs {
            left: Option<f32>,
            right: Option<f32>,
            first_line: Option<f32>,
            hanging: Option<f32>,
        }
        let indent: IndentCharsJs = from_js(indent, "indent")?;
        let hundredths = |chars: Option<f32>| chars.map(|chars| (chars * 100.0).round() as i32);
        self.apply_paragraph_style_headless(move |start, end| HeadlessCommand::SetParagraphIndent {
            start_paragraph: start,
            end_paragraph: end,
            left: None,
            right: None,
            first_line: None,
            hanging: None,
            left_chars: hundredths(indent.left),
            right_chars: hundredths(indent.right),
            first_line_chars: hundredths(indent.first_line),
            hanging_chars: hundredths(indent.hanging),
            clear_special: false,
        })
    }

    /// 上書き入力 (Insert key): `text` typed over the characters after the
    /// caret, as many as it has, never past the paragraph's end; with a
    /// selection, the selection is replaced as usual. One undo step.
    #[wasm_bindgen(js_name = insertTextOvertype)]
    pub fn insert_text_overtype(&self, text: String) -> Result<JsValue, JsValue> {
        let selection = self.current_selection();
        let (start, end) = selection.normalized();
        let end_offset = if selection.is_collapsed() {
            let state = self.state.borrow();
            let paragraph_text: String = state
                .word_doc
                .as_ref()
                .and_then(|document| document.paragraph(start.paragraph))
                .map(|paragraph| paragraph.runs.iter().map(|run| run.text.as_str()).collect())
                .unwrap_or_default();
            let rest = paragraph_text.get(start.offset..).unwrap_or("");
            start.offset
                + rest
                    .char_indices()
                    .nth(text.chars().count())
                    .map_or(rest.len(), |(at, _)| at)
        } else {
            end.offset
        };
        let end_paragraph = if selection.is_collapsed() {
            start.paragraph
        } else {
            end.paragraph
        };
        let caret = TextPosition::new(start.paragraph, start.offset + text.len());
        self.apply_paragraph_style_headless_many_with(
            move |_, _| {
                vec![HeadlessCommand::ReplaceRange {
                    start_paragraph: start.paragraph,
                    start_offset: start.offset,
                    end_paragraph,
                    end_offset,
                    text,
                }]
            },
            HeadlessApplyOverrides {
                dirty_from: Some(start.paragraph),
                selection_after: Some(Box::new(move |_| Some(SelectionRange::collapsed(caret)))),
            },
        )
    }

    /// Whether the point (`x` page-local, `y` down the pages, as pointer
    /// events give them) is inside the selection: where a press starts
    /// dragging the selected text rather than a new selection.
    #[wasm_bindgen(js_name = selectionContainsPoint)]
    pub fn selection_contains_point(&self, x: f64, y: f64) -> bool {
        let selection = self.current_selection();
        if selection.is_collapsed() {
            return false;
        }
        let Some(at) = self.renderer.borrow().hit_test_position(x, y) else {
            return false;
        };
        let (start, end) = selection.normalized();
        (start.paragraph, start.offset) < (at.paragraph, at.offset)
            && (at.paragraph, at.offset) < (end.paragraph, end.offset)
    }

    /// ドラッグ アンド ドロップ: the selected text, with its formatting, put
    /// where the pointer let go (`x`, `y` as pointer events give them) —
    /// moved, or copied with `copy` — as one undo step; the dropped text is
    /// selected after. `null` when it was let go inside the selection.
    #[wasm_bindgen(js_name = dropSelectionAt)]
    pub fn drop_selection_at(&self, x: f64, y: f64, copy: bool) -> Result<JsValue, JsValue> {
        use crate::word::clipboard::WordClipboard;
        let selection = self.current_selection();
        let Some(target) = self.renderer.borrow().hit_test_position(x, y) else {
            return Ok(JsValue::NULL);
        };
        if selection.is_collapsed() || self.selection_contains_point(x, y) {
            return Ok(JsValue::NULL);
        }
        let (start, end) = selection.normalized();
        self.run_word_session("word_drop_text", move |session| {
            let clip = session.copy_rich().ok_or("nothing is selected")?;
            let moved_from =
                (!copy).then_some(((start.paragraph, start.offset), (end.paragraph, end.offset)));
            let dropped_end =
                session.drop_rich(&clip, moved_from, (target.paragraph, target.offset))?;
            session.set_selection_range(Some(SelectionRange::collapsed(TextPosition::new(
                dropped_end.0,
                dropped_end.1,
            ))));
            Ok(())
        })
    }
}
