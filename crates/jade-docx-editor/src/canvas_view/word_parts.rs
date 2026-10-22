//! Browser entry points for クイック パーツ and 定型句 — the glossary's
//! building blocks — from [`crate::word::building_blocks`], and 音声読み上げ's
//! paragraphs from [`crate::word::read_aloud`], run on the view as a session.

use super::word_models::to_js;
use super::*;
use crate::word::building_blocks::{Gallery, WordBuildingBlocks};
use crate::word::WordSession;
use serde_json::json;

fn gallery(name: &str) -> Result<Gallery, JsValue> {
    match name {
        "quickParts" => Ok(Gallery::QuickParts),
        "autoText" => Ok(Gallery::AutoText),
        other => Err(JsValue::from_str(&format!("unknown gallery {other}"))),
    }
}

#[wasm_bindgen]
impl OffscreenDocumentView {
    /// The glossary's entries, in order: `index` names one for
    /// `insertQuickPart` and `deleteQuickPart`.
    #[wasm_bindgen(js_name = quickParts)]
    pub fn quick_parts(&self) -> Result<JsValue, JsValue> {
        let parts = self.read_word_session(|session| {
            session
                .building_blocks()
                .iter()
                .enumerate()
                .map(|(index, block)| {
                    json!({
                        "index": index,
                        "name": block.name,
                        "gallery": match block.gallery {
                            Gallery::QuickParts => "quickParts",
                            Gallery::AutoText => "autoText",
                        },
                        "category": block.category,
                    })
                })
                .collect::<Vec<_>>()
        })?;
        Ok(to_js(&parts))
    }

    /// 選択範囲をクイック パーツ ギャラリーに保存 (`quickParts`) / 定型句に
    /// 保存 (`autoText`).
    #[wasm_bindgen(js_name = saveQuickPart)]
    pub fn save_quick_part(&self, name: String, gallery_name: String) -> Result<JsValue, JsValue> {
        let gallery = gallery(&gallery_name)?;
        if name.trim().is_empty() {
            return Err(JsValue::from_str("a building block needs a name"));
        }
        self.run_word_session("word_save_building_block", move |session| {
            session.save_building_block(name.trim(), gallery)
        })
    }

    /// Entry `index` at the caret, over the selection, with its formatting.
    #[wasm_bindgen(js_name = insertQuickPart)]
    pub fn insert_quick_part(&self, index: usize) -> Result<JsValue, JsValue> {
        self.run_word_session("word_insert_building_block", move |session| {
            let (paragraph, offset) = session.insert_building_block(index)?;
            session.set_selection_range(Some(SelectionRange::collapsed(TextPosition::new(
                paragraph, offset,
            ))));
            Ok(())
        })
    }

    /// 読み上げ from the caret: each paragraph with something to say, as
    /// `{paragraph, offset, story, anchorParagraph, noteId, text}` — the
    /// place its `text` starts, as `setSelectionRange` takes it. The voice
    /// is the browser's; its word boundaries are UTF-16 ranges of `text`.
    #[wasm_bindgen(js_name = readAloudUtterances)]
    pub fn read_aloud_utterances(&self) -> Result<JsValue, JsValue> {
        let caret = self.current_selection().focus;
        let utterances = self.read_word_session(|session| {
            let document = session.word_document();
            crate::word::read_aloud::utterances(session, (caret.paragraph, caret.offset))
                .into_iter()
                .filter_map(|utterance| {
                    let place = document.story_position_for_surface(utterance.paragraph)?;
                    Some(json!({
                        "story": place.story,
                        "paragraph": place.index,
                        "offset": utterance.offset,
                        "anchorParagraph": place.anchor_paragraph,
                        "noteId": place.note_id,
                        "text": utterance.text,
                    }))
                })
                .collect::<Vec<_>>()
        })?;
        Ok(to_js(&utterances))
    }

    /// 表示 ▸ イマーシブ リーダー: each body paragraph with text as the reader
    /// shows it (テキストの間隔, 音節), with the reader's column widths, page
    /// colours and line metrics at its font size.
    #[wasm_bindgen(js_name = immersiveReader)]
    pub fn immersive_reader(
        &self,
        syllables: bool,
        text_spacing: bool,
    ) -> Result<JsValue, JsValue> {
        use crate::word::immersive::{shown_paragraphs, ColumnWidth, ImmersiveSettings, PageColor};
        let settings = ImmersiveSettings {
            syllables,
            text_spacing,
            ..ImmersiveSettings::default()
        };
        let paragraphs = self.read_word_session(|session| {
            let document = session.word_document();
            shown_paragraphs(session, &settings)
                .into_iter()
                .filter_map(|shown| {
                    Some(json!({
                        "paragraph": document.body_index_for_surface(shown.paragraph)?,
                        "text": shown.text,
                    }))
                })
                .collect::<Vec<_>>()
        })?;
        let hex = |color: u32| format!("#{color:06X}");
        Ok(to_js(&json!({
            "paragraphs": paragraphs,
            "fontSize": settings.font_size,
            "lineHeight": settings.line_height(),
            "paragraphGap": settings.paragraph_gap(),
            "columns": ColumnWidth::ALL.iter().map(|column| json!({
                "name": column.label_key().rsplit('.').next(),
                "width": column.pixels(),
            })).collect::<Vec<_>>(),
            "pages": PageColor::ALL.iter().map(|page| {
                let (background, text) = page.colors();
                json!({
                    "name": page.label_key().rsplit('.').next(),
                    "background": hex(background),
                    "text": hex(text),
                })
            }).collect::<Vec<_>>(),
        })))
    }

    /// 表示 ▸ 表示: `print` (印刷レイアウト, the sections' pages), `web` (Web
    /// レイアウト, the text wrapped to a window `width`×`height` CSS pixels
    /// at `zoom`, no page chrome) or `outline` (アウトライン, the headings
    /// and body text set in by level). The document is laid out again; the
    /// result carries the new pages.
    #[wasm_bindgen(js_name = setViewMode)]
    pub fn set_view_mode(
        &self,
        mode: String,
        width: f32,
        height: f32,
        zoom: f32,
    ) -> Result<JsValue, JsValue> {
        use crate::word::view::{outline_geometry, view_geometry, ViewKind};
        let (geometry, indents) = {
            let state = self.state.borrow();
            let document = state
                .word_doc
                .as_ref()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;
            match mode.as_str() {
                "print" => (None, None),
                "web" => (view_geometry(ViewKind::Web, (width, height), zoom), None),
                "outline" => (
                    outline_geometry(&document.section_layout(Some(0))),
                    Some(std::rc::Rc::new(crate::word::outline::indents(document))),
                ),
                other => return Err(JsValue::from_str(&format!("unknown view {other}"))),
            }
        };
        self.rerender_after_display_change(move |renderer| {
            renderer.set_view_layout(geometry, indents);
        })
    }

    /// 下書き: what of a page the view keeps — its text band, with a strip of
    /// the left margin — for the caret's section, as `{top, height, left,
    /// width, pageWidth, pageHeight}` in the page's CSS pixels.
    /// With `hide_white_space`, 余白を表示しない's crop instead: the page's
    /// width, cut above and below its text.
    #[wasm_bindgen(js_name = draftCrop)]
    pub fn draft_crop(&self, hide_white_space: Option<bool>) -> Result<JsValue, JsValue> {
        use crate::word::view::{points_to_px, PageBand, ViewKind};
        let section = self.ribbon_target_section();
        let state = self.state.borrow();
        let document = state
            .word_doc
            .as_ref()
            .ok_or_else(|| JsValue::from_str("No document loaded"))?;
        let layout = document.section_layout(section);
        let page = (points_to_px(layout.width), points_to_px(layout.height));
        let kind = if hide_white_space.unwrap_or(false) {
            ViewKind::Print
        } else {
            ViewKind::Draft
        };
        let crop = PageBand::of(&layout).crop(kind, page);
        Ok(to_js(&json!({
            "top": crop.top,
            "height": crop.height,
            "left": crop.left,
            "width": crop.width,
            "pageWidth": page.0,
            "pageHeight": page.1,
        })))
    }

    /// 文書パーツ オーガナイザー's 削除.
    #[wasm_bindgen(js_name = deleteQuickPart)]
    pub fn delete_quick_part(&self, index: usize) -> Result<JsValue, JsValue> {
        self.run_word_session("word_delete_building_block", move |session| {
            session.delete_building_block(index)
        })
    }
}
