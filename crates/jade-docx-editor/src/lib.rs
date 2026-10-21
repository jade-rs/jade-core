use js_sys::Reflect;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

pub mod canvas_renderer;
pub mod canvas_view;
pub(crate) mod chart;
pub mod commands;
pub mod draw_command;
pub mod headless_api;
pub(crate) mod legal_structure;
pub(crate) mod number_format;
pub mod plugins;
pub use jade_docx_reader::{
    comments, common, document, docx_xml, field_datetime, numbering, parser, settings, styles,
    theme,
};
#[cfg(feature = "otel")]
pub mod otel_layer;
pub mod render_host;
#[cfg(not(feature = "otel"))]
#[macro_use]
pub(crate) mod tracing_stub;
pub mod serializer;
pub mod shortcuts;
pub mod drawing_xml;
pub mod picture_effects;
pub mod smartart;
pub(crate) mod chart_paint;
pub(crate) mod wordart_paint;
pub(crate) mod snapshot_plan;
pub(crate) mod typed_autoformat;
pub mod word_document;

use crate::canvas_view::OffscreenDocumentView;
use crate::shortcuts::{ShortcutBinding, ShortcutConfig};
use word_document::WordDocument;

#[cfg(feature = "opentype-math")]
#[wasm_bindgen(js_name = validateMathFont)]
pub fn validate_math_font(bytes: &[u8]) -> Result<(), JsValue> {
    jade_docx_layout::math_layout::styled::font::MathFontData::validate(bytes)
        .map_err(|e| JsValue::from_str(&format!("MATH font: {e:?}")))
}

/// Metadata-only preflight used before replacing an active browser document.
#[wasm_bindgen(js_name = validateDocxPackage)]
pub fn validate_docx_package(data: &[u8]) -> Result<(), JsValue> {
    jade_docx_reader::package_limits::validate_package(data, Default::default())
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

/// Transform review annotations across every DOCX XML part. The positive
/// flags keep comments and tracked changes respectively. The returned Promise
/// rejects if the package cannot be inspected or a requested annotation
/// decision cannot be completed.
#[wasm_bindgen(js_name = transformDocxAnnotations)]
pub fn transform_docx_annotations(
    data: js_sys::Uint8Array,
    include_comments: bool,
    include_tracked_changes: bool,
) -> js_sys::Promise {
    let source = data.to_vec();
    wasm_bindgen_futures::future_to_promise(async move {
        let output = jade_docx_annotations::transform_docx_annotations(
            &source,
            include_comments,
            include_tracked_changes,
        )
        .await
        .map_err(|error| JsValue::from_str(&error))?;
        Ok(js_sys::Uint8Array::from(output.as_slice()).into())
    })
}

/// Shared defaults let browser hosts reject large Blobs before reading them.
#[wasm_bindgen(js_name = docxPackageLoadLimits)]
pub fn docx_package_load_limits() -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(&jade_docx_reader::package_limits::PackageLoadLimits::default())
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

/// Inspect the source package for known import limitations before opening it.
/// An empty inventory does not certify complete Word compatibility.
#[wasm_bindgen(js_name = inspectDocxCompatibility)]
pub async fn inspect_docx_compatibility(data: &[u8]) -> Result<JsValue, JsValue> {
    let package = common::OpenXmlPackage::load(data)
        .await
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    serde_wasm_bindgen::to_value(&package.compatibility_report())
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

/// The fonts the author embedded in the package (`w:embed*` in the fontTable),
/// deobfuscated and filtered by their OS/2 embedding licence:
/// `[{ family, bold, italic, subsetted, data: Uint8Array }]`. Hosts register
/// them as `FontFace`s under `family`, on every thread that lays the document
/// out, before loading it. Only the relationship parts, the fontTable and the
/// font parts are inflated, so a document without embedded fonts costs a few
/// small reads.
#[wasm_bindgen(js_name = embeddedDocxFonts)]
pub fn embedded_docx_fonts(data: &[u8]) -> Result<js_sys::Array, JsValue> {
    let embedded = jade_docx_reader::docx_embedded_fonts(data)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    let fonts = js_sys::Array::new();
    for font in embedded {
        let entry = js_sys::Object::new();
        Reflect::set(&entry, &"family".into(), &font.family.as_str().into())?;
        Reflect::set(&entry, &"bold".into(), &font.style.is_bold().into())?;
        Reflect::set(&entry, &"italic".into(), &font.style.is_italic().into())?;
        Reflect::set(&entry, &"subsetted".into(), &font.subsetted.into())?;
        Reflect::set(
            &entry,
            &"data".into(),
            &js_sys::Uint8Array::from(font.data.as_slice()).into(),
        )?;
        fonts.push(&entry);
    }
    Ok(fonts)
}

/// One `{ family, substitutes }` entry of [`set_word_font_fallbacks`].
#[derive(serde::Deserialize)]
struct WordFontFallback {
    family: String,
    substitutes: Vec<String>,
}

/// Installs the families the host registered as stand-ins for missing Word
/// fonts (`[{ family, substitutes }]`) for layout on this thread. Call before
/// loading a document; see `font_resolver::set_host_font_fallbacks`.
#[wasm_bindgen(js_name = setWordFontFallbacks)]
pub fn set_word_font_fallbacks(fallbacks: JsValue) -> Result<(), JsValue> {
    let fallbacks: Vec<WordFontFallback> = serde_wasm_bindgen::from_value(fallbacks)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    jade_docx_layout::font_resolver::set_host_font_fallbacks(fallbacks.into_iter().map(
        |fallback| jade_docx_layout::font_resolver::HostFontFallback {
            family: fallback.family,
            substitutes: fallback.substitutes,
        },
    ));
    Ok(())
}

/// Chooses which Word this thread's layout reproduces where Word for Mac and
/// Word for Windows differ: `"mac"` or `"windows"` (anything else). Call
/// before loading a document; see `line_metrics::set_word_layout_platform`.
#[wasm_bindgen(js_name = setWordLayoutPlatform)]
pub fn set_word_layout_platform(platform: &str) {
    use jade_docx_layout::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
    set_word_layout_platform(if platform.eq_ignore_ascii_case("mac") {
        WordLayoutPlatform::Mac
    } else {
        WordLayoutPlatform::Windows
    });
}

// Note: Using the default allocator (dlmalloc) for WASM which is more performant
// than wee_alloc and better maintained. wee_alloc was deprecated and had memory
// leak issues.

#[wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();
    #[cfg(feature = "otel")]
    otel_layer::init_tracing();
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

#[wasm_bindgen]
#[derive(Clone)]
/// Public configuration object exposed to JavaScript callers. Mirrors the
/// rendering options supported by the jade editor runtime so that behaviour is
/// consistent across host integrations.
pub struct Options {
    pub in_wrapper: bool,
    pub hide_wrapper_on_print: bool,
    pub ignore_width: bool,
    pub ignore_height: bool,
    pub ignore_fonts: bool,
    pub break_pages: bool,
    pub debug: bool,
    pub experimental: bool,
    class_name: String,
    pub trim_xml_declaration: bool,
    pub render_headers: bool,
    pub render_footers: bool,
    pub render_footnotes: bool,
    pub render_endnotes: bool,
    pub ignore_last_rendered_page_break: bool,
    pub use_base64_url: bool,
    pub render_changes: bool,
    revision_display_explicit: bool,
    pub render_original: bool,
    pub render_comments: bool,
    pub render_alt_chunks: bool,
    pub show_formatting_marks: bool,
    pub show_crop_marks: bool,
    record_changes: bool,
    revision_author: Option<String>,
    revision_date: Option<String>,
    revision_date_utc: Option<String>,
    pageless: bool,
    preview_mode: bool,
    /// Shared: the renderer keeps a copy of the options on every render, and
    /// the binding table is the bulk of them.
    shortcut_config: std::rc::Rc<ShortcutConfig>,
    dark_mode: bool,
}

#[wasm_bindgen]
impl Options {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Options {
        Options::default()
    }

    #[wasm_bindgen(getter)]
    pub fn class_name(&self) -> String {
        self.class_name.clone()
    }

    #[wasm_bindgen(setter)]
    pub fn set_class_name(&mut self, class_name: String) {
        self.class_name = class_name;
    }

    #[wasm_bindgen(getter)]
    pub fn pageless(&self) -> bool {
        self.pageless
    }

    #[wasm_bindgen]
    pub fn set_pageless(&mut self, value: bool) {
        self.pageless = value;
    }

    #[wasm_bindgen(getter)]
    pub fn preview_mode(&self) -> bool {
        self.preview_mode
    }

    #[wasm_bindgen]
    pub fn set_preview_mode(&mut self, value: bool) {
        self.preview_mode = value;
    }

    /// Use the next loaded document's authored content-revision visibility.
    /// Explicit renderChanges/renderOriginal setters restore host precedence.
    #[wasm_bindgen(js_name = useDocumentRevisionView)]
    pub fn use_document_revision_view(&mut self) {
        self.revision_display_explicit = false;
        self.render_original = false;
    }

    #[wasm_bindgen(getter = renderChanges)]
    pub fn render_changes(&self) -> bool {
        self.render_changes
    }

    #[wasm_bindgen(setter = renderChanges)]
    pub fn set_render_changes(&mut self, value: bool) {
        self.revision_display_explicit = true;
        self.render_changes = value;
        if value {
            self.render_original = false;
        }
    }

    #[wasm_bindgen(getter = renderOriginal)]
    pub fn render_original(&self) -> bool {
        self.render_original
    }

    #[wasm_bindgen(setter = renderOriginal)]
    pub fn set_render_original(&mut self, value: bool) {
        self.revision_display_explicit = true;
        self.render_original = value;
        if value {
            self.render_changes = false;
        }
    }

    #[wasm_bindgen(getter = recordChanges)]
    pub fn record_changes(&self) -> bool {
        self.record_changes
    }

    #[wasm_bindgen(setter = recordChanges)]
    pub fn set_record_changes(&mut self, value: bool) {
        self.record_changes = value;
    }

    #[wasm_bindgen(getter = revisionAuthor)]
    pub fn revision_author(&self) -> Option<String> {
        self.revision_author.clone()
    }

    #[wasm_bindgen(setter = revisionAuthor)]
    pub fn set_revision_author(&mut self, value: Option<String>) {
        self.revision_author = value.and_then(normalize_optional_string);
    }

    #[wasm_bindgen(getter = revisionDate)]
    pub fn revision_date(&self) -> Option<String> {
        self.revision_date.clone()
    }

    #[wasm_bindgen(setter = revisionDate)]
    pub fn set_revision_date(&mut self, value: Option<String>) {
        self.revision_date = value.and_then(normalize_optional_string);
    }

    #[wasm_bindgen(getter = revisionDateUtc)]
    pub fn revision_date_utc(&self) -> Option<String> {
        self.revision_date_utc.clone()
    }

    #[wasm_bindgen(setter = revisionDateUtc)]
    pub fn set_revision_date_utc(&mut self, value: Option<String>) {
        self.revision_date_utc = value.and_then(normalize_optional_string);
    }

    #[wasm_bindgen(js_name = setShortcuts)]
    /// Replaces the default keyboard shortcuts used by the editor.
    ///
    /// The supplied object accepts the following optional array properties:
    /// `{ deleteBackward, deleteForward, undo, redo, copy, paste, cut,
    ///    toggleBold, toggleItalic, toggleUnderline, toggleWordUnderline,
    ///    toggleDoubleUnderline, toggleSubscript, toggleSuperscript,
    ///    clearFormatting, increaseFontSize, decreaseFontSize,
    ///    alignLeft, alignCenter, alignRight, alignJustify,
    ///    moveParagraphStart, moveParagraphEnd }`, the remaining paragraph
    /// accelerators, and every name in `ShortcutAction::WORD_EXTRAS`
    /// (`changeCase`, `applyHeading1`, `insertCopyright`, ...). Each string uses a
    /// `Modifier+Key` representation (e.g. `"Ctrl+H"`). Unknown or empty
    /// arrays fall back to the built-in defaults; `moveParagraphStart` /
    /// `moveParagraphEnd` default to no binding so apps must opt in
    /// explicitly (otherwise the browser keeps `Ctrl+A` = Select All).
    pub fn set_shortcuts(&mut self, value: JsValue) -> Result<(), JsValue> {
        if value.is_null() || value.is_undefined() {
            self.shortcut_config = std::rc::Rc::new(ShortcutConfig::default());
            return Ok(());
        }

        let object = value
            .dyn_into::<js_sys::Object>()
            .map_err(|_| JsValue::from_str("shortcuts configuration must be an object"))?;

        let backward_value = Reflect::get(&object, &JsValue::from_str("deleteBackward"))?;
        let forward_value = Reflect::get(&object, &JsValue::from_str("deleteForward"))?;
        let undo_value = Reflect::get(&object, &JsValue::from_str("undo"))?;
        let redo_value = Reflect::get(&object, &JsValue::from_str("redo"))?;
        let copy_value = Reflect::get(&object, &JsValue::from_str("copy"))?;
        let paste_value = Reflect::get(&object, &JsValue::from_str("paste"))?;
        let cut_value = Reflect::get(&object, &JsValue::from_str("cut"))?;
        let toggle_bold_value = Reflect::get(&object, &JsValue::from_str("toggleBold"))?;
        let toggle_italic_value = Reflect::get(&object, &JsValue::from_str("toggleItalic"))?;
        let toggle_underline_value = Reflect::get(&object, &JsValue::from_str("toggleUnderline"))?;
        let toggle_word_underline_value =
            Reflect::get(&object, &JsValue::from_str("toggleWordUnderline"))?;
        let toggle_double_underline_value =
            Reflect::get(&object, &JsValue::from_str("toggleDoubleUnderline"))?;
        let toggle_subscript_value = Reflect::get(&object, &JsValue::from_str("toggleSubscript"))?;
        let toggle_superscript_value =
            Reflect::get(&object, &JsValue::from_str("toggleSuperscript"))?;
        let clear_formatting_value = Reflect::get(&object, &JsValue::from_str("clearFormatting"))?;
        let increase_font_size_value =
            Reflect::get(&object, &JsValue::from_str("increaseFontSize"))?;
        let decrease_font_size_value =
            Reflect::get(&object, &JsValue::from_str("decreaseFontSize"))?;
        let align_left_value = Reflect::get(&object, &JsValue::from_str("alignLeft"))?;
        let align_center_value = Reflect::get(&object, &JsValue::from_str("alignCenter"))?;
        let align_right_value = Reflect::get(&object, &JsValue::from_str("alignRight"))?;
        let align_justify_value = Reflect::get(&object, &JsValue::from_str("alignJustify"))?;
        let increase_indent_value = Reflect::get(&object, &JsValue::from_str("increaseIndent"))?;
        let decrease_indent_value = Reflect::get(&object, &JsValue::from_str("decreaseIndent"))?;
        let reset_paragraph_formatting_value =
            Reflect::get(&object, &JsValue::from_str("resetParagraphFormatting"))?;
        let increase_hanging_indent_value =
            Reflect::get(&object, &JsValue::from_str("increaseHangingIndent"))?;
        let decrease_hanging_indent_value =
            Reflect::get(&object, &JsValue::from_str("decreaseHangingIndent"))?;
        let line_spacing_single_value =
            Reflect::get(&object, &JsValue::from_str("setLineSpacingSingle"))?;
        let line_spacing_one_and_half_value =
            Reflect::get(&object, &JsValue::from_str("setLineSpacingOneAndHalf"))?;
        let line_spacing_double_value =
            Reflect::get(&object, &JsValue::from_str("setLineSpacingDouble"))?;
        let toggle_space_before_value =
            Reflect::get(&object, &JsValue::from_str("toggleSpaceBefore"))?;
        let increase_font_size_by_point_value =
            Reflect::get(&object, &JsValue::from_str("increaseFontSizeByPoint"))?;
        let decrease_font_size_by_point_value =
            Reflect::get(&object, &JsValue::from_str("decreaseFontSizeByPoint"))?;
        let insert_page_break_value = Reflect::get(&object, &JsValue::from_str("insertPageBreak"))?;
        let toggle_bullet_list_value =
            Reflect::get(&object, &JsValue::from_str("toggleBulletList"))?;
        let move_paragraph_start_value =
            Reflect::get(&object, &JsValue::from_str("moveParagraphStart"))?;
        let move_paragraph_end_value =
            Reflect::get(&object, &JsValue::from_str("moveParagraphEnd"))?;

        let mut config = ShortcutConfig::default();

        if !backward_value.is_null() && !backward_value.is_undefined() {
            let bindings = parse_shortcut_array(&backward_value)?;
            config = config.with_delete_backward(bindings);
        }

        if !forward_value.is_null() && !forward_value.is_undefined() {
            let bindings = parse_shortcut_array(&forward_value)?;
            config = config.with_delete_forward(bindings);
        }

        if !undo_value.is_null() && !undo_value.is_undefined() {
            let bindings = parse_shortcut_array(&undo_value)?;
            config = config.with_undo(bindings);
        }

        if !redo_value.is_null() && !redo_value.is_undefined() {
            let bindings = parse_shortcut_array(&redo_value)?;
            config = config.with_redo(bindings);
        }

        if !copy_value.is_null() && !copy_value.is_undefined() {
            let bindings = parse_shortcut_array(&copy_value)?;
            config = config.with_copy(bindings);
        }

        if !paste_value.is_null() && !paste_value.is_undefined() {
            let bindings = parse_shortcut_array(&paste_value)?;
            config = config.with_paste(bindings);
        }

        if !cut_value.is_null() && !cut_value.is_undefined() {
            let bindings = parse_shortcut_array(&cut_value)?;
            config = config.with_cut(bindings);
        }

        if !toggle_bold_value.is_null() && !toggle_bold_value.is_undefined() {
            let bindings = parse_shortcut_array(&toggle_bold_value)?;
            config = config.with_toggle_bold(bindings);
        }

        if !toggle_italic_value.is_null() && !toggle_italic_value.is_undefined() {
            let bindings = parse_shortcut_array(&toggle_italic_value)?;
            config = config.with_toggle_italic(bindings);
        }

        if !toggle_underline_value.is_null() && !toggle_underline_value.is_undefined() {
            let bindings = parse_shortcut_array(&toggle_underline_value)?;
            config = config.with_toggle_underline(bindings);
        }

        if !toggle_word_underline_value.is_null() && !toggle_word_underline_value.is_undefined() {
            let bindings = parse_shortcut_array(&toggle_word_underline_value)?;
            config = config.with_toggle_word_underline(bindings);
        }

        if !toggle_double_underline_value.is_null() && !toggle_double_underline_value.is_undefined()
        {
            let bindings = parse_shortcut_array(&toggle_double_underline_value)?;
            config = config.with_toggle_double_underline(bindings);
        }

        if !toggle_subscript_value.is_null() && !toggle_subscript_value.is_undefined() {
            let bindings = parse_shortcut_array(&toggle_subscript_value)?;
            config = config.with_toggle_subscript(bindings);
        }

        if !toggle_superscript_value.is_null() && !toggle_superscript_value.is_undefined() {
            let bindings = parse_shortcut_array(&toggle_superscript_value)?;
            config = config.with_toggle_superscript(bindings);
        }

        if !clear_formatting_value.is_null() && !clear_formatting_value.is_undefined() {
            let bindings = parse_shortcut_array(&clear_formatting_value)?;
            config = config.with_clear_formatting(bindings);
        }

        if !increase_font_size_value.is_null() && !increase_font_size_value.is_undefined() {
            let bindings = parse_shortcut_array(&increase_font_size_value)?;
            config = config.with_increase_font_size(bindings);
        }

        if !decrease_font_size_value.is_null() && !decrease_font_size_value.is_undefined() {
            let bindings = parse_shortcut_array(&decrease_font_size_value)?;
            config = config.with_decrease_font_size(bindings);
        }

        if !align_left_value.is_null() && !align_left_value.is_undefined() {
            let bindings = parse_shortcut_array(&align_left_value)?;
            config = config.with_align_left(bindings);
        }

        if !align_center_value.is_null() && !align_center_value.is_undefined() {
            let bindings = parse_shortcut_array(&align_center_value)?;
            config = config.with_align_center(bindings);
        }

        if !align_right_value.is_null() && !align_right_value.is_undefined() {
            let bindings = parse_shortcut_array(&align_right_value)?;
            config = config.with_align_right(bindings);
        }

        if !align_justify_value.is_null() && !align_justify_value.is_undefined() {
            let bindings = parse_shortcut_array(&align_justify_value)?;
            config = config.with_align_justify(bindings);
        }
        if !increase_indent_value.is_null() && !increase_indent_value.is_undefined() {
            let bindings = parse_shortcut_array(&increase_indent_value)?;
            config = config.with_increase_indent(bindings);
        }
        if !decrease_indent_value.is_null() && !decrease_indent_value.is_undefined() {
            let bindings = parse_shortcut_array(&decrease_indent_value)?;
            config = config.with_decrease_indent(bindings);
        }
        if !reset_paragraph_formatting_value.is_null()
            && !reset_paragraph_formatting_value.is_undefined()
        {
            let bindings = parse_shortcut_array(&reset_paragraph_formatting_value)?;
            config = config.with_reset_paragraph_formatting(bindings);
        }
        if !increase_hanging_indent_value.is_null() && !increase_hanging_indent_value.is_undefined()
        {
            let bindings = parse_shortcut_array(&increase_hanging_indent_value)?;
            config = config.with_increase_hanging_indent(bindings);
        }
        if !decrease_hanging_indent_value.is_null() && !decrease_hanging_indent_value.is_undefined()
        {
            let bindings = parse_shortcut_array(&decrease_hanging_indent_value)?;
            config = config.with_decrease_hanging_indent(bindings);
        }
        if !line_spacing_single_value.is_null() && !line_spacing_single_value.is_undefined() {
            let bindings = parse_shortcut_array(&line_spacing_single_value)?;
            config = config.with_set_line_spacing_single(bindings);
        }
        if !line_spacing_one_and_half_value.is_null()
            && !line_spacing_one_and_half_value.is_undefined()
        {
            let bindings = parse_shortcut_array(&line_spacing_one_and_half_value)?;
            config = config.with_set_line_spacing_one_and_half(bindings);
        }
        if !line_spacing_double_value.is_null() && !line_spacing_double_value.is_undefined() {
            let bindings = parse_shortcut_array(&line_spacing_double_value)?;
            config = config.with_set_line_spacing_double(bindings);
        }
        if !toggle_space_before_value.is_null() && !toggle_space_before_value.is_undefined() {
            let bindings = parse_shortcut_array(&toggle_space_before_value)?;
            config = config.with_toggle_space_before(bindings);
        }
        if !increase_font_size_by_point_value.is_null()
            && !increase_font_size_by_point_value.is_undefined()
        {
            let bindings = parse_shortcut_array(&increase_font_size_by_point_value)?;
            config = config.with_increase_font_size_by_point(bindings);
        }
        if !decrease_font_size_by_point_value.is_null()
            && !decrease_font_size_by_point_value.is_undefined()
        {
            let bindings = parse_shortcut_array(&decrease_font_size_by_point_value)?;
            config = config.with_decrease_font_size_by_point(bindings);
        }
        if !insert_page_break_value.is_null() && !insert_page_break_value.is_undefined() {
            let bindings = parse_shortcut_array(&insert_page_break_value)?;
            config = config.with_insert_page_break(bindings);
        }
        if !toggle_bullet_list_value.is_null() && !toggle_bullet_list_value.is_undefined() {
            let bindings = parse_shortcut_array(&toggle_bullet_list_value)?;
            config = config.with_toggle_bullet_list(bindings);
        }

        if !move_paragraph_start_value.is_null() && !move_paragraph_start_value.is_undefined() {
            let bindings = parse_shortcut_array(&move_paragraph_start_value)?;
            config = config.with_move_paragraph_start(bindings);
        }

        if !move_paragraph_end_value.is_null() && !move_paragraph_end_value.is_undefined() {
            let bindings = parse_shortcut_array(&move_paragraph_end_value)?;
            config = config.with_move_paragraph_end(bindings);
        }

        // Actions added after the per-field builders are read by name.
        for action in crate::shortcuts::ShortcutAction::WORD_EXTRAS {
            let value = Reflect::get(&object, &JsValue::from_str(action.name()))?;
            if !value.is_null() && !value.is_undefined() {
                let bindings = parse_shortcut_array(&value)?;
                config = config.with_bindings(action, bindings);
            }
        }

        self.shortcut_config = std::rc::Rc::new(config);
        Ok(())
    }

    #[wasm_bindgen(getter = darkMode)]
    pub fn dark_mode(&self) -> bool {
        self.dark_mode
    }

    #[wasm_bindgen(setter = darkMode)]
    pub fn set_dark_mode(&mut self, value: bool) {
        self.dark_mode = value;
    }
}

impl Options {
    /// Resolve only an omitted host preference, once at the document load boundary.
    pub(crate) fn apply_saved_revision_view(&mut self, visible: Option<bool>) {
        if !self.revision_display_explicit && !self.render_original {
            self.render_changes = visible.unwrap_or(false);
        }
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            ignore_height: false,
            ignore_width: false,
            ignore_fonts: false,
            break_pages: true,
            debug: false,
            experimental: false,
            class_name: "docx".to_string(),
            in_wrapper: true,
            hide_wrapper_on_print: false,
            trim_xml_declaration: true,
            ignore_last_rendered_page_break: true,
            render_headers: true,
            render_footers: true,
            render_footnotes: true,
            render_endnotes: true,
            use_base64_url: false,
            render_changes: false,
            revision_display_explicit: true,
            render_original: false,
            // Comment ranges/markers are decorated by default. The renderer only
            // disables its paragraph render cache / seek fast-paths when the
            // document actually contains comments (see `document_has_comments`),
            // so comment-free documents keep the full-speed render path.
            render_comments: true,
            render_alt_chunks: true,
            show_formatting_marks: true,
            show_crop_marks: true,
            record_changes: false,
            revision_author: None,
            revision_date: None,
            revision_date_utc: None,
            pageless: false,
            preview_mode: false,
            shortcut_config: std::rc::Rc::new(ShortcutConfig::default()),
            dark_mode: false,
        }
    }
}

impl Options {
    pub(crate) fn shortcut_config(&self) -> &ShortcutConfig {
        &self.shortcut_config
    }

    pub(crate) fn record_changes_enabled(&self) -> bool {
        self.record_changes
    }

    pub(crate) fn revision_author_internal(&self) -> Option<String> {
        self.revision_author.clone()
    }

    /// Name to record on comments and tracked changes.
    ///
    /// Word puts the editing user's name on every `w:comment` / `w:ins` /
    /// `w:del` and review flows break down when everything claims one author,
    /// so hosts set this from the signed-in user. Blank / unset falls back to
    /// [`DEFAULT_AUTHOR_NAME`] rather than writing an empty author.
    pub(crate) fn author_name_or_default(&self) -> String {
        self.revision_author
            .clone()
            .unwrap_or_else(|| DEFAULT_AUTHOR_NAME.to_string())
    }

    pub(crate) fn revision_date_internal(&self) -> Option<String> {
        self.revision_date.clone()
    }

    pub(crate) fn revision_date_utc_internal(&self) -> Option<String> {
        self.revision_date_utc.clone()
    }

    pub(crate) fn clear_revision_metadata(&mut self) {
        self.revision_author = None;
        self.revision_date = None;
        self.revision_date_utc = None;
    }
}

fn parse_shortcut_array(value: &JsValue) -> Result<Vec<ShortcutBinding>, JsValue> {
    if !js_sys::Array::is_array(value) {
        return Err(JsValue::from_str(
            "shortcut bindings must be provided as an array of strings",
        ));
    }

    let array = js_sys::Array::from(value);
    let mut result = Vec::new();

    for entry in array.iter() {
        let text = entry.as_string().ok_or_else(|| {
            JsValue::from_str("shortcut bindings must contain only string values")
        })?;

        let binding = ShortcutBinding::parse(&text)
            .map_err(|err| JsValue::from_str(&format!("invalid shortcut `{text}`: {err}")))?;
        result.push(binding);
    }

    Ok(result)
}

/// Author written on comments and tracked changes when the host has not told
/// us who the user is. Word shows this verbatim in the review pane, so it stays
/// a neutral product name rather than an internal component name.
pub(crate) const DEFAULT_AUTHOR_NAME: &str = "jade";

fn normalize_optional_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[wasm_bindgen]
/// Parses the main document part of a DOCX file and returns the intermediate
/// representation as a JavaScript `Promise`.
pub fn parse_async(data: js_sys::Uint8Array, options: Option<Options>) -> js_sys::Promise {
    let opts = options.unwrap_or_default();
    let data_vec = data.to_vec();

    wasm_bindgen_futures::future_to_promise(async move {
        let word_doc = WordDocument::load(&data_vec, &opts)
            .await
            .map_err(|e| JsValue::from_str(&format!("{e}")))?;

        serde_wasm_bindgen::to_value(&word_doc).map_err(|e| JsValue::from_str(&format!("{e}")))
    })
}

#[wasm_bindgen]
/// Renders a DOCX file inside a web worker using an `OffscreenCanvas`. The
/// resolved value mirrors the previous DOM-based renderer, returning the parsed
/// document structure for further inspection.
pub fn render_offscreen_canvas_async(
    data: js_sys::Uint8Array,
    canvas: web_sys::OffscreenCanvas,
    options: Option<Options>,
    device_pixel_ratio: Option<f64>,
) -> js_sys::Promise {
    let opts = options.unwrap_or_default();
    let ratio = device_pixel_ratio.unwrap_or(1.0);

    match OffscreenDocumentView::new(canvas, Some(opts), ratio) {
        Ok(view) => view.load_docx(data),
        Err(err) => js_sys::Promise::reject(&err),
    }
}

#[cfg(test)]
mod saved_revision_view_tests {
    use super::Options;
    #[test]
    fn saved_revision_view_is_recomputed_for_each_document_without_changing_source() {
        let mut options = Options::default();
        options.use_document_revision_view();
        options.apply_saved_revision_view(Some(true));
        assert!(options.render_changes);
        options.apply_saved_revision_view(None);
        assert!(!options.render_changes);
        options.apply_saved_revision_view(Some(false));
        assert!(!options.render_changes);
    }
    #[test]
    fn saved_revision_view_keeps_legacy_rust_options_until_opted_in() {
        let mut options = Options::default();
        options.apply_saved_revision_view(Some(true));
        assert!(!options.render_changes);
        let mut explicit = Options {
            render_changes: false,
            ..Options::default()
        };
        explicit.apply_saved_revision_view(Some(true));
        assert!(!explicit.render_changes);
    }

    #[test]
    fn saved_revision_view_keeps_explicit_final_markup_and_original() {
        let mut options = Options::default();
        options.set_render_changes(false);
        options.apply_saved_revision_view(Some(true));
        assert!(!options.render_changes);
        options.set_render_changes(true);
        options.apply_saved_revision_view(Some(false));
        assert!(options.render_changes);
        options.set_render_original(true);
        options.apply_saved_revision_view(Some(true));
        assert!(options.render_original);
        assert!(!options.render_changes);
    }
}

#[cfg(test)]
mod author_name_tests {
    use super::*;

    #[test]
    fn author_name_falls_back_when_unset_or_blank() {
        let mut options = Options::default();
        assert_eq!(options.author_name_or_default(), DEFAULT_AUTHOR_NAME);

        // Blank input must not produce `w:author=""` in the output document.
        for blank in ["", "   ", "\t\n"] {
            options.set_revision_author(Some(blank.to_string()));
            assert_eq!(options.author_name_or_default(), DEFAULT_AUTHOR_NAME);
            assert_eq!(options.revision_author_internal(), None);
        }
    }

    #[test]
    fn author_name_uses_the_configured_user_and_trims_it() {
        let mut options = Options::default();
        options.set_revision_author(Some("  山田 太郎 ".to_string()));
        assert_eq!(options.author_name_or_default(), "山田 太郎");

        // Clearing returns to the fallback so a signed-out host does not keep
        // stamping the previous user's name.
        options.set_revision_author(None);
        assert_eq!(options.author_name_or_default(), DEFAULT_AUTHOR_NAME);
    }
}
