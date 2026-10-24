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

impl OffscreenDocumentView {
    /// Refresh paragraph-indexed view snapshots inside a structural edit's
    /// existing render. Callers must not use this for ordinary text input.
    pub(super) fn refresh_outline_after_structure(&self) -> Result<Option<Options>, JsValue> {
        let active = {
            let state = self.state.borrow();
            if !state.outline_display.active && state.outline_display.collapsed.is_empty() {
                return Ok(None);
            }
            state.outline_display.active
        };
        let hidden = self.refresh_outline_display(|_, _| {})?;
        let indents = if active {
            let state = self.state.borrow();
            state
                .word_doc
                .as_ref()
                .map(|document| Rc::new(crate::word::outline::indents(document)))
        } else {
            None
        };
        let mut renderer = self.renderer.borrow_mut();
        renderer.set_outline_indents(indents);
        renderer.set_collapsed_blocks(hidden);
        Ok(Some(self.state.borrow().options.clone()))
    }

    pub(super) fn refresh_outline_display(
        &self,
        update: impl FnOnce(&mut crate::word::outline::OutlineDisplay, &WordDocument),
    ) -> Result<Vec<usize>, JsValue> {
        let mut display = self.state.borrow().outline_display.clone();
        let (display, hidden) = self.read_word_session(|session| {
            update(&mut display, session.word_document());
            let hidden = display.hidden_blocks(session);
            (display, hidden)
        })?;
        let mut state = self.state.borrow_mut();
        state.outline_display = display;
        state.options.set_collapsed_blocks(hidden.clone());
        Ok(hidden)
    }
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
        let hidden = self.refresh_outline_display(|display, document| {
            display.collapsed = headings
                .iter()
                .filter_map(|body| document.surface_index_for_body(*body))
                .collect();
        })?;
        self.rerender_after_display_change(move |renderer| renderer.set_collapsed_blocks(hidden))
    }

    /// Navigation and Outline share this document's fold preferences. Reading
    /// them does not relayout the document or change its undo history.
    #[wasm_bindgen(js_name = outlineDisplay)]
    pub fn outline_display(&self) -> Result<JsValue, JsValue> {
        let mut display = self.state.borrow().outline_display.clone();
        self.read_word_session(|session| {
            display.hidden_blocks(session);
            let collapsed: Vec<_> = display
                .collapsed
                .iter()
                .filter_map(|surface| session.word_document().body_index_for_surface(*surface))
                .collect();
            to_js(&json!({ "collapsedHeadings": collapsed, "shownLevel": display.shown }))
        })
    }

    /// レベルの表示: Heading 1–9, or 10 for all levels including body. This
    /// filter only applies in Outline and is independent of navigation folds.
    #[wasm_bindgen(js_name = setOutlineShownLevel)]
    pub fn set_outline_shown_level(&self, level: u8) -> Result<JsValue, JsValue> {
        if !(1..=crate::word::outline::BODY_LEVEL).contains(&level) {
            return Err(JsValue::from_str("outline level must be between 1 and 10"));
        }
        let hidden = self.refresh_outline_display(|display, _| display.shown = level)?;
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

    /// 脚注の配置: the caret's section's footnotes at the bottom of the page
    /// (`false`, ページの最後) or right after the text (`true`, 文字列の直後),
    /// one undo step.
    #[wasm_bindgen(js_name = setFootnoteBeneathText)]
    pub fn set_footnote_beneath_text(&self, beneath: bool) -> Result<JsValue, JsValue> {
        use crate::word::references::notes::FootnotePlace;
        self.run_word_session("word_footnote_place", move |session| {
            let mut dialog = session.note_dialog(NoteKind::Footnote);
            dialog.footnote_place = if beneath {
                FootnotePlace::BeneathText
            } else {
                FootnotePlace::PageBottom
            };
            session.apply_note_dialog(&dialog)
        })
    }

    /// Whether the caret's section puts its footnotes right after the text.
    #[wasm_bindgen(js_name = footnoteBeneathText)]
    pub fn footnote_beneath_text(&self) -> Result<bool, JsValue> {
        use crate::word::references::notes::FootnotePlace;
        self.read_word_session(|session| {
            session.note_dialog(NoteKind::Footnote).footnote_place == FootnotePlace::BeneathText
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
                dirty_to: None,
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

    /// 罫線を引く: the pencil's stroke from (`from_x`, `from_y`) to (`to_x`,
    /// `to_y`), as pointer events give them. Across a cell it splits the
    /// cell — into two rows when mostly horizontal, two columns when
    /// vertical; a rectangle outside any table becomes a one-cell table that
    /// size where its top-left corner is. One undo step; `null` when the
    /// stroke drew nothing.
    #[wasm_bindgen(js_name = drawTableStroke)]
    pub fn draw_table_stroke(
        &self,
        from_x: f64,
        from_y: f64,
        to_x: f64,
        to_y: f64,
    ) -> Result<JsValue, JsValue> {
        use crate::word::table::WordTables;
        let (from, to) = ((from_x, from_y), (to_x, to_y));
        let middle = ((from_x + to_x) / 2.0, (from_y + to_y) / 2.0);
        let in_cell = self.read_word_session(|session| {
            session.cell_layouts().iter().any(|cell| {
                middle.0 > cell.left
                    && middle.0 < cell.right
                    && middle.1 > cell.top
                    && middle.1 < cell.bottom
            })
        })?;
        let at = self
            .renderer
            .borrow()
            .hit_test_position(from_x.min(to_x), from_y.min(to_y))
            .map(|at| (at.paragraph, at.offset));
        if !in_cell && (at.is_none() || (to_x - from_x).abs() * 0.75 < 9.0) {
            return Ok(JsValue::NULL);
        }
        self.run_word_session("word_draw_table", move |session| {
            session.draw_stroke(from, to, at).map(|_| ())
        })
    }

    /// 罫線の削除: the eraser on the border at (`x`, `y`) — the two cells it
    /// separates become one. One undo step; `null` when no border is there.
    #[wasm_bindgen(js_name = eraseTableBorder)]
    pub fn erase_table_border(&self, x: f64, y: f64) -> Result<JsValue, JsValue> {
        use crate::word::table::{TableHit, WordTables};
        let border = self.read_word_session(|session| {
            matches!(
                session.table_hit(x, y),
                Some(TableHit::ColumnBorder { .. } | TableHit::RowBorder { .. })
            )
        })?;
        if !border {
            return Ok(JsValue::NULL);
        }
        self.run_word_session("word_erase_border", move |session| {
            session.erase_border(x, y).map(|_| ())
        })
    }

    /// レイアウト ▸ 選択: the cell, row or column the caret is in, or the
    /// whole table (`part`: `cell`, `row`, `column`, `table`), as a cell
    /// selection.
    #[wasm_bindgen(js_name = selectTablePart)]
    pub fn select_table_part_js(&self, part: &str) -> Result<(), JsValue> {
        use crate::word::table::{TablePart, WordTables};
        let part = match part {
            "cell" => TablePart::Cell,
            "row" => TablePart::Row,
            "column" => TablePart::Column,
            "table" => TablePart::Table,
            other => return Err(JsValue::from_str(&format!("unknown table part {other}"))),
        };
        self.select_with_word_session(move |session| session.select_table_part(part))
    }

    /// 校閲者: everyone who made a change or a comment, in the order they
    /// first appear — the list 変更履歴とコメントの表示 ticks.
    #[wasm_bindgen(js_name = markupReviewers)]
    pub fn markup_reviewers(&self) -> Result<JsValue, JsValue> {
        use crate::word::markup::WordMarkup;
        let reviewers = self.read_word_session(|session| session.reviewers())?;
        Ok(to_js(&reviewers))
    }

    /// 変更内容の表示 and 変更履歴とコメントの表示: `mode` (`simple`, `all`,
    /// `none`, `original`), whether insertions and deletions and formatting
    /// are marked up, and the reviewers whose changes are not. Only what is
    /// drawn changes.
    #[wasm_bindgen(js_name = setMarkupView)]
    pub fn set_markup_view(
        &self,
        mode: &str,
        insertions_deletions: bool,
        formatting: bool,
        hidden_reviewers: Vec<String>,
    ) -> Result<JsValue, JsValue> {
        use crate::word::markup::{MarkupMode, MarkupOptions, WordMarkup};
        let mode = match mode {
            "simple" => MarkupMode::Simple,
            "all" => MarkupMode::All,
            "none" | "final" => MarkupMode::NoMarkup,
            "original" => MarkupMode::Original,
            other => return Err(JsValue::from_str(&format!("unknown markup view {other}"))),
        };
        let options = MarkupOptions {
            mode,
            insertions_deletions,
            formatting,
            hidden_reviewers,
            ..MarkupOptions::default()
        };
        let reviewers = self.read_word_session(|session| session.reviewers())?;
        let (render_changes, render_original, view) = options.display(&reviewers);
        {
            let mut state_mut = self.state.borrow_mut();
            state_mut.options.revision_display_explicit = true;
            state_mut.options.render_changes = render_changes;
            state_mut.options.render_original = render_original;
            state_mut.options.set_revision_view(view.clone());
        }
        self.rerender_after_display_change(move |renderer| {
            renderer.set_revision_display_flags(render_changes, render_original);
            renderer.set_revision_view_flag(view);
        })
    }

    /// 引用文の登録: a `TA` field for the selection's citation — its long
    /// form, short form and 1-based category — at the selection's end, one
    /// undo step.
    #[wasm_bindgen(js_name = markCitation)]
    pub fn mark_citation_js(
        &self,
        long: String,
        short: String,
        category: u8,
    ) -> Result<JsValue, JsValue> {
        use crate::word::references::index::{MarkCitationDialog, WordIndex};
        self.run_word_session("word_mark_citation", move |session| {
            session.mark_citation(&MarkCitationDialog {
                long,
                short,
                category: category.max(1),
            })
        })
    }

    /// 引用文献一覧の挿入: a `TOA` field per category (or for `category`),
    /// with 前掲 for a citation on five pages or more when `passim`, after
    /// the caret's paragraph; one undo step.
    #[wasm_bindgen(js_name = insertTableOfAuthorities)]
    pub fn insert_table_of_authorities_js(
        &self,
        category: Option<u8>,
        passim: bool,
    ) -> Result<JsValue, JsValue> {
        use crate::word::references::index::{AuthoritiesDialog, WordIndex};
        self.run_word_session("word_table_of_authorities", move |session| {
            session.insert_table_of_authorities(&AuthoritiesDialog {
                category,
                passim,
                ..AuthoritiesDialog::default()
            })
        })
    }

    /// 引用文献一覧の更新: each table built again; the result is how many.
    #[wasm_bindgen(js_name = updateTablesOfAuthorities)]
    pub fn update_tables_of_authorities_js(&self) -> Result<JsValue, JsValue> {
        use crate::word::references::index::WordIndex;
        self.run_word_session("word_update_authorities", move |session| {
            session.update_tables_of_authorities()
        })
    }
}
