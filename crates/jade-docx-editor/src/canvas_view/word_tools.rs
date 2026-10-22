//! Browser entry points for 検索と置換's オプション, the ナビゲーション
//! pane and ジャンプ, アクセシビリティ チェック and the デザイン tab — the
//! [`crate::word`] models run on the view as a session.
//!
//! The models count paragraphs as the document's surfaces; what goes out to
//! script is a story position (`body`, `textbox`, `footnote`…), as
//! `setSelectionRange` and `setSelectionRangeInStory` take them.

use super::word_models::{from_js, to_js};
use super::*;
use crate::word::accessibility::{IssueKind, Severity, WordAccessibility};
use crate::word::design::{
    style_sets, WordDesign, COLOR_SCHEMES, EFFECT_SCHEMES, FONT_SCHEMES, PARAGRAPH_SPACINGS, THEMES,
};
use crate::word::references::navigation::{GoTo, GoToTarget, WordNavigation};
use crate::word::search::{Direction, FormatFilter, Scope, SearchOptions, WordSearch};
use crate::word::WordSession;
use serde_json::{json, Value};

/// 書式: each field set is a condition, or a change for the replacement.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
struct FormatFilterJs {
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    font_family: Option<String>,
    font_size: Option<f32>,
    color: Option<String>,
    highlight: Option<String>,
    style: Option<String>,
}

impl From<FormatFilterJs> for FormatFilter {
    fn from(js: FormatFilterJs) -> Self {
        FormatFilter {
            bold: js.bold,
            italic: js.italic,
            underline: js.underline,
            font_family: js.font_family,
            font_size: js.font_size,
            color: js.color,
            highlight: js.highlight,
            style: js.style,
        }
    }
}

/// 検索と置換's オプション.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
struct SearchOptionsJs {
    match_case: bool,
    whole_word: bool,
    wildcards: bool,
    match_prefix: bool,
    match_suffix: bool,
    match_width: bool,
    match_kana: bool,
    ignore_punctuation: bool,
    ignore_whitespace: bool,
    /// `down`, `up` or `all`.
    direction: Option<String>,
    /// `main`, `notes` or `selection`.
    scope: Option<String>,
    format: FormatFilterJs,
    replace_format: FormatFilterJs,
}

fn search_options(value: JsValue) -> Result<SearchOptions, JsValue> {
    let js: SearchOptionsJs = if value.is_undefined() || value.is_null() {
        SearchOptionsJs::default()
    } else {
        from_js(value, "search options")?
    };
    let direction = match js.direction.as_deref() {
        None | Some("down") => Direction::Down,
        Some("up") => Direction::Up,
        Some("all") => Direction::All,
        Some(other) => return Err(JsValue::from_str(&format!("unknown direction {other}"))),
    };
    let scope = match js.scope.as_deref() {
        None | Some("main") => Scope::Main,
        Some("notes") => Scope::Notes,
        Some("selection") => Scope::Selection,
        Some(other) => return Err(JsValue::from_str(&format!("unknown scope {other}"))),
    };
    Ok(SearchOptions {
        match_case: js.match_case,
        whole_word: js.whole_word,
        wildcards: js.wildcards,
        match_prefix: js.match_prefix,
        match_suffix: js.match_suffix,
        match_width: js.match_width,
        match_kana: js.match_kana,
        ignore_punctuation: js.ignore_punctuation,
        ignore_whitespace: js.ignore_whitespace,
        direction,
        scope,
        format: js.format.into(),
        replace_format: js.replace_format.into(),
    })
}

/// A surface position as script addresses it: the story, the paragraph in
/// it and the byte offset.
fn story_place(document: &WordDocument, (paragraph, offset): (usize, usize)) -> Value {
    match document.story_position_for_surface(paragraph) {
        Some(place) => json!({
            "story": place.story,
            "paragraph": place.index,
            "offset": offset,
            "anchorParagraph": place.anchor_paragraph,
            "noteId": place.note_id,
        }),
        None => Value::Null,
    }
}

fn body_index(document: &WordDocument, surface: usize) -> Option<usize> {
    document.body_index_for_surface(surface)
}

fn surface_of_body(document: &WordDocument, body: usize) -> Result<usize, String> {
    document
        .surface_index_for_body(body)
        .ok_or_else(|| format!("body paragraph {body} is unavailable"))
}

fn go_to_target(name: &str) -> Result<GoToTarget, JsValue> {
    GoToTarget::ALL
        .into_iter()
        .find(|target| target.label_key().strip_prefix("goto.") == Some(name))
        .ok_or_else(|| JsValue::from_str(&format!("unknown go-to target {name}")))
}

fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Tip => "tip",
    }
}

fn issue_kind_name(kind: &IssueKind) -> &'static str {
    match kind {
        IssueKind::MissingAltText(_) => "missingAltText",
        IssueKind::NoHeaderRow { .. } => "noHeaderRow",
        IssueKind::SkippedHeading { .. } => "skippedHeading",
        IssueKind::UnclearLinkText => "unclearLinkText",
        IssueKind::LowContrast { .. } => "lowContrast",
        IssueKind::NotInline(_) => "notInline",
    }
}

#[wasm_bindgen]
impl OffscreenDocumentView {
    /// 検索と置換 with every option: each match's start and end, and what
    /// was found.
    #[wasm_bindgen(js_name = searchDocument)]
    pub fn search_document(&self, query: String, options: JsValue) -> Result<JsValue, JsValue> {
        let options = search_options(options)?;
        let hits = self.read_word_session(|session| {
            let document = session.word_document();
            session.search(&query, &options).map(|hits| {
                hits.iter()
                    .map(|hit| {
                        json!({
                            "start": story_place(document, hit.start),
                            "end": story_place(document, hit.end),
                            "found": hit.found,
                        })
                    })
                    .collect::<Vec<_>>()
            })
        })?;
        hits.map(|hits| to_js(&hits))
            .map_err(|error| JsValue::from_str(&error.0))
    }

    /// 置換: the `index`th match takes `replacement` (its `\1` and `^` codes
    /// filled in, the replacement's 書式 put on), as one undo step.
    #[wasm_bindgen(js_name = replaceSearchHit)]
    pub fn replace_search_hit(
        &self,
        query: String,
        replacement: String,
        options: JsValue,
        index: usize,
    ) -> Result<JsValue, JsValue> {
        let options = search_options(options)?;
        self.run_word_session("word_replace_hit", move |session| {
            let hits = session.search(&query, &options).map_err(|error| error.0)?;
            let hit = hits
                .get(index)
                .ok_or_else(|| format!("no match {index} for {query}"))?;
            session.replace_hit(hit, &replacement, &options)?;
            Ok(())
        })
    }

    /// すべて置換, as one undo step. `result` is how many were replaced.
    #[wasm_bindgen(js_name = replaceAllInDocument)]
    pub fn replace_all_in_document(
        &self,
        query: String,
        replacement: String,
        options: JsValue,
    ) -> Result<JsValue, JsValue> {
        let options = search_options(options)?;
        self.run_word_session("word_replace_all", move |session| {
            session.replace_every(&query, &replacement, &options)
        })
    }

    /// The ナビゲーション pane's 見出し: each heading's body paragraph, level,
    /// text, page and whether it has headings under it.
    #[wasm_bindgen(js_name = documentOutline)]
    pub fn document_outline(&self) -> Result<JsValue, JsValue> {
        let headings = self.read_word_session(|session| {
            let document = session.word_document();
            session
                .outline()
                .into_iter()
                .filter_map(|heading| {
                    Some(json!({
                        "paragraph": body_index(document, heading.paragraph)?,
                        "level": heading.level,
                        "text": heading.text,
                        "page": heading.page,
                        "hasChildren": heading.has_children,
                    }))
                })
                .collect::<Vec<_>>()
        })?;
        Ok(to_js(&headings))
    }

    /// 上位レベルへ / 下位レベルへ for the heading at body paragraph
    /// `heading`.
    #[wasm_bindgen(js_name = shiftHeadingLevel)]
    pub fn shift_heading_level(&self, heading: usize, promote: bool) -> Result<JsValue, JsValue> {
        self.run_word_session("word_shift_heading_level", move |session| {
            let surface = surface_of_body(session.word_document(), heading)?;
            session.shift_heading_level(surface, promote)
        })
    }

    /// The heading at body paragraph `heading` moves up or down past its
    /// neighbour, with everything under it — dragging it in the pane.
    #[wasm_bindgen(js_name = moveHeadingSection)]
    pub fn move_heading_section(&self, heading: usize, up: bool) -> Result<JsValue, JsValue> {
        self.run_word_session("word_move_heading_section", move |session| {
            let surface = surface_of_body(session.word_document(), heading)?;
            session.move_heading_section(surface, up).map(|_| ())
        })
    }

    /// ジャンプ: where `target` (page, section, line, bookmark, comment,
    /// footnote, endnote, field, table, graphic, equation, heading) and what
    /// was typed (a number, `+n`, `-n` or a name) take the caret.
    /// `{position}` is a place in the text; `{page}` and `{line}` are
    /// 1-based, for the shell's own layout to find.
    #[wasm_bindgen(js_name = goTo)]
    pub fn go_to(&self, target: String, spec: String) -> Result<JsValue, JsValue> {
        let target = go_to_target(&target)?;
        let at = self.current_selection().focus;
        let found = self.read_word_session(|session| {
            session
                .go_to(target, &spec, (at.paragraph, at.offset))
                .map(|found| match found {
                    GoTo::Position(position) => {
                        json!({ "position": story_place(session.word_document(), position) })
                    }
                    GoTo::Page(page) => json!({ "page": page }),
                    GoTo::Line(line) => json!({ "line": line }),
                })
        })?;
        found
            .map(|found| to_js(&found))
            .map_err(|error| JsValue::from_str(&error))
    }

    /// The places ジャンプ steps through for `target`, in order.
    #[wasm_bindgen(js_name = goToPlaces)]
    pub fn go_to_places(&self, target: String) -> Result<JsValue, JsValue> {
        let target = go_to_target(&target)?;
        let places = self.read_word_session(|session| {
            let document = session.word_document();
            session
                .go_to_places(target)
                .into_iter()
                .map(|place| story_place(document, place))
                .collect::<Vec<_>>()
        })?;
        Ok(to_js(&places))
    }

    /// アクセシビリティ チェック: every issue, with its severity, kind,
    /// place and whether a quick fix exists. `index` names it for
    /// `fixAccessibilityIssue`.
    #[wasm_bindgen(js_name = accessibilityIssues)]
    pub fn accessibility_issues(&self) -> Result<JsValue, JsValue> {
        let issues = self.read_word_session(|session| {
            let document = session.word_document();
            session
                .accessibility_issues()
                .iter()
                .enumerate()
                .map(|(index, issue)| {
                    let mut value = json!({
                        "index": index,
                        "kind": issue_kind_name(&issue.kind),
                        "severity": severity_name(issue.severity()),
                        "start": story_place(document, issue.start()),
                        "end": story_place(document, issue.end()),
                        "excerpt": issue.excerpt,
                        "quickFix": issue.kind.has_quick_fix(),
                    });
                    match issue.kind {
                        IssueKind::SkippedHeading { from, to } => {
                            value["from"] = json!(from);
                            value["to"] = json!(to);
                        }
                        IssueKind::LowContrast { ratio_tenths } => {
                            value["ratio"] = json!(f64::from(ratio_tenths) / 10.0);
                        }
                        IssueKind::NoHeaderRow { table_order_index } => {
                            value["table"] = json!(table_order_index);
                        }
                        _ => {}
                    }
                    value
                })
                .collect::<Vec<_>>()
        })?;
        Ok(to_js(&issues))
    }

    /// The checker's one-click fix for issue `index` of `accessibilityIssues`.
    #[wasm_bindgen(js_name = fixAccessibilityIssue)]
    pub fn fix_accessibility_issue(&self, index: usize) -> Result<JsValue, JsValue> {
        self.run_word_session("word_fix_accessibility_issue", move |session| {
            let issue = session
                .accessibility_issues()
                .into_iter()
                .nth(index)
                .ok_or_else(|| format!("no accessibility issue {index}"))?;
            if !issue.kind.has_quick_fix() {
                return Err("この問題には自動の修正がありません".to_string());
            }
            session.quick_fix(&issue)
        })
    }

    /// The デザイン tab: the document's theme as it stands, and the
    /// galleries — themes, colours, fonts, effects, style sets and
    /// paragraph spacings.
    #[wasm_bindgen(js_name = designGallery)]
    pub fn design_gallery(&self) -> Result<JsValue, JsValue> {
        let state = self.read_word_session(|session| session.design_state())?;
        let gallery = json!({
            "current": {
                "theme": state.theme,
                "colors": state.colors,
                "fonts": state.fonts,
                "effects": state.effects,
                "accents": state.accents,
                "majorFont": state.major_font,
                "minorFont": state.minor_font,
            },
            "themes": THEMES.iter().map(|theme| json!({
                "name": theme.name,
                "accents": theme.colors.accents,
                "majorFont": theme.fonts.major_latin,
                "minorFont": theme.fonts.minor_latin,
            })).collect::<Vec<_>>(),
            "colors": COLOR_SCHEMES.iter().map(|scheme| json!({
                "name": scheme.name,
                "accents": scheme.accents,
            })).collect::<Vec<_>>(),
            "fonts": FONT_SCHEMES.iter().map(|scheme| json!({
                "name": scheme.name,
                "majorLatin": scheme.major_latin,
                "minorLatin": scheme.minor_latin,
                "majorEastAsia": scheme.major_east_asia,
                "minorEastAsia": scheme.minor_east_asia,
            })).collect::<Vec<_>>(),
            "effects": EFFECT_SCHEMES.iter().map(|scheme| scheme.name).collect::<Vec<_>>(),
            "styleSets": style_sets().iter().map(|set| json!({
                "name": set.name,
                "label": set.label_ja,
            })).collect::<Vec<_>>(),
            "paragraphSpacings": PARAGRAPH_SPACINGS.iter().map(|(name, label, ..)| json!({
                "name": name,
                "label": label,
            })).collect::<Vec<_>>(),
        });
        Ok(to_js(&gallery))
    }

    /// Applies one of the デザイン tab's galleries by name: `kind` is
    /// `theme`, `colors`, `fonts`, `effects`, `styleSet` or
    /// `paragraphSpacing`.
    #[wasm_bindgen(js_name = applyDesign)]
    pub fn apply_design(&self, kind: String, name: String) -> Result<JsValue, JsValue> {
        let unknown = |kind: &str, name: &str| format!("unknown {kind} {name}");
        match kind.as_str() {
            "theme" => {
                let theme = *THEMES
                    .iter()
                    .find(|theme| theme.name == name)
                    .ok_or_else(|| JsValue::from_str(&unknown(&kind, &name)))?;
                self.run_word_session("word_apply_theme", move |session| {
                    session.apply_theme(&theme)
                })
            }
            "colors" => {
                let scheme = *COLOR_SCHEMES
                    .iter()
                    .find(|scheme| scheme.name == name)
                    .ok_or_else(|| JsValue::from_str(&unknown(&kind, &name)))?;
                self.run_word_session("word_apply_color_scheme", move |session| {
                    session.apply_color_scheme(&scheme)
                })
            }
            "fonts" => {
                let scheme = *FONT_SCHEMES
                    .iter()
                    .find(|scheme| scheme.name == name)
                    .ok_or_else(|| JsValue::from_str(&unknown(&kind, &name)))?;
                self.run_word_session("word_apply_font_scheme", move |session| {
                    session.apply_font_scheme(&scheme)
                })
            }
            "effects" => {
                let scheme = *EFFECT_SCHEMES
                    .iter()
                    .find(|scheme| scheme.name == name)
                    .ok_or_else(|| JsValue::from_str(&unknown(&kind, &name)))?;
                self.run_word_session("word_apply_effect_scheme", move |session| {
                    session.apply_effect_scheme(&scheme)
                })
            }
            "styleSet" => {
                let set = style_sets()
                    .into_iter()
                    .find(|set| set.name == name)
                    .ok_or_else(|| JsValue::from_str(&unknown(&kind, &name)))?;
                self.run_word_session("word_apply_style_set", move |session| {
                    session.apply_style_set(&set)
                })
            }
            "paragraphSpacing" => {
                let (_, _, before, after, line) = *PARAGRAPH_SPACINGS
                    .iter()
                    .find(|(id, ..)| *id == name)
                    .ok_or_else(|| JsValue::from_str(&unknown(&kind, &name)))?;
                self.run_word_session("word_apply_paragraph_spacing", move |session| {
                    session.apply_paragraph_spacing(before, after, line)
                })
            }
            _ => Err(JsValue::from_str(&format!("unknown design gallery {kind}"))),
        }
    }

    /// 既定に設定: the caret's font, size and spacing become the document's
    /// defaults.
    #[wasm_bindgen(js_name = setDesignAsDefault)]
    pub fn set_design_as_default(&self) -> Result<JsValue, JsValue> {
        self.run_word_session("word_set_as_default", |session| session.set_as_default())
    }
}
