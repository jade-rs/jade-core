use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[cfg(not(feature = "otel"))]
use crate::tracing_stub::{info_span, Instrument};
use js_sys::{Array, Object, Promise, Reflect};
use serde::{Deserialize, Serialize};
use serde_wasm_bindgen::to_value;
#[cfg(feature = "otel")]
use tracing::{info_span, Instrument};
use unicode_segmentation::UnicodeSegmentation;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{future_to_promise, JsFuture};
use web_sys::{self, ImageBitmap, OffscreenCanvas, WorkerGlobalScope};

use crate::canvas_renderer::{
    CanvasRenderer, FieldDateTime, TableResizeHandleKind, TableResizeMetadata,
    TableRowResizeHandleKind,
};
use crate::commands::apply_underline::ApplyUnderlineCommand;
use crate::commands::bookmark::{
    CreateBookmarkCommand, DeleteBookmarkCommand, InsertCrossReferenceCommand,
};
use crate::commands::delete_object::DeleteObjectCommand;
use crate::commands::delete_text::DeleteDirection;
use crate::commands::format_image::{FormatImageCommand, ImageFormatSpec, ImageLayoutPreset};
use crate::commands::format_text_box::{FormatTextBoxCommand, TextBoxFormatSpec};
use crate::commands::hyperlink::{InsertHyperlinkCommand, RemoveHyperlinkCommand};
use crate::commands::insert_chart::InsertChartCommand;
use crate::commands::insert_citation::{AddCitationSourceCommand, InsertCitationCommand};
use crate::commands::insert_endnote::InsertEndnoteCommand;
use crate::commands::insert_field::InsertFieldCommand;
use crate::commands::insert_footnote::InsertFootnoteCommand;
use crate::commands::insert_image::InsertImageCommand;
use crate::commands::insert_index::MarkIndexEntryCommand;
use crate::commands::insert_shape::InsertShapeCommand;
use crate::commands::insert_text::InsertTextCommand;
use crate::commands::insert_text_box::InsertTextBoxCommand;
use crate::commands::merge_table_cells::{MergeTableCellsCommand, TableCellSelection};
use crate::commands::replace_image::ReplaceImageCommand;
use crate::commands::resize_table_column::ResizeTableColumnCommand;
use crate::commands::resize_table_row::visual_height_for_resize;
use crate::commands::set_document_protection::SetDocumentProtectionCommand;
use crate::commands::set_ink_annotations::SetInkAnnotationsCommand;
use crate::commands::snapshot_edit::PackageSnapshotCommand;
use crate::commands::transform_object::TransformObjectCommand;
use crate::commands::unmerge_table_cells::UnmergeTableCellsCommand;
use crate::commands::update_chart::UpdateChartCommand;
use crate::commands::{Command, CommandContext, CommandExecutor, HistoryRenderFootprint};
use crate::document::{
    resolve_cell_margin_edge_points, ImageCrop, ParagraphProperties, RunChart, RunImageKind,
    RunProperties, TableCellMargins, TextBoxBorderStyle, TextBoxShape, TextBoxVerticalAlignment,
    WrapMode, WrapTextSide, DEFAULT_VERTICAL_CELL_MARGIN_PT,
};
use crate::headless_api::{
    apply_headless_commands_with_executor_to_document, next_revision_from,
    plain_text_paste_commands, rebase_position_after_decision, revision_snapshots,
    revision_targets_for, select_revisions, RevisionSelector,
};
use crate::render_host::{BoxedRenderHost, OffscreenRenderHost};
use crate::styles::{merge_paragraph_properties_override, merge_run_properties_override};
use crate::word_document::{
    ChartInsertSpec, CitationSourceInfo, CrossReferenceKind, CrossReferenceTarget,
    DocumentProtectionInfo, FloatingObjectKind, FloatingObjectTarget, FloatingObjectTransform,
    HeaderFooterAlignment, HeaderFooterRole, HeaderFooterVariant, HeaderLine, HfRunFormat,
    HfRunState, InkAnnotationInfo, RevisionSession, ShapeInsertSpec, WordDocument,
};
use crate::{Options, DEFAULT_AUTHOR_NAME};
use jade_docx_headless_api::{
    HeadlessBorderEdge, HeadlessChangeCaseMode, HeadlessCommand, HeadlessCoverParagraph,
    HeadlessCustomNumberingLevel, HeadlessDeleteDirection, HeadlessDocumentTheme,
    HeadlessDropCapKind, HeadlessEditRequest, HeadlessEquationPlacement, HeadlessFieldKind,
    HeadlessMultilevelListPreset, HeadlessRevisionSnapshot, HeadlessSectionBreakType,
    HeadlessTableCellDeleteDirection, HeadlessTableCellInsertDirection, HeadlessTableCellSelection,
    HeadlessTableFitMode, HeadlessTableTextWrapping,
};
use jade_docx_layout::layout_cache::{DirtyRect, SelectionRect};
use jade_docx_layout::selection::{CaretMovement, SelectionRange, TextPosition};

/// Tracks which document area is currently being edited.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EditingMode {
    #[default]
    Body,
    Header,
    Footer,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeTextPosition {
    paragraph: usize,
    offset: usize,
    story: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    anchor_paragraph: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note_id: Option<String>,
}

#[derive(Serialize)]
struct RuntimeSelectionRange {
    anchor: RuntimeTextPosition,
    focus: RuntimeTextPosition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct HeaderFooterVariantSettingsJs {
    different_first_page: bool,
    different_odd_even_pages: bool,
}

#[derive(Serialize)]
struct PageNumberFormatSettingsJs {
    format: String,
    start: Option<u32>,
}

#[derive(Serialize)]
struct NoteNumberingSettingsJs {
    format: String,
    start: u32,
    restart: String,
}

fn note_kind_is_footnote(kind: &str) -> Result<bool, JsValue> {
    match kind {
        "footnote" => Ok(true),
        "endnote" => Ok(false),
        _ => Err(JsValue::from_str("note kind must be footnote or endnote")),
    }
}

/// Which header/footer variant the current edit session targets. Matches the
/// three OOXML variants (`w:type="default" | "first" | "even"`). The active
/// variant depends on which page the user clicked — see
/// `variant_for_page_index`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HfVariant {
    #[default]
    Default,
    First,
    Even,
}

/// Which segment of a header/footer line the cursor is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum HfSegment {
    Left,
    Center,
    Right,
}

/// Cursor state when editing a header or footer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct HfEditCursor {
    page_index: usize,
    line_index: usize,
    segment: HfSegment,
    offset: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct HfPackageTarget {
    part_path: String,
    range: SelectionRange,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HfClipboardSelection {
    variant: HfVariant,
    anchor: HfEditCursor,
    focus: HfEditCursor,
    package_target: Option<HfPackageTarget>,
}

const HF_EDIT_LINE_HEIGHT: f64 = 20.0;
const HF_EDIT_CONTENT_START_RATIO: f64 = 0.5;

struct CanvasDocumentState {
    options: Options,
    /// Which fields show their codes (⌥F9 / ⇧F9).
    field_code_view: crate::fields::FieldCodeView,
    /// Word's オートコレクト while typing; `None` while it is off.
    word_autocorrect: Option<crate::word::autocorrect::AutoCorrectOptions>,
    /// The equation being edited in place, its caret inside the structure.
    equation_editor: Option<crate::word::equation::EquationEditor>,
    word_doc: Option<WordDocument>,
    paged_images: Vec<JsValue>,
    /// True when this view was initialized via lazy rendering (render-lazy / page-boundary lazy layout).
    /// In this mode, edit operations should avoid transferring ImageBitmaps eagerly; the host can
    /// re-fetch visible pages on demand via `getPageBitmap()`.
    lazy_mode: bool,
    preview_mode: bool,
    page_export_original_formatting_marks: Option<bool>,
    page_export_original_raster_scale: Option<f64>,
    command_executor: CommandExecutor,
    composition_preview: Option<CompositionPreviewState>,
    selection: Option<SelectionRange>,
    object_selection: Option<FloatingObjectTarget>,
    inline_formatting: InlineFormattingState,
    caret_goal_x: Option<f64>,
    table_resize: TableResizeMetadata,
    editing_mode: EditingMode,
    /// Header/footer variant the active edit session is attached to. Only
    /// meaningful when `editing_mode != Body`. Chosen at click time based on
    /// `title_page` / `even_and_odd_headers` and the clicked page index.
    hf_variant: HfVariant,
    hf_edit_cursor: Option<HfEditCursor>,
    /// Body caret/selection to restore when the header/footer session closes.
    /// Switching between header/footer stories must keep the original range.
    hf_body_selection: Option<SelectionRange>,
    /// Other end of a header/footer selection (Shift+arrows, drag). A
    /// selection exists when this and the cursor sit on the same segment.
    hf_selection_anchor: Option<HfEditCursor>,
    hf_undo_stack: Vec<HfHistoryEntry>,
    hf_redo_stack: Vec<HfHistoryEntry>,
    hf_history_revision: u64,
    hf_provisional_recording: bool,
}

/// Updates the user's live formatting-mark preference. While a clean page
/// export is active, marks remain hidden in the renderer and only the saved
/// preference changes; leaving export mode restores the latest preference.
fn set_formatting_marks_preference(
    options: &mut Options,
    export_original: &mut Option<bool>,
    visible: bool,
) -> bool {
    if let Some(saved) = export_original.as_mut() {
        *saved = visible;
        false
    } else {
        options.show_formatting_marks = visible;
        visible
    }
}

fn set_page_export_render_mode(
    options: &mut Options,
    export_original: &mut Option<bool>,
    enabled: bool,
) -> (bool, bool) {
    if enabled {
        if export_original.is_none() {
            *export_original = Some(options.show_formatting_marks);
        }
        options.show_formatting_marks = false;
        options.show_crop_marks = false;
    } else {
        if let Some(original) = export_original.take() {
            options.show_formatting_marks = original;
        }
        options.show_crop_marks = true;
    }
    (options.show_formatting_marks, options.show_crop_marks)
}

fn command_mutates_header_footer_stories(name: &str) -> bool {
    matches!(
        name,
        "set_watermark" | "set_document_theme" | "set_header_footer_variants"
    )
}

fn command_mutates_style_definitions(name: &str) -> bool {
    // A Word model action (`word_*`, see `canvas_view/word_session.rs`) can
    // rewrite the styles part or the package with it.
    matches!(name, "edit_document_package" | "set_document_theme") || name.starts_with("word_")
}

fn sync_renderer_header_footer_stories(renderer: &mut CanvasRenderer, word_doc: &WordDocument) {
    renderer.sync_document_header_footer_stories(word_doc);
}

/// Maximum number of header/footer undo steps retained. Each entry holds two
/// full `WordDocument` clones, so this is intentionally smaller than the body
/// command-history cap; it bounds memory on large documents while keeping a
/// generous header/footer undo depth.
const MAX_HF_UNDO_HISTORY: usize = 50;

#[derive(Clone)]
struct HfHistoryEntry {
    before_doc: WordDocument,
    after_doc: WordDocument,
    before_cursor: Option<HfEditCursor>,
    after_cursor: Option<HfEditCursor>,
    before_selection_anchor: Option<HfEditCursor>,
    after_selection_anchor: Option<HfEditCursor>,
    mode: EditingMode,
    variant: HfVariant,
}

struct CompositionPreviewState {
    base_selection: Option<SelectionRange>,
    commands: Vec<Box<dyn Command>>,
}

/// Stores revision metadata extracted from the JavaScript payload.
#[derive(Default)]
struct RevisionMetadata {
    author: Option<String>,
    date: Option<String>,
    date_utc: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ObjectSelectionPayload {
    kind: crate::word_document::FloatingObjectKind,
    paragraph_index: usize,
    anchor_paragraph_index: usize,
    run_index: usize,
    page: usize,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    rotation_deg: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<SelectedImagePayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text_box: Option<SelectedTextBoxPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chart: Option<RunChart>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SelectedTextBoxPayload {
    shape: TextBoxShape,
    fill_color: Option<String>,
    border_color: Option<String>,
    border_style: &'static str,
    is_line: bool,
    /// Word's 文字列の折り返し preset the shape currently uses.
    layout: String,
    wrap_side: String,
    width_px: f64,
    height_px: f64,
    dist_top: f64,
    dist_bottom: f64,
    dist_left: f64,
    dist_right: f64,
    inset_left: f64,
    inset_top: f64,
    inset_right: f64,
    inset_bottom: f64,
    text_vertical_align: &'static str,
    rotation_deg: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SelectedImagePayload {
    width_px: f64,
    height_px: f64,
    crop: Option<ImageCrop>,
    layout: String,
    horizontal_align: Option<String>,
    vertical_align: Option<String>,
    x: Option<f64>,
    y: Option<f64>,
    rotation_deg: f64,
    flip_horizontal: bool,
    flip_vertical: bool,
    dist_top: f64,
    dist_bottom: f64,
    dist_left: f64,
    dist_right: f64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
struct ImageFormatInput {
    width_px: Option<f64>,
    height_px: Option<f64>,
    crop: Option<ImageCrop>,
    clear_crop: bool,
    layout: Option<String>,
    horizontal_align: Option<String>,
    vertical_align: Option<String>,
    x: Option<f64>,
    y: Option<f64>,
    rotation_deg: Option<f64>,
    flip_horizontal: Option<bool>,
    flip_vertical: Option<bool>,
    dist_top: Option<f64>,
    dist_bottom: Option<f64>,
    dist_left: Option<f64>,
    dist_right: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
struct TextBoxFormatInput {
    fill_color: Option<String>,
    clear_fill: bool,
    border_color: Option<String>,
    clear_border: bool,
    layout: Option<String>,
    wrap_side: Option<String>,
    width_px: Option<f64>,
    height_px: Option<f64>,
    horizontal_align: Option<String>,
    vertical_align: Option<String>,
    dist_top: Option<f64>,
    dist_bottom: Option<f64>,
    dist_left: Option<f64>,
    dist_right: Option<f64>,
    inset_left: Option<f64>,
    inset_top: Option<f64>,
    inset_right: Option<f64>,
    inset_bottom: Option<f64>,
    text_vertical_align: Option<String>,
    rotation_deg: Option<f64>,
}

impl TryFrom<TextBoxFormatInput> for TextBoxFormatSpec {
    type Error = String;

    fn try_from(value: TextBoxFormatInput) -> Result<Self, Self::Error> {
        let layout = value
            .layout
            .as_deref()
            .map(|layout| match layout {
                "inline" => Ok(ImageLayoutPreset::Inline),
                "square" => Ok(ImageLayoutPreset::Square),
                "tight" => Ok(ImageLayoutPreset::Tight),
                "through" => Ok(ImageLayoutPreset::Through),
                "topAndBottom" => Ok(ImageLayoutPreset::TopAndBottom),
                "behindText" => Ok(ImageLayoutPreset::BehindText),
                "inFrontOfText" => Ok(ImageLayoutPreset::InFrontOfText),
                _ => Err(format!("unsupported shape layout `{layout}`")),
            })
            .transpose()?;
        let wrap_side = value
            .wrap_side
            .as_deref()
            .map(|side| match side {
                "bothSides" => Ok(WrapTextSide::BothSides),
                "left" => Ok(WrapTextSide::Left),
                "right" => Ok(WrapTextSide::Right),
                "largest" => Ok(WrapTextSide::Largest),
                _ => Err(format!("unsupported wrap side `{side}`")),
            })
            .transpose()?;
        let text_vertical_align = value
            .text_vertical_align
            .as_deref()
            .map(|align| match align {
                "top" => Ok(TextBoxVerticalAlignment::Top),
                "center" => Ok(TextBoxVerticalAlignment::Center),
                "bottom" => Ok(TextBoxVerticalAlignment::Bottom),
                "justified" => Ok(TextBoxVerticalAlignment::Justified),
                "distributed" => Ok(TextBoxVerticalAlignment::Distributed),
                _ => Err(format!("unsupported text vertical alignment `{align}`")),
            })
            .transpose()?;
        Ok(Self {
            fill_color: value.fill_color,
            clear_fill: value.clear_fill,
            border_color: value.border_color,
            clear_border: value.clear_border,
            layout,
            wrap_side,
            inset_left: value.inset_left,
            inset_top: value.inset_top,
            inset_right: value.inset_right,
            inset_bottom: value.inset_bottom,
            text_vertical_align,
            rotation_deg: value.rotation_deg,
            width_px: value.width_px,
            height_px: value.height_px,
            horizontal_align: value.horizontal_align,
            vertical_align: value.vertical_align,
            dist_top: value.dist_top,
            dist_bottom: value.dist_bottom,
            dist_left: value.dist_left,
            dist_right: value.dist_right,
        })
    }
}

impl TryFrom<ImageFormatInput> for ImageFormatSpec {
    type Error = String;

    fn try_from(value: ImageFormatInput) -> Result<Self, Self::Error> {
        let layout = value
            .layout
            .as_deref()
            .map(|layout| match layout {
                "inline" => Ok(ImageLayoutPreset::Inline),
                "square" => Ok(ImageLayoutPreset::Square),
                "tight" => Ok(ImageLayoutPreset::Tight),
                "through" => Ok(ImageLayoutPreset::Through),
                "topAndBottom" => Ok(ImageLayoutPreset::TopAndBottom),
                "behindText" => Ok(ImageLayoutPreset::BehindText),
                "inFrontOfText" => Ok(ImageLayoutPreset::InFrontOfText),
                _ => Err(format!("unsupported image layout `{layout}`")),
            })
            .transpose()?;
        let spec = ImageFormatSpec {
            width_px: value.width_px,
            height_px: value.height_px,
            crop: value.crop,
            clear_crop: value.clear_crop,
            layout,
            horizontal_align: value.horizontal_align,
            vertical_align: value.vertical_align,
            x: value.x,
            y: value.y,
            rotation_deg: value.rotation_deg,
            flip_horizontal: value.flip_horizontal,
            flip_vertical: value.flip_vertical,
            dist_top: value.dist_top,
            dist_bottom: value.dist_bottom,
            dist_left: value.dist_left,
            dist_right: value.dist_right,
        };
        spec.validate()?;
        Ok(spec)
    }
}

/// Snapshots the host's local wall-clock for resolving `DATE`/`TIME` fields.
fn current_field_datetime() -> FieldDateTime {
    let now = js_sys::Date::new_0();
    FieldDateTime {
        year: now.get_full_year() as i32,
        // `js_sys::Date::get_month` is 0-based; the model is 1-based.
        month: now.get_month() + 1,
        day: now.get_date(),
        hour: now.get_hours(),
        minute: now.get_minutes(),
        second: now.get_seconds(),
        weekday: now.get_day(),
    }
}

/// Parses revision metadata supplied by the web UI and normalises blank strings.
fn parse_revision_metadata(metadata: JsValue) -> Result<RevisionMetadata, JsValue> {
    if metadata.is_null() || metadata.is_undefined() {
        return Ok(RevisionMetadata::default());
    }

    let object = metadata
        .dyn_into::<Object>()
        .map_err(|_| JsValue::from_str("record changes metadata must be an object"))?;

    let author = extract_optional_string(&object, "author")?;
    let date = extract_optional_string(&object, "date")?;
    let date_utc = extract_optional_string(&object, "dateUtc")?;

    Ok(RevisionMetadata {
        author,
        date,
        date_utc,
    })
}

/// Extracts an optional trimmed string from a JavaScript object property.
fn extract_optional_string(object: &Object, key: &str) -> Result<Option<String>, JsValue> {
    let value = Reflect::get(object, &JsValue::from_str(key))?;
    if value.is_null() || value.is_undefined() {
        return Ok(None);
    }

    let Some(text) = value.as_string() else {
        return Err(JsValue::from_str(&format!(
            "metadata field `{key}` must be a string"
        )));
    };

    let trimmed = text.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

impl CanvasDocumentState {
    fn enter_hf_editing_mode(&mut self, mode: EditingMode) {
        if self.editing_mode == EditingMode::Body {
            self.hf_body_selection = self.selection;
        }
        self.editing_mode = mode;
    }

    fn exit_hf_editing_mode(&mut self) -> Option<SelectionRange> {
        if self.editing_mode != EditingMode::Body {
            self.selection = self.hf_body_selection.take();
        }
        self.editing_mode = EditingMode::Body;
        if let Some(document) = self.word_doc.as_mut() {
            document.hf_edit_section = None;
        }
        self.hf_edit_cursor = None;
        self.hf_selection_anchor = None;
        self.caret_goal_x = None;
        self.selection
    }

    fn history_flags(&self) -> (bool, bool) {
        (
            self.command_executor.can_undo() || !self.hf_undo_stack.is_empty(),
            self.command_executor.can_redo() || !self.hf_redo_stack.is_empty(),
        )
    }
}

#[derive(Clone, Debug, Default)]
struct InlineFormattingState {
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    underline_kind: Option<String>,
    strike: Option<bool>,
    double_strike: Option<bool>,
    baseline: Option<String>,
    font_size: Option<f32>,
    font_family: Option<String>,
    color: Option<String>,
    highlight: Option<String>,
    /// Caret position the pending formatting was armed at. Word discards
    /// pending formatting as soon as the caret moves, so consumers only honor
    /// this state while the collapsed caret still sits exactly here.
    armed_at: Option<TextPosition>,
}

#[derive(Clone, Copy, Debug)]
enum HistoryAction {
    Undo,
    Redo,
}

impl InlineFormattingState {
    fn as_run_properties(&self) -> Option<RunProperties> {
        let mut props = RunProperties::default();
        let mut has_value = false;

        if let Some(value) = self.bold {
            props.bold = Some(value);
            props.bold_complex = Some(value);
            has_value = true;
        }

        if let Some(value) = self.italic {
            props.italic = Some(value);
            props.italic_complex = Some(value);
            has_value = true;
        }

        if let Some(value) = self.underline {
            props.underline = Some(value);
            has_value = true;
        }
        if let Some(value) = self.underline_kind.clone() {
            props.underline_kind = Some(value.into());
            has_value = true;
        }

        if let Some(value) = self.strike {
            props.strike = Some(value);
            has_value = true;
        }
        if let Some(value) = self.double_strike {
            props.double_strike = Some(value);
            has_value = true;
        }

        if let Some(value) = self.baseline.clone() {
            props.baseline = Some(value.into());
            has_value = true;
        }

        if let Some(value) = self.font_size {
            props.font_size = Some(value);
            has_value = true;
        }

        if let Some(value) = self.font_family.clone() {
            let font: Rc<str> = Rc::from(value.as_str());
            props.font_family = Some(font.clone());
            props.fonts = Some(crate::commands::set_font_family::ribbon_run_fonts(&font));
            has_value = true;
        }

        if let Some(value) = self.color.clone() {
            props.color = Some(value.into());
            has_value = true;
        }

        if let Some(value) = self.highlight.clone() {
            props.highlight = Some(value.into());
            has_value = true;
        }

        if has_value {
            Some(props)
        } else {
            None
        }
    }

    /// Pending formatting is only live while the collapsed caret still sits at
    /// the position it was armed at; any caret move discards it, like Word.
    fn is_armed_for(&self, selection: &SelectionRange) -> bool {
        selection.is_collapsed() && self.armed_at == Some(selection.focus)
    }

    fn pending_run_properties_for(&self, selection: &SelectionRange) -> Option<RunProperties> {
        if !self.is_armed_for(selection) {
            return None;
        }
        self.as_run_properties()
    }

    /// Consumes the pending formatting for a text insertion at `selection`.
    /// The caret advances past the inserted text, so the armed position can
    /// never match again; disarming keeps a later caret return from reviving
    /// stale state.
    fn take_pending_for(&mut self, selection: &SelectionRange) -> Option<RunProperties> {
        let props = self.pending_run_properties_for(selection);
        if props.is_some() {
            self.armed_at = None;
        }
        props
    }

    fn set_color(&mut self, value: String) {
        self.color = Some(value);
    }

    fn set_highlight(&mut self, value: String) {
        self.highlight = Some(value);
    }

    fn set_bold(&mut self, value: bool) {
        self.bold = Some(value);
    }

    fn set_italic(&mut self, value: bool) {
        self.italic = Some(value);
    }

    fn set_underline(&mut self, value: bool) {
        self.underline = Some(value);
        if !value {
            self.underline_kind = None;
        }
    }

    fn set_underline_kind(&mut self, value: String) {
        self.underline = Some(true);
        self.underline_kind = Some(value);
    }

    fn set_strike(&mut self, value: bool) {
        self.strike = Some(value);
        self.double_strike = Some(false);
    }

    fn set_double_strike(&mut self) {
        self.strike = Some(false);
        self.double_strike = Some(true);
    }

    fn set_baseline(&mut self, value: &str, enabled: bool) {
        // At a collapsed caret, `None` means "no direct override". Inserting
        // text would then inherit superscript/subscript from the adjacent run.
        // OOXML's explicit `baseline` value is the Word-compatible off state.
        self.baseline = Some(if enabled {
            value.to_string()
        } else {
            "baseline".to_string()
        });
    }

    fn set_font_size(&mut self, value: f32) {
        self.font_size = Some(value);
    }

    fn set_font_family(&mut self, value: String) {
        self.font_family = Some(value);
    }

    fn clear(&mut self) {
        self.bold = None;
        self.italic = None;
        self.underline = None;
        self.underline_kind = None;
        self.strike = None;
        self.double_strike = None;
        self.baseline = None;
        self.font_size = None;
        self.font_family = None;
        self.color = None;
        self.highlight = None;
        self.armed_at = None;
    }
}

/// Per-field formatting summary over every run a selection covers, with
/// Word's ribbon semantics: toggle flags read as pressed only when ALL text
/// carries them, and value fields (font, size, color, highlight) blank out
/// when the selection mixes values.
#[derive(Default, Debug, PartialEq)]
struct SelectionRunSummary {
    saw_text: bool,
    all_bold: bool,
    all_italic: bool,
    all_underline: bool,
    all_strike: bool,
    baseline: UniformField<String>,
    font_size: UniformField<ordered_size::SizeKey>,
    font_family: UniformField<String>,
    color: UniformField<String>,
    highlight: UniformField<String>,
}

/// Tracks whether one formatting field holds the same value over every run.
#[derive(Debug, PartialEq, Default)]
enum UniformField<T: PartialEq> {
    #[default]
    Unseen,
    Uniform(Option<T>),
    Mixed,
}

impl<T: PartialEq> UniformField<T> {
    fn observe(&mut self, value: Option<T>) {
        match self {
            UniformField::Unseen => *self = UniformField::Uniform(value),
            UniformField::Uniform(current) => {
                if *current != value {
                    *self = UniformField::Mixed;
                }
            }
            UniformField::Mixed => {}
        }
    }

    fn is_mixed(&self) -> bool {
        matches!(self, UniformField::Mixed)
    }

    fn uniform(&self) -> Option<&T> {
        match self {
            UniformField::Uniform(Some(value)) => Some(value),
            _ => None,
        }
    }
}

#[path = "canvas_view/word_autocorrect.rs"]
mod word_autocorrect;
#[path = "canvas_view/word_checks.rs"]
mod word_checks;
#[path = "canvas_view/word_controls.rs"]
mod word_controls;
#[path = "canvas_view/word_equation.rs"]
mod word_equation;
#[path = "canvas_view/word_ink.rs"]
mod word_ink;
#[path = "canvas_view/word_layout_objects.rs"]
mod word_layout_objects;
#[path = "canvas_view/word_lists.rs"]
mod word_lists;
#[path = "canvas_view/word_mailings.rs"]
mod word_mailings;
#[path = "canvas_view/word_misc.rs"]
mod word_misc;
#[path = "canvas_view/word_models.rs"]
mod word_models;
#[path = "canvas_view/word_objects.rs"]
mod word_objects;
#[path = "canvas_view/word_parity.rs"]
mod word_parity;
#[path = "canvas_view/word_parts.rs"]
mod word_parts;
#[path = "canvas_view/word_ranges.rs"]
mod word_ranges;
#[path = "canvas_view/word_references.rs"]
mod word_references;
#[path = "canvas_view/word_review.rs"]
mod word_review;
#[path = "canvas_view/word_session.rs"]
mod word_session;
#[path = "canvas_view/word_tools.rs"]
mod word_tools;

mod ordered_size {
    /// f32 font sizes wrapped for equality comparison in [`super::UniformField`].
    #[derive(Debug, PartialEq)]
    pub struct SizeKey(pub f32);
}

/// Installs (or clears) the loaded document's `fontTable.xml` substitutes
/// into the font resolver. Lives at the editor boundary because the shared
/// document core stays layout-free; call it whenever a NEW document becomes
/// the view's active document.
pub(crate) fn install_document_font_substitutes(word_doc: &WordDocument) {
    jade_docx_layout::font_resolver::set_document_font_alt_names(
        word_doc
            .font_table_part
            .as_ref()
            .map(|table| {
                table
                    .alt_names()
                    .map(|(name, alt)| (name.to_string(), vec![alt.to_string()]))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    );
}

const HANDLE_HIT_THRESHOLD: f64 = 4.0;
const MIN_COLUMN_WIDTH_PX: f64 = 12.0;
const WIDTH_CHANGE_EPSILON: f64 = 0.5;
const MIN_ROW_HEIGHT_PX: f64 = 12.0;
const ROW_HANDLE_PRIORITY_PENALTY: f64 = -0.25;

/// Converts physical pointer movement into logical column-width movement.
/// bidiVisual places the lower logical index on the physical right, so its
/// width changes in the opposite direction at an interior boundary.
fn column_resize_delta(pointer_delta: f64, reverse_delta: bool) -> f64 {
    if reverse_delta {
        -pointer_delta
    } else {
        pointer_delta
    }
}

#[derive(Clone, Debug, Default)]
enum PointerDragState {
    #[default]
    None,
    Selecting {
        anchor: TextPosition,
        /// If the pointer-down hit landed inside a table cell, record the
        /// anchor cell (by `CellLayout.order`) so `pointer_move` can upgrade
        /// to a cell-range selection once the cursor leaves the anchor cell.
        anchor_cell: Option<usize>,
        /// A pointer-down in the leading-edge hotspot selects the whole cell
        /// immediately and keeps subsequent dragging in cell-selection mode.
        cell_mode: bool,
    },
    ColumnResize(ColumnResizeState),
    RowResize(RowResizeState),
    /// Dragging inside a header or footer: the pointer-down cursor.
    HfSelecting {
        anchor: HfEditCursor,
    },
}

#[derive(Clone, Copy, Debug)]
enum ColumnResizeKind {
    BetweenColumns,
    TableRightEdge,
}

#[derive(Clone, Debug)]
struct ColumnResizeState {
    table_order_index: usize,
    column_index: usize,
    kind: ColumnResizeKind,
    initial_left_width: f64,
    current_left_width: f64,
    initial_right_width: Option<f64>,
    current_right_width: Option<f64>,
    total_width: Option<f64>,
    initial_column_widths: Vec<f64>,
    initial_pointer_x: f64,
    reverse_delta: bool,
}

impl ColumnResizeState {
    fn between_columns(
        table_order_index: usize,
        column_index: usize,
        initial_left_width: f64,
        initial_right_width: f64,
        initial_column_widths: Vec<f64>,
        initial_pointer_x: f64,
        reverse_delta: bool,
    ) -> Self {
        let total_width = initial_left_width + initial_right_width;
        Self {
            table_order_index,
            column_index,
            kind: ColumnResizeKind::BetweenColumns,
            initial_left_width,
            current_left_width: initial_left_width,
            initial_right_width: Some(initial_right_width),
            current_right_width: Some(initial_right_width),
            total_width: Some(total_width),
            initial_column_widths,
            initial_pointer_x,
            reverse_delta,
        }
    }

    fn table_right_edge(
        table_order_index: usize,
        column_index: usize,
        initial_left_width: f64,
        initial_column_widths: Vec<f64>,
        initial_pointer_x: f64,
    ) -> Self {
        Self {
            table_order_index,
            column_index,
            kind: ColumnResizeKind::TableRightEdge,
            initial_left_width,
            current_left_width: initial_left_width,
            initial_right_width: None,
            current_right_width: None,
            total_width: None,
            initial_column_widths,
            initial_pointer_x,
            reverse_delta: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum RowResizeKind {
    BetweenRows,
    TableBottomEdge,
}

/// Grid coordinates of the table cell containing the caret, plus the
/// resolved table dimensions. Backs the Word-style table layout commands
/// (行/列の挿入・削除) and the `caretTableContext` UI query.
#[derive(Clone, Copy, Debug)]
struct CaretTableCellContext {
    table_order_index: usize,
    row: usize,
    column: usize,
    row_count: usize,
    column_count: usize,
}

/// The table cells a ribbon command works on: Word applies 行/列の削除, the
/// cell shading and the border editors to the whole selected cell range, not
/// just the cell holding the caret. Falls back to that single cell when there
/// is no range selection.
#[derive(Clone, Debug)]
struct TableRangeContext {
    table_order_index: usize,
    /// Every selected cell, in the renderer's row-major registration order.
    cells: Vec<TableCellSelection>,
    /// Grid rows the selection covers, ascending and de-duplicated.
    rows: Vec<usize>,
    /// Grid columns the selection covers (spans expanded), ascending.
    columns: Vec<usize>,
    /// The caret's own cell, which Word uses as the template for inserted
    /// rows and columns.
    anchor_row: usize,
    anchor_column: usize,
    row_count: usize,
    column_count: usize,
}

/// Grid rows a cell selection covers, ascending and de-duplicated.
fn selected_grid_rows(cells: &[TableCellSelection]) -> Vec<usize> {
    let mut rows: Vec<usize> = cells.iter().map(|cell| cell.row).collect();
    rows.sort_unstable();
    rows.dedup();
    rows
}

/// Grid columns a cell selection covers, expanding each cell's `gridSpan` so a
/// merged cell counts every slot it occupies. Ascending and de-duplicated.
fn selected_grid_columns(cells: &[TableCellSelection]) -> Vec<usize> {
    let mut columns: Vec<usize> = cells
        .iter()
        .flat_map(|cell| cell.column..cell.column.saturating_add(cell.column_span.max(1)))
        .collect();
    columns.sort_unstable();
    columns.dedup();
    columns
}

/// Where Word leaves the caret after deleting `removed` rows (or columns) from
/// a track of `count`: in the one that slid into the first deleted slot, or in
/// the last surviving one when the deletion ran to the end of the table.
/// `None` when the deletion emptied the table.
/// The list item a collapsed caret sits at the start of, as `(paragraph,
/// is empty)`. Word gives Tab, Shift+Tab, Backspace and Enter list meanings
/// there. `w:numId="0"` switches numbering off, so it is no list item.
fn list_item_at_caret(word_doc: &WordDocument, selection: SelectionRange) -> Option<(usize, bool)> {
    if !selection.is_collapsed() || selection.focus.offset != 0 {
        return None;
    }
    let paragraph = word_doc.paragraph(selection.focus.paragraph)?;
    paragraph_list_id(paragraph)?;
    let empty = paragraph.runs.iter().all(|run| {
        run.text.is_empty() && run.image.is_none() && run.text_box.is_none() && run.field.is_none()
    });
    Some((selection.focus.paragraph, empty))
}

fn paragraph_list_id(paragraph: &crate::document::Paragraph) -> Option<&str> {
    paragraph.properties.as_ref()?.numbering.as_ref()?.list_id()
}

/// The paragraphs Word's Tab / Shift+Tab re-level instead of typing a tab:
/// the list item a collapsed caret starts, or — with a selection made only of
/// list items — every selected item, when the selection spans paragraphs or
/// starts at an item's start.
fn list_tab_paragraphs(
    word_doc: &WordDocument,
    selection: SelectionRange,
) -> Option<(usize, usize)> {
    if selection.is_collapsed() {
        return list_item_at_caret(word_doc, selection)
            .map(|(paragraph, _)| (paragraph, paragraph));
    }
    let (start, end) = selection.normalized();
    if start.paragraph == end.paragraph && start.offset != 0 {
        return None;
    }
    (start.paragraph..=end.paragraph)
        .all(|index| {
            word_doc
                .paragraph(index)
                .and_then(paragraph_list_id)
                .is_some()
        })
        .then_some((start.paragraph, end.paragraph))
}

/// Where Word's triple-click selection of `paragraph` ends: past its
/// paragraph mark (the start of the next paragraph in the same story or
/// cell), so Delete removes the paragraph. The last paragraph of a story has
/// nothing after its mark to reach, so the selection stops at its text end.
fn paragraph_selection_end(
    word_doc: &WordDocument,
    paragraph: usize,
    text_end: usize,
) -> TextPosition {
    if paragraph + 1 < word_doc.paragraph_count()
        && word_doc.paragraphs_share_a_container(paragraph, paragraph + 1)
    {
        TextPosition::new(paragraph + 1, 0)
    } else {
        TextPosition::new(paragraph, text_end)
    }
}

fn track_index_after_delete(removed: &[usize], count: usize) -> Option<usize> {
    let remaining = count.checked_sub(removed.len())?;
    if remaining == 0 {
        return None;
    }
    let first = removed.first().copied().unwrap_or(0);
    Some(first.min(remaining - 1))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TableBordersInput {
    #[serde(default)]
    top: Option<HeadlessBorderEdge>,
    #[serde(default)]
    bottom: Option<HeadlessBorderEdge>,
    #[serde(default)]
    left: Option<HeadlessBorderEdge>,
    #[serde(default)]
    right: Option<HeadlessBorderEdge>,
    #[serde(default)]
    inside_h: Option<HeadlessBorderEdge>,
    #[serde(default)]
    inside_v: Option<HeadlessBorderEdge>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PageBordersInput {
    #[serde(default)]
    top: Option<HeadlessBorderEdge>,
    #[serde(default)]
    bottom: Option<HeadlessBorderEdge>,
    #[serde(default)]
    left: Option<HeadlessBorderEdge>,
    #[serde(default)]
    right: Option<HeadlessBorderEdge>,
    #[serde(default)]
    offset_from: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CoverPageParagraphInput {
    #[serde(default)]
    text: String,
    #[serde(default)]
    bold: bool,
    #[serde(default)]
    italic: bool,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    font_size: Option<f32>,
    #[serde(default)]
    alignment: Option<String>,
    #[serde(default)]
    space_before: Option<f32>,
    #[serde(default)]
    space_after: Option<f32>,
    #[serde(default)]
    shading_color: Option<String>,
    #[serde(default)]
    border_top: bool,
    #[serde(default)]
    border_bottom: bool,
    #[serde(default)]
    border_left: bool,
    #[serde(default)]
    border_right: bool,
    #[serde(default)]
    indent_left: Option<f32>,
    #[serde(default)]
    indent_right: Option<f32>,
}

impl From<CoverPageParagraphInput> for HeadlessCoverParagraph {
    fn from(value: CoverPageParagraphInput) -> Self {
        Self {
            text: value.text,
            bold: value.bold,
            italic: value.italic,
            color: value.color,
            font_size: value.font_size,
            alignment: value.alignment,
            space_before: value.space_before,
            space_after: value.space_after,
            shading_color: value.shading_color,
            border_top: value.border_top,
            border_bottom: value.border_bottom,
            border_left: value.border_left,
            border_right: value.border_right,
            indent_left: value.indent_left,
            indent_right: value.indent_right,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PageBorderEdgeStateJs {
    style: String,
    width: Option<f32>,
    color: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PageBordersStateJs {
    top: Option<PageBorderEdgeStateJs>,
    bottom: Option<PageBorderEdgeStateJs>,
    left: Option<PageBorderEdgeStateJs>,
    right: Option<PageBorderEdgeStateJs>,
    offset_from: String,
}

impl TableBordersInput {
    fn is_empty(&self) -> bool {
        self.top.is_none()
            && self.bottom.is_none()
            && self.left.is_none()
            && self.right.is_none()
            && self.inside_h.is_none()
            && self.inside_v.is_none()
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TableCellMarginsInput {
    #[serde(default)]
    top: Option<f32>,
    #[serde(default)]
    bottom: Option<f32>,
    #[serde(default)]
    left: Option<f32>,
    #[serde(default)]
    right: Option<f32>,
}

impl TableCellMarginsInput {
    fn is_empty(&self) -> bool {
        self.top.is_none() && self.bottom.is_none() && self.left.is_none() && self.right.is_none()
    }
}

#[derive(Clone, Debug)]
struct RowResizeState {
    table_order_index: usize,
    row_index: usize,
    kind: RowResizeKind,
    initial_top_height: f64,
    current_top_height: f64,
    initial_bottom_height: Option<f64>,
    current_bottom_height: Option<f64>,
    total_height: Option<f64>,
    initial_pointer_y: f64,
}

impl RowResizeState {
    fn between_rows(
        table_order_index: usize,
        row_index: usize,
        initial_top_height: f64,
        initial_bottom_height: f64,
        initial_pointer_y: f64,
    ) -> Self {
        let total_height = initial_top_height + initial_bottom_height;
        Self {
            table_order_index,
            row_index,
            kind: RowResizeKind::BetweenRows,
            initial_top_height,
            current_top_height: initial_top_height,
            initial_bottom_height: Some(initial_bottom_height),
            current_bottom_height: Some(initial_bottom_height),
            total_height: Some(total_height),
            initial_pointer_y,
        }
    }

    fn table_bottom_edge(
        table_order_index: usize,
        row_index: usize,
        initial_height: f64,
        initial_pointer_y: f64,
    ) -> Self {
        Self {
            table_order_index,
            row_index,
            kind: RowResizeKind::TableBottomEdge,
            initial_top_height: initial_height,
            current_top_height: initial_height,
            initial_bottom_height: None,
            current_bottom_height: None,
            total_height: None,
            initial_pointer_y,
        }
    }
}

fn resolved_ruler_indent(properties: &ParagraphProperties, unit_pt: f64) -> JsValue {
    let indent = properties.indent.as_ref();
    let bidi = properties.bidi.unwrap_or(false);
    serde_json::json!({
        "left": indent.and_then(|i| jade_docx_layout::line_wrap::resolve_indent_left_pt(i, bidi, unit_pt)).unwrap_or(0.0),
        "right": indent.and_then(|i| jade_docx_layout::line_wrap::resolve_indent_right_pt(i, bidi, unit_pt)).unwrap_or(0.0),
        "first": indent.and_then(|i| jade_docx_layout::line_wrap::resolve_indent_first_line_pt(i, unit_pt, true)).unwrap_or(0.0),
    }).serialize(&serde_wasm_bindgen::Serializer::json_compatible()).unwrap_or(JsValue::NULL)
}

#[derive(Clone, Copy)]
struct FormatPreviewState {
    revision: u64,
    selection: Option<SelectionRange>,
}

/// Story previews never enter the bounded undo stack or clear existing redo.
struct HfFormatPreviewState {
    document: WordDocument,
    cursor: HfEditCursor,
    anchor: Option<HfEditCursor>,
    mode: EditingMode,
    variant: HfVariant,
    revision: u64,
    body_revision: u64,
}

#[wasm_bindgen]
#[derive(Clone)]
pub struct OffscreenDocumentView {
    renderer: Rc<RefCell<CanvasRenderer>>,
    state: Rc<RefCell<CanvasDocumentState>>,
    drag_state: Rc<RefCell<PointerDragState>>,
    /// Whether the column/row drag in progress is applied to the document as
    /// a live preview (its history entry is retracted before the next one).
    table_resize_preview: Rc<Cell<bool>>,
    /// The live preview of a gallery choice under the pointer: the history
    /// revision its provisional edit left and any selection to restore.
    style_preview: Rc<Cell<Option<FormatPreviewState>>>,
    hf_format_preview: Rc<RefCell<Option<HfFormatPreviewState>>>,
    resize_debug: Rc<RefCell<Option<String>>>,
    pending_image_wait: Rc<RefCell<Option<Promise>>>,
}

const IMAGE_WAIT_DEBOUNCE_MS: i32 = 16;

fn valid_paragraph_indents(left: f32, right: f32) -> bool {
    left.is_finite() && right.is_finite()
}

fn valid_optional_paragraph_metrics(
    left: Option<f32>,
    right: Option<f32>,
    before: Option<f32>,
    after: Option<f32>,
) -> bool {
    [left, right].into_iter().flatten().all(f32::is_finite)
        && [before, after]
            .into_iter()
            .flatten()
            .all(|value| value.is_finite() && value >= 0.0)
}

#[cfg(test)]
fn stepped_start_indent_chars(
    indent: Option<&crate::document::Indent>,
    bidi: bool,
    delta: i32,
) -> (Option<i32>, Option<i32>) {
    let current = indent
        .and_then(|indent| {
            if bidi {
                indent.physical_right_chars(true)
            } else {
                indent.physical_left_chars(false)
            }
        })
        .unwrap_or(0);
    let next = current.saturating_add(delta).max(0);
    if bidi {
        (None, Some(next))
    } else {
        (Some(next), None)
    }
}

/// Keeps the renderer pointed at the live document across an edit.
///
/// Edits drop the renderer's cached `Rc<Document>` before mutating so
/// `Rc::make_mut` does not deep-clone, and rely on the edit render to set it
/// again. An edit that fails in between (a rejected offset, a failed undo)
/// returns before that render, and the renderer used to keep no document:
/// every later `rerenderWithoutSnapshots` then returned without painting, and
/// a bitmap fetch of an evicted page came back fully transparent. Dropping this
/// guard restores the reference whenever the renderer is still empty; after a
/// successful render it is a no-op.
struct RendererDocumentCacheGuard {
    renderer: Rc<RefCell<CanvasRenderer>>,
    state: Rc<RefCell<CanvasDocumentState>>,
}

impl Drop for RendererDocumentCacheGuard {
    fn drop(&mut self) {
        // Never panic while unwinding an edit: skip if either cell is busy.
        let Ok(mut renderer) = self.renderer.try_borrow_mut() else {
            return;
        };
        if renderer.has_document_cache() {
            return;
        }
        let Ok(state) = self.state.try_borrow() else {
            return;
        };
        if let Some(document) = state
            .word_doc
            .as_ref()
            .and_then(|word_doc| word_doc.document.as_ref())
        {
            renderer.restore_document_cache(Rc::clone(document));
        }
    }
}

impl OffscreenDocumentView {
    fn inline_image_delete_target(
        word_doc: &WordDocument,
        selection: SelectionRange,
        direction: DeleteDirection,
    ) -> Option<(FloatingObjectTarget, usize)> {
        if !selection.is_collapsed() {
            return None;
        }

        let paragraph = word_doc.paragraph(selection.focus.paragraph)?;
        let mut offset = 0usize;
        let mut preceding_whitespace_start: Option<usize> = None;
        for (run_index, run) in paragraph.runs.iter().enumerate() {
            if let Some(text_box) = run.text_box.as_ref() {
                if matches!(text_box.layout.kind, RunImageKind::Inline) {
                    offset = offset.saturating_add(text_box.text.len());
                    preceding_whitespace_start = None;
                }
            }

            if let Some(image) = run.image.as_ref() {
                if matches!(image.layout.kind, RunImageKind::Inline) {
                    let start = offset;
                    let visual_start = preceding_whitespace_start.unwrap_or(start);
                    let should_delete = match direction {
                        DeleteDirection::Backward => selection.focus.offset == start,
                        DeleteDirection::Forward => {
                            selection.focus.offset == start
                                || selection.focus.offset == visual_start
                        }
                    };
                    if should_delete {
                        return Some((
                            FloatingObjectTarget {
                                paragraph_index: selection.focus.paragraph,
                                run_index,
                                kind: FloatingObjectKind::Image,
                            },
                            visual_start,
                        ));
                    }
                    preceding_whitespace_start = None;
                    continue;
                }
            }

            let run_start = offset;
            offset = offset.saturating_add(run.text.len());
            preceding_whitespace_start =
                if !run.text.is_empty() && run.text.chars().all(char::is_whitespace) {
                    Some(run_start)
                } else if run.text.is_empty() {
                    preceding_whitespace_start
                } else {
                    None
                };
        }

        None
    }

    fn clear_composition_preview_in_state(
        state: &mut CanvasDocumentState,
    ) -> Result<Option<SelectionRange>, JsValue> {
        let Some(mut preview) = state.composition_preview.take() else {
            return Ok(None);
        };
        let mut word_doc = state
            .word_doc
            .take()
            .ok_or_else(|| JsValue::from_str("No document loaded"))?;
        let mut selection = state.selection;
        let mut clear_error = None;
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            for command in preview.commands.iter_mut().rev() {
                if let Err(err) = command.undo(&mut ctx) {
                    clear_error = Some(err.to_string());
                    break;
                }
            }
        }
        if let Some(error) = clear_error {
            // A command that was successfully applied must be undoable. If an
            // invariant violation still makes undo fail, keep the best-effort
            // restored document instead of cloning the entire DOCX before every
            // normal composition update solely for this exceptional path.
            state.selection = preview.base_selection;
            state.word_doc = Some(word_doc);
            return Err(JsValue::from_str(&error));
        }
        state.selection = preview.base_selection;
        state.word_doc = Some(word_doc);
        Ok(preview.base_selection)
    }

    /// Drops the renderer's document cache for an edit and returns the guard
    /// that restores it if the edit exits without re-rendering. Keep the guard
    /// alive until the edit's render has run.
    #[must_use]
    fn drop_renderer_document_cache(&self) -> RendererDocumentCacheGuard {
        self.renderer.borrow_mut().drop_document_cache();
        RendererDocumentCacheGuard {
            renderer: Rc::clone(&self.renderer),
            state: Rc::clone(&self.state),
        }
    }

    fn clear_composition_preview(&self) -> Result<Option<SelectionRange>, JsValue> {
        if self.state.borrow().composition_preview.is_none() {
            return Ok(None);
        }
        let _document_cache = self.drop_renderer_document_cache();
        let mut state = self.state.borrow_mut();
        Self::clear_composition_preview_in_state(&mut state)
    }

    fn ensure_visible_page_range_for_edit(
        renderer: &mut CanvasRenderer,
        lazy_mode: bool,
        anchor_page: usize,
    ) {
        if lazy_mode {
            // Keep lazy edit work bounded to a small window around the caret page.
            // Expanding the visible range monotonically causes layout_page_limit growth,
            // which makes Enter latency increase over time.
            const EDIT_WINDOW_BEFORE: usize = 1;
            const EDIT_WINDOW_AFTER: usize = 2;
            const MAX_ACCEPTED_SPAN: usize = 8;

            let window_start = anchor_page.saturating_sub(EDIT_WINDOW_BEFORE).max(1);
            let window_end = anchor_page
                .saturating_add(EDIT_WINDOW_AFTER)
                .max(window_start);

            if let Some(range) = renderer.visible_page_range() {
                let start = *range.start();
                let end = *range.end();
                let span = end.saturating_sub(start);
                let anchor_inside = anchor_page >= start && anchor_page <= end;
                if anchor_inside && span <= MAX_ACCEPTED_SPAN {
                    return;
                }
            }

            renderer.set_visible_page_range(Some(window_start..=window_end));
            return;
        }

        if let Some(range) = renderer.visible_page_range() {
            let start = *range.start();
            let end = *range.end();
            if anchor_page < start || anchor_page > end {
                let next_start = start.min(anchor_page);
                let next_end = end.max(anchor_page);
                renderer.set_visible_page_range(Some(next_start..=next_end));
            }
        } else if let Some(limit) = renderer.layout_page_limit() {
            renderer.set_visible_page_range(Some(1..=limit));
        }
    }

    fn lazy_visible_end(renderer: &CanvasRenderer, anchor_page: usize) -> usize {
        let fallback = anchor_page.saturating_add(2).max(1);
        renderer
            .visible_page_range()
            .as_ref()
            .map(|range| (*range.end()).max(anchor_page).max(1))
            .unwrap_or(fallback)
    }

    fn prepare_hf_edit_rerender(&self, mode: EditingMode, anchor_page_index: usize) {
        let lazy_mode = {
            let mut state = self.state.borrow_mut();
            // An edit or a formatting change invalidates the old vertical goal.
            state.caret_goal_x = None;
            state.lazy_mode
        };
        let anchor_page = anchor_page_index.saturating_add(1).max(1);
        let mut renderer = self.renderer.borrow_mut();
        renderer.set_editing_mode(mode);
        Self::ensure_visible_page_range_for_edit(&mut renderer, lazy_mode, anchor_page);
        if lazy_mode {
            let desired = Self::lazy_visible_end(&renderer, anchor_page);
            renderer.set_layout_page_limit(Some(desired));
        }
        renderer.mark_full_repaint();
    }

    fn hf_dirty_regions_to_js(&self, mode: EditingMode) -> JsValue {
        let state = self.state.borrow();
        let Some(cursor) = state.hf_edit_cursor else {
            return Array::new().into();
        };
        drop(state);

        let renderer = self.renderer.borrow();
        let page_width = renderer.page_width().max(1.0);
        let page_height = renderer.page_height_css().max(1.0);
        let page_range = renderer.visible_page_range().clone().unwrap_or_else(|| {
            cursor.page_index.saturating_add(1)..=cursor.page_index.saturating_add(1)
        });
        let array = Array::new();
        for page_1idx in page_range {
            let page = page_1idx.saturating_sub(1);
            let obj = Object::new();
            let (y, height) = match mode {
                EditingMode::Header => (0.0, renderer.header_edit_boundary_for_page(page)),
                EditingMode::Footer => {
                    let line_count = self.hf_line_count(page, EditingMode::Footer);
                    let boundary =
                        renderer
                            .footer_edit_boundary_for_page(page)
                            .unwrap_or_else(|| {
                                Self::footer_edit_boundary(
                                    page_height,
                                    renderer.margin_bottom(),
                                    line_count,
                                )
                            });
                    (boundary, page_height - boundary)
                }
                EditingMode::Body => (0.0, page_height),
            };
            let _ = Reflect::set(&obj, &JsValue::from_str("x"), &JsValue::from_f64(0.0));
            let _ = Reflect::set(
                &obj,
                &JsValue::from_str("y"),
                &JsValue::from_f64(y.max(0.0)),
            );
            let _ = Reflect::set(
                &obj,
                &JsValue::from_str("width"),
                &JsValue::from_f64(page_width),
            );
            let _ = Reflect::set(
                &obj,
                &JsValue::from_str("height"),
                &JsValue::from_f64(height.max(1.0)),
            );
            let _ = Reflect::set(
                &obj,
                &JsValue::from_str("page"),
                &JsValue::from_f64(page as f64),
            );
            array.push(&obj);
        }
        array.into()
    }

    async fn finish_hf_edit_result_async(&self, mode: EditingMode) -> Result<JsValue, JsValue> {
        let lazy_mode = self.state.borrow().lazy_mode;
        let dirty_regions = self.hf_dirty_regions_to_js(mode);
        if lazy_mode {
            let snapshots = self.capture_hf_active_page_after_render().await?;
            self.store_paged_images(snapshots);
        } else {
            self.rerender_document()?;
            let snapshots = self.capture_pages_after_render().await?;
            self.store_paged_images(snapshots);
        }

        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &self.canvas_size()?)?;
        Reflect::set(&result, &JsValue::from_str("pages"), &self.paged_images())?;
        Reflect::set(&result, &JsValue::from_str("dirtyRegions"), &dirty_regions)?;
        Reflect::set(
            &result,
            &JsValue::from_str("debugMutationKind"),
            &JsValue::from_str("header-footer-edit"),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("editingMode"),
            &JsValue::from_str(Self::hf_editing_mode_name(mode)),
        )?;
        self.write_hf_caret_to_result(&result)?;
        Self::write_history_flags(&result, self.history_flags())?;
        Ok(result.into())
    }

    async fn capture_hf_active_page_after_render(&self) -> Result<Option<Vec<JsValue>>, JsValue> {
        let Some(cursor) = self.state.borrow().hf_edit_cursor else {
            self.state.borrow_mut().paged_images.clear();
            return Ok(None);
        };
        let page_1idx = cursor.page_index.saturating_add(1).max(1);
        let previous_range = {
            let mut renderer = self.renderer.borrow_mut();
            let previous = renderer.visible_page_range().clone();
            renderer.set_visible_page_range(Some(page_1idx..=page_1idx));
            previous
        };

        let render_result = self.rerender_document();
        if let Err(err) = render_result {
            self.renderer
                .borrow_mut()
                .set_visible_page_range(previous_range);
            return Err(err);
        }
        let snapshots = self.capture_pages_after_render().await;
        self.renderer
            .borrow_mut()
            .set_visible_page_range(previous_range);
        snapshots
    }

    fn capture_hf_active_page_after_render_sync(&self) -> Result<Option<Vec<JsValue>>, JsValue> {
        let Some(cursor) = self.state.borrow().hf_edit_cursor else {
            self.state.borrow_mut().paged_images.clear();
            return Ok(None);
        };
        let page_1idx = cursor.page_index.saturating_add(1).max(1);
        let previous_range = {
            let mut renderer = self.renderer.borrow_mut();
            let previous = renderer.visible_page_range().clone();
            renderer.set_visible_page_range(Some(page_1idx..=page_1idx));
            previous
        };

        let render_result = self.rerender_document();
        if let Err(err) = render_result {
            self.renderer
                .borrow_mut()
                .set_visible_page_range(previous_range);
            return Err(err);
        }
        let snapshots = {
            let mut renderer_mut = self.renderer.borrow_mut();
            let (snapshots, _dirty_regions) = self.capture_paged_images(&mut renderer_mut)?;
            snapshots
        };
        self.renderer
            .borrow_mut()
            .set_visible_page_range(previous_range);
        Ok(snapshots)
    }

    fn schedule_image_wait(delay_ms: i32) -> Promise {
        Promise::new(&mut |resolve, _reject| {
            let resolve_for_callback = resolve.clone();
            let callback = Closure::once_into_js(move || {
                let _ = resolve_for_callback.call0(&JsValue::UNDEFINED);
            });

            if let Some(window) = web_sys::window() {
                let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                    callback.unchecked_ref(),
                    delay_ms,
                );
                return;
            }

            if let Ok(scope) = js_sys::global().dyn_into::<WorkerGlobalScope>() {
                let _ = scope.set_timeout_with_callback_and_timeout_and_arguments_0(
                    callback.unchecked_ref(),
                    delay_ms,
                );
                return;
            }

            let _ = resolve.call0(&JsValue::UNDEFINED);
        })
    }

    #[cfg_attr(feature = "otel", tracing::instrument(skip_all))]
    async fn wait_for_pending_images(&self) -> Result<(), JsValue> {
        let pending = { self.pending_image_wait.borrow().clone() };
        if let Some(pending) = pending {
            JsFuture::from(pending).await?;
            return Ok(());
        }

        let has_pending_images = {
            let renderer_ref = self.renderer.borrow();
            renderer_ref.has_pending_image_promises()
        };
        if !has_pending_images {
            return Ok(());
        }

        let renderer = Rc::clone(&self.renderer);
        let pending_state = Rc::clone(&self.pending_image_wait);
        let promise = future_to_promise(async move {
            let delay = OffscreenDocumentView::schedule_image_wait(IMAGE_WAIT_DEBOUNCE_MS);
            let _ = JsFuture::from(delay).await;

            let wait = {
                let renderer_ref = renderer.borrow();
                renderer_ref.drain_pending_image_promises()
            };

            let result = if let Some(wait) = wait {
                JsFuture::from(wait).await.map(|_| JsValue::UNDEFINED)
            } else {
                Ok(JsValue::UNDEFINED)
            };
            *pending_state.borrow_mut() = None;
            result
        });

        *self.pending_image_wait.borrow_mut() = Some(promise.clone());
        JsFuture::from(promise).await?;

        Ok(())
    }

    #[cfg_attr(feature = "otel", tracing::instrument(skip_all))]
    async fn capture_pages_after_render(&self) -> Result<Option<Vec<JsValue>>, JsValue> {
        self.wait_for_pending_images().await?;
        let mut renderer_mut = self.renderer.borrow_mut();
        let (snapshots, _dirty_regions) = self.capture_paged_images(&mut renderer_mut)?;
        let metadata = renderer_mut.table_resize_metadata();
        drop(renderer_mut);
        {
            let mut state_mut = self.state.borrow_mut();
            state_mut.table_resize = metadata;
        }
        Ok(snapshots)
    }

    /// Like `capture_pages_after_render`, but also returns the serialized dirty regions.
    /// This enables the frontend to apply partial bitmap updates during edits.
    #[cfg_attr(feature = "otel", tracing::instrument(skip_all))]
    async fn capture_pages_after_render_with_dirty(
        &self,
    ) -> Result<(Option<Vec<JsValue>>, JsValue), JsValue> {
        self.wait_for_pending_images().await?;
        let mut renderer_mut = self.renderer.borrow_mut();
        let (snapshots, dirty_regions) = self.capture_paged_images(&mut renderer_mut)?;
        let metadata = renderer_mut.table_resize_metadata();
        drop(renderer_mut);
        {
            let mut state_mut = self.state.borrow_mut();
            state_mut.table_resize = metadata;
        }
        Ok((snapshots, dirty_regions))
    }

    fn dirty_regions_to_js(dirty_regions: &[DirtyRect]) -> JsValue {
        let result = Array::new();
        for rect in dirty_regions {
            let obj = Object::new();
            let _ = Reflect::set(&obj, &JsValue::from_str("x"), &JsValue::from_f64(rect.x));
            let _ = Reflect::set(&obj, &JsValue::from_str("y"), &JsValue::from_f64(rect.y));
            let _ = Reflect::set(
                &obj,
                &JsValue::from_str("width"),
                &JsValue::from_f64(rect.width),
            );
            let _ = Reflect::set(
                &obj,
                &JsValue::from_str("height"),
                &JsValue::from_f64(rect.height),
            );
            let _ = Reflect::set(
                &obj,
                &JsValue::from_str("page"),
                &JsValue::from_f64(rect.page as f64),
            );
            result.push(&obj);
        }
        result.into()
    }

    fn selection_rects_to_js(rects: &[SelectionRect]) -> JsValue {
        let result = Array::new();
        for rect in rects {
            let obj = Object::new();
            let _ = Reflect::set(&obj, &JsValue::from_str("x"), &JsValue::from_f64(rect.x));
            let _ = Reflect::set(&obj, &JsValue::from_str("y"), &JsValue::from_f64(rect.y));
            let _ = Reflect::set(
                &obj,
                &JsValue::from_str("width"),
                &JsValue::from_f64(rect.width),
            );
            let _ = Reflect::set(
                &obj,
                &JsValue::from_str("height"),
                &JsValue::from_f64(rect.height),
            );
            let _ = Reflect::set(
                &obj,
                &JsValue::from_str("page"),
                &JsValue::from_f64(rect.page as f64),
            );
            result.push(&obj);
        }
        result.into()
    }

    fn collect_table_selection(&self) -> Option<(usize, Vec<TableCellSelection>)> {
        let layouts = {
            let renderer = self.renderer.borrow();
            renderer.selected_cells()
        }?;
        if layouts.len() < 2 {
            return None;
        }
        let first_table = layouts.first()?.table_id;
        if layouts.iter().any(|layout| layout.table_id != first_table) {
            return None;
        }
        let table_order_index = {
            let renderer = self.renderer.borrow();
            renderer.table_order_for(first_table)?
        };
        let selections = layouts
            .into_iter()
            .map(|layout| TableCellSelection {
                row: layout.row,
                column: layout.column,
                column_span: layout.column_span.max(1),
            })
            .collect::<Vec<_>>();
        Some((table_order_index, selections))
    }

    fn can_merge_selected_cells_internal(&self) -> bool {
        let (table_order_index, selections) = match self.collect_table_selection() {
            Some(value) => value,
            None => return false,
        };
        let state = self.state.borrow();
        let word_doc = match state.word_doc.as_ref() {
            Some(doc) => doc,
            None => return false,
        };
        MergeTableCellsCommand::preview_plan(word_doc, table_order_index, &selections).is_ok()
    }

    /// A cell selection (or any single cell currently occupied by the caret
    /// when inside a table) can be un-merged when at least one of the cells
    /// covered by the selection carries a `gridSpan > 1` or a `vMerge`
    /// marker. We feed the same selections list into the preview plan.
    fn can_unmerge_selected_cells_internal(&self) -> bool {
        let (table_order_index, selections) = match self.caret_or_selected_cells() {
            Some(value) => value,
            None => return false,
        };
        let state = self.state.borrow();
        let word_doc = match state.word_doc.as_ref() {
            Some(doc) => doc,
            None => return false,
        };
        UnmergeTableCellsCommand::preview_plan(word_doc, table_order_index, &selections).is_ok()
    }

    /// The cells a table command addresses: the selected cell range when one
    /// exists, otherwise the single cell holding the caret. Merge needs at
    /// least two cells and uses `collect_table_selection` directly; every
    /// other table command (unmerge, 行/列の削除, cell shading, borders) works
    /// on one cell just as well, which is what this adds.
    fn caret_or_selected_cells(&self) -> Option<(usize, Vec<TableCellSelection>)> {
        if let Some(value) = self.collect_table_selection() {
            return Some(value);
        }
        // Fall back to the single cell containing the caret/focus paragraph.
        let renderer = self.renderer.borrow();
        let range = renderer.selection_range()?;
        let paragraph_index = range.focus.paragraph;
        let cell_id = renderer.cell_id_for_paragraph(paragraph_index)?;
        let layout = renderer.cell_layout(cell_id)?;
        let table_order_index = renderer.table_order_for(layout.table_id)?;
        Some((
            table_order_index,
            vec![TableCellSelection {
                row: layout.row,
                column: layout.column,
                column_span: layout.column_span.max(1),
            }],
        ))
    }

    /// Whether a body-story command may run right now. While a header or
    /// footer is being edited the body selection is cleared, so a ribbon
    /// command that fell through to the body would land on paragraph 0 — a
    /// table, a section break or bold text appearing at the top of the
    /// document while the user is looking at the header. Those commands are
    /// no-ops until the story returns to the body; only the header/footer
    /// routed operations (text, Enter, Backspace, arrows, alignment, page
    /// numbers) run in that mode.
    fn body_commands_allowed(&self) -> bool {
        self.state.borrow().editing_mode == EditingMode::Body
    }

    /// Resolve the table cell containing the caret (focus position) into grid
    /// coordinates plus the resolved table dimensions. Mirrors the single-cell
    /// fallback used by `caret_or_selected_cells`. Returns `None` when the
    /// caret is not inside a rendered table cell.
    fn caret_table_cell(&self) -> Option<CaretTableCellContext> {
        let (table_order_index, row, column) = {
            let renderer = self.renderer.borrow();
            let range = renderer.selection_range()?;
            let paragraph_index = range.focus.paragraph;
            let cell_id = renderer.cell_id_for_paragraph(paragraph_index)?;
            // Look the cell up by its registry id. Matching `CellLayout::order`
            // against the id only worked for the first table of a document:
            // `order` restarts at zero per table, so the caret in any later
            // (or nested) table resolved to the wrong cell or to none at all
            // and every ribbon table command silently did nothing there.
            let layout = renderer.cell_layout(cell_id)?;
            let table_order_index = renderer.table_order_for(layout.table_id)?;
            (table_order_index, layout.row, layout.column)
        };
        let state = self.state.borrow();
        let word_doc = state.word_doc.as_ref()?;
        let (row_count, column_count) = word_doc.table_dimensions(table_order_index)?;
        Some(CaretTableCellContext {
            table_order_index,
            row,
            column,
            row_count,
            column_count,
        })
    }

    /// Resolve the cells a Word table command works on. Word's 行の削除,
    /// 列の削除, cell shading, borders, vertical alignment and row properties
    /// all apply to the selected cell range; only with a collapsed caret do
    /// they act on one cell. Returns `None` when the caret is outside a table.
    fn caret_table_range(&self) -> Option<TableRangeContext> {
        let (table_order_index, cells) = self.caret_or_selected_cells()?;
        let (row_count, column_count) = {
            let state = self.state.borrow();
            state
                .word_doc
                .as_ref()?
                .table_dimensions(table_order_index)?
        };
        // The caret cell is the template Word copies when inserting; with a
        // range dragged out of the renderer it may not resolve, so fall back
        // to the first cell of the range.
        let (anchor_row, anchor_column) = self
            .caret_table_cell()
            .filter(|cell| cell.table_order_index == table_order_index)
            .map(|cell| (cell.row, cell.column))
            .or_else(|| cells.first().map(|cell| (cell.row, cell.column)))?;
        Some(TableRangeContext {
            table_order_index,
            rows: selected_grid_rows(&cells),
            columns: selected_grid_columns(&cells),
            cells,
            anchor_row,
            anchor_column,
            row_count,
            column_count,
        })
    }

    /// Put the caret where Word leaves it after 行/列の削除: at the start of
    /// the cell that took the deleted one's place. Falls back to the generic
    /// selection clamp when the target cell is not in the rebuilt layout (the
    /// table moved off the laid-out page range, say).
    fn focus_table_cell(
        &self,
        table_order_index: usize,
        row: usize,
        column: usize,
        payload: JsValue,
    ) -> Result<JsValue, JsValue> {
        let paragraphs = {
            let renderer = self.renderer.borrow();
            let mut candidates: Vec<&jade_docx_layout::selection::CellLayout> = renderer
                .cell_layouts_debug_ref()
                .iter()
                .filter(|layout| {
                    renderer.table_order_for(layout.table_id) == Some(table_order_index)
                        && layout.row == row
                })
                .collect();
            candidates.sort_by_key(|layout| layout.column);
            candidates
                .iter()
                .find(|layout| {
                    let span = layout.column_span.max(1);
                    layout.column <= column && column < layout.column + span
                })
                .or_else(|| candidates.first())
                .map(|layout| layout.paragraphs.clone())
        };
        match paragraphs {
            Some(paragraphs) => self.select_table_cell_contents(&paragraphs, Some(payload), true),
            None => {
                self.clamp_state_selection_to_layout();
                Ok(payload)
            }
        }
    }

    /// Re-clamp the stored selection to the rebuilt layout. Structural table
    /// edits (delete row/column) can remove the paragraphs the caret pointed
    /// at; the selection manager clamps internally when rendering, but the
    /// state copy must be brought back in range too so follow-up commands do
    /// not address stale paragraph indices.
    fn clamp_state_selection_to_layout(&self) {
        let renderer = self.renderer.borrow();
        let mut state = self.state.borrow_mut();
        if let Some(selection) = state.selection {
            state.selection = Some(SelectionRange {
                anchor: renderer.clamp_text_position(selection.anchor),
                focus: renderer.clamp_text_position(selection.focus),
            });
        }
    }

    fn write_merge_state(&self, result: &Object) -> Result<(), JsValue> {
        let can_merge = self.can_merge_selected_cells_internal();
        Reflect::set(
            result,
            &JsValue::from_str("canMergeCells"),
            &JsValue::from_bool(can_merge),
        )?;
        let can_unmerge = self.can_unmerge_selected_cells_internal();
        Reflect::set(
            result,
            &JsValue::from_str("canUnmergeCells"),
            &JsValue::from_bool(can_unmerge),
        )?;
        Ok(())
    }

    fn write_selection_rects(
        result: &Object,
        renderer: &CanvasRenderer,
        selection: Option<SelectionRange>,
    ) -> Result<(), JsValue> {
        if !renderer.use_selection_overlay() {
            return Ok(());
        }
        let Some(range) = selection else {
            return Ok(());
        };
        if range.is_collapsed() && renderer.selected_cell_span().is_none() {
            return Ok(());
        }
        let rects = renderer.get_selection_rects();
        if rects.is_empty() {
            return Ok(());
        }
        let rects_js = Self::selection_rects_to_js(&rects);
        Reflect::set(result, &JsValue::from_str("selectionRects"), &rects_js)?;
        Ok(())
    }

    fn object_selection_payload(&self) -> Option<ObjectSelectionPayload> {
        let target = self.state.borrow().object_selection?;
        self.object_selection_payload_for(target)
    }

    /// The text box or shape the caret is typing inside, if any. Word keeps
    /// 図形の書式 available in that state, so the shape commands and the
    /// ribbon query fall back to it when no object is selected.
    fn caret_text_box_target(&self) -> Option<FloatingObjectTarget> {
        if self.state.borrow().editing_mode != EditingMode::Body {
            return None;
        }
        let focus = {
            let state = self.state.borrow();
            state
                .selection
                .or_else(|| self.renderer.borrow().selection_range())?
                .focus
        };
        let state = self.state.borrow();
        state
            .word_doc
            .as_ref()?
            .text_box_host_for_surface(focus.paragraph)
    }

    fn object_selection_payload_for(
        &self,
        target: FloatingObjectTarget,
    ) -> Option<ObjectSelectionPayload> {
        let renderer = self.renderer.borrow();
        let region = renderer.object_region(target)?;
        let selected_run_image = (target.kind == FloatingObjectKind::Image)
            .then(|| {
                self.state
                    .borrow()
                    .word_doc
                    .as_ref()
                    .and_then(|document| document.paragraph(target.paragraph_index))
                    .and_then(|paragraph| paragraph.runs.get(target.run_index))
                    .and_then(|run| run.image.clone())
            })
            .flatten();
        let selected_text_box = (target.kind == FloatingObjectKind::TextBox)
            .then(|| {
                self.state
                    .borrow()
                    .word_doc
                    .as_ref()
                    .and_then(|document| document.paragraph(target.paragraph_index))
                    .and_then(|paragraph| paragraph.runs.get(target.run_index))
                    .and_then(|run| run.text_box.clone())
            })
            .flatten();
        let chart = selected_run_image
            .as_ref()
            .and_then(|image| image.chart.clone());
        let image = if target.kind == FloatingObjectKind::Image {
            selected_run_image.as_ref().map(|image| {
                let layout = match image.layout.kind {
                    RunImageKind::Inline => "inline",
                    RunImageKind::Floating => match image.layout.wrap {
                        WrapMode::Square => "square",
                        WrapMode::Tight => "tight",
                        WrapMode::Through => "through",
                        WrapMode::TopAndBottom => "topAndBottom",
                        WrapMode::None if image.layout.behind_doc => "behindText",
                        WrapMode::None => "inFrontOfText",
                    },
                };
                SelectedImagePayload {
                    width_px: image.width_px.unwrap_or(region.width).max(1.0),
                    height_px: image.height_px.unwrap_or(region.height).max(1.0),
                    crop: image.crop,
                    layout: layout.to_string(),
                    horizontal_align: image
                        .layout
                        .position_h
                        .as_ref()
                        .and_then(|position| position.align.clone()),
                    vertical_align: image
                        .layout
                        .position_v
                        .as_ref()
                        .and_then(|position| position.align.clone()),
                    x: image
                        .layout
                        .position_h
                        .as_ref()
                        .and_then(|position| position.offset_px),
                    y: image
                        .layout
                        .position_v
                        .as_ref()
                        .and_then(|position| position.offset_px),
                    rotation_deg: image.layout.rotation_deg.unwrap_or(0.0),
                    flip_horizontal: image.layout.flip_horizontal,
                    flip_vertical: image.layout.flip_vertical,
                    dist_top: image.layout.dist_top.unwrap_or(0.0),
                    dist_bottom: image.layout.dist_bottom.unwrap_or(0.0),
                    dist_left: image.layout.dist_left.unwrap_or(0.0),
                    dist_right: image.layout.dist_right.unwrap_or(0.0),
                }
            })
        } else {
            None
        };
        let text_box = selected_text_box.as_ref().map(|text_box| {
            let border_style = match text_box.border.style {
                TextBoxBorderStyle::Solid => "solid",
                TextBoxBorderStyle::Dashed => "dashed",
                TextBoxBorderStyle::Dotted => "dotted",
                TextBoxBorderStyle::DashDot => "dashDot",
                TextBoxBorderStyle::DashDotDot => "dashDotDot",
                TextBoxBorderStyle::Double => "double",
                TextBoxBorderStyle::None => "none",
            };
            let layout = match (text_box.layout.kind, text_box.layout.wrap) {
                (RunImageKind::Inline, _) => "inline",
                (_, WrapMode::Square) => "square",
                (_, WrapMode::Tight) => "tight",
                (_, WrapMode::Through) => "through",
                (_, WrapMode::TopAndBottom) => "topAndBottom",
                (_, WrapMode::None) if text_box.layout.behind_doc => "behindText",
                (_, WrapMode::None) => "inFrontOfText",
            };
            let wrap_side = match text_box.layout.wrap_text {
                WrapTextSide::BothSides => "bothSides",
                WrapTextSide::Left => "left",
                WrapTextSide::Right => "right",
                WrapTextSide::Largest => "largest",
            };
            SelectedTextBoxPayload {
                shape: text_box.shape,
                fill_color: text_box.fill_color.clone(),
                border_color: text_box.border.color.clone(),
                border_style,
                is_line: text_box.shape.is_line(),
                layout: layout.to_string(),
                wrap_side: wrap_side.to_string(),
                width_px: text_box.width_px.unwrap_or(0.0),
                height_px: text_box.height_px.unwrap_or(0.0),
                dist_top: text_box.layout.dist_top.unwrap_or(0.0),
                dist_bottom: text_box.layout.dist_bottom.unwrap_or(0.0),
                dist_left: text_box.layout.dist_left.unwrap_or(0.0),
                dist_right: text_box.layout.dist_right.unwrap_or(0.0),
                inset_left: text_box.insets.left_px,
                inset_top: text_box.insets.top_px,
                inset_right: text_box.insets.right_px,
                inset_bottom: text_box.insets.bottom_px,
                text_vertical_align: match text_box.vertical_alignment {
                    TextBoxVerticalAlignment::Top => "top",
                    TextBoxVerticalAlignment::Center => "center",
                    TextBoxVerticalAlignment::Bottom => "bottom",
                    TextBoxVerticalAlignment::Justified => "justified",
                    TextBoxVerticalAlignment::Distributed => "distributed",
                },
                rotation_deg: text_box.layout.rotation_deg.unwrap_or(0.0),
            }
        });
        Some(ObjectSelectionPayload {
            kind: region.kind,
            paragraph_index: region.paragraph_index,
            anchor_paragraph_index: renderer
                .object_transform_preview_anchor(target)
                .unwrap_or(region.paragraph_index),
            run_index: region.run_index,
            page: region.page,
            x: region.x,
            y: region.y,
            width: region.width,
            height: region.height,
            rotation_deg: region.rotation_deg,
            image,
            text_box,
            chart,
        })
    }

    fn write_object_selection(&self, result: &Object) -> Result<(), JsValue> {
        let value = match self.object_selection_payload() {
            // Object-selection is a public JSON-shaped payload. In particular,
            // missing chart values are `null` in `ChartData`, not JavaScript's
            // `undefined`, so they survive worker structured cloning and host
            // form editing without changing the rectangular series shape.
            Some(selection) => selection
                .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
                .unwrap_or(JsValue::NULL),
            None => JsValue::NULL,
        };
        Reflect::set(result, &JsValue::from_str("objectSelection"), &value)?;
        Ok(())
    }

    /// Dispatches to the right `WordDocument::replace_*_paragraph_text`
    /// variant for the (mode, variant) pair selected by the user.
    fn replace_hf_line(
        word_doc: &mut WordDocument,
        mode: EditingMode,
        variant: HfVariant,
        line_index: usize,
        full_text: &str,
    ) -> Result<(), String> {
        match (mode, variant) {
            (EditingMode::Header, HfVariant::Default) => {
                word_doc.replace_header_paragraph_text(line_index, full_text)
            }
            (EditingMode::Header, HfVariant::First) => {
                word_doc.replace_header_first_paragraph_text(line_index, full_text)
            }
            (EditingMode::Header, HfVariant::Even) => {
                word_doc.replace_header_even_paragraph_text(line_index, full_text)
            }
            (EditingMode::Footer, HfVariant::Default) => {
                word_doc.replace_footer_paragraph_text(line_index, full_text)
            }
            (EditingMode::Footer, HfVariant::First) => {
                word_doc.replace_footer_first_paragraph_text(line_index, full_text)
            }
            (EditingMode::Footer, HfVariant::Even) => {
                word_doc.replace_footer_even_paragraph_text(line_index, full_text)
            }
            (EditingMode::Body, _) => Err("cannot replace HF line while editing body".to_string()),
        }
    }

    fn hf_role_variant(
        mode: EditingMode,
        variant: HfVariant,
    ) -> Option<(HeaderFooterRole, HeaderFooterVariant)> {
        let role = match mode {
            EditingMode::Header => HeaderFooterRole::Header,
            EditingMode::Footer => HeaderFooterRole::Footer,
            EditingMode::Body => return None,
        };
        let variant = match variant {
            HfVariant::Default => HeaderFooterVariant::Default,
            HfVariant::First => HeaderFooterVariant::First,
            HfVariant::Even => HeaderFooterVariant::Even,
        };
        Some((role, variant))
    }

    fn hf_line_count(&self, page_index: usize, mode: EditingMode) -> usize {
        let is_header = mode == EditingMode::Header;
        let variant = self.variant_for_page_index(page_index, is_header);
        let state = self.state.borrow();
        state
            .word_doc
            .as_ref()
            .and_then(|word_doc| Self::variant_lines(word_doc, mode, variant))
            .map(|lines| lines.len().max(1))
            .unwrap_or(1)
    }

    fn footer_edit_content_bottom(page_height: f64, margin_bottom: f64) -> f64 {
        page_height - margin_bottom * HF_EDIT_CONTENT_START_RATIO
    }

    fn footer_edit_boundary(page_height: f64, margin_bottom: f64, line_count: usize) -> f64 {
        let default_boundary = page_height - margin_bottom;
        let content_bottom = Self::footer_edit_content_bottom(page_height, margin_bottom);
        (content_bottom - HF_EDIT_LINE_HEIGHT * (line_count as f64 + 1.0)).min(default_boundary)
    }

    /// Resolves the `HeaderLine`s for a given (mode, variant) on a
    /// `WordDocument`: the story of the section being edited (see
    /// `WordDocument::hf_lines_for_edit`), so a title-page header or an
    /// inherited footer on an earlier section is edited in its own part
    /// rather than read from the final section's fields. Returns `None` when
    /// the requested variant isn't loaded — callers should fall back to
    /// `Default` or surface an error.
    fn variant_lines(
        word_doc: &WordDocument,
        mode: EditingMode,
        variant: HfVariant,
    ) -> Option<Vec<HeaderLine>> {
        let (role, hf_variant) = Self::hf_role_variant(mode, variant)?;
        word_doc.hf_lines_for_edit(role, hf_variant)
    }

    /// Returns the 0-based page index that contains the given canvas y
    /// coordinate. Returns 0 for pageless / degenerate geometries.
    fn page_index_at_y(&self, y: f64) -> usize {
        let renderer = self.renderer.borrow();
        if renderer.pageless_mode() {
            return 0;
        }
        let page_height = renderer.page_height_css();
        let page_gap = 16.0_f64;
        let page_with_gap = page_height + page_gap;
        if page_with_gap <= 0.0 {
            return 0;
        }
        (y / page_with_gap).max(0.0) as usize
    }

    /// Picks the header/footer variant that is visible on a given 0-based
    /// page, matching the renderer's per-page selection rules
    /// (`title_page` / `even_and_odd_headers`). Callers should only invoke
    /// this for Header / Footer editing modes — Body never has a variant.
    ///
    /// The rule set mirrors `select_header_footer_variant` in canvas_renderer
    /// so clicks land on whichever variant is currently being rendered:
    /// - page 0 + `title_page` + first variant exists → First
    /// - even page (1, 3, ... in 0-based = 2, 4, ... in 1-based)
    ///   + `even_and_odd_headers` + even variant exists → Even
    /// - otherwise → Default
    fn variant_for_page_index(&self, zero_based_page: usize, is_header: bool) -> HfVariant {
        let state = self.state.borrow();
        let Some(word_doc) = state.word_doc.as_ref() else {
            return HfVariant::Default;
        };
        let renderer = self.renderer.borrow();
        let section_index = renderer.section_index_for_zero_based_page(zero_based_page);
        let sections = word_doc.section_header_footers();
        let section = sections.get(section_index);
        let (first_present, even_present, title_page) = match section {
            Some(section) if is_header => (
                section.header_first.is_some(),
                section.header_even.is_some(),
                section.title_page,
            ),
            Some(section) => (
                section.footer_first.is_some(),
                section.footer_even.is_some(),
                section.title_page,
            ),
            None if is_header => (
                word_doc.header_lines_first.is_some(),
                word_doc.header_lines_even.is_some(),
                word_doc.title_page,
            ),
            None => (
                word_doc.footer_lines_first.is_some(),
                word_doc.footer_lines_even.is_some(),
                word_doc.title_page,
            ),
        };
        let one_based = zero_based_page.saturating_add(1);
        let first_page_of_section = zero_based_page == 0
            || renderer.section_index_for_zero_based_page(zero_based_page - 1) != section_index;
        if title_page && first_page_of_section && first_present {
            HfVariant::First
        } else if word_doc.even_and_odd_headers && one_based.is_multiple_of(2) && even_present {
            HfVariant::Even
        } else {
            HfVariant::Default
        }
    }

    /// Determines which document zone the given canvas coordinates fall into.
    fn hit_test_zone(&self, _x: f64, y: f64) -> EditingMode {
        let renderer = self.renderer.borrow();
        if renderer.pageless_mode() {
            return EditingMode::Body;
        }
        let page_height = renderer.page_height_css();
        let page_gap = 16.0_f64; // PAGE_GAP_PX
        let page_with_gap = page_height + page_gap;
        if page_with_gap <= 0.0 {
            return EditingMode::Body;
        }
        let local_y = y % page_with_gap;
        let page_index = self.page_index_at_y(y);
        let margin_bottom = renderer.margin_bottom();
        let header_boundary = renderer.header_hit_boundary_for_page(page_index);
        let footer_boundary = renderer
            .footer_edit_boundary_for_page(page_index)
            .unwrap_or_else(|| {
                Self::footer_edit_boundary(
                    page_height,
                    margin_bottom,
                    self.hf_line_count(page_index, EditingMode::Footer),
                )
            });
        if local_y < header_boundary {
            EditingMode::Header
        } else if local_y > footer_boundary {
            EditingMode::Footer
        } else {
            EditingMode::Body
        }
    }

    fn clamp_body_position_to_document(&self, position: TextPosition) -> TextPosition {
        self.state
            .borrow()
            .word_doc
            .as_ref()
            .and_then(|document| document.clamp_rendered_text_position(position))
            .unwrap_or(position)
    }

    /// Resolves which header/footer segment and offset the pointer is closest to.
    fn resolve_hf_cursor(
        &self,
        x: f64,
        y: f64,
        mode: EditingMode,
        variant: HfVariant,
    ) -> Option<HfEditCursor> {
        let renderer = self.renderer.borrow();
        let state = self.state.borrow();
        let word_doc = state.word_doc.as_ref()?;
        let lines = Self::variant_lines(word_doc, mode, variant)?;
        let page_height = renderer.page_height_css();
        let page_gap = 16.0_f64;
        let page_with_gap = page_height + page_gap;
        let page_index = if page_with_gap > 0.0 {
            (y / page_with_gap).max(0.0) as usize
        } else {
            0
        };
        if lines.is_empty() {
            return Some(HfEditCursor {
                page_index,
                line_index: 0,
                segment: HfSegment::Left,
                offset: 0,
            });
        }

        if let Some(position) = renderer.hf_story_hit_test(
            page_index,
            mode == EditingMode::Header,
            x,
            y,
            |index, text| {
                lines
                    .get(index)
                    .and_then(Self::hf_single_segment)
                    .is_some_and(|(_, segment_text)| segment_text == text)
            },
        ) {
            let (segment, _) = Self::hf_single_segment(&lines[position.paragraph])?;
            let text = Self::hf_single_segment(&lines[position.paragraph])?.1;
            let metrics = renderer.hf_story_caret_metrics(
                page_index,
                mode == EditingMode::Header,
                position,
                text,
            );
            let margin_left = renderer.margin_left();
            let right = renderer.page_width() - renderer.margin_right();
            let empty_segment_distance = [
                (HfSegment::Left, margin_left),
                (HfSegment::Center, (margin_left + right) / 2.0),
                (HfSegment::Right, right),
            ]
            .into_iter()
            .filter(|(candidate, _)| *candidate != segment)
            .map(|(_, anchor)| (x - anchor).abs())
            .reduce(f64::min)
            .unwrap_or(f64::INFINITY);
            // Preserve clicking an empty alignment segment while resolving
            // clicks nearest the rendered text with its shared glyph layout.
            if metrics.is_some_and(|caret| (x - caret.x).abs() <= empty_segment_distance) {
                return Some(HfEditCursor {
                    page_index,
                    line_index: position.paragraph,
                    segment,
                    offset: position.offset,
                });
            }
        }

        let local_y = if page_with_gap > 0.0 {
            y % page_with_gap
        } else {
            y
        };
        let margin_bottom = renderer.margin_bottom();
        let margin_left = renderer.margin_left();
        let margin_right = renderer.margin_right();
        let page_width = renderer.page_width();

        let line_height = HF_EDIT_LINE_HEIGHT;
        let base_y = match mode {
            EditingMode::Header => renderer.header_edit_content_top(),
            EditingMode::Footer => {
                Self::footer_edit_content_bottom(page_height, margin_bottom)
                    - line_height * lines.len() as f64
            }
            EditingMode::Body => return None,
        };

        // Find closest line by y
        let mut best_line = 0usize;
        let mut best_dist = f64::MAX;
        for (i, _) in lines.iter().enumerate() {
            let line_center_y = base_y + line_height * (i as f64 + 0.5);
            let dist = (local_y - line_center_y).abs();
            if dist < best_dist {
                best_dist = dist;
                best_line = i;
            }
        }

        let line = &lines[best_line];
        if line.left.as_deref().unwrap_or("").is_empty()
            && line.center.as_deref().unwrap_or("").is_empty()
            && line.right.as_deref().unwrap_or("").is_empty()
        {
            return Some(HfEditCursor {
                page_index,
                line_index: best_line,
                segment: Self::hf_single_segment(line)?.0,
                offset: 0,
            });
        }

        let content_width = page_width - margin_left - margin_right;
        let candidates = [
            (
                HfSegment::Left,
                line.left.as_deref().unwrap_or(""),
                margin_left,
            ),
            (
                HfSegment::Center,
                line.center.as_deref().unwrap_or(""),
                margin_left + content_width / 2.0
                    - renderer.measure_header_footer_display_text_width(
                        line.center.as_deref().unwrap_or(""),
                    ) / 2.0,
            ),
            (
                HfSegment::Right,
                line.right.as_deref().unwrap_or(""),
                page_width
                    - margin_right
                    - renderer.measure_header_footer_display_text_width(
                        line.right.as_deref().unwrap_or(""),
                    ),
            ),
        ];
        let (segment, text, segment_start_x) = candidates
            .into_iter()
            .min_by(|(_, text_a, start_a), (_, text_b, start_b)| {
                let end_a = *start_a + renderer.measure_header_footer_display_text_width(text_a);
                let end_b = *start_b + renderer.measure_header_footer_display_text_width(text_b);
                let dist_a = if x < *start_a {
                    *start_a - x
                } else if x > end_a {
                    x - end_a
                } else {
                    0.0
                };
                let dist_b = if x < *start_b {
                    *start_b - x
                } else if x > end_b {
                    x - end_b
                } else {
                    0.0
                };
                dist_a
                    .partial_cmp(&dist_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or((HfSegment::Left, "", margin_left));
        let target_x = (x - segment_start_x).max(0.0);
        let mut offset = 0usize;
        let mut best_dist = f64::MAX;
        for boundary in Self::hf_caret_boundaries(text) {
            let prefix = &text[..boundary];
            let prefix_width = renderer.measure_header_footer_display_text_width(prefix);
            let dist = (prefix_width - target_x).abs();
            if dist < best_dist {
                best_dist = dist;
                offset = boundary;
            }
        }

        Some(HfEditCursor {
            page_index,
            line_index: best_line,
            segment,
            offset,
        })
    }

    /// Triggers a full re-render of the document with current state.
    fn rerender_document(&self) -> Result<(), JsValue> {
        let state_ref = self.state.borrow();
        let word_doc = match state_ref.word_doc.as_ref() {
            Some(wd) => wd,
            None => return Ok(()),
        };
        let document = match word_doc.document.as_ref() {
            Some(d) => Rc::clone(d),
            None => return Ok(()),
        };
        let options = state_ref.options.clone();
        let numbering_part = word_doc.numbering_part.clone();
        let header_lines = word_doc.header_lines.clone();
        let footer_lines = word_doc.footer_lines.clone();
        let header_body =
            word_doc.header_footer_body(HeaderFooterRole::Header, HeaderFooterVariant::Default);
        let footer_body =
            word_doc.header_footer_body(HeaderFooterRole::Footer, HeaderFooterVariant::Default);
        let header_body_first =
            word_doc.header_footer_body(HeaderFooterRole::Header, HeaderFooterVariant::First);
        let footer_body_first =
            word_doc.header_footer_body(HeaderFooterRole::Footer, HeaderFooterVariant::First);
        let header_body_even =
            word_doc.header_footer_body(HeaderFooterRole::Header, HeaderFooterVariant::Even);
        let footer_body_even =
            word_doc.header_footer_body(HeaderFooterRole::Footer, HeaderFooterVariant::Even);
        let comments = word_doc.comments.clone();
        drop(state_ref);

        // Reading the HF variants here is cheap (Option<Vec>) and keeps the
        // renderer's per-page selection in sync with any first/even edits
        // applied since load.
        let (
            header_lines_first,
            footer_lines_first,
            header_lines_even,
            footer_lines_even,
            title_page,
            even_and_odd_headers,
            section_header_footers,
        ) = {
            let state = self.state.borrow();
            if let Some(word_doc) = state.word_doc.as_ref() {
                (
                    word_doc.header_lines_first.clone(),
                    word_doc.footer_lines_first.clone(),
                    word_doc.header_lines_even.clone(),
                    word_doc.footer_lines_even.clone(),
                    word_doc.title_page,
                    word_doc.even_and_odd_headers,
                    word_doc.section_header_footers(),
                )
            } else {
                (None, None, None, None, false, false, Vec::new())
            }
        };

        let mut renderer_mut = self.renderer.borrow_mut();
        renderer_mut.set_header_footer_variants(
            header_lines_first,
            footer_lines_first,
            header_lines_even,
            footer_lines_even,
            title_page,
            even_and_odd_headers,
        );
        renderer_mut.set_header_footer_bodies(
            header_body,
            footer_body,
            header_body_first,
            footer_body_first,
            header_body_even,
            footer_body_even,
        );
        renderer_mut.set_section_header_footers(section_header_footers);
        renderer_mut.render_with_numbering_rc(
            document,
            &options,
            numbering_part.as_ref(),
            header_lines.as_deref(),
            footer_lines.as_deref(),
            Some(&comments),
        )?;
        Ok(())
    }

    /// Returns the segment whose text can map directly to a rich paragraph.
    fn hf_single_segment(line: &HeaderLine) -> Option<(HfSegment, &str)> {
        let present = [
            (HfSegment::Left, line.left.as_deref()),
            (HfSegment::Center, line.center.as_deref()),
            (HfSegment::Right, line.right.as_deref()),
        ];
        let mut segments = present
            .into_iter()
            .filter_map(|(segment, text)| text.map(|text| (segment, text)))
            .filter(|(_, text)| !text.is_empty());
        let segment = segments
            .next()
            .or_else(|| {
                present
                    .into_iter()
                    .find_map(|(segment, text)| text.map(|text| (segment, text)))
            })
            .unwrap_or((HfSegment::Left, ""));
        segments.next().is_none().then_some(segment)
    }

    fn hf_plain_line_segment<'a>(
        word_doc: &WordDocument,
        mode: EditingMode,
        variant: HfVariant,
        line_index: usize,
        line: &'a HeaderLine,
    ) -> Option<(HfSegment, &'a str)> {
        let (segment, text) = Self::hf_single_segment(line)?;
        let (role, hf_variant) = Self::hf_role_variant(mode, variant)?;
        (!text.contains('\t')
            && word_doc
                .hf_plain_paragraph_text(role, hf_variant, line_index)
                .as_deref()
                == Some(text))
        .then_some((segment, text))
    }

    /// Resolve only lines whose UTF-8 offsets address ordinary story paragraphs.
    /// Table rows, fields and tab-separated segments keep the text paste path.
    fn hf_package_target(state: &CanvasDocumentState) -> Option<HfPackageTarget> {
        let word_doc = state.word_doc.as_ref()?;
        let focus = state.hf_edit_cursor?;
        let anchor = state.hf_selection_anchor.unwrap_or(focus);
        Self::hf_package_target_for_selection(
            word_doc,
            state.editing_mode,
            state.hf_variant,
            anchor,
            focus,
        )
    }

    fn hf_package_target_for_selection(
        word_doc: &WordDocument,
        mode: EditingMode,
        hf_variant: HfVariant,
        anchor: HfEditCursor,
        focus: HfEditCursor,
    ) -> Option<HfPackageTarget> {
        if anchor.page_index != focus.page_index {
            return None;
        }
        let (role, variant) = Self::hf_role_variant(mode, hf_variant)?;
        let reference = word_doc.active_hf_reference(role, variant)?;
        let part_path = word_doc.hf_part_path_for_reference(&reference)?;
        let lines = Self::variant_lines(word_doc, mode, hf_variant)?;
        for index in
            anchor.line_index.min(focus.line_index)..=anchor.line_index.max(focus.line_index)
        {
            Self::hf_plain_line_segment(word_doc, mode, hf_variant, index, lines.get(index)?)?;
        }
        for cursor in [anchor, focus] {
            let (segment, text) = Self::hf_plain_line_segment(
                word_doc,
                mode,
                hf_variant,
                cursor.line_index,
                lines.get(cursor.line_index)?,
            )?;
            if segment != cursor.segment || !text.is_char_boundary(cursor.offset) {
                return None;
            }
        }
        Some(HfPackageTarget {
            part_path,
            range: SelectionRange::new(
                TextPosition::new(anchor.line_index, anchor.offset),
                TextPosition::new(focus.line_index, focus.offset),
            ),
        })
    }

    fn hf_segment_text(
        &self,
        mode: EditingMode,
        variant: HfVariant,
        cursor: &HfEditCursor,
    ) -> Option<String> {
        let state = self.state.borrow();
        let word_doc = state.word_doc.as_ref()?;
        let lines = Self::variant_lines(word_doc, mode, variant)?;
        let empty_line = HeaderLine::default();
        let line = match lines.get(cursor.line_index) {
            Some(line) => line,
            None if cursor.line_index == 0 => &empty_line,
            None => return None,
        };
        let text = match cursor.segment {
            HfSegment::Left => line.left.as_deref().unwrap_or(""),
            HfSegment::Center => line.center.as_deref().unwrap_or(""),
            HfSegment::Right => line.right.as_deref().unwrap_or(""),
        };
        Some(text.to_string())
    }

    /// Reconstructs the full paragraph text from a HeaderLine for replacement.
    fn hf_line_full_text(
        &self,
        mode: EditingMode,
        variant: HfVariant,
        line_index: usize,
        segment: HfSegment,
        new_segment_text: &str,
    ) -> Option<String> {
        let state = self.state.borrow();
        let word_doc = state.word_doc.as_ref()?;
        let lines = Self::variant_lines(word_doc, mode, variant)?;
        let empty_line = HeaderLine::default();
        let line = match lines.get(line_index) {
            Some(line) => line,
            None if line_index == 0 => &empty_line,
            None => return None,
        };

        if let Some((single_segment, _)) =
            Self::hf_plain_line_segment(word_doc, mode, variant, line_index, line)
        {
            if single_segment == segment {
                // Paragraph alignment does not imply tab-separated content.
                return Some(new_segment_text.to_string());
            }
        }

        let left = match segment {
            HfSegment::Left => new_segment_text,
            _ => line.left.as_deref().unwrap_or(""),
        };
        let center = match segment {
            HfSegment::Center => new_segment_text,
            _ => line.center.as_deref().unwrap_or(""),
        };
        let right = match segment {
            HfSegment::Right => new_segment_text,
            _ => line.right.as_deref().unwrap_or(""),
        };

        // Reconstruct paragraph text with tab separators
        let mut parts = vec![left.to_string()];
        if !center.is_empty() || !right.is_empty() {
            parts.push(center.to_string());
        }
        if !right.is_empty() {
            parts.push(right.to_string());
        }
        Some(parts.join("\t"))
    }

    fn hf_caret_boundaries(text: &str) -> Vec<usize> {
        let mut boundaries = Vec::new();
        let mut index = 0usize;
        while index < text.len() {
            boundaries.push(index);
            let rest = &text[index..];
            if rest.starts_with(crate::document::PAGE_FIELD_PLACEHOLDER) {
                index += crate::document::PAGE_FIELD_PLACEHOLDER.len();
                continue;
            }
            if rest.starts_with(crate::document::PAGE_COUNT_FIELD_PLACEHOLDER) {
                index += crate::document::PAGE_COUNT_FIELD_PLACEHOLDER.len();
                continue;
            }
            if rest.starts_with(crate::document::SECTION_PAGE_COUNT_FIELD_PLACEHOLDER) {
                index += crate::document::SECTION_PAGE_COUNT_FIELD_PLACEHOLDER.len();
                continue;
            }
            if rest.starts_with(crate::document::SECTION_NUMBER_FIELD_PLACEHOLDER) {
                index += crate::document::SECTION_NUMBER_FIELD_PLACEHOLDER.len();
                continue;
            }
            let Some(grapheme) = rest.graphemes(true).next() else {
                break;
            };
            index += grapheme.len();
        }
        boundaries.push(text.len());
        boundaries.sort_unstable();
        boundaries.dedup();
        boundaries
    }

    fn nearest_hf_caret_boundary(text: &str, target: usize) -> usize {
        Self::hf_caret_boundaries(text)
            .into_iter()
            .min_by_key(|boundary| boundary.abs_diff(target))
            .unwrap_or(0)
    }

    fn hf_editing_mode_name(mode: EditingMode) -> &'static str {
        match mode {
            EditingMode::Header => "header",
            EditingMode::Footer => "footer",
            EditingMode::Body => "body",
        }
    }

    fn hf_segment_for_alignment(alignment: &str) -> HfSegment {
        match alignment.trim() {
            "center" => HfSegment::Center,
            "right" | "end" => HfSegment::Right,
            _ => HfSegment::Left,
        }
    }

    fn parse_hf_role(role: &str) -> Result<EditingMode, JsValue> {
        match role.trim() {
            "header" => Ok(EditingMode::Header),
            "footer" => Ok(EditingMode::Footer),
            _ => Err(JsValue::from_str(
                "header/footer role must be `header` or `footer`",
            )),
        }
    }

    fn parse_hf_alignment(alignment: &str) -> Result<(HfSegment, HeaderFooterAlignment), JsValue> {
        match alignment.trim() {
            "left" => Ok((HfSegment::Left, HeaderFooterAlignment::Left)),
            "center" => Ok((HfSegment::Center, HeaderFooterAlignment::Center)),
            "right" => Ok((HfSegment::Right, HeaderFooterAlignment::Right)),
            _ => Err(JsValue::from_str(
                "header/footer alignment must be `left`, `center`, or `right`",
            )),
        }
    }

    fn validate_hf_page_index(&self, page_index: usize) -> Result<(), JsValue> {
        let page_count = self.renderer.borrow().page_count();
        if page_count == 0 {
            return Err(JsValue::from_str("document has no rendered pages"));
        }
        if page_index >= page_count {
            return Err(JsValue::from_str(&format!(
                "page index {page_index} is out of range for {page_count} rendered pages"
            )));
        }
        Ok(())
    }

    fn hf_cursor_at_segment_end(
        &self,
        page_index: usize,
        mode: EditingMode,
        variant: HfVariant,
        segment: HfSegment,
    ) -> HfEditCursor {
        let probe = HfEditCursor {
            page_index,
            line_index: 0,
            segment,
            offset: 0,
        };
        let offset = self
            .hf_segment_text(mode, variant, &probe)
            .map(|text| text.len())
            .unwrap_or(0);
        HfEditCursor { offset, ..probe }
    }

    /// The header/footer selection as `(start, end)` cursors on one segment,
    /// or `None` when the anchor is unset, collapsed, or on another segment.
    fn hf_selection(&self) -> Option<(HfEditCursor, HfEditCursor)> {
        let state = self.state.borrow();
        let cursor = state.hf_edit_cursor?;
        let anchor = state.hf_selection_anchor?;
        if anchor.page_index != cursor.page_index
            || anchor.line_index != cursor.line_index
            || anchor.segment != cursor.segment
            || anchor.offset == cursor.offset
        {
            return None;
        }
        Some(if anchor.offset < cursor.offset {
            (anchor, cursor)
        } else {
            (cursor, anchor)
        })
    }

    /// Selection rectangles of the header/footer selection, page-local like
    /// the body's, for the runtime's selection overlay.
    fn hf_selection_rects(&self) -> Vec<(f64, f64, f64, f64, usize)> {
        let Some((start, end)) = self.hf_selection() else {
            return Vec::new();
        };
        let renderer = self.renderer.borrow();
        let state = self.state.borrow();
        let mode = state.editing_mode;
        let line = state
            .word_doc
            .as_ref()
            .and_then(|document| Self::variant_lines(document, mode, state.hf_variant))
            .and_then(|lines| lines.get(start.line_index).cloned());
        drop(state);
        if let Some((segment, text)) = line.as_ref().and_then(Self::hf_single_segment) {
            if segment == start.segment {
                if let Some(rects) = renderer.hf_story_selection_rects(
                    start.page_index,
                    mode == EditingMode::Header,
                    SelectionRange {
                        anchor: TextPosition::new(start.line_index, start.offset),
                        focus: TextPosition::new(end.line_index, end.offset),
                    },
                    text,
                ) {
                    return rects
                        .into_iter()
                        .map(|rect| (rect.x, rect.y, rect.width, rect.height, rect.page))
                        .collect();
                }
            }
        }
        let (Some(start_rect), Some(end_rect)) = (
            self.compute_hf_caret_rect(&renderer, &start),
            self.compute_hf_caret_rect(&renderer, &end),
        ) else {
            return Vec::new();
        };
        let page_gap = 16.0_f64;
        let page_offset = start.page_index as f64 * (renderer.page_height_css() + page_gap);
        let x = start_rect.0.min(end_rect.0);
        let width = (end_rect.0 - start_rect.0).abs();
        vec![(
            x,
            start_rect.1 - page_offset,
            width,
            start_rect.2,
            start.page_index,
        )]
    }

    fn write_hf_selection_rects(&self, result: &Object) -> Result<(), JsValue> {
        let rects = Array::new();
        for (x, y, width, height, page) in self.hf_selection_rects() {
            let obj = Object::new();
            Reflect::set(&obj, &JsValue::from_str("x"), &JsValue::from_f64(x))?;
            Reflect::set(&obj, &JsValue::from_str("y"), &JsValue::from_f64(y))?;
            Reflect::set(&obj, &JsValue::from_str("width"), &JsValue::from_f64(width))?;
            Reflect::set(
                &obj,
                &JsValue::from_str("height"),
                &JsValue::from_f64(height),
            )?;
            Reflect::set(
                &obj,
                &JsValue::from_str("page"),
                &JsValue::from_f64(page as f64),
            )?;
            rects.push(&obj);
        }
        Reflect::set(result, &JsValue::from_str("selectionRects"), &rects)?;
        Ok(())
    }

    /// Deletes the header/footer selection, if any, leaving the caret at its
    /// start. The caller records the complete edit as one history entry.
    /// Returns whether text was removed.
    fn delete_hf_selection(&self, mode: EditingMode) -> Result<bool, JsValue> {
        let Some((start, end)) = self.hf_selection() else {
            return Ok(false);
        };
        let variant = self.state.borrow().hf_variant;
        let current_text = self
            .hf_segment_text(mode, variant, &start)
            .unwrap_or_default();
        let (from, to) = (
            start.offset.min(current_text.len()),
            end.offset.min(current_text.len()),
        );
        if from >= to {
            self.state.borrow_mut().hf_selection_anchor = None;
            return Ok(false);
        }
        let mut new_text = current_text.clone();
        new_text.replace_range(from..to, "");
        let Some(full_text) =
            self.hf_line_full_text(mode, variant, start.line_index, start.segment, &new_text)
        else {
            return Ok(false);
        };
        {
            let mut state_mut = self.state.borrow_mut();
            if let Some(word_doc) = state_mut.word_doc.as_mut() {
                Self::replace_hf_line(word_doc, mode, variant, start.line_index, &full_text)
                    .map_err(|err| JsValue::from_str(&err))?;
            }
            state_mut.hf_edit_cursor = Some(HfEditCursor {
                offset: from,
                ..start
            });
            state_mut.hf_selection_anchor = None;
        }
        Ok(true)
    }

    /// Byte range of the header/footer selection in its paragraph's text,
    /// including tab prefixes only for tab-separated segments.
    fn hf_selection_paragraph_range(&self, mode: EditingMode) -> Option<(usize, usize, usize)> {
        let (start, end) = self.hf_selection()?;
        let state = self.state.borrow();
        let word_doc = state.word_doc.as_ref()?;
        let variant = state.hf_variant;
        let lines = Self::variant_lines(word_doc, mode, variant)?;
        let line = lines.get(start.line_index)?;
        if Self::hf_plain_line_segment(word_doc, mode, variant, start.line_index, line)
            .is_some_and(|(segment, _)| segment == start.segment)
        {
            return Some((start.line_index, start.offset, end.offset));
        }
        let left = line.left.as_deref().unwrap_or("");
        let center = line.center.as_deref().unwrap_or("");
        let prefix = match start.segment {
            HfSegment::Left => 0,
            HfSegment::Center => left.len() + 1,
            HfSegment::Right => left.len() + 1 + center.len() + 1,
        };
        Some((start.line_index, prefix + start.offset, prefix + end.offset))
    }

    fn write_hf_caret_to_result(&self, result: &Object) -> Result<(), JsValue> {
        self.write_hf_selection_rects(result)?;
        let Some(cursor) = self.state.borrow().hf_edit_cursor else {
            return Ok(());
        };
        let renderer = self.renderer.borrow();
        if let Some(caret_rect) = self.compute_hf_caret_rect(&renderer, &cursor) {
            let caret_obj = Object::new();
            Reflect::set(
                &caret_obj,
                &JsValue::from_str("x"),
                &JsValue::from_f64(caret_rect.0),
            )?;
            Reflect::set(
                &caret_obj,
                &JsValue::from_str("y"),
                &JsValue::from_f64(caret_rect.1),
            )?;
            Reflect::set(
                &caret_obj,
                &JsValue::from_str("height"),
                &JsValue::from_f64(caret_rect.2),
            )?;
            Reflect::set(result, &JsValue::from_str("hfCaret"), &caret_obj)?;
        }
        Ok(())
    }

    fn push_hf_history(
        &self,
        before_doc: WordDocument,
        after_doc: WordDocument,
        before_cursor: Option<HfEditCursor>,
        after_cursor: Option<HfEditCursor>,
        mode: EditingMode,
        variant: HfVariant,
    ) {
        self.push_hf_history_entry(HfHistoryEntry {
            before_doc,
            after_doc,
            before_cursor,
            after_cursor,
            before_selection_anchor: None,
            after_selection_anchor: None,
            mode,
            variant,
        });
    }

    fn push_hf_history_entry(&self, entry: HfHistoryEntry) {
        let mut state = self.state.borrow_mut();
        if state.hf_provisional_recording {
            return;
        }
        state.hf_history_revision = state.hf_history_revision.wrapping_add(1);
        state.hf_undo_stack.push(entry);
        // Each entry retains TWO full `WordDocument` clones (before + after), so
        // an unbounded stack grows memory steeply over a long header/footer
        // editing session — especially on large documents. Bound it to a small
        // number of recent steps, dropping the oldest. (The deeper fix is to stop
        // snapshotting whole documents for header/footer undo; this cap keeps
        // memory bounded in the meantime.)
        if state.hf_undo_stack.len() > MAX_HF_UNDO_HISTORY {
            let overflow = state.hf_undo_stack.len() - MAX_HF_UNDO_HISTORY;
            state.hf_undo_stack.drain(0..overflow);
        }
        state.hf_redo_stack.clear();
    }

    async fn begin_hf_editing_async(
        &self,
        role: String,
        page_index: usize,
        alignment: String,
    ) -> Result<JsValue, JsValue> {
        let mode = Self::parse_hf_role(&role)?;
        let (segment, _) = Self::parse_hf_alignment(&alignment)?;
        self.validate_hf_page_index(page_index)?;
        let variant = self.variant_for_page_index(page_index, mode == EditingMode::Header);
        let (hf_role, hf_variant) = Self::hf_role_variant(mode, variant)
            .ok_or_else(|| JsValue::from_str("body is not a header/footer editing role"))?;

        // Edits target the parts of the section that owns the clicked page
        // (with Word's link-to-previous inheritance), so a header edited on
        // a section-2 page changes section 2's story, not the document-wide
        // default.
        let edit_section = self
            .renderer
            .borrow()
            .section_index_for_zero_based_page(page_index);
        let (before_doc, before_cursor, mut updated_doc, created_part) = {
            let state = self.state.borrow();
            let mut before_doc = state
                .word_doc
                .clone()
                .ok_or_else(|| JsValue::from_str("document is not loaded"))?;
            before_doc.hf_edit_section = Some(edit_section);
            let created_part = before_doc
                .active_hf_reference(hf_role, hf_variant)
                .is_none();
            (
                before_doc.clone(),
                state.hf_edit_cursor,
                before_doc,
                created_part,
            )
        };
        updated_doc
            .ensure_hf_part_for_edit(hf_role, hf_variant)
            .map_err(|error| JsValue::from_str(&error))?;
        {
            let mut state = self.state.borrow_mut();
            state.word_doc = Some(updated_doc.clone());
            state.enter_hf_editing_mode(mode);
            state.hf_variant = variant;
            state.selection = None;
            state.object_selection = None;
            state.caret_goal_x = None;
        }
        let cursor = self.hf_cursor_at_segment_end(page_index, mode, variant, segment);
        self.state.borrow_mut().hf_edit_cursor = Some(cursor);
        self.state.borrow_mut().hf_selection_anchor = None;
        self.renderer.borrow_mut().set_selection_range(None);
        if created_part {
            self.push_hf_history(
                before_doc,
                updated_doc,
                before_cursor,
                Some(cursor),
                mode,
                variant,
            );
        }
        self.prepare_hf_edit_rerender(mode, page_index);
        self.finish_hf_edit_result_async(mode).await
    }

    async fn insert_page_number_async(
        &self,
        role: String,
        page_index: usize,
        alignment: String,
    ) -> Result<JsValue, JsValue> {
        let mode = Self::parse_hf_role(&role)?;
        let (segment, hf_alignment) = Self::parse_hf_alignment(&alignment)?;
        self.validate_hf_page_index(page_index)?;
        let variant = self.variant_for_page_index(page_index, mode == EditingMode::Header);
        let (hf_role, hf_variant) = Self::hf_role_variant(mode, variant)
            .ok_or_else(|| JsValue::from_str("body is not a header/footer editing role"))?;
        let edit_section = self
            .renderer
            .borrow()
            .section_index_for_zero_based_page(page_index);
        let (before_doc, before_cursor, mut updated_doc) = {
            let state = self.state.borrow();
            let mut before_doc = state
                .word_doc
                .clone()
                .ok_or_else(|| JsValue::from_str("document is not loaded"))?;
            before_doc.hf_edit_section = Some(edit_section);
            (before_doc.clone(), state.hf_edit_cursor, before_doc)
        };
        updated_doc
            .set_hf_page_number(hf_role, hf_variant, hf_alignment)
            .map_err(|error| JsValue::from_str(&error))?;
        {
            let mut state = self.state.borrow_mut();
            state.word_doc = Some(updated_doc.clone());
            state.enter_hf_editing_mode(mode);
            state.hf_variant = variant;
            state.selection = None;
            state.object_selection = None;
            state.caret_goal_x = None;
        }
        let cursor = self.hf_cursor_at_segment_end(page_index, mode, variant, segment);
        self.state.borrow_mut().hf_edit_cursor = Some(cursor);
        self.state.borrow_mut().hf_selection_anchor = None;
        self.renderer.borrow_mut().set_selection_range(None);
        self.push_hf_history(
            before_doc,
            updated_doc,
            before_cursor,
            Some(cursor),
            mode,
            variant,
        );
        self.prepare_hf_edit_rerender(mode, page_index);
        self.finish_hf_edit_result_async(mode).await
    }

    async fn set_page_number_format_async(
        &self,
        format: String,
        start: Option<u32>,
    ) -> Result<JsValue, JsValue> {
        // Word's ページ番号の書式設定 writes `w:pgNumType` into the section the
        // caret (or the footer being edited) belongs to, so front matter can
        // number in roman numerals while the body restarts at 1.
        let section = self.ribbon_target_section();
        let (mode, variant, cursor, before_doc) = {
            let state = self.state.borrow();
            (
                state.editing_mode,
                state.hf_variant,
                state.hf_edit_cursor,
                state
                    .word_doc
                    .clone()
                    .ok_or_else(|| JsValue::from_str("document is not loaded"))?,
            )
        };
        let page_index = cursor.map(|cursor| cursor.page_index).unwrap_or(0);
        if mode == EditingMode::Body {
            let mut state = self.state.borrow_mut();
            let mut document = state.word_doc.take().unwrap();
            let mut executor = std::mem::take(&mut state.command_executor);
            let mut context = CommandContext::new(&mut document, &mut state.selection);
            let result = executor.execute(
                Box::new(
                    crate::commands::set_page_number_format::SetPageNumberFormatCommand::new(
                        format, start, section,
                    ),
                ),
                &mut context,
            );
            state.command_executor = executor;
            state.word_doc = Some(document);
            result.map_err(|error| JsValue::from_str(&error.to_string()))?;
        } else {
            let mut updated_doc = before_doc.clone();
            let changed = updated_doc
                .set_page_number_format(&format, start, section)
                .map_err(|error| JsValue::from_str(&error))?;
            self.state.borrow_mut().word_doc = Some(updated_doc.clone());
            if changed {
                self.push_hf_history(before_doc, updated_doc, cursor, cursor, mode, variant);
            }
        }
        self.finish_hf_package_result_async(mode, page_index).await
    }

    async fn set_note_numbering_async(
        &self,
        is_footnote: bool,
        format: String,
        start: u32,
        restart: String,
        section: Option<usize>,
    ) -> Result<JsValue, JsValue> {
        let section = section.or_else(|| self.ribbon_target_section());
        let (mode, variant, cursor, before_doc) = {
            let state = self.state.borrow();
            (
                state.editing_mode,
                state.hf_variant,
                state.hf_edit_cursor,
                state
                    .word_doc
                    .clone()
                    .ok_or_else(|| JsValue::from_str("document is not loaded"))?,
            )
        };
        let page_index = cursor.map(|cursor| cursor.page_index).unwrap_or(0);
        if mode == EditingMode::Body {
            let selection = self.renderer.borrow().selection_range();
            let mut state = self.state.borrow_mut();
            state.selection = selection;
            state.caret_goal_x = None;
            let mut document = state.word_doc.take().unwrap();
            let mut executor = std::mem::take(&mut state.command_executor);
            let mut context = CommandContext::new(&mut document, &mut state.selection);
            let result = executor.execute(
                Box::new(
                    crate::commands::set_note_numbering::SetNoteNumberingCommand::new(
                        is_footnote,
                        format,
                        start,
                        restart,
                        section,
                    ),
                ),
                &mut context,
            );
            state.command_executor = executor;
            state.word_doc = Some(document);
            result.map_err(|error| JsValue::from_str(&error.to_string()))?;
        } else {
            let mut updated_doc = before_doc.clone();
            let changed = updated_doc
                .set_note_numbering(is_footnote, &format, start, &restart, section)
                .map_err(|error| JsValue::from_str(&error))?;
            self.state.borrow_mut().word_doc = Some(updated_doc.clone());
            if changed {
                self.push_hf_history(before_doc, updated_doc, cursor, cursor, mode, variant);
            }
        }
        let result = self
            .finish_hf_package_result_async(mode, page_index)
            .await?;
        if mode == EditingMode::Body {
            let (selection, preview_mode) = {
                let state = self.state.borrow();
                (state.selection, state.preview_mode)
            };
            let mut renderer = self.renderer.borrow_mut();
            renderer.set_selection_range(selection);
            let caret = if preview_mode {
                JsValue::NULL
            } else {
                renderer
                    .caret_metrics()
                    .and_then(|metrics| to_value(&metrics).ok())
                    .unwrap_or(JsValue::NULL)
            };
            let result = result.dyn_into::<Object>()?;
            Reflect::set(&result, &JsValue::from_str("caret"), &caret)?;
            Self::write_selection_rects(&result, &renderer, selection)?;
            return Ok(result.into());
        }
        Ok(result)
    }

    async fn set_endnote_position_async(
        &self,
        position: crate::document::EndnotePosition,
        section: Option<usize>,
    ) -> Result<JsValue, JsValue> {
        let section = section.or_else(|| self.ribbon_target_section());
        let (mode, variant, cursor, before_doc) = {
            let state = self.state.borrow();
            (
                state.editing_mode,
                state.hf_variant,
                state.hf_edit_cursor,
                state
                    .word_doc
                    .clone()
                    .ok_or_else(|| JsValue::from_str("document is not loaded"))?,
            )
        };
        let page_index = cursor.map(|cursor| cursor.page_index).unwrap_or(0);
        if mode == EditingMode::Body {
            let selection = self.renderer.borrow().selection_range();
            let mut state = self.state.borrow_mut();
            state.selection = selection;
            state.caret_goal_x = None;
            let mut document = state.word_doc.take().unwrap();
            let mut executor = std::mem::take(&mut state.command_executor);
            let mut context = CommandContext::new(&mut document, &mut state.selection);
            let result = executor.execute(
                Box::new(
                    crate::commands::set_endnote_position::SetEndnotePositionCommand::new(
                        position, section,
                    ),
                ),
                &mut context,
            );
            state.command_executor = executor;
            state.word_doc = Some(document);
            result.map_err(|error| JsValue::from_str(&error.to_string()))?;
        } else {
            let mut updated_doc = before_doc.clone();
            let changed = updated_doc
                .set_endnote_position(position, section)
                .map_err(|error| JsValue::from_str(&error))?;
            self.state.borrow_mut().word_doc = Some(updated_doc.clone());
            if changed {
                self.push_hf_history(before_doc, updated_doc, cursor, cursor, mode, variant);
            }
        }
        let result = self
            .finish_hf_package_result_async(mode, page_index)
            .await?;
        if mode == EditingMode::Body {
            let (selection, preview_mode) = {
                let state = self.state.borrow();
                (state.selection, state.preview_mode)
            };
            let mut renderer = self.renderer.borrow_mut();
            renderer.set_selection_range(selection);
            let caret = if preview_mode {
                JsValue::NULL
            } else {
                renderer
                    .caret_metrics()
                    .and_then(|metrics| to_value(&metrics).ok())
                    .unwrap_or(JsValue::NULL)
            };
            let result = result.dyn_into::<Object>()?;
            Reflect::set(&result, &JsValue::from_str("caret"), &caret)?;
            Self::write_selection_rects(&result, &renderer, selection)?;
            return Ok(result.into());
        }
        Ok(result)
    }

    async fn finish_hf_package_result_async(
        &self,
        mode: EditingMode,
        page_index: usize,
    ) -> Result<JsValue, JsValue> {
        self.prepare_hf_edit_rerender(mode, page_index);
        if mode != EditingMode::Body && self.state.borrow().hf_edit_cursor.is_some() {
            return self.finish_hf_edit_result_async(mode).await;
        }
        self.rerender_document()?;
        let snapshots = self.capture_pages_after_render().await?;
        self.store_paged_images(snapshots);
        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &self.canvas_size()?)?;
        Reflect::set(&result, &JsValue::from_str("pages"), &self.paged_images())?;
        Reflect::set(
            &result,
            &JsValue::from_str("debugMutationKind"),
            &JsValue::from_str("header-footer-edit"),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("editingMode"),
            &JsValue::from_str(Self::hf_editing_mode_name(mode)),
        )?;
        Self::write_history_flags(&result, self.history_flags())?;
        Ok(result.into())
    }

    async fn remove_page_numbers_async(&self) -> Result<JsValue, JsValue> {
        let (before_doc, before_cursor, mode, variant, page_index, mut updated_doc) = {
            let state = self.state.borrow();
            let before_doc = state
                .word_doc
                .clone()
                .ok_or_else(|| JsValue::from_str("document is not loaded"))?;
            (
                before_doc.clone(),
                state.hf_edit_cursor,
                state.editing_mode,
                state.hf_variant,
                state
                    .hf_edit_cursor
                    .map(|cursor| cursor.page_index)
                    .unwrap_or(0),
                before_doc,
            )
        };
        let removed = updated_doc
            .remove_page_numbers_from_header_footers()
            .map_err(|error| JsValue::from_str(&error))?;
        if removed == 0 {
            return self.finish_hf_package_result_async(mode, page_index).await;
        }
        {
            let mut state = self.state.borrow_mut();
            state.word_doc = Some(updated_doc.clone());
            if let Some(cursor) = state.hf_edit_cursor.as_mut() {
                cursor.offset = 0;
            }
        }
        let after_cursor = self.state.borrow().hf_edit_cursor;
        self.push_hf_history(
            before_doc,
            updated_doc,
            before_cursor,
            after_cursor,
            mode,
            variant,
        );
        self.finish_hf_package_result_async(mode, page_index).await
    }

    async fn perform_hf_history_action_async(
        &self,
        action: HistoryAction,
    ) -> Result<Option<JsValue>, JsValue> {
        let should_try_hf = {
            let state = self.state.borrow();
            let has_hf_history = match action {
                HistoryAction::Undo => !state.hf_undo_stack.is_empty(),
                HistoryAction::Redo => !state.hf_redo_stack.is_empty(),
            };
            let has_body_history = match action {
                HistoryAction::Undo => state.command_executor.can_undo(),
                HistoryAction::Redo => state.command_executor.can_redo(),
            };
            has_hf_history && (state.editing_mode != EditingMode::Body || !has_body_history)
        };
        if !should_try_hf {
            return Ok(None);
        }

        let Some(mode) = ({
            let mut state = self.state.borrow_mut();
            let Some(entry) = (match action {
                HistoryAction::Undo => state.hf_undo_stack.pop(),
                HistoryAction::Redo => state.hf_redo_stack.pop(),
            }) else {
                return Ok(None);
            };
            let (word_doc, cursor, selection_anchor) = match action {
                HistoryAction::Undo => (
                    entry.before_doc.clone(),
                    entry.before_cursor,
                    entry.before_selection_anchor,
                ),
                HistoryAction::Redo => (
                    entry.after_doc.clone(),
                    entry.after_cursor,
                    entry.after_selection_anchor,
                ),
            };
            state.word_doc = Some(word_doc);
            state.hf_edit_cursor = cursor;
            state.hf_selection_anchor = selection_anchor;
            state.enter_hf_editing_mode(entry.mode);
            state.hf_variant = entry.variant;
            state.selection = None;
            state.object_selection = None;
            state.caret_goal_x = None;
            state.paged_images.clear();
            let mode = entry.mode;
            match action {
                HistoryAction::Undo => state.hf_redo_stack.push(entry),
                HistoryAction::Redo => state.hf_undo_stack.push(entry),
            }
            Some(mode)
        }) else {
            return Ok(None);
        };

        let anchor_page_index = {
            let mut renderer = self.renderer.borrow_mut();
            renderer.set_selection_range(None);
            renderer.clear_object_transform_preview();
            self.state
                .borrow()
                .hf_edit_cursor
                .map(|cursor| cursor.page_index)
                .unwrap_or(0)
        };
        self.prepare_hf_edit_rerender(mode, anchor_page_index);
        self.finish_hf_edit_result_async(mode).await.map(Some)
    }

    /// Replace the selection and apply an insertion as one history/paint update.
    fn apply_hf_insertion(
        &self,
        mode: EditingMode,
        edit: impl FnOnce() -> Result<Option<HfEditCursor>, JsValue>,
    ) -> Result<Option<HfEditCursor>, JsValue> {
        let (before_doc, before_cursor, before_selection_anchor, variant) = {
            let state = self.state.borrow();
            let (Some(word_doc), Some(cursor)) = (&state.word_doc, state.hf_edit_cursor) else {
                return Ok(None);
            };
            (
                word_doc.clone(),
                cursor,
                state.hf_selection_anchor,
                state.hf_variant,
            )
        };
        let result = self.delete_hf_selection(mode).and_then(|_| edit());
        let cursor = match result {
            Ok(Some(cursor)) => cursor,
            other => {
                // A failed batch must not leave a partially pasted story.
                let mut state = self.state.borrow_mut();
                state.word_doc = Some(before_doc);
                state.hf_edit_cursor = Some(before_cursor);
                state.hf_selection_anchor = before_selection_anchor;
                return other;
            }
        };
        let after_doc = self.state.borrow().word_doc.clone();
        if let Some(after_doc) = after_doc {
            self.push_hf_history_entry(HfHistoryEntry {
                before_doc,
                after_doc,
                before_cursor: Some(before_cursor),
                after_cursor: Some(cursor),
                before_selection_anchor,
                after_selection_anchor: None,
                mode,
                variant,
            });
        }
        self.prepare_hf_edit_rerender(mode, cursor.page_index);
        Ok(Some(cursor))
    }

    async fn apply_hf_package_edit(
        &self,
        bytes: Vec<u8>,
        expected: Vec<u8>,
        part_path: String,
        caret: TextPosition,
    ) -> Result<JsValue, JsValue> {
        let (mode, variant, before_cursor, before_anchor, target, options) = {
            let state = self.state.borrow();
            let target = Self::hf_package_target(&state)
                .filter(|target| target.part_path == part_path)
                .ok_or_else(|| JsValue::from_str("The story paste destination is unavailable"))?;
            if state.preview_mode || state.options.record_changes() {
                return Err(JsValue::from_str(
                    "Package edits require an editable untracked story",
                ));
            }
            (
                state.editing_mode,
                state.hf_variant,
                state.hf_edit_cursor,
                state.hf_selection_anchor,
                target,
                state.options.clone(),
            )
        };
        let replacement = WordDocument::load(&bytes, &options)
            .await
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let snapshot = replacement
            .snapshot_for_replacement()
            .ok_or_else(|| JsValue::from_str("Document is unavailable"))?;
        let (before_doc, after_doc, after_cursor) = {
            let mut state = self.state.borrow_mut();
            // Loading images yields to other messages. Recheck both the story
            // destination and the entire source before changing document/history.
            if state.preview_mode
                || state.options.record_changes()
                || state.editing_mode != mode
                || state.hf_variant != variant
                || state.hf_edit_cursor != before_cursor
                || state.hf_selection_anchor != before_anchor
                || Self::hf_package_target(&state).as_ref() != Some(&target)
            {
                return Err(JsValue::from_str(
                    "The story paste destination changed; try again",
                ));
            }
            let before_doc = state
                .word_doc
                .as_ref()
                .ok_or_else(|| JsValue::from_str("Document is unavailable"))?;
            if before_doc
                .export_docx()
                .map_err(|error| JsValue::from_str(&error.to_string()))?
                != expected
            {
                return Err(JsValue::from_str(
                    "The document changed while preparing the edit; try again",
                ));
            }
            let before_doc = before_doc.clone();
            let mut after_doc = before_doc.clone();
            after_doc.restore_snapshot(snapshot);
            let (role, hf_variant) = Self::hf_role_variant(mode, variant).unwrap();
            let reference = after_doc.active_hf_reference(role, hf_variant);
            if reference
                .and_then(|id| after_doc.hf_part_path_for_reference(&id))
                .as_deref()
                != Some(part_path.as_str())
            {
                return Err(JsValue::from_str(
                    "The replacement changed the story destination",
                ));
            }
            after_doc
                .hf_ruler_paragraph(role, hf_variant, caret.paragraph)
                .ok_or_else(|| {
                    JsValue::from_str("The pasted caret is not an ordinary story paragraph")
                })?;
            let lines = Self::variant_lines(&after_doc, mode, variant)
                .ok_or_else(|| JsValue::from_str("The pasted story is unavailable"))?;
            let (segment, text) = lines
                .get(caret.paragraph)
                .and_then(Self::hf_single_segment)
                .ok_or_else(|| {
                    JsValue::from_str("The pasted caret cannot address a story segment")
                })?;
            if !text.is_char_boundary(caret.offset) {
                return Err(JsValue::from_str(
                    "The pasted caret is outside the story text",
                ));
            }
            let after_cursor = HfEditCursor {
                line_index: caret.paragraph,
                segment,
                offset: caret.offset,
                ..before_cursor.unwrap()
            };
            state.word_doc = Some(after_doc.clone());
            state.hf_edit_cursor = Some(after_cursor);
            state.hf_selection_anchor = None;
            state.selection = None;
            state.object_selection = None;
            (before_doc, after_doc, after_cursor)
        };
        self.push_hf_history_entry(HfHistoryEntry {
            before_doc,
            after_doc,
            before_cursor,
            after_cursor: Some(after_cursor),
            before_selection_anchor: before_anchor,
            after_selection_anchor: None,
            mode,
            variant,
        });
        // Package imports can change shared styles and numbering used by body pages.
        self.renderer.borrow_mut().invalidate_layout_from(0);
        self.renderer.borrow_mut().clear_paragraph_render_cache();
        self.prepare_hf_edit_rerender(mode, after_cursor.page_index);
        self.finish_hf_edit_result_async(mode).await
    }

    async fn insert_hf_text(&self, text: String, mode: EditingMode) -> Result<JsValue, JsValue> {
        if self
            .apply_hf_insertion(mode, || self.insert_hf_text_at_caret(&text, mode))?
            .is_none()
        {
            return Ok(JsValue::NULL);
        }
        self.finish_hf_edit_result_async(mode).await
    }

    fn insert_hf_text_at_caret(
        &self,
        text: &str,
        mode: EditingMode,
    ) -> Result<Option<HfEditCursor>, JsValue> {
        let (cursor, variant) = {
            let state = self.state.borrow();
            match state.hf_edit_cursor {
                Some(c) => (c, state.hf_variant),
                None => return Ok(None),
            }
        };
        let current_text = self
            .hf_segment_text(mode, variant, &cursor)
            .unwrap_or_default();

        let offset = cursor.offset.min(current_text.len());
        let mut new_text = current_text.clone();
        new_text.insert_str(offset, text);

        let full_text = match self.hf_line_full_text(
            mode,
            variant,
            cursor.line_index,
            cursor.segment,
            &new_text,
        ) {
            Some(t) => t,
            None => return Ok(None),
        };

        // Apply the edit
        let after_cursor = {
            let mut state_mut = self.state.borrow_mut();
            let Some(word_doc) = state_mut.word_doc.as_mut() else {
                return Ok(None);
            };
            Self::replace_hf_line(word_doc, mode, variant, cursor.line_index, &full_text)
                .map_err(|err| JsValue::from_str(&err))?;
            // Update cursor offset
            state_mut.hf_edit_cursor = Some(HfEditCursor {
                page_index: cursor.page_index,
                line_index: cursor.line_index,
                segment: cursor.segment,
                offset: offset + text.len(),
            });
            state_mut.hf_selection_anchor = None;
            state_mut.hf_edit_cursor
        };
        Ok(after_cursor)
    }

    async fn insert_hf_paragraph_breaks(
        &self,
        count: usize,
        mode: EditingMode,
    ) -> Result<JsValue, JsValue> {
        if self
            .apply_hf_insertion(mode, || self.insert_hf_breaks_at_caret(count, mode))?
            .is_none()
        {
            return Ok(JsValue::NULL);
        }
        self.finish_hf_edit_result_async(mode).await
    }

    fn insert_hf_breaks_at_caret(
        &self,
        count: usize,
        mode: EditingMode,
    ) -> Result<Option<HfEditCursor>, JsValue> {
        let break_count = count.max(1);
        let (cursor, variant) = {
            let state = self.state.borrow();
            match state.hf_edit_cursor {
                Some(c) => (c, state.hf_variant),
                None => return Ok(None),
            }
        };
        let Some((role, hf_variant)) = Self::hf_role_variant(mode, variant) else {
            return Ok(None);
        };
        let current_text = self
            .hf_segment_text(mode, variant, &cursor)
            .unwrap_or_default();
        let split_offset =
            Self::nearest_hf_caret_boundary(&current_text, cursor.offset.min(current_text.len()));
        let before_segment_text = current_text[..split_offset].to_string();
        let after_segment_text = current_text[split_offset..].to_string();
        let before_full_text = match self.hf_line_full_text(
            mode,
            variant,
            cursor.line_index,
            cursor.segment,
            &before_segment_text,
        ) {
            Some(text) => text,
            None => return Ok(None),
        };
        let after_full_text = match self.hf_line_full_text(
            mode,
            variant,
            cursor.line_index,
            cursor.segment,
            &after_segment_text,
        ) {
            Some(text) => text,
            None => return Ok(None),
        };
        let after_cursor = {
            let mut state_mut = self.state.borrow_mut();
            let Some(word_doc) = state_mut.word_doc.as_mut() else {
                return Ok(None);
            };
            // Split ordinary story XML directly so each side retains its runs.
            let split_plain_paragraph =
                Self::variant_lines(word_doc, mode, variant).is_some_and(|lines| {
                    let Some((segment, text)) = lines.get(cursor.line_index).and_then(|line| {
                        Self::hf_plain_line_segment(
                            word_doc,
                            mode,
                            variant,
                            cursor.line_index,
                            line,
                        )
                    }) else {
                        return false;
                    };
                    segment == cursor.segment && text == current_text
                });
            if !split_plain_paragraph {
                Self::replace_hf_line(
                    word_doc,
                    mode,
                    variant,
                    cursor.line_index,
                    &before_full_text,
                )
                .map_err(|err| JsValue::from_str(&err))?;
            }
            word_doc
                .split_hf_paragraph_at_display_offset(
                    role,
                    hf_variant,
                    cursor.line_index,
                    if split_plain_paragraph {
                        before_segment_text.chars().count()
                    } else {
                        before_full_text.chars().count()
                    },
                    break_count,
                )
                .map_err(|err| JsValue::from_str(&err))?;
            if !split_plain_paragraph {
                Self::replace_hf_line(
                    word_doc,
                    mode,
                    variant,
                    cursor.line_index + break_count,
                    &after_full_text,
                )
                .map_err(|err| JsValue::from_str(&err))?;
            }
            state_mut.hf_edit_cursor = Some(HfEditCursor {
                page_index: cursor.page_index,
                line_index: cursor.line_index + break_count,
                segment: cursor.segment,
                offset: 0,
            });
            state_mut.hf_selection_anchor = None;
            state_mut.hf_edit_cursor
        };
        Ok(after_cursor)
    }

    async fn paste_hf_text(&self, text: &str, mode: EditingMode) -> Result<JsValue, JsValue> {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let cursor = self.apply_hf_insertion(mode, || {
            for (index, line) in normalized.split('\n').enumerate() {
                if index > 0 && self.insert_hf_breaks_at_caret(1, mode)?.is_none() {
                    return Ok(None);
                }
                if !line.is_empty() && self.insert_hf_text_at_caret(line, mode)?.is_none() {
                    return Ok(None);
                }
            }
            Ok(self.state.borrow().hf_edit_cursor)
        })?;
        if cursor.is_none() {
            return Ok(JsValue::NULL);
        }
        self.finish_hf_edit_result_async(mode).await
    }

    /// Only merge boundaries that map to adjacent ordinary story paragraphs.
    fn hf_paragraph_merge_target(
        word_doc: &WordDocument,
        mode: EditingMode,
        variant: HfVariant,
        cursor: HfEditCursor,
        backward: bool,
    ) -> Option<(usize, HfEditCursor)> {
        let lines = Self::variant_lines(word_doc, mode, variant)?;
        let (role, hf_variant) = Self::hf_role_variant(mode, variant)?;
        let paragraph = |index| {
            let (segment, text) = Self::hf_single_segment(lines.get(index)?)?;
            let plain_text = word_doc.hf_plain_paragraph_text(role, hf_variant, index)?;
            (!plain_text.contains('\t') && plain_text == text).then_some((segment, text))
        };
        let (segment, text) = paragraph(cursor.line_index)?;
        let boundary = if backward { 0 } else { text.len() };
        if cursor.segment != segment || cursor.offset != boundary {
            return None;
        }
        let merge_index = if backward {
            cursor.line_index
        } else {
            cursor.line_index.checked_add(1)?
        };
        let previous_index = merge_index.checked_sub(1)?;
        let (previous_segment, previous_text) = paragraph(previous_index)?;
        paragraph(merge_index)?;
        Some((
            merge_index,
            HfEditCursor {
                line_index: previous_index,
                segment: previous_segment,
                offset: previous_text.len(),
                ..cursor
            },
        ))
    }

    fn delete_hf_text(&self, backward: bool, mode: EditingMode) -> Result<JsValue, JsValue> {
        // Backspace / Delete remove the selection as a whole.
        if self.hf_selection().is_some() {
            let (before_doc, before_cursor, before_selection_anchor, variant) = {
                let state = self.state.borrow();
                (
                    state.word_doc.clone(),
                    state.hf_edit_cursor,
                    state.hf_selection_anchor,
                    state.hf_variant,
                )
            };
            if self.delete_hf_selection(mode)? {
                let (after_doc, after_cursor) = {
                    let state = self.state.borrow();
                    (state.word_doc.clone(), state.hf_edit_cursor)
                };
                if let (Some(before_doc), Some(after_doc)) = (before_doc, after_doc) {
                    self.push_hf_history_entry(HfHistoryEntry {
                        before_doc,
                        after_doc,
                        before_cursor,
                        after_cursor,
                        before_selection_anchor,
                        after_selection_anchor: None,
                        mode,
                        variant,
                    });
                }
                let page_index = after_cursor.map(|cursor| cursor.page_index).unwrap_or(0);
                self.prepare_hf_edit_rerender(mode, page_index);
                return self.finish_hf_delete_result(mode);
            }
        }
        let (cursor, variant) = {
            let state = self.state.borrow();
            match state.hf_edit_cursor {
                Some(c) => (c, state.hf_variant),
                None => return Ok(JsValue::NULL),
            }
        };
        let current_text = self
            .hf_segment_text(mode, variant, &cursor)
            .unwrap_or_default();

        let offset = Self::nearest_hf_caret_boundary(&current_text, cursor.offset);
        let merge_target = self.state.borrow().word_doc.as_ref().and_then(|word_doc| {
            Self::hf_paragraph_merge_target(word_doc, mode, variant, cursor, backward)
        });
        if let Some((merge_index, merged_cursor)) = merge_target {
            let Some((role, hf_variant)) = Self::hf_role_variant(mode, variant) else {
                return Ok(JsValue::NULL);
            };
            let before_doc = self.state.borrow().word_doc.clone();
            let before_cursor = Some(cursor);
            let (after_doc, after_cursor) = {
                let mut state_mut = self.state.borrow_mut();
                let Some(word_doc) = state_mut.word_doc.as_mut() else {
                    return Ok(JsValue::NULL);
                };
                word_doc
                    .merge_hf_paragraph_with_previous(role, hf_variant, merge_index)
                    .map_err(|err| JsValue::from_str(&err))?;
                state_mut.hf_edit_cursor = Some(merged_cursor);
                state_mut.hf_selection_anchor = None;
                (state_mut.word_doc.clone(), state_mut.hf_edit_cursor)
            };
            if let (Some(before_doc), Some(after_doc)) = (before_doc, after_doc) {
                self.push_hf_history(
                    before_doc,
                    after_doc,
                    before_cursor,
                    after_cursor,
                    mode,
                    variant,
                );
            }
            return self.finish_hf_delete_result(mode);
        }

        let (new_text, new_offset) = if backward {
            if offset == 0 {
                return Ok(JsValue::NULL);
            }
            let prev = Self::hf_caret_boundaries(&current_text)
                .into_iter()
                .take_while(|boundary| *boundary < offset)
                .last()
                .unwrap_or(0);
            let mut t = current_text.clone();
            t.replace_range(prev..offset, "");
            (t, prev)
        } else {
            if offset >= current_text.len() {
                return Ok(JsValue::NULL);
            }
            let next = Self::hf_caret_boundaries(&current_text)
                .into_iter()
                .find(|boundary| *boundary > offset)
                .unwrap_or(current_text.len());
            let mut t = current_text.clone();
            t.replace_range(offset..next, "");
            (t, offset)
        };

        let full_text = match self.hf_line_full_text(
            mode,
            variant,
            cursor.line_index,
            cursor.segment,
            &new_text,
        ) {
            Some(t) => t,
            None => return Ok(JsValue::NULL),
        };

        let before_doc = self.state.borrow().word_doc.clone();
        let before_cursor = Some(cursor);
        let (after_doc, after_cursor) = {
            let mut state_mut = self.state.borrow_mut();
            if let Some(word_doc) = state_mut.word_doc.as_mut() {
                let result =
                    Self::replace_hf_line(word_doc, mode, variant, cursor.line_index, &full_text);
                if let Err(err) = result {
                    return Err(JsValue::from_str(&err));
                }
            }
            state_mut.hf_edit_cursor = Some(HfEditCursor {
                page_index: cursor.page_index,
                line_index: cursor.line_index,
                segment: cursor.segment,
                offset: new_offset,
            });
            state_mut.hf_selection_anchor = None;
            (state_mut.word_doc.clone(), state_mut.hf_edit_cursor)
        };
        if let (Some(before_doc), Some(after_doc)) = (before_doc, after_doc) {
            self.push_hf_history(
                before_doc,
                after_doc,
                before_cursor,
                after_cursor,
                mode,
                variant,
            );
        }

        self.finish_hf_delete_result(mode)
    }

    fn finish_hf_delete_result(&self, mode: EditingMode) -> Result<JsValue, JsValue> {
        let anchor_page_index = self
            .state
            .borrow()
            .hf_edit_cursor
            .map(|cursor| cursor.page_index)
            .unwrap_or(0);
        self.prepare_hf_edit_rerender(mode, anchor_page_index);
        let dirty_regions = self.hf_dirty_regions_to_js(mode);
        if self.state.borrow().lazy_mode {
            let snapshots = self.capture_hf_active_page_after_render_sync()?;
            self.store_paged_images(snapshots);
        } else {
            self.rerender_document()?;
            let snapshots = {
                let mut renderer_mut = self.renderer.borrow_mut();
                let (snapshots, _dirty_regions) = self.capture_paged_images(&mut renderer_mut)?;
                snapshots
            };
            self.store_paged_images(snapshots);
        }

        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &self.canvas_size()?)?;
        Reflect::set(&result, &JsValue::from_str("pages"), &self.paged_images())?;
        Reflect::set(&result, &JsValue::from_str("dirtyRegions"), &dirty_regions)?;
        Reflect::set(
            &result,
            &JsValue::from_str("debugMutationKind"),
            &JsValue::from_str("header-footer-edit"),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("editingMode"),
            &JsValue::from_str(Self::hf_editing_mode_name(mode)),
        )?;
        self.write_hf_caret_to_result(&result)?;
        Self::write_history_flags(&result, self.history_flags())?;
        Ok(result.into())
    }

    /// The formatting of the header/footer line the caret is in.
    fn caret_hf_run_state(&self) -> Option<HfRunState> {
        let (mode, cursor, variant) = {
            let state = self.state.borrow();
            (state.editing_mode, state.hf_edit_cursor?, state.hf_variant)
        };
        let (role, hf_variant) = Self::hf_role_variant(mode, variant)?;
        let selection_range = self.hf_selection_paragraph_range(mode);
        let mut state = self.state.borrow_mut();
        let word_doc = state.word_doc.as_mut()?;
        if let Some((line_index, start, end)) = selection_range {
            if let Ok(Some(range_state)) =
                word_doc.hf_range_run_state(role, hf_variant, line_index, start, end)
            {
                return Some(range_state);
            }
        }
        word_doc
            .hf_paragraph_run_state(role, hf_variant, cursor.line_index)
            .ok()
    }

    /// The line-level character format a body inline command stands for in a
    /// header/footer story, or `None` for commands the story cannot take.
    fn hf_run_format_for_command(command: &HeadlessCommand) -> Option<HfRunFormat> {
        let mut format = HfRunFormat::default();
        match command {
            HeadlessCommand::ApplyBold { enabled, .. } => format.bold = Some(*enabled),
            HeadlessCommand::ApplyItalic { enabled, .. } => format.italic = Some(*enabled),
            HeadlessCommand::ApplyUnderline { enabled, kind, .. } => {
                format.underline = Some(*enabled);
                format.underline_kind = kind.clone();
            }
            HeadlessCommand::ApplyStrike {
                enabled, double, ..
            } => {
                format.strike = Some(*enabled && !double);
                format.double_strike = Some(*enabled && *double);
            }
            HeadlessCommand::ApplySuperscript { enabled, .. } => {
                format.baseline = Some(if *enabled { "superscript" } else { "baseline" }.into());
            }
            HeadlessCommand::ApplySubscript { enabled, .. } => {
                format.baseline = Some(if *enabled { "subscript" } else { "baseline" }.into());
            }
            HeadlessCommand::SetFontSize { font_size, .. } => format.font_size = Some(*font_size),
            HeadlessCommand::SetFontFamily { font_family, .. } => {
                format.font_family = Some(font_family.clone())
            }
            HeadlessCommand::SetFontColor { color, .. } => format.color = Some(color.clone()),
            HeadlessCommand::SetHighlightColor { color, .. } => {
                format.highlight = Some(color.clone())
            }
            HeadlessCommand::ClearFormatting { .. } => format.clear_visual = true,
            _ => return None,
        }
        Some(format)
    }

    /// Applies `format` to the header/footer line under the caret, with the
    /// story's own history and repaint.
    fn apply_hf_run_format(&self, format: HfRunFormat) -> Result<JsValue, JsValue> {
        self.apply_hf_run_format_and_reset_paragraph(format, false)
    }

    fn apply_hf_run_format_and_reset_paragraph(
        &self,
        format: HfRunFormat,
        reset_paragraph: bool,
    ) -> Result<JsValue, JsValue> {
        let (mode, cursor, variant) = {
            let state = self.state.borrow();
            let Some(cursor) = state.hf_edit_cursor else {
                return Ok(JsValue::NULL);
            };
            (state.editing_mode, cursor, state.hf_variant)
        };
        let Some((role, hf_variant)) = Self::hf_role_variant(mode, variant) else {
            return Ok(JsValue::NULL);
        };
        let (before_doc, selection_anchor) = {
            let state = self.state.borrow();
            (state.word_doc.clone(), state.hf_selection_anchor)
        };
        let selection_range = self.hf_selection_paragraph_range(mode);
        let after_doc = {
            let mut state_mut = self.state.borrow_mut();
            let Some(word_doc) = state_mut.word_doc.as_mut() else {
                return Ok(JsValue::NULL);
            };
            // Word formats the selection; without one, the caret's line.
            let applied_to_range = match selection_range {
                Some((line_index, start, end)) => word_doc
                    .apply_hf_range_run_format(role, hf_variant, line_index, start, end, &format)
                    .map_err(|error| JsValue::from_str(&error))?,
                None => false,
            };
            if format.clear_visual && selection_range.is_some() && !applied_to_range {
                return Ok(JsValue::NULL);
            }
            if !applied_to_range {
                word_doc
                    .apply_hf_paragraph_run_format(role, hf_variant, cursor.line_index, &format)
                    .map_err(|error| JsValue::from_str(&error))?;
            }
            if reset_paragraph {
                word_doc
                    .reset_hf_paragraph_formatting(role, hf_variant, cursor.line_index)
                    .map_err(|error| JsValue::from_str(&error))?;
            }
            state_mut.word_doc.clone()
        };
        if let (Some(before_doc), Some(after_doc)) = (before_doc, after_doc) {
            self.push_hf_history_entry(HfHistoryEntry {
                before_doc,
                after_doc,
                before_cursor: Some(cursor),
                after_cursor: Some(cursor),
                before_selection_anchor: selection_anchor,
                after_selection_anchor: selection_anchor,
                mode,
                variant,
            });
        }
        self.prepare_hf_edit_rerender(mode, cursor.page_index);
        self.finish_hf_edit_result_sync(mode)
    }

    fn change_hf_case(&self, case: HeadlessChangeCaseMode) -> Result<JsValue, JsValue> {
        let Some((start_cursor, end_cursor)) = self.hf_selection() else {
            return Ok(JsValue::NULL);
        };
        let (mode, cursor, variant) = {
            let state = self.state.borrow();
            let Some(cursor) = state.hf_edit_cursor else {
                return Ok(JsValue::NULL);
            };
            (state.editing_mode, cursor, state.hf_variant)
        };
        let Some((role, hf_variant)) = Self::hf_role_variant(mode, variant) else {
            return Ok(JsValue::NULL);
        };
        let Some((line, start, end)) = self.hf_selection_paragraph_range(mode) else {
            return Ok(JsValue::NULL);
        };
        let before_doc = self.state.borrow().word_doc.clone();
        let (after_doc, after_cursor) = {
            let mut state = self.state.borrow_mut();
            let Some(word_doc) = state.word_doc.as_mut() else {
                return Ok(JsValue::NULL);
            };
            let Some(length) = word_doc
                .change_hf_range_case(role, hf_variant, line, start..end, case)
                .map_err(|error| JsValue::from_str(&error))?
            else {
                return Ok(JsValue::NULL);
            };
            let transformed_end = HfEditCursor {
                offset: start_cursor.offset + length,
                ..end_cursor
            };
            if cursor == end_cursor {
                state.hf_edit_cursor = Some(transformed_end);
            } else {
                state.hf_selection_anchor = Some(transformed_end);
            }
            (state.word_doc.clone(), state.hf_edit_cursor)
        };
        if let (Some(before_doc), Some(after_doc)) = (before_doc, after_doc) {
            self.push_hf_history(
                before_doc,
                after_doc,
                Some(cursor),
                after_cursor,
                mode,
                variant,
            );
        }
        self.prepare_hf_edit_rerender(mode, cursor.page_index);
        self.finish_hf_edit_result_sync(mode)
    }

    /// Synchronous twin of `finish_hf_edit_result_async` for bindings whose
    /// worker handler does not await (the inline formatting commands).
    fn finish_hf_edit_result_sync(&self, mode: EditingMode) -> Result<JsValue, JsValue> {
        let dirty_regions = self.hf_dirty_regions_to_js(mode);
        if self.state.borrow().lazy_mode {
            let snapshots = self.capture_hf_active_page_after_render_sync()?;
            self.store_paged_images(snapshots);
        } else {
            self.rerender_document()?;
            let snapshots = {
                let mut renderer_mut = self.renderer.borrow_mut();
                let (snapshots, _dirty_regions) = self.capture_paged_images(&mut renderer_mut)?;
                snapshots
            };
            self.store_paged_images(snapshots);
        }
        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &self.canvas_size()?)?;
        Reflect::set(&result, &JsValue::from_str("pages"), &self.paged_images())?;
        Reflect::set(&result, &JsValue::from_str("dirtyRegions"), &dirty_regions)?;
        Reflect::set(
            &result,
            &JsValue::from_str("debugMutationKind"),
            &JsValue::from_str("header-footer-edit"),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("editingMode"),
            &JsValue::from_str(Self::hf_editing_mode_name(mode)),
        )?;
        self.write_hf_caret_to_result(&result)?;
        Self::write_history_flags(&result, self.history_flags())?;
        Ok(result.into())
    }

    fn remap_hf_paragraph_cursor(
        state: &mut CanvasDocumentState,
        mode: EditingMode,
        variant: HfVariant,
        cursor: HfEditCursor,
        alignment: &str,
    ) {
        let segment = state
            .word_doc
            .as_ref()
            .and_then(|doc| Self::variant_lines(doc, mode, variant))
            .and_then(|lines| lines.get(cursor.line_index).cloned())
            .and_then(|line| {
                let segments: Vec<_> = [
                    (HfSegment::Left, line.left),
                    (HfSegment::Center, line.center),
                    (HfSegment::Right, line.right),
                ]
                .into_iter()
                .filter(|(_, text)| text.is_some())
                .collect();
                match segments.as_slice() {
                    [] => Some(Self::hf_segment_for_alignment(alignment)),
                    [(segment, _)] => Some(*segment),
                    _ => None,
                }
            });
        if let Some(segment) = segment {
            state.hf_edit_cursor = Some(HfEditCursor { segment, ..cursor });
            if let Some(anchor) = state.hf_selection_anchor.as_mut() {
                if anchor.line_index == cursor.line_index {
                    anchor.segment = segment;
                }
            }
        }
    }

    fn apply_hf_paragraph_commands(
        &self,
        commands: Vec<HeadlessCommand>,
    ) -> Result<JsValue, JsValue> {
        let (mode, cursor, variant) = {
            let state = self.state.borrow();
            let Some(cursor) = state.hf_edit_cursor else {
                return Ok(JsValue::NULL);
            };
            (state.editing_mode, cursor, state.hf_variant)
        };
        let Some((role, hf_variant)) = Self::hf_role_variant(mode, variant) else {
            return Ok(JsValue::NULL);
        };
        let (before_doc, before_selection_anchor) = {
            let state = self.state.borrow();
            (state.word_doc.clone(), state.hf_selection_anchor)
        };
        let (after_doc, after_cursor, after_selection_anchor) = {
            let mut state = self.state.borrow_mut();
            let Some(doc) = state.word_doc.as_mut() else {
                return Ok(JsValue::NULL);
            };
            let changed = match commands.as_slice() {
                [HeadlessCommand::SetParagraphStyle { style_id, .. }] => doc
                    .set_hf_paragraph_style(role, hf_variant, cursor.line_index, style_id.clone()),
                _ => {
                    doc.apply_hf_paragraph_commands(role, hf_variant, cursor.line_index, &commands)
                }
            }
            .map_err(|error| JsValue::from_str(&error))?;
            if !changed {
                return Ok(JsValue::NULL);
            }
            let alignment = doc
                .hf_paragraph_alignment(role, hf_variant, cursor.line_index)
                .unwrap_or_else(|| "left".to_string());
            Self::remap_hf_paragraph_cursor(&mut state, mode, variant, cursor, &alignment);
            (
                state.word_doc.clone(),
                state.hf_edit_cursor,
                state.hf_selection_anchor,
            )
        };
        if let (Some(before_doc), Some(after_doc)) = (before_doc, after_doc) {
            self.push_hf_history_entry(HfHistoryEntry {
                before_doc,
                after_doc,
                before_cursor: Some(cursor),
                after_cursor,
                before_selection_anchor,
                after_selection_anchor,
                mode,
                variant,
            });
        }
        self.prepare_hf_edit_rerender(mode, cursor.page_index);
        self.finish_hf_edit_result_sync(mode)
    }

    fn apply_hf_alignment(&self, alignment: String, mode: EditingMode) -> Result<JsValue, JsValue> {
        let (cursor, variant) = {
            let state = self.state.borrow();
            if state.preview_mode {
                return Ok(JsValue::NULL);
            }
            let Some(cursor) = state.hf_edit_cursor else {
                return Ok(JsValue::NULL);
            };
            (cursor, state.hf_variant)
        };
        let Some((role, hf_variant)) = Self::hf_role_variant(mode, variant) else {
            return Ok(JsValue::NULL);
        };
        let (before_doc, before_selection_anchor) = {
            let state = self.state.borrow();
            (state.word_doc.clone(), state.hf_selection_anchor)
        };
        let (after_doc, after_cursor, after_selection_anchor) = {
            let mut state = self.state.borrow_mut();
            let Some(doc) = state.word_doc.as_mut() else {
                return Ok(JsValue::NULL);
            };
            doc.set_hf_paragraph_alignment(role, hf_variant, cursor.line_index, &alignment)
                .map_err(|error| JsValue::from_str(&error))?;
            Self::remap_hf_paragraph_cursor(&mut state, mode, variant, cursor, &alignment);
            (
                state.word_doc.clone(),
                state.hf_edit_cursor,
                state.hf_selection_anchor,
            )
        };
        if let (Some(before_doc), Some(after_doc)) = (before_doc, after_doc) {
            self.push_hf_history_entry(HfHistoryEntry {
                before_doc,
                after_doc,
                before_cursor: Some(cursor),
                after_cursor,
                before_selection_anchor,
                after_selection_anchor,
                mode,
                variant,
            });
        }
        self.prepare_hf_edit_rerender(mode, cursor.page_index);
        self.finish_hf_edit_result_sync(mode)
    }

    fn adjust_hf_font_size(
        &self,
        direction: jade_docx_headless_api::HeadlessFontSizeDirection,
        step: jade_docx_headless_api::HeadlessFontSizeStep,
    ) -> Result<JsValue, JsValue> {
        use crate::commands::adjust_font_size::{FontSizeDirection, FontSizeStep};
        let (mode, cursor, variant) = {
            let state = self.state.borrow();
            let Some(cursor) = state.hf_edit_cursor else {
                return Ok(JsValue::NULL);
            };
            (state.editing_mode, cursor, state.hf_variant)
        };
        let Some((role, hf_variant)) = Self::hf_role_variant(mode, variant) else {
            return Ok(JsValue::NULL);
        };
        let selection = self
            .hf_selection_paragraph_range(mode)
            .map(|(_, start, end)| start..end);
        // A selection spanning lines/segments is not a whole-line command.
        if self.hf_selection().is_some() && selection.is_none() {
            return Ok(JsValue::NULL);
        }
        let before_doc = self.state.borrow().word_doc.clone();
        let after_doc = {
            let mut state = self.state.borrow_mut();
            let Some(doc) = state.word_doc.as_mut() else {
                return Ok(JsValue::NULL);
            };
            let applied = doc
                .adjust_hf_font_size(
                    role,
                    hf_variant,
                    cursor.line_index,
                    selection,
                    match direction {
                        jade_docx_headless_api::HeadlessFontSizeDirection::Grow => {
                            FontSizeDirection::Grow
                        }
                        jade_docx_headless_api::HeadlessFontSizeDirection::Shrink => {
                            FontSizeDirection::Shrink
                        }
                    },
                    match step {
                        jade_docx_headless_api::HeadlessFontSizeStep::Ladder => {
                            FontSizeStep::Ladder
                        }
                        jade_docx_headless_api::HeadlessFontSizeStep::Point => FontSizeStep::Point,
                    },
                )
                .map_err(|error| JsValue::from_str(&error))?;
            if !applied {
                return Ok(JsValue::NULL);
            }
            state.word_doc.clone()
        };
        if let (Some(before_doc), Some(after_doc)) = (before_doc, after_doc) {
            self.push_hf_history(
                before_doc,
                after_doc,
                Some(cursor),
                Some(cursor),
                mode,
                variant,
            );
        }
        self.prepare_hf_edit_rerender(mode, cursor.page_index);
        self.finish_hf_edit_result_sync(mode)
    }

    async fn pointer_dblclick_async(&self, x: f64, y: f64) -> Result<JsValue, JsValue> {
        // Floating objects can intentionally overlap the header/footer area.
        // Word gives the object hit precedence: double-clicking a textbox edits
        // its text instead of silently switching the active story to header.
        let object_hit = { self.renderer.borrow().hit_test_object(x, y) };
        if object_hit.is_some() {
            if self.state.borrow().editing_mode != EditingMode::Body {
                let selection = self.state.borrow_mut().exit_hf_editing_mode();
                {
                    let mut renderer = self.renderer.borrow_mut();
                    renderer.set_selection_range(selection);
                    renderer.set_editing_mode(EditingMode::Body);
                }
                self.rerender_document()?;
            }

            // A real dblclick sequence normally selected the object on its
            // preceding click. Calling the same path twice also makes the
            // worker-level pointerEvent API deterministic: select, then edit.
            self.pointer_down_async(x, y).await?;
            self.pointer_down_async(x, y).await?;
            let result = Object::new();
            Reflect::set(
                &result,
                &JsValue::from_str("editingMode"),
                &JsValue::from_str("body"),
            )?;
            return Ok(result.into());
        }

        let zone = self.hit_test_zone(x, y);
        match zone {
            EditingMode::Header | EditingMode::Footer => {
                let variant = self
                    .variant_for_page_index(self.page_index_at_y(y), zone == EditingMode::Header);
                if let Some((role, hf_variant)) = Self::hf_role_variant(zone, variant) {
                    let mut state_mut = self.state.borrow_mut();
                    if let Some(word_doc) = state_mut.word_doc.as_mut() {
                        word_doc
                            .ensure_hf_part_for_edit(role, hf_variant)
                            .map_err(|err| JsValue::from_str(&err))?;
                    }
                }
                let cursor = self.resolve_hf_cursor(x, y, zone, variant);
                {
                    let mut state_mut = self.state.borrow_mut();
                    state_mut.enter_hf_editing_mode(zone);
                    state_mut.hf_variant = variant;
                    state_mut.hf_edit_cursor = cursor;
                    state_mut.hf_selection_anchor = None;
                    state_mut.selection = None;
                    state_mut.object_selection = None;
                    state_mut.caret_goal_x = None;
                }
                {
                    let mut renderer_mut = self.renderer.borrow_mut();
                    renderer_mut.set_selection_range(None);
                    renderer_mut.set_editing_mode(zone);
                }
                self.rerender_document()?;
                let result = Object::new();
                Reflect::set(
                    &result,
                    &JsValue::from_str("editingMode"),
                    &JsValue::from_str(match zone {
                        EditingMode::Header => "header",
                        EditingMode::Footer => "footer",
                        EditingMode::Body => "body",
                    }),
                )?;
                // Return caret rect for header/footer
                if let Some(cursor) = self.state.borrow().hf_edit_cursor {
                    let renderer = self.renderer.borrow();
                    if let Some(caret_rect) = self.compute_hf_caret_rect(&renderer, &cursor) {
                        let caret_obj = Object::new();
                        Reflect::set(
                            &caret_obj,
                            &JsValue::from_str("x"),
                            &JsValue::from_f64(caret_rect.0),
                        )?;
                        Reflect::set(
                            &caret_obj,
                            &JsValue::from_str("y"),
                            &JsValue::from_f64(caret_rect.1),
                        )?;
                        Reflect::set(
                            &caret_obj,
                            &JsValue::from_str("height"),
                            &JsValue::from_f64(caret_rect.2),
                        )?;
                        Reflect::set(&result, &JsValue::from_str("hfCaret"), &caret_obj)?;
                    }
                }
                Ok(result.into())
            }
            EditingMode::Body => {
                if self.state.borrow().editing_mode != EditingMode::Body {
                    let selection = self.state.borrow_mut().exit_hf_editing_mode();
                    {
                        let mut renderer_mut = self.renderer.borrow_mut();
                        renderer_mut.set_selection_range(selection);
                        renderer_mut.set_editing_mode(EditingMode::Body);
                    }
                    self.rerender_document()?;

                    let result = Object::new();
                    Reflect::set(
                        &result,
                        &JsValue::from_str("editingMode"),
                        &JsValue::from_str("body"),
                    )?;
                    Ok(result.into())
                } else {
                    // Body double-click: place the caret, then select the word
                    // under it plus its trailing spaces, as Word does.
                    self.pointer_down_async(x, y).await?;
                    if self.state.borrow().object_selection.is_none() {
                        let word = {
                            let renderer = self.renderer.borrow();
                            renderer.selection_range().and_then(|range| {
                                let manager = renderer.selection_manager();
                                manager
                                    .word_range_at(range.focus)
                                    .map(|(start, end)| (range.focus.paragraph, start, end))
                            })
                        };
                        if let Some((paragraph, start, end)) = word {
                            if end > start {
                                self.set_selection_range_surface_async(
                                    paragraph, start, paragraph, end,
                                )
                                .await?;
                            }
                        }
                    }
                    Ok(JsValue::undefined())
                }
            }
        }
    }

    async fn pointer_tripleclick_async(&self, x: f64, y: f64) -> Result<JsValue, JsValue> {
        self.pointer_down_async(x, y).await?;
        if self.state.borrow().editing_mode != EditingMode::Body
            || self.state.borrow().object_selection.is_some()
        {
            return Ok(JsValue::undefined());
        }

        let paragraph = {
            let renderer = self.renderer.borrow();
            renderer.selection_range().map(|range| {
                let manager = renderer.selection_manager();
                let start = manager.move_paragraph_start(range.focus);
                let end = manager.move_paragraph_end(range.focus);
                (start.paragraph, start.offset, end.offset)
            })
        };
        if let Some((paragraph, start, end)) = paragraph {
            let end = {
                let state = self.state.borrow();
                state
                    .word_doc
                    .as_ref()
                    .map_or(TextPosition::new(paragraph, end), |word_doc| {
                        paragraph_selection_end(word_doc, paragraph, end)
                    })
            };
            self.set_selection_range_surface_async(paragraph, start, end.paragraph, end.offset)
                .await?;
        }
        Ok(JsValue::undefined())
    }

    /// Computes the caret rectangle for the current header/footer cursor position.
    fn compute_hf_caret_rect(
        &self,
        renderer: &CanvasRenderer,
        cursor: &HfEditCursor,
    ) -> Option<(f64, f64, f64)> {
        let state = self.state.borrow();
        let word_doc = state.word_doc.as_ref()?;
        let mode = state.editing_mode;
        let variant = state.hf_variant;
        let lines = Self::variant_lines(word_doc, mode, variant)?;
        let empty_line = HeaderLine::default();
        let line = match lines.get(cursor.line_index) {
            Some(line) => line,
            None if cursor.line_index == 0 => &empty_line,
            None => return None,
        };

        if let Some((segment, text)) = Self::hf_single_segment(line) {
            if segment == cursor.segment || text.is_empty() {
                if let Some(metrics) = renderer.hf_story_caret_metrics(
                    cursor.page_index,
                    mode == EditingMode::Header,
                    TextPosition::new(cursor.line_index, cursor.offset),
                    text,
                ) {
                    return Some((metrics.x, metrics.absolute_top, metrics.height));
                }
            }
        }

        let page_height = renderer.page_height_css();
        let margin_bottom = renderer.margin_bottom();
        let margin_left = renderer.margin_left();
        let margin_right = renderer.margin_right();
        let page_width = renderer.page_width();
        let content_width = page_width - margin_left - margin_right;

        let line_height = HF_EDIT_LINE_HEIGHT;
        let base_y = match mode {
            EditingMode::Header => renderer.header_edit_content_top(),
            EditingMode::Footer => {
                Self::footer_edit_content_bottom(page_height, margin_bottom)
                    - line_height * lines.len() as f64
            }
            EditingMode::Body => return None,
        };
        let local_line_y = base_y + line_height * cursor.line_index as f64;
        let page_gap = 16.0_f64;
        let line_y =
            local_line_y + cursor.page_index as f64 * (renderer.page_height_css() + page_gap);

        let text = match cursor.segment {
            HfSegment::Left => line.left.as_deref().unwrap_or(""),
            HfSegment::Center => line.center.as_deref().unwrap_or(""),
            HfSegment::Right => line.right.as_deref().unwrap_or(""),
        };
        let before_cursor = &text[..cursor.offset.min(text.len())];

        let text_before_width = renderer.measure_header_footer_display_text_width(before_cursor);

        let x = match cursor.segment {
            HfSegment::Left => margin_left + text_before_width,
            HfSegment::Center => {
                let full_width = renderer.measure_header_footer_display_text_width(text);
                let center_x = margin_left + content_width / 2.0 - full_width / 2.0;
                center_x + text_before_width
            }
            HfSegment::Right => {
                let full_width = renderer.measure_header_footer_display_text_width(text);
                let right_x = page_width - margin_right - full_width;
                right_x + text_before_width
            }
        };

        Some((x, line_y, line_height))
    }

    #[cfg_attr(feature = "otel", tracing::instrument(skip_all, fields(x, y)))]
    async fn pointer_down_async(&self, x: f64, y: f64) -> Result<(), JsValue> {
        // If in header/footer editing mode, check if clicking body to exit
        {
            let current_mode = self.state.borrow().editing_mode;
            if current_mode != EditingMode::Body {
                let zone = self.hit_test_zone(x, y);
                if zone == EditingMode::Body {
                    // Word keeps header/footer editing active on a single click
                    // into the document body; a body double-click exits the mode.
                    return Ok(());
                } else if zone == current_mode {
                    // Clicking within the same hf zone -> update cursor position.
                    // Variant may change if the user clicked a different page
                    // (default → even → default → ...).
                    let variant = self.variant_for_page_index(
                        self.page_index_at_y(y),
                        current_mode == EditingMode::Header,
                    );
                    let cursor = self.resolve_hf_cursor(x, y, current_mode, variant);
                    {
                        let mut state_mut = self.state.borrow_mut();
                        state_mut.hf_variant = variant;
                        state_mut.hf_edit_cursor = cursor;
                        state_mut.hf_selection_anchor = None;
                        state_mut.caret_goal_x = None;
                    }
                    if let Some(anchor) = cursor {
                        self.drag_state
                            .replace(PointerDragState::HfSelecting { anchor });
                    }
                    return Ok(());
                } else {
                    // Clicking the other hf zone -> switch to it
                    let variant = self.variant_for_page_index(
                        self.page_index_at_y(y),
                        zone == EditingMode::Header,
                    );
                    let cursor = self.resolve_hf_cursor(x, y, zone, variant);
                    {
                        let mut state_mut = self.state.borrow_mut();
                        state_mut.enter_hf_editing_mode(zone);
                        state_mut.hf_variant = variant;
                        state_mut.hf_edit_cursor = cursor;
                        state_mut.hf_selection_anchor = None;
                        state_mut.caret_goal_x = None;
                    }
                    {
                        let mut renderer_mut = self.renderer.borrow_mut();
                        renderer_mut.set_editing_mode(zone);
                    }
                    self.rerender_document()?;
                    return Ok(());
                }
            }
        }
        {
            let mut debug = self.resize_debug.borrow_mut();
            *debug = Some(format!("pointer_down x={} y={}", x, y));
        }
        if self.try_begin_table_resize(x, y)? {
            let mut debug = self.resize_debug.borrow_mut();
            *debug = Some(format!("pointer_down started_row_resize x={} y={}", x, y));
            return Ok(());
        }
        {
            let mut debug = self.resize_debug.borrow_mut();
            *debug = Some(format!(
                "pointer_down fallback_to_selection x={} y={}",
                x, y
            ));
        }

        let object_hit = {
            let renderer = self.renderer.borrow();
            renderer.hit_test_object(x, y)
        };
        if let Some(object_hit) = object_hit {
            let target = FloatingObjectTarget {
                paragraph_index: object_hit.paragraph_index,
                run_index: object_hit.run_index,
                kind: object_hit.kind,
            };
            let edit_selected_text_box = {
                let state = self.state.borrow();
                state.object_selection == Some(target) && target.kind == FloatingObjectKind::TextBox
            };
            if edit_selected_text_box {
                let position = {
                    let mut renderer_mut = self.renderer.borrow_mut();
                    let position = renderer_mut
                        .hit_test_position(x, y)
                        .map(|pos| renderer_mut.clamp_text_position(pos))
                        .and_then(|pos| {
                            let state_ref = self.state.borrow();
                            state_ref.word_doc.as_ref().and_then(|doc| {
                                if doc.text_box_contains_paragraph(target, pos.paragraph) {
                                    Some(pos)
                                } else {
                                    doc.last_text_box_position_for_target(target)
                                }
                            })
                        })
                        .or_else(|| {
                            self.state
                                .borrow()
                                .word_doc
                                .as_ref()
                                .and_then(|doc| doc.last_text_box_position_for_target(target))
                        })
                        .unwrap_or_else(|| {
                            renderer_mut.clamp_text_position(TextPosition::default())
                        });
                    renderer_mut.set_selection_range(Some(SelectionRange::collapsed(position)));
                    renderer_mut.clear_object_transform_preview();
                    position
                };
                self.drag_state.replace(PointerDragState::None);
                let mut state_mut = self.state.borrow_mut();
                state_mut
                    .selection
                    .replace(SelectionRange::collapsed(position));
                state_mut.object_selection = Some(target);
                state_mut.caret_goal_x = None;
                return Ok(());
            } else {
                let text_box_caret = if target.kind == FloatingObjectKind::TextBox {
                    self.state
                        .borrow()
                        .word_doc
                        .as_ref()
                        .and_then(|doc| doc.last_text_box_position_for_target(target))
                } else {
                    None
                };
                {
                    let mut renderer_mut = self.renderer.borrow_mut();
                    renderer_mut.set_selection_range(text_box_caret.map(SelectionRange::collapsed));
                    renderer_mut.clear_object_transform_preview();
                }
                self.drag_state.replace(PointerDragState::None);
                let mut state_mut = self.state.borrow_mut();
                state_mut.selection = text_box_caret.map(SelectionRange::collapsed);
                state_mut.object_selection = Some(target);
                state_mut.caret_goal_x = None;
                return Ok(());
            }
        }

        let (selection, anchor, anchor_cell, cell_mode) = {
            let mut renderer_mut = self.renderer.borrow_mut();
            let position = renderer_mut
                .hit_test_position(x, y)
                .map(|pos| renderer_mut.clamp_text_position(pos))
                .unwrap_or_else(|| renderer_mut.clamp_text_position(TextPosition::default()));
            let position = self.clamp_body_position_to_document(position);
            renderer_mut.clear_object_transform_preview();

            if let Some(cell_id) = renderer_mut.cell_selection_target_at_point(x, y) {
                let range = renderer_mut
                    .cell_selection_range(cell_id, cell_id)
                    .unwrap_or_else(|| SelectionRange::collapsed(position));
                renderer_mut.set_selection_range(Some(range));
                renderer_mut.set_cell_selection(cell_id, cell_id);
                (range, range.anchor, Some(cell_id), true)
            } else {
                let collapsed = SelectionRange::collapsed(position);
                // Update selection state without full re-render - selection overlay handles display
                renderer_mut.set_selection_range(Some(collapsed));

                // Resolve the cell under the pointer using geometry first and
                // fall back to the anchor paragraph's cell (covers clicks on
                // paragraph padding inside a cell). A fresh click always clears
                // any pre-existing cell-range selection — cell mode is re-entered
                // only once the drag crosses a cell boundary.
                let anchor_cell = renderer_mut
                    .cell_at_point(x, y)
                    .or_else(|| renderer_mut.cell_id_for_paragraph(position.paragraph));
                renderer_mut.clear_cell_selection();
                (collapsed, position, anchor_cell, false)
            }
        };

        self.drag_state.replace(PointerDragState::Selecting {
            anchor,
            anchor_cell,
            cell_mode,
        });

        {
            let mut state_mut = self.state.borrow_mut();
            state_mut.selection.replace(selection);
            state_mut.object_selection = None;
            state_mut.caret_goal_x = None;
        }

        Ok(())
    }

    #[cfg_attr(feature = "otel", tracing::instrument(skip_all, fields(x, y)))]
    async fn pointer_move_async(&self, x: f64, y: f64) -> Result<(), JsValue> {
        if let PointerDragState::HfSelecting { anchor } = *self.drag_state.borrow() {
            // Dragging inside a header or footer extends the selection from
            // the pointer-down cursor along the same segment.
            let (mode, variant) = {
                let state = self.state.borrow();
                (state.editing_mode, state.hf_variant)
            };
            if mode == EditingMode::Body {
                return Ok(());
            }
            if let Some(cursor) = self.resolve_hf_cursor(x, y, mode, variant) {
                let mut state_mut = self.state.borrow_mut();
                state_mut.hf_edit_cursor = Some(cursor);
                state_mut.hf_selection_anchor = Some(anchor);
                state_mut.caret_goal_x = None;
            }
            return Ok(());
        }
        let (anchor, anchor_cell, cell_mode) = {
            let mut drag = self.drag_state.borrow_mut();
            match &mut *drag {
                PointerDragState::ColumnResize(state) => {
                    self.update_column_resize_state(state, x);
                    return Ok(());
                }
                PointerDragState::RowResize(state) => {
                    self.update_row_resize_state(state, y);
                    return Ok(());
                }
                PointerDragState::Selecting {
                    anchor,
                    anchor_cell,
                    cell_mode,
                } => (*anchor, *anchor_cell, *cell_mode),
                PointerDragState::None | PointerDragState::HfSelecting { .. } => return Ok(()),
            }
        };

        let range = {
            let mut renderer_mut = self.renderer.borrow_mut();
            let focus = renderer_mut
                .hit_test_position(x, y)
                .map(|pos| renderer_mut.clamp_text_position(pos))
                .unwrap_or_else(|| renderer_mut.clamp_text_position(TextPosition::default()));
            let focus = self.clamp_body_position_to_document(focus);

            let mut range = SelectionRange::new(anchor, focus);

            // If the drag originated inside a table cell, upgrade to a
            // cell-range selection once the cursor crosses into a different
            // cell. While the cursor stays inside the anchor cell we keep
            // the normal text selection (Word-like behaviour).
            //
            // If the drag leaves the anchor's table entirely — e.g., the
            // cursor is on a body paragraph above/below the table — we
            // convert the rectangular selection to "whole table selected"
            // (so every cell is highlighted) and keep the linear text range
            // so body paragraphs beyond the table boundary are highlighted
            // alongside. This matches Word: you can select a column inside
            // a table and keep dragging down past it, and the table becomes
            // fully selected while the body paragraphs below are also
            // swept into the selection.
            if let Some(anchor_cell_id) = anchor_cell {
                let anchor_table = renderer_mut.cell_table_id(anchor_cell_id);
                let focus_cell_from_point = renderer_mut.cell_at_point(x, y);
                let focus_cell_from_paragraph = renderer_mut.cell_id_for_paragraph(focus.paragraph);
                let focus_cell_in_anchor_table = focus_cell_from_point
                    .or(focus_cell_from_paragraph)
                    .filter(|&cell_id| renderer_mut.cell_table_id(cell_id) == anchor_table);

                match focus_cell_in_anchor_table {
                    Some(focus_cell_id) if cell_mode => {
                        if let Some(cell_range) =
                            renderer_mut.cell_selection_range(anchor_cell_id, focus_cell_id)
                        {
                            range = cell_range;
                        }
                        renderer_mut.set_selection_range(Some(range));
                        renderer_mut.set_cell_selection(anchor_cell_id, focus_cell_id);
                    }
                    Some(focus_cell_id) if focus_cell_id != anchor_cell_id => {
                        renderer_mut.set_selection_range(Some(range));
                        renderer_mut.set_cell_selection(anchor_cell_id, focus_cell_id);
                    }
                    Some(_) => {
                        // Cursor is back inside the anchor cell — fall back
                        // to normal (single-cell) text selection.
                        renderer_mut.set_selection_range(Some(range));
                        renderer_mut.clear_cell_selection();
                    }
                    None => {
                        // Focus is outside the anchor's table — expand the
                        // cell selection to cover the whole table.
                        renderer_mut.set_selection_range(Some(range));
                        if let Some(table_id) = anchor_table {
                            renderer_mut.select_entire_table(table_id);
                        }
                    }
                }
            } else {
                // Update selection state without full re-render during drag.
                // Selection overlay in JS handles the visual feedback.
                renderer_mut.set_selection_range(Some(range));
            }

            range
        };

        {
            let mut state_mut = self.state.borrow_mut();
            state_mut.selection = Some(range);
            state_mut.caret_goal_x = None;
        }

        Ok(())
    }

    #[cfg_attr(feature = "otel", tracing::instrument(skip_all))]
    async fn pointer_up_async(&self) -> Result<(), JsValue> {
        let drag_state = self.drag_state.replace(PointerDragState::None);

        match drag_state {
            drag @ (PointerDragState::ColumnResize(_) | PointerDragState::RowResize(_)) => {
                self.finish_table_resize(drag).await
            }
            PointerDragState::HfSelecting { .. } => Ok(()),
            PointerDragState::Selecting { .. } | PointerDragState::None => {
                // Selection changes don't require full re-render.
                // The selection overlay in JS handles the visual feedback.
                // Only update the selection state.
                let selection_after = {
                    let renderer_ref = self.renderer.borrow();
                    renderer_ref.selection_range()
                };

                {
                    let mut state_mut = self.state.borrow_mut();
                    state_mut.selection = selection_after;
                    state_mut.caret_goal_x = None;
                }

                Ok(())
            }
        }
    }

    async fn pointer_cancel_async(&self) -> Result<(), JsValue> {
        self.drag_state.replace(PointerDragState::None);
        // A cancelled drag leaves the table as it was before the drag.
        if self.retract_table_resize_preview()? {
            self.render_after_table_resize().await?;
        }
        Ok(())
    }

    fn try_begin_table_resize(&self, x: f64, y: f64) -> Result<bool, JsValue> {
        {
            let mut debug = self.resize_debug.borrow_mut();
            *debug = Some(format!("try_begin_table_resize x={} y={}", x, y));
        }

        let metadata = {
            let renderer_ref = self.renderer.borrow();
            renderer_ref.table_resize_metadata()
        };

        let mut best_column: Option<(usize, f64)> = None;
        for (idx, handle) in metadata.handles.iter().enumerate() {
            let dx = (handle.x - x).abs();
            if dx > HANDLE_HIT_THRESHOLD {
                continue;
            }
            if y < handle.top - HANDLE_HIT_THRESHOLD || y > handle.bottom + HANDLE_HIT_THRESHOLD {
                continue;
            }
            match best_column {
                Some((_, best_dx)) if dx >= best_dx => {}
                _ => best_column = Some((idx, dx)),
            }
        }

        let mut best_row: Option<(usize, f64, usize, f64)> = None;
        for (idx, handle) in metadata.row_handles.iter().enumerate() {
            if x < handle.left - HANDLE_HIT_THRESHOLD || x > handle.right + HANDLE_HIT_THRESHOLD {
                continue;
            }
            let dy_raw = handle.y - y;
            let dy = dy_raw.abs();
            // Keep the same narrow band as browser hover hit-testing. An
            // expanded interior band intercepts the final line of short cells.
            if dy > HANDLE_HIT_THRESHOLD {
                continue;
            }
            let table_info = match metadata.tables.get(handle.table_index) {
                Some(info) => info,
                None => continue,
            };
            #[cfg(debug_assertions)]
            {
                web_sys::console::log_1(
                    &format!(
                        "row handle candidate idx={} table={} row_index={} kind={:?} rows={} handle_y={} pointer_y={}",
                        idx,
                        handle.table_index,
                        handle.row_index,
                        handle.kind,
                        table_info.row_heights.len(),
                        handle.y,
                        y
                    )
                    .into(),
                );
            }
            if table_info.row_heights.len() <= handle.row_index {
                continue;
            }
            if handle.kind != TableRowResizeHandleKind::TableBottomEdge
                && table_info.row_heights.len() <= handle.row_index + 1
            {
                continue;
            }
            let order = table_info.order;
            let width = (handle.right - handle.left).abs();
            const ROW_DISTANCE_EPSILON: f64 = 0.01;
            match best_row {
                Some((_, best_dy, best_order, best_width))
                    if dy > best_dy + ROW_DISTANCE_EPSILON
                        || (((dy - best_dy).abs() <= ROW_DISTANCE_EPSILON)
                            && (order < best_order
                                || (order == best_order && width >= best_width))) => {}
                _ => {
                    #[cfg(debug_assertions)]
                    {
                        web_sys::console::log_1(
                            &format!(
                                "selecting row candidate idx={} table={} order={} row_index={} dy={} width={}",
                                idx,
                                handle.table_index,
                                order,
                                handle.row_index,
                                dy,
                                width
                            )
                            .into(),
                        );
                    }
                    best_row = Some((idx, dy, order, width));
                }
            }
        }

        if best_row.is_none() {
            for (idx, handle) in metadata.row_handles.iter().enumerate() {
                if handle.kind != TableRowResizeHandleKind::TableBottomEdge {
                    continue;
                }
                if x < handle.left - HANDLE_HIT_THRESHOLD || x > handle.right + HANDLE_HIT_THRESHOLD
                {
                    continue;
                }
                let dy = handle.y - y;
                if dy.abs() > HANDLE_HIT_THRESHOLD {
                    continue;
                }
                let table_info = match metadata.tables.get(handle.table_index) {
                    Some(info) => info,
                    None => continue,
                };
                if table_info.row_heights.len() <= handle.row_index {
                    continue;
                }
                let width = (handle.right - handle.left).abs();
                best_row = Some((idx, dy.abs(), table_info.order, width));
                break;
            }
        }

        enum SelectedHandle {
            Column(usize),
            Row(usize),
        }

        let selected = match (best_column, best_row) {
            (Some((col_idx, col_dist)), Some((row_idx, row_dist, ..))) => {
                if row_dist + ROW_HANDLE_PRIORITY_PENALTY < col_dist {
                    web_sys::console::log_1(
                        &format!("selecting row handle idx={} (col conflict)", row_idx).into(),
                    );
                    self.resize_debug.borrow_mut().replace(format!(
                        "select_row_handle idx={} reason=col_conflict",
                        row_idx
                    ));
                    SelectedHandle::Row(row_idx)
                } else {
                    SelectedHandle::Column(col_idx)
                }
            }
            (Some((col_idx, _)), None) => {
                self.resize_debug
                    .borrow_mut()
                    .replace(format!("select_column_handle idx={}", col_idx));
                SelectedHandle::Column(col_idx)
            }
            (None, Some((row_idx, _, _, _))) => {
                web_sys::console::log_1(&format!("selecting row handle idx={}", row_idx).into());
                self.resize_debug
                    .borrow_mut()
                    .replace(format!("select_row_handle idx={} reason=row_only", row_idx));
                SelectedHandle::Row(row_idx)
            }
            (None, None) => return Ok(false),
        };

        match selected {
            SelectedHandle::Column(handle_index) => {
                let handle = metadata
                    .handles
                    .get(handle_index)
                    .ok_or_else(|| JsValue::from_str("table resize handle out of range"))?;
                let table_info = metadata
                    .tables
                    .get(handle.table_index)
                    .ok_or_else(|| JsValue::from_str("table layout missing for handle"))?;

                let resize_state = match handle.kind {
                    TableResizeHandleKind::BetweenColumns => {
                        if table_info.column_widths.len() <= handle.column_index + 1 {
                            return Ok(false);
                        }
                        let left_width = table_info.column_widths[handle.column_index];
                        let right_width = table_info.column_widths[handle.column_index + 1];
                        if (left_width + right_width).abs() <= f64::EPSILON {
                            return Ok(false);
                        }
                        ColumnResizeState::between_columns(
                            table_info.order,
                            handle.column_index,
                            left_width,
                            right_width,
                            table_info.column_widths.clone(),
                            x,
                            handle.reverse_delta,
                        )
                    }
                    TableResizeHandleKind::TableRightEdge => {
                        if table_info.column_widths.len() <= handle.column_index {
                            return Ok(false);
                        }
                        let left_width = table_info.column_widths[handle.column_index];
                        if left_width.abs() <= f64::EPSILON {
                            return Ok(false);
                        }
                        ColumnResizeState::table_right_edge(
                            table_info.order,
                            handle.column_index,
                            left_width,
                            table_info.column_widths.clone(),
                            x,
                        )
                    }
                };

                self.drag_state
                    .replace(PointerDragState::ColumnResize(resize_state));
                Ok(true)
            }
            SelectedHandle::Row(handle_index) => {
                let handle = metadata
                    .row_handles
                    .get(handle_index)
                    .ok_or_else(|| JsValue::from_str("table row resize handle out of range"))?;
                let table_info = metadata
                    .tables
                    .get(handle.table_index)
                    .ok_or_else(|| JsValue::from_str("table layout missing for handle"))?;

                if table_info.row_heights.len() <= handle.row_index {
                    return Ok(false);
                }

                let top_height = table_info.row_heights[handle.row_index];
                let resize_state = match handle.kind {
                    TableRowResizeHandleKind::BetweenRows => {
                        if table_info.row_heights.len() <= handle.row_index + 1 {
                            return Ok(false);
                        }
                        let bottom_height = table_info.row_heights[handle.row_index + 1];
                        if (top_height + bottom_height).abs() <= f64::EPSILON {
                            return Ok(false);
                        }
                        self.resize_debug.borrow_mut().replace(format!(
                            "start_between_rows table={} row={} top={} bottom={}",
                            table_info.order, handle.row_index, top_height, bottom_height
                        ));
                        RowResizeState::between_rows(
                            table_info.order,
                            handle.row_index,
                            top_height,
                            bottom_height,
                            y,
                        )
                    }
                    TableRowResizeHandleKind::TableBottomEdge => {
                        self.resize_debug.borrow_mut().replace(format!(
                            "start_table_bottom_edge table={} row={} top={} pointer_y={}",
                            table_info.order, handle.row_index, top_height, y
                        ));
                        RowResizeState::table_bottom_edge(
                            table_info.order,
                            handle.row_index,
                            top_height,
                            y,
                        )
                    }
                };

                self.drag_state
                    .replace(PointerDragState::RowResize(resize_state));
                Ok(true)
            }
        }
    }

    fn update_column_resize_state(&self, state: &mut ColumnResizeState, pointer_x: f64) {
        let delta = column_resize_delta(pointer_x - state.initial_pointer_x, state.reverse_delta);
        match state.kind {
            ColumnResizeKind::BetweenColumns => {
                if let Some(total_width) = state.total_width {
                    let min_width = (total_width / 2.0).min(MIN_COLUMN_WIDTH_PX);
                    let max_left = (total_width - min_width).max(min_width);
                    let new_left = (state.initial_left_width + delta).clamp(min_width, max_left);
                    state.current_left_width = new_left;
                    let remaining = total_width - new_left;
                    state.current_right_width = Some(remaining);
                }
            }
            ColumnResizeKind::TableRightEdge => {
                let new_left = (state.initial_left_width + delta).max(MIN_COLUMN_WIDTH_PX);
                state.current_left_width = new_left;
            }
        }
    }

    fn update_row_resize_state(&self, state: &mut RowResizeState, pointer_y: f64) {
        let delta = pointer_y - state.initial_pointer_y;
        match state.kind {
            RowResizeKind::BetweenRows => {
                if let Some(total_height) = state.total_height {
                    let min_height = MIN_ROW_HEIGHT_PX;
                    let max_top = (total_height - min_height).max(min_height);
                    let new_top = (state.initial_top_height + delta).clamp(min_height, max_top);
                    state.current_top_height = new_top;
                    let remaining = total_height - new_top;
                    state.current_bottom_height = Some(remaining.max(min_height));
                }
            }
            RowResizeKind::TableBottomEdge => {
                let new_top = (state.initial_top_height + delta).max(MIN_ROW_HEIGHT_PX);
                state.current_top_height = new_top;
            }
        }
    }

    /// The command that applies a column drag's current widths, or `None`
    /// while the boundary is still where the drag started.
    fn column_resize_command(
        state: &ColumnResizeState,
    ) -> Result<Option<ResizeTableColumnCommand>, JsValue> {
        let mut resized_widths = state.initial_column_widths.clone();
        match state.kind {
            ColumnResizeKind::BetweenColumns => {
                let initial_right = state.initial_right_width.unwrap_or(0.0);
                let current_right = state.current_right_width.unwrap_or(initial_right);
                let left_diff = (state.current_left_width - state.initial_left_width).abs();
                let right_diff = (current_right - initial_right).abs();
                if left_diff < WIDTH_CHANGE_EPSILON && right_diff < WIDTH_CHANGE_EPSILON {
                    return Ok(None);
                }

                if resized_widths.len() <= state.column_index + 1 {
                    return Err(JsValue::from_str("table resize grid is incomplete"));
                }
                resized_widths[state.column_index] = state.current_left_width;
                resized_widths[state.column_index + 1] = current_right;
            }
            ColumnResizeKind::TableRightEdge => {
                let diff = (state.current_left_width - state.initial_left_width).abs();
                if diff < WIDTH_CHANGE_EPSILON {
                    return Ok(None);
                }
                if resized_widths.len() <= state.column_index {
                    return Err(JsValue::from_str("table resize grid is incomplete"));
                }
                resized_widths[state.column_index] = state.current_left_width;
            }
        }
        Ok(Some(ResizeTableColumnCommand::rendered_columns(
            state.table_order_index,
            resized_widths.into_iter().map(Self::px_to_points).collect(),
        )))
    }

    /// The command that applies a row drag's current heights, or `None`
    /// while the boundary is still where the drag started.
    fn row_resize_command(state: &RowResizeState) -> Option<HeadlessCommand> {
        match state.kind {
            RowResizeKind::BetweenRows => {
                let initial_bottom = state.initial_bottom_height.unwrap_or(0.0);
                let current_bottom = state.current_bottom_height.unwrap_or(initial_bottom);
                let top_diff = (state.current_top_height - state.initial_top_height).abs();
                let bottom_diff = (current_bottom - initial_bottom).abs();
                if top_diff < WIDTH_CHANGE_EPSILON && bottom_diff < WIDTH_CHANGE_EPSILON {
                    return None;
                }
                Some(HeadlessCommand::ResizeTableRowBetween {
                    table_order_index: state.table_order_index,
                    row_index: state.row_index,
                    new_top_height: Self::px_to_points(state.current_top_height),
                    new_bottom_height: Self::px_to_points(current_bottom),
                })
            }
            RowResizeKind::TableBottomEdge => {
                let diff = (state.current_top_height - state.initial_top_height).abs();
                if diff < WIDTH_CHANGE_EPSILON {
                    return None;
                }
                Some(HeadlessCommand::ResizeTableRowBottomEdge {
                    table_order_index: state.table_order_index,
                    row_index: state.row_index,
                    new_height: Self::px_to_points(state.current_top_height),
                })
            }
        }
    }

    /// Applies the drag in progress to the document: the command for its
    /// current geometry, or none when the boundary is back where it started.
    /// Returns whether it recorded a history entry.
    fn apply_table_resize_drag(&self, drag: &PointerDragState) -> Result<bool, JsValue> {
        enum Resize {
            Column(ResizeTableColumnCommand),
            Row(Box<HeadlessCommand>),
        }
        let resize = match drag {
            PointerDragState::ColumnResize(state) => {
                Self::column_resize_command(state)?.map(Resize::Column)
            }
            PointerDragState::RowResize(state) => {
                Self::row_resize_command(state).map(|command| Resize::Row(Box::new(command)))
            }
            _ => None,
        };
        let Some(resize) = resize else {
            return Ok(false);
        };
        // See `apply_inline_style_with_scope`.
        let _document_cache = self.drop_renderer_document_cache();
        let mut state_mut = self.state.borrow_mut();
        let mut word_doc = state_mut
            .word_doc
            .take()
            .ok_or_else(|| JsValue::from_str("No document loaded"))?;
        // A drag arrives as pointer events, which the worker does not treat as
        // edits, so a deferred load may not have its preservation XML yet.
        // Without it the grid change reaches the model but never the saved file.
        if let Err(err) = word_doc.ensure_roundtrip_xml() {
            state_mut.word_doc = Some(word_doc);
            return Err(JsValue::from_str(&err.to_string()));
        }
        let mut command_executor = std::mem::take(&mut state_mut.command_executor);
        let revision_before = command_executor.history_revision();
        let result = match resize {
            Resize::Column(command) => {
                let mut selection_after = state_mut.selection;
                let mut context = CommandContext::new(&mut word_doc, &mut selection_after);
                command_executor
                    .execute(Box::new(command), &mut context)
                    .map(|()| selection_after)
                    .map_err(|err| JsValue::from_str(&err.to_string()))
            }
            // Table row resize is applied through headless commands for a single mutation path.
            Resize::Row(command) => apply_headless_commands_with_executor_to_document(
                &mut word_doc,
                &mut command_executor,
                state_mut.selection,
                &HeadlessEditRequest {
                    commands: vec![*command],
                },
            )
            .map_err(|err| JsValue::from_str(&err.to_string())),
        };
        let recorded = command_executor.history_revision() != revision_before;
        state_mut.command_executor = command_executor;
        state_mut.word_doc = Some(word_doc);
        state_mut.selection = result?;
        state_mut.paged_images.clear();
        Ok(recorded)
    }

    /// Takes the live preview of the drag in progress back off the document
    /// without leaving it in the redo history. Returns whether there was one.
    fn retract_table_resize_preview(&self) -> Result<bool, JsValue> {
        if !self.table_resize_preview.replace(false) {
            return Ok(false);
        }
        let _document_cache = self.drop_renderer_document_cache();
        let mut state_mut = self.state.borrow_mut();
        let mut word_doc = state_mut
            .word_doc
            .take()
            .ok_or_else(|| JsValue::from_str("No document loaded"))?;
        let mut command_executor = std::mem::take(&mut state_mut.command_executor);
        let mut selection = state_mut.selection;
        let result = {
            let mut context = CommandContext::new(&mut word_doc, &mut selection);
            command_executor.retract_last(&mut context)
        };
        state_mut.command_executor = command_executor;
        state_mut.word_doc = Some(word_doc);
        state_mut.selection = selection;
        result.map_err(|err| JsValue::from_str(&err.to_string()))?;
        Ok(true)
    }

    /// Word's 「ドラッグ中も文書の内容を更新する」: reflow the table to the
    /// boundary being dragged. Each preview replaces the previous one, so the
    /// drag still ends as a single undo step.
    async fn preview_table_resize(&self) -> Result<(), JsValue> {
        if self.state.borrow().preview_mode {
            return Ok(());
        }
        let drag = self.drag_state.borrow().clone();
        if !matches!(
            drag,
            PointerDragState::ColumnResize(_) | PointerDragState::RowResize(_)
        ) {
            return Ok(());
        }
        let retracted = self.retract_table_resize_preview()?;
        let applied = self.apply_table_resize_drag(&drag)?;
        self.table_resize_preview.set(applied);
        if applied || retracted {
            self.render_after_table_resize().await?;
        }
        Ok(())
    }

    /// Ends a column or row drag: the preview, if any, gives way to one
    /// recorded edit for the final geometry.
    async fn finish_table_resize(&self, drag: PointerDragState) -> Result<(), JsValue> {
        if self.state.borrow().preview_mode {
            return Ok(());
        }
        let retracted = self.retract_table_resize_preview()?;
        let applied = self.apply_table_resize_drag(&drag)?;
        if applied || retracted {
            self.render_after_table_resize().await?;
        }
        if let PointerDragState::RowResize(state) = &drag {
            self.resize_debug.borrow_mut().replace(format!(
                "finish_row_resize table={} row={} current_px={}",
                state.table_order_index, state.row_index, state.current_top_height
            ));
        }
        Ok(())
    }

    /// Re-lays the document out after a table's column widths or row heights
    /// changed and captures the repainted pages.
    async fn render_after_table_resize(&self) -> Result<(), JsValue> {
        // Table geometry changes can affect layout before/after the current text selection.
        // Re-layout from the document start so table edits never get skipped by a stale caret range.
        let dirty_from = 0usize;
        let (options, numbering_part, header_lines, footer_lines) = {
            let state_ref = self.state.borrow();
            let word_doc = state_ref
                .word_doc
                .as_ref()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;
            (
                state_ref.options.clone(),
                word_doc.numbering_part.clone(),
                word_doc.header_lines.clone(),
                word_doc.footer_lines.clone(),
            )
        };

        let anchor_page = {
            let renderer = self.renderer.borrow();
            let selection_anchor = renderer.selection_range().and_then(|range| {
                let paragraph = range.anchor.paragraph.min(range.focus.paragraph);
                renderer.cached_paragraph_start_page(paragraph)
            });
            selection_anchor
                .or_else(|| renderer.visible_page_range().as_ref().map(|r| *r.end()))
                .or_else(|| renderer.cached_paragraph_start_page(dirty_from))
                .or_else(|| renderer.layout_page_limit())
                .unwrap_or(1)
        };

        {
            let mut renderer_mut = self.renderer.borrow_mut();
            // Table resize changes cell widths which affects line wrapping, paragraph
            // layout positions, and draw commands for the entire table region.
            // Invalidate all caches and force a full repaint so stale borders/content
            // from the old geometry are not replayed.
            renderer_mut.mark_full_repaint();
            renderer_mut.invalidate_paragraph_content_hash_cache();
            renderer_mut.clear_paragraph_render_cache();
            renderer_mut.invalidate_paragraph_wrap_cache_from(dirty_from);
            // The table's new height moves every page after it, and a lazy
            // layout only walks the visible window here: the page-start seek
            // snapshots recorded past it describe the old geometry, and a later
            // extension resuming from one would paginate as if the table never
            // changed.
            renderer_mut.set_incremental_dirty_from(dirty_from);
            let lazy_mode = self.state.borrow().lazy_mode;
            // Respect host-provided visible range if configured (viewport virtualization).
            // Ensure the edited page stays visible even if the range is stale.
            Self::ensure_visible_page_range_for_edit(&mut renderer_mut, lazy_mode, anchor_page);
            // In lazy mode, keep layout scoped to the visible window to avoid full re-layout.
            let prev_limit: Option<Option<usize>> = if lazy_mode {
                let desired = Self::lazy_visible_end(&renderer_mut, anchor_page);
                let prev = renderer_mut.layout_page_limit();
                renderer_mut.set_layout_page_limit(Some(desired));
                Some(prev)
            } else {
                renderer_mut.set_layout_page_limit(None);
                None
            };
            let state_ref = self.state.borrow();
            if let Some(word_doc) = state_ref.word_doc.as_ref() {
                if let Some(document) = word_doc.document.as_ref() {
                    renderer_mut.render_with_numbering_rc(
                        Rc::clone(document),
                        &options,
                        numbering_part.as_ref(),
                        header_lines.as_deref(),
                        footer_lines.as_deref(),
                        Some(&word_doc.comments),
                    )?;
                }
            }
            if let Some(prev) = prev_limit {
                renderer_mut.set_layout_page_limit(prev);
            }
        }

        let snapshots = self.capture_pages_after_render().await?;
        self.store_paged_images(snapshots);

        {
            let mut state_mut = self.state.borrow_mut();
            state_mut.caret_goal_x = None;
        }

        Ok(())
    }

    fn px_to_points(px: f64) -> f32 {
        (px * 72.0 / 96.0) as f32
    }

    fn runtime_text_position(
        document: &WordDocument,
        position: TextPosition,
    ) -> Option<RuntimeTextPosition> {
        let story = document.story_position_for_surface(position.paragraph)?;
        Some(RuntimeTextPosition {
            paragraph: story.index,
            offset: position.offset,
            story: story.story,
            anchor_paragraph: story.anchor_paragraph,
            note_id: story.note_id,
        })
    }

    async fn set_selection_range_surface_async(
        &self,
        anchor_paragraph: usize,
        anchor_offset: usize,
        focus_paragraph: usize,
        focus_offset: usize,
    ) -> Result<(), JsValue> {
        // SelectionManager clamps against registered layout paragraphs. In lazy
        // mode a valid model paragraph after a section break may not be registered
        // yet; clamping it would silently redirect the next edit to an earlier
        // paragraph. Extend through the target before installing the selection.
        loop {
            let next_page = {
                let renderer = self.renderer.borrow();
                if renderer.last_layout_complete()
                    || (renderer.has_selection_paragraph(anchor_paragraph)
                        && renderer.has_selection_paragraph(focus_paragraph))
                {
                    break;
                }
                renderer.layout_page_limit().unwrap_or(0).saturating_add(1)
            };
            self.ensure_layout_through_page_async(next_page, true)
                .await?;
        }

        let range = SelectionRange::new(
            TextPosition::new(anchor_paragraph, anchor_offset),
            TextPosition::new(focus_paragraph, focus_offset),
        );

        let (changed, use_overlay, needs_rerender, selection_after) = {
            let mut renderer_mut = self.renderer.borrow_mut();
            renderer_mut.clear_cell_selection();
            let changed = renderer_mut.set_selection_range(Some(range));
            let selection_after = renderer_mut.selection_range();
            let use_overlay = renderer_mut.use_selection_overlay();
            let mut needs_rerender = false;
            if changed && use_overlay {
                if let Some(selection) = selection_after {
                    if selection.is_collapsed() {
                        needs_rerender = renderer_mut.caret_metrics().is_none();
                    } else {
                        needs_rerender = renderer_mut.get_selection_rects().is_empty();
                    }
                }
            }
            if changed && (!use_overlay || needs_rerender) {
                renderer_mut.skip_change_detection_once();
                renderer_mut.rerender_with_cached_state()?;
            }
            renderer_mut.clear_object_transform_preview();
            (changed, use_overlay, needs_rerender, selection_after)
        };

        let snapshots = if changed && (!use_overlay || needs_rerender) {
            self.capture_pages_after_render().await?
        } else {
            None
        };

        self.store_paged_images(snapshots);
        {
            let mut state_mut = self.state.borrow_mut();
            state_mut.selection = selection_after;
            state_mut.object_selection = None;
            state_mut.caret_goal_x = None;
        }

        Ok(())
    }

    async fn clear_selection_async(&self) -> Result<(), JsValue> {
        self.drag_state.replace(PointerDragState::None);

        let (changed, use_overlay) = {
            let mut renderer_mut = self.renderer.borrow_mut();
            let changed = renderer_mut.clear_selection();
            let use_overlay = renderer_mut.use_selection_overlay();
            renderer_mut.clear_object_transform_preview();
            if changed && !use_overlay {
                renderer_mut.skip_change_detection_once();
                renderer_mut.rerender_with_cached_state()?;
            }
            (changed, use_overlay)
        };

        let snapshots = if changed && !use_overlay {
            self.capture_pages_after_render().await?
        } else {
            None
        };
        let selection_after = {
            let renderer_ref = self.renderer.borrow();
            renderer_ref.selection_range()
        };

        self.store_paged_images(snapshots);
        {
            let mut state_mut = self.state.borrow_mut();
            state_mut.selection = selection_after;
            state_mut.object_selection = None;
            state_mut.caret_goal_x = None;
        }

        Ok(())
    }

    fn object_transform_response(&self) -> Result<JsValue, JsValue> {
        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &self.canvas_size()?)?;
        Reflect::set(&result, &JsValue::from_str("pages"), &self.paged_images())?;
        Reflect::set(&result, &JsValue::from_str("caret"), &self.caret_rect())?;
        Self::write_history_flags(&result, self.history_flags())?;
        self.write_object_selection(&result)?;
        self.write_merge_state(&result)?;
        Ok(result.into())
    }

    async fn preview_selected_object_transform_async(
        &self,
        transform: FloatingObjectTransform,
        reanchor: bool,
    ) -> Result<JsValue, JsValue> {
        let target = self
            .state
            .borrow()
            .object_selection
            .ok_or_else(|| JsValue::from_str("No object selected"))?;

        {
            let mut renderer = self.renderer.borrow_mut();
            // Render the transform first. Insertion and the previous preview
            // can change pagination, so resolving the anchor against the old
            // selection geometry makes the first drag frame stick to its
            // source paragraph.
            renderer.set_object_transform_preview(target, transform, None);
            renderer.mark_object_target_page_dirty(target);
            renderer.skip_change_detection_once();
            renderer.rerender_with_cached_state()?;
        }

        let anchor_paragraph = reanchor
            .then(|| self.object_anchor_paragraph_for_transform(transform))
            .flatten();
        self.renderer.borrow_mut().set_object_transform_preview(
            target,
            transform,
            anchor_paragraph,
        );

        let snapshots = self.capture_pages_after_render().await?;
        self.store_paged_images(snapshots);
        self.object_transform_response()
    }

    async fn clear_selected_object_transform_preview_async(&self) -> Result<JsValue, JsValue> {
        {
            let mut renderer = self.renderer.borrow_mut();
            if let Some(target) = self.state.borrow().object_selection {
                renderer.mark_object_target_page_dirty(target);
            }
            renderer.clear_object_transform_preview();
            renderer.skip_change_detection_once();
            renderer.rerender_with_cached_state()?;
        }
        let snapshots = self.capture_pages_after_render().await?;
        self.store_paged_images(snapshots);
        self.object_transform_response()
    }

    async fn commit_selected_object_transform_async(
        &self,
        transform: FloatingObjectTransform,
        reanchor: bool,
    ) -> Result<JsValue, JsValue> {
        let target = self
            .state
            .borrow()
            .object_selection
            .ok_or_else(|| JsValue::from_str("No object selected"))?;
        let anchor_paragraph = reanchor
            .then(|| self.object_anchor_paragraph_for_transform(transform))
            .flatten();
        let anchor_paragraph_top = if target.kind == FloatingObjectKind::TextBox && reanchor {
            let paragraph_index = anchor_paragraph.unwrap_or(target.paragraph_index);
            self.renderer
                .borrow()
                .paragraph_layout_origin(paragraph_index)
                .map(|(_, _, top)| top)
        } else {
            None
        };
        let mut command = TransformObjectCommand::new(target, transform);
        if let Some(paragraph_index) = anchor_paragraph {
            command = command.with_anchor_paragraph(paragraph_index);
        }
        if let Some(top) = anchor_paragraph_top {
            command = command.with_anchor_paragraph_top(top);
        }
        self.commit_selected_object_command_async(target, Box::new(command), anchor_paragraph)
            .await
    }

    async fn format_selected_image_async(&self, spec: ImageFormatSpec) -> Result<JsValue, JsValue> {
        let target = self
            .state
            .borrow()
            .object_selection
            .ok_or_else(|| JsValue::from_str("No object selected"))?;
        if target.kind != FloatingObjectKind::Image {
            return Err(JsValue::from_str("Selected object is not an image"));
        }
        self.commit_selected_object_command_async(
            target,
            Box::new(FormatImageCommand::new(target, spec)),
            None,
        )
        .await
    }

    async fn format_selected_text_box_async(
        &self,
        spec: TextBoxFormatSpec,
    ) -> Result<JsValue, JsValue> {
        let selected = self.state.borrow().object_selection;
        let target = match selected {
            Some(target) => target,
            // Typing inside a shape: format the shape around the caret.
            None => self
                .caret_text_box_target()
                .ok_or_else(|| JsValue::from_str("No object selected"))?,
        };
        if target.kind != FloatingObjectKind::TextBox {
            return Err(JsValue::from_str(
                "Selected object is not a text box or shape",
            ));
        }
        self.commit_selected_object_command_async(
            target,
            Box::new(FormatTextBoxCommand::new(target, spec)),
            None,
        )
        .await
    }

    async fn replace_selected_image_async(
        &self,
        bytes: Vec<u8>,
        mime_type: String,
        width_px: Option<f64>,
        height_px: Option<f64>,
    ) -> Result<JsValue, JsValue> {
        let target = self
            .state
            .borrow()
            .object_selection
            .ok_or_else(|| JsValue::from_str("No object selected"))?;
        if target.kind != FloatingObjectKind::Image {
            return Err(JsValue::from_str("Selected object is not an image"));
        }
        self.commit_selected_object_command_async(
            target,
            Box::new(ReplaceImageCommand::new(
                target, bytes, mime_type, width_px, height_px,
            )),
            None,
        )
        .await
    }

    fn object_anchor_paragraph_for_transform(
        &self,
        transform: FloatingObjectTransform,
    ) -> Option<usize> {
        let selection = self.object_selection_payload()?;
        let renderer = self.renderer.borrow();
        let page_top = renderer.page_top_y(selection.page.saturating_sub(1));
        let global_y = page_top + transform.y;
        let state = self.state.borrow();
        let document = state.word_doc.as_ref()?;
        // Word reanchors a moved floating object as its top edge crosses body
        // paragraph lines. Keep that visual-line decision in jade-docx-layout and
        // only filter out textbox/note surfaces here.
        renderer.hit_test_body_anchor_paragraph(transform.x, global_y, document)
    }

    async fn commit_selected_object_command_async(
        &self,
        target: FloatingObjectTarget,
        command: Box<dyn Command>,
        relocated_anchor: Option<usize>,
    ) -> Result<JsValue, JsValue> {
        if self.state.borrow().preview_mode {
            return self.object_transform_response();
        }
        let anchor_page = self
            .object_selection_payload()
            .map(|selection| selection.page)
            .unwrap_or(1);

        // See `apply_inline_style_with_scope`.
        let _document_cache = self.drop_renderer_document_cache();
        let (options, numbering_part, header_lines, footer_lines) = {
            let mut state_mut = self.state.borrow_mut();
            let mut word_doc = state_mut
                .word_doc
                .take()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;

            let relocated_target = relocated_anchor
                .filter(|&paragraph_index| paragraph_index != target.paragraph_index)
                .and_then(|paragraph_index| {
                    let body_index = word_doc.body_index_for_surface(paragraph_index)?;
                    let run_index = word_doc.paragraph(paragraph_index)?.runs.len();
                    Some((body_index, run_index))
                });

            state_mut.selection = Some(SelectionRange::collapsed(TextPosition::new(
                target.paragraph_index,
                0,
            )));
            let mut command_executor = std::mem::take(&mut state_mut.command_executor);
            let command_result = {
                let mut ctx = CommandContext::new(&mut word_doc, &mut state_mut.selection);
                command_executor.execute(command, &mut ctx)
            };
            state_mut.command_executor = command_executor;
            if let Err(err) = command_result {
                state_mut.word_doc = Some(word_doc);
                return Err(JsValue::from_str(&err.to_string()));
            }

            let selected_target = relocated_target
                .and_then(|(body_index, run_index)| {
                    word_doc
                        .surface_index_for_body(body_index)
                        .map(|paragraph_index| FloatingObjectTarget {
                            paragraph_index,
                            run_index,
                            kind: target.kind,
                        })
                })
                .unwrap_or(target);
            state_mut.object_selection = Some(selected_target);
            state_mut.paged_images.clear();

            let options = state_mut.options.clone();
            let numbering_part = word_doc.numbering_part.clone();
            let header_lines = word_doc.header_lines.clone();
            let footer_lines = word_doc.footer_lines.clone();
            state_mut.word_doc = Some(word_doc);

            (options, numbering_part, header_lines, footer_lines)
        };

        {
            let mut renderer_mut = self.renderer.borrow_mut();
            renderer_mut.clear_object_transform_preview();
            // Repaint every visible page from scratch. An object command is not
            // a single-paragraph text edit: the object leaves pixels on the page
            // it moved away from, a re-anchor rewrites two paragraphs, and a
            // wrapped object reflows body text below it. Modelling it as
            // `set_incremental_dirty_from(0)` plus the object's page made the
            // renderer treat it as an edit of paragraph 0 (band redraw clipped
            // the repaint to that paragraph, so the old border survived) and
            // left other visible pages to be promoted mid-walk, dropping the
            // lines that had already flowed onto them.
            renderer_mut.mark_full_repaint();
            let lazy_mode = self.state.borrow().lazy_mode;
            Self::ensure_visible_page_range_for_edit(&mut renderer_mut, lazy_mode, anchor_page);
            let prev_limit: Option<Option<usize>> = if lazy_mode {
                let desired = Self::lazy_visible_end(&renderer_mut, anchor_page);
                let prev = renderer_mut.layout_page_limit();
                renderer_mut.set_layout_page_limit(Some(desired));
                Some(prev)
            } else {
                renderer_mut.set_layout_page_limit(None);
                None
            };
            let state_ref = self.state.borrow();
            if let Some(word_doc) = state_ref.word_doc.as_ref() {
                if let Some(document) = word_doc.document.as_ref() {
                    renderer_mut.render_with_numbering_rc(
                        Rc::clone(document),
                        &options,
                        numbering_part.as_ref(),
                        header_lines.as_deref(),
                        footer_lines.as_deref(),
                        Some(&word_doc.comments),
                    )?;
                }
            }
            if let Some(prev) = prev_limit {
                renderer_mut.set_layout_page_limit(prev);
            }
        }

        let snapshots = self.capture_pages_after_render().await?;
        self.store_paged_images(snapshots);
        self.object_transform_response()
    }

    #[cfg_attr(feature = "otel", tracing::instrument(skip_all, fields(text_len = text.len())))]
    async fn insert_text_async(&self, text: String) -> Result<JsValue, JsValue> {
        self.insert_text_with_autoformat_async(text, None).await
    }

    async fn insert_text_with_autoformat_async(
        &self,
        text: String,
        autoformat: Option<(String, String)>,
    ) -> Result<JsValue, JsValue> {
        let _manual_span = info_span!("insert_text_async_manual", text_len = text.len()).entered();

        if text.is_empty() {
            return Ok(JsValue::NULL);
        }

        if self.state.borrow().preview_mode {
            return Ok(JsValue::NULL);
        }

        // Resolve context inside the serialized worker mutation, so preceding
        // keystrokes and story/caret changes are already reflected here.
        let context = autoformat
            .as_ref()
            .and_then(|_| self.spelling_context_value());
        let prefix = context.as_ref().and_then(|context| {
            let text = context["text"].as_str()?;
            let start = context["anchorOffset"]
                .as_u64()?
                .min(context["focusOffset"].as_u64()?) as usize;
            if let Some(surface) = context["surface"].as_u64() {
                let state = self.state.borrow();
                let document = state.word_doc.as_ref()?;
                let position = document
                    .rendered_to_model_text_position(TextPosition::new(surface as usize, start));
                let model_text: String = document
                    .paragraph(position.paragraph)?
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect();
                return model_text.get(..position.offset).map(str::to_owned);
            }
            text.get(..start).map(str::to_owned)
        });
        let language = autoformat.as_ref().map(|(language, _)| {
            if language.is_empty() {
                context
                    .as_ref()
                    .and_then(|c| c["language"].as_str())
                    .unwrap_or("")
                    .to_owned()
            } else {
                language.clone()
            }
        });
        let list_start = prefix
            .as_deref()
            .and_then(|prefix| crate::typed_autoformat::list_start(prefix, &text));
        let text = match (autoformat.as_ref(), prefix.as_deref()) {
            (Some((_, capitalization)), Some(prefix)) => {
                let language = language.as_deref().unwrap_or("");
                crate::typed_autoformat::format_text(prefix, &text, language, capitalization)
            }
            _ => text,
        };

        // Route to header/footer editing if in hf mode
        {
            let editing_mode = self.state.borrow().editing_mode;
            if editing_mode != EditingMode::Body {
                return self.insert_hf_text(text, editing_mode).await;
            }
        }

        self.clear_composition_preview()?;

        let selected_text_box_caret = {
            let state_ref = self.state.borrow();
            state_ref.object_selection.and_then(|target| {
                if target.kind != FloatingObjectKind::TextBox {
                    return None;
                }
                state_ref.word_doc.as_ref().and_then(|doc| {
                    state_ref
                        .selection
                        .filter(|range| {
                            let (start, end) = range.normalized();
                            doc.text_box_contains_paragraph(target, start.paragraph)
                                && doc.text_box_contains_paragraph(target, end.paragraph)
                        })
                        .or_else(|| {
                            doc.last_text_box_position_for_target(target)
                                .map(SelectionRange::collapsed)
                        })
                })
            })
        };
        let editing_text_box = selected_text_box_caret.is_some();
        let selection_before = selected_text_box_caret.unwrap_or_else(|| {
            let state_ref = self.state.borrow();
            state_ref.selection.unwrap_or_else(|| {
                let renderer = self.renderer.borrow();
                renderer
                    .selection_range()
                    .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
            })
        });
        let edited_paragraph_index = selection_before.focus.paragraph;
        let dirty_from = {
            let (start, _) = selection_before.normalized();
            start.paragraph
        };
        let old_height = {
            let renderer = self.renderer.borrow();
            renderer.cached_paragraph_height(edited_paragraph_index)
        };

        let inline_properties = {
            let mut state_ref = self.state.borrow_mut();
            state_ref
                .inline_formatting
                .take_pending_for(&selection_before)
        };
        let (mut selection_start, selection_end) = selection_before.normalized();
        let list_start = list_start.filter(|_| {
            selection_start == selection_end
                && self
                    .state
                    .borrow()
                    .word_doc
                    .as_ref()
                    .and_then(|doc| doc.paragraph(selection_start.paragraph))
                    .is_some_and(|p| {
                        p.properties
                            .as_ref()
                            .and_then(|p| p.numbering.as_ref())
                            .is_none()
                    })
        });
        let grouped_autoformat = list_start.is_some();
        let text = if let Some((_, remainder)) = list_start.as_ref() {
            selection_start.offset = 0;
            let (_, capitalization) = autoformat.as_ref().expect("typed list options");
            crate::typed_autoformat::format_text(
                "",
                remainder,
                language.as_deref().unwrap_or(""),
                capitalization,
            )
        } else {
            text
        };
        // The armed-format commands below select the range they format, like
        // their ribbon counterparts. Typing must leave the caret collapsed
        // after the new text (Word), or the next command and keystroke would
        // act on — and replace — what was just typed.
        let caret_after_formatted_insert =
            (inline_properties.is_some() && !text.is_empty()).then(|| {
                SelectionRange::collapsed(TextPosition::new(
                    selection_start.paragraph,
                    selection_start.offset.saturating_add(text.len()),
                ))
            });
        // Route text edits through the headless command contract so UI and CLI use identical logic.
        let mut commands = vec![HeadlessCommand::ReplaceRange {
            start_paragraph: selection_start.paragraph,
            start_offset: selection_start.offset,
            end_paragraph: selection_end.paragraph,
            end_offset: selection_end.offset,
            text: text.clone(),
        }];
        if let Some((preset, _)) = list_start {
            commands.push(HeadlessCommand::ApplyMultilevelList {
                start_paragraph: selection_start.paragraph,
                end_paragraph: selection_start.paragraph,
                preset,
                level: 0,
            });
        }
        if !text.is_empty() {
            let insert_end = selection_start.offset.saturating_add(text.len());
            if let Some(props) = inline_properties.as_ref() {
                // Pending formatting carries explicit OFF states too (Word:
                // toggle bold off at the end of bold text, then type — the
                // new text must NOT inherit bold from the run it splits), so
                // every Some(..) maps to a command, not just the enabling
                // ones.
                if let Some(enabled) = props.bold {
                    commands.push(HeadlessCommand::ApplyBold {
                        start_paragraph: selection_start.paragraph,
                        start_offset: selection_start.offset,
                        end_paragraph: selection_start.paragraph,
                        end_offset: insert_end,
                        enabled,
                    });
                }
                if let Some(enabled) = props.italic {
                    commands.push(HeadlessCommand::ApplyItalic {
                        start_paragraph: selection_start.paragraph,
                        start_offset: selection_start.offset,
                        end_paragraph: selection_start.paragraph,
                        end_offset: insert_end,
                        enabled,
                    });
                }
                if let Some(enabled) = props.underline {
                    commands.push(HeadlessCommand::ApplyUnderline {
                        start_paragraph: selection_start.paragraph,
                        start_offset: selection_start.offset,
                        end_paragraph: selection_start.paragraph,
                        end_offset: insert_end,
                        kind: enabled
                            .then(|| props.underline_kind.as_deref().map(str::to_owned))
                            .flatten(),
                        enabled,
                    });
                }
                if props.strike.is_some() || props.double_strike.is_some() {
                    let enabled = props.strike == Some(true) || props.double_strike == Some(true);
                    commands.push(HeadlessCommand::ApplyStrike {
                        start_paragraph: selection_start.paragraph,
                        start_offset: selection_start.offset,
                        end_paragraph: selection_start.paragraph,
                        end_offset: insert_end,
                        double: props.double_strike == Some(true),
                        enabled,
                    });
                }
                match props.baseline.as_deref() {
                    Some("superscript") => {
                        commands.push(HeadlessCommand::ApplySuperscript {
                            start_paragraph: selection_start.paragraph,
                            start_offset: selection_start.offset,
                            end_paragraph: selection_start.paragraph,
                            end_offset: insert_end,
                            enabled: true,
                        });
                    }
                    Some("subscript") => {
                        commands.push(HeadlessCommand::ApplySubscript {
                            start_paragraph: selection_start.paragraph,
                            start_offset: selection_start.offset,
                            end_paragraph: selection_start.paragraph,
                            end_offset: insert_end,
                            enabled: true,
                        });
                    }
                    // The explicit off state (`"baseline"`): clear whichever
                    // vertical alignment the split run would hand down.
                    Some(_) => {
                        commands.push(HeadlessCommand::ApplySuperscript {
                            start_paragraph: selection_start.paragraph,
                            start_offset: selection_start.offset,
                            end_paragraph: selection_start.paragraph,
                            end_offset: insert_end,
                            enabled: false,
                        });
                        commands.push(HeadlessCommand::ApplySubscript {
                            start_paragraph: selection_start.paragraph,
                            start_offset: selection_start.offset,
                            end_paragraph: selection_start.paragraph,
                            end_offset: insert_end,
                            enabled: false,
                        });
                    }
                    None => {}
                }
                if let Some(font_size) = props.font_size {
                    commands.push(HeadlessCommand::SetFontSize {
                        start_paragraph: selection_start.paragraph,
                        start_offset: selection_start.offset,
                        end_paragraph: selection_start.paragraph,
                        end_offset: insert_end,
                        font_size,
                    });
                }
                if let Some(font_family) = props.font_family.as_deref().map(str::to_string) {
                    commands.push(HeadlessCommand::SetFontFamily {
                        start_paragraph: selection_start.paragraph,
                        start_offset: selection_start.offset,
                        end_paragraph: selection_start.paragraph,
                        end_offset: insert_end,
                        font_family,
                    });
                }
            }
        }

        // Drop the renderer's document cache before mutating.
        // This allows Rc::make_mut to avoid cloning when ref_count becomes 1.
        let _document_cache = self.drop_renderer_document_cache();

        let (options, numbering_part, header_lines, footer_lines, new_selection) = {
            let mut state = self.state.borrow_mut();
            let mut word_doc = state
                .word_doc
                .take()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;

            state.selection = Some(selection_before);
            {
                // Reuse the same executor to keep undo/redo history coherent in the interactive UI.
                let request = HeadlessEditRequest { commands };
                let mut command_executor = std::mem::take(&mut state.command_executor);
                if grouped_autoformat {
                    command_executor.begin_group();
                }
                let apply_result = apply_headless_commands_with_executor_to_document(
                    &mut word_doc,
                    &mut command_executor,
                    state.selection,
                    &request,
                );
                if grouped_autoformat {
                    if apply_result.is_ok() {
                        command_executor.end_group();
                    } else {
                        let mut selection = state.selection;
                        command_executor
                            .abort_group(&mut CommandContext::new(&mut word_doc, &mut selection));
                        state.selection = selection;
                    }
                }
                state.command_executor = command_executor;
                match apply_result {
                    Ok(selection_after) => {
                        state.selection = caret_after_formatted_insert.or(selection_after)
                    }
                    Err(err) => {
                        // Put the document back before bailing out — `word_doc`
                        // was taken out of the state above, so returning early
                        // without restoring it would wipe the loaded document
                        // from the editor on any failed command.
                        state.word_doc = Some(word_doc);
                        return Err(JsValue::from_str(&err.to_string()));
                    }
                }
            }

            state.paged_images.clear();

            let options = state.options.clone();
            let numbering_part = word_doc.numbering_part.clone();
            let header_lines = word_doc.header_lines.clone();
            let footer_lines = word_doc.footer_lines.clone();
            let new_selection = state.selection;

            state.word_doc = Some(word_doc);

            (
                options,
                numbering_part,
                header_lines,
                footer_lines,
                new_selection,
            )
        };

        let anchor_page = {
            let renderer = self.renderer.borrow();
            renderer
                .cached_paragraph_start_page(edited_paragraph_index)
                .or_else(|| renderer.visible_page_range().as_ref().map(|r| *r.end()))
                .or_else(|| renderer.layout_page_limit())
                .unwrap_or(1)
        };
        {
            let mut renderer = self.renderer.borrow_mut();
            // IMPORTANT (correctness):
            // Text insertion can change line wrapping and paragraph height, which can shift *all*
            // downstream content on the same page (and potentially subsequent pages).
            //
            // If we only mark the edited paragraph rect dirty, partial bitmap updates can leave
            // stale pixels below the paragraph, causing "content/page appears/disappears" glitches.
            //
            // Mark from the edited paragraph onward so the visible page range gets a full refresh.
            //
            // In lazy/virtualized mode, bound this to a small page window to avoid dirtying far
            // pages that are not currently laid out / visible.
            let lazy_mode = self.state.borrow().lazy_mode;
            // Replacing a selection across paragraphs removes the paragraphs after the
            // first: everything below moves up even when the merged paragraph keeps the
            // old one's geometry, which is all band redraw checks before keeping the
            // pixels under the band.
            if selection_start.paragraph != selection_end.paragraph {
                renderer.set_band_redraw_suppressed_for_structural_edit(true);
            }
            if lazy_mode {
                // IMPORTANT (2026-03 late-page editing regression):
                //
                // Lazy/virtualized editing must not reuse the normal unchecked incremental
                // text-edit replay path here.
                //
                // Why:
                // - `set_incremental_dirty_from()` feeds `forced_dirty_from`, which then
                //   participates in prefix replay / seek restoration inside the renderer.
                // - During ordinary editing this is fast, but in lazy mode the renderer is
                //   simultaneously scoped to a moving visible-page window.
                // - When the edited paragraph is near the top of a page and the previous page
                //   is still in the viewport, the dirty anchor can be stale relative to the
                //   current visible-page window.
                // - The result is that `render_seek` re-enters from an older cached paragraph
                //   boundary (`forced=Some(...) seek=Some(...)`) and the page being edited can
                //   appear to "jump", partially collapse, or momentarily show pixels from the
                //   wrong logical state while typing.
                //
                // The safe policy for lazy text insertion is therefore:
                // - seek only to the first visible page, never an arbitrary dirty anchor
                // - dirty from the edited paragraph to the end of the visible edit window
                // - retain completed downstream layout only until the edited paragraph's full
                //   cached page extent has been revalidated; otherwise invalidate it normally
                //
                // Fast path: prefer seeking to the top of the first visible page so a
                // keystroke deep in a large document does not re-render every paragraph
                // from the document start (the cause of editing getting progressively
                // slower as you scroll). This seek only ever targets the first *visible*
                // page top (never skipping a visible page), so it does not reintroduce the
                // jump/blank bug above; when no valid page-start snapshot exists the
                // renderer transparently falls back to the render-from-start policy.
                renderer.prefer_visible_window_seek_once();
                renderer.set_incremental_dirty_from(dirty_from);
                let retained_completed_pagination = !editing_text_box
                    && selection_start.paragraph == selection_end.paragraph
                    && dirty_from == edited_paragraph_index
                    && old_height.is_some_and(|height| {
                        renderer.prepare_completed_pagination_edit(edited_paragraph_index, height)
                    });
                if !retained_completed_pagination {
                    renderer.invalidate_layout_from(dirty_from);
                }
            } else {
                renderer.set_incremental_dirty_from(dirty_from);
            }
            renderer.invalidate_paragraph_wrap_cache_from(dirty_from);
            if lazy_mode {
                let end_page_1idx = Self::lazy_visible_end(&renderer, anchor_page);
                renderer.mark_dirty_from_paragraph_to_end_bounded(dirty_from, end_page_1idx);
            } else {
                renderer.mark_dirty_from_paragraph_to_end(dirty_from);
            }
        }
        if !self.state.borrow().lazy_mode {
            // Fast replay is still valuable for the non-lazy path where the renderer owns the
            // full page sequence and does not have to reconcile against a viewport-limited draw
            // window. A stable edit stops the walk after the edited paragraph, so it has to keep
            // the completed pagination; lazy mode arms the same retention above. Only for an
            // edit inside one paragraph, as there: replacing a selection across paragraphs
            // removes paragraphs, so the layout after the edited one is not the old layout
            // even when the merged paragraph happens to keep the old height.
            if let Some(old_height) =
                old_height.filter(|_| selection_start.paragraph == selection_end.paragraph)
            {
                let mut renderer = self.renderer.borrow_mut();
                renderer
                    .arm_fast_replay_for_complete_layout_edit(edited_paragraph_index, old_height);
            }
        }

        {
            let state_ref = self.state.borrow();
            if let Some(word_doc) = state_ref.word_doc.as_ref() {
                if let Some(document) = word_doc.document.as_ref() {
                    let mut renderer = self.renderer.borrow_mut();
                    // IMPORTANT (performance):
                    // Do not force `layout_page_limit = None` during edits.
                    //
                    // In lazy/virtualized flows, the host intentionally keeps a page limit
                    // (set via `load_docx_lazy_until` / `ensureLayoutThroughPage`) so that
                    // interactive edits can update only the currently laid-out region quickly.
                    // Full document layout (accurate pageCount) can be progressed later by
                    // calling `ensureLayoutThroughPage` in the background/on-demand.
                    //
                    // Respect host-provided visible range if configured (viewport virtualization).
                    // Ensure the edited page stays visible even if the range is stale.
                    let lazy_mode = self.state.borrow().lazy_mode;
                    Self::ensure_visible_page_range_for_edit(&mut renderer, lazy_mode, anchor_page);
                    // Also clamp layout breadth during the edit render to a small window around the edited
                    // paragraph's current page. This avoids re-laying out far-ahead pages while typing.
                    let prev_limit: Option<Option<usize>> = if self.state.borrow().lazy_mode {
                        let desired = Self::lazy_visible_end(&renderer, anchor_page);
                        let prev = renderer.layout_page_limit();
                        // Clamp layout work to the viewport window during edits so large documents
                        // do not force a full layout pass while typing.
                        renderer.set_layout_page_limit(Some(desired));
                        Some(prev)
                    } else {
                        None
                    };
                    // Use _rc variant to avoid cloning the document (Rc is already shared).
                    renderer.render_with_numbering_rc(
                        Rc::clone(document),
                        &options,
                        numbering_part.as_ref(),
                        header_lines.as_deref(),
                        footer_lines.as_deref(),
                        Some(&word_doc.comments),
                    )?;
                    if let Some(prev) = prev_limit {
                        renderer.set_layout_page_limit(prev);
                    }
                }
            }
        }

        if let Some(range) = new_selection {
            let mut renderer = self.renderer.borrow_mut();
            renderer.set_selection_range(Some(range));
        }
        let caret_metrics = {
            let renderer = self.renderer.borrow();
            renderer.caret_metrics()
        };
        let selection_after = new_selection;

        let lazy_mode = self.state.borrow().lazy_mode;
        // In lazy mode, always skip snapshotting to avoid expensive bitmap generation.
        // The main thread will fetch bitmaps on-demand via getPageBitmap for visible pages only.
        // This dramatically improves edit performance for large documents.
        let skip_snapshots = lazy_mode;
        let dirty_regions_js = if !skip_snapshots {
            let (snapshots, dirty_regions) = self.capture_pages_after_render_with_dirty().await?;
            self.store_paged_images(snapshots);
            dirty_regions
        } else {
            // IMPORTANT (stability):
            // Rendering can schedule async image draws (promises). If we skip snapshotting and
            // return immediately, those in-flight draws can accumulate across rapid edits/undos,
            // making subsequent operations progressively slower.
            self.wait_for_pending_images().await?;

            // Keep state (e.g. table resize metadata) updated and clear dirty markers.
            let (metadata, dirty_regions_js) = {
                let renderer_mut = self.renderer.borrow_mut();
                renderer_mut.coalesce_dirty_regions();
                let dirty_regions = renderer_mut.dirty_regions();
                let dirty_regions_js = Self::dirty_regions_to_js(&dirty_regions);
                let meta = renderer_mut.table_resize_metadata();
                renderer_mut.clear_dirty_regions();
                (meta, dirty_regions_js)
            };
            self.state.borrow_mut().table_resize = metadata;
            dirty_regions_js
        };
        let (preview_mode, history_flags) = {
            let mut state_mut = self.state.borrow_mut();
            state_mut.selection = selection_after;
            state_mut.object_selection = None;
            state_mut.caret_goal_x = None;
            let flags = state_mut.history_flags();
            (state_mut.preview_mode, flags)
        };

        let size = self.canvas_size()?;
        let pages = self.paged_images();
        let caret_value = if preview_mode {
            JsValue::NULL
        } else {
            match caret_metrics {
                Some(metrics) => to_value(&metrics).unwrap_or(JsValue::NULL),
                None => JsValue::NULL,
            }
        };

        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &size)?;
        Reflect::set(&result, &JsValue::from_str("pages"), &pages)?;
        Reflect::set(&result, &JsValue::from_str("caret"), &caret_value)?;
        Reflect::set(
            &result,
            &JsValue::from_str("dirtyRegions"),
            &dirty_regions_js,
        )?;
        Self::write_history_flags(&result, history_flags)?;
        self.write_object_selection(&result)?;
        self.write_merge_state(&result)?;
        {
            let renderer_ref = self.renderer.borrow();
            Reflect::set(
                &result,
                &JsValue::from_str("fastReplayStable"),
                &JsValue::from_bool(renderer_ref.last_fast_replay_stable()),
            )?;
            Reflect::set(
                &result,
                &JsValue::from_str("completedPaginationEditCandidate"),
                &JsValue::from_bool(renderer_ref.last_completed_pagination_edit_candidate()),
            )?;
            Reflect::set(
                &result,
                &JsValue::from_str("completedPaginationEditPreserved"),
                &JsValue::from_bool(renderer_ref.last_completed_pagination_edit_preserved()),
            )?;
            // Edit-pixel-reuse signal (gated in the worker by `?edit_pixel_reuse`,
            // default on since PR #711): tell the
            // worker the caret page's full canvas is valid after this edit render
            // so it can skip the redundant bitmap-fetch full repaint. `null` when
            // the render did not fully paint the caret page.
            // A completed-pagination edit that proves paragraph geometry stable
            // stops right after the edited paragraph, exactly like the lazy
            // stable edit: the band paints over the page's retained pixels, so
            // the caret page is as valid as any other band render.
            match renderer_ref.last_edit_full_page_valid() {
                None => {
                    Reflect::set(
                        &result,
                        &JsValue::from_str("editRenderedPageIndex"),
                        &JsValue::NULL,
                    )?;
                    Reflect::set(
                        &result,
                        &JsValue::from_str("editRenderKind"),
                        &JsValue::NULL,
                    )?;
                }
                Some((page0, kind)) => {
                    Reflect::set(
                        &result,
                        &JsValue::from_str("editRenderedPageIndex"),
                        &JsValue::from_f64(page0 as f64),
                    )?;
                    Reflect::set(
                        &result,
                        &JsValue::from_str("editRenderKind"),
                        &JsValue::from_str(kind.as_str()),
                    )?;
                }
            }
            Self::write_selection_rects(&result, &renderer_ref, selection_after)?;
        }

        Ok(result.into())
    }

    async fn preview_composition_text_async(&self, text: String) -> Result<JsValue, JsValue> {
        if self.state.borrow().preview_mode || self.state.borrow().editing_mode != EditingMode::Body
        {
            return Ok(JsValue::NULL);
        }

        let fallback_selection = {
            let state_ref = self.state.borrow();
            state_ref.selection.unwrap_or_else(|| {
                let renderer = self.renderer.borrow();
                renderer
                    .selection_range()
                    .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
            })
        };
        let had_preview = self.state.borrow().composition_preview.is_some();
        let _document_cache =
            (had_preview || !text.is_empty()).then(|| self.drop_renderer_document_cache());
        let (
            options,
            numbering_part,
            header_lines,
            footer_lines,
            selection_after,
            dirty_from,
            edited_paragraph_index,
        ) = {
            let mut state = self.state.borrow_mut();
            let base_selection =
                Self::clear_composition_preview_in_state(&mut state)?.unwrap_or(fallback_selection);
            let dirty_from = base_selection.normalized().0.paragraph;
            let edited_paragraph_index = base_selection.focus.paragraph;
            let inline_properties = state.inline_formatting.take_pending_for(&base_selection);

            if !text.is_empty() {
                let mut word_doc = state
                    .word_doc
                    .take()
                    .ok_or_else(|| JsValue::from_str("No document loaded"))?;
                let mut selection = Some(base_selection);
                let mut applied_commands: Vec<Box<dyn Command>> = Vec::new();
                let revision_session = word_doc.revision_session().cloned();
                word_doc.set_revision_session(None);
                let apply_result =
                    (|| -> Result<Option<SelectionRange>, JsValue> {
                        let mut insert_command: Box<dyn Command> = Box::new(
                            InsertTextCommand::new(base_selection, text.clone(), inline_properties),
                        );
                        {
                            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                            insert_command
                                .apply(&mut ctx)
                                .map_err(|err| JsValue::from_str(&err.to_string()))?;
                        }
                        applied_commands.push(insert_command);

                        let selection_after_insert = selection;
                        let (selection_start, _) = base_selection.normalized();
                        let insert_end = selection_start.offset.saturating_add(text.len());
                        if insert_end > selection_start.offset {
                            let underline_range = SelectionRange::new(
                                selection_start,
                                TextPosition::new(selection_start.paragraph, insert_end),
                            );
                            let mut underline_command: Box<dyn Command> =
                                Box::new(ApplyUnderlineCommand::new(underline_range));
                            {
                                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                                underline_command
                                    .apply(&mut ctx)
                                    .map_err(|err| JsValue::from_str(&err.to_string()))?;
                            }
                            applied_commands.push(underline_command);
                            selection = selection_after_insert;
                        }

                        Ok(selection)
                    })();
                word_doc.set_revision_session(revision_session);
                selection = match apply_result {
                    Ok(selection) => selection,
                    Err(err) => {
                        let mut rollback_error = None;
                        {
                            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                            for command in applied_commands.iter_mut().rev() {
                                if let Err(undo_err) = command.undo(&mut ctx) {
                                    rollback_error = Some(undo_err.to_string());
                                    break;
                                }
                            }
                        }
                        state.selection = Some(base_selection);
                        state.word_doc = Some(word_doc);
                        return match rollback_error {
                            Some(rollback_error) => Err(JsValue::from_str(&format!(
                                "{err:?}; composition rollback failed: {rollback_error}"
                            ))),
                            None => Err(err),
                        };
                    }
                };

                state.selection = selection;
                state.composition_preview = Some(CompositionPreviewState {
                    base_selection: Some(base_selection),
                    commands: applied_commands,
                });
                state.word_doc = Some(word_doc);
            } else {
                state.selection = Some(base_selection);
            }

            state.paged_images.clear();
            let word_doc = state
                .word_doc
                .as_ref()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;
            (
                state.options.clone(),
                word_doc.numbering_part.clone(),
                word_doc.header_lines.clone(),
                word_doc.footer_lines.clone(),
                state.selection,
                dirty_from,
                edited_paragraph_index,
            )
        };

        let lazy_mode = self.state.borrow().lazy_mode;
        {
            let mut renderer = self.renderer.borrow_mut();
            let anchor_page = renderer
                .cached_paragraph_start_page(edited_paragraph_index)
                .or_else(|| renderer.visible_page_range().as_ref().map(|r| *r.end()))
                .or_else(|| renderer.layout_page_limit())
                .unwrap_or(1);
            Self::ensure_visible_page_range_for_edit(&mut renderer, lazy_mode, anchor_page);
            if lazy_mode {
                // Each IME update replaces the preview text the same way
                // insert-text does, so seek to the first visible page instead of
                // re-rendering from the document start on every keystroke of a
                // composition. See the insert-text path for why the window seek
                // cannot reintroduce the late-page jump/blank bug.
                renderer.prefer_visible_window_seek_once();
                renderer.set_incremental_dirty_from(dirty_from);
                renderer.invalidate_layout_from(dirty_from);
            } else {
                renderer.set_incremental_dirty_from(dirty_from);
            }
            renderer.invalidate_paragraph_wrap_cache_from(dirty_from);
            if lazy_mode {
                let end_page_1idx = Self::lazy_visible_end(&renderer, anchor_page);
                renderer.mark_dirty_from_paragraph_to_end_bounded(dirty_from, end_page_1idx);
            } else {
                renderer.mark_dirty_from_paragraph_to_end(dirty_from);
            }
            let prev_limit: Option<Option<usize>> = if lazy_mode {
                let desired = Self::lazy_visible_end(&renderer, anchor_page);
                let prev = renderer.layout_page_limit();
                renderer.set_layout_page_limit(Some(desired));
                Some(prev)
            } else {
                None
            };
            // Non-lazy composition previews still capture full pages through the
            // destructive snapshot path below, so they cannot preserve the pixels
            // required by a later band redraw. Lazy previews use the same
            // non-destructive on-demand bitmap path as insertText and can safely
            // retain the caret page between consecutive composition updates.
            renderer.set_band_redraw_suppressed_for_composition(!lazy_mode);
            let state_ref = self.state.borrow();
            let word_doc = state_ref
                .word_doc
                .as_ref()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;
            let render_result = if let Some(document) = word_doc.document.as_ref() {
                renderer.render_with_numbering_rc(
                    Rc::clone(document),
                    &options,
                    numbering_part.as_ref(),
                    header_lines.as_deref(),
                    footer_lines.as_deref(),
                    Some(&word_doc.comments),
                )
            } else {
                Ok(())
            };
            renderer.set_band_redraw_suppressed_for_composition(false);
            if let Some(prev) = prev_limit {
                renderer.set_layout_page_limit(prev);
            }
            render_result?;
            if let Some(range) = selection_after {
                renderer.set_selection_range(Some(range));
            }
        }

        let caret_metrics = {
            let renderer = self.renderer.borrow();
            renderer.caret_metrics()
        };
        // Lazy composition previews preserve the edit-rendered caret-page canvas
        // and let the worker snapshot it non-destructively on demand. This avoids
        // both the full-page eager transfer and the full repaint on every IME
        // update. The runtime gives composition caret-page fetches immediate
        // priority, so the composing glyph still appears without edit debounce.
        let (snapshots, dirty_regions_js) = if lazy_mode {
            self.wait_for_pending_images().await?;
            let (metadata, dirty_regions_js) = {
                let renderer_mut = self.renderer.borrow_mut();
                renderer_mut.coalesce_dirty_regions();
                let dirty_regions = renderer_mut.dirty_regions();
                let dirty_regions_js = Self::dirty_regions_to_js(&dirty_regions);
                let metadata = renderer_mut.table_resize_metadata();
                renderer_mut.clear_dirty_regions();
                (metadata, dirty_regions_js)
            };
            self.state.borrow_mut().table_resize = metadata;
            (None, dirty_regions_js)
        } else {
            self.capture_pages_after_render_with_dirty().await?
        };
        self.store_paged_images(snapshots);

        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &self.canvas_size()?)?;
        Reflect::set(&result, &JsValue::from_str("pages"), &self.paged_images())?;
        let caret_value = match caret_metrics {
            Some(metrics) => to_value(&metrics).unwrap_or(JsValue::NULL),
            None => JsValue::NULL,
        };
        Reflect::set(&result, &JsValue::from_str("caret"), &caret_value)?;
        Reflect::set(
            &result,
            &JsValue::from_str("dirtyRegions"),
            &dirty_regions_js,
        )?;
        Self::write_history_flags(&result, self.history_flags())?;
        self.write_object_selection(&result)?;
        self.write_merge_state(&result)?;
        {
            let renderer_ref = self.renderer.borrow();
            Reflect::set(
                &result,
                &JsValue::from_str("fastReplayStable"),
                &JsValue::from_bool(renderer_ref.last_fast_replay_stable()),
            )?;
            match renderer_ref.last_edit_full_page_valid() {
                Some((page0, kind)) => {
                    Reflect::set(
                        &result,
                        &JsValue::from_str("editRenderedPageIndex"),
                        &JsValue::from_f64(page0 as f64),
                    )?;
                    Reflect::set(
                        &result,
                        &JsValue::from_str("editRenderKind"),
                        &JsValue::from_str(kind.as_str()),
                    )?;
                }
                None => {
                    Reflect::set(
                        &result,
                        &JsValue::from_str("editRenderedPageIndex"),
                        &JsValue::NULL,
                    )?;
                    Reflect::set(
                        &result,
                        &JsValue::from_str("editRenderKind"),
                        &JsValue::NULL,
                    )?;
                }
            }
            Self::write_selection_rects(&result, &renderer_ref, selection_after)?;
        }
        Reflect::set(
            &result,
            &JsValue::from_str("compositionPreview"),
            &JsValue::TRUE,
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("debugMutationKind"),
            &JsValue::from_str("composition-preview"),
        )?;
        Ok(result.into())
    }

    fn history_flags(&self) -> (bool, bool) {
        let state = self.state.borrow();
        state.history_flags()
    }

    fn numbered_paragraph_indices_for_num_id(
        word_doc: &WordDocument,
        from: usize,
        to_inclusive: usize,
        target_num_id: &str,
    ) -> Vec<usize> {
        let Some(end) = word_doc
            .paragraph_count()
            .checked_sub(1)
            .map(|last| to_inclusive.min(last))
        else {
            return Vec::new();
        };
        if from > end {
            return Vec::new();
        }

        (from..=end)
            .filter(|&paragraph_index| {
                word_doc
                    .paragraph(paragraph_index)
                    .and_then(|paragraph| paragraph.properties.as_ref())
                    .and_then(|props| props.numbering.as_ref())
                    .and_then(|numbering| numbering.id.as_deref())
                    == Some(target_num_id)
            })
            .collect()
    }

    fn write_history_flags(result: &Object, flags: (bool, bool)) -> Result<(), JsValue> {
        let (can_undo, can_redo) = flags;
        Reflect::set(
            result,
            &JsValue::from_str("canUndo"),
            &JsValue::from_bool(can_undo),
        )?;
        Reflect::set(
            result,
            &JsValue::from_str("canRedo"),
            &JsValue::from_bool(can_redo),
        )?;
        Ok(())
    }

    async fn perform_history_action_async(
        &self,
        action: HistoryAction,
    ) -> Result<JsValue, JsValue> {
        self.perform_history_action_with_selection_async(action, None)
            .await
    }

    async fn perform_history_action_with_selection_async(
        &self,
        action: HistoryAction,
        restore_selection: Option<SelectionRange>,
    ) -> Result<JsValue, JsValue> {
        if let Some(result) = self.perform_hf_history_action_async(action).await? {
            return Ok(result);
        }

        let (can_undo, can_redo) = self.history_flags();
        if matches!(action, HistoryAction::Undo) && !can_undo
            || matches!(action, HistoryAction::Redo) && !can_redo
        {
            return Ok(self.history_state());
        }
        self.clear_composition_preview()?;

        let (refresh_header_footer_stories, refresh_styles) = {
            let state = self.state.borrow();
            let command_name = match action {
                HistoryAction::Undo => state.command_executor.next_undo_name(),
                HistoryAction::Redo => state.command_executor.next_redo_name(),
            };
            (
                command_name.is_some_and(command_mutates_header_footer_stories),
                command_name.is_some_and(command_mutates_style_definitions),
            )
        };

        let (selection_before, selected_object_before) = {
            let state_ref = self.state.borrow();
            (
                state_ref.selection.unwrap_or_else(|| {
                    let renderer = self.renderer.borrow();
                    renderer
                        .selection_range()
                        .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
                }),
                state_ref.object_selection,
            )
        };

        // Drop the renderer's cached document Rc before the command mutates it.
        // Undo/redo commands reach the document through `paragraph_mut` ->
        // `document_mut` -> `Rc::make_mut`; while the renderer still holds a
        // clone the ref count is 2 and every history action deep-clones the
        // whole `Document`. Releasing the cache here makes `word_doc` the sole
        // owner so `make_mut` is a no-op, mirroring the insert/paragraph-break
        // edit paths. The guard outlives the block so a failed or no-op action
        // leaves the renderer pointed at the document again.
        let _document_cache = self.drop_renderer_document_cache();
        let maybe_context = {
            let mut state_mut = self.state.borrow_mut();
            let mut word_doc = state_mut
                .word_doc
                .take()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;

            state_mut.selection = Some(selection_before);
            state_mut.object_selection = None;
            self.renderer.borrow_mut().clear_object_transform_preview();

            let mut command_executor = std::mem::take(&mut state_mut.command_executor);
            let mut ctx = CommandContext::new(&mut word_doc, &mut state_mut.selection);
            let action_result = match action {
                HistoryAction::Undo => command_executor.undo_with_footprint(&mut ctx),
                HistoryAction::Redo => command_executor.redo_with_footprint(&mut ctx),
            };

            state_mut.command_executor = command_executor;
            let footprint = match action_result {
                Ok(footprint) => footprint,
                Err(err) => {
                    state_mut.word_doc = Some(word_doc);
                    return Err(JsValue::from_str(&err.to_string()));
                }
            };

            match footprint {
                None => {
                    state_mut.word_doc = Some(word_doc);
                    None
                }
                Some(footprint) => {
                    if let Some(selection) = restore_selection {
                        state_mut.selection = Some(selection);
                    }
                    state_mut.paged_images.clear();
                    let options = state_mut.options.clone();
                    let numbering_part = word_doc.numbering_part.clone();
                    let header_lines = word_doc.header_lines.clone();
                    let footer_lines = word_doc.footer_lines.clone();
                    let new_selection = state_mut.selection;
                    state_mut.word_doc = Some(word_doc);
                    Some((
                        options,
                        numbering_part,
                        header_lines,
                        footer_lines,
                        new_selection,
                        footprint,
                    ))
                }
            }
        };

        let Some((options, numbering_part, header_lines, footer_lines, new_selection, footprint)) =
            maybe_context
        else {
            return Ok(self.history_state());
        };

        let lazy_mode = self.state.borrow().lazy_mode;

        // History operations can change layout; ensure we get updated bitmaps for the affected page.
        let dirty_from = {
            let (before_start, _) = selection_before.normalized();
            let mut dirty = before_start.paragraph;
            if let Some(range) = new_selection {
                let (after_start, _) = range.normalized();
                dirty = dirty.min(after_start.paragraph);
            }
            if matches!(footprint, HistoryRenderFootprint::Structural) {
                let renderer = self.renderer.borrow();
                let table_id = renderer
                    .cell_id_for_paragraph(before_start.paragraph)
                    .and_then(|cell_id| renderer.cell_layout(cell_id))
                    .map(|cell| cell.table_id);
                if let Some(table_id) = table_id {
                    // A grouped table edit can affect cells before the caret.
                    // Replaying only from the caret skips the table's opening
                    // cells and leaves its render-time table metadata empty.
                    if let Some(table_start) = renderer
                        .cell_layouts_debug_ref()
                        .iter()
                        .filter(|cell| cell.table_id == table_id)
                        .flat_map(|cell| cell.paragraphs.iter())
                        .min()
                    {
                        dirty = dirty.min(*table_start);
                    }
                }
            }
            dirty
        };

        // Single-paragraph text undo/redo (e.g. undoing an `InsertTextCommand`
        // that touched one paragraph) is layout-equivalent to typing in that
        // paragraph, so route it through the same incremental dirty/seek/band
        // redraw setup that the insert-text edit path uses instead of the
        // full-render `mark_paragraph_dirty` below. Structural history actions
        // (paragraph split/merge, table ops, multi-paragraph range restores)
        // keep the conservative full-render path — their reflow can shift
        // downstream content that a single-paragraph band would miss.
        let incremental = matches!(footprint, HistoryRenderFootprint::SingleParagraph { .. });
        // Typing keeps the layout after the edited paragraph when it comes out
        // with its old geometry, so an edit on a completed layout does not walk
        // to the last page. A restore of one body paragraph that heads the
        // dirty range is the same edit, so it keeps that layout too. Text boxes
        // stay out, as for typing.
        let retained_paragraph = match footprint {
            HistoryRenderFootprint::SingleParagraph { index } if index == dirty_from => self
                .state
                .borrow()
                .word_doc
                .as_ref()
                .is_some_and(|doc| doc.is_body_paragraph_surface(index))
                .then_some(index),
            _ => None,
        };

        {
            let mut renderer = self.renderer.borrow_mut();
            let retained_height = retained_paragraph.and_then(|index| {
                renderer
                    .cached_paragraph_height(index)
                    .map(|height| (index, height))
            });
            // In lazy mode, keep layout_page_limit to avoid full document re-layout.
            // In non-lazy mode, structural history actions compute full layout to
            // keep pageCount accurate. A single-paragraph text undo/redo cannot
            // change the paragraph or page count (it is layout-equivalent to
            // typing), so it keeps the existing layout breadth instead of forcing
            // a full-document re-layout — matching the insert-text edit path,
            // which never sets `layout_page_limit = None`.
            if !lazy_mode && !incremental {
                renderer.set_layout_page_limit(None);
            }
            renderer.set_incremental_dirty_from(dirty_from);
            if incremental {
                // Mirror the insert-text edit path (canvas_view.rs ~L3873):
                // invalidate the wrap cache from the restored paragraph so the
                // seek does not reuse stale wrapping, and dirty from that
                // paragraph to the end of the (visible) page window so band
                // redraw's downstream repaint stays correct — echoing the #702
                // guard that band redraw must not wipe non-edited paragraphs.
                let anchor_page = renderer
                    .cached_paragraph_start_page(dirty_from)
                    .or_else(|| renderer.visible_page_range().as_ref().map(|r| *r.end()))
                    .or_else(|| renderer.layout_page_limit())
                    .unwrap_or(1);
                Self::ensure_visible_page_range_for_edit(&mut renderer, lazy_mode, anchor_page);
                if lazy_mode {
                    // Same visible-window seek as insert-text: a single-paragraph
                    // restore cannot change the paragraph count, so resuming at
                    // the first visible page (never past `dirty_from`) is as safe
                    // as it is for typing and avoids a walk from the document top.
                    renderer.prefer_visible_window_seek_once();
                    let retained_completed_pagination =
                        retained_height.is_some_and(|(index, height)| {
                            renderer.prepare_completed_pagination_edit(index, height)
                        });
                    if !retained_completed_pagination {
                        renderer.invalidate_layout_from(dirty_from);
                    }
                } else if let Some((index, height)) = retained_height {
                    renderer.arm_fast_replay_for_complete_layout_edit(index, height);
                }
                renderer.invalidate_paragraph_wrap_cache_from(dirty_from);
                if lazy_mode {
                    let end_page_1idx = Self::lazy_visible_end(&renderer, anchor_page);
                    renderer.mark_dirty_from_paragraph_to_end_bounded(dirty_from, end_page_1idx);
                } else {
                    renderer.mark_dirty_from_paragraph_to_end(dirty_from);
                }
            } else {
                // A structural history action (paragraph split/merge, table op,
                // multi-paragraph restore) can move every line below the edited
                // paragraph while leaving that paragraph's own geometry intact —
                // undoing a paragraph break merges the empty paragraph back and
                // keeps the merge target's start, page and height. Band redraw
                // would read that as "geometry stable", repaint only the
                // paragraph's own band and keep the pre-undo pixels for the
                // shifted lines below it, so it must stay off for this render.
                renderer.set_band_redraw_suppressed_for_structural_edit(true);
                renderer.mark_paragraph_dirty(dirty_from);
            }
        }

        {
            let state_ref = self.state.borrow();
            if let Some(word_doc) = state_ref.word_doc.as_ref() {
                if let Some(document) = word_doc.document.as_ref() {
                    let mut renderer = self.renderer.borrow_mut();
                    if refresh_styles {
                        renderer.set_style_context(
                            word_doc.styles_part.clone().map(Rc::new),
                            word_doc.loaded_theme_part().map(Rc::new),
                        );
                    }
                    if refresh_header_footer_stories {
                        sync_renderer_header_footer_stories(&mut renderer, word_doc);
                    }
                    renderer.render_with_numbering_rc(
                        Rc::clone(document),
                        &options,
                        numbering_part.as_ref(),
                        header_lines.as_deref(),
                        footer_lines.as_deref(),
                        Some(&word_doc.comments),
                    )?;
                }
            }
        }

        // NOTE: never hold a RefCell borrow across an await.
        let (caret_metrics, selection_after) = {
            let mut renderer = self.renderer.borrow_mut();
            let selection_changed = renderer.set_selection_range(new_selection);
            // Only re-render the body when the restored selection actually has
            // to be painted into the page bitmap. With the JS-side overlay
            // active (the default) the caret and highlight are composited
            // separately, so a full second render pass just to reflect the
            // selection repaints an identical bitmap — pure waste on every
            // undo/redo. Mirror the interactive click-selection path
            // (`set_text_selection_async`): rerender only when the overlay is
            // disabled, or when the overlay data is not yet available.
            let use_overlay = renderer.use_selection_overlay();
            let mut needs_rerender = false;
            if selection_changed && use_overlay {
                if let Some(selection) = renderer.selection_range() {
                    if selection.is_collapsed() {
                        needs_rerender = renderer.caret_metrics().is_none();
                    } else {
                        needs_rerender = renderer.get_selection_rects().is_empty();
                    }
                }
            }
            if selection_changed && (!use_overlay || needs_rerender) {
                renderer.skip_change_detection_once();
                renderer.rerender_with_cached_state()?;
            }
            (renderer.caret_metrics(), renderer.selection_range())
        };

        // In lazy mode, skip snapshot generation - main thread will fetch bitmaps on-demand.
        // Still, we must await any async image draws that were scheduled by the render pass;
        // otherwise, they can accumulate and slow down later edits/undos.
        let (snapshots, dirty_regions) = if lazy_mode {
            self.wait_for_pending_images().await?;
            let (metadata, dirty_regions_js) = {
                let renderer_mut = self.renderer.borrow_mut();
                renderer_mut.coalesce_dirty_regions();
                let dirty_regions = renderer_mut.dirty_regions();
                let dirty_regions_js = Self::dirty_regions_to_js(&dirty_regions);
                let meta = renderer_mut.table_resize_metadata();
                renderer_mut.clear_dirty_regions();
                (meta, dirty_regions_js)
            };
            self.state.borrow_mut().table_resize = metadata;
            (None, dirty_regions_js)
        } else {
            let mut renderer = self.renderer.borrow_mut();
            self.capture_paged_images(&mut renderer)?
        };

        self.store_paged_images(snapshots);

        let retained_object_selection = selected_object_before.and_then(|target| {
            let renderer = self.renderer.borrow();
            renderer.object_region(target).map(|_| target)
        });

        let (preview_mode, history_flags) = {
            let mut state_mut = self.state.borrow_mut();
            state_mut.selection = selection_after;
            state_mut.object_selection = retained_object_selection;
            state_mut.caret_goal_x = None;
            let flags = state_mut.history_flags();
            (state_mut.preview_mode, flags)
        };

        let size = self.canvas_size()?;
        let pages = self.paged_images();
        let caret_value = if preview_mode {
            JsValue::NULL
        } else {
            match caret_metrics {
                Some(metrics) => to_value(&metrics).unwrap_or(JsValue::NULL),
                None => JsValue::NULL,
            }
        };

        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &size)?;
        Reflect::set(&result, &JsValue::from_str("pages"), &pages)?;
        Reflect::set(&result, &JsValue::from_str("caret"), &caret_value)?;
        Reflect::set(&result, &JsValue::from_str("dirtyRegions"), &dirty_regions)?;
        Self::write_history_flags(&result, history_flags)?;
        self.write_object_selection(&result)?;
        self.write_merge_state(&result)?;
        {
            let renderer_ref = self.renderer.borrow();
            match renderer_ref.last_edit_full_page_valid() {
                Some((page0, kind)) => {
                    Reflect::set(
                        &result,
                        &JsValue::from_str("editRenderedPageIndex"),
                        &JsValue::from_f64(page0 as f64),
                    )?;
                    Reflect::set(
                        &result,
                        &JsValue::from_str("editRenderKind"),
                        &JsValue::from_str(kind.as_str()),
                    )?;
                }
                None => {
                    Reflect::set(
                        &result,
                        &JsValue::from_str("editRenderedPageIndex"),
                        &JsValue::NULL,
                    )?;
                    Reflect::set(
                        &result,
                        &JsValue::from_str("editRenderKind"),
                        &JsValue::NULL,
                    )?;
                }
            }
            Self::write_selection_rects(&result, &renderer_ref, selection_after)?;
        }

        Ok(result.into())
    }

    fn apply_inline_style<F, G>(
        &self,
        update_inline: F,
        command_factory: G,
    ) -> Result<JsValue, JsValue>
    where
        F: Fn(&mut InlineFormattingState),
        G: Fn(SelectionRange) -> Box<dyn Command>,
    {
        self.apply_inline_style_with_scope(update_inline, command_factory, false)
    }

    fn apply_inline_style_with_scope<F, G>(
        &self,
        update_inline: F,
        command_factory: G,
        full_document: bool,
    ) -> Result<JsValue, JsValue>
    where
        F: Fn(&mut InlineFormattingState),
        G: Fn(SelectionRange) -> Box<dyn Command>,
    {
        if self.state.borrow().preview_mode || !self.body_commands_allowed() {
            return Ok(JsValue::NULL);
        }
        self.clear_composition_preview()?;

        let selection_before = {
            let state_ref = self.state.borrow();
            state_ref.selection.unwrap_or_else(|| {
                let renderer = self.renderer.borrow();
                renderer
                    .selection_range()
                    .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
            })
        };
        let dirty_from = if full_document {
            0
        } else {
            selection_before
                .anchor
                .paragraph
                .min(selection_before.focus.paragraph)
        };
        let dirty_to = selection_before
            .anchor
            .paragraph
            .max(selection_before.focus.paragraph);

        // Release the renderer's document Rc before the command mutates the
        // model, as the typing and history paths do: while the renderer holds
        // it, the first `paragraph_mut` deep-clones the whole `Document`.
        let _document_cache = self.drop_renderer_document_cache();
        let (options, numbering_part, header_lines, footer_lines, new_selection, refresh_styles) = {
            let mut state = self.state.borrow_mut();
            let mut word_doc = state
                .word_doc
                .take()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;

            state.selection = Some(selection_before);
            let mut command_executor = std::mem::take(&mut state.command_executor);
            let command = command_factory(selection_before);
            let refresh_styles = command_mutates_style_definitions(command.name());
            let command_result = {
                let mut ctx = CommandContext::new(&mut word_doc, &mut state.selection);
                command_executor.execute(command, &mut ctx)
            };
            state.command_executor = command_executor;
            if let Err(err) = command_result {
                state.word_doc = Some(word_doc);
                return Err(JsValue::from_str(&err.to_string()));
            }

            state.paged_images.clear();

            let options = state.options.clone();
            let numbering_part = word_doc.numbering_part.clone();
            let header_lines = word_doc.header_lines.clone();
            let footer_lines = word_doc.footer_lines.clone();
            let new_selection = state.selection;

            state.word_doc = Some(word_doc);

            (
                options,
                numbering_part,
                header_lines,
                footer_lines,
                new_selection,
                refresh_styles,
            )
        };

        let anchor_page = {
            let renderer = self.renderer.borrow();
            renderer
                .cached_paragraph_start_page(dirty_from)
                .or_else(|| renderer.visible_page_range().as_ref().map(|r| *r.end()))
                .or_else(|| renderer.layout_page_limit())
                .unwrap_or(1)
        };

        let mut renderer = self.renderer.borrow_mut();
        if let Some(word_doc) = self.state.borrow().word_doc.as_ref() {
            // Package replacement can import new style IDs. Refresh the
            // renderer cascade before drawing, including history restores.
            if refresh_styles {
                renderer.set_style_context(
                    word_doc.styles_part.clone().map(Rc::new),
                    word_doc.loaded_theme_part().map(Rc::new),
                );
            }
            renderer.set_bookmark_paragraphs(word_doc.bookmark_render_map());
            renderer.set_bookmark_note_targets(word_doc.bookmark_note_targets());
        }
        let lazy_mode = self.state.borrow().lazy_mode;
        // Respect host-provided visible range if configured (viewport virtualization).
        // Ensure the edited page stays visible even if the range is stale.
        Self::ensure_visible_page_range_for_edit(&mut renderer, lazy_mode, anchor_page);
        // Inline style changes can reflow the entire document. In lazy/virtualized mode,
        // keep layout scoped to the visible window to avoid full re-layout during edits.
        let prev_limit: Option<Option<usize>> = if lazy_mode {
            let desired = Self::lazy_visible_end(&renderer, anchor_page);
            let prev = renderer.layout_page_limit();
            renderer.set_layout_page_limit(Some(desired));
            Some(prev)
        } else {
            renderer.set_layout_page_limit(None);
            None
        };

        // Inline commands can change both drawing and metadata hit regions without changing
        // paragraph geometry. Replaying the old paragraph cache here leaves stale hyperlink
        // regions behind after removeHyperlink (and similarly risks stale bookmark/field draws)
        // until a later keypress happens to force a broader render.
        renderer.invalidate_paragraph_render_cache_range(dirty_from, dirty_to);
        renderer.disable_seek_replay_once();
        renderer.skip_change_detection_once();
        renderer.set_incremental_dirty_from(dirty_from);
        for pi in dirty_from..=dirty_to {
            renderer.mark_paragraph_dirty(pi);
        }
        // Do not set a fast replay trigger: equal paragraph height does not imply that inline
        // drawing commands or interactive regions are unchanged.

        {
            let state_ref = self.state.borrow();
            if let Some(word_doc) = state_ref.word_doc.as_ref() {
                if let Some(document) = word_doc.document.as_ref() {
                    renderer.render_with_numbering_rc(
                        Rc::clone(document),
                        &options,
                        numbering_part.as_ref(),
                        header_lines.as_deref(),
                        footer_lines.as_deref(),
                        Some(&word_doc.comments),
                    )?;
                }
            }
        }
        if let Some(prev) = prev_limit {
            renderer.set_layout_page_limit(prev);
        }

        if let Some(range) = new_selection {
            renderer.set_selection_range(Some(range));
        }
        let caret_metrics = renderer.caret_metrics();
        let selection_after = new_selection;
        // In lazy mode, skip snapshotting to avoid expensive bitmap generation.
        let skip_snapshots = self.state.borrow().lazy_mode;
        let (snapshots, dirty_regions) = if skip_snapshots {
            renderer.coalesce_dirty_regions();
            let dirty_regions = renderer.dirty_regions();
            let dirty_regions_js = Self::dirty_regions_to_js(&dirty_regions);
            renderer.clear_dirty_regions();
            (None, dirty_regions_js)
        } else if full_document {
            // Package imports can introduce uncached images. The async caller
            // waits for decoding before transferring these page canvases.
            (None, Array::new().into())
        } else {
            self.capture_paged_images(&mut renderer)?
        };
        drop(renderer);

        self.store_paged_images(snapshots);
        let (preview_mode, history_flags) = {
            let mut state_mut = self.state.borrow_mut();
            state_mut.selection = selection_after;
            state_mut.caret_goal_x = None;
            update_inline(&mut state_mut.inline_formatting);
            // The formatting was applied to a real range; keep the toolbar
            // state but do not arm it as pending caret formatting.
            state_mut.inline_formatting.armed_at = None;
            let flags = state_mut.history_flags();
            (state_mut.preview_mode, flags)
        };

        let size = self.canvas_size()?;
        let pages = self.paged_images();
        let caret_value = if preview_mode {
            JsValue::NULL
        } else {
            match caret_metrics {
                Some(metrics) => to_value(&metrics).unwrap_or(JsValue::NULL),
                None => JsValue::NULL,
            }
        };

        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &size)?;
        Reflect::set(&result, &JsValue::from_str("pages"), &pages)?;
        Reflect::set(&result, &JsValue::from_str("caret"), &caret_value)?;
        Reflect::set(&result, &JsValue::from_str("dirtyRegions"), &dirty_regions)?;
        Self::write_history_flags(&result, history_flags)?;
        self.write_object_selection(&result)?;
        self.write_merge_state(&result)?;
        {
            let renderer_ref = self.renderer.borrow();
            Self::write_selection_rects(&result, &renderer_ref, selection_after)?;
        }

        Ok(result.into())
    }

    fn apply_inline_style_headless<F, G>(
        &self,
        update_inline: F,
        command_factory: G,
    ) -> Result<JsValue, JsValue>
    where
        F: Fn(&mut InlineFormattingState),
        G: Fn(SelectionRange) -> HeadlessCommand,
    {
        if self.state.borrow().preview_mode {
            return Ok(JsValue::NULL);
        }
        if !self.body_commands_allowed() {
            // Header/footer stories: character formatting applies to their
            // selection or caret line. Commands the
            // story cannot take are no-ops rather than body edits.
            let probe = command_factory(SelectionRange::collapsed(TextPosition::default()));
            if let HeadlessCommand::AdjustFontSize {
                direction, step, ..
            } = probe
            {
                return self.adjust_hf_font_size(direction, step);
            }
            if let HeadlessCommand::ChangeCase { mode, .. } = probe {
                return self.change_hf_case(mode);
            }
            return match Self::hf_run_format_for_command(&probe) {
                Some(format) => self.apply_hf_run_format(format),
                None => Ok(JsValue::NULL),
            };
        }

        let selection_before = {
            let state_ref = self.state.borrow();
            state_ref.selection.unwrap_or_else(|| {
                let renderer = self.renderer.borrow();
                renderer
                    .selection_range()
                    .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
            })
        };
        let dirty_from = selection_before
            .anchor
            .paragraph
            .min(selection_before.focus.paragraph);
        let dirty_to = selection_before
            .anchor
            .paragraph
            .max(selection_before.focus.paragraph);

        if selection_before.is_collapsed() {
            {
                let mut state_mut = self.state.borrow_mut();
                update_inline(&mut state_mut.inline_formatting);
                // Arm the pending formatting at this caret; it dies as soon as
                // the caret moves, matching Word.
                state_mut.inline_formatting.armed_at = Some(selection_before.focus);
                state_mut.selection = Some(selection_before);
            }

            let caret_metrics = self.renderer.borrow().caret_metrics();
            let result = Object::new();
            if let Some(metrics) = caret_metrics {
                let caret_value =
                    to_value(&metrics).map_err(|e| JsValue::from_str(&format!("{e}")))?;
                Reflect::set(&result, &JsValue::from_str("caret"), &caret_value)?;
            }
            Self::write_history_flags(&result, self.history_flags())?;
            self.write_object_selection(&result)?;
            return Ok(result.into());
        }

        // See `apply_inline_style_with_scope`.
        let _document_cache = self.drop_renderer_document_cache();
        let (options, numbering_part, header_lines, footer_lines, new_selection) = {
            let mut state = self.state.borrow_mut();
            let mut word_doc = state
                .word_doc
                .take()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;

            state.selection = Some(selection_before);
            {
                // Delete is routed through headless API so caret deletion behaves the same as CLI.
                let request = HeadlessEditRequest {
                    commands: vec![command_factory(selection_before)],
                };
                let mut command_executor = std::mem::take(&mut state.command_executor);
                let apply_result = apply_headless_commands_with_executor_to_document(
                    &mut word_doc,
                    &mut command_executor,
                    state.selection,
                    &request,
                );
                state.command_executor = command_executor;
                match apply_result {
                    Ok(selection_after) => state.selection = selection_after,
                    Err(err) => {
                        // Put the document back before bailing out — `word_doc`
                        // was taken out of the state above, so returning early
                        // without restoring it would wipe the loaded document
                        // from the editor on any failed command.
                        state.word_doc = Some(word_doc);
                        return Err(JsValue::from_str(&err.to_string()));
                    }
                }
            }

            state.paged_images.clear();

            let options = state.options.clone();
            let numbering_part = word_doc.numbering_part.clone();
            let header_lines = word_doc.header_lines.clone();
            let footer_lines = word_doc.footer_lines.clone();
            let new_selection = state.selection;

            state.word_doc = Some(word_doc);

            (
                options,
                numbering_part,
                header_lines,
                footer_lines,
                new_selection,
            )
        };

        let anchor_page = {
            let renderer = self.renderer.borrow();
            renderer
                .cached_paragraph_start_page(dirty_from)
                .or_else(|| renderer.visible_page_range().as_ref().map(|r| *r.end()))
                .or_else(|| renderer.layout_page_limit())
                .unwrap_or(1)
        };

        let mut renderer = self.renderer.borrow_mut();
        let lazy_mode = self.state.borrow().lazy_mode;
        Self::ensure_visible_page_range_for_edit(&mut renderer, lazy_mode, anchor_page);
        let prev_limit: Option<Option<usize>> = if lazy_mode {
            let desired = Self::lazy_visible_end(&renderer, anchor_page);
            let prev = renderer.layout_page_limit();
            renderer.set_layout_page_limit(Some(desired));
            Some(prev)
        } else {
            renderer.set_layout_page_limit(None);
            None
        };

        // Inline style commands can change drawing without changing paragraph geometry.
        // Invalidate affected draw caches so fully-selected middle paragraphs repaint using
        // the updated run properties instead of replaying stale draw commands. The wrap
        // cache is content-addressed and includes drawing-only properties (color/highlight/
        // underline/strike/baseline) in its hash, so inline-style changes miss naturally.
        renderer.invalidate_paragraph_render_cache_range(dirty_from, dirty_to);
        // Inline-style updates must render from the real dirty span rather than re-entering from
        // a cached seek boundary or relying on content-hash change detection.
        renderer.disable_seek_replay_once();
        renderer.skip_change_detection_once();
        renderer.set_incremental_dirty_from(dirty_from);
        for pi in dirty_from..=dirty_to {
            renderer.mark_paragraph_dirty(pi);
        }
        // Do not set fast replay trigger for inline-style operations. It can stop redraw early
        // at the focus paragraph and leave later affected content unpainted.

        {
            let state_ref = self.state.borrow();
            if let Some(word_doc) = state_ref.word_doc.as_ref() {
                if let Some(document) = word_doc.document.as_ref() {
                    renderer.render_with_numbering_rc(
                        Rc::clone(document),
                        &options,
                        numbering_part.as_ref(),
                        header_lines.as_deref(),
                        footer_lines.as_deref(),
                        Some(&word_doc.comments),
                    )?;
                }
            }
        }

        if let Some(prev) = prev_limit {
            renderer.set_layout_page_limit(prev);
        }

        if let Some(range) = new_selection {
            renderer.set_selection_range(Some(range));
        }
        let caret_metrics = renderer.caret_metrics();
        let selection_after = new_selection;
        let (snapshots, dirty_regions) = self.capture_paged_images(&mut renderer)?;
        drop(renderer);

        self.store_paged_images(snapshots);

        let (preview_mode, history_flags) = {
            let mut state_mut = self.state.borrow_mut();
            state_mut.selection = selection_after;
            state_mut.object_selection = None;
            state_mut.caret_goal_x = None;
            let flags = state_mut.history_flags();
            (state_mut.preview_mode, flags)
        };

        let size = self.canvas_size()?;
        let pages = self.paged_images();
        let caret_value = if preview_mode {
            JsValue::NULL
        } else {
            match caret_metrics {
                Some(metrics) => to_value(&metrics).unwrap_or(JsValue::NULL),
                None => JsValue::NULL,
            }
        };

        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &size)?;
        Reflect::set(&result, &JsValue::from_str("pages"), &pages)?;
        Reflect::set(&result, &JsValue::from_str("caret"), &caret_value)?;
        Reflect::set(&result, &JsValue::from_str("dirtyRegions"), &dirty_regions)?;
        Self::write_history_flags(&result, history_flags)?;
        self.write_object_selection(&result)?;
        self.write_merge_state(&result)?;
        {
            let renderer_ref = self.renderer.borrow();
            Self::write_selection_rects(&result, &renderer_ref, selection_after)?;
        }

        Ok(result.into())
    }
}

#[derive(Serialize)]
struct CanvasSize {
    width: f64,
    height: f64,
}

/// Pages painted by a lazy `switchRenderer` when the previous presentation
/// never narrowed the visible window. Matches the lazy-load bootstrap.
const SWITCH_RENDERER_LAZY_BOOTSTRAP_PAGES: usize = 3;

/// Run-length encoded page sizes: (page count, width, height).
fn page_dimension_runs(sizes: impl IntoIterator<Item = (f64, f64)>) -> Vec<(usize, f64, f64)> {
    let mut runs: Vec<(usize, f64, f64)> = Vec::new();
    for (width, height) in sizes {
        if let Some(last) = runs.last_mut() {
            if last.1 == width && last.2 == height {
                last.0 += 1;
                continue;
            }
        }
        runs.push((1, width, height));
    }
    runs
}

/// A single search match returned by [`OffscreenDocumentView::search_text`].
/// Offsets are byte offsets into the paragraph's concatenated run text.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchMatchJs {
    paragraph: usize,
    start_offset: usize,
    end_offset: usize,
}

/// One paragraph style the document defines, for the style gallery.
///
/// The gallery used to be hard-coded to 標準 / 見出し 1〜3. A 規程集 or a
/// contract template carries its own styles (`条`, `号`, `別表`), and a
/// paragraph typed without them lands in Normal — the outline, the table of
/// contents and the numbering all come apart. Reading the styles out of the
/// document is what lets the gallery offer what the file actually has.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ParagraphStyleJs {
    /// `w:styleId`, the token `setParagraphStyle` takes.
    style_id: String,
    /// `w:name` when the file gives one, else the id. This is what Word shows.
    name: String,
    /// The document's default paragraph style (Word の「標準」).
    is_default: bool,
    /// `w:outlineLvl`, 0-based (0 = 見出し 1). `None` for body text, which is
    /// how the gallery tells headings from everything else.
    outline_level: Option<u8>,
}

/// Builds the character-style list. Same shape as the paragraph one minus the
/// outline level, which character styles do not carry.
fn character_style_options(styles: &jade_docx_reader::styles::StylesPart) -> Vec<ParagraphStyleJs> {
    let mut out: Vec<ParagraphStyleJs> = styles
        .character_style_gallery_definitions()
        .map(|definition| ParagraphStyleJs {
            style_id: definition.style_id.clone(),
            name: definition
                .name
                .clone()
                .unwrap_or_else(|| definition.style_id.clone()),
            is_default: definition.is_default,
            outline_level: None,
        })
        .collect();
    // Same reason as the paragraph list: the source is a HashMap.
    out.sort_by(|a, b| {
        b.is_default
            .cmp(&a.is_default)
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

/// Builds the gallery's list from a styles part.
///
/// Split out of the wasm method so the ordering can be tested: the underlying
/// iterator walks a `HashMap`, and without a deterministic sort the gallery
/// reshuffles between calls.
fn paragraph_style_options(styles: &jade_docx_reader::styles::StylesPart) -> Vec<ParagraphStyleJs> {
    let mut out: Vec<ParagraphStyleJs> = styles
        .paragraph_style_definitions()
        .map(|definition| ParagraphStyleJs {
            style_id: definition.style_id.clone(),
            name: definition
                .name
                .clone()
                .unwrap_or_else(|| definition.style_id.clone()),
            is_default: definition.is_default,
            outline_level: definition
                .paragraph_properties
                .as_ref()
                .and_then(|props| props.outline_level)
                // 9 means "body text" in the file format; the gallery wants to
                // treat that the same as absent.
                .filter(|level| *level < 9),
        })
        .collect();
    out.sort_by(|a, b| {
        b.is_default
            .cmp(&a.is_default)
            .then_with(|| match (a.outline_level, b.outline_level) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

/// One tracked change surfaced to the review panel via
/// [`OffscreenDocumentView::list_revisions`].
///
/// Adjacent runs carrying the same mark are merged into a single entry, so the
/// list reads the way a reviewer sees the change rather than the way it is
/// stored. `id` is `None` for a revision the file left unidentified; such a
/// change can only be accepted or rejected together with everything else,
/// because `w:id` is the only handle the accept/reject path can target.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RevisionJs {
    id: Option<String>,
    /// `"inserted"` / `"deleted"` / `"movedFrom"` / `"movedTo"`.
    kind: String,
    author: Option<String>,
    date: Option<String>,
    date_utc: Option<String>,
    paragraph: usize,
    /// Byte range within `paragraph`, matching the offsets used by
    /// `listComments` and `searchText`.
    start_offset: usize,
    end_offset: usize,
    text: String,
    paragraph_mark: bool,
}

impl From<&HeadlessRevisionSnapshot> for RevisionJs {
    fn from(snapshot: &HeadlessRevisionSnapshot) -> Self {
        Self {
            id: snapshot.id.clone(),
            kind: snapshot.kind.clone(),
            author: snapshot.author.clone(),
            date: snapshot.date.clone(),
            date_utc: snapshot.date_utc.clone(),
            paragraph: snapshot.paragraph,
            start_offset: snapshot.start_offset,
            end_offset: snapshot.end_offset,
            text: snapshot.text.clone(),
            paragraph_mark: snapshot.paragraph_mark,
        }
    }
}

/// Options of [`OffscreenDocumentView::decide_revisions`].
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RevisionDecisionOptionsJs {
    /// `"selection"` (the default) or `"all"`.
    scope: Option<String>,
    /// Revision kinds to decide; omitted means every kind.
    kinds: Option<Vec<String>>,
    /// Reviewers to decide; `null` in the list is an unattributed revision.
    authors: Option<Vec<Option<String>>>,
    /// Report the revision after the decided range (Word の「反映して次へ進む」).
    move_to_next: bool,
}

/// What [`OffscreenDocumentView::decide_revisions`] did, attached to its
/// payload as `revisionDecision`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RevisionDecisionJs {
    /// Revisions the scope and filters selected; all of them were decided.
    decided: usize,
    /// The revision to select next when `moveToNext` was requested.
    next: Option<RevisionJs>,
}

/// Picks the selection after a headless command from the edited document.
type SelectionAfterEdit = Box<dyn FnOnce(&WordDocument) -> Option<SelectionRange>>;

/// Adjustments to [`OffscreenDocumentView::apply_paragraph_style_headless_many_with`]
/// for a command whose effect does not start at, or keep, the selection.
#[derive(Default)]
struct HeadlessApplyOverrides {
    /// Re-lay-out from this paragraph when it comes before the selection.
    dirty_from: Option<usize>,
    /// The selection after the command, from the edited document. `None`
    /// from the closure keeps the executor's selection.
    selection_after: Option<SelectionAfterEdit>,
}

/// Maps a selection endpoint from the editor's display space to the model
/// offsets revisions are reported in.
///
/// No Markup and Original drop the hidden side of each revision from layout,
/// so a display offset skips those runs. `after_hidden` decides which side
/// of a hidden run a boundary offset lands on: a selection start moves past
/// it and a selection end stays before it, so a selection never reaches a
/// change the reader could not see at its edge.
fn display_to_model_position(
    doc: &WordDocument,
    position: TextPosition,
    render_changes: bool,
    render_original: bool,
    after_hidden: bool,
) -> TextPosition {
    let position = doc.rendered_to_model_text_position(position);
    if render_changes {
        return position;
    }
    let Some(paragraph) = doc.paragraph(position.paragraph) else {
        return position;
    };
    let target = position.offset;
    let (mut visible, mut model) = (0usize, 0usize);
    for run in &paragraph.runs {
        let len = run.text.len();
        if crate::canvas_renderer::run_is_hidden_revision(run, render_changes, render_original) {
            if visible == target && !after_hidden {
                return TextPosition::new(position.paragraph, model);
            }
            model += len;
            continue;
        }
        if target < visible + len || (target == visible + len && !after_hidden) {
            return TextPosition::new(position.paragraph, model + (target - visible));
        }
        visible += len;
        model += len;
    }
    TextPosition::new(position.paragraph, model)
}

/// The inverse of [`display_to_model_position`]: where a model position
/// shows in the editor's display space. A position inside a run the markup
/// mode hides lands where that run would have been.
fn model_to_display_position(
    doc: &WordDocument,
    position: TextPosition,
    render_changes: bool,
    render_original: bool,
) -> TextPosition {
    let last = doc.paragraph_count().saturating_sub(1);
    let paragraph_index = position.paragraph.min(last);
    let Some(paragraph) = doc.paragraph(paragraph_index) else {
        return TextPosition::default();
    };
    let text_len: usize = paragraph.runs.iter().map(|run| run.text.len()).sum();
    let target = if position.paragraph > last {
        text_len
    } else {
        position.offset.min(text_len)
    };
    let mut visible = 0usize;
    if render_changes {
        visible = target;
    } else {
        let mut model = 0usize;
        for run in &paragraph.runs {
            let run_end = model + run.text.len();
            if crate::canvas_renderer::run_is_hidden_revision(run, render_changes, render_original)
            {
                if target < run_end {
                    break;
                }
            } else if target <= run_end {
                visible += target - model;
                break;
            } else {
                visible += run.text.len();
            }
            model = run_end;
        }
    }
    doc.model_to_rendered_text_position(TextPosition::new(paragraph_index, visible))
}

/// A comment surfaced to the IDE comments panel via
/// [`OffscreenDocumentView::list_comments`].
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CommentJs {
    id: String,
    author: Option<String>,
    text: String,
    resolved: bool,
    date: Option<String>,
    /// Body paragraph index of the comment's first anchor (None if unanchored).
    paragraph: Option<usize>,
    /// Byte range of the first anchor within `paragraph` (for jump-to-select).
    start_offset: Option<usize>,
    end_offset: Option<usize>,
    /// Comment this one replies to, from `commentsEx`. `None` for a thread
    /// root. Without it a three-turn negotiation reads as a flat list and
    /// nobody can tell which remark answers which.
    thread_parent_id: Option<String>,
}

#[wasm_bindgen]
impl OffscreenDocumentView {
    fn capture_paged_images(
        &self,
        renderer: &mut CanvasRenderer,
    ) -> Result<(Option<Vec<JsValue>>, JsValue), JsValue> {
        // Keep table resize metadata in sync for all render paths, including
        // non-lazy snapshot capture. Frontend hit-testing fetches this state
        // asynchronously, so stale values can cause handler-coordinate drift.
        let metadata = renderer.table_resize_metadata();
        self.state.borrow_mut().table_resize = metadata;

        if self.state.borrow().options.pageless() {
            return Ok((None, Array::new().into()));
        }
        // Page-level caching strategy:
        // - Only generate ImageBitmaps for dirty pages.
        // - Clean pages are left untouched on the main thread (canvas retains pixels).
        // NOTE: DirtyRect.page is 0-indexed.
        use crate::snapshot_plan::{filter_pages_by_visible_range, pages_to_snapshot};

        if renderer.needs_full_repaint() {
            let lazy_mode = self.state.borrow().lazy_mode;
            let pages = if lazy_mode {
                let pages_to_snapshot = Self::pages_with_rendered_pixels(
                    renderer,
                    filter_pages_by_visible_range(
                        (0..renderer.page_count()).collect(),
                        renderer.visible_page_range(),
                    ),
                );
                if pages_to_snapshot.is_empty() {
                    None
                } else {
                    renderer.take_paged_snapshots_for_pages(Some(&pages_to_snapshot))?
                }
            } else {
                let pages_to_snapshot =
                    Self::pages_with_rendered_pixels(renderer, 0..renderer.page_count());
                renderer.take_paged_snapshots_for_pages(Some(&pages_to_snapshot))?
            };
            renderer.clear_dirty_regions();
            return Ok((pages, Array::new().into()));
        }

        // Reduce dirty rect count before serializing them into the payload.
        // (This keeps the frontend fast when applying partial bitmap updates.)
        renderer.coalesce_dirty_regions();

        // Capture dirty regions before they are cleared so the main thread can apply
        // partial bitmap updates (clearRect + drawImage crop) instead of redrawing whole pages.
        let dirty_regions = renderer.dirty_regions();
        let dirty_regions_js = {
            let result = Array::new();
            for rect in &dirty_regions {
                let obj = Object::new();
                let _ = Reflect::set(&obj, &JsValue::from_str("x"), &JsValue::from_f64(rect.x));
                let _ = Reflect::set(&obj, &JsValue::from_str("y"), &JsValue::from_f64(rect.y));
                let _ = Reflect::set(
                    &obj,
                    &JsValue::from_str("width"),
                    &JsValue::from_f64(rect.width),
                );
                let _ = Reflect::set(
                    &obj,
                    &JsValue::from_str("height"),
                    &JsValue::from_f64(rect.height),
                );
                let _ = Reflect::set(
                    &obj,
                    &JsValue::from_str("page"),
                    &JsValue::from_f64(rect.page as f64),
                );
                result.push(&obj);
            }
            result.into()
        };

        let page_count = renderer.page_count();
        let mut dirty_pages: Vec<usize> = dirty_regions.into_iter().map(|r| r.page).collect();

        let pages_to_snapshot = pages_to_snapshot(
            renderer.needs_full_repaint(),
            page_count,
            dirty_pages.drain(..),
        );
        let pages_to_snapshot = Self::pages_with_rendered_pixels(
            renderer,
            filter_pages_by_visible_range(pages_to_snapshot, renderer.visible_page_range()),
        );

        if pages_to_snapshot.is_empty() {
            // Even if nothing is snapshotted (e.g. all dirty pages are outside visible range),
            // we clear dirty markers here. Pages will be re-rendered on-demand when requested.
            renderer.clear_dirty_regions();
            return Ok((None, dirty_regions_js));
        }

        let pages = renderer.take_paged_snapshots_for_pages(Some(&pages_to_snapshot))?;
        renderer.clear_dirty_regions();
        Ok((pages, dirty_regions_js))
    }

    /// The pages among `pages` whose canvas holds what the last render
    /// painted. A page outside the keep window is a placeholder — never given
    /// a context, or emptied by an earlier transfer — so snapshotting it
    /// either fails outright or hands the browser a blank page.
    fn pages_with_rendered_pixels(
        renderer: &CanvasRenderer,
        pages: impl IntoIterator<Item = usize>,
    ) -> Vec<usize> {
        pages
            .into_iter()
            .filter(|&page| renderer.page_has_rendered_pixels(page))
            .collect()
    }

    fn store_paged_images(&self, images: Option<Vec<JsValue>>) {
        if let Some(pages) = images {
            self.state.borrow_mut().paged_images = pages;
        }
    }

    #[wasm_bindgen(constructor)]
    pub fn new(
        canvas: OffscreenCanvas,
        options: Option<Options>,
        device_pixel_ratio: f64,
    ) -> Result<OffscreenDocumentView, JsValue> {
        let opts = options.unwrap_or_default();
        let host = OffscreenRenderHost::new(canvas, device_pixel_ratio)?;
        let renderer = Rc::new(RefCell::new(CanvasRenderer::with_host(
            Box::new(host) as BoxedRenderHost,
            opts.pageless(),
        )?));

        let state = Rc::new(RefCell::new(CanvasDocumentState {
            options: opts.clone(),
            field_code_view: Default::default(),
            word_autocorrect: None,
            equation_editor: None,
            word_doc: None,
            paged_images: Vec::new(),
            lazy_mode: false,
            preview_mode: opts.preview_mode(),
            page_export_original_formatting_marks: None,
            page_export_original_raster_scale: None,
            command_executor: CommandExecutor::new(),
            composition_preview: None,
            selection: None,
            object_selection: None,
            inline_formatting: InlineFormattingState::default(),
            caret_goal_x: None,
            table_resize: TableResizeMetadata::default(),
            editing_mode: EditingMode::default(),
            hf_variant: HfVariant::default(),
            hf_edit_cursor: None,
            hf_body_selection: None,
            hf_selection_anchor: None,
            hf_undo_stack: Vec::new(),
            hf_redo_stack: Vec::new(),
            hf_history_revision: 0,
            hf_provisional_recording: false,
        }));

        Ok(OffscreenDocumentView {
            renderer,
            state,
            drag_state: Rc::new(RefCell::new(PointerDragState::None)),
            table_resize_preview: Rc::new(Cell::new(false)),
            style_preview: Rc::new(Cell::new(None)),
            hf_format_preview: Rc::new(RefCell::new(None)),
            resize_debug: Rc::new(RefCell::new(None)),
            pending_image_wait: Rc::new(RefCell::new(None)),
        })
    }

    /// Register before loading the document so pagination and bitmap snapshots
    /// never retain metrics from a previous face.
    #[cfg(feature = "opentype-math")]
    #[wasm_bindgen(js_name = registerMathFont)]
    pub fn register_math_font(&self, family: String, bytes: Vec<u8>) -> Result<(), JsValue> {
        if self.state.borrow().word_doc.is_some() {
            return Err(JsValue::from_str(
                "Register MATH fonts before loading a document",
            ));
        }
        self.renderer
            .borrow_mut()
            .register_math_font(family, bytes)
            .map_err(|e| JsValue::from_str(&format!("MATH font registration: {e:?}")))
    }

    #[cfg(feature = "opentype-math")]
    #[wasm_bindgen(js_name = mathFontStats)]
    pub fn math_font_stats(&self) -> Result<JsValue, JsValue> {
        self.renderer
            .borrow()
            .math_font_stats()
            .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    #[wasm_bindgen]
    pub fn can_undo(&self) -> bool {
        self.history_flags().0
    }

    #[wasm_bindgen]
    pub fn can_redo(&self) -> bool {
        self.history_flags().1
    }

    #[wasm_bindgen]
    pub fn history_state(&self) -> JsValue {
        let flags = self.history_flags();
        let result = Object::new();
        if let Err(err) = Self::write_history_flags(&result, flags) {
            web_sys::console::error_1(&err);
        }
        if let Err(err) = self.write_object_selection(&result) {
            web_sys::console::error_1(&err);
        }
        result.into()
    }

    #[wasm_bindgen]
    pub fn undo(&self) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            let ended = JsFuture::from(view.end_style_preview()).await?;
            if !view.history_flags().0 && ended.is_object() {
                return Ok(ended);
            }
            let protection_history = {
                let state = view.state.borrow();
                state.command_executor.next_undo_name() == Some("set_document_protection")
            };
            if view.state.borrow().preview_mode && !protection_history {
                return Ok(view.history_state());
            }
            let payload = view
                .perform_history_action_async(HistoryAction::Undo)
                .await?;
            if protection_history {
                view.sync_preview_mode_from_document_protection();
                Reflect::set(
                    &payload,
                    &JsValue::from_str("documentProtectionChanged"),
                    &JsValue::TRUE,
                )?;
            }
            Ok(payload)
        })
    }

    /// リアルタイムのプレビュー: style `id` on the selection while the pointer
    /// is over it in the gallery — applied as an edit that
    /// [`Self::end_style_preview`] takes back without a trace in the history.
    /// A preview already showing gives way first.
    #[wasm_bindgen(js_name = previewStyle)]
    pub fn preview_style(&self, id: String) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            if view.state.borrow().preview_mode {
                return Ok(JsValue::NULL);
            }
            let ended = JsFuture::from(view.end_style_preview()).await?;
            if view.state.borrow().editing_mode != EditingMode::Body {
                return view.apply_hf_story_preview(ended, || view.set_paragraph_style(Some(id)));
            }
            let before = {
                let mut state = view.state.borrow_mut();
                state.command_executor.set_provisional_recording(true);
                state.command_executor.history_revision()
            };
            let payload = view.run_word_session("word_style_preview", move |session| {
                use crate::word::styles::WordStyles;
                session.apply_style(&id)
            });
            let revision = {
                let mut state = view.state.borrow_mut();
                state.command_executor.set_provisional_recording(false);
                state.command_executor.history_revision()
            };
            match payload {
                Ok(payload) => {
                    if revision != before {
                        view.style_preview.set(Some(FormatPreviewState {
                            revision,
                            selection: None,
                        }));
                    }
                    Ok(payload)
                }
                // A style that cannot go on the selection previews nothing.
                Err(_) => Ok(ended),
            }
        })
    }

    /// Takes the gallery preview back off the document, leaving neither an
    /// undo nor a redo step; `null` when none is showing or anything was
    /// edited since.
    #[wasm_bindgen(js_name = endStylePreview)]
    pub fn end_style_preview(&self) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            let story_preview = view.hf_format_preview.borrow_mut().take();
            if let Some(preview) = story_preview {
                {
                    let mut state = view.state.borrow_mut();
                    if state.hf_history_revision != preview.revision
                        || state.command_executor.history_revision() != preview.body_revision
                    {
                        return Ok(JsValue::NULL);
                    }
                    state.word_doc = Some(preview.document);
                    state.hf_edit_cursor = Some(preview.cursor);
                    state.hf_selection_anchor = preview.anchor;
                    state.editing_mode = preview.mode;
                    state.hf_variant = preview.variant;
                }
                view.prepare_hf_edit_rerender(preview.mode, preview.cursor.page_index);
                return view.finish_hf_edit_result_sync(preview.mode);
            }
            let Some(FormatPreviewState {
                revision,
                selection,
            }) = view.style_preview.replace(None)
            else {
                return Ok(JsValue::NULL);
            };
            if view.state.borrow().command_executor.history_revision() != revision {
                return Ok(JsValue::NULL);
            }
            let payload = view
                .perform_history_action_with_selection_async(HistoryAction::Undo, selection)
                .await?;
            view.state
                .borrow_mut()
                .command_executor
                .discard_newest_redo();
            Self::write_history_flags(payload.unchecked_ref(), view.history_flags())?;
            Ok(payload)
        })
    }

    #[wasm_bindgen]
    pub fn redo(&self) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            let ended = JsFuture::from(view.end_style_preview()).await?;
            if !view.history_flags().1 && ended.is_object() {
                return Ok(ended);
            }
            let protection_history = {
                let state = view.state.borrow();
                state.command_executor.next_redo_name() == Some("set_document_protection")
            };
            if view.state.borrow().preview_mode && !protection_history {
                return Ok(view.history_state());
            }
            let payload = view
                .perform_history_action_async(HistoryAction::Redo)
                .await?;
            if protection_history {
                view.sync_preview_mode_from_document_protection();
                Reflect::set(
                    &payload,
                    &JsValue::from_str("documentProtectionChanged"),
                    &JsValue::TRUE,
                )?;
            }
            Ok(payload)
        })
    }

    fn sync_preview_mode_from_document_protection(&self) {
        let mut state = self.state.borrow_mut();
        let protected = state
            .word_doc
            .as_ref()
            .and_then(WordDocument::document_protection)
            .is_some_and(|value| value.enforced && value.edit == "readOnly");
        state.preview_mode = protected;
        state.options.set_preview_mode(protected);
    }

    #[wasm_bindgen]
    pub fn load_docx(&self, data: js_sys::Uint8Array) -> js_sys::Promise {
        let renderer = Rc::clone(&self.renderer);
        let state = Rc::clone(&self.state);
        let data_vec = data.to_vec();
        let _data_len = data_vec.len();

        future_to_promise(
            async move {
                let mut options = { state.borrow().options.clone() };

                let word_doc = WordDocument::load(&data_vec, &options)
                    .await
                    .map_err(|e| JsValue::from_str(&format!("{e}")))?;
                options.apply_saved_revision_view(
                    word_doc
                        .document
                        .as_ref()
                        .and_then(|doc| doc.display_content_revisions),
                );
                install_document_font_substitutes(&word_doc);

                let (pending_images, metadata) = {
                    let mut renderer_mut = renderer.borrow_mut();
                    renderer_mut.clear_selection();
                    renderer_mut.set_style_context(
                        word_doc.styles_part.clone().map(Rc::new),
                        word_doc.loaded_theme_part().map(Rc::new),
                    );

                    if let Some(document) = word_doc.document.as_ref() {
                        // Install first/even variants before the render so
                        // `render_header_simple` / `render_footer_simple` can
                        // select the right variant per page.
                        renderer_mut.set_header_footer_variants(
                            word_doc.header_lines_first.clone(),
                            word_doc.footer_lines_first.clone(),
                            word_doc.header_lines_even.clone(),
                            word_doc.footer_lines_even.clone(),
                            word_doc.title_page,
                            word_doc.even_and_odd_headers,
                        );
                        renderer_mut.set_header_footer_bodies(
                            word_doc.header_footer_body(
                                HeaderFooterRole::Header,
                                HeaderFooterVariant::Default,
                            ),
                            word_doc.header_footer_body(
                                HeaderFooterRole::Footer,
                                HeaderFooterVariant::Default,
                            ),
                            word_doc.header_footer_body(
                                HeaderFooterRole::Header,
                                HeaderFooterVariant::First,
                            ),
                            word_doc.header_footer_body(
                                HeaderFooterRole::Footer,
                                HeaderFooterVariant::First,
                            ),
                            word_doc.header_footer_body(
                                HeaderFooterRole::Header,
                                HeaderFooterVariant::Even,
                            ),
                            word_doc.header_footer_body(
                                HeaderFooterRole::Footer,
                                HeaderFooterVariant::Even,
                            ),
                        );
                        renderer_mut.set_section_header_footers(word_doc.section_header_footers());
                        // Map bookmark names to their owning paragraph so that
                        // PAGEREF fields can resolve to the bookmark's page.
                        renderer_mut.set_bookmark_paragraphs(word_doc.bookmark_render_map());
                        renderer_mut.set_bookmark_note_targets(word_doc.bookmark_note_targets());
                        // Snapshot the host clock so DATE/TIME fields resolve.
                        renderer_mut.set_current_datetime(Some(current_field_datetime()));
                        renderer_mut
                            .render_with_numbering_rc(
                                Rc::clone(document),
                                &options,
                                word_doc.numbering_part.as_ref(),
                                word_doc.header_lines.as_deref(),
                                word_doc.footer_lines.as_deref(),
                                Some(&word_doc.comments),
                            )
                            .map_err(|e| JsValue::from_str(&format!("{e:?}")))?;
                    }

                    let metadata = renderer_mut.table_resize_metadata();
                    let pending = renderer_mut.drain_pending_image_promises();
                    (pending, metadata)
                };

                if let Some(wait) = pending_images {
                    if let Err(err) = JsFuture::from(wait).await {
                        web_sys::console::error_1(&err);
                    }
                }

                let paged_images = if options.pageless() {
                    None
                } else {
                    let mut renderer_mut = renderer.borrow_mut();
                    renderer_mut.take_paged_snapshots()?
                };

                {
                    let mut state_mut = state.borrow_mut();
                    state_mut.options = options;
                    state_mut.word_doc = Some(word_doc.clone());
                    state_mut.paged_images = paged_images.unwrap_or_default();
                    state_mut.command_executor = CommandExecutor::new();
                    state_mut.composition_preview = None;
                    state_mut.selection = None;
                    state_mut.hf_body_selection = None;
                    state_mut.caret_goal_x = None;
                    state_mut.inline_formatting = InlineFormattingState::default();
                    state_mut.table_resize = metadata;
                    state_mut.lazy_mode = false;
                }
                renderer
                    .borrow_mut()
                    .set_band_redraw_paints_band_page_only(false);

                to_value(&word_doc).map_err(|e| JsValue::from_str(&format!("{e}")))
            }
            .instrument(info_span!("load_docx", bytes = _data_len)),
        )
    }

    /// Loads a DOCX document and renders to canvas, but does NOT create ImageBitmaps.
    /// Use `create_page_bitmap(pageIndex)` to get ImageBitmaps on-demand.
    /// Returns { document, pageCount, pages: [{width, height}, ...] }
    #[wasm_bindgen]
    pub fn load_docx_lazy(&self, data: js_sys::Uint8Array) -> js_sys::Promise {
        let renderer = Rc::clone(&self.renderer);
        let state = Rc::clone(&self.state);
        let data_vec = data.to_vec();
        let _data_len = data_vec.len();

        future_to_promise(
            async move {
                let mut options = { state.borrow().options.clone() };

                let word_doc = WordDocument::load(&data_vec, &options)
                    .await
                    .map_err(|e| JsValue::from_str(&format!("{e}")))?;
                options.apply_saved_revision_view(
                    word_doc
                        .document
                        .as_ref()
                        .and_then(|doc| doc.display_content_revisions),
                );
                install_document_font_substitutes(&word_doc);

                let (pending_images, metadata, page_count) = {
                    let mut renderer_mut = renderer.borrow_mut();
                    renderer_mut.clear_selection();
                    renderer_mut.set_style_context(
                        word_doc.styles_part.clone().map(Rc::new),
                        word_doc.loaded_theme_part().map(Rc::new),
                    );

                    if let Some(document) = word_doc.document.as_ref() {
                        renderer_mut.set_header_footer_variants(
                            word_doc.header_lines_first.clone(),
                            word_doc.footer_lines_first.clone(),
                            word_doc.header_lines_even.clone(),
                            word_doc.footer_lines_even.clone(),
                            word_doc.title_page,
                            word_doc.even_and_odd_headers,
                        );
                        renderer_mut.set_header_footer_bodies(
                            word_doc.header_footer_body(
                                HeaderFooterRole::Header,
                                HeaderFooterVariant::Default,
                            ),
                            word_doc.header_footer_body(
                                HeaderFooterRole::Footer,
                                HeaderFooterVariant::Default,
                            ),
                            word_doc.header_footer_body(
                                HeaderFooterRole::Header,
                                HeaderFooterVariant::First,
                            ),
                            word_doc.header_footer_body(
                                HeaderFooterRole::Footer,
                                HeaderFooterVariant::First,
                            ),
                            word_doc.header_footer_body(
                                HeaderFooterRole::Header,
                                HeaderFooterVariant::Even,
                            ),
                            word_doc.header_footer_body(
                                HeaderFooterRole::Footer,
                                HeaderFooterVariant::Even,
                            ),
                        );
                        renderer_mut.set_section_header_footers(word_doc.section_header_footers());
                        // Map bookmark names to their owning paragraph so that
                        // PAGEREF fields can resolve to the bookmark's page.
                        renderer_mut.set_bookmark_paragraphs(word_doc.bookmark_render_map());
                        renderer_mut.set_bookmark_note_targets(word_doc.bookmark_note_targets());
                        // Snapshot the host clock so DATE/TIME fields resolve.
                        renderer_mut.set_current_datetime(Some(current_field_datetime()));
                        renderer_mut
                            .render_with_numbering_rc(
                                Rc::clone(document),
                                &options,
                                word_doc.numbering_part.as_ref(),
                                word_doc.header_lines.as_deref(),
                                word_doc.footer_lines.as_deref(),
                                Some(&word_doc.comments),
                            )
                            .map_err(|e| JsValue::from_str(&format!("{e:?}")))?;
                    }

                    let metadata = renderer_mut.table_resize_metadata();
                    let pending = renderer_mut.drain_pending_image_promises();
                    let page_count = renderer_mut.page_count();
                    (pending, metadata, page_count)
                };

                if let Some(wait) = pending_images {
                    if let Err(err) = JsFuture::from(wait).await {
                        web_sys::console::error_1(&err);
                    }
                }

                // Collect page dimensions without creating ImageBitmaps
                let page_dimensions: Vec<CanvasSize> = {
                    let renderer_ref = renderer.borrow();
                    (0..page_count)
                        .filter_map(|i| {
                            renderer_ref.page_dimensions(i).map(|(w, h)| CanvasSize {
                                width: w,
                                height: h,
                            })
                        })
                        .collect()
                };

                {
                    let mut state_mut = state.borrow_mut();
                    state_mut.options = options;
                    state_mut.word_doc = Some(word_doc.clone());
                    state_mut.paged_images = Vec::new(); // Don't store ImageBitmaps
                    state_mut.command_executor = CommandExecutor::new();
                    state_mut.composition_preview = None;
                    state_mut.selection = None;
                    state_mut.hf_body_selection = None;
                    state_mut.caret_goal_x = None;
                    state_mut.inline_formatting = InlineFormattingState::default();
                    state_mut.table_resize = metadata;
                    state_mut.lazy_mode = true;
                }
                renderer
                    .borrow_mut()
                    .set_band_redraw_paints_band_page_only(true);

                // Build response object
                let result = Object::new();
                let doc_value =
                    to_value(&word_doc).map_err(|e| JsValue::from_str(&format!("{e}")))?;
                Reflect::set(&result, &JsValue::from_str("document"), &doc_value)?;
                Reflect::set(
                    &result,
                    &JsValue::from_str("pageCount"),
                    &JsValue::from_f64(page_count as f64),
                )?;

                let pages_array = Array::new();
                for dim in &page_dimensions {
                    let page_obj = Object::new();
                    Reflect::set(
                        &page_obj,
                        &JsValue::from_str("width"),
                        &JsValue::from_f64(dim.width),
                    )?;
                    Reflect::set(
                        &page_obj,
                        &JsValue::from_str("height"),
                        &JsValue::from_f64(dim.height),
                    )?;
                    pages_array.push(&page_obj);
                }
                Reflect::set(&result, &JsValue::from_str("pages"), &pages_array)?;

                Ok(result.into())
            }
            .instrument(info_span!("load_docx_lazy", bytes = _data_len)),
        )
    }

    /// Loads a DOCX document and performs a *partial* layout up to the given page (1-indexed, inclusive).
    ///
    /// This is the foundation of "lazy layout": the host can request just enough pages
    /// for the current viewport (+buffer) and progressively extend the layout as the user scrolls.
    ///
    /// Returns:
    /// - document: serialized WordDocument
    /// - pageCount: number of pages currently laid out (may grow later)
    /// - pages: [{ width, height }, ...] for the pages currently laid out
    /// - complete: boolean indicating whether the full document layout is complete
    #[wasm_bindgen(js_name = loadDocxLazyUntil)]
    pub fn load_docx_lazy_until(
        &self,
        data: js_sys::Uint8Array,
        end_page: usize,
    ) -> js_sys::Promise {
        self.load_docx_lazy_until_impl(data, end_page, true, false)
    }

    /// Editor-runtime variant of `loadDocxLazyUntil` that omits the serialized
    /// `WordDocument` from the response. The renderer keeps the authoritative
    /// model in Rust; normal UI code only consumes page/layout metadata, so
    /// serializing the complete model into JS is avoidable startup work.
    #[wasm_bindgen(js_name = loadDocxLazyUntilCompact)]
    pub fn load_docx_lazy_until_compact(
        &self,
        data: js_sys::Uint8Array,
        end_page: usize,
    ) -> js_sys::Promise {
        self.load_docx_lazy_until_impl(data, end_page, false, false)
    }

    /// Compact lazy-load variant that also keeps the preservation XML parse off
    /// the initial render path. The worker prepares it before editing/export.
    #[wasm_bindgen(js_name = loadDocxLazyUntilDeferredRoundtrip)]
    pub fn load_docx_lazy_until_deferred_roundtrip(
        &self,
        data: js_sys::Uint8Array,
        end_page: usize,
    ) -> js_sys::Promise {
        self.load_docx_lazy_until_impl(data, end_page, false, true)
    }

    fn load_docx_lazy_until_impl(
        &self,
        data: js_sys::Uint8Array,
        end_page: usize,
        include_document: bool,
        defer_roundtrip_xml: bool,
    ) -> js_sys::Promise {
        let renderer = Rc::clone(&self.renderer);
        let state = Rc::clone(&self.state);
        let data_vec = data.to_vec();
        let _data_len = data_vec.len();
        let end_page = end_page.max(1);

        future_to_promise(
            async move {
                let mut options = { state.borrow().options.clone() };

                let word_doc = if defer_roundtrip_xml {
                    WordDocument::load_for_initial_render(&data_vec, &options).await
                } else {
                    WordDocument::load(&data_vec, &options).await
                }
                .map_err(|e| JsValue::from_str(&format!("{e}")))?;
                options.apply_saved_revision_view(
                    word_doc
                        .document
                        .as_ref()
                        .and_then(|doc| doc.display_content_revisions),
                );
                install_document_font_substitutes(&word_doc);

                let (pending_images, metadata, page_count, complete) = {
                    let mut renderer_mut = renderer.borrow_mut();
                    renderer_mut.clear_selection();
                    renderer_mut.set_style_context(
                        word_doc.styles_part.clone().map(Rc::new),
                        word_doc.loaded_theme_part().map(Rc::new),
                    );
                    renderer_mut.set_layout_page_limit(Some(end_page));
                    // In lazy layout mode, we only need pixels for the laid-out pages.
                    renderer_mut.set_visible_page_range(Some(1..=end_page));

                    if let Some(document) = word_doc.document.as_ref() {
                        renderer_mut.set_header_footer_variants(
                            word_doc.header_lines_first.clone(),
                            word_doc.footer_lines_first.clone(),
                            word_doc.header_lines_even.clone(),
                            word_doc.footer_lines_even.clone(),
                            word_doc.title_page,
                            word_doc.even_and_odd_headers,
                        );
                        renderer_mut.set_header_footer_bodies(
                            word_doc.header_footer_body(
                                HeaderFooterRole::Header,
                                HeaderFooterVariant::Default,
                            ),
                            word_doc.header_footer_body(
                                HeaderFooterRole::Footer,
                                HeaderFooterVariant::Default,
                            ),
                            word_doc.header_footer_body(
                                HeaderFooterRole::Header,
                                HeaderFooterVariant::First,
                            ),
                            word_doc.header_footer_body(
                                HeaderFooterRole::Footer,
                                HeaderFooterVariant::First,
                            ),
                            word_doc.header_footer_body(
                                HeaderFooterRole::Header,
                                HeaderFooterVariant::Even,
                            ),
                            word_doc.header_footer_body(
                                HeaderFooterRole::Footer,
                                HeaderFooterVariant::Even,
                            ),
                        );
                        renderer_mut.set_section_header_footers(word_doc.section_header_footers());
                        // Map bookmark names to their owning paragraph so that
                        // PAGEREF fields can resolve to the bookmark's page.
                        renderer_mut.set_bookmark_paragraphs(word_doc.bookmark_render_map());
                        renderer_mut.set_bookmark_note_targets(word_doc.bookmark_note_targets());
                        // Snapshot the host clock so DATE/TIME fields resolve.
                        renderer_mut.set_current_datetime(Some(current_field_datetime()));
                        renderer_mut
                            .render_with_numbering_rc(
                                Rc::clone(document),
                                &options,
                                word_doc.numbering_part.as_ref(),
                                word_doc.header_lines.as_deref(),
                                word_doc.footer_lines.as_deref(),
                                Some(&word_doc.comments),
                            )
                            .map_err(|e| JsValue::from_str(&format!("{e:?}")))?;
                    }

                    let metadata = renderer_mut.table_resize_metadata();
                    let pending = renderer_mut.drain_pending_image_promises();
                    let page_count = renderer_mut.page_count();
                    let complete = renderer_mut.last_layout_complete();
                    (pending, metadata, page_count, complete)
                };

                if let Some(wait) = pending_images {
                    if let Err(err) = JsFuture::from(wait).await {
                        web_sys::console::error_1(&err);
                    }
                }

                let page_dimensions: Vec<CanvasSize> = {
                    let renderer_ref = renderer.borrow();
                    (0..page_count)
                        .filter_map(|i| {
                            renderer_ref.page_dimensions(i).map(|(w, h)| CanvasSize {
                                width: w,
                                height: h,
                            })
                        })
                        .collect()
                };

                let doc_value = if include_document {
                    Some(to_value(&word_doc).map_err(|e| JsValue::from_str(&format!("{e}")))?)
                } else {
                    None
                };

                {
                    let mut state_mut = state.borrow_mut();
                    state_mut.options = options;
                    state_mut.word_doc = Some(word_doc);
                    state_mut.paged_images = Vec::new();
                    state_mut.command_executor = CommandExecutor::new();
                    state_mut.composition_preview = None;
                    state_mut.selection = None;
                    state_mut.hf_body_selection = None;
                    state_mut.caret_goal_x = None;
                    state_mut.inline_formatting = InlineFormattingState::default();
                    state_mut.table_resize = metadata;
                    state_mut.lazy_mode = true;
                }
                renderer
                    .borrow_mut()
                    .set_band_redraw_paints_band_page_only(true);

                let result = Object::new();
                if let Some(doc_value) = doc_value {
                    Reflect::set(&result, &JsValue::from_str("document"), &doc_value)?;
                }
                Reflect::set(
                    &result,
                    &JsValue::from_str("pageCount"),
                    &JsValue::from_f64(page_count as f64),
                )?;
                Reflect::set(
                    &result,
                    &JsValue::from_str("complete"),
                    &JsValue::from_bool(complete),
                )?;

                let pages_array = Array::new();
                for dim in &page_dimensions {
                    let page_obj = Object::new();
                    Reflect::set(
                        &page_obj,
                        &JsValue::from_str("width"),
                        &JsValue::from_f64(dim.width),
                    )?;
                    Reflect::set(
                        &page_obj,
                        &JsValue::from_str("height"),
                        &JsValue::from_f64(dim.height),
                    )?;
                    pages_array.push(&page_obj);
                }
                Reflect::set(&result, &JsValue::from_str("pages"), &pages_array)?;

                Ok(result.into())
            }
            .instrument(info_span!(
                "load_docx_lazy_until",
                bytes = _data_len,
                end_page
            )),
        )
    }

    /// Ensures the document is laid out up to `end_page` (1-indexed, inclusive).
    ///
    /// This re-renders the document with a higher layout limit and returns updated
    /// page dimensions. The host can call this when the viewport approaches the end
    /// of the currently laid-out pages.
    ///
    /// Returns { pageCount, pages: [{width,height}, ...], complete } by default.
    /// With compact_dimensions, replaces pages with pageDimensionRuns: [[count,width,height], ...].
    #[wasm_bindgen(js_name = ensureLayoutThroughPage)]
    pub fn ensure_layout_through_page(
        &self,
        end_page: usize,
        compact_dimensions: Option<bool>,
    ) -> js_sys::Promise {
        let view = self.clone();
        let end_page = end_page.max(1);
        future_to_promise(async move {
            view.ensure_layout_through_page_async(end_page, compact_dimensions.unwrap_or(false))
                .await
        })
    }

    #[cfg_attr(feature = "otel", tracing::instrument(skip_all))]
    async fn ensure_layout_through_page_async(
        &self,
        end_page: usize,
        compact_dimensions: bool,
    ) -> Result<JsValue, JsValue> {
        let (options, document, numbering_part, header_lines, footer_lines, comments) = {
            let state = self.state.borrow();
            let word_doc = state
                .word_doc
                .as_ref()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;
            (
                state.options.clone(),
                word_doc.document.as_ref().map(Rc::clone),
                word_doc.numbering_part.clone(),
                word_doc.header_lines.clone(),
                word_doc.footer_lines.clone(),
                word_doc.comments.clone(),
            )
        };

        let (pending_images, page_count, complete, table_resize_metadata) = {
            let mut renderer_mut = self.renderer.borrow_mut();
            let previous_visible_range = renderer_mut.visible_page_range();
            // IMPORTANT:
            // Do NOT clear the selection here.
            //
            // `ensureLayoutThroughPage` may run in the background while the user is typing.
            // If we clear selection, subsequent edit commands (e.g. insert_text) can observe
            // `selection_range() == None` and fall back to `TextPosition::default()` (0,0),
            // which manifests as "caret jumps to the beginning while typing".
            let previous_limit = renderer_mut.layout_page_limit();
            renderer_mut.set_layout_page_limit(Some(end_page));
            // Preserve the current viewport window while extending layout in the background.
            // Expanding the visible range to `1..=end_page` makes later edit renders treat the
            // whole laid-out prefix as visible, which causes avoidable tail bitmap churn.
            renderer_mut.set_visible_page_range(previous_visible_range.clone());
            // Resume the layout-only walk from the last valid page-top snapshot
            // instead of re-walking every laid-out page; falls back to the
            // from-start walk when no snapshot qualifies.
            renderer_mut.prepare_layout_extension_seek(previous_limit, end_page, &options);

            if let Some(document) = document.as_ref() {
                renderer_mut
                    .render_layout_with_numbering_rc(
                        Rc::clone(document),
                        &options,
                        numbering_part.as_ref(),
                        header_lines.as_deref(),
                        footer_lines.as_deref(),
                        Some(&comments),
                    )
                    .map_err(|e| JsValue::from_str(&format!("{e:?}")))?;
            }
            // Layout expansion should not leave dirty regions that can slow subsequent renders.
            renderer_mut.clear_dirty_regions();

            let pending = renderer_mut.drain_pending_image_promises();
            let page_count = renderer_mut.page_count();
            let complete = renderer_mut.last_layout_complete();
            let table_resize_metadata = renderer_mut.table_resize_metadata();
            (pending, page_count, complete, table_resize_metadata)
        };

        if let Some(wait) = pending_images {
            if let Err(err) = JsFuture::from(wait).await {
                web_sys::console::error_1(&err);
            }
        }

        {
            let mut state_mut = self.state.borrow_mut();
            state_mut.table_resize = table_resize_metadata;
        }

        let result = Object::new();
        Reflect::set(
            &result,
            &JsValue::from_str("pageCount"),
            &JsValue::from_f64(page_count as f64),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("complete"),
            &JsValue::from_bool(complete),
        )?;
        // Encode consecutive equal sizes before crossing the WASM/JS boundary.
        // Complete snapshots need no cache invalidation after edits or shrinkage.
        if compact_dimensions {
            let renderer = self.renderer.borrow();
            let sizes = (0..page_count).filter_map(|i| renderer.page_dimensions(i));
            let runs = to_value(&page_dimension_runs(sizes))
                .map_err(|e| JsValue::from_str(&format!("{e}")))?;
            Reflect::set(&result, &JsValue::from_str("pageDimensionRuns"), &runs)?;
            return Ok(result.into());
        }
        let page_dimensions: Vec<CanvasSize> = {
            let renderer_ref = self.renderer.borrow();
            (0..page_count)
                .filter_map(|i| {
                    renderer_ref.page_dimensions(i).map(|(w, h)| CanvasSize {
                        width: w,
                        height: h,
                    })
                })
                .collect()
        };
        let pages_array = Array::new();
        for dim in &page_dimensions {
            let page_obj = Object::new();
            Reflect::set(
                &page_obj,
                &JsValue::from_str("width"),
                &JsValue::from_f64(dim.width),
            )?;
            Reflect::set(
                &page_obj,
                &JsValue::from_str("height"),
                &JsValue::from_f64(dim.height),
            )?;
            pages_array.push(&page_obj);
        }
        Reflect::set(&result, &JsValue::from_str("pages"), &pages_array)?;
        Ok(result.into())
    }

    #[wasm_bindgen]
    pub fn insert_text(&self, text: String) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move { view.insert_text_async(text).await })
    }

    #[wasm_bindgen(js_name = insertTypedText)]
    pub fn insert_typed_text(
        &self,
        text: String,
        language: String,
        capitalization: String,
    ) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            view.insert_text_with_autoformat_async(text, Some((language, capitalization)))
                .await
        })
    }

    #[wasm_bindgen(js_name = previewCompositionText)]
    pub fn preview_composition_text(&self, text: String) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move { view.preview_composition_text_async(text).await })
    }

    #[wasm_bindgen]
    pub fn insert_paragraph_break(&self) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move { view.insert_paragraph_break_async().await })
    }

    #[wasm_bindgen]
    pub fn insert_paragraph_breaks(&self, count: usize) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move { view.insert_paragraph_breaks_async(count).await })
    }

    #[cfg_attr(feature = "otel", tracing::instrument(skip_all))]
    async fn insert_paragraph_break_async(&self) -> Result<JsValue, JsValue> {
        self.insert_paragraph_breaks_async(1).await
    }

    #[cfg_attr(feature = "otel", tracing::instrument(skip_all))]
    async fn insert_paragraph_breaks_async(&self, count: usize) -> Result<JsValue, JsValue> {
        if self.state.borrow().preview_mode {
            return Ok(JsValue::NULL);
        }
        let break_count = count.max(1);
        {
            let editing_mode = self.state.borrow().editing_mode;
            if editing_mode != EditingMode::Body {
                return self
                    .insert_hf_paragraph_breaks(break_count, editing_mode)
                    .await;
            }
        }
        // Word's Enter on an empty list item ends the list: the item becomes
        // a plain paragraph instead of a second empty numbered one.
        if break_count == 1 {
            if let Some((_, true)) = self.caret_at_list_item_start() {
                return self.apply_paragraph_style_headless(|s, e| HeadlessCommand::SetNumbering {
                    start_paragraph: s,
                    end_paragraph: e,
                    num_id: None,
                    level: None,
                });
            }
        }
        let use_single_break_optimizations = break_count == 1;
        let mut break_paragraph_indices: Vec<usize> = Vec::with_capacity(break_count);

        let selection_before = {
            let renderer = self.renderer.borrow();
            renderer
                .selection_range()
                .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
        };
        let edited_paragraph_index = selection_before.focus.paragraph;
        let dirty_from = selection_before
            .anchor
            .paragraph
            .min(selection_before.focus.paragraph);
        let old_height = {
            let renderer = self.renderer.borrow();
            renderer.cached_paragraph_height(edited_paragraph_index)
        };

        // Detect if we're splitting at the end of the paragraph (creating an empty new paragraph)
        // This allows for more aggressive cache reuse optimizations.
        let (
            split_at_end,
            edited_paragraph_in_table,
            edited_paragraph_numbered,
            edited_paragraph_num_id,
        ) = if use_single_break_optimizations {
            let state_ref = self.state.borrow();
            if let Some(word_doc) = state_ref.word_doc.as_ref() {
                let in_table = word_doc.paragraph_is_in_table(edited_paragraph_index);
                if let Some(paragraph) = word_doc.paragraph(edited_paragraph_index) {
                    let paragraph_len: usize =
                        paragraph.runs.iter().map(|run| run.text.len()).sum();
                    let numbered = paragraph
                        .properties
                        .as_ref()
                        .and_then(|props| props.numbering.as_ref())
                        .is_some();
                    let num_id = paragraph
                        .properties
                        .as_ref()
                        .and_then(|props| props.numbering.as_ref())
                        .and_then(|numbering| numbering.id.clone());
                    (
                        selection_before.focus.offset >= paragraph_len,
                        in_table,
                        numbered,
                        num_id,
                    )
                } else {
                    (false, in_table, false, None)
                }
            } else {
                (false, false, false, None)
            }
        } else {
            let state_ref = self.state.borrow();
            if let Some(word_doc) = state_ref.word_doc.as_ref() {
                let in_table = word_doc.paragraph_is_in_table(edited_paragraph_index);
                let numbering = word_doc
                    .paragraph(edited_paragraph_index)
                    .and_then(|paragraph| paragraph.properties.as_ref())
                    .and_then(|props| props.numbering.as_ref());
                let num_id = numbering.and_then(|numbering| numbering.id.clone());
                (false, in_table, numbering.is_some(), num_id)
            } else {
                (false, false, false, None)
            }
        };

        // Drop the renderer's document cache before mutating.
        let _document_cache = self.drop_renderer_document_cache();

        let (
            options,
            numbering_part,
            header_lines,
            footer_lines,
            new_selection,
            paragraph_count_after,
        ) = {
            let mut state = self.state.borrow_mut();
            let mut word_doc = state
                .word_doc
                .take()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;

            state.selection = Some(selection_before);
            {
                // Normalize selection first, then execute paragraph breaks from a deterministic caret.
                let mut commands = Vec::new();
                let mut command_executor = std::mem::take(&mut state.command_executor);
                let mut collapsed = state
                    .selection
                    .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()));
                if !collapsed.is_collapsed() {
                    let (start, end) = collapsed.normalized();
                    commands.push(HeadlessCommand::ReplaceRange {
                        start_paragraph: start.paragraph,
                        start_offset: start.offset,
                        end_paragraph: end.paragraph,
                        end_offset: end.offset,
                        text: String::new(),
                    });
                    collapsed = SelectionRange::collapsed(start);
                }

                if break_count > 1 {
                    let start_index = collapsed.focus.paragraph;
                    for offset in 0..break_count {
                        break_paragraph_indices.push(start_index.saturating_add(offset));
                    }
                } else {
                    break_paragraph_indices.push(collapsed.focus.paragraph);
                }
                commands.push(HeadlessCommand::InsertParagraphBreak {
                    paragraph: collapsed.focus.paragraph,
                    offset: collapsed.focus.offset,
                    count: break_count,
                });
                let request = HeadlessEditRequest { commands };
                let command_result = apply_headless_commands_with_executor_to_document(
                    &mut word_doc,
                    &mut command_executor,
                    state.selection,
                    &request,
                );
                state.command_executor = command_executor;
                let selection_after = match command_result {
                    Ok(selection) => selection,
                    Err(err) => {
                        state.word_doc = Some(word_doc);
                        return Err(JsValue::from_str(&err.to_string()));
                    }
                };
                state.selection = selection_after;
            }

            state.paged_images.clear();

            let options = state.options.clone();
            let numbering_part = word_doc.numbering_part.clone();
            let header_lines = word_doc.header_lines.clone();
            let footer_lines = word_doc.footer_lines.clone();
            let new_selection = state.selection;
            let paragraph_count_after = word_doc.paragraph_count();

            state.word_doc = Some(word_doc);

            (
                options,
                numbering_part,
                header_lines,
                footer_lines,
                new_selection,
                paragraph_count_after,
            )
        };

        let anchor_page = {
            let renderer = self.renderer.borrow();
            renderer
                .cached_paragraph_start_page(edited_paragraph_index)
                .or_else(|| renderer.visible_page_range().as_ref().map(|r| *r.end()))
                .or_else(|| renderer.layout_page_limit())
                .unwrap_or(1)
        };
        let numbered_render_cache_invalidations = if edited_paragraph_numbered {
            let state_ref = self.state.borrow();
            if let Some(word_doc) = state_ref.word_doc.as_ref() {
                if let Some(num_id) = edited_paragraph_num_id.as_deref() {
                    Self::numbered_paragraph_indices_for_num_id(
                        word_doc,
                        dirty_from,
                        paragraph_count_after.saturating_sub(1),
                        num_id,
                    )
                } else {
                    (dirty_from..paragraph_count_after).collect()
                }
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        let lazy_mode = self.state.borrow().lazy_mode;

        {
            let mut renderer = self.renderer.borrow_mut();
            if edited_paragraph_numbered {
                // Numbered paragraph breaks change the marker text of downstream list items.
                // Cached render entries only know the resolved marker drawn during the previous
                // render. Shift indices to preserve unrelated downstream entries, then invalidate
                // the same numId chain whose marker ordinal can change after the insert.
                renderer.set_incremental_dirty_from(dirty_from);
                renderer.apply_paragraph_break_cache_shift_batch(dirty_from, break_count);
                renderer.invalidate_layout_from_keep_render_cache(dirty_from);
                renderer.invalidate_paragraph_render_cache_indices(
                    &numbered_render_cache_invalidations,
                );
                // Wrap cache includes marker width and remains a tail invalidation for now.
                renderer.invalidate_paragraph_wrap_cache_from(dirty_from);
                renderer.disable_seek_replay_once();
            } else if !use_single_break_optimizations {
                renderer.set_incremental_dirty_from(dirty_from);
                // Preserve downstream render cache for multi-break inserts by shifting
                // indices instead of invalidating the entire render cache tail.
                let is_contiguous_break_run = break_paragraph_indices
                    .windows(2)
                    .all(|pair| pair[1] == pair[0].saturating_add(1));
                if let Some(first_break_index) = break_paragraph_indices.first().copied() {
                    if is_contiguous_break_run {
                        renderer.apply_paragraph_break_cache_shift_batch(
                            first_break_index,
                            break_paragraph_indices.len(),
                        );
                    } else {
                        for edited_index in break_paragraph_indices.iter().copied() {
                            renderer.apply_paragraph_break_cache_shift(edited_index);
                        }
                    }
                }
                renderer.invalidate_layout_from_keep_render_cache(dirty_from);
                if !lazy_mode {
                    if let Some(first_break_index) = break_paragraph_indices.first().copied() {
                        // shift_replay must land on the first downstream cache entry
                        // that survived the batched shift. Anything earlier lands on
                        // one of the freshly-inserted None gaps (or the invalidated
                        // edited paragraph), the very first replay attempt misses,
                        // and shift-replay disables itself for the rest of the render
                        // — defeating the optimization and, in tight page-bounded
                        // render windows, leaving the new empty paragraphs
                        // unrendered after rapid Enter.
                        let first_downstream =
                            <CanvasRenderer>::paragraph_break_batch_first_downstream_index(
                                first_break_index,
                                break_paragraph_indices.len(),
                            );
                        renderer.set_shift_replay_from(first_downstream);
                    }
                }
                // Mirror the single-break invariant: keep the fast-replay short-circuit
                // from truncating before the new empty paragraphs are drawn. Currently
                // the multi-break path does not arm `fast_replay_trigger`, so this is
                // defensive — but it costs nothing and keeps both paths consistent
                // should a future change start arming it for batches too.
                renderer.set_paragraph_break_pending(true);
            } else if edited_paragraph_in_table {
                // Table paragraphs still take the conservative path. Their layout dependencies
                // are broader than plain body paragraphs and can invalidate cache indices in
                // less predictable ways.
                renderer.set_incremental_dirty_from(dirty_from);
                renderer.invalidate_layout_from(dirty_from);
            } else {
                renderer.set_incremental_dirty_from(dirty_from);

                // Use enhanced paragraph break optimizations which handles:
                // - Layout cache update (gap insertion, edited entry invalidation)
                // - Render cache update (gap insertion, edited entry invalidation)
                // - Seek cache invalidation
                // - shift_replay setup based on split position
                // - fast_replay trigger setup for height comparison
                //
                // This is now used in lazy mode as well for non-table paragraphs so
                // Enter at paragraph start can reuse downstream cache entries.
                renderer.set_paragraph_break_optimizations(
                    edited_paragraph_index,
                    split_at_end,
                    old_height,
                );
            }

            // Paragraph break can reflow content across subsequent pages.
            // Instead of a full repaint, mark the page range from this paragraph onward dirty.
            // Snapshotting is still viewport-limited (see capture_paged_images), and pages outside
            // the snapshot range will be re-rendered on-demand when requested.
            //
            // Optimization levels:
            // 1. If splitting at end AND content fits on page: minimal dirty (current page only)
            // 2. If splitting at end: bound to current + 1 page
            // 3. Otherwise: bound to current + 2 pages (conservative)
            let lazy_mode = self.state.borrow().lazy_mode;
            if lazy_mode {
                let visible_end = Self::lazy_visible_end(&renderer, anchor_page);
                let dirty_page_bound = if split_at_end {
                    // Be conservative even when we *think* it fits: line-height estimates can be off,
                    // and a single break can still push content to the next page.
                    anchor_page.saturating_add(1).max(1)
                } else {
                    // Content moved: could cause more reflow, bound to current + 2 pages
                    anchor_page.saturating_add(2).max(1)
                };
                let dirty_page_bound = dirty_page_bound.min(visible_end);
                renderer.mark_dirty_from_paragraph_to_end_bounded(dirty_from, dirty_page_bound);
            } else {
                renderer.mark_dirty_from_paragraph_to_end(dirty_from);
            }
        }

        {
            let state_ref = self.state.borrow();
            if let Some(word_doc) = state_ref.word_doc.as_ref() {
                if let Some(document) = word_doc.document.as_ref() {
                    let mut renderer = self.renderer.borrow_mut();

                    // Snapshot the renderer's prior `layout_page_limit` BEFORE the
                    // single-break optimization tightens it. In non-lazy mode this
                    // limit is otherwise never reset across edits, so a tighter
                    // value chosen for one Enter would persist into the next render
                    // pass. After rapid 連打 the accumulated limit can fall behind
                    // the page where new empty paragraphs eventually land — the
                    // render then truncates before drawing them and the user sees
                    // the caret advance with no visible new line. See the
                    // `single_break_lightweight_setup_does_not_inherit_stale_layout_page_limit`
                    // regression test in `canvas_renderer.rs` for the exact repro.
                    let prior_layout_page_limit = renderer.layout_page_limit();
                    let mut restore_layout_page_limit = false;

                    if !lazy_mode
                        && use_single_break_optimizations
                        && !edited_paragraph_in_table
                        && !edited_paragraph_numbered
                    {
                        // Setup lightweight rendering path for paragraph breaks,
                        // without its page cap: the walk would end at the cap,
                        // and as nothing extends a non-lazy layout afterwards,
                        // every page past it would be dropped from the document.
                        renderer.setup_lightweight_paragraph_break_render(
                            edited_paragraph_index,
                            split_at_end,
                            false,
                        );
                        // Only restore in non-lazy mode; lazy mode manages the
                        // limit explicitly below and intentionally retains it
                        // across edits (see canvas_view.rs:2526 commentary).
                        restore_layout_page_limit = true;
                    }

                    // Respect host-provided visible range if configured (viewport virtualization).
                    // Ensure the edited page stays visible even if the range is stale.
                    let lazy_mode = self.state.borrow().lazy_mode;
                    Self::ensure_visible_page_range_for_edit(&mut renderer, lazy_mode, anchor_page);
                    let prev_limit: Option<Option<usize>> = if self.state.borrow().lazy_mode {
                        let desired = Self::lazy_visible_end(&renderer, anchor_page);
                        let prev = renderer.layout_page_limit();
                        // Clamp layout work to the viewport window during edits so large documents
                        // do not force a full layout pass while typing.
                        renderer.set_layout_page_limit(Some(desired));
                        Some(prev)
                    } else {
                        None
                    };
                    // Use _rc variant to avoid cloning the document.
                    renderer.render_with_numbering_rc(
                        Rc::clone(document),
                        &options,
                        numbering_part.as_ref(),
                        header_lines.as_deref(),
                        footer_lines.as_deref(),
                        Some(&word_doc.comments),
                    )?;
                    if let Some(prev) = prev_limit {
                        renderer.set_layout_page_limit(prev);
                    } else if restore_layout_page_limit {
                        // Non-lazy single-break path: drop the per-render
                        // tightening applied by `setup_lightweight_paragraph_break_render`
                        // so it does not bleed into the next Enter's render pass.
                        // Without this, rapid Enter accumulates an ever-tighter
                        // limit and eventually truncates the render before the
                        // newly-inserted empty paragraph is drawn.
                        renderer.set_layout_page_limit(prior_layout_page_limit);
                    }
                }
            }
        }

        if let Some(range) = new_selection {
            let mut renderer = self.renderer.borrow_mut();
            renderer.set_selection_range(Some(range));
        }
        let caret_metrics = {
            let renderer = self.renderer.borrow();
            renderer.caret_metrics()
        };
        let selection_after = new_selection;

        // In lazy mode, always skip snapshotting to avoid expensive bitmap generation.
        let skip_snapshots = self.state.borrow().lazy_mode;
        let dirty_regions_js = if !skip_snapshots {
            let (snapshots, dirty_regions) = self.capture_pages_after_render_with_dirty().await?;
            self.store_paged_images(snapshots);
            dirty_regions
        } else {
            // See insert_text_async: avoid accumulating in-flight async image draws.
            self.wait_for_pending_images().await?;
            let (metadata, dirty_regions_js) = {
                let renderer_mut = self.renderer.borrow_mut();
                renderer_mut.coalesce_dirty_regions();
                let dirty_regions = renderer_mut.dirty_regions();
                let dirty_regions_js = Self::dirty_regions_to_js(&dirty_regions);
                let meta = renderer_mut.table_resize_metadata();
                renderer_mut.clear_dirty_regions();
                (meta, dirty_regions_js)
            };
            self.state.borrow_mut().table_resize = metadata;
            dirty_regions_js
        };
        let (preview_mode, history_flags) = {
            let mut state_mut = self.state.borrow_mut();
            state_mut.selection = selection_after;
            state_mut.caret_goal_x = None;
            let flags = state_mut.history_flags();
            (state_mut.preview_mode, flags)
        };

        let size = self.canvas_size()?;
        let pages = self.paged_images();
        let caret_value = if preview_mode {
            JsValue::NULL
        } else {
            match caret_metrics {
                Some(metrics) => to_value(&metrics).unwrap_or(JsValue::NULL),
                None => JsValue::NULL,
            }
        };

        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &size)?;
        Reflect::set(&result, &JsValue::from_str("pages"), &pages)?;
        Reflect::set(&result, &JsValue::from_str("caret"), &caret_value)?;
        Reflect::set(
            &result,
            &JsValue::from_str("dirtyRegions"),
            &dirty_regions_js,
        )?;
        Self::write_history_flags(&result, history_flags)?;
        self.write_object_selection(&result)?;
        self.write_merge_state(&result)?;
        {
            let renderer_ref = self.renderer.borrow();
            Self::write_selection_rects(&result, &renderer_ref, selection_after)?;
        }

        Ok(result.into())
    }

    #[wasm_bindgen]
    pub fn move_caret(&self, direction: String, extend: bool) -> Result<JsValue, JsValue> {
        {
            let state = self.state.borrow();
            if state.preview_mode {
                return Ok(JsValue::NULL);
            }
            if state.editing_mode != EditingMode::Body {
                drop(state);
                return self.move_hf_caret(Self::caret_movement(&direction)?, extend);
            }
        }

        let movement = Self::caret_movement(&direction)?;

        let stored_goal = { self.state.borrow().caret_goal_x };

        let mut renderer = self.renderer.borrow_mut();
        let use_overlay = renderer.use_selection_overlay();
        let current_range = renderer.selection_range();
        let (selection, next_goal) =
            renderer.navigate_selection(current_range, movement, extend, stored_goal);

        let mut changed = false;
        if current_range != Some(selection) {
            changed = renderer.set_selection_range(Some(selection));
            if changed && !use_overlay {
                renderer.skip_change_detection_once();
                renderer.rerender_with_cached_state()?;
            }
        }

        let caret_metrics = renderer.caret_metrics();
        let selection_after = renderer.selection_range();
        let (snapshots, dirty_regions) = if changed && !use_overlay {
            self.capture_paged_images(&mut renderer)?
        } else {
            (None, Array::new().into())
        };
        drop(renderer);

        self.store_paged_images(snapshots);
        {
            let mut state_mut = self.state.borrow_mut();
            state_mut.selection = selection_after;
            state_mut.object_selection = None;
            state_mut.caret_goal_x = next_goal;
        }

        let size = self.canvas_size()?;
        let pages = self.paged_images();
        let caret_value = match caret_metrics {
            Some(metrics) => to_value(&metrics).unwrap_or(JsValue::NULL),
            None => JsValue::NULL,
        };

        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &size)?;
        Reflect::set(&result, &JsValue::from_str("pages"), &pages)?;
        Reflect::set(&result, &JsValue::from_str("caret"), &caret_value)?;
        Reflect::set(&result, &JsValue::from_str("dirtyRegions"), &dirty_regions)?;
        {
            let renderer_ref = self.renderer.borrow();
            Self::write_selection_rects(&result, &renderer_ref, selection_after)?;
        }

        Ok(result.into())
    }

    fn caret_movement(direction: &str) -> Result<CaretMovement, JsValue> {
        Ok(match direction {
            "left" => CaretMovement::Left,
            "right" => CaretMovement::Right,
            "up" => CaretMovement::Up,
            "down" => CaretMovement::Down,
            "paragraph_start" => CaretMovement::ParagraphStart,
            "paragraph_end" => CaretMovement::ParagraphEnd,
            "paragraph_previous" => CaretMovement::ParagraphPrevious,
            "paragraph_next" => CaretMovement::ParagraphNext,
            "line_start" => CaretMovement::LineStart,
            "line_end" => CaretMovement::LineEnd,
            "word_left" => CaretMovement::WordLeft,
            "word_right" => CaretMovement::WordRight,
            "document_start" => CaretMovement::DocumentStart,
            "document_end" => CaretMovement::DocumentEnd,
            other => {
                return Err(JsValue::from_str(&format!(
                    "unknown caret direction `{other}`"
                )));
            }
        })
    }

    fn navigate_hf_caret(&self, movement: CaretMovement, extend: bool) -> Option<()> {
        let state = self.state.borrow();
        let cursor = state.hf_edit_cursor?;
        let mode = state.editing_mode;
        let lines = Self::variant_lines(state.word_doc.as_ref()?, mode, state.hf_variant)?;
        let segments = lines
            .iter()
            .map(Self::hf_single_segment)
            .collect::<Option<Vec<_>>>()?;
        let position = |probe: HfEditCursor| {
            (probe.page_index == cursor.page_index
                && segments.get(probe.line_index)?.0 == probe.segment)
                .then_some(TextPosition::new(probe.line_index, probe.offset))
        };
        let range = SelectionRange::new(
            position(state.hf_selection_anchor.unwrap_or(cursor))?,
            position(cursor)?,
        );
        let expected_texts: Vec<_> = segments.iter().map(|(_, text)| *text).collect();
        let (selection, next_goal) = self.renderer.borrow().hf_story_navigate_selection(
            (cursor.page_index, mode == EditingMode::Header),
            range,
            movement,
            extend,
            state.caret_goal_x,
            &expected_texts,
        )?;
        let to_cursor = |position: TextPosition| {
            Some(HfEditCursor {
                line_index: position.paragraph,
                segment: segments.get(position.paragraph)?.0,
                offset: position.offset,
                ..cursor
            })
        };
        let focus = to_cursor(selection.focus)?;
        let anchor = to_cursor(selection.anchor)?;
        drop(state);
        let mut state = self.state.borrow_mut();
        state.hf_edit_cursor = Some(focus);
        state.hf_selection_anchor = extend.then_some(anchor);
        state.caret_goal_x = next_goal;
        Some(())
    }

    fn move_hf_caret(&self, movement: CaretMovement, extend: bool) -> Result<JsValue, JsValue> {
        if self.navigate_hf_caret(movement, extend).is_some() {
            let result = Object::new();
            let mode = self.state.borrow().editing_mode;
            Reflect::set(
                &result,
                &JsValue::from_str("editingMode"),
                &JsValue::from_str(Self::hf_editing_mode_name(mode)),
            )?;
            self.write_hf_caret_to_result(&result)?;
            return Ok(result.into());
        }

        // Segmented stories and resolved fields retain their placeholder-safe
        // fallback when they cannot map to the shared painted layout.
        // Shift keeps the anchor where the selection started; a plain move
        // drops it (Word collapses the selection).
        {
            let mut state_mut = self.state.borrow_mut();
            if extend {
                if state_mut.hf_selection_anchor.is_none() {
                    state_mut.hf_selection_anchor = state_mut.hf_edit_cursor;
                }
            } else {
                state_mut.hf_selection_anchor = None;
            }
        }
        let (mode, variant, mut cursor) = {
            let state = self.state.borrow();
            let Some(cursor) = state.hf_edit_cursor else {
                return Ok(JsValue::NULL);
            };
            (state.editing_mode, state.hf_variant, cursor)
        };

        let line_count = {
            let state = self.state.borrow();
            let Some(word_doc) = state.word_doc.as_ref() else {
                return Ok(JsValue::NULL);
            };
            Self::variant_lines(word_doc, mode, variant)
                .map(|lines| lines.len().max(1))
                .unwrap_or(1)
        };

        let text_at = |line_index: usize, segment: HfSegment| -> String {
            let probe = HfEditCursor {
                page_index: cursor.page_index,
                line_index,
                segment,
                offset: 0,
            };
            self.hf_segment_text(mode, variant, &probe)
                .unwrap_or_default()
        };

        match movement {
            CaretMovement::Left | CaretMovement::WordLeft => {
                let text = text_at(cursor.line_index, cursor.segment);
                let previous = Self::hf_caret_boundaries(&text)
                    .into_iter()
                    .rfind(|boundary| *boundary < cursor.offset);
                if let Some(offset) = previous {
                    cursor.offset = offset;
                } else if cursor.line_index > 0 {
                    cursor.line_index -= 1;
                    let previous_text = text_at(cursor.line_index, cursor.segment);
                    cursor.offset = previous_text.len();
                }
            }
            CaretMovement::Right | CaretMovement::WordRight => {
                let text = text_at(cursor.line_index, cursor.segment);
                let next = Self::hf_caret_boundaries(&text)
                    .into_iter()
                    .find(|boundary| *boundary > cursor.offset);
                if let Some(offset) = next {
                    cursor.offset = offset;
                } else if cursor.line_index + 1 < line_count {
                    cursor.line_index += 1;
                    cursor.offset = 0;
                }
            }
            CaretMovement::Up => {
                if cursor.line_index > 0 {
                    cursor.line_index -= 1;
                    let text = text_at(cursor.line_index, cursor.segment);
                    cursor.offset = Self::nearest_hf_caret_boundary(&text, cursor.offset);
                }
            }
            CaretMovement::Down => {
                if cursor.line_index + 1 < line_count {
                    cursor.line_index += 1;
                    let text = text_at(cursor.line_index, cursor.segment);
                    cursor.offset = Self::nearest_hf_caret_boundary(&text, cursor.offset);
                }
            }
            CaretMovement::LineStart
            | CaretMovement::ParagraphStart
            | CaretMovement::DocumentStart
            | CaretMovement::ParagraphPrevious => {
                cursor.offset = 0;
            }
            CaretMovement::LineEnd
            | CaretMovement::ParagraphEnd
            | CaretMovement::DocumentEnd
            | CaretMovement::ParagraphNext => {
                cursor.offset = text_at(cursor.line_index, cursor.segment).len();
            }
        }

        {
            let mut state_mut = self.state.borrow_mut();
            state_mut.hf_edit_cursor = Some(cursor);
            state_mut.caret_goal_x = None;
        }

        let result = Object::new();
        Reflect::set(
            &result,
            &JsValue::from_str("editingMode"),
            &JsValue::from_str(Self::hf_editing_mode_name(mode)),
        )?;
        self.write_hf_caret_to_result(&result)?;
        Ok(result.into())
    }

    #[wasm_bindgen]
    pub fn set_font_size(&self, font_size: f32) -> Result<JsValue, JsValue> {
        if !font_size.is_finite() || font_size <= 0.0 {
            return Err(JsValue::from_str(
                "font size must be a positive finite number",
            ));
        }
        // The pending caret format and the header/footer path bypass
        // SetFontSizeCommand, so snap here too (10.3 -> 10.5, <= 1638pt).
        let font_size = crate::commands::set_font_size::word_font_size_points(font_size);
        self.apply_inline_style_headless(
            |inline| inline.set_font_size(font_size),
            |range| HeadlessCommand::SetFontSize {
                start_paragraph: range.anchor.paragraph,
                start_offset: range.anchor.offset,
                end_paragraph: range.focus.paragraph,
                end_offset: range.focus.offset,
                font_size,
            },
        )
    }

    /// Word's grow-font ribbon action: every run in the selection steps up
    /// relative to its own size; a collapsed caret arms the stepped size as
    /// pending formatting.
    #[wasm_bindgen(js_name = growFontSize)]
    pub fn grow_font_size(&self) -> Result<JsValue, JsValue> {
        self.adjust_font_size_impl(
            jade_docx_headless_api::HeadlessFontSizeDirection::Grow,
            jade_docx_headless_api::HeadlessFontSizeStep::Ladder,
        )
    }

    #[wasm_bindgen(js_name = shrinkFontSize)]
    pub fn shrink_font_size(&self) -> Result<JsValue, JsValue> {
        self.adjust_font_size_impl(
            jade_docx_headless_api::HeadlessFontSizeDirection::Shrink,
            jade_docx_headless_api::HeadlessFontSizeStep::Ladder,
        )
    }

    /// Word's Ctrl+]: every run in the selection grows by exactly one point
    /// (the ribbon's Ctrl+Shift+> walks the size list instead).
    #[wasm_bindgen(js_name = growFontSizeByPoint)]
    pub fn grow_font_size_by_point(&self) -> Result<JsValue, JsValue> {
        self.adjust_font_size_impl(
            jade_docx_headless_api::HeadlessFontSizeDirection::Grow,
            jade_docx_headless_api::HeadlessFontSizeStep::Point,
        )
    }

    /// Word's Ctrl+[.
    #[wasm_bindgen(js_name = shrinkFontSizeByPoint)]
    pub fn shrink_font_size_by_point(&self) -> Result<JsValue, JsValue> {
        self.adjust_font_size_impl(
            jade_docx_headless_api::HeadlessFontSizeDirection::Shrink,
            jade_docx_headless_api::HeadlessFontSizeStep::Point,
        )
    }

    fn adjust_font_size_impl(
        &self,
        direction: jade_docx_headless_api::HeadlessFontSizeDirection,
        step: jade_docx_headless_api::HeadlessFontSizeStep,
    ) -> Result<JsValue, JsValue> {
        use crate::commands::adjust_font_size::{step_font_size, FontSizeDirection, FontSizeStep};
        let caret_size = self.caret_effective_font_size();
        let pending_size = step_font_size(
            caret_size,
            match direction {
                jade_docx_headless_api::HeadlessFontSizeDirection::Grow => FontSizeDirection::Grow,
                jade_docx_headless_api::HeadlessFontSizeDirection::Shrink => {
                    FontSizeDirection::Shrink
                }
            },
            match step {
                jade_docx_headless_api::HeadlessFontSizeStep::Ladder => FontSizeStep::Ladder,
                jade_docx_headless_api::HeadlessFontSizeStep::Point => FontSizeStep::Point,
            },
        );
        self.apply_inline_style_headless(
            move |inline| inline.set_font_size(pending_size),
            move |range| HeadlessCommand::AdjustFontSize {
                start_paragraph: range.anchor.paragraph,
                start_offset: range.anchor.offset,
                end_paragraph: range.focus.paragraph,
                end_offset: range.focus.offset,
                direction,
                step,
            },
        )
    }

    /// Effective font size at the caret through the full style cascade,
    /// including pending inline formatting; falls back to the Japanese body
    /// default when nothing resolves.
    fn caret_effective_font_size(&self) -> f32 {
        const DEFAULT_FONT_SIZE_PT: f32 = 10.5;
        let state = self.state.borrow();
        let selection = state.selection.unwrap_or_else(|| {
            let renderer = self.renderer.borrow();
            renderer
                .selection_range()
                .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
        });
        let Some(word_doc) = state.word_doc.as_ref() else {
            return DEFAULT_FONT_SIZE_PT;
        };
        if state.inline_formatting.is_armed_for(&selection) {
            if let Some(size) = state.inline_formatting.font_size {
                return size;
            }
        }
        let position = if selection.is_collapsed() {
            selection.focus
        } else {
            selection.normalized().0
        };
        let Some(paragraph) = word_doc.paragraph(position.paragraph) else {
            return DEFAULT_FONT_SIZE_PT;
        };
        let mut run_start = 0usize;
        let mut direct: Option<&RunProperties> = None;
        for run in &paragraph.runs {
            let run_end = run_start + run.text.len();
            if position.offset < run_end || (position.offset == run_end && run_end == run_start) {
                direct = run.properties.as_ref();
                break;
            }
            if position.offset == run_end {
                // Word's caret formatting follows the character before the
                // caret, so a boundary reports the preceding run's size.
                direct = run.properties.as_ref();
                break;
            }
            run_start = run_end;
        }
        let para_style_id = paragraph
            .properties
            .as_ref()
            .and_then(|props| props.style.as_deref());
        direct
            .and_then(|props| props.font_size)
            .or_else(|| {
                word_doc.styles_part.as_ref().and_then(|styles| {
                    styles
                        .resolve_effective_run_properties(para_style_id, direct)
                        .and_then(|props| props.font_size)
                })
            })
            .unwrap_or(DEFAULT_FONT_SIZE_PT)
    }

    #[wasm_bindgen]
    pub fn set_font_family(&self, font_family: String) -> Result<JsValue, JsValue> {
        let normalized = font_family.trim();
        if normalized.is_empty() {
            return Err(JsValue::from_str("font family must be a non-empty string"));
        }
        let normalized_owned = normalized.to_string();
        self.apply_inline_style_headless(
            |inline| inline.set_font_family(normalized_owned.clone()),
            |range| HeadlessCommand::SetFontFamily {
                start_paragraph: range.anchor.paragraph,
                start_offset: range.anchor.offset,
                end_paragraph: range.focus.paragraph,
                end_offset: range.focus.offset,
                font_family: normalized_owned.clone(),
            },
        )
    }

    #[wasm_bindgen]
    pub fn clear_formatting(&self) -> Result<JsValue, JsValue> {
        self.apply_inline_style_headless(InlineFormattingState::clear, |range| {
            HeadlessCommand::ClearFormatting {
                start_paragraph: range.anchor.paragraph,
                start_offset: range.anchor.offset,
                end_paragraph: range.focus.paragraph,
                end_offset: range.focus.offset,
            }
        })
    }

    /// Word's ribbon 「すべての書式をクリア」: the selected text loses its
    /// direct character formatting (highlighting stays, as in Word), and the
    /// paragraphs it touches go back to the default paragraph style (標準)
    /// without direct paragraph formatting. One click is one undo step.
    /// Ctrl+Space ([`Self::clear_formatting`]) stays character-only.
    #[wasm_bindgen(js_name = "clearAllFormatting")]
    pub fn clear_all_formatting(&self) -> Result<JsValue, JsValue> {
        if self.state.borrow().preview_mode {
            return Ok(JsValue::NULL);
        }
        if !self.body_commands_allowed() {
            return self.apply_hf_run_format_and_reset_paragraph(
                HfRunFormat {
                    clear_visual: true,
                    ..HfRunFormat::default()
                },
                true,
            );
        }
        let selection = {
            let state = self.state.borrow();
            state.selection.unwrap_or_else(|| {
                self.renderer
                    .borrow()
                    .selection_range()
                    .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
            })
        };
        if selection.is_collapsed() {
            // Nothing selected: drop formatting armed at the caret too, so
            // the next keystroke types plain text.
            self.state.borrow_mut().inline_formatting.clear();
        }
        self.apply_paragraph_style_headless_many(move |start, end| {
            let mut commands = vec![
                HeadlessCommand::SetParagraphStyle {
                    start_paragraph: start,
                    end_paragraph: end,
                    style_id: None,
                },
                HeadlessCommand::ResetParagraphFormatting {
                    start_paragraph: start,
                    end_paragraph: end,
                },
            ];
            if !selection.is_collapsed() {
                commands.push(HeadlessCommand::ClearFormatting {
                    start_paragraph: selection.anchor.paragraph,
                    start_offset: selection.anchor.offset,
                    end_paragraph: selection.focus.paragraph,
                    end_offset: selection.focus.offset,
                });
            }
            commands
        })
    }

    #[wasm_bindgen]
    pub fn change_case(&self, mode: String) -> Result<JsValue, JsValue> {
        let mode = match mode.as_str() {
            "upper" => HeadlessChangeCaseMode::Upper,
            "lower" => HeadlessChangeCaseMode::Lower,
            "title" => HeadlessChangeCaseMode::Title,
            "sentence" => HeadlessChangeCaseMode::Sentence,
            "toggle" => HeadlessChangeCaseMode::Toggle,
            "halfWidth" => HeadlessChangeCaseMode::HalfWidth,
            "fullWidth" => HeadlessChangeCaseMode::FullWidth,
            "katakana" => HeadlessChangeCaseMode::Katakana,
            "hiragana" => HeadlessChangeCaseMode::Hiragana,
            _ => {
                return Err(JsValue::from_str(
                    "case mode must be upper, lower, title, sentence, toggle, halfWidth, \
                     fullWidth, katakana, or hiragana",
                ));
            }
        };
        self.apply_inline_style_headless(
            |_| {},
            |range| HeadlessCommand::ChangeCase {
                start_paragraph: range.anchor.paragraph,
                start_offset: range.anchor.offset,
                end_paragraph: range.focus.paragraph,
                end_offset: range.focus.offset,
                mode,
            },
        )
    }

    #[wasm_bindgen(js_name = setFontColor)]
    pub fn set_font_color(&self, color: String) -> Result<JsValue, JsValue> {
        let trimmed = color.trim().to_string();
        if trimmed.is_empty() {
            return Err(JsValue::from_str("color must be a non-empty string"));
        }
        // Ensure the color has a "#" prefix for CSS compatibility.
        let normalized = if !trimmed.starts_with('#') {
            format!("#{trimmed}")
        } else {
            trimmed
        };
        self.apply_inline_style_headless(
            |inline| inline.set_color(normalized.clone()),
            |range| HeadlessCommand::SetFontColor {
                start_paragraph: range.anchor.paragraph,
                start_offset: range.anchor.offset,
                end_paragraph: range.focus.paragraph,
                end_offset: range.focus.offset,
                color: normalized.clone(),
            },
        )
    }

    #[wasm_bindgen(js_name = setHighlightColor)]
    pub fn set_highlight_color(&self, color: String) -> Result<JsValue, JsValue> {
        let normalized = color.trim().to_string();
        if normalized.is_empty() {
            return Err(JsValue::from_str("color must be a non-empty string"));
        }
        self.apply_inline_style_headless(
            |inline| inline.set_highlight(normalized.clone()),
            |range| HeadlessCommand::SetHighlightColor {
                start_paragraph: range.anchor.paragraph,
                start_offset: range.anchor.offset,
                end_paragraph: range.focus.paragraph,
                end_offset: range.focus.offset,
                color: normalized.clone(),
            },
        )
    }

    /// Applies a paragraph-level command to the paragraphs covered by the
    /// current selection (or the caret paragraph when collapsed).
    fn apply_paragraph_style_headless(
        &self,
        command_factory: impl FnOnce(usize, usize) -> HeadlessCommand,
    ) -> Result<JsValue, JsValue> {
        self.apply_paragraph_style_headless_many(|start, end| vec![command_factory(start, end)])
    }

    /// The batch form of [`Self::apply_paragraph_style_headless`]. Word's table
    /// ribbon works on the whole selected cell range — 行の削除 removes every
    /// selected row, 塗りつぶし fills every selected cell — which jade applies
    /// as one headless command per row/cell. The batch runs inside one executor
    /// group so the ribbon click stays a single Ctrl+Z, and one document render
    /// covers all of it.
    fn apply_paragraph_style_headless_many(
        &self,
        command_factory: impl FnOnce(usize, usize) -> Vec<HeadlessCommand>,
    ) -> Result<JsValue, JsValue> {
        self.apply_paragraph_style_headless_many_with(
            command_factory,
            HeadlessApplyOverrides::default(),
        )
    }

    /// [`Self::apply_paragraph_style_headless_many`] with `overrides` for
    /// commands whose effect does not start at the selection.
    fn apply_paragraph_style_headless_many_with(
        &self,
        command_factory: impl FnOnce(usize, usize) -> Vec<HeadlessCommand>,
        overrides: HeadlessApplyOverrides,
    ) -> Result<JsValue, JsValue> {
        if self.state.borrow().preview_mode {
            return Ok(JsValue::NULL);
        }
        if !self.body_commands_allowed() {
            return self.apply_hf_paragraph_commands(command_factory(0, 0));
        }

        let selection_before = {
            let state_ref = self.state.borrow();
            state_ref.selection.unwrap_or_else(|| {
                let renderer = self.renderer.borrow();
                renderer
                    .selection_range()
                    .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
            })
        };
        let (start_para, end_para) = {
            let a = selection_before.anchor.paragraph;
            let f = selection_before.focus.paragraph;
            (a.min(f), a.max(f))
        };
        let mut dirty_from = overrides
            .dirty_from
            .map_or(start_para, |from| from.min(start_para));
        let edited_paragraph_index = selection_before.focus.paragraph;
        let old_height = {
            let renderer = self.renderer.borrow();
            renderer.cached_paragraph_height(edited_paragraph_index)
        };

        let commands = command_factory(start_para, end_para);
        if commands.is_empty() {
            return Ok(JsValue::NULL);
        }
        let refresh_header_footer_stories = commands.iter().any(|command| {
            matches!(
                command,
                HeadlessCommand::SetWatermark { .. }
                    | HeadlessCommand::SetDocumentTheme { .. }
                    | HeadlessCommand::SetHeaderFooterVariants { .. }
                    | HeadlessCommand::SetHeaderFooterLink { .. }
                    | HeadlessCommand::SetSectionLayout { .. }
            )
        });
        let mut refresh_styles = commands
            .iter()
            .any(|command| matches!(command, HeadlessCommand::SetDocumentTheme { .. }));
        let inserts_cover_at_start = commands
            .iter()
            .any(|command| matches!(command, HeadlessCommand::InsertCoverPage { .. }));
        let mutates_endnotes_globally = commands.iter().any(|command| {
            matches!(
                command,
                HeadlessCommand::UpdateEndnote { .. }
                    | HeadlessCommand::DeleteEndnote { .. }
                    | HeadlessCommand::ConvertFootnoteToEndnote { .. }
                    | HeadlessCommand::ConvertEndnoteToFootnote { .. }
            )
        });
        if inserts_cover_at_start || mutates_endnotes_globally {
            dirty_from = 0;
        }
        // Structural inserts (e.g. InsertTable) add brand-new body elements
        // *after* the anchor paragraph without changing the anchor's own
        // content or height. That violates the fast-replay assumption — if we
        // leave the fast-replay trigger wired up, the renderer will see the
        // anchor paragraph's height is unchanged, enable fast replay, and
        // early-exit the body-element loop right after the anchor. The newly
        // inserted table body element would then never be rendered, and the
        // pages containing it would not be marked dirty, so the main-thread
        // canvas keeps its stale bitmap until the next edit that happens to
        // dirty those pages. See CLAUDE.md 2026-04-21 entry.
        // `InsertTable`, `SetParagraphStyle`, and `SetCharacterStyle` break the fast-replay
        // assumption that the edited paragraph's height is the sole signal of
        // whether downstream paragraphs can be reused:
        //
        // - `InsertTable` adds brand-new body elements after the anchor without
        //   touching the anchor's own height (see CLAUDE.md 2026-04-21).
        // - `SetParagraphStyle` re-cascades the style's `rPr`/`pPr` onto the
        //   edited paragraphs, changing their font size / line height / spacing,
        //   but the cache still holds the pre-edit height until layout reruns.
        //   Leaving `set_fast_replay_trigger` wired up would let the renderer
        //   decide "old height == old height" and early-exit before it paints
        //   the newly-styled paragraph, and the caret would then be drawn from
        //   stale layout metrics — which the user perceives as "selection/caret
        //   got cleared" after clicking a style preset.
        // - `SetCharacterStyle` changes the effective run formatting through
        //   the package style cascade. The run's direct formatting can remain
        //   byte-for-byte unchanged, so fast replay may otherwise reuse the
        //   pre-style bitmap even though `w:rStyle` was authored correctly.
        // Table grid mutations (insert/delete row/column) likewise add or
        // remove body paragraphs inside the table without touching the
        // anchor paragraph's own height, so they need the same full-repaint
        // treatment as `InsertTable`.
        let is_structural_insert = commands.iter().any(|command| {
            matches!(
                command,
                HeadlessCommand::InsertTable { .. }
                | HeadlessCommand::InsertTableOfContents { .. }
                | HeadlessCommand::InsertCaption { .. }
                | HeadlessCommand::InsertBibliography { .. }
                | HeadlessCommand::InsertBlankPage { .. }
                | HeadlessCommand::InsertCoverPage { .. }
                | HeadlessCommand::UpdateEndnote { .. }
                | HeadlessCommand::DeleteEndnote { .. }
                | HeadlessCommand::ConvertFootnoteToEndnote { .. }
                | HeadlessCommand::ConvertEndnoteToFootnote { .. }
                | HeadlessCommand::InsertField { .. }
                | HeadlessCommand::InsertEquation { .. }
                | HeadlessCommand::ReplaceEquation { .. }
                | HeadlessCommand::InsertDateTimeField { .. }
                | HeadlessCommand::InsertEnclosedCharacter { .. }
                | HeadlessCommand::InsertTableFormula { .. }
                | HeadlessCommand::UpdateTableFormulas
                | HeadlessCommand::InsertIndex { .. }
                | HeadlessCommand::UpdateTableOfContents
                | HeadlessCommand::SetParagraphStyle { .. }
                | HeadlessCommand::SetCharacterStyle { .. }
                | HeadlessCommand::SetDropCap { .. }
                | HeadlessCommand::SetHeaderFooterVariants { .. }
                | HeadlessCommand::SetHeaderFooterLink { .. }
                // The Office-parity dialogs: each reflows past the edited
                // paragraph (pagination, typography, grid snapping, ruby line
                // height) or reshapes the body (sections, tables).
                | HeadlessCommand::SetParagraphOptions { .. }
                | HeadlessCommand::SetParagraphPagination { .. }
                | HeadlessCommand::SetParagraphTypography { .. }
                | HeadlessCommand::SetParagraphSnapToGrid { .. }
                | HeadlessCommand::SetJapaneseTypography { .. }
                | HeadlessCommand::SetRuby { .. }
                | HeadlessCommand::SetSectionLayout { .. }
                | HeadlessCommand::SetHyphenation { .. }
                | HeadlessCommand::SetTableCellSpacing { .. }
                | HeadlessCommand::SetTableCellTextDirection { .. }
                | HeadlessCommand::SetTableIndent { .. }
                | HeadlessCommand::SetTablePosition { .. }
                | HeadlessCommand::ConvertTextToTable { .. }
                | HeadlessCommand::ConvertTableToText { .. }
                | HeadlessCommand::SortTable { .. }
                | HeadlessCommand::InsertTableRow { .. }
                | HeadlessCommand::DeleteTableRow { .. }
                | HeadlessCommand::InsertTableColumn { .. }
                | HeadlessCommand::DeleteTableColumn { .. }
                | HeadlessCommand::DeleteTableCell { .. }
                | HeadlessCommand::InsertTableCell { .. }
                | HeadlessCommand::DeleteTable { .. }
                // Accepting/rejecting revisions rewrites runs across the whole
                // document (deleted runs vanish, inserted runs stay), reflowing
                // every paragraph after the change. Treat it as a structural
                // edit so the full document is re-laid-out and re-snapshotted
                // rather than fast-replayed from the caret paragraph.
                | HeadlessCommand::AcceptRevisions { .. }
                | HeadlessCommand::RejectRevisions { .. }
                // Page size/orientation/margins change the body box geometry,
                // so every page must be re-laid-out and re-snapshotted.
                | HeadlessCommand::SetPageOrientation { .. }
                | HeadlessCommand::SetPageSize { .. }
                | HeadlessCommand::SetPageMargins { .. }
                | HeadlessCommand::SetPageColumns { .. }
                | HeadlessCommand::SetPageBorders { .. }
                | HeadlessCommand::SetWatermark { .. }
                | HeadlessCommand::SetDocumentTheme { .. }
                | HeadlessCommand::InsertSectionBreak { .. }
                | HeadlessCommand::SetPageBackgroundColor { .. }
                // Comment add/update/delete/resolve change comment ranges and
                // markers within paragraphs; re-lay-out and re-snapshot the
                // whole document so the highlight bands update everywhere.
                | HeadlessCommand::AddComment { .. }
                | HeadlessCommand::UpdateComment { .. }
                | HeadlessCommand::DeleteComment { .. }
                | HeadlessCommand::ResolveComment { .. }
                // Line spacing changes the height of every selected paragraph,
                // so downstream content reflows — re-lay-out, don't fast-replay
                // off the focus paragraph's height alone.
                | HeadlessCommand::SetParagraphLineSpacing { .. }
                | HeadlessCommand::SetParagraphMetrics { .. }
                // A pasted line break adds a paragraph after the anchor, and a
                // replacement across paragraphs removes some; neither shows in
                // the anchor paragraph's height.
                | HeadlessCommand::InsertParagraphBreak { .. }
            ) || matches!(
                command,
                HeadlessCommand::ReplaceRange {
                    start_paragraph,
                    end_paragraph,
                    ..
                } if start_paragraph != end_paragraph
            )
        });
        // Every list item after the edited paragraphs takes its number from
        // the ones before it, so adding, removing or re-levelling an item
        // renumbers paragraphs whose own content and height are unchanged —
        // and a fast-replay stop would leave their old labels on the later
        // pages. Ribbon indent steps re-level list items rather than indent
        // them.
        let edits_list_item = {
            let state = self.state.borrow();
            state.word_doc.as_ref().is_some_and(|word_doc| {
                (start_para..=end_para).any(|index| {
                    word_doc
                        .paragraph(index)
                        .and_then(paragraph_list_id)
                        .is_some()
                })
            })
        };
        let is_structural_insert = is_structural_insert
            || commands.iter().any(|command| match command {
                HeadlessCommand::SetNumbering { .. }
                | HeadlessCommand::ApplyMultilevelList { .. }
                | HeadlessCommand::ApplyCustomMultilevelList { .. } => true,
                HeadlessCommand::AdjustParagraphIndent {
                    target: jade_docx_headless_api::HeadlessIndentTarget::Start,
                    ..
                } => edits_list_item,
                _ => false,
            });
        // A TOC insert creates `_Toc…` bookmarks on headings; the renderer's
        // bookmark→paragraph map must be rebuilt so the entries' PAGEREF page
        // numbers resolve.
        let rebuild_bookmarks = commands.iter().any(|command| {
            matches!(
                command,
                HeadlessCommand::InsertTableOfContents { .. }
                    | HeadlessCommand::UpdateTableOfContents
                    | HeadlessCommand::InsertCoverPage { .. }
                    | HeadlessCommand::ConvertFootnoteToEndnote { .. }
                    | HeadlessCommand::ConvertEndnoteToFootnote { .. }
            )
        });

        // Release the renderer's document Rc before the command mutates the
        // model, as the typing and history paths do: while the renderer holds
        // it, the first `paragraph_mut` deep-clones the whole `Document`.
        let _document_cache = self.drop_renderer_document_cache();
        let (options, numbering_part, header_lines, footer_lines, new_selection, bookmark_map) = {
            let mut state = self.state.borrow_mut();
            let mut word_doc = state
                .word_doc
                .take()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;

            state.selection = Some(selection_before);
            let style_count = |word_doc: &WordDocument| {
                word_doc
                    .styles_part
                    .as_ref()
                    .map_or(0, |styles| styles.style_count())
            };
            let style_count_before = style_count(&word_doc);
            {
                let request = HeadlessEditRequest { commands };
                let mut command_executor = std::mem::take(&mut state.command_executor);
                // One ribbon click, one undo step — even when the selected cell
                // range turned it into several commands.
                command_executor.begin_group();
                let apply_result = apply_headless_commands_with_executor_to_document(
                    &mut word_doc,
                    &mut command_executor,
                    state.selection,
                    &request,
                );
                match apply_result {
                    Ok(selection_after) => {
                        command_executor.end_group();
                        state.command_executor = command_executor;
                        state.selection = match overrides.selection_after {
                            Some(select) => select(&word_doc).or(selection_after),
                            None => selection_after,
                        };
                    }
                    Err(err) => {
                        // Roll the batch back: a command that failed half-way
                        // through must not leave the earlier rows deleted.
                        let mut selection = state.selection;
                        {
                            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                            command_executor.abort_group(&mut ctx);
                        }
                        state.command_executor = command_executor;
                        // Put the document back before bailing out — `word_doc`
                        // was taken out of the state above, so returning early
                        // without restoring it would wipe the loaded document
                        // from the editor on any failed command.
                        state.word_doc = Some(word_doc);
                        return Err(JsValue::from_str(&err.to_string()));
                    }
                }
            }

            // Applying Heading 1 to a document without it adds Word's built-in
            // definition; the renderer resolves numbering markers and empty
            // lines through its own copy of the styles, which must have it too.
            if style_count(&word_doc) != style_count_before {
                refresh_styles = true;
            }
            state.paged_images.clear();
            let options = state.options.clone();
            let numbering_part = word_doc.numbering_part.clone();
            let header_lines = word_doc.header_lines.clone();
            let footer_lines = word_doc.footer_lines.clone();
            let new_selection = state.selection;
            let bookmark_map: Option<std::collections::HashMap<String, usize>> =
                if rebuild_bookmarks {
                    Some(word_doc.bookmark_render_map())
                } else {
                    None
                };
            state.word_doc = Some(word_doc);
            (
                options,
                numbering_part,
                header_lines,
                footer_lines,
                new_selection,
                bookmark_map,
            )
        };

        let anchor_page = if inserts_cover_at_start {
            1
        } else {
            let renderer = self.renderer.borrow();
            renderer
                .cached_paragraph_start_page(dirty_from)
                .or_else(|| renderer.visible_page_range().as_ref().map(|r| *r.end()))
                .or_else(|| renderer.layout_page_limit())
                .unwrap_or(1)
        };

        let mut renderer = self.renderer.borrow_mut();
        if refresh_styles {
            if let Some(word_doc) = self.state.borrow().word_doc.as_ref() {
                renderer.set_style_context(
                    word_doc.styles_part.clone().map(Rc::new),
                    word_doc.loaded_theme_part().map(Rc::new),
                );
            }
        }
        let lazy_mode = self.state.borrow().lazy_mode;
        Self::ensure_visible_page_range_for_edit(&mut renderer, lazy_mode, anchor_page);
        let prev_limit: Option<Option<usize>> = if lazy_mode {
            let desired = Self::lazy_visible_end(&renderer, anchor_page);
            let prev = renderer.layout_page_limit();
            renderer.set_layout_page_limit(Some(desired));
            Some(prev)
        } else {
            renderer.set_layout_page_limit(None);
            None
        };

        renderer.set_incremental_dirty_from(dirty_from);
        for pi in start_para..=end_para {
            renderer.mark_paragraph_dirty(pi);
        }
        if is_structural_insert {
            // The inserted table pushes all content after the anchor paragraph
            // downward and introduces fresh body elements that no cache entry
            // covers. Force a full repaint so every affected page is
            // re-snapshotted and no fast-replay early-exit can skip rendering
            // the new table.
            //
            // A repaint alone is insufficient: structural commands can shift a
            // paragraph index to a different vertical position while leaving its
            // text/hash unchanged. In that case the layout cache would replay the
            // old Y coordinate (for example the empty tail split by a block
            // equation), placing the caret before the inserted body element.
            renderer.invalidate_layout_from(dirty_from);
            renderer.mark_full_repaint();
        } else if let Some(old_h) = old_height {
            if lazy_mode {
                renderer.set_fast_replay_trigger(edited_paragraph_index, old_h);
            } else {
                renderer.arm_fast_replay_for_complete_layout_edit(edited_paragraph_index, old_h);
            }
        }
        if let Some(map) = bookmark_map {
            renderer.set_bookmark_paragraphs(map);
            if let Some(word_doc) = self.state.borrow().word_doc.as_ref() {
                renderer.set_bookmark_note_targets(word_doc.bookmark_note_targets());
            }
        }

        {
            let state_ref = self.state.borrow();
            if let Some(word_doc) = state_ref.word_doc.as_ref() {
                if let Some(document) = word_doc.document.as_ref() {
                    if refresh_header_footer_stories {
                        sync_renderer_header_footer_stories(&mut renderer, word_doc);
                    }
                    renderer.render_with_numbering_rc(
                        Rc::clone(document),
                        &options,
                        numbering_part.as_ref(),
                        header_lines.as_deref(),
                        footer_lines.as_deref(),
                        Some(&word_doc.comments),
                    )?;
                }
            }
        }

        if let Some(prev) = prev_limit {
            renderer.set_layout_page_limit(prev);
        }

        if let Some(range) = new_selection {
            renderer.set_selection_range(Some(range));
        }
        let caret_metrics = renderer.caret_metrics();
        let selection_after = new_selection;
        let (snapshots, dirty_regions) = self.capture_paged_images(&mut renderer)?;
        drop(renderer);

        self.store_paged_images(snapshots);

        {
            let mut state_mut = self.state.borrow_mut();
            state_mut.selection = selection_after;
            state_mut.object_selection = None;
            state_mut.caret_goal_x = None;
        }

        let size = self.canvas_size()?;
        let pages = self.paged_images();
        let caret_value = match caret_metrics {
            Some(metrics) => to_value(&metrics).unwrap_or(JsValue::NULL),
            None => JsValue::NULL,
        };

        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &size)?;
        Reflect::set(&result, &JsValue::from_str("pages"), &pages)?;
        Reflect::set(&result, &JsValue::from_str("caret"), &caret_value)?;
        Reflect::set(&result, &JsValue::from_str("dirtyRegions"), &dirty_regions)?;
        {
            let renderer_ref = self.renderer.borrow();
            Self::write_selection_rects(&result, &renderer_ref, selection_after)?;
        }
        Self::write_history_flags(&result, self.history_flags())?;
        self.write_object_selection(&result)?;
        self.write_merge_state(&result)?;
        Ok(result.into())
    }

    #[wasm_bindgen(js_name = setAlignment)]
    pub fn set_alignment(&self, alignment: String) -> Result<JsValue, JsValue> {
        let editing_mode = self.state.borrow().editing_mode;
        if editing_mode != EditingMode::Body {
            return self.apply_hf_alignment(alignment, editing_mode);
        }
        self.apply_paragraph_style_headless(|start, end| HeadlessCommand::SetParagraphAlignment {
            start_paragraph: start,
            end_paragraph: end,
            alignment,
        })
    }

    #[wasm_bindgen(js_name = setParagraphDirection)]
    pub fn set_paragraph_direction(&self, direction: String) -> Result<JsValue, JsValue> {
        let bidi = match direction.trim().to_ascii_lowercase().as_str() {
            "ltr" => false,
            "rtl" => true,
            _ => return Err(JsValue::from_str("paragraph direction must be ltr or rtl")),
        };
        self.apply_paragraph_style_headless(|start, end| HeadlessCommand::SetParagraphDirection {
            start_paragraph: start,
            end_paragraph: end,
            bidi,
        })
    }

    /// Insert a `rows x columns` table after the paragraph that contains the
    /// current caret. Passing 0 for either dimension is treated as a no-op by
    /// the underlying headless command.
    #[wasm_bindgen(js_name = insertTable)]
    pub fn insert_table(&self, rows: usize, columns: usize) -> Result<JsValue, JsValue> {
        if rows == 0 || columns == 0 {
            return Ok(JsValue::NULL);
        }
        // Word places the table at the caret — above the paragraph from its
        // start, splitting it from the middle — and moves the caret into the
        // first cell. A range selection inserts at its start.
        let caret = {
            let state = self.state.borrow();
            state
                .selection
                .or_else(|| self.renderer.borrow().selection_range())
                .map(|range| range.normalized().0)
        };
        self.apply_paragraph_style_headless(|start, _end| HeadlessCommand::InsertTable {
            after_paragraph: start,
            rows,
            columns,
            column_widths: Vec::new(),
            caret_offset: caret
                .filter(|position| position.paragraph == start)
                .map(|position| position.offset),
        })
    }

    #[wasm_bindgen(js_name = insertTextBox)]
    #[allow(clippy::too_many_arguments)]
    pub fn insert_text_box(
        &self,
        shape: Option<String>,
        width_px: Option<f64>,
        height_px: Option<f64>,
        fill_color: Option<String>,
        border_color: Option<String>,
        border_width_px: Option<f64>,
        text: Option<String>,
        text_color: Option<String>,
        text_font_size_pt: Option<f32>,
        text_bold: Option<bool>,
        text_italic: Option<bool>,
        text_alignment: Option<String>,
    ) -> Result<JsValue, JsValue> {
        if self.state.borrow().preview_mode || self.state.borrow().editing_mode != EditingMode::Body
        {
            return Ok(JsValue::NULL);
        }

        let selection_before = {
            let state_ref = self.state.borrow();
            state_ref
                .selection
                .or_else(|| self.renderer.borrow().selection_range())
                .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
        };
        let caret = selection_before.focus;
        let paragraph_index = caret.paragraph;
        let shape_spec = shape
            .as_deref()
            .map(|shape| {
                let shape = TextBoxShape::from_public_name(shape).ok_or_else(|| {
                    JsValue::from_str(&format!("unsupported shape geometry {shape:?}"))
                })?;
                Ok::<_, JsValue>(ShapeInsertSpec {
                    shape,
                    width_px: width_px.unwrap_or(188.976),
                    height_px: height_px.unwrap_or(113.386),
                    fill_color,
                    border_color,
                    border_width_px: border_width_px.unwrap_or(1.0),
                    text: text.unwrap_or_default(),
                    text_color,
                    text_font_size_pt,
                    text_bold,
                    text_italic,
                    text_alignment,
                })
            })
            .transpose()?;
        let inserting_shape = shape_spec.is_some();
        let textbox_order_before = {
            let state_ref = self.state.borrow();
            state_ref
                .word_doc
                .as_ref()
                .and_then(|doc| doc.paragraph(paragraph_index))
                .map(|paragraph| {
                    paragraph
                        .runs
                        .iter()
                        .filter(|run| run.text_box.is_some())
                        .count()
                })
                .unwrap_or(0)
        };

        // See `apply_inline_style_with_scope`.
        let _document_cache = self.drop_renderer_document_cache();
        let (options, numbering_part, header_lines, footer_lines, target) = {
            let mut state = self.state.borrow_mut();
            let mut word_doc = state
                .word_doc
                .take()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;
            state.selection = Some(selection_before);
            let mut command_executor = std::mem::take(&mut state.command_executor);
            let command_result = {
                let mut ctx = CommandContext::new(&mut word_doc, &mut state.selection);
                let command: Box<dyn Command> = match shape_spec.clone() {
                    Some(spec) => Box::new(PackageSnapshotCommand::wrapping(Box::new(
                        InsertShapeCommand::new(paragraph_index, caret.offset, spec),
                    ))),
                    None => Box::new(InsertTextBoxCommand::new(paragraph_index, caret.offset)),
                };
                command_executor.execute(command, &mut ctx)
            };
            state.command_executor = command_executor;
            if let Err(err) = command_result {
                state.word_doc = Some(word_doc);
                return Err(JsValue::from_str(&err.to_string()));
            }
            state.paged_images.clear();

            let target = word_doc.paragraph(paragraph_index).and_then(|paragraph| {
                paragraph
                    .runs
                    .iter()
                    .enumerate()
                    .filter(|(_, run)| run.text_box.is_some())
                    .nth(textbox_order_before)
                    .map(|(run_index, _)| FloatingObjectTarget {
                        paragraph_index,
                        run_index,
                        kind: FloatingObjectKind::TextBox,
                    })
            });
            let options = state.options.clone();
            let numbering_part = word_doc.numbering_part.clone();
            let header_lines = word_doc.header_lines.clone();
            let footer_lines = word_doc.footer_lines.clone();
            state.word_doc = Some(word_doc);
            (options, numbering_part, header_lines, footer_lines, target)
        };

        let anchor_page = {
            let renderer = self.renderer.borrow();
            renderer
                .cached_paragraph_start_page(paragraph_index)
                .or_else(|| renderer.visible_page_range().as_ref().map(|r| *r.end()))
                .or_else(|| renderer.layout_page_limit())
                .unwrap_or(1)
        };

        let mut renderer = self.renderer.borrow_mut();
        let lazy_mode = self.state.borrow().lazy_mode;
        Self::ensure_visible_page_range_for_edit(&mut renderer, lazy_mode, anchor_page);
        let prev_limit: Option<Option<usize>> = if lazy_mode {
            let desired = Self::lazy_visible_end(&renderer, anchor_page);
            let prev = renderer.layout_page_limit();
            renderer.set_layout_page_limit(Some(desired));
            Some(prev)
        } else {
            renderer.set_layout_page_limit(None);
            None
        };
        renderer.disable_seek_replay_once();
        renderer.invalidate_layout_from(paragraph_index);
        renderer.invalidate_paragraph_wrap_cache_from(paragraph_index);
        if lazy_mode {
            let end_page_1idx = Self::lazy_visible_end(&renderer, anchor_page);
            renderer.mark_dirty_from_paragraph_to_end_bounded(paragraph_index, end_page_1idx);
        } else {
            renderer.mark_full_repaint();
        }

        {
            let state_ref = self.state.borrow();
            if let Some(word_doc) = state_ref.word_doc.as_ref() {
                if let Some(document) = word_doc.document.as_ref() {
                    renderer.render_with_numbering_rc(
                        Rc::clone(document),
                        &options,
                        numbering_part.as_ref(),
                        header_lines.as_deref(),
                        footer_lines.as_deref(),
                        Some(&word_doc.comments),
                    )?;
                }
            }
        }
        if let Some(prev) = prev_limit {
            renderer.set_layout_page_limit(prev);
        }
        let caret_metrics = renderer.caret_metrics();
        let selection_after = renderer.selection_range();
        let (snapshots, dirty_regions_js) = if lazy_mode {
            renderer.coalesce_dirty_regions();
            let dirty_regions = renderer.dirty_regions();
            let dirty_regions_js = Self::dirty_regions_to_js(&dirty_regions);
            renderer.clear_dirty_regions();
            (None, dirty_regions_js)
        } else {
            let (snapshots, dirty_regions) = self.capture_paged_images(&mut renderer)?;
            (snapshots, dirty_regions)
        };
        drop(renderer);

        self.store_paged_images(snapshots);
        let (preview_mode, history_flags) = {
            let mut state_mut = self.state.borrow_mut();
            state_mut.selection = selection_after;
            state_mut.object_selection = target;
            state_mut.caret_goal_x = None;
            (state_mut.preview_mode, state_mut.history_flags())
        };

        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &self.canvas_size()?)?;
        Reflect::set(&result, &JsValue::from_str("pages"), &self.paged_images())?;
        Reflect::set(
            &result,
            &JsValue::from_str("dirtyRegions"),
            &dirty_regions_js,
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("debugMutationKind"),
            &JsValue::from_str(if inserting_shape {
                "insert-shape"
            } else {
                "insert-text-box"
            }),
        )?;
        let caret_value = if preview_mode {
            JsValue::NULL
        } else {
            caret_metrics
                .as_ref()
                .map(|metrics| to_value(metrics).unwrap_or(JsValue::NULL))
                .unwrap_or(JsValue::NULL)
        };
        Reflect::set(&result, &JsValue::from_str("caret"), &caret_value)?;
        Self::write_history_flags(&result, history_flags)?;
        self.write_object_selection(&result)?;
        self.write_merge_state(&result)?;
        {
            let renderer_ref = self.renderer.borrow();
            Self::write_selection_rects(&result, &renderer_ref, selection_after)?;
        }
        Ok(result.into())
    }

    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(js_name = insertImage)]
    pub fn insert_image(
        &self,
        image_bytes: Vec<u8>,
        mime_type: String,
        width_px: f64,
        height_px: f64,
        title: Option<String>,
        description: Option<String>,
    ) -> Result<JsValue, JsValue> {
        if self.state.borrow().preview_mode || self.state.borrow().editing_mode != EditingMode::Body
        {
            return Ok(JsValue::NULL);
        }

        let selection_before = {
            let state_ref = self.state.borrow();
            state_ref
                .selection
                .or_else(|| self.renderer.borrow().selection_range())
                .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
        };
        let paragraph_index = selection_before.focus.paragraph;
        let existing_relationship_ids = {
            let state_ref = self.state.borrow();
            state_ref
                .word_doc
                .as_ref()
                .and_then(|doc| doc.paragraph(paragraph_index))
                .map(|paragraph| {
                    paragraph
                        .runs
                        .iter()
                        .filter_map(|run| run.image.as_ref()?.relationship_id.clone())
                        .collect::<std::collections::HashSet<_>>()
                })
                .unwrap_or_default()
        };

        // See `apply_inline_style_with_scope`.
        let _document_cache = self.drop_renderer_document_cache();
        let (options, numbering_part, header_lines, footer_lines, target) = {
            let mut state = self.state.borrow_mut();
            let mut word_doc = state
                .word_doc
                .take()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;
            state.selection = Some(selection_before);
            let mut command_executor = std::mem::take(&mut state.command_executor);
            let command_result = {
                let mut ctx = CommandContext::new(&mut word_doc, &mut state.selection);
                command_executor.execute(
                    Box::new(InsertImageCommand::new(
                        selection_before,
                        image_bytes,
                        mime_type,
                        width_px,
                        height_px,
                        title,
                        description,
                    )),
                    &mut ctx,
                )
            };
            state.command_executor = command_executor;
            if let Err(err) = command_result {
                state.word_doc = Some(word_doc);
                return Err(JsValue::from_str(&err.to_string()));
            }
            state.paged_images.clear();

            let target = word_doc.paragraph(paragraph_index).and_then(|paragraph| {
                paragraph
                    .runs
                    .iter()
                    .enumerate()
                    .find(|(_, run)| {
                        run.image
                            .as_ref()
                            .and_then(|image| image.relationship_id.as_ref())
                            .is_some_and(|id| !existing_relationship_ids.contains(id))
                    })
                    .map(|(run_index, _)| FloatingObjectTarget {
                        paragraph_index,
                        run_index,
                        kind: FloatingObjectKind::Image,
                    })
            });
            let options = state.options.clone();
            let numbering_part = word_doc.numbering_part.clone();
            let header_lines = word_doc.header_lines.clone();
            let footer_lines = word_doc.footer_lines.clone();
            state.word_doc = Some(word_doc);
            (options, numbering_part, header_lines, footer_lines, target)
        };

        let anchor_page = {
            let renderer = self.renderer.borrow();
            renderer
                .cached_paragraph_start_page(paragraph_index)
                .or_else(|| renderer.visible_page_range().as_ref().map(|r| *r.end()))
                .or_else(|| renderer.layout_page_limit())
                .unwrap_or(1)
        };

        let mut renderer = self.renderer.borrow_mut();
        let lazy_mode = self.state.borrow().lazy_mode;
        Self::ensure_visible_page_range_for_edit(&mut renderer, lazy_mode, anchor_page);
        let prev_limit: Option<Option<usize>> = if lazy_mode {
            let desired = Self::lazy_visible_end(&renderer, anchor_page);
            let prev = renderer.layout_page_limit();
            renderer.set_layout_page_limit(Some(desired));
            Some(prev)
        } else {
            renderer.set_layout_page_limit(None);
            None
        };
        renderer.disable_seek_replay_once();
        renderer.invalidate_layout_from(paragraph_index);
        renderer.invalidate_paragraph_wrap_cache_from(paragraph_index);
        if lazy_mode {
            let end_page_1idx = Self::lazy_visible_end(&renderer, anchor_page);
            renderer.mark_dirty_from_paragraph_to_end_bounded(paragraph_index, end_page_1idx);
        } else {
            renderer.mark_full_repaint();
        }

        {
            let state_ref = self.state.borrow();
            if let Some(word_doc) = state_ref.word_doc.as_ref() {
                if let Some(document) = word_doc.document.as_ref() {
                    renderer.render_with_numbering_rc(
                        Rc::clone(document),
                        &options,
                        numbering_part.as_ref(),
                        header_lines.as_deref(),
                        footer_lines.as_deref(),
                        Some(&word_doc.comments),
                    )?;
                }
            }
        }
        if let Some(prev) = prev_limit {
            renderer.set_layout_page_limit(prev);
        }
        let caret_metrics = renderer.caret_metrics();
        let selection_after = renderer.selection_range();
        let (snapshots, dirty_regions_js) = if lazy_mode {
            renderer.coalesce_dirty_regions();
            let dirty_regions = renderer.dirty_regions();
            let dirty_regions_js = Self::dirty_regions_to_js(&dirty_regions);
            renderer.clear_dirty_regions();
            (None, dirty_regions_js)
        } else {
            let (snapshots, dirty_regions) = self.capture_paged_images(&mut renderer)?;
            (snapshots, dirty_regions)
        };
        drop(renderer);

        self.store_paged_images(snapshots);
        let (preview_mode, history_flags) = {
            let mut state_mut = self.state.borrow_mut();
            state_mut.selection = selection_after;
            state_mut.object_selection = target;
            state_mut.caret_goal_x = None;
            (state_mut.preview_mode, state_mut.history_flags())
        };

        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &self.canvas_size()?)?;
        Reflect::set(&result, &JsValue::from_str("pages"), &self.paged_images())?;
        Reflect::set(
            &result,
            &JsValue::from_str("dirtyRegions"),
            &dirty_regions_js,
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("debugMutationKind"),
            &JsValue::from_str("insert-image"),
        )?;
        let caret_value = if preview_mode {
            JsValue::NULL
        } else {
            caret_metrics
                .as_ref()
                .map(|metrics| to_value(metrics).unwrap_or(JsValue::NULL))
                .unwrap_or(JsValue::NULL)
        };
        Reflect::set(&result, &JsValue::from_str("caret"), &caret_value)?;
        Self::write_history_flags(&result, history_flags)?;
        self.write_object_selection(&result)?;
        self.write_merge_state(&result)?;
        {
            let renderer_ref = self.renderer.borrow();
            Self::write_selection_rects(&result, &renderer_ref, selection_after)?;
        }
        Ok(result.into())
    }

    /// Insert an editable native Word chart. The JSON payload is the public
    /// `RunChart` wire shape (`kind`, `title`, `categories`, `series`).
    #[wasm_bindgen(js_name = insertChart)]
    pub fn insert_chart(
        &self,
        chart_json: String,
        width_px: f64,
        height_px: f64,
    ) -> Result<JsValue, JsValue> {
        if self.state.borrow().preview_mode || self.state.borrow().editing_mode != EditingMode::Body
        {
            return Ok(JsValue::NULL);
        }
        let chart: RunChart = serde_json::from_str(&chart_json)
            .map_err(|error| JsValue::from_str(&format!("invalid chart payload: {error}")))?;
        let selection_before = {
            let state = self.state.borrow();
            state
                .selection
                .or_else(|| self.renderer.borrow().selection_range())
                .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
        };
        let paragraph_index = selection_before.focus.paragraph;
        let existing_relationship_ids = self
            .state
            .borrow()
            .word_doc
            .as_ref()
            .and_then(|document| document.paragraph(paragraph_index))
            .map(|paragraph| {
                paragraph
                    .runs
                    .iter()
                    .filter_map(|run| {
                        let image = run.image.as_ref()?;
                        image.chart.as_ref()?;
                        image.relationship_id.clone()
                    })
                    .collect::<std::collections::HashSet<_>>()
            })
            .unwrap_or_default();
        let spec = ChartInsertSpec {
            kind: chart.kind,
            title: chart.title,
            categories: chart.categories,
            series: chart.series,
            width_px,
            height_px,
        };
        let result = self.apply_inline_style(
            |_| {},
            move |range| Box::new(InsertChartCommand::new(range, spec.clone())),
        )?;
        let target = self
            .state
            .borrow()
            .word_doc
            .as_ref()
            .and_then(|document| document.paragraph(paragraph_index))
            .and_then(|paragraph| {
                paragraph
                    .runs
                    .iter()
                    .enumerate()
                    .find_map(|(run_index, run)| {
                        let image = run.image.as_ref()?;
                        image.chart.as_ref()?;
                        let relationship_id = image.relationship_id.as_ref()?;
                        (!existing_relationship_ids.contains(relationship_id)).then_some(
                            FloatingObjectTarget {
                                paragraph_index,
                                run_index,
                                kind: FloatingObjectKind::Image,
                            },
                        )
                    })
            });
        self.state.borrow_mut().object_selection = target;
        let result: Object = result
            .dyn_into()
            .map_err(|_| JsValue::from_str("chart insertion did not return an object"))?;
        Reflect::set(
            &result,
            &JsValue::from_str("debugMutationKind"),
            &JsValue::from_str("insert-chart"),
        )?;
        self.write_object_selection(&result)?;
        Ok(result.into())
    }

    /// Replaces the title/categories/series cache of the currently selected
    /// native chart while preserving its position and dimensions.
    #[wasm_bindgen(js_name = updateSelectedChart)]
    pub fn update_selected_chart(&self, chart_json: String) -> Result<JsValue, JsValue> {
        let chart: RunChart = serde_json::from_str(&chart_json)
            .map_err(|error| JsValue::from_str(&format!("invalid chart payload: {error}")))?;
        let target = self
            .state
            .borrow()
            .object_selection
            .ok_or_else(|| JsValue::from_str("no chart is selected"))?;
        let is_chart = self
            .state
            .borrow()
            .word_doc
            .as_ref()
            .and_then(|document| document.paragraph(target.paragraph_index))
            .and_then(|paragraph| paragraph.runs.get(target.run_index))
            .and_then(|run| run.image.as_ref())
            .is_some_and(|image| image.chart.is_some());
        if !is_chart {
            return Err(JsValue::from_str("selected object is not a chart"));
        }
        self.apply_inline_style(
            |_| {},
            move |_range| Box::new(UpdateChartCommand::new(target, chart.clone())),
        )
    }

    /// Word の「レイアウト」タブ相当のコンテキスト照会: キャレットが表セル内に
    /// あるとき、その表のグリッド座標と寸法を返す。表外なら null。
    /// IDE は行/列ボタンの活性状態の判定に使う。
    #[wasm_bindgen(js_name = caretTableContext)]
    pub fn caret_table_context(&self) -> Result<JsValue, JsValue> {
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        let (
            column_widths,
            table_alignment,
            repeat_header,
            cant_split,
            cell_shading,
            cell_vertical_alignment,
            cell_text_direction,
            table_borders,
            cell_borders,
            table_cell_margins,
            cell_margins,
            effective_cell_margins,
            row_height,
            row_height_value,
            row_height_rule,
            table_style_id,
            table_width,
            table_layout,
            table_text_wrapping,
            look_first_row,
            banded_rows,
        ) = {
            let state = self.state.borrow();
            let word_doc = state
                .word_doc
                .as_ref()
                .ok_or_else(|| JsValue::from_str("Caret document is unavailable"))?;
            let table = word_doc
                .table_by_order(cell.table_order_index)
                .ok_or_else(|| JsValue::from_str("Caret table is unavailable"))?;
            let row = table.rows.get(cell.row);
            // Logical grid coordinates include omitted columns (`gridBefore`),
            // whereas `row.cells` contains physical cells only.
            let mut grid_column =
                row.and_then(|candidate| candidate.grid_before).unwrap_or(0) as usize;
            let caret_cell = row.and_then(|row| {
                row.cells.iter().find(|candidate| {
                    let span = candidate.span.unwrap_or(1).max(1) as usize;
                    let contains = cell.column >= grid_column
                        && cell.column < grid_column.saturating_add(span);
                    grid_column = grid_column.saturating_add(span);
                    contains
                })
            });
            let direct_margins = caret_cell.and_then(|cell| cell.margins.as_ref());
            let row_margins = row.and_then(|row| row.cell_margins.as_ref());
            let table_margins = table.effective_cell_margins();
            let effective_margins = TableCellMargins {
                top: Some(
                    resolve_cell_margin_edge_points(
                        direct_margins,
                        row_margins,
                        table_margins,
                        |margins| margins.top,
                    )
                    .unwrap_or(DEFAULT_VERTICAL_CELL_MARGIN_PT),
                ),
                bottom: Some(
                    resolve_cell_margin_edge_points(
                        direct_margins,
                        row_margins,
                        table_margins,
                        |margins| margins.bottom,
                    )
                    .unwrap_or(DEFAULT_VERTICAL_CELL_MARGIN_PT),
                ),
                left: Some(
                    resolve_cell_margin_edge_points(
                        direct_margins,
                        row_margins,
                        table_margins,
                        |margins| margins.physical_left(table.bidi_visual),
                    )
                    .unwrap_or(6.0),
                ),
                right: Some(
                    resolve_cell_margin_edge_points(
                        direct_margins,
                        row_margins,
                        table_margins,
                        |margins| margins.physical_right(table.bidi_visual),
                    )
                    .unwrap_or(6.0),
                ),
                ..TableCellMargins::default()
            };
            (
                (0..cell.column_count)
                    .map(|index| table.columns.get(index).and_then(|column| column.width))
                    .collect::<Vec<_>>(),
                table
                    .style
                    .get("text-align")
                    .cloned()
                    .unwrap_or_else(|| "left".to_string()),
                row.is_some_and(|row| row.is_header),
                row.is_some_and(|row| row.cant_split),
                caret_cell
                    .and_then(|cell| cell.style.get("background-color"))
                    .cloned(),
                caret_cell
                    .and_then(|cell| cell.vertical_align.clone())
                    .unwrap_or_else(|| "top".to_string()),
                caret_cell.and_then(|cell| cell.text_direction),
                table.borders.clone(),
                word_doc
                    .cell_borders_from_style(cell.table_order_index, cell.row, cell.column)
                    .ok(),
                table.cell_margins.clone(),
                caret_cell.and_then(|cell| cell.margins.clone()),
                effective_margins,
                visual_height_for_resize(table, cell.row),
                row.and_then(|row| row.height),
                row.and_then(|row| row.height_rule),
                table.style_name.clone(),
                Some(
                    table
                        .style
                        .get("width")
                        .cloned()
                        .unwrap_or_else(|| "auto".to_string()),
                ),
                match table.table_layout {
                    Some(crate::document::TableLayoutMode::Fixed) => "fixed",
                    _ => "autofit",
                },
                if table
                    .style
                    .get("table-position")
                    .is_some_and(|value| value.eq_ignore_ascii_case("floating"))
                {
                    "around"
                } else {
                    "none"
                },
                table.look.first_row_enabled(),
                table.look.horizontal_banding_enabled(),
            )
        };
        let result = Object::new();
        Reflect::set(
            &result,
            &JsValue::from_str("tableOrderIndex"),
            &JsValue::from_f64(cell.table_order_index as f64),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("row"),
            &JsValue::from_f64(cell.row as f64),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("column"),
            &JsValue::from_f64(cell.column as f64),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("rowCount"),
            &JsValue::from_f64(cell.row_count as f64),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("columnCount"),
            &JsValue::from_f64(cell.column_count as f64),
        )?;
        let widths = Array::new();
        for width in column_widths {
            widths.push(
                &width
                    .map(|value| JsValue::from_f64(value as f64))
                    .unwrap_or(JsValue::NULL),
            );
        }
        Reflect::set(&result, &JsValue::from_str("columnWidths"), &widths)?;
        Reflect::set(
            &result,
            &JsValue::from_str("tableAlignment"),
            &JsValue::from_str(&table_alignment),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("repeatHeader"),
            &JsValue::from_bool(repeat_header),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("cantSplit"),
            &JsValue::from_bool(cant_split),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("cellShading"),
            &cell_shading
                .map(|color| JsValue::from_str(&color))
                .unwrap_or(JsValue::NULL),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("cellVerticalAlignment"),
            &JsValue::from_str(&cell_vertical_alignment),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("cellTextDirection"),
            &cell_text_direction
                .map(|direction| to_value(&direction))
                .transpose()?
                .unwrap_or(JsValue::NULL),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("tableBorders"),
            &to_value(&table_borders)?,
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("cellBorders"),
            &to_value(&cell_borders)?,
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("tableCellMargins"),
            &to_value(&table_cell_margins)?,
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("cellMargins"),
            &to_value(&cell_margins)?,
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("effectiveCellMargins"),
            &to_value(&effective_cell_margins)?,
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("rowHeight"),
            &row_height
                .map(|height| JsValue::from_f64(height as f64))
                .unwrap_or(JsValue::NULL),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("rowHeightValue"),
            &row_height_value
                .map(|height| JsValue::from_f64(height as f64))
                .unwrap_or(JsValue::NULL),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("rowHeightRule"),
            &to_value(&row_height_rule)?,
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("tableStyleId"),
            &table_style_id
                .map(|style_id| JsValue::from_str(&style_id))
                .unwrap_or(JsValue::NULL),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("tableWidth"),
            &table_width
                .map(|width| JsValue::from_str(&width))
                .unwrap_or(JsValue::NULL),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("tableLayout"),
            &JsValue::from_str(table_layout),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("tableTextWrapping"),
            &JsValue::from_str(table_text_wrapping),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("lookFirstRow"),
            &JsValue::from_bool(look_first_row),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("bandedRows"),
            &JsValue::from_bool(banded_rows),
        )?;
        Ok(result.into())
    }

    /// Package-local table styles suitable for a host ribbon gallery.
    #[wasm_bindgen(js_name = tableStyleCatalog)]
    pub fn table_style_catalog(&self) -> Result<JsValue, JsValue> {
        let styles = self
            .state
            .borrow()
            .word_doc
            .as_ref()
            .map(|word_doc| word_doc.table_style_infos())
            .unwrap_or_default();
        to_value(&styles).map_err(|err| JsValue::from_str(&format!("Table styles: {err}")))
    }

    /// Current package-local style id for the caret table, or null outside a
    /// table / when the table has no named style.
    #[wasm_bindgen(js_name = currentTableStyleId)]
    pub fn current_table_style_id(&self) -> Result<JsValue, JsValue> {
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        let style_id = self
            .state
            .borrow()
            .word_doc
            .as_ref()
            .and_then(|word_doc| word_doc.table_style_id(cell.table_order_index).ok())
            .flatten();
        Ok(style_id
            .map(|style_id| JsValue::from_str(&style_id))
            .unwrap_or(JsValue::NULL))
    }

    /// Set the full logical grid width of the table containing the caret.
    /// Values are points and must contain one entry per grid column.
    #[wasm_bindgen(js_name = setTableColumnWidths)]
    pub fn set_table_column_widths(&self, widths: JsValue) -> Result<JsValue, JsValue> {
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        let widths: Vec<f32> = serde_wasm_bindgen::from_value(widths)
            .map_err(|err| JsValue::from_str(&format!("Invalid table column widths: {err}")))?;
        self.apply_paragraph_style_headless(move |_, _| HeadlessCommand::SetTableColumnWidths {
            table_order_index: cell.table_order_index,
            widths,
        })
    }

    #[wasm_bindgen(js_name = setTableWidth)]
    pub fn set_table_width(&self, width: String) -> Result<JsValue, JsValue> {
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        self.apply_paragraph_style_headless(move |_, _| HeadlessCommand::SetTableWidth {
            table_order_index: cell.table_order_index,
            width,
        })
    }

    /// Apply a table properties dialog as one atomic history entry.
    #[wasm_bindgen(js_name = setTableProperties)]
    pub fn set_table_properties(&self, spec: JsValue) -> Result<JsValue, JsValue> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Properties {
            width: Option<String>,
            column_widths: Option<Vec<f32>>,
            alignment: Option<String>,
            wrapping: Option<HeadlessTableTextWrapping>,
        }
        let spec: Properties = serde_wasm_bindgen::from_value(spec)
            .map_err(|error| JsValue::from_str(&format!("Invalid table properties: {error}")))?;
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        let table_order_index = cell.table_order_index;
        self.apply_paragraph_style_headless_many(move |_, _| {
            let mut commands = Vec::new();
            if let Some(width) = spec.width {
                commands.push(HeadlessCommand::SetTableWidth {
                    table_order_index,
                    width,
                });
            }
            if let Some(widths) = spec.column_widths {
                commands.push(HeadlessCommand::SetTableColumnWidths {
                    table_order_index,
                    widths,
                });
            }
            if let Some(alignment) = spec.alignment {
                commands.push(HeadlessCommand::SetTableAlignment {
                    table_order_index,
                    alignment,
                });
            }
            if let Some(wrapping) = spec.wrapping {
                commands.push(HeadlessCommand::SetTableTextWrapping {
                    table_order_index,
                    wrapping,
                });
            }
            commands
        })
    }

    #[wasm_bindgen(js_name = setTableFit)]
    pub fn set_table_fit(&self, mode: String) -> Result<JsValue, JsValue> {
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        let mode = match mode.as_str() {
            "contents" => HeadlessTableFitMode::Contents,
            "window" => HeadlessTableFitMode::Window,
            "fixed" => HeadlessTableFitMode::Fixed,
            _ => {
                return Err(JsValue::from_str(
                    "Table fit mode must be contents, window or fixed",
                ))
            }
        };
        self.apply_paragraph_style_headless(move |_, _| HeadlessCommand::SetTableFit {
            table_order_index: cell.table_order_index,
            mode,
        })
    }

    #[wasm_bindgen(js_name = setTableTextWrapping)]
    pub fn set_table_text_wrapping(&self, wrapping: String) -> Result<JsValue, JsValue> {
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        let wrapping = match wrapping.as_str() {
            "none" => HeadlessTableTextWrapping::None,
            "around" => HeadlessTableTextWrapping::Around,
            _ => {
                return Err(JsValue::from_str(
                    "Table text wrapping must be none or around",
                ))
            }
        };
        self.apply_paragraph_style_headless(move |_, _| HeadlessCommand::SetTableTextWrapping {
            table_order_index: cell.table_order_index,
            wrapping,
        })
    }

    #[wasm_bindgen(js_name = setTableLook)]
    pub fn set_table_look(
        &self,
        first_row: Option<bool>,
        banded_rows: Option<bool>,
    ) -> Result<JsValue, JsValue> {
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        self.apply_paragraph_style_headless(move |_, _| HeadlessCommand::SetTableLook {
            table_order_index: cell.table_order_index,
            first_row,
            banded_rows,
            last_row: None,
            first_column: None,
            last_column: None,
            banded_columns: None,
        })
    }

    /// Word's 表の分割 (Ctrl+Shift+Enter): split the caret table above the
    /// caret row. No-op (null) when the caret is not inside a table.
    #[wasm_bindgen(js_name = splitTable)]
    pub fn split_table(&self) -> Result<JsValue, JsValue> {
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        self.apply_paragraph_style_headless(move |_, _| HeadlessCommand::SplitTable {
            table_order_index: cell.table_order_index,
            row_index: cell.row,
        })
    }

    /// Word's セルの分割: split the caret cell into `columns` × `rows` cells.
    #[wasm_bindgen(js_name = splitTableCell)]
    pub fn split_table_cell(&self, columns: usize, rows: usize) -> Result<JsValue, JsValue> {
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        self.apply_paragraph_style_headless(move |_, _| HeadlessCommand::SplitTableCell {
            table_order_index: cell.table_order_index,
            row: cell.row,
            column: cell.column,
            columns,
            rows,
        })
    }

    /// Word's 行の高さを揃える for the table containing the caret: every row
    /// takes the average rendered row height.
    #[wasm_bindgen(js_name = distributeTableRows)]
    pub fn distribute_table_rows(&self) -> Result<JsValue, JsValue> {
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        let height = self
            .rendered_table_extent_pt(cell.table_order_index)
            .map(|(average_row_height, _)| average_row_height);
        self.apply_paragraph_style_headless(move |_, _| HeadlessCommand::DistributeTableRows {
            table_order_index: cell.table_order_index,
            height,
        })
    }

    /// Word's 列の幅を揃える for the table containing the caret: the rendered
    /// table width is shared evenly between the grid columns.
    #[wasm_bindgen(js_name = distributeTableColumns)]
    pub fn distribute_table_columns(&self) -> Result<JsValue, JsValue> {
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        let total_width = self
            .rendered_table_extent_pt(cell.table_order_index)
            .map(|(_, total_width)| total_width);
        self.apply_paragraph_style_headless(move |_, _| HeadlessCommand::DistributeTableColumns {
            table_order_index: cell.table_order_index,
            total_width,
        })
    }

    /// `(average row height, table width)` in points, read from the rendered
    /// cell registry of the table — the numbers Word distributes, which the
    /// model does not know for auto-sized rows and columns.
    fn rendered_table_extent_pt(&self, table_order_index: usize) -> Option<(f32, f32)> {
        let renderer = self.renderer.borrow();
        let layouts = renderer.cell_layouts_debug_ref();
        let table_id = layouts
            .iter()
            .map(|layout| layout.table_id)
            .find(|table_id| renderer.table_order_for(*table_id) == Some(table_order_index))?;
        let mut rows: std::collections::BTreeMap<usize, (f64, f64)> =
            std::collections::BTreeMap::new();
        let mut left = f64::INFINITY;
        let mut right = f64::NEG_INFINITY;
        for layout in layouts.iter().filter(|layout| layout.table_id == table_id) {
            rows.entry(layout.row)
                .and_modify(|(top, bottom)| {
                    *top = top.min(layout.top);
                    *bottom = bottom.max(layout.bottom);
                })
                .or_insert((layout.top, layout.bottom));
            left = left.min(layout.left);
            right = right.max(layout.right);
        }
        if rows.is_empty() || right <= left {
            return None;
        }
        let average_height_px = rows
            .values()
            .map(|(top, bottom)| (bottom - top).max(0.0))
            .sum::<f64>()
            / rows.len() as f64;
        Some((
            Self::px_to_points(average_height_px),
            Self::px_to_points(right - left),
        ))
    }

    /// Set how the table containing the caret sits between the page margins.
    #[wasm_bindgen(js_name = setTableAlignment)]
    pub fn set_table_alignment(&self, alignment: String) -> Result<JsValue, JsValue> {
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        self.apply_paragraph_style_headless(move |_, _| HeadlessCommand::SetTableAlignment {
            table_order_index: cell.table_order_index,
            alignment,
        })
    }

    /// Fill or clear the background of every selected cell (Word's
    /// 塗りつぶし), or of the caret's cell when nothing is selected.
    #[wasm_bindgen(js_name = setTableCellShading)]
    pub fn set_table_cell_shading(&self, color: Option<String>) -> Result<JsValue, JsValue> {
        let Some(range) = self.caret_table_range() else {
            return Ok(JsValue::NULL);
        };
        self.apply_paragraph_style_headless_many(move |_, _| {
            range
                .cells
                .iter()
                .map(|cell| HeadlessCommand::SetTableCellShading {
                    table_order_index: range.table_order_index,
                    row: cell.row,
                    column: cell.column,
                    color: color.clone(),
                })
                .collect()
        })
    }

    /// Apply a deterministic direct-formatting preset to the caret table.
    #[wasm_bindgen(js_name = applyTableStylePreset)]
    pub fn apply_table_style_preset(&self, preset: String) -> Result<JsValue, JsValue> {
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        self.apply_paragraph_style_headless(move |_, _| HeadlessCommand::ApplyTableStylePreset {
            table_order_index: cell.table_order_index,
            preset,
        })
    }

    /// Apply or clear a document-defined table style on the caret table.
    #[wasm_bindgen(js_name = applyTableNamedStyle)]
    pub fn apply_table_named_style(&self, style_id: Option<String>) -> Result<JsValue, JsValue> {
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        self.apply_paragraph_style_headless(move |_, _| HeadlessCommand::ApplyTableNamedStyle {
            table_order_index: cell.table_order_index,
            style_id,
        })
    }

    /// Set vertical alignment for every selected cell (Word applies the
    /// 配置 buttons to the whole cell range).
    #[wasm_bindgen(js_name = setTableCellVerticalAlignment)]
    pub fn set_table_cell_vertical_alignment(&self, alignment: String) -> Result<JsValue, JsValue> {
        let Some(range) = self.caret_table_range() else {
            return Ok(JsValue::NULL);
        };
        self.apply_paragraph_style_headless_many(move |_, _| {
            range
                .cells
                .iter()
                .map(|cell| HeadlessCommand::SetTableCellVerticalAlignment {
                    table_order_index: range.table_order_index,
                    row: cell.row,
                    column: cell.column,
                    alignment: alignment.clone(),
                })
                .collect()
        })
    }

    /// Update one or more table-level border edges. Omitted edges are kept.
    #[wasm_bindgen(js_name = setTableBorders)]
    pub fn set_table_borders(&self, spec: JsValue) -> Result<JsValue, JsValue> {
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        let spec: TableBordersInput = serde_wasm_bindgen::from_value(spec)
            .map_err(|err| JsValue::from_str(&format!("Invalid table borders: {err}")))?;
        if spec.is_empty() {
            return Err(JsValue::from_str(
                "At least one table border edge is required",
            ));
        }
        self.apply_paragraph_style_headless(move |_, _| HeadlessCommand::SetTableBorders {
            table_order_index: cell.table_order_index,
            top: spec.top,
            bottom: spec.bottom,
            left: spec.left,
            right: spec.right,
            inside_h: spec.inside_h,
            inside_v: spec.inside_v,
        })
    }

    /// Update one or more borders of the cell containing the caret.
    #[wasm_bindgen(js_name = setTableCellBorders)]
    pub fn set_table_cell_borders(&self, spec: JsValue) -> Result<JsValue, JsValue> {
        let Some(range) = self.caret_table_range() else {
            return Ok(JsValue::NULL);
        };
        let spec: TableBordersInput = serde_wasm_bindgen::from_value(spec)
            .map_err(|err| JsValue::from_str(&format!("Invalid table cell borders: {err}")))?;
        if spec.is_empty() {
            return Err(JsValue::from_str(
                "At least one cell border edge is required",
            ));
        }
        if spec.inside_h.is_some() || spec.inside_v.is_some() {
            return Err(JsValue::from_str(
                "Cell borders support top, bottom, left and right only",
            ));
        }
        self.apply_paragraph_style_headless_many(move |_, _| {
            range
                .cells
                .iter()
                .map(|cell| HeadlessCommand::SetTableCellBorders {
                    table_order_index: range.table_order_index,
                    row: cell.row,
                    column: cell.column,
                    top: spec.top.clone(),
                    bottom: spec.bottom.clone(),
                    left: spec.left.clone(),
                    right: spec.right.clone(),
                })
                .collect()
        })
    }

    /// Update one or more default cell margins for the table containing the caret.
    #[wasm_bindgen(js_name = setTableDefaultCellMargins)]
    pub fn set_table_default_cell_margins(&self, spec: JsValue) -> Result<JsValue, JsValue> {
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        let spec: TableCellMarginsInput = serde_wasm_bindgen::from_value(spec)
            .map_err(|err| JsValue::from_str(&format!("Invalid table cell margins: {err}")))?;
        if spec.is_empty() {
            return Err(JsValue::from_str(
                "At least one table cell margin is required",
            ));
        }
        self.apply_paragraph_style_headless(move |_, _| HeadlessCommand::SetTableCellMargins {
            table_order_index: cell.table_order_index,
            row: None,
            column: None,
            top: spec.top,
            bottom: spec.bottom,
            left: spec.left,
            right: spec.right,
        })
    }

    /// Update one or more margins of the cell containing the caret.
    #[wasm_bindgen(js_name = setTableCellMargins)]
    pub fn set_table_cell_margins(&self, spec: JsValue) -> Result<JsValue, JsValue> {
        let Some(range) = self.caret_table_range() else {
            return Ok(JsValue::NULL);
        };
        let spec: TableCellMarginsInput = serde_wasm_bindgen::from_value(spec)
            .map_err(|err| JsValue::from_str(&format!("Invalid cell margins: {err}")))?;
        if spec.is_empty() {
            return Err(JsValue::from_str("At least one cell margin is required"));
        }
        self.apply_paragraph_style_headless_many(move |_, _| {
            range
                .cells
                .iter()
                .map(|cell| HeadlessCommand::SetTableCellMargins {
                    table_order_index: range.table_order_index,
                    row: Some(cell.row),
                    column: Some(cell.column),
                    top: spec.top,
                    bottom: spec.bottom,
                    left: spec.left,
                    right: spec.right,
                })
                .collect()
        })
    }

    /// Restore inherited margins for every selected cell in one history entry.
    #[wasm_bindgen(js_name = resetTableCellMargins)]
    pub fn reset_table_cell_margins(&self) -> Result<JsValue, JsValue> {
        let Some(range) = self.caret_table_range() else {
            return Ok(JsValue::NULL);
        };
        self.apply_paragraph_style_headless_many(move |_, _| {
            range
                .cells
                .iter()
                .map(|cell| HeadlessCommand::ResetTableCellMargins {
                    table_order_index: range.table_order_index,
                    row: cell.row,
                    column: cell.column,
                })
                .collect()
        })
    }

    /// Word's authored row-height constraint for every selected row.
    #[wasm_bindgen(js_name = setTableRowHeightProperties)]
    pub fn set_table_row_height_properties(
        &self,
        mode: String,
        height: Option<f32>,
    ) -> Result<JsValue, JsValue> {
        let Some(range) = self.caret_table_range() else {
            return Ok(JsValue::NULL);
        };
        self.apply_paragraph_style_headless_many(move |_, _| {
            range
                .rows
                .iter()
                .map(|row| HeadlessCommand::SetTableRowHeightProperties {
                    table_order_index: range.table_order_index,
                    row: *row,
                    mode: mode.clone(),
                    height,
                })
                .collect()
        })
    }

    /// Set the visual border-to-border height of every selected row (Word's
    /// 行の高さ applies to the whole selection).
    #[wasm_bindgen(js_name = setTableRowHeight)]
    pub fn set_table_row_height(&self, height: f32) -> Result<JsValue, JsValue> {
        let Some(range) = self.caret_table_range() else {
            return Ok(JsValue::NULL);
        };
        self.apply_paragraph_style_headless_many(move |_, _| {
            range
                .rows
                .iter()
                .map(|row| HeadlessCommand::ResizeTableRowBottomEdge {
                    table_order_index: range.table_order_index,
                    row_index: *row,
                    new_height: height,
                })
                .collect()
        })
    }

    /// Update page-break behavior for the row containing the caret.
    #[wasm_bindgen(js_name = setTableRowProperties)]
    pub fn set_table_row_properties(
        &self,
        repeat_header: Option<bool>,
        cant_split: Option<bool>,
    ) -> Result<JsValue, JsValue> {
        let Some(range) = self.caret_table_range() else {
            return Ok(JsValue::NULL);
        };
        self.apply_paragraph_style_headless_many(move |_, _| {
            range
                .rows
                .iter()
                .map(|row| HeadlessCommand::SetTableRowProperties {
                    table_order_index: range.table_order_index,
                    row: *row,
                    repeat_header,
                    cant_split,
                })
                .collect()
        })
    }

    /// Insert rows above the selection — as many as are selected, which is
    /// what Word does. No-op (null) when the caret is not inside a table cell.
    #[wasm_bindgen(js_name = insertTableRowAbove)]
    pub fn insert_table_row_above(&self) -> Result<JsValue, JsValue> {
        let Some(range) = self.caret_table_range() else {
            return Ok(JsValue::NULL);
        };
        let Some(first) = range.rows.first().copied() else {
            return Ok(JsValue::NULL);
        };
        let count = range.rows.len();
        self.apply_paragraph_style_headless_many(move |_, _| {
            // Every insert at `first` pushes the template row one slot down,
            // so the next command has to follow it.
            (0..count)
                .map(|step| HeadlessCommand::InsertTableRow {
                    table_order_index: range.table_order_index,
                    row_index: first,
                    template_row: Some(first + step),
                })
                .collect()
        })
    }

    /// Insert rows below the selection — as many as are selected (Word).
    #[wasm_bindgen(js_name = insertTableRowBelow)]
    pub fn insert_table_row_below(&self) -> Result<JsValue, JsValue> {
        let Some(range) = self.caret_table_range() else {
            return Ok(JsValue::NULL);
        };
        let Some(last) = range.rows.last().copied() else {
            return Ok(JsValue::NULL);
        };
        let count = range.rows.len();
        self.apply_paragraph_style_headless_many(move |_, _| {
            (0..count)
                .map(|step| HeadlessCommand::InsertTableRow {
                    table_order_index: range.table_order_index,
                    row_index: last + 1 + step,
                    template_row: Some(last),
                })
                .collect()
        })
    }

    /// Delete every selected row (Word's 行の削除 removes the whole selection,
    /// not just the caret's row). Deleting all of them removes the table. The
    /// caret lands in the row that took the first deleted one's place, the way
    /// Word leaves it. No-op when the caret is outside a table.
    #[wasm_bindgen(js_name = deleteTableRow)]
    pub fn delete_table_row(&self) -> Result<JsValue, JsValue> {
        let Some(range) = self.caret_table_range() else {
            return Ok(JsValue::NULL);
        };
        let Some(target_row) = track_index_after_delete(&range.rows, range.row_count) else {
            return self.delete_table_internal(range.table_order_index);
        };
        let rows = range.rows.clone();
        let table_order_index = range.table_order_index;
        let payload = self.apply_paragraph_style_headless_many(move |_, _| {
            // Bottom-up: deleting a row renumbers the ones after it.
            rows.iter()
                .rev()
                .map(|row| HeadlessCommand::DeleteTableRow {
                    table_order_index,
                    row_index: *row,
                })
                .collect()
        })?;
        self.focus_table_cell(table_order_index, target_row, range.anchor_column, payload)
    }

    /// Width to assign to a column inserted next to `column`. Word copies the
    /// neighbouring column's grid width (左/右に列を挿入); the autofit layout
    /// then scales an over-wide grid back into the page. Falls back to `None`
    /// (even leftover distribution) when the grid defines no width.
    fn column_insert_width_for(&self, table_order_index: usize, column: usize) -> Option<f32> {
        let state = self.state.borrow();
        state
            .word_doc
            .as_ref()
            .and_then(|doc| doc.column_insert_width(table_order_index, column))
    }

    /// Insert columns to the left of the selection — as many as are selected,
    /// matching Word. Each new column copies the selection's leftmost width.
    #[wasm_bindgen(js_name = insertTableColumnLeft)]
    pub fn insert_table_column_left(&self) -> Result<JsValue, JsValue> {
        let Some(range) = self.caret_table_range() else {
            return Ok(JsValue::NULL);
        };
        let Some(first) = range.columns.first().copied() else {
            return Ok(JsValue::NULL);
        };
        let width = self.column_insert_width_for(range.table_order_index, first);
        let count = range.columns.len();
        self.apply_paragraph_style_headless_many(move |_, _| {
            // As with rows, each insert at `first` shifts the template column.
            (0..count)
                .map(|step| HeadlessCommand::InsertTableColumn {
                    table_order_index: range.table_order_index,
                    column_index: first,
                    width,
                    template_column: Some(first + step),
                })
                .collect()
        })
    }

    /// Insert a column to the right of the cell containing the caret. For a
    /// merged (gridSpan) cell the column is inserted after its rightmost grid
    /// slot, and the new column copies the caret column's width, matching
    /// Word.
    #[wasm_bindgen(js_name = insertTableColumnRight)]
    pub fn insert_table_column_right(&self) -> Result<JsValue, JsValue> {
        let Some(range) = self.caret_table_range() else {
            return Ok(JsValue::NULL);
        };
        let Some(last) = range.columns.last().copied() else {
            return Ok(JsValue::NULL);
        };
        let width = self.column_insert_width_for(range.table_order_index, last);
        let count = range.columns.len();
        self.apply_paragraph_style_headless_many(move |_, _| {
            (0..count)
                .map(|step| HeadlessCommand::InsertTableColumn {
                    table_order_index: range.table_order_index,
                    column_index: last + 1 + step,
                    width,
                    template_column: Some(last),
                })
                .collect()
        })
    }

    /// Delete the grid column containing the caret (the leftmost grid slot of
    /// a merged cell). Deleting the only remaining column removes the whole
    /// table, matching Word's 列の削除 behaviour. No-op when the caret is
    /// outside a table.
    #[wasm_bindgen(js_name = deleteTableColumn)]
    pub fn delete_table_column(&self) -> Result<JsValue, JsValue> {
        let Some(range) = self.caret_table_range() else {
            return Ok(JsValue::NULL);
        };
        let Some(target_column) = track_index_after_delete(&range.columns, range.column_count)
        else {
            return self.delete_table_internal(range.table_order_index);
        };
        let columns = range.columns.clone();
        let table_order_index = range.table_order_index;
        let payload = self.apply_paragraph_style_headless_many(move |_, _| {
            // Right-to-left: deleting a column renumbers the ones after it.
            columns
                .iter()
                .rev()
                .map(|column| HeadlessCommand::DeleteTableColumn {
                    table_order_index,
                    column_index: *column,
                })
                .collect()
        })?;
        self.focus_table_cell(table_order_index, range.anchor_row, target_column, payload)
    }

    /// Word's セルの挿入: insert an empty cell in the caret cell's place and
    /// shift the existing cells right or down. The caret goes to the new cell.
    #[wasm_bindgen(js_name = insertTableCell)]
    pub fn insert_table_cell(&self, direction: String) -> Result<JsValue, JsValue> {
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        let direction = match direction.as_str() {
            "right" => HeadlessTableCellInsertDirection::ShiftRight,
            "down" => HeadlessTableCellInsertDirection::ShiftDown,
            _ => {
                return Err(JsValue::from_str(
                    "Cell insert direction must be right or down",
                ))
            }
        };
        let payload =
            self.apply_paragraph_style_headless(move |_, _| HeadlessCommand::InsertTableCell {
                table_order_index: cell.table_order_index,
                row: cell.row,
                column: cell.column,
                direction,
            })?;
        self.focus_table_cell(cell.table_order_index, cell.row, cell.column, payload)
    }

    /// Remove the caret cell and compact the remaining cells left or up.
    #[wasm_bindgen(js_name = deleteTableCell)]
    pub fn delete_table_cell(&self, direction: String) -> Result<JsValue, JsValue> {
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        let direction = match direction.as_str() {
            "left" => HeadlessTableCellDeleteDirection::ShiftLeft,
            "up" => HeadlessTableCellDeleteDirection::ShiftUp,
            _ => {
                return Err(JsValue::from_str(
                    "Cell delete direction must be left or up",
                ))
            }
        };
        let payload =
            self.apply_paragraph_style_headless(move |_, _| HeadlessCommand::DeleteTableCell {
                table_order_index: cell.table_order_index,
                row: cell.row,
                column: cell.column,
                direction,
            })?;
        let target_column = if matches!(direction, HeadlessTableCellDeleteDirection::ShiftLeft)
            || cell.row + 1 == cell.row_count
        {
            cell.column.min(cell.column_count.saturating_sub(2))
        } else {
            cell.column
        };
        self.focus_table_cell(cell.table_order_index, cell.row, target_column, payload)
    }

    /// Delete the whole table containing the caret (Word's 表の削除), whether
    /// it is a top-level table or one nested inside a cell. No-op when the
    /// caret is outside a table.
    #[wasm_bindgen(js_name = deleteTable)]
    pub fn delete_table(&self) -> Result<JsValue, JsValue> {
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        self.delete_table_internal(cell.table_order_index)
    }

    fn delete_table_internal(&self, table_order_index: usize) -> Result<JsValue, JsValue> {
        let payload = self.apply_paragraph_style_headless(|_, _| HeadlessCommand::DeleteTable {
            table_order_index,
        })?;
        self.clamp_state_selection_to_layout();
        Ok(payload)
    }

    /// Word's Tab key inside a table. Moves the caret to the next cell
    /// (`backward`, Shift+Tab: the previous cell) and selects that cell's
    /// contents, the way Word does. Tab in the last cell appends a row and
    /// lands in its first cell; Shift+Tab in the first cell stays put. Returns
    /// null when the caret is not inside a table cell (or a header/footer is
    /// being edited) so the caller inserts a tab character instead.
    #[wasm_bindgen(js_name = tableTab)]
    pub fn table_tab(&self, backward: bool) -> Result<JsValue, JsValue> {
        if self.state.borrow().preview_mode || !self.body_commands_allowed() {
            return Ok(JsValue::NULL);
        }
        let Some(cell) = self.caret_table_cell() else {
            return Ok(JsValue::NULL);
        };
        let target_paragraphs = {
            let renderer = self.renderer.borrow();
            let Some(range) = renderer.selection_range() else {
                return Ok(JsValue::NULL);
            };
            let Some(cell_id) = renderer.cell_id_for_paragraph(range.focus.paragraph) else {
                return Ok(JsValue::NULL);
            };
            let Some(current) = renderer.cell_layout(cell_id) else {
                return Ok(JsValue::NULL);
            };
            // Cells of one table in Word's traversal order: row-major, which
            // is also the order the renderer registers them in.
            let mut same_table: Vec<&jade_docx_layout::selection::CellLayout> = renderer
                .cell_layouts_debug_ref()
                .iter()
                .filter(|layout| layout.table_id == current.table_id)
                .collect();
            same_table.sort_by_key(|layout| (layout.row, layout.column));
            let Some(position) = same_table
                .iter()
                .position(|layout| layout.order == current.order)
            else {
                return Ok(JsValue::NULL);
            };
            let neighbour = if backward {
                position
                    .checked_sub(1)
                    .and_then(|index| same_table.get(index))
            } else {
                same_table.get(position + 1)
            };
            neighbour.map(|layout| layout.paragraphs.clone())
        };

        match target_paragraphs {
            Some(paragraphs) => self.select_table_cell_contents(&paragraphs, None, false),
            None if backward => {
                // Shift+Tab in the first cell: Word keeps the caret where it is.
                Ok(JsValue::NULL)
            }
            None => {
                // Tab in the last cell appends a row (Word) and moves into
                // its first cell.
                let appended =
                    self.apply_paragraph_style_headless(|_, _| HeadlessCommand::InsertTableRow {
                        table_order_index: cell.table_order_index,
                        row_index: cell.row_count,
                        template_row: Some(cell.row),
                    })?;
                let first_new_cell = {
                    let renderer = self.renderer.borrow();
                    let mut candidates: Vec<&jade_docx_layout::selection::CellLayout> = renderer
                        .cell_layouts_debug_ref()
                        .iter()
                        .filter(|layout| {
                            renderer.table_order_for(layout.table_id)
                                == Some(cell.table_order_index)
                                && layout.row == cell.row_count
                        })
                        .collect();
                    candidates.sort_by_key(|layout| layout.column);
                    candidates.first().map(|layout| layout.paragraphs.clone())
                };
                match first_new_cell {
                    Some(paragraphs) => {
                        self.select_table_cell_contents(&paragraphs, Some(appended), false)
                    }
                    None => Ok(appended),
                }
            }
        }
    }

    /// Selects everything inside a table cell given its paragraph indices in
    /// document order (Word selects the destination cell's contents on Tab).
    /// `base` is an update payload from a structural edit that ran first; its
    /// bitmaps stay valid because the selection is drawn as an overlay, so
    /// only the caret and selection rectangles are replaced on it.
    fn select_table_cell_contents(
        &self,
        paragraphs: &[usize],
        base: Option<JsValue>,
        collapse: bool,
    ) -> Result<JsValue, JsValue> {
        let (Some(first), Some(last)) = (paragraphs.first(), paragraphs.last()) else {
            return Ok(base.unwrap_or(JsValue::NULL));
        };
        let range = {
            let renderer = self.renderer.borrow();
            let anchor = renderer.clamp_text_position(TextPosition::new(*first, 0));
            if collapse {
                SelectionRange::collapsed(anchor)
            } else {
                let focus = renderer.clamp_text_position(TextPosition::new(*last, usize::MAX));
                SelectionRange::new(anchor, focus)
            }
        };
        let (caret_metrics, selection_after, snapshots, dirty_regions) = {
            let mut renderer = self.renderer.borrow_mut();
            renderer.clear_cell_selection();
            let use_overlay = renderer.use_selection_overlay();
            let changed = renderer.set_selection_range(Some(range));
            if changed && !use_overlay {
                renderer.skip_change_detection_once();
                renderer.rerender_with_cached_state()?;
            }
            let caret_metrics = renderer.caret_metrics();
            let selection_after = renderer.selection_range();
            let (snapshots, dirty_regions) = if changed && !use_overlay {
                self.capture_paged_images(&mut renderer)?
            } else {
                (None, Array::new().into())
            };
            (caret_metrics, selection_after, snapshots, dirty_regions)
        };
        self.store_paged_images(snapshots);
        {
            let mut state_mut = self.state.borrow_mut();
            state_mut.selection = selection_after;
            state_mut.object_selection = None;
            state_mut.caret_goal_x = None;
        }

        let caret_value = match caret_metrics {
            Some(metrics) => to_value(&metrics).unwrap_or(JsValue::NULL),
            None => JsValue::NULL,
        };
        let result = match base {
            Some(base) if base.is_object() => Object::from(base),
            _ => {
                let result = Object::new();
                Reflect::set(&result, &JsValue::from_str("size"), &self.canvas_size()?)?;
                Reflect::set(&result, &JsValue::from_str("pages"), &self.paged_images())?;
                Reflect::set(&result, &JsValue::from_str("dirtyRegions"), &dirty_regions)?;
                result
            }
        };
        Reflect::set(&result, &JsValue::from_str("caret"), &caret_value)?;
        Reflect::set(&result, &JsValue::from_str("selectionRects"), &Array::new())?;
        {
            let renderer_ref = self.renderer.borrow();
            Self::write_selection_rects(&result, &renderer_ref, selection_after)?;
        }
        self.write_merge_state(&result)?;
        Self::write_history_flags(&result, self.history_flags())?;
        Ok(result.into())
    }

    /// Insert a generated table of contents after the paragraph containing the
    /// caret (Word's References ▸ Table of Contents). Collects headings in the
    /// inclusive outline-level range `[outline_min, outline_max]` (defaults
    /// 1..=3 when 0 is passed). Returns an error when there are no headings.
    #[wasm_bindgen(js_name = insertTableOfContents)]
    pub fn insert_table_of_contents(
        &self,
        outline_min: u8,
        outline_max: u8,
    ) -> Result<JsValue, JsValue> {
        let min = if outline_min == 0 { 1 } else { outline_min };
        let max = if outline_max == 0 {
            3
        } else {
            outline_max.max(min)
        };
        self.apply_paragraph_style_headless(|start, _end| HeadlessCommand::InsertTableOfContents {
            after_paragraph: start,
            outline_min: min,
            outline_max: max,
        })
    }

    /// Regenerate the first table of contents in the document from the current
    /// headings (Word's References ▸ Update Table / right-click ▸ update field).
    /// Returns an error when the document has no TOC field.
    #[wasm_bindgen(js_name = updateTableOfContents)]
    pub fn update_table_of_contents(&self) -> Result<JsValue, JsValue> {
        self.apply_paragraph_style_headless(|_start, _end| HeadlessCommand::UpdateTableOfContents)
    }

    #[wasm_bindgen(js_name = setParagraphStyle)]
    pub fn set_paragraph_style(&self, style_id: Option<String>) -> Result<JsValue, JsValue> {
        let normalized = style_id
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if self.state.borrow().preview_mode {
            return Ok(JsValue::NULL);
        }
        if self.state.borrow().editing_mode != EditingMode::Body {
            return self.apply_hf_paragraph_commands(vec![HeadlessCommand::SetParagraphStyle {
                start_paragraph: 0,
                end_paragraph: 0,
                style_id: normalized,
            }]);
        }
        self.apply_paragraph_style_headless(|start, end| HeadlessCommand::SetParagraphStyle {
            start_paragraph: start,
            end_paragraph: end,
            style_id: normalized,
        })
    }

    #[wasm_bindgen(js_name = setDropCap)]
    pub fn set_drop_cap(&self, kind: Option<String>, lines: u8) -> Result<JsValue, JsValue> {
        let kind = match kind
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            None | Some("none") => None,
            Some("drop") => Some(HeadlessDropCapKind::Drop),
            Some("margin") => Some(HeadlessDropCapKind::Margin),
            Some(_) => {
                return Err(JsValue::from_str(
                    "drop cap kind must be 'drop', 'margin', or null",
                ));
            }
        };
        let lines = if kind.is_some() { lines } else { 3 };
        self.apply_paragraph_style_headless(|start, end| HeadlessCommand::SetDropCap {
            start_paragraph: start,
            end_paragraph: end,
            kind,
            lines,
        })
    }

    /// The section a ribbon command aimed at "this section" targets: the one
    /// owning the page whose header/footer is being edited, or the one holding
    /// the caret in the body. `None` when it cannot be resolved, which every
    /// caller reads as Word's fallback of the final section.
    fn ribbon_target_section(&self) -> Option<usize> {
        let state = self.state.borrow();
        if state.editing_mode != EditingMode::Body {
            let cursor = state.hf_edit_cursor?;
            return Some(
                self.renderer
                    .borrow()
                    .section_index_for_zero_based_page(cursor.page_index),
            );
        }
        let word_doc = state.word_doc.as_ref()?;
        let paragraph = state
            .selection
            .or_else(|| self.renderer.borrow().selection_range())?
            .focus
            .paragraph;
        word_doc.section_index_for_paragraph(paragraph)
    }

    /// Returns the active Word header/footer variant switches for the section
    /// the caret (or the header being edited) sits in — `w:titlePg` is a
    /// per-section property, so the ribbon checkbox has to follow the caret.
    /// Kept separate from mutation so embedders can initialize checked ribbon
    /// controls after opening an existing document.
    #[wasm_bindgen(js_name = getHeaderFooterVariants)]
    pub fn get_header_footer_variants(&self) -> JsValue {
        let section = self.ribbon_target_section();
        let state = self.state.borrow();
        let Some(document) = state.word_doc.as_ref() else {
            return JsValue::NULL;
        };
        let different_first_page = section
            .and_then(|index| document.section_title_page(index))
            .unwrap_or(document.title_page);
        to_value(&HeaderFooterVariantSettingsJs {
            different_first_page,
            different_odd_even_pages: document.even_and_odd_headers,
        })
        .unwrap_or(JsValue::NULL)
    }

    /// Page numbering follows the same section target as its setter, including
    /// the page whose header or footer is being edited.
    #[wasm_bindgen(js_name = getPageNumberFormat)]
    pub fn get_page_number_format(&self) -> JsValue {
        let section = self.ribbon_target_section();
        let state = self.state.borrow();
        state.word_doc.as_ref().map_or(JsValue::NULL, |document| {
            let settings = document.page_number_format(section);
            PageNumberFormatSettingsJs {
                format: settings.format.unwrap_or_else(|| "decimal".to_string()),
                start: settings.start,
            }
            .serialize(&serde_wasm_bindgen::Serializer::new().serialize_missing_as_null(true))
            .unwrap_or(JsValue::NULL)
        })
    }

    /// Toggles Word's first-page and odd/even header/footer stories. The
    /// first-page switch is a section property, so it lands on the caret's
    /// section; odd/even is one document-wide setting. Newly enabled stories
    /// are materialized as editable blank package parts.
    #[wasm_bindgen(js_name = setHeaderFooterVariants)]
    pub fn set_header_footer_variants(
        &self,
        different_first_page: bool,
        different_odd_even_pages: bool,
    ) -> Result<JsValue, JsValue> {
        let section = self.ribbon_target_section();
        self.apply_paragraph_style_headless(move |_start, _end| {
            HeadlessCommand::SetHeaderFooterVariants {
                different_first_page,
                different_odd_even_pages,
                section,
            }
        })
    }

    /// The caret's paragraph when the selection is collapsed at its start
    /// and the paragraph is a list item: `(paragraph index, is empty)`.
    /// Word gives Tab, Shift+Tab, Backspace and Enter list meanings there.
    fn caret_at_list_item_start(&self) -> Option<(usize, bool)> {
        let selection = self.body_list_selection()?;
        let state = self.state.borrow();
        list_item_at_caret(state.word_doc.as_ref()?, selection)
    }

    /// The selection the list keys act on, or None outside the body text.
    fn body_list_selection(&self) -> Option<SelectionRange> {
        if self.state.borrow().editing_mode != EditingMode::Body
            || self.state.borrow().object_selection.is_some()
        {
            return None;
        }
        // The renderer holds the caret the user sees (pointer and
        // `setSelectionRange` update it directly); the state copy lags behind
        // until the next edit.
        self.renderer
            .borrow()
            .selection_range()
            .or_else(|| self.state.borrow().selection)
    }

    /// Word's Tab / Shift+Tab on list items: demote or promote the item at
    /// the caret, or every selected item, one list level. Shift+Tab elsewhere
    /// in body text does nothing in Word, so it returns a no-op payload. Null
    /// when the caller should type a tab character instead.
    #[wasm_bindgen(js_name = listTab)]
    pub fn list_tab(&self, backward: bool) -> Result<JsValue, JsValue> {
        if self.state.borrow().preview_mode {
            return Ok(JsValue::NULL);
        }
        let Some(selection) = self.body_list_selection() else {
            return Ok(JsValue::NULL);
        };
        let target = {
            let state = self.state.borrow();
            state
                .word_doc
                .as_ref()
                .and_then(|word_doc| list_tab_paragraphs(word_doc, selection))
        };
        if target.is_none() {
            if !backward {
                return Ok(JsValue::NULL);
            }
            let caret = self.renderer.borrow().caret_metrics();
            let result = Object::new();
            Reflect::set(
                &result,
                &JsValue::from_str("caret"),
                &caret
                    .map(|metrics| to_value(&metrics).unwrap_or(JsValue::NULL))
                    .unwrap_or(JsValue::NULL),
            )?;
            return Ok(result.into());
        }
        // The indent commands act on the selected paragraphs; make sure they
        // see the selection the decision was taken on.
        self.state.borrow_mut().selection = Some(selection);
        if backward {
            self.decrease_indent()
        } else {
            self.increase_indent()
        }
    }

    #[wasm_bindgen(js_name = increaseIndent)]
    pub fn increase_indent(&self) -> Result<JsValue, JsValue> {
        // Each paragraph steps relative to its own logical start indent:
        // character-unit indents by 1字 (Word's Japanese ribbon), point
        // indents on the default tab grid (Word's Western ribbon). RTL
        // paragraphs step their physical right edge.
        self.apply_paragraph_style_headless(|s, e| HeadlessCommand::AdjustParagraphIndent {
            start_paragraph: s,
            end_paragraph: e,
            direction: jade_docx_headless_api::HeadlessIndentDirection::Increase,
            target: jade_docx_headless_api::HeadlessIndentTarget::Start,
        })
    }

    #[wasm_bindgen(js_name = decreaseIndent)]
    pub fn decrease_indent(&self) -> Result<JsValue, JsValue> {
        self.apply_paragraph_style_headless(|s, e| HeadlessCommand::AdjustParagraphIndent {
            start_paragraph: s,
            end_paragraph: e,
            direction: jade_docx_headless_api::HeadlessIndentDirection::Decrease,
            target: jade_docx_headless_api::HeadlessIndentTarget::Start,
        })
    }

    /// Word's Ctrl+T: step the hanging indent — the left edge of every line
    /// but the first moves one grid step in while the first line stays put.
    #[wasm_bindgen(js_name = increaseHangingIndent)]
    pub fn increase_hanging_indent(&self) -> Result<JsValue, JsValue> {
        self.apply_paragraph_style_headless(|s, e| HeadlessCommand::AdjustParagraphIndent {
            start_paragraph: s,
            end_paragraph: e,
            direction: jade_docx_headless_api::HeadlessIndentDirection::Increase,
            target: jade_docx_headless_api::HeadlessIndentTarget::Hanging,
        })
    }

    /// Word's Ctrl+Shift+T.
    #[wasm_bindgen(js_name = decreaseHangingIndent)]
    pub fn decrease_hanging_indent(&self) -> Result<JsValue, JsValue> {
        self.apply_paragraph_style_headless(|s, e| HeadlessCommand::AdjustParagraphIndent {
            start_paragraph: s,
            end_paragraph: e,
            direction: jade_docx_headless_api::HeadlessIndentDirection::Decrease,
            target: jade_docx_headless_api::HeadlessIndentTarget::Hanging,
        })
    }

    /// Word's Ctrl+0: add 12pt of space before the selected paragraphs, or
    /// remove it when every one of them already has it.
    #[wasm_bindgen(js_name = toggleSpaceBefore)]
    pub fn toggle_space_before(&self) -> Result<JsValue, JsValue> {
        self.apply_paragraph_style_headless(|s, e| HeadlessCommand::ToggleParagraphSpaceBefore {
            start_paragraph: s,
            end_paragraph: e,
        })
    }

    #[wasm_bindgen(js_name = toggleBulletList)]
    pub fn toggle_bullet_list(&self) -> Result<JsValue, JsValue> {
        let is_bullet = {
            let state = self.state.borrow();
            let selection = state.selection.unwrap_or_else(|| {
                let renderer = self.renderer.borrow();
                renderer
                    .selection_range()
                    .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
            });
            let start = selection.anchor.paragraph.min(selection.focus.paragraph);
            let word_doc = state.word_doc.as_ref();
            let current_num = word_doc
                .and_then(|doc| doc.paragraph(start))
                .and_then(|p| p.properties.as_ref())
                .and_then(|props| props.numbering.as_ref());
            current_num
                .and_then(|n| n.id.as_deref())
                .and_then(|id| {
                    word_doc
                        .and_then(|doc| doc.numbering_part.as_ref())
                        .and_then(|np| np.get_level_info(id, 0))
                })
                .and_then(|level| level.num_fmt.as_deref())
                == Some("bullet")
        };

        if is_bullet {
            // Toggle off: remove numbering
            self.apply_paragraph_style_headless(|s, e| HeadlessCommand::SetNumbering {
                start_paragraph: s,
                end_paragraph: e,
                num_id: None,
                level: None,
            })
        } else {
            self.apply_paragraph_style_headless(|s, e| HeadlessCommand::ApplyMultilevelList {
                start_paragraph: s,
                end_paragraph: e,
                preset: HeadlessMultilevelListPreset::BulletHierarchy,
                level: 0,
            })
        }
    }

    #[wasm_bindgen(js_name = toggleNumberedList)]
    pub fn toggle_numbered_list(&self) -> Result<JsValue, JsValue> {
        let is_decimal = {
            let state = self.state.borrow();
            let selection = state.selection.unwrap_or_else(|| {
                let renderer = self.renderer.borrow();
                renderer
                    .selection_range()
                    .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
            });
            let start = selection.anchor.paragraph.min(selection.focus.paragraph);
            let word_doc = state.word_doc.as_ref();
            let current_num = word_doc
                .and_then(|doc| doc.paragraph(start))
                .and_then(|p| p.properties.as_ref())
                .and_then(|props| props.numbering.as_ref());
            current_num
                .and_then(|n| n.id.as_deref())
                .and_then(|id| {
                    word_doc
                        .and_then(|doc| doc.numbering_part.as_ref())
                        .and_then(|np| np.get_level_info(id, 0))
                })
                .and_then(|level| level.num_fmt.as_deref())
                == Some("decimal")
        };

        if is_decimal {
            self.apply_paragraph_style_headless(|s, e| HeadlessCommand::SetNumbering {
                start_paragraph: s,
                end_paragraph: e,
                num_id: None,
                level: None,
            })
        } else {
            self.apply_paragraph_style_headless(|s, e| HeadlessCommand::ApplyMultilevelList {
                start_paragraph: s,
                end_paragraph: e,
                preset: HeadlessMultilevelListPreset::DecimalHierarchy,
                level: 0,
            })
        }
    }

    #[wasm_bindgen(js_name = applyMultilevelList)]
    pub fn apply_multilevel_list(
        &self,
        preset: String,
        level: Option<u32>,
    ) -> Result<JsValue, JsValue> {
        let preset = match preset.as_str() {
            "decimalHierarchy" => HeadlessMultilevelListPreset::DecimalHierarchy,
            "chineseOfficial" => HeadlessMultilevelListPreset::ChineseOfficial,
            "bulletHierarchy" => HeadlessMultilevelListPreset::BulletHierarchy,
            _ => return Err(JsValue::from_str("Unknown multilevel list preset")),
        };
        let effective_level = level.unwrap_or_else(|| {
            let state = self.state.borrow();
            let selection = state.selection.unwrap_or_else(|| {
                let renderer = self.renderer.borrow();
                renderer
                    .selection_range()
                    .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
            });
            let start = selection.anchor.paragraph.min(selection.focus.paragraph);
            state
                .word_doc
                .as_ref()
                .and_then(|document| document.paragraph(start))
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|properties| properties.numbering.as_ref())
                .and_then(|numbering| numbering.level)
                .unwrap_or(0)
        });
        self.apply_paragraph_style_headless(|start_paragraph, end_paragraph| {
            HeadlessCommand::ApplyMultilevelList {
                start_paragraph,
                end_paragraph,
                preset,
                level: effective_level,
            }
        })
    }

    #[wasm_bindgen(js_name = applyCustomMultilevelList)]
    pub fn apply_custom_multilevel_list(
        &self,
        levels: JsValue,
        level: Option<u32>,
    ) -> Result<JsValue, JsValue> {
        let levels: Vec<HeadlessCustomNumberingLevel> = serde_wasm_bindgen::from_value(levels)
            .map_err(|error| {
                JsValue::from_str(&format!("Invalid custom multilevel list: {error}"))
            })?;
        let effective_level = level.unwrap_or_else(|| {
            let state = self.state.borrow();
            let selection = state.selection.unwrap_or_else(|| {
                let renderer = self.renderer.borrow();
                renderer
                    .selection_range()
                    .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
            });
            let start = selection.anchor.paragraph.min(selection.focus.paragraph);
            state
                .word_doc
                .as_ref()
                .and_then(|document| document.paragraph(start))
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|properties| properties.numbering.as_ref())
                .and_then(|numbering| numbering.level)
                .unwrap_or(0)
        });
        self.apply_paragraph_style_headless(|start_paragraph, end_paragraph| {
            HeadlessCommand::ApplyCustomMultilevelList {
                start_paragraph,
                end_paragraph,
                levels: levels.clone(),
                level: effective_level,
            }
        })
    }

    #[wasm_bindgen(js_name = insertHyperlink)]
    pub fn insert_hyperlink(
        &self,
        display_text: Option<String>,
        target: String,
        tooltip: Option<String>,
    ) -> Result<JsValue, JsValue> {
        self.apply_inline_style(
            |_| {},
            move |range| {
                Box::new(InsertHyperlinkCommand::new(
                    range,
                    display_text.clone(),
                    target.clone(),
                    tooltip.clone(),
                ))
            },
        )
    }

    #[wasm_bindgen(js_name = removeHyperlink)]
    pub fn remove_hyperlink(&self) -> Result<JsValue, JsValue> {
        self.apply_inline_style(|_| {}, |range| Box::new(RemoveHyperlinkCommand::new(range)))
    }

    #[wasm_bindgen(js_name = createBookmark)]
    pub fn create_bookmark(&self, name: String) -> Result<JsValue, JsValue> {
        self.apply_inline_style(
            |_| {},
            move |range| {
                Box::new(CreateBookmarkCommand::new(
                    range.focus.paragraph,
                    name.clone(),
                ))
            },
        )
    }

    #[wasm_bindgen(js_name = deleteBookmark)]
    pub fn delete_bookmark(&self, name: String) -> Result<JsValue, JsValue> {
        self.apply_inline_style(
            |_| {},
            move |_| Box::new(DeleteBookmarkCommand::new(name.clone())),
        )
    }

    #[wasm_bindgen(js_name = insertCrossReference)]
    pub fn insert_cross_reference(
        &self,
        bookmark_name: String,
        reference_kind: String,
    ) -> Result<JsValue, JsValue> {
        let kind = match reference_kind.trim().to_ascii_lowercase().as_str() {
            "text" | "ref" => CrossReferenceKind::Text,
            "page" | "pageref" => CrossReferenceKind::Page,
            _ => {
                return Err(JsValue::from_str(
                    "cross-reference kind must be text or page",
                ));
            }
        };
        self.apply_inline_style(
            |_| {},
            move |range| {
                Box::new(InsertCrossReferenceCommand::new(
                    range,
                    bookmark_name.clone(),
                    kind,
                ))
            },
        )
    }

    #[wasm_bindgen(js_name = insertTargetCrossReference)]
    pub fn insert_target_cross_reference(
        &self,
        target: JsValue,
        reference_kind: String,
    ) -> Result<JsValue, JsValue> {
        let target: CrossReferenceTarget = serde_wasm_bindgen::from_value(target)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let kind = match reference_kind.as_str() {
            "text" => CrossReferenceKind::Text,
            "page" => CrossReferenceKind::Page,
            "number" => CrossReferenceKind::Number,
            _ => {
                return Err(JsValue::from_str(
                    "cross-reference kind must be text, page or number",
                ))
            }
        };
        self.apply_inline_style(
            |_| {},
            move |range| {
                Box::new(InsertCrossReferenceCommand::for_target(
                    range,
                    target.clone(),
                    kind,
                ))
            },
        )
    }

    #[wasm_bindgen(js_name = listCrossReferenceTargets)]
    pub fn list_cross_reference_targets(&self) -> Result<JsValue, JsValue> {
        let mut state = self.state.borrow_mut();
        let Some(document) = state.word_doc.as_mut() else {
            return to_value(&Vec::<CrossReferenceTarget>::new())
                .map_err(|error| JsValue::from_str(&error.to_string()));
        };
        // Deferred initial loads have no preservation node IDs yet. The
        // explicit target picker needs stable identities across subsequent edits.
        document
            .ensure_roundtrip_xml()
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        to_value(&document.cross_reference_targets())
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    #[wasm_bindgen(js_name = insertCaption)]
    pub fn insert_caption(&self, label: String, description: String) -> Result<JsValue, JsValue> {
        self.apply_paragraph_style_headless(move |start, _end| HeadlessCommand::InsertCaption {
            after_paragraph: start,
            label,
            description,
        })
    }

    #[wasm_bindgen(js_name = markIndexEntry)]
    pub fn mark_index_entry(&self, term: String) -> Result<JsValue, JsValue> {
        self.apply_inline_style(
            |_| {},
            move |range| Box::new(MarkIndexEntryCommand::new(range, term.clone())),
        )
    }

    #[wasm_bindgen(js_name = insertIndex)]
    pub fn insert_index(&self) -> Result<JsValue, JsValue> {
        self.apply_paragraph_style_headless(|start, _end| HeadlessCommand::InsertIndex {
            after_paragraph: start,
        })
    }

    /// Returns the Word bibliography source catalog in document order.
    #[wasm_bindgen(js_name = listCitationSources)]
    pub fn list_citation_sources(&self) -> JsValue {
        let state = self.state.borrow();
        let sources = state
            .word_doc
            .as_ref()
            .map(WordDocument::citation_sources)
            .unwrap_or_default();
        serde_wasm_bindgen::to_value(&sources).unwrap_or(JsValue::NULL)
    }

    /// Adds one source to Word's `b:Sources` custom XML catalog.
    #[wasm_bindgen(js_name = addCitationSource)]
    pub fn add_citation_source(&self, value: JsValue) -> Result<JsValue, JsValue> {
        let source: CitationSourceInfo = serde_wasm_bindgen::from_value(value)
            .map_err(|error| JsValue::from_str(&format!("invalid citation source: {error}")))?;
        self.apply_inline_style(
            |_| {},
            move |_| Box::new(AddCitationSourceCommand::new(source.clone())),
        )
    }

    /// Replaces the exact current selection with an in-text citation.
    #[wasm_bindgen(js_name = insertCitation)]
    pub fn insert_citation(&self, tag: String) -> Result<JsValue, JsValue> {
        self.apply_inline_style(
            |_| {},
            move |range| Box::new(InsertCitationCommand::new(range, tag.clone())),
        )
    }

    /// Inserts the visible bibliography after the current body paragraph.
    #[wasm_bindgen(js_name = insertBibliography)]
    pub fn insert_bibliography(&self, heading: String) -> Result<JsValue, JsValue> {
        self.apply_paragraph_style_headless(move |start, _end| {
            HeadlessCommand::InsertBibliography {
                after_paragraph: start,
                heading,
            }
        })
    }

    /// Returns the raw Word editing-restriction metadata. The worker keeps the
    /// verifier inside its own thread and exposes only `hasPassword` to hosts.
    #[wasm_bindgen(js_name = getDocumentProtection)]
    pub fn get_document_protection(&self) -> JsValue {
        let state = self.state.borrow();
        let protection = state
            .word_doc
            .as_ref()
            .and_then(WordDocument::document_protection);
        serde_wasm_bindgen::to_value(&protection).unwrap_or(JsValue::NULL)
    }

    /// Sets or removes `w:documentProtection`. This remains callable while the
    /// view is in preview mode so a verified password can unlock the document.
    #[wasm_bindgen(js_name = setDocumentProtection)]
    pub fn set_document_protection(&self, value: JsValue) -> Result<JsValue, JsValue> {
        let protection = if value.is_null() || value.is_undefined() {
            None
        } else {
            Some(
                serde_wasm_bindgen::from_value::<DocumentProtectionInfo>(value).map_err(
                    |error| JsValue::from_str(&format!("invalid document protection: {error}")),
                )?,
            )
        };

        let mut state = self.state.borrow_mut();
        let mut word_doc = state
            .word_doc
            .take()
            .ok_or_else(|| JsValue::from_str("No document loaded"))?;
        let mut command_executor = std::mem::take(&mut state.command_executor);
        let mut selection = state.selection;
        let result = {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            command_executor.execute(
                Box::new(SetDocumentProtectionCommand::new(protection)),
                &mut ctx,
            )
        };
        state.word_doc = Some(word_doc);
        state.command_executor = command_executor;
        state.selection = selection;
        result.map_err(|error| JsValue::from_str(&error.to_string()))?;
        drop(state);
        Ok(self.history_state())
    }

    /// Returns the editable ink layer. Vector payloads remain opaque strings;
    /// hosts that understand GenOffice's v1 JSON can restore the original path.
    #[wasm_bindgen(js_name = listInkAnnotations)]
    pub fn list_ink_annotations(&self) -> JsValue {
        let state = self.state.borrow();
        let annotations = state
            .word_doc
            .as_ref()
            .map(WordDocument::ink_annotations)
            .unwrap_or_default();
        serde_wasm_bindgen::to_value(&annotations).unwrap_or(JsValue::NULL)
    }

    /// Replaces all editable ink in one undoable command. Validation is
    /// completed before the package is mutated, so malformed input is atomic.
    #[wasm_bindgen(js_name = setInkAnnotations)]
    pub fn set_ink_annotations(&self, value: JsValue) -> Result<JsValue, JsValue> {
        let annotations: Vec<InkAnnotationInfo> = serde_wasm_bindgen::from_value(value)
            .map_err(|error| JsValue::from_str(&format!("invalid ink annotations: {error}")))?;
        // Ink is floating and does not reflow text, but it may live in a
        // paragraph unrelated to the caret. Force a repaint rather than using
        // the inline-style dirty anchor.
        self.renderer.borrow_mut().mark_full_repaint();
        self.apply_inline_style(
            |_| {},
            move |_| Box::new(SetInkAnnotationsCommand::new(annotations.clone())),
        )
    }

    #[wasm_bindgen(js_name = listIndexEntries)]
    pub fn list_index_entries(&self) -> JsValue {
        let state = self.state.borrow();
        let entries = state
            .word_doc
            .as_ref()
            .map(|document| document.index_entries())
            .unwrap_or_default();
        serde_wasm_bindgen::to_value(&entries).unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen(js_name = insertField)]
    pub fn insert_field(&self, kind: String, cached_text: String) -> Result<JsValue, JsValue> {
        let kind = match kind.as_str() {
            "date" => HeadlessFieldKind::Date,
            "time" => HeadlessFieldKind::Time,
            "page" => HeadlessFieldKind::Page,
            "numPages" => HeadlessFieldKind::NumPages,
            "fileName" => HeadlessFieldKind::FileName,
            _ => return Err(JsValue::from_str("unsupported inline field kind")),
        };
        self.apply_inline_style(
            |_| {},
            move |range| Box::new(InsertFieldCommand::new(range, kind, cached_text.clone())),
        )
    }

    #[wasm_bindgen(js_name = insertEquation)]
    pub fn insert_equation(&self, omml: String, placement: String) -> Result<JsValue, JsValue> {
        let placement = match placement.as_str() {
            "inline" => HeadlessEquationPlacement::Inline,
            "block" => HeadlessEquationPlacement::Block,
            _ => {
                return Err(JsValue::from_str(
                    "equation placement must be inline or block",
                ));
            }
        };
        let selection = {
            let state = self.state.borrow();
            state.selection.unwrap_or_else(|| {
                self.renderer
                    .borrow()
                    .selection_range()
                    .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
            })
        };
        if !selection.is_collapsed() {
            return Err(JsValue::from_str(
                "equation insertion requires a collapsed selection",
            ));
        }
        self.apply_paragraph_style_headless(move |_, _| HeadlessCommand::InsertEquation {
            paragraph: selection.focus.paragraph,
            offset: selection.focus.offset,
            omml,
            placement,
        })
    }

    #[wasm_bindgen(js_name = insertBlankPage)]
    pub fn insert_blank_page(&self) -> Result<JsValue, JsValue> {
        let selection = {
            let state = self.state.borrow();
            state.selection.unwrap_or_else(|| {
                self.renderer
                    .borrow()
                    .selection_range()
                    .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
            })
        };
        self.apply_paragraph_style_headless(move |_, _| HeadlessCommand::InsertBlankPage {
            start_paragraph: selection.anchor.paragraph,
            start_offset: selection.anchor.offset,
            end_paragraph: selection.focus.paragraph,
            end_offset: selection.focus.offset,
        })
    }

    /// Whether edits are being recorded as tracked changes. The formatted
    /// paste path replaces the package and cannot record a revision, so the
    /// runtime pastes plain text through the tracked text path instead.
    #[wasm_bindgen(js_name = isRecordingChanges)]
    pub fn is_recording_changes(&self) -> bool {
        self.state.borrow().options.record_changes()
    }

    /// Word's plain-text paste at the selection: one undo step, blank lines
    /// become empty paragraphs (or empty list items). Body change tracking
    /// records the text and paragraph marks as insertions; header/footer
    /// paste retains the story's formatting, selection history and caret.
    #[wasm_bindgen(js_name = pasteText)]
    pub async fn paste_text(&self, text: String) -> Result<JsValue, JsValue> {
        if text.is_empty() || self.state.borrow().preview_mode {
            return Ok(JsValue::NULL);
        }
        let mode = self.state.borrow().editing_mode;
        if matches!(mode, EditingMode::Header | EditingMode::Footer) {
            return self.paste_hf_text(&text, mode).await;
        }
        if !self.body_commands_allowed() {
            return Ok(JsValue::NULL);
        }
        let selection = {
            let state = self.state.borrow();
            self.renderer
                .borrow()
                .selection_range()
                .or(state.selection)
                .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
        };
        self.state.borrow_mut().command_executor.begin_group();
        let result = (|| {
            let caret = if selection.is_collapsed() {
                selection.focus
            } else {
                // Replace the selection first; under change tracking the old
                // text stays struck through and the caret lands after it.
                let (start, end) = selection.normalized();
                self.apply_paragraph_style_headless_many(|_, _| {
                    vec![HeadlessCommand::ReplaceRange {
                        start_paragraph: start.paragraph,
                        start_offset: start.offset,
                        end_paragraph: end.paragraph,
                        end_offset: end.offset,
                        text: String::new(),
                    }]
                })?;
                self.state
                    .borrow()
                    .selection
                    .map(|range| range.focus)
                    .unwrap_or(start)
            };
            self.apply_paragraph_style_headless_many(|_, _| plain_text_paste_commands(caret, &text))
        })();
        // A failed batch has already been rolled back by the inner group;
        // closing the outer one records the paste as a single history entry.
        self.state.borrow_mut().command_executor.end_group();
        if result.is_err() {
            self.rerender_document()?;
        }
        result
    }

    /// Applies an optimistic, undoable replacement including source package parts.
    /// `caret_paragraph` / `caret_offset` place the caret afterwards (a paste
    /// leaves it right after the pasted content, as in Word); without them it
    /// goes to the document start. Header/footer edits require their resolved
    /// `part_path` and a caret in that story; body edits may omit `part_path`.
    #[wasm_bindgen(js_name = applyPackageEdit)]
    pub async fn apply_package_edit(
        &self,
        bytes: Vec<u8>,
        expected: Vec<u8>,
        caret_paragraph: Option<usize>,
        caret_offset: Option<usize>,
        part_path: Option<String>,
    ) -> Result<JsValue, JsValue> {
        if let Some(part_path) = part_path {
            if self.state.borrow().editing_mode != EditingMode::Body {
                let caret = caret_paragraph
                    .map(|paragraph| TextPosition::new(paragraph, caret_offset.unwrap_or(0)))
                    .ok_or_else(|| JsValue::from_str("A story package edit requires a caret"))?;
                return self
                    .apply_hf_package_edit(bytes, expected, part_path, caret)
                    .await;
            }
            if part_path != "word/document.xml" {
                return Err(JsValue::from_str(
                    "The package destination is not the document body",
                ));
            }
        }
        if self.state.borrow().preview_mode || !self.body_commands_allowed() {
            return Err(JsValue::from_str(
                "Package edits require an editable document body",
            ));
        }
        let options = self.state.borrow().options.clone();
        if options.record_changes() {
            return Err(JsValue::from_str(
                "変更履歴の記録をオフにしてから実行してください。",
            ));
        }
        let replacement = WordDocument::load(&bytes, &options)
            .await
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let snapshot = replacement
            .snapshot_for_replacement()
            .ok_or_else(|| JsValue::from_str("Document is unavailable"))?;
        // Package edits may insert blocks before the caret, so invalidate from
        // the document start instead of reusing paragraph-local layout caches.
        self.renderer.borrow_mut().invalidate_layout_from(0);
        self.renderer.borrow_mut().clear_paragraph_render_cache();
        let payload = self.apply_inline_style_with_scope(
            |_| {},
            move |_| {
                crate::commands::snapshot_edit::package_edit_command(
                    expected.clone(),
                    snapshot.clone(),
                    caret_paragraph
                        .map(|paragraph| TextPosition::new(paragraph, caret_offset.unwrap_or(0))),
                )
            },
            true,
        )?;
        self.wait_for_pending_images().await?;
        if !self.state.borrow().lazy_mode {
            let (snapshots, dirty_regions) = {
                let mut renderer = self.renderer.borrow_mut();
                self.capture_paged_images(&mut renderer)?
            };
            self.store_paged_images(snapshots);
            Reflect::set(&payload, &JsValue::from_str("pages"), &self.paged_images())?;
            Reflect::set(&payload, &JsValue::from_str("dirtyRegions"), &dirty_regions)?;
        }
        Ok(payload)
    }

    #[wasm_bindgen(js_name = insertCoverPage)]
    pub fn insert_cover_page(&self, value: JsValue) -> Result<JsValue, JsValue> {
        if value.is_undefined() || value.is_null() || !Array::is_array(&value) {
            return Err(JsValue::from_str("cover page paragraphs must be an array"));
        }
        if self.state.borrow().editing_mode != EditingMode::Body {
            return Err(JsValue::from_str(
                "cover pages can only be inserted while editing the document body",
            ));
        }
        let paragraphs: Vec<CoverPageParagraphInput> = serde_wasm_bindgen::from_value(value)
            .map_err(|error| JsValue::from_str(&format!("invalid cover page: {error}")))?;
        let paragraphs = paragraphs
            .into_iter()
            .map(HeadlessCoverParagraph::from)
            .collect();
        self.apply_paragraph_style_headless(move |_, _| HeadlessCommand::InsertCoverPage {
            paragraphs,
        })
    }

    /// Word-style ribbon toggle for an inline flag: the flag turns ON unless
    /// every run in the selection already carries it (a mixed selection
    /// becomes uniformly formatted, never inverted), and a collapsed caret
    /// toggles against the effective formatting at the caret including any
    /// pending state.
    #[wasm_bindgen(js_name = toggleInlineStyle)]
    pub fn toggle_inline_style(&self, kind: String) -> Result<JsValue, JsValue> {
        let currently_on = if self.body_commands_allowed() {
            self.selection_inline_flag(&kind)?
        } else {
            // Word's toggle over the header line: all on → off, else on.
            let state = self.caret_hf_run_state();
            match kind.as_str() {
                "bold" => state.as_ref().is_some_and(|state| state.all_bold),
                "italic" => state.as_ref().is_some_and(|state| state.all_italic),
                "underline" => state.as_ref().is_some_and(|state| state.all_underline),
                "strike" => state.as_ref().is_some_and(|state| state.all_strike),
                "superscript" => state.as_ref().is_some_and(|state| state.all_superscript),
                "subscript" => state.as_ref().is_some_and(|state| state.all_subscript),
                _ => false,
            }
        };
        let enabled = Some(!currently_on);
        match kind.as_str() {
            "bold" => self.apply_bold(enabled),
            "italic" => self.apply_italic(enabled),
            "underline" => self.apply_underline(enabled),
            "strike" => self.apply_strike(enabled),
            "subscript" => self.apply_subscript(enabled),
            "superscript" => self.apply_superscript(enabled),
            _ => Err(JsValue::from_str(&format!(
                "unknown inline style kind: {kind}"
            ))),
        }
    }

    /// Scans every run the (non-collapsed) selection covers through the full
    /// style cascade and summarizes the formatting with Word's ribbon
    /// semantics. Returns None when the selection is collapsed.
    fn selection_run_summary(
        word_doc: &WordDocument,
        selection: &SelectionRange,
    ) -> Option<SelectionRunSummary> {
        if selection.is_collapsed() {
            return None;
        }
        let cascade = |para_style_id: Option<&str>, direct: Option<&RunProperties>| {
            word_doc
                .styles_part
                .as_ref()
                .and_then(|styles| styles.resolve_effective_run_properties(para_style_id, direct))
                .or_else(|| direct.cloned())
        };

        let mut summary = SelectionRunSummary {
            all_bold: true,
            all_italic: true,
            all_underline: true,
            all_strike: true,
            ..SelectionRunSummary::default()
        };

        let (start, end) = selection.normalized();
        for paragraph_index in start.paragraph..=end.paragraph {
            let Some(paragraph) = word_doc.paragraph(paragraph_index) else {
                continue;
            };
            let para_style_id = paragraph
                .properties
                .as_ref()
                .and_then(|props| props.style.as_deref());
            let paragraph_len: usize = paragraph.runs.iter().map(|run| run.text.len()).sum();
            // Mirror the apply commands' char-boundary flooring so the
            // summary judges exactly the bytes a formatting command would
            // touch (see selection_inline_flag).
            let floor_to_char_boundary = |paragraph: &crate::document::Paragraph, offset: usize| {
                let mut run_start = 0usize;
                for run in &paragraph.runs {
                    let run_end = run_start + run.text.len();
                    if offset <= run_end {
                        let mut relative = offset - run_start;
                        while relative > 0 && !run.text.is_char_boundary(relative) {
                            relative -= 1;
                        }
                        return run_start + relative;
                    }
                    run_start = run_end;
                }
                offset
            };
            let range_start = if paragraph_index == start.paragraph {
                floor_to_char_boundary(paragraph, start.offset.min(paragraph_len))
            } else {
                0
            };
            let range_end = if paragraph_index == end.paragraph {
                floor_to_char_boundary(paragraph, end.offset.min(paragraph_len))
            } else {
                paragraph_len
            };

            let mut run_start = 0usize;
            for run in &paragraph.runs {
                let run_end = run_start + run.text.len();
                let overlaps = run_start < range_end && run_end > range_start;
                if overlaps && run_end > run_start {
                    summary.saw_text = true;
                    let effective =
                        cascade(para_style_id, run.properties.as_ref()).unwrap_or_default();
                    summary.all_bold &= effective.bold == Some(true);
                    summary.all_italic &= effective.italic == Some(true);
                    summary.all_underline &= effective.underline == Some(true);
                    summary.all_strike &=
                        effective.strike == Some(true) || effective.double_strike == Some(true);
                    summary
                        .baseline
                        .observe(effective.baseline.as_deref().map(str::to_owned));
                    summary
                        .font_size
                        .observe(effective.font_size.map(ordered_size::SizeKey));
                    summary
                        .font_family
                        .observe(effective.font_family.as_deref().map(str::to_owned));
                    summary
                        .color
                        .observe(effective.color.as_deref().map(str::to_owned));
                    summary
                        .highlight
                        .observe(effective.highlight.as_deref().map(str::to_owned));
                }
                run_start = run_end;
            }
        }

        summary.saw_text.then_some(summary)
    }

    fn selection_inline_flag(&self, kind: &str) -> Result<bool, JsValue> {
        fn run_has_inline_flag(props: &RunProperties, kind: &str) -> bool {
            match kind {
                "bold" => props.bold == Some(true),
                "italic" => props.italic == Some(true),
                "underline" => props.underline == Some(true),
                // The ribbon's strike button also reads as pressed for double
                // strikethrough, and disabling clears both variants.
                "strike" => props.strike == Some(true) || props.double_strike == Some(true),
                "subscript" => props.baseline.as_deref() == Some("subscript"),
                "superscript" => props.baseline.as_deref() == Some("superscript"),
                _ => false,
            }
        }

        let state = self.state.borrow();
        let selection = state.selection.unwrap_or_else(|| {
            let renderer = self.renderer.borrow();
            renderer
                .selection_range()
                .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
        });
        let Some(word_doc) = state.word_doc.as_ref() else {
            return Ok(false);
        };

        let cascade = |para_style_id: Option<&str>, direct: Option<&RunProperties>| {
            word_doc
                .styles_part
                .as_ref()
                .and_then(|styles| styles.resolve_effective_run_properties(para_style_id, direct))
                .or_else(|| direct.cloned())
        };

        if selection.is_collapsed() {
            let position = selection.focus;
            let Some(paragraph) = word_doc.paragraph(position.paragraph) else {
                return Ok(false);
            };
            let mut run_start = 0usize;
            let mut direct: Option<&RunProperties> = None;
            for run in &paragraph.runs {
                let run_end = run_start + run.text.len();
                if position.offset < run_end || (position.offset == run_end && run_end == run_start)
                {
                    direct = run.properties.as_ref();
                    break;
                }
                if position.offset == run_end {
                    // Word's caret formatting is the character BEFORE the
                    // caret: at a run boundary the preceding run governs what
                    // a toggle sees, or turning a flag off at the end of a
                    // styled run could never settle.
                    direct = run.properties.as_ref();
                    break;
                }
                run_start = run_end;
            }
            let para_style_id = paragraph
                .properties
                .as_ref()
                .and_then(|props| props.style.as_deref());
            let mut effective = cascade(para_style_id, direct).unwrap_or_default();
            if state.inline_formatting.is_armed_for(&selection) {
                if let Some(pending) = state.inline_formatting.as_run_properties() {
                    merge_run_properties_override(&mut effective, &pending);
                }
            }
            return Ok(run_has_inline_flag(&effective, kind));
        }

        let (start, end) = selection.normalized();
        let mut saw_any_text = false;
        for paragraph_index in start.paragraph..=end.paragraph {
            let Some(paragraph) = word_doc.paragraph(paragraph_index) else {
                continue;
            };
            let para_style_id = paragraph
                .properties
                .as_ref()
                .and_then(|props| props.style.as_deref());
            let paragraph_len: usize = paragraph.runs.iter().map(|run| run.text.len()).sum();
            // Selection offsets can land inside a multi-byte character; the
            // apply commands floor such offsets to the previous character
            // boundary (`clamp_to_char_boundary` in `split_runs_at`), so the
            // scan must judge exactly the bytes an apply would touch or a
            // toggle can never settle.
            let floor_to_char_boundary = |paragraph: &crate::document::Paragraph, offset: usize| {
                let mut run_start = 0usize;
                for run in &paragraph.runs {
                    let run_end = run_start + run.text.len();
                    if offset <= run_end {
                        let mut relative = offset - run_start;
                        while relative > 0 && !run.text.is_char_boundary(relative) {
                            relative -= 1;
                        }
                        return run_start + relative;
                    }
                    run_start = run_end;
                }
                offset
            };
            let range_start = if paragraph_index == start.paragraph {
                floor_to_char_boundary(paragraph, start.offset.min(paragraph_len))
            } else {
                0
            };
            let range_end = if paragraph_index == end.paragraph {
                floor_to_char_boundary(paragraph, end.offset.min(paragraph_len))
            } else {
                paragraph_len
            };

            let mut run_start = 0usize;
            for run in &paragraph.runs {
                let run_end = run_start + run.text.len();
                let overlaps = run_start < range_end && run_end > range_start;
                if overlaps && run_end > run_start {
                    saw_any_text = true;
                    let effective =
                        cascade(para_style_id, run.properties.as_ref()).unwrap_or_default();
                    if !run_has_inline_flag(&effective, kind) {
                        return Ok(false);
                    }
                }
                run_start = run_end;
            }
        }
        // An empty selection (only empty paragraphs) toggles ON, like Word.
        Ok(saw_any_text)
    }

    #[wasm_bindgen]
    pub fn apply_bold(&self, enabled: Option<bool>) -> Result<JsValue, JsValue> {
        let enabled = enabled.unwrap_or(true);
        self.apply_inline_style_headless(
            move |inline| inline.set_bold(enabled),
            |range| HeadlessCommand::ApplyBold {
                start_paragraph: range.anchor.paragraph,
                start_offset: range.anchor.offset,
                end_paragraph: range.focus.paragraph,
                end_offset: range.focus.offset,
                enabled,
            },
        )
    }

    #[wasm_bindgen]
    pub fn apply_italic(&self, enabled: Option<bool>) -> Result<JsValue, JsValue> {
        let enabled = enabled.unwrap_or(true);
        self.apply_inline_style_headless(
            move |inline| inline.set_italic(enabled),
            |range| HeadlessCommand::ApplyItalic {
                start_paragraph: range.anchor.paragraph,
                start_offset: range.anchor.offset,
                end_paragraph: range.focus.paragraph,
                end_offset: range.focus.offset,
                enabled,
            },
        )
    }

    #[wasm_bindgen]
    pub fn apply_underline(&self, enabled: Option<bool>) -> Result<JsValue, JsValue> {
        let enabled = enabled.unwrap_or(true);
        self.apply_inline_style_headless(
            move |inline| inline.set_underline(enabled),
            |range| HeadlessCommand::ApplyUnderline {
                start_paragraph: range.anchor.paragraph,
                start_offset: range.anchor.offset,
                end_paragraph: range.focus.paragraph,
                end_offset: range.focus.offset,
                kind: None,
                enabled,
            },
        )
    }

    #[wasm_bindgen]
    pub fn apply_underline_kind(&self, kind: String) -> Result<JsValue, JsValue> {
        const SUPPORTED: &[&str] = &[
            "single", "words", "double", "thick", "dotted", "dash", "wave",
        ];
        if !SUPPORTED.contains(&kind.as_str()) {
            return Err(JsValue::from_str("unsupported underline kind"));
        }
        self.apply_inline_style_headless(
            |inline| inline.set_underline_kind(kind.clone()),
            |range| HeadlessCommand::ApplyUnderline {
                start_paragraph: range.anchor.paragraph,
                start_offset: range.anchor.offset,
                end_paragraph: range.focus.paragraph,
                end_offset: range.focus.offset,
                kind: Some(kind.clone()),
                enabled: true,
            },
        )
    }

    #[wasm_bindgen]
    pub fn apply_strike(&self, enabled: Option<bool>) -> Result<JsValue, JsValue> {
        let enabled = enabled.unwrap_or(true);
        self.apply_inline_style_headless(
            move |inline| inline.set_strike(enabled),
            |range| HeadlessCommand::ApplyStrike {
                start_paragraph: range.anchor.paragraph,
                start_offset: range.anchor.offset,
                end_paragraph: range.focus.paragraph,
                end_offset: range.focus.offset,
                double: false,
                enabled,
            },
        )
    }

    #[wasm_bindgen]
    pub fn apply_strike_kind(&self, kind: String) -> Result<JsValue, JsValue> {
        if kind != "single" && kind != "double" {
            return Err(JsValue::from_str("unsupported strike kind"));
        }
        let double = kind == "double";
        self.apply_inline_style_headless(
            move |inline| {
                if double {
                    inline.set_double_strike();
                } else {
                    inline.set_strike(true);
                }
            },
            move |range| HeadlessCommand::ApplyStrike {
                start_paragraph: range.anchor.paragraph,
                start_offset: range.anchor.offset,
                end_paragraph: range.focus.paragraph,
                end_offset: range.focus.offset,
                double,
                enabled: true,
            },
        )
    }

    #[wasm_bindgen]
    pub fn apply_superscript(&self, enabled: Option<bool>) -> Result<JsValue, JsValue> {
        let enabled = enabled.unwrap_or(true);
        self.apply_inline_style_headless(
            move |inline| inline.set_baseline("superscript", enabled),
            |range| HeadlessCommand::ApplySuperscript {
                start_paragraph: range.anchor.paragraph,
                start_offset: range.anchor.offset,
                end_paragraph: range.focus.paragraph,
                end_offset: range.focus.offset,
                enabled,
            },
        )
    }

    #[wasm_bindgen]
    pub fn apply_subscript(&self, enabled: Option<bool>) -> Result<JsValue, JsValue> {
        let enabled = enabled.unwrap_or(true);
        self.apply_inline_style_headless(
            move |inline| inline.set_baseline("subscript", enabled),
            |range| HeadlessCommand::ApplySubscript {
                start_paragraph: range.anchor.paragraph,
                start_offset: range.anchor.offset,
                end_paragraph: range.focus.paragraph,
                end_offset: range.focus.offset,
                enabled,
            },
        )
    }

    #[wasm_bindgen]
    pub fn export_docx(&self) -> Result<js_sys::Uint8Array, JsValue> {
        let bytes = {
            let mut state = self.state.borrow_mut();
            let word_doc = state
                .word_doc
                .as_mut()
                .ok_or_else(|| JsValue::from_str("document is not loaded"))?;
            word_doc
                .ensure_roundtrip_xml()
                .map_err(|err| JsValue::from_str(&err.to_string()))?;
            word_doc
                .export_docx()
                .map_err(|err| JsValue::from_str(&err.to_string()))?
        };
        Ok(js_sys::Uint8Array::from(bytes.as_slice()))
    }

    /// Ensures edit/export preservation metadata is ready after a deferred
    /// initial load. Returns true only when this call performed the parse.
    #[wasm_bindgen(js_name = prepareRoundtripDocument)]
    pub fn prepare_roundtrip_document(&self) -> Result<bool, JsValue> {
        let mut state = self.state.borrow_mut();
        let word_doc = state
            .word_doc
            .as_mut()
            .ok_or_else(|| JsValue::from_str("document is not loaded"))?;
        word_doc
            .ensure_roundtrip_xml()
            .map_err(|err| JsValue::from_str(&err.to_string()))
    }

    #[wasm_bindgen]
    pub fn delete_text(&self, direction: String) -> Result<JsValue, JsValue> {
        if self.state.borrow().preview_mode {
            return Ok(JsValue::NULL);
        }

        // Route to header/footer delete if in hf mode
        {
            let editing_mode = self.state.borrow().editing_mode;
            if editing_mode != EditingMode::Body {
                let backward = direction == "backward";
                return self.delete_hf_text(backward, editing_mode);
            }
        }
        self.clear_composition_preview()?;

        // Word's Ctrl+Backspace / Ctrl+Delete remove the word before or
        // after the caret; with a selection they delete the selection like
        // a plain Backspace / Delete.
        let (direction, word_delete) = match direction.as_str() {
            "word_backward" => ("backward".to_string(), true),
            "word_forward" => ("forward".to_string(), true),
            _ => (direction, false),
        };
        if word_delete {
            let collapsed = {
                let renderer = self.renderer.borrow();
                renderer
                    .selection_range()
                    .filter(|range| range.is_collapsed())
                    .map(|range| range.focus)
            };
            if let Some(caret) = collapsed {
                let target = {
                    let renderer = self.renderer.borrow();
                    if direction == "backward" {
                        renderer.selection_manager().move_word_left(caret)
                    } else {
                        renderer.selection_manager().move_word_right(caret)
                    }
                };
                if target != caret {
                    let mut renderer = self.renderer.borrow_mut();
                    renderer.set_selection_range(Some(SelectionRange::new(caret, target)));
                    self.state.borrow_mut().selection = Some(SelectionRange::new(caret, target));
                }
            }
        }

        // Word's Backspace at the start of a list item first removes the
        // number or bullet; only the next Backspace joins the paragraphs.
        if direction == "backward" && self.caret_at_list_item_start().is_some() {
            return self.apply_paragraph_style_headless(|s, e| HeadlessCommand::SetNumbering {
                start_paragraph: s,
                end_paragraph: e,
                num_id: None,
                level: None,
            });
        }

        let direction = match direction.as_str() {
            "backward" => DeleteDirection::Backward,
            "forward" => DeleteDirection::Forward,
            other => {
                return Err(JsValue::from_str(&format!(
                    "unknown delete direction `{other}`"
                )));
            }
        };

        let selected_object = self.state.borrow().object_selection;
        let selection_before = if let Some(target) = selected_object {
            SelectionRange::collapsed(TextPosition::new(target.paragraph_index, 0))
        } else {
            let renderer = self.renderer.borrow();
            renderer
                .selection_range()
                .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
        };
        // Backspace at the start of the document changes neither the model nor
        // the layout. Avoid rebuilding the renderer for this no-op: doing so can
        // drop a trailing empty paragraph from the visible selection surface.
        if selected_object.is_none()
            && direction == DeleteDirection::Backward
            && selection_before.is_collapsed()
            && selection_before.focus == TextPosition::default()
        {
            let caret = self.renderer.borrow().caret_metrics();
            let result = Object::new();
            Reflect::set(
                &result,
                &JsValue::from_str("caret"),
                &caret
                    .map(|metrics| to_value(&metrics).unwrap_or(JsValue::NULL))
                    .unwrap_or(JsValue::NULL),
            )?;
            return Ok(result.into());
        }
        let inline_delete_target = if selected_object.is_none() {
            let state = self.state.borrow();
            state.word_doc.as_ref().and_then(|word_doc| {
                Self::inline_image_delete_target(word_doc, selection_before, direction)
            })
        } else {
            None
        };
        let edited_paragraph_index = selected_object
            .or_else(|| inline_delete_target.map(|(target, _)| target))
            .map(|target| target.paragraph_index)
            .unwrap_or(selection_before.focus.paragraph);

        // Optimization: When backspace at paragraph start (offset == 0) with collapsed selection,
        // the paragraph will merge with the previous one. Set dirty_from to the previous paragraph
        // to avoid unnecessary hash scanning of upstream paragraphs.
        let will_merge_with_previous = selected_object.is_none()
            && direction == DeleteDirection::Backward
            && selection_before.is_collapsed()
            && selection_before.focus.offset == 0
            && selection_before.focus.paragraph > 0;

        let dirty_from = if let Some(target) = selected_object {
            target.paragraph_index
        } else if let Some((target, _)) = inline_delete_target {
            target.paragraph_index
        } else if will_merge_with_previous {
            // Merge will happen at the previous paragraph
            selection_before.focus.paragraph.saturating_sub(1)
        } else {
            selection_before
                .anchor
                .paragraph
                .min(selection_before.focus.paragraph)
        };

        let old_height = {
            let renderer = self.renderer.borrow();
            // For merge case, track the height of the target paragraph (previous one)
            if will_merge_with_previous {
                renderer.cached_paragraph_height(dirty_from)
            } else {
                renderer.cached_paragraph_height(edited_paragraph_index)
            }
        };
        // A join removes the paragraph after `dirty_from`, and the lines below
        // move up by its height unless the joined paragraph grew by as much.
        let joined_old_height = old_height
            .zip(
                self.renderer
                    .borrow()
                    .cached_paragraph_height(dirty_from + 1),
            )
            .map(|(merged, removed)| merged + removed);

        // Drop the renderer's document cache before mutating.
        let _document_cache = self.drop_renderer_document_cache();

        let (options, numbering_part, header_lines, footer_lines, new_selection, merged_paragraphs) = {
            let mut state = self.state.borrow_mut();
            let mut word_doc = state
                .word_doc
                .take()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;
            let paragraph_count_before = word_doc.paragraph_count();

            state.selection = Some(selection_before);
            state.object_selection = None;
            let mut command_executor = std::mem::take(&mut state.command_executor);
            let command_result = if let Some(target) = selected_object {
                let mut ctx = CommandContext::new(&mut word_doc, &mut state.selection);
                command_executor
                    .execute(Box::new(DeleteObjectCommand::new(target)), &mut ctx)
                    .map(|_| state.selection)
                    .map_err(|err| err.to_string())
            } else if let Some((target, caret_offset)) = inline_delete_target {
                let mut ctx = CommandContext::new(&mut word_doc, &mut state.selection);
                command_executor
                    .execute(Box::new(DeleteObjectCommand::new(target)), &mut ctx)
                    .map(|_| {
                        let collapsed = SelectionRange::collapsed(TextPosition::new(
                            target.paragraph_index,
                            caret_offset,
                        ));
                        state.selection = Some(collapsed);
                        state.selection
                    })
                    .map_err(|err| err.to_string())
            } else {
                let headless_direction = match direction {
                    DeleteDirection::Backward => HeadlessDeleteDirection::Backward,
                    DeleteDirection::Forward => HeadlessDeleteDirection::Forward,
                };
                // Preserve legacy delete semantics: selected ranges are deleted as a whole.
                let request = if selection_before.is_collapsed() {
                    // One key press removes one grapheme cluster (葛󠄀, 🇯🇵,
                    // e + U+0301), which the run command counts in code points.
                    let count = crate::commands::delete_text::grapheme_delete_count(
                        &word_doc,
                        selection_before.focus,
                        direction,
                    );
                    HeadlessEditRequest {
                        commands: vec![HeadlessCommand::DeleteText {
                            paragraph: selection_before.focus.paragraph,
                            offset: selection_before.focus.offset,
                            direction: headless_direction,
                            count,
                        }],
                    }
                } else {
                    let (start, end) = selection_before.normalized();
                    HeadlessEditRequest {
                        commands: vec![HeadlessCommand::ReplaceRange {
                            start_paragraph: start.paragraph,
                            start_offset: start.offset,
                            end_paragraph: end.paragraph,
                            end_offset: end.offset,
                            text: String::new(),
                        }],
                    }
                };
                apply_headless_commands_with_executor_to_document(
                    &mut word_doc,
                    &mut command_executor,
                    state.selection,
                    &request,
                )
                .map_err(|err| err.to_string())
            };
            state.command_executor = command_executor;
            state.selection = match command_result {
                Ok(selection) => selection,
                Err(err) => {
                    state.word_doc = Some(word_doc);
                    return Err(JsValue::from_str(&err));
                }
            };

            state.paged_images.clear();

            let options = state.options.clone();
            let numbering_part = word_doc.numbering_part.clone();
            let header_lines = word_doc.header_lines.clone();
            let footer_lines = word_doc.footer_lines.clone();
            let new_selection = state.selection;
            let merged_paragraphs = word_doc.paragraph_count() != paragraph_count_before;

            state.word_doc = Some(word_doc);

            (
                options,
                numbering_part,
                header_lines,
                footer_lines,
                new_selection,
                merged_paragraphs,
            )
        };

        let anchor_page = {
            let renderer = self.renderer.borrow();
            // For merge operations, use the target paragraph (dirty_from) for anchor page calculation
            let effective_index = if will_merge_with_previous {
                dirty_from
            } else {
                edited_paragraph_index
            };
            renderer
                .cached_paragraph_start_page(effective_index)
                .or_else(|| renderer.visible_page_range().as_ref().map(|r| *r.end()))
                .or_else(|| renderer.layout_page_limit())
                .unwrap_or(1)
        };
        // Text deleted inside one body paragraph, with no paragraph joined, is
        // the same edit as typing there.
        let edits_one_body_paragraph = selected_object.is_none()
            && inline_delete_target.is_none()
            && !merged_paragraphs
            && !will_merge_with_previous
            && selection_before.anchor.paragraph == selection_before.focus.paragraph
            && dirty_from == edited_paragraph_index
            && self
                .state
                .borrow()
                .word_doc
                .as_ref()
                .is_some_and(|doc| doc.is_body_paragraph_surface(edited_paragraph_index));
        let mut renderer = self.renderer.borrow_mut();
        // Respect host-provided visible range if configured (viewport virtualization).
        // Ensure the edited page stays visible even if the range is stale.
        let lazy_mode = self.state.borrow().lazy_mode;
        Self::ensure_visible_page_range_for_edit(&mut renderer, lazy_mode, anchor_page);

        // Set selection BEFORE rendering to avoid a second rerender pass.
        // Previously, we rendered first and then called rerender_with_cached_state
        // if selection changed, causing an unnecessary full redraw.
        renderer.set_selection_range(new_selection);
        renderer.set_incremental_dirty_from(dirty_from);
        if lazy_mode {
            // A delete that leaves the paragraph count unchanged is
            // layout-equivalent to typing, so it takes the same visible-window
            // seek as insert-text instead of re-rendering every paragraph from
            // the document start (O(scroll depth) per Backspace). The seek never
            // resumes past `dirty_from`: `invalidate_layout_from` drops every
            // snapshot after it. Merges shift paragraph indices below the caret
            // and keep the conservative render-from-start walk.
            if merged_paragraphs {
                renderer.disable_seek_replay_once();
            } else {
                renderer.prefer_visible_window_seek_once();
            }
            // As for typing, the completed layout after the paragraph stays
            // until the render shows whether the paragraph kept its geometry.
            let retained_completed_pagination = edits_one_body_paragraph
                && old_height.is_some_and(|height| {
                    renderer.prepare_completed_pagination_edit(edited_paragraph_index, height)
                });
            if !retained_completed_pagination {
                renderer.invalidate_layout_from(dirty_from);
            }
        }
        renderer.invalidate_paragraph_wrap_cache_from(dirty_from);

        // Both Backspace at paragraph start and Delete at paragraph end can
        // remove the following paragraph. Their cached indices must shift.
        let merged_one_adjacent_paragraph = merged_paragraphs && selection_before.is_collapsed();
        // Apply cache optimizations based on operation type
        if will_merge_with_previous || merged_one_adjacent_paragraph {
            // Paragraph merge: use specialized optimization that handles
            // render cache gap removal and layout cache updates
            // Without lazy mode nothing extends the layout after this render,
            // and a merge changes the paragraph count, so the completed
            // pagination cannot be retained across a fast-replay stop.
            renderer.set_paragraph_merge_optimizations(
                dirty_from,
                joined_old_height.filter(|_| lazy_mode),
            );
        } else {
            renderer.mark_paragraph_dirty(dirty_from);
            if !merged_paragraphs {
                if let Some(old_height) = old_height {
                    if lazy_mode {
                        renderer.set_fast_replay_trigger(edited_paragraph_index, old_height);
                    } else {
                        renderer.arm_fast_replay_for_complete_layout_edit(
                            edited_paragraph_index,
                            old_height,
                        );
                    }
                }
            }
        }
        if lazy_mode {
            let end_page_1idx = Self::lazy_visible_end(&renderer, anchor_page);
            renderer.mark_dirty_from_paragraph_to_end_bounded(dirty_from, end_page_1idx);
        } else if merged_paragraphs {
            renderer.mark_dirty_from_paragraph_to_end(dirty_from);
        }

        {
            let state_ref = self.state.borrow();
            if let Some(word_doc) = state_ref.word_doc.as_ref() {
                if let Some(document) = word_doc.document.as_ref() {
                    let prev_limit: Option<Option<usize>> = if self.state.borrow().lazy_mode {
                        let desired = Self::lazy_visible_end(&renderer, anchor_page);
                        let prev = renderer.layout_page_limit();
                        // Clamp layout work to the viewport window during edits so large documents
                        // do not force a full layout pass while typing.
                        renderer.set_layout_page_limit(Some(desired));
                        Some(prev)
                    } else {
                        None
                    };
                    // Use _rc variant to avoid cloning the document.
                    renderer.render_with_numbering_rc(
                        Rc::clone(document),
                        &options,
                        numbering_part.as_ref(),
                        header_lines.as_deref(),
                        footer_lines.as_deref(),
                        Some(&word_doc.comments),
                    )?;
                    if let Some(prev) = prev_limit {
                        renderer.set_layout_page_limit(prev);
                    }
                }
            }
        }

        let caret_metrics = renderer.caret_metrics();
        let selection_after = renderer.selection_range();
        // Lazy-mode paragraph merges are prone to downstream stale-page artifacts because the
        // merge can pull content back from the next page while leaving no reliable dirty rects.
        // For that specific case, return visible-page snapshots instead of relying only on
        // incremental dirty regions.
        let skip_snapshots = self.state.borrow().lazy_mode && !merged_paragraphs;
        let (snapshots, dirty_regions) = if skip_snapshots {
            renderer.coalesce_dirty_regions();
            let dirty_regions = renderer.dirty_regions();
            let dirty_regions_js = Self::dirty_regions_to_js(&dirty_regions);
            renderer.clear_dirty_regions();
            (None, dirty_regions_js)
        } else {
            self.capture_paged_images(&mut renderer)?
        };
        drop(renderer);

        self.store_paged_images(snapshots);
        let (preview_mode, history_flags) = {
            let mut state_mut = self.state.borrow_mut();
            state_mut.selection = selection_after;
            state_mut.object_selection = None;
            state_mut.caret_goal_x = None;
            let flags = state_mut.history_flags();
            (state_mut.preview_mode, flags)
        };

        let size = self.canvas_size()?;
        let pages = self.paged_images();
        let caret_value = if preview_mode {
            JsValue::NULL
        } else {
            match caret_metrics {
                Some(metrics) => to_value(&metrics).unwrap_or(JsValue::NULL),
                None => JsValue::NULL,
            }
        };

        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &size)?;
        Reflect::set(&result, &JsValue::from_str("pages"), &pages)?;
        Reflect::set(&result, &JsValue::from_str("caret"), &caret_value)?;
        Reflect::set(&result, &JsValue::from_str("dirtyRegions"), &dirty_regions)?;
        Self::write_history_flags(&result, history_flags)?;
        self.write_object_selection(&result)?;
        self.write_merge_state(&result)?;
        {
            let renderer_ref = self.renderer.borrow();
            Self::write_selection_rects(&result, &renderer_ref, selection_after)?;
        }

        Ok(result.into())
    }

    #[wasm_bindgen]
    pub fn resolve_shortcut(
        &self,
        key: String,
        ctrl: bool,
        alt: bool,
        shift: bool,
        meta: bool,
    ) -> Option<String> {
        let action = {
            let state = self.state.borrow();
            let config = state.options.shortcut_config();
            config.resolve_action(&key, ctrl, alt, shift, meta)
        };
        action.map(|action| action.name().to_string())
    }

    /// The effective shortcut table as `[{ action, bindings: [{ key, ctrl,
    /// alt, shift, meta }] }]`, in match priority order.
    ///
    /// The browser decides whether to run its own accelerator (Ctrl+R
    /// reload, Ctrl+= zoom, Ctrl+D bookmark) as soon as the keydown listener
    /// returns, so a `preventDefault` issued after the `resolve_shortcut`
    /// worker round trip comes too late. The runtime keeps this copy on the
    /// main thread to cancel bound chords synchronously.
    #[wasm_bindgen]
    pub fn shortcut_table(&self) -> Result<JsValue, JsValue> {
        let state = self.state.borrow();
        let entries = js_sys::Array::new();
        for (action, bindings) in state.options.shortcut_config().binding_table() {
            let entry = js_sys::Object::new();
            Reflect::set(&entry, &"action".into(), &action.name().into())?;
            let list = js_sys::Array::new();
            for binding in bindings {
                let item = js_sys::Object::new();
                Reflect::set(&item, &"key".into(), &binding.key().into())?;
                Reflect::set(&item, &"ctrl".into(), &binding.ctrl().into())?;
                Reflect::set(&item, &"alt".into(), &binding.alt().into())?;
                Reflect::set(&item, &"shift".into(), &binding.shift().into())?;
                Reflect::set(&item, &"meta".into(), &binding.meta().into())?;
                list.push(&item);
            }
            Reflect::set(&entry, &"bindings".into(), &list)?;
            entries.push(&entry);
        }
        Ok(entries.into())
    }

    #[wasm_bindgen]
    pub fn pointer_down(&self, x: f64, y: f64) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            view.pointer_down_async(x, y).await?;
            let mode = view.state.borrow().editing_mode;
            if mode == EditingMode::Body {
                return Ok(JsValue::undefined());
            }
            let result = Object::new();
            Reflect::set(
                &result,
                &JsValue::from_str("editingMode"),
                &JsValue::from_str(Self::hf_editing_mode_name(mode)),
            )?;
            view.write_hf_caret_to_result(&result)?;
            Ok(result.into())
        })
    }

    /// Word's Shift+click: keep the selection's anchor and move its focus to
    /// the pointer. A drag that follows keeps extending from that anchor.
    /// Without a body selection to extend it is an ordinary pointer down.
    #[wasm_bindgen]
    pub fn pointer_extend(&self, x: f64, y: f64) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            let anchor = {
                let state = view.state.borrow();
                let extendable =
                    state.editing_mode == EditingMode::Body && state.object_selection.is_none();
                extendable
                    .then(|| {
                        view.renderer
                            .borrow()
                            .selection_range()
                            .or(state.selection)
                            .map(|range| range.anchor)
                    })
                    .flatten()
            };
            let Some(anchor) = anchor else {
                return JsFuture::from(view.pointer_down(x, y)).await;
            };
            let anchor_cell = view
                .renderer
                .borrow()
                .cell_id_for_paragraph(anchor.paragraph);
            view.drag_state.replace(PointerDragState::Selecting {
                anchor,
                anchor_cell,
                cell_mode: false,
            });
            view.pointer_move_async(x, y).await?;
            Ok(JsValue::undefined())
        })
    }

    #[wasm_bindgen]
    pub fn pointer_dblclick(&self, x: f64, y: f64) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move { view.pointer_dblclick_async(x, y).await })
    }

    #[wasm_bindgen]
    pub fn pointer_tripleclick(&self, x: f64, y: f64) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move { view.pointer_tripleclick_async(x, y).await })
    }

    /// Starts explicit editing in the header/footer visible on `page_index`,
    /// placing the caret at the end of the requested aligned segment.
    #[wasm_bindgen(js_name = "beginHeaderFooterEditing")]
    pub fn begin_header_footer_editing(
        &self,
        role: String,
        page_index: usize,
        alignment: String,
    ) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            view.begin_hf_editing_async(role, page_index, alignment)
                .await
        })
    }

    /// Inserts a live Word PAGE field using the built-in gallery semantics.
    #[wasm_bindgen(js_name = "insertPageNumber")]
    pub fn insert_page_number(
        &self,
        role: String,
        page_index: usize,
        alignment: String,
    ) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            view.insert_page_number_async(role, page_index, alignment)
                .await
        })
    }

    /// Sets endnote placement for an explicit section or the section owning the caret.
    #[wasm_bindgen(js_name = "setEndnotePosition")]
    pub fn set_endnote_position(
        &self,
        position: String,
        section: Option<usize>,
    ) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            let position = match position.as_str() {
                "sectEnd" => crate::document::EndnotePosition::SectionEnd,
                "docEnd" => crate::document::EndnotePosition::DocumentEnd,
                _ => return Err(JsValue::from_str("unsupported endnote position")),
            };
            view.set_endnote_position_async(position, section).await
        })
    }

    #[wasm_bindgen(js_name = "getEndnotePosition")]
    pub fn get_endnote_position(&self, section: Option<usize>) -> Result<JsValue, JsValue> {
        let section = section.or_else(|| self.ribbon_target_section());
        let state = self.state.borrow();
        let document = state
            .word_doc
            .as_ref()
            .ok_or_else(|| JsValue::from_str("document is not loaded"))?;
        let position = document
            .endnote_position(section)
            .map_err(|error| JsValue::from_str(&error))?;
        to_value(&position).map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Changes note numbering in the explicit section or the section owning the active caret.
    #[wasm_bindgen(js_name = "setNoteNumbering")]
    pub fn set_note_numbering(
        &self,
        kind: String,
        format: String,
        start: u32,
        restart: String,
        section: Option<usize>,
    ) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            let is_footnote = note_kind_is_footnote(&kind)?;
            view.set_note_numbering_async(is_footnote, format, start, restart, section)
                .await
        })
    }

    #[wasm_bindgen(js_name = "getNoteNumbering")]
    pub fn get_note_numbering(
        &self,
        kind: String,
        section: Option<usize>,
    ) -> Result<JsValue, JsValue> {
        let is_footnote = note_kind_is_footnote(&kind)?;
        let section = section.or_else(|| self.ribbon_target_section());
        let state = self.state.borrow();
        let document = state
            .word_doc
            .as_ref()
            .ok_or_else(|| JsValue::from_str("document is not loaded"))?;
        let numbering = document
            .note_numbering(is_footnote, section)
            .map_err(|error| JsValue::from_str(&error))?;
        let format = numbering.number_format.as_deref().unwrap_or("decimal");
        let format = jade_docx_reader::numbering::canonical_number_format(format)
            .unwrap_or(format)
            .to_string();
        let restart = match numbering.number_restart.as_deref().unwrap_or("continuous") {
            "eachsect" => "eachSect",
            "eachpage" => "eachPage",
            other => other,
        }
        .to_string();
        to_value(&NoteNumberingSettingsJs {
            format,
            start: numbering.number_start.unwrap_or(1),
            restart,
        })
        .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Sets the final/default section's PAGE-field numeral style and optional
    /// restart value.
    #[wasm_bindgen(js_name = "setPageNumberFormat")]
    pub fn set_page_number_format(&self, format: String, start: Option<u32>) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move { view.set_page_number_format_async(format, start).await })
    }

    /// Removes PAGE fields from all existing header/footer variants.
    #[wasm_bindgen(js_name = "removePageNumbers")]
    pub fn remove_page_numbers(&self) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move { view.remove_page_numbers_async().await })
    }

    #[wasm_bindgen]
    pub fn editing_mode(&self) -> String {
        let state = self.state.borrow();
        match state.editing_mode {
            EditingMode::Body => "body".to_string(),
            EditingMode::Header => "header".to_string(),
            EditingMode::Footer => "footer".to_string(),
        }
    }

    #[wasm_bindgen]
    pub fn exit_hf_editing(&self) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            let selection = view.state.borrow_mut().exit_hf_editing_mode();
            {
                let mut renderer_mut = view.renderer.borrow_mut();
                renderer_mut.set_selection_range(selection);
                renderer_mut.set_editing_mode(EditingMode::Body);
            }
            view.rerender_document()?;
            Ok(JsValue::undefined())
        })
    }

    #[wasm_bindgen]
    pub fn pointer_move(&self, x: f64, y: f64) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            view.pointer_move_async(x, y).await?;
            Ok(JsValue::undefined())
        })
    }

    #[wasm_bindgen]
    pub fn pointer_up(&self) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            view.pointer_up_async().await?;
            Ok(JsValue::undefined())
        })
    }

    /// Reflows the table to the column/row boundary being dragged.
    #[wasm_bindgen(js_name = pointerResizePreview)]
    pub fn pointer_resize_preview(&self) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            view.preview_table_resize().await?;
            Ok(JsValue::undefined())
        })
    }

    #[wasm_bindgen]
    pub fn pointer_cancel(&self) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            view.pointer_cancel_async().await?;
            Ok(JsValue::undefined())
        })
    }

    /// The word around a collapsed body caret as `{ paragraph, start, end }`
    /// (body paragraph index, byte offsets), using the same word rule as a
    /// double-click. `null` when the selection is not a collapsed body caret
    /// or the caret touches no word.
    ///
    /// Word's Shift+F3 (and character formatting with no selection) acts on
    /// the word the caret is in; the runtime selects this range first.
    #[wasm_bindgen(js_name = wordRangeAtCaret)]
    pub fn word_range_at_caret(&self) -> JsValue {
        let renderer = self.renderer.borrow();
        let Some(range) = renderer.selection_range() else {
            return JsValue::NULL;
        };
        if !range.is_collapsed() {
            return JsValue::NULL;
        }
        let Some((start, end)) = renderer.selection_manager().word_range_at(range.focus) else {
            return JsValue::NULL;
        };
        if end <= start {
            return JsValue::NULL;
        }
        let state = self.state.borrow();
        let Some(position) = state
            .word_doc
            .as_ref()
            .and_then(|document| document.story_position_for_surface(range.focus.paragraph))
            .filter(|position| position.story == "body")
        else {
            return JsValue::NULL;
        };
        let result = Object::new();
        let set = |key: &str, value: usize| {
            Reflect::set(
                &result,
                &JsValue::from_str(key),
                &JsValue::from(value as u32),
            )
        };
        if set("paragraph", position.index).is_err()
            || set("start", start).is_err()
            || set("end", end).is_err()
        {
            return JsValue::NULL;
        }
        result.into()
    }

    #[wasm_bindgen]
    pub fn set_selection_range(
        &self,
        anchor_paragraph: usize,
        anchor_offset: usize,
        focus_paragraph: usize,
        focus_offset: usize,
    ) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            let (anchor_surface, focus_surface) = {
                let state = view.state.borrow();
                let document = state
                    .word_doc
                    .as_ref()
                    .ok_or_else(|| JsValue::from_str("No document loaded"))?;
                let anchor = document
                    .surface_index_for_body(anchor_paragraph)
                    .ok_or_else(|| {
                        JsValue::from_str(&format!(
                            "body story paragraph {anchor_paragraph} is unavailable"
                        ))
                    })?;
                let focus = document
                    .surface_index_for_body(focus_paragraph)
                    .ok_or_else(|| {
                        JsValue::from_str(&format!(
                            "body story paragraph {focus_paragraph} is unavailable"
                        ))
                    })?;
                (anchor, focus)
            };
            view.set_selection_range_surface_async(
                anchor_surface,
                anchor_offset,
                focus_surface,
                focus_offset,
            )
            .await?;
            Ok(JsValue::undefined())
        })
    }

    #[wasm_bindgen(js_name = setSelectionRangeInStory)]
    #[allow(clippy::too_many_arguments)]
    pub fn set_selection_range_in_story(
        &self,
        story: String,
        anchor_paragraph: usize,
        anchor_offset: usize,
        focus_paragraph: usize,
        focus_offset: usize,
        story_anchor_paragraph: Option<usize>,
        note_id: Option<String>,
    ) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            let (anchor_surface, focus_surface) = {
                let state = view.state.borrow();
                let document = state
                    .word_doc
                    .as_ref()
                    .ok_or_else(|| JsValue::from_str("No document loaded"))?;
                let resolve = |index| {
                    document
                        .surface_index_for_story(
                            &story,
                            index,
                            story_anchor_paragraph,
                            note_id.as_deref(),
                        )
                        .ok_or_else(|| {
                            JsValue::from_str(&format!(
                                "{story} story paragraph {index} is unavailable"
                            ))
                        })
                };
                (resolve(anchor_paragraph)?, resolve(focus_paragraph)?)
            };
            view.set_selection_range_surface_async(
                anchor_surface,
                anchor_offset,
                focus_surface,
                focus_offset,
            )
            .await?;
            Ok(JsValue::undefined())
        })
    }

    #[wasm_bindgen]
    pub fn clear_selection(&self) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            view.clear_selection_async().await?;
            Ok(JsValue::undefined())
        })
    }

    #[wasm_bindgen]
    pub fn selection_text(&self) -> Option<String> {
        let (mode, variant) = {
            let state = self.state.borrow();
            (state.editing_mode, state.hf_variant)
        };
        if mode != EditingMode::Body {
            let (start, end) = self.hf_selection()?;
            let text = self.hf_segment_text(mode, variant, &start)?;
            return text.get(start.offset..end.offset).map(str::to_owned);
        }
        self.renderer.borrow().selection_text()
    }

    /// Snapshot the active story in the same UTF-8 coordinates as its caret.
    /// Generated note/reference marks belong to those layout coordinates;
    /// correction validation below prevents replacing them as editable text.
    fn spelling_context_value(&self) -> Option<serde_json::Value> {
        let state = self.state.borrow();
        if state.preview_mode {
            return None;
        }
        let document = state.word_doc.as_ref()?;
        if state.editing_mode != EditingMode::Body {
            let cursor = state.hf_edit_cursor?;
            let anchor = state.hf_selection_anchor.unwrap_or(cursor);
            if anchor.page_index != cursor.page_index
                || anchor.line_index != cursor.line_index
                || anchor.segment != cursor.segment
            {
                return None;
            }
            let text = self.hf_segment_text(state.editing_mode, state.hf_variant, &cursor)?;
            let lines = Self::variant_lines(document, state.editing_mode, state.hf_variant)?;
            let runs = lines
                .get(cursor.line_index)
                .map(|line| match cursor.segment {
                    HfSegment::Left => &line.left_runs,
                    HfSegment::Center => &line.center_runs,
                    HfSegment::Right => &line.right_runs,
                });
            let mut offset = 0;
            let probe = anchor.offset.min(cursor.offset);
            let properties = runs.and_then(|runs| {
                let run = runs
                    .iter()
                    .find(|run| {
                        offset += run.text.len();
                        probe < offset
                    })
                    .or_else(|| runs.last());
                run.and_then(|run| run.properties.as_ref())
            });
            let defaults = document
                .styles_part
                .as_ref()
                .and_then(|styles| styles.doc_default_run_properties());
            let language = properties
                .and_then(|p| p.language.as_deref())
                .or_else(|| properties.and_then(|p| p.east_asia_language.as_deref()))
                .or_else(|| defaults.and_then(|p| p.language.as_deref()))
                .or_else(|| defaults.and_then(|p| p.east_asia_language.as_deref()));
            return Some(serde_json::json!({
                "text": text, "anchorOffset": anchor.offset, "focusOffset": cursor.offset,
                "language": language, "editingMode": state.editing_mode,
                "variant": format!("{:?}", state.hf_variant), "section": document.hf_edit_section,
                "page": cursor.page_index, "line": cursor.line_index,
                "segment": format!("{:?}", cursor.segment),
            }));
        }
        let renderer = self.renderer.borrow();
        let range = renderer.selection_range()?;
        if range.anchor.paragraph != range.focus.paragraph {
            return None;
        }
        let surface = range.focus.paragraph;
        let story = document.story_position_for_surface(surface)?;
        let text = renderer.selection_paragraph_text(surface)?;
        let properties: serde_json::Value =
            serde_wasm_bindgen::from_value(self.run_properties_at_caret()).ok()?;
        let language = properties.get("run").and_then(|run| {
            run.get("language")
                .filter(|v| v.is_string())
                .or_else(|| run.get("east_asia_language").filter(|v| v.is_string()))
        });
        Some(serde_json::json!({
            "text": text, "anchorOffset": range.anchor.offset, "focusOffset": range.focus.offset,
            "language": language, "editingMode": "body", "surface": surface,
            "story": story.story, "paragraph": story.index,
            "anchorParagraph": story.anchor_paragraph, "noteId": story.note_id,
        }))
    }

    #[wasm_bindgen(js_name = spellingContext)]
    pub fn spelling_context(&self) -> JsValue {
        self.spelling_context_value()
            .and_then(|context| {
                context
                    .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
                    .ok()
            })
            .unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen(js_name = applySpellingCorrection)]
    pub fn apply_spelling_correction(
        &self,
        context: String,
        start: usize,
        end: usize,
        replacement: String,
    ) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            let expected: serde_json::Value = serde_json::from_str(&context)
                .map_err(|_| JsValue::from_str("Invalid spelling context"))?;
            if view.spelling_context_value().as_ref() != Some(&expected) {
                return Err(JsValue::from_str("Spelling context changed"));
            }
            let text = expected["text"].as_str().unwrap_or("");
            let anchor = expected["anchorOffset"].as_u64().unwrap_or(u64::MAX);
            let focus = expected["focusOffset"].as_u64().unwrap_or(u64::MAX);
            if replacement.is_empty()
                || start >= end
                || end > text.len()
                || !text.is_char_boundary(start)
                || !text.is_char_boundary(end)
                || anchor < start as u64
                || anchor > end as u64
                || focus < start as u64
                || focus > end as u64
            {
                return Err(JsValue::from_str("Invalid spelling correction range"));
            }
            let mode = view.state.borrow().editing_mode;
            if mode != EditingMode::Body {
                let mut state = view.state.borrow_mut();
                let cursor = state
                    .hf_edit_cursor
                    .ok_or_else(|| JsValue::from_str("Missing header/footer caret"))?;
                state.hf_selection_anchor = Some(HfEditCursor {
                    offset: start,
                    ..cursor
                });
                state.hf_edit_cursor = Some(HfEditCursor {
                    offset: end,
                    ..cursor
                });
            } else {
                let surface = expected["surface"]
                    .as_u64()
                    .ok_or_else(|| JsValue::from_str("Missing spelling story"))?
                    as usize;
                {
                    let state = view.state.borrow();
                    let document = state
                        .word_doc
                        .as_ref()
                        .ok_or_else(|| JsValue::from_str("Missing document"))?;
                    let model_start =
                        document.rendered_to_model_text_position(TextPosition::new(surface, start));
                    let model_end =
                        document.rendered_to_model_text_position(TextPosition::new(surface, end));
                    if model_end.offset.saturating_sub(model_start.offset) != end - start {
                        return Err(JsValue::from_str(
                            "Spelling correction overlaps a generated mark",
                        ));
                    }
                }
                view.set_selection_range_surface_async(surface, start, surface, end)
                    .await?;
            }
            view.insert_text_async(replacement).await
        })
    }

    #[wasm_bindgen]
    pub fn selection_range(&self) -> JsValue {
        let renderer = self.renderer.borrow();
        match renderer.selection_range() {
            Some(range) => {
                let state = self.state.borrow();
                let Some(document) = state.word_doc.as_ref() else {
                    return JsValue::NULL;
                };
                let Some(anchor_story) =
                    document.story_position_for_surface(range.anchor.paragraph)
                else {
                    return JsValue::NULL;
                };
                let Some(focus_story) = document.story_position_for_surface(range.focus.paragraph)
                else {
                    return JsValue::NULL;
                };
                to_value(&RuntimeSelectionRange {
                    anchor: RuntimeTextPosition {
                        paragraph: anchor_story.index,
                        offset: range.anchor.offset,
                        story: anchor_story.story,
                        anchor_paragraph: anchor_story.anchor_paragraph,
                        note_id: anchor_story.note_id,
                    },
                    focus: RuntimeTextPosition {
                        paragraph: focus_story.index,
                        offset: range.focus.offset,
                        story: focus_story.story,
                        anchor_paragraph: focus_story.anchor_paragraph,
                        note_id: focus_story.note_id,
                    },
                })
                .unwrap_or(JsValue::NULL)
            }
            None => JsValue::NULL,
        }
    }

    /// Capture the current paste destination without treating a header/footer
    /// paragraph index as an index in the body document package.
    #[wasm_bindgen]
    pub fn clipboard_selection(&self) -> JsValue {
        let state = self.state.borrow();
        if state.editing_mode == EditingMode::Body {
            drop(state);
            return self.selection_range();
        }
        let Some(focus) = state.hf_edit_cursor else {
            return JsValue::NULL;
        };
        to_value(&HfClipboardSelection {
            variant: state.hf_variant,
            anchor: state.hf_selection_anchor.unwrap_or(focus),
            focus,
            package_target: Self::hf_package_target(&state),
        })
        .unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen]
    pub fn object_selection_state(&self) -> JsValue {
        self.object_selection_payload()
            .and_then(|selection| to_value(&selection).ok())
            .unwrap_or(JsValue::NULL)
    }

    /// Selection payload for the shape the caret sits in (see
    /// [`Self::caret_text_box_target`]); null outside shapes. Unlike
    /// `objectSelectionState` this never drives the canvas overlay, so typing
    /// in the box keeps its text caret.
    #[wasm_bindgen(js_name = caretTextBoxSelection)]
    pub fn caret_text_box_selection(&self) -> JsValue {
        self.caret_text_box_target()
            .and_then(|target| self.object_selection_payload_for(target))
            .and_then(|selection| to_value(&selection).ok())
            .unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen(js_name = selectedImageDataUrl)]
    pub fn selected_image_data_url(&self) -> Option<String> {
        let target = self.state.borrow().object_selection?;
        if target.kind != FloatingObjectKind::Image {
            return None;
        }
        self.state
            .borrow()
            .word_doc
            .as_ref()
            .and_then(|document| document.paragraph(target.paragraph_index))
            .and_then(|paragraph| paragraph.runs.get(target.run_index))
            .and_then(|run| run.image.as_ref())
            .and_then(|image| image.data_uri.clone())
    }

    #[wasm_bindgen(js_name = formatSelectedImage)]
    pub fn format_selected_image(&self, value: JsValue) -> Result<js_sys::Promise, JsValue> {
        if value.is_null() || value.is_undefined() || !value.is_object() {
            return Err(JsValue::from_str("image format must be an object"));
        }
        let input: ImageFormatInput = serde_wasm_bindgen::from_value(value)
            .map_err(|error| JsValue::from_str(&format!("invalid image format: {error}")))?;
        let spec = ImageFormatSpec::try_from(input).map_err(|error| JsValue::from_str(&error))?;
        let view = self.clone();
        Ok(future_to_promise(async move {
            view.format_selected_image_async(spec).await
        }))
    }

    #[wasm_bindgen(js_name = formatSelectedTextBox)]
    pub fn format_selected_text_box(&self, value: JsValue) -> Result<js_sys::Promise, JsValue> {
        if value.is_null() || value.is_undefined() || !value.is_object() {
            return Err(JsValue::from_str("text box format must be an object"));
        }
        let input: TextBoxFormatInput = serde_wasm_bindgen::from_value(value)
            .map_err(|error| JsValue::from_str(&format!("invalid text box format: {error}")))?;
        let spec = TextBoxFormatSpec::try_from(input).map_err(|error| JsValue::from_str(&error))?;
        spec.validate().map_err(|error| JsValue::from_str(&error))?;
        let view = self.clone();
        Ok(future_to_promise(async move {
            view.format_selected_text_box_async(spec).await
        }))
    }

    #[wasm_bindgen(js_name = replaceSelectedImage)]
    pub fn replace_selected_image(
        &self,
        image_bytes: Vec<u8>,
        mime_type: String,
        width_px: Option<f64>,
        height_px: Option<f64>,
    ) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            view.replace_selected_image_async(image_bytes, mime_type, width_px, height_px)
                .await
        })
    }

    #[wasm_bindgen(js_name = previewSelectedObjectTransform)]
    pub fn preview_selected_object_transform(
        &self,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        rotation_deg: f64,
        reanchor: bool,
    ) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            view.preview_selected_object_transform_async(
                FloatingObjectTransform {
                    x,
                    y,
                    width,
                    height,
                    rotation_deg,
                },
                reanchor,
            )
            .await
        })
    }

    #[wasm_bindgen(js_name = commitSelectedObjectTransform)]
    pub fn commit_selected_object_transform(
        &self,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        rotation_deg: f64,
        reanchor: bool,
    ) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move {
            view.commit_selected_object_transform_async(
                FloatingObjectTransform {
                    x,
                    y,
                    width,
                    height,
                    rotation_deg,
                },
                reanchor,
            )
            .await
        })
    }

    #[wasm_bindgen(js_name = clearSelectedObjectTransformPreview)]
    pub fn clear_selected_object_transform_preview(&self) -> js_sys::Promise {
        let view = self.clone();
        future_to_promise(async move { view.clear_selected_object_transform_preview_async().await })
    }

    #[wasm_bindgen]
    pub fn can_merge_selected_cells(&self) -> bool {
        self.can_merge_selected_cells_internal()
    }

    #[wasm_bindgen]
    pub fn can_unmerge_selected_cells(&self) -> bool {
        self.can_unmerge_selected_cells_internal()
    }

    #[wasm_bindgen]
    pub fn cell_selection_debug(&self) -> JsValue {
        let renderer = self.renderer.borrow();
        match renderer.selected_cell_span() {
            Some(span) => to_value(&span).unwrap_or(JsValue::NULL),
            None => JsValue::NULL,
        }
    }

    #[wasm_bindgen]
    pub fn cell_layouts_debug(&self) -> JsValue {
        let renderer = self.renderer.borrow();
        to_value(&renderer.cell_layouts_debug()).unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen]
    pub fn table_resize_metadata(&self) -> JsValue {
        let Ok(state) = self.state.try_borrow() else {
            // Rendering/editing may hold a mutable borrow; return null instead of panicking.
            return JsValue::NULL;
        };
        to_value(&state.table_resize).unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen]
    pub fn selection_debug_lines(&self) -> JsValue {
        let renderer = self.renderer.borrow();
        to_value(&renderer.selection_debug_lines()).unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen]
    pub fn layout_debug_lines(&self) -> JsValue {
        let renderer = self.renderer.borrow();
        to_value(&renderer.layout_debug_lines()).unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen]
    pub fn set_auxiliary_story_capture_enabled(&self, enabled: bool) {
        self.renderer
            .borrow_mut()
            .set_auxiliary_story_capture_enabled(enabled);
    }

    #[wasm_bindgen]
    pub fn auxiliary_story_debug_lines(&self) -> JsValue {
        let renderer = self.renderer.borrow();
        to_value(renderer.auxiliary_story_debug_lines()).unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen(js_name = captureFullLayoutDebug)]
    pub fn capture_full_layout_debug(&self) -> Result<JsValue, JsValue> {
        let snapshot = self.renderer.borrow_mut().capture_full_layout_debug()?;
        to_value(&snapshot).map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Layout lines for the given paragraph indices only. Search highlighting
    /// asks for the matched paragraphs instead of every glyph in the document.
    #[wasm_bindgen]
    pub fn layout_debug_lines_for_paragraphs(&self, paragraphs: &[u32]) -> JsValue {
        let renderer = self.renderer.borrow();
        let indices: Vec<usize> = paragraphs.iter().map(|index| *index as usize).collect();
        to_value(&renderer.layout_debug_lines_for_paragraphs(&indices)).unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen]
    pub fn paragraph_debug_texts(&self) -> js_sys::Array {
        let renderer = self.renderer.borrow();
        let texts = renderer.paragraph_debug_texts();
        let array = js_sys::Array::new();
        for text in texts {
            array.push(&JsValue::from(text));
        }
        array
    }

    #[wasm_bindgen]
    pub fn image_debug_regions(&self) -> JsValue {
        let renderer = self.renderer.borrow();
        to_value(&renderer.image_debug_regions()).unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen]
    pub fn text_box_debug_regions(&self) -> JsValue {
        let renderer = self.renderer.borrow();
        to_value(&renderer.text_box_debug_regions()).unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen]
    pub fn revision_debug_regions(&self) -> JsValue {
        let renderer = self.renderer.borrow();
        to_value(&renderer.revision_debug_regions()).unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen]
    pub fn numbering_debug_regions(&self) -> JsValue {
        let renderer = self.renderer.borrow();
        to_value(&renderer.numbering_debug_regions()).unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen]
    pub fn line_overflow_debug_regions(&self) -> JsValue {
        let renderer = self.renderer.borrow();
        to_value(&renderer.line_overflow_debug_regions()).unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen]
    pub fn hyperlink_regions(&self) -> JsValue {
        let renderer = self.renderer.borrow();
        // Rebuild from the current document metadata and existing jade-docx-layout line fragments.
        // Painted regions are a useful draw cache, but they can outlive metadata-only edits
        // such as Remove Hyperlink because paragraph geometry remains identical. Treating the
        // document as authoritative also keeps table/text-box paragraph coordinates current.
        let state = self.state.borrow();
        let Some(document) = state.word_doc.as_ref() else {
            return renderer.hyperlink_regions_js_value();
        };
        let mut derived = renderer.derive_hyperlink_regions_from_document(document);
        renderer.append_object_hyperlink_regions(&mut derived);
        to_value(&derived).unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen(js_name = pageHyperlinkRegions)]
    pub fn page_hyperlink_regions(&self, page_index: usize) -> JsValue {
        to_value(&self.renderer.borrow().page_hyperlink_regions(page_index))
            .unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen(js_name = pageBookmarks)]
    pub fn page_bookmarks(&self) -> JsValue {
        let state = self.state.borrow();
        let Some(document) = state.word_doc.as_ref() else {
            return JsValue::NULL;
        };
        to_value(&self.renderer.borrow().page_bookmarks(document)).unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen(js_name = beginPdfTextCapture)]
    pub fn begin_pdf_text_capture(&self) {
        jade_canvas_painter::text_batch::begin_text_capture();
    }

    #[wasm_bindgen(js_name = endPdfTextCapture)]
    pub fn end_pdf_text_capture(&self) {
        jade_canvas_painter::text_batch::end_text_capture();
    }

    #[wasm_bindgen(js_name = takePagePdfTextRuns)]
    pub fn take_page_pdf_text_runs(&self, page_index: usize) -> JsValue {
        self.renderer.borrow().take_page_text_capture(page_index)
    }

    /// Returns user and document bookmarks in source order for a host-side
    /// bookmark manager / cross-reference picker.
    #[wasm_bindgen(js_name = listBookmarks)]
    pub fn list_bookmarks(&self) -> JsValue {
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct BookmarkInfo {
            name: String,
            start_paragraph: usize,
            start_offset: usize,
            end_paragraph: usize,
            end_offset: usize,
            preview: String,
        }

        let state = self.state.borrow();
        let Some(word_doc) = state.word_doc.as_ref() else {
            return to_value(&Vec::<BookmarkInfo>::new()).unwrap_or(JsValue::NULL);
        };
        let Some(document) = word_doc.document.as_ref() else {
            return to_value(&Vec::<BookmarkInfo>::new()).unwrap_or(JsValue::NULL);
        };
        let bookmarks = document
            .bookmarks
            .iter()
            .map(|bookmark| {
                let preview = word_doc
                    .bookmark_surface_index(bookmark.start_paragraph)
                    .and_then(|index| word_doc.paragraph(index))
                    .map(|paragraph| {
                        paragraph
                            .runs
                            .iter()
                            .map(|run| run.text.as_str())
                            .collect::<String>()
                    })
                    .unwrap_or_default()
                    .chars()
                    .take(80)
                    .collect();
                BookmarkInfo {
                    name: bookmark.name.clone(),
                    start_paragraph: word_doc
                        .bookmark_surface_index(bookmark.start_paragraph)
                        .unwrap_or(bookmark.start_paragraph),
                    start_offset: bookmark.start_offset,
                    end_paragraph: word_doc
                        .bookmark_surface_index(bookmark.end_paragraph)
                        .unwrap_or(bookmark.end_paragraph),
                    end_offset: bookmark.end_offset,
                    preview,
                }
            })
            .collect::<Vec<_>>();
        to_value(&bookmarks).unwrap_or(JsValue::NULL)
    }

    /// Returns the semantic top-level heading outline in document order.
    /// `pageIndex` is best-effort in lazy mode and remains null until the
    /// heading's paragraph has been laid out; paragraph coordinates are always
    /// available and can still be used to move the caret and request layout.
    #[wasm_bindgen(js_name = listNavigationHeadings)]
    pub fn list_navigation_headings(&self) -> JsValue {
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct NavigationHeadingJs {
            paragraph_index: usize,
            level: u8,
            text: String,
            page_index: Option<usize>,
        }

        let state = self.state.borrow();
        let Some(word_doc) = state.word_doc.as_ref() else {
            return to_value(&Vec::<NavigationHeadingJs>::new()).unwrap_or(JsValue::NULL);
        };
        let renderer = self.renderer.borrow();
        let headings = word_doc
            .navigation_headings()
            .into_iter()
            .map(|heading| NavigationHeadingJs {
                paragraph_index: heading.paragraph_index,
                level: heading.level,
                text: heading.text,
                page_index: renderer
                    .cached_paragraph_start_page(heading.paragraph_index)
                    .map(|page| page.saturating_sub(1)),
            })
            .collect::<Vec<_>>();
        to_value(&headings).unwrap_or(JsValue::NULL)
    }

    /// Resolves a bookmark name (as used by `<w:hyperlink w:anchor="name"/>`)
    /// to its start position. Returns `null` when the document has no matching
    /// bookmark. The `pageIndex` field is 0-based and may be `null` when the
    /// paragraph has not yet been laid out (e.g. lazy-load mode before the
    /// target page is materialized) — callers should fall back to
    /// `ensureLayoutThroughPage` or simply set the selection and rely on the
    /// existing caret-into-view plumbing.
    #[wasm_bindgen(js_name = bookmarkPosition)]
    pub fn bookmark_position(&self, name: &str) -> JsValue {
        let state = self.state.borrow();
        let Some(word_doc) = state.word_doc.as_ref() else {
            return JsValue::NULL;
        };
        let Some(document) = word_doc.document.as_ref() else {
            return JsValue::NULL;
        };
        // Linear scan — documents rarely carry more than a handful of
        // bookmarks and this is only called on user-driven clicks. No index
        // structure yet.
        let Some(bookmark) = document.bookmarks.iter().find(|b| b.name == name) else {
            return JsValue::NULL;
        };
        let Some(paragraph) = word_doc.bookmark_surface_index(bookmark.start_paragraph) else {
            return JsValue::NULL;
        };
        let page_index = self
            .renderer
            .borrow()
            .cached_paragraph_start_page(paragraph)
            .map(|p| p.saturating_sub(1));

        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct BookmarkPosition {
            paragraph: usize,
            offset: usize,
            page_index: Option<usize>,
        }

        to_value(&BookmarkPosition {
            paragraph,
            offset: bookmark.start_offset,
            page_index,
        })
        .unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen]
    pub fn page_height(&self) -> f64 {
        self.renderer.borrow().page_height_css()
    }

    #[wasm_bindgen]
    pub fn first_paragraph_text(&self) -> Option<String> {
        let state = self.state.borrow();
        let word_doc = state.word_doc.as_ref()?;
        let document = word_doc.document.as_ref()?;
        let body = document.body.as_ref()?;
        for element in &body.elements {
            if let crate::document::BodyElement::Paragraph(paragraph) = element {
                let paragraph = paragraph.as_ref();
                let mut text = String::new();
                for run in &paragraph.runs {
                    text.push_str(&run.text);
                }
                return Some(text);
            }
        }
        None
    }

    #[wasm_bindgen]
    pub fn paragraph_count(&self) -> usize {
        let state = self.state.borrow();
        state
            .word_doc
            .as_ref()
            .map(|doc| doc.body_paragraph_count())
            .unwrap_or(0)
    }

    #[wasm_bindgen]
    pub fn paragraph_text(&self, index: usize) -> Option<String> {
        let state = self.state.borrow();
        let doc = state.word_doc.as_ref()?;
        let paragraph = doc.paragraph(doc.surface_index_for_body(index)?)?;
        let mut text = String::new();
        for run in &paragraph.runs {
            text.push_str(&run.text);
        }
        Some(text)
    }

    /// Cached text counts for the status bar. This avoids copying every
    /// paragraph through the wasm boundary after each keystroke.
    #[wasm_bindgen]
    pub fn document_text_metrics(&self) -> JsValue {
        let state = self.state.borrow();
        let Some(doc) = state.word_doc.as_ref() else {
            return JsValue::NULL;
        };
        to_value(&doc.document_text_metrics()).unwrap_or(JsValue::NULL)
    }

    /// Searches the body for `query`, returning up to `max_results` matches as
    /// `[{paragraph, startOffset, endOffset}]` (byte offsets into the
    /// paragraph's concatenated run text, matching the selection/replace
    /// coordinate system). Used by the IDE 検索/置換 panel.
    #[wasm_bindgen(js_name = "searchText")]
    pub fn search_text(&self, query: &str, max_results: usize) -> JsValue {
        if query.is_empty() {
            return to_value(&Vec::<SearchMatchJs>::new()).unwrap_or(JsValue::NULL);
        }
        let state = self.state.borrow();
        let Some(doc) = state.word_doc.as_ref() else {
            return JsValue::NULL;
        };
        let total = doc.body_paragraph_count();
        let limit = max_results.max(1);
        let mut matches: Vec<SearchMatchJs> = Vec::new();
        'paragraphs: for paragraph_index in 0..total {
            let Some(surface_index) = doc.surface_index_for_body(paragraph_index) else {
                continue;
            };
            let Some(paragraph) = doc.paragraph(surface_index) else {
                continue;
            };
            let mut text = String::new();
            for run in &paragraph.runs {
                text.push_str(&run.text);
            }
            for (start_offset, matched) in text.match_indices(query) {
                matches.push(SearchMatchJs {
                    paragraph: paragraph_index,
                    start_offset,
                    end_offset: start_offset + matched.len(),
                });
                if matches.len() >= limit {
                    break 'paragraphs;
                }
            }
        }
        to_value(&matches).unwrap_or(JsValue::NULL)
    }

    /// Returns the 0-based page index a body paragraph starts on, or null when
    /// the paragraph has not been laid out yet (lazy mode). Used by the IDE to
    /// scroll a search match / comment anchor into view.
    #[wasm_bindgen(js_name = "paragraphPage")]
    pub fn paragraph_page(&self, index: usize) -> JsValue {
        let Some(surface_index) = self
            .state
            .borrow()
            .word_doc
            .as_ref()
            .and_then(|document| document.surface_index_for_body(index))
        else {
            return JsValue::NULL;
        };
        let renderer = self.renderer.borrow();
        match renderer.cached_paragraph_start_page(surface_index) {
            // `cached_paragraph_start_page` is 1-based; convert to 0-based to
            // match the `[data-page-index]` attribute the host renders.
            Some(page) if page >= 1 => JsValue::from_f64((page - 1) as f64),
            _ => JsValue::NULL,
        }
    }

    #[wasm_bindgen]
    pub fn section_ranges(&self) -> JsValue {
        let state = self.state.borrow();
        let word_doc = match state.word_doc.as_ref() {
            Some(doc) => doc,
            None => return JsValue::NULL,
        };
        let document = match word_doc.document.as_ref() {
            Some(doc) => doc,
            None => return JsValue::NULL,
        };
        to_value(&document.sections).unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen]
    pub fn hit_test(&self, x: f64, y: f64) -> JsValue {
        let renderer = self.renderer.borrow();
        let state = self.state.borrow();
        let position = state
            .word_doc
            .as_ref()
            // Public hit-testing must not leak renderer-only surfaces such as
            // headers and footers: callers feed this position back into model
            // commands. Keep every model-backed story (body, textbox, and
            // footnote) eligible.
            .and_then(|document| renderer.hit_test_document_position(x, y, document));
        match position {
            Some(position) => state
                .word_doc
                .as_ref()
                .and_then(|document| Self::runtime_text_position(document, position))
                .and_then(|position| to_value(&position).ok())
                .unwrap_or(JsValue::NULL),
            None => JsValue::NULL,
        }
    }

    /// Page/content bounds in CSS pixels, without changing or consulting the caret.
    #[wasm_bindgen(js_name = pageGeometry)]
    pub fn page_geometry(&self, page_index: usize) -> JsValue {
        let state = self.state.borrow();
        let Some(document) = state
            .word_doc
            .as_ref()
            .and_then(|word| word.document.as_ref())
        else {
            return JsValue::NULL;
        };
        let renderer = self.renderer.borrow();
        if page_index >= renderer.page_count() {
            return JsValue::NULL;
        }
        let section = document
            .sections
            .get(renderer.section_index_for_zero_based_page(page_index))
            .and_then(|section| section.properties.as_ref());
        let geometry =
            jade_docx_layout::text_extract::ruler_geometry(document, section, page_index + 1, 0.0);
        let value = to_value(&geometry).unwrap_or(JsValue::NULL);
        let _ = Reflect::set(
            &value,
            &JsValue::from_str("pageIndex"),
            &JsValue::from_f64(page_index as f64),
        );
        value
    }

    /// Geometry for UI rulers, resolved by the shared layout engine.
    #[wasm_bindgen(js_name = rulerGeometry)]
    pub fn ruler_geometry(&self) -> JsValue {
        let state = self.state.borrow();
        let Some(word_doc) = state.word_doc.as_ref() else {
            return JsValue::NULL;
        };
        let Some(document) = word_doc.document.as_ref() else {
            return JsValue::NULL;
        };
        let selection = state
            .selection
            .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::new(0, 0)));
        let renderer = self.renderer.borrow();
        let hf_paragraph = state.hf_edit_cursor.and_then(|cursor| {
            let (role, variant) = Self::hf_role_variant(state.editing_mode, state.hf_variant)?;
            word_doc.hf_ruler_paragraph(role, variant, cursor.line_index)
        });
        let hf_geometry = if state.editing_mode != EditingMode::Body {
            state.hf_edit_cursor.and_then(|cursor| {
                self.compute_hf_caret_rect(&renderer, &cursor)
                    .map(|(x, _, _)| (cursor.page_index, x))
            })
        } else {
            None
        };
        if state.editing_mode != EditingMode::Body && hf_geometry.is_none() {
            return JsValue::NULL;
        }
        let focus_geometry = hf_geometry.or_else(|| {
            renderer
                .caret_page_index()
                .zip(renderer.caret_metrics())
                .map(|(page, metrics)| (page, metrics.x))
                .or_else(|| {
                    let rects = renderer.get_selection_rects();
                    let backwards = (selection.anchor.paragraph, selection.anchor.offset)
                        > (selection.focus.paragraph, selection.focus.offset);
                    let rect = if backwards {
                        rects.first()
                    } else {
                        rects.last()
                    }?;
                    Some((
                        rect.page,
                        if backwards {
                            rect.x
                        } else {
                            rect.x + rect.width
                        },
                    ))
                })
        });
        let focus_geometry = focus_geometry.or_else(|| {
            renderer
                .paragraph_layout_origin(selection.focus.paragraph)
                .map(|(page, x, _)| (page, x))
        });
        let Some((page_index, caret_x)) = focus_geometry else {
            return JsValue::NULL;
        };
        let section_index = if state.editing_mode == EditingMode::Body {
            word_doc.section_index_for_paragraph(selection.focus.paragraph)
        } else {
            Some(renderer.section_index_for_zero_based_page(page_index))
        };
        let section = section_index
            .and_then(|index| document.sections.get(index))
            .and_then(|section| section.properties.as_ref());
        let mut geometry = jade_docx_layout::text_extract::ruler_geometry(
            document,
            section,
            page_index + 1,
            caret_x,
        );
        // Header/footer stories span the page content width, even in a
        // section whose body flows through multiple columns.
        if state.editing_mode != EditingMode::Body {
            geometry.text_left = geometry.content_left;
            geometry.text_right = geometry.content_right;
        }
        let in_cell = state.editing_mode == EditingMode::Body
            && renderer
                .cell_id_for_paragraph(selection.focus.paragraph)
                .is_some();
        let cell_bounds = (state.editing_mode == EditingMode::Body)
            .then(|| {
                renderer
                    .caret_metrics_at(selection.focus)
                    .and_then(|caret| {
                        renderer.selection_manager().cell_text_bounds_for_paragraph(
                            selection.focus.paragraph,
                            caret.absolute_top,
                        )
                    })
            })
            .flatten();
        if let Some((left, right)) = cell_bounds {
            geometry.text_left = left;
            geometry.text_right = right;
        }
        let value = to_value(&geometry).unwrap_or(JsValue::NULL);
        let _ = Reflect::set(
            &value,
            &JsValue::from_str("paragraphEditable"),
            &JsValue::from_bool(if state.editing_mode == EditingMode::Body {
                !in_cell || cell_bounds.is_some()
            } else {
                hf_paragraph.is_some()
            }),
        );
        let paragraph = if state.editing_mode == EditingMode::Body {
            word_doc.paragraph(selection.focus.paragraph)
        } else {
            hf_paragraph.as_ref()
        };
        if let Some(paragraph) = paragraph {
            let unit = jade_docx_layout::text_extract::ruler_character_unit_px(
                document,
                section,
                paragraph,
                word_doc.styles_part.as_ref(),
            );
            let _ = Reflect::set(
                &value,
                &JsValue::from_str("characterWidth"),
                &JsValue::from_f64(unit),
            );
        }
        let _ = Reflect::set(
            &value,
            &JsValue::from_str("pageIndex"),
            &JsValue::from_f64(page_index as f64),
        );
        value
    }

    /// Returns a paragraph's first line origin in page-local CSS pixels.
    #[wasm_bindgen(js_name = paragraphLayoutOrigin)]
    pub fn paragraph_layout_origin(&self, index: usize) -> JsValue {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct ParagraphOrigin {
            page_index: usize,
            x: f64,
            y: f64,
        }

        let Some(surface_index) = self
            .state
            .borrow()
            .word_doc
            .as_ref()
            .and_then(|document| document.surface_index_for_body(index))
        else {
            return JsValue::NULL;
        };

        self.renderer
            .borrow()
            .paragraph_layout_origin(surface_index)
            .map(|(page_index, x, y)| ParagraphOrigin { page_index, x, y })
            .and_then(|origin| to_value(&origin).ok())
            .unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen]
    pub fn rerender(&self) -> Result<(), JsValue> {
        let (metadata, snapshots) = {
            let mut renderer = self.renderer.borrow_mut();
            renderer.skip_change_detection_once();
            renderer.rerender_with_cached_state()?;
            let metadata = renderer.table_resize_metadata();
            let (snapshots, _dirty_regions) = self.capture_paged_images(&mut renderer)?;
            (metadata, snapshots)
        };
        self.store_paged_images(snapshots);
        {
            let mut state_mut = self.state.borrow_mut();
            state_mut.table_resize = metadata;
        }
        Ok(())
    }

    /// Re-renders the document into the offscreen canvases, but does NOT snapshot/transfer ImageBitmaps.
    /// This is important for viewport/lazy mode: `create_page_bitmap()` relies on canvases retaining pixels.
    #[wasm_bindgen(js_name = rerenderWithoutSnapshots)]
    pub fn rerender_without_snapshots(&self) -> Result<(), JsValue> {
        let mut renderer = self.renderer.borrow_mut();
        renderer.skip_change_detection_once();
        renderer.rerender_with_cached_state()?;
        Ok(())
    }

    /// Prepares a single-page rerender by seeding a seek snapshot for the page.
    #[wasm_bindgen(js_name = prepareRerenderForPage)]
    pub fn prepare_rerender_for_page(&self, page_index: usize) {
        let mut renderer = self.renderer.borrow_mut();
        renderer.prepare_rerender_for_page(page_index);
    }

    /// Prepares a dirty-page bitmap rerender to resume at the first valid
    /// visible-window snapshot. A spanning snapshot restarts at the paragraph
    /// beginning, so the whole cross-page paragraph is painted fresh; if no
    /// valid snapshot exists, the renderer conservatively walks from the start.
    #[wasm_bindgen(js_name = preferVisibleWindowSeekOnce)]
    pub fn prefer_visible_window_seek_once(&self) {
        let mut renderer = self.renderer.borrow_mut();
        renderer.prefer_visible_window_seek_once();
    }

    #[wasm_bindgen(js_name = disableSeekReplayOnce)]
    pub fn disable_seek_replay_once(&self) {
        let mut renderer = self.renderer.borrow_mut();
        renderer.disable_seek_replay_once();
    }

    /// Freezes the selection manager for the next pixel-only re-render so a
    /// bitmap fetch (which narrows the visible window) does not rebuild the
    /// global selection from that narrow window and drop on-screen paragraphs'
    /// line layouts. Without this, clicking after scrolling could land the caret
    /// on the wrong paragraph. The render auto-unfreezes when it completes.
    #[wasm_bindgen(js_name = preserveSelectionForNextRerender)]
    pub fn preserve_selection_for_next_rerender(&self) {
        let mut renderer = self.renderer.borrow_mut();
        renderer.freeze_selection_for_pixel_rerender();
    }

    #[wasm_bindgen]
    pub fn canvas_size(&self) -> Result<JsValue, JsValue> {
        let renderer = self.renderer.borrow();
        let (width, height) = renderer.canvas_size_css();
        to_value(&CanvasSize { width, height }).map_err(|e| JsValue::from_str(&format!("{e}")))
    }

    #[wasm_bindgen]
    pub fn paged_images(&self) -> JsValue {
        let state = self.state.borrow();
        if state.paged_images.is_empty() {
            JsValue::NULL
        } else {
            let array = Array::new();
            for value in &state.paged_images {
                array.push(value);
            }
            array.into()
        }
    }

    /// Returns the total number of pages.
    #[wasm_bindgen]
    pub fn page_count(&self) -> usize {
        self.renderer.borrow().page_count()
    }

    /// Returns the currently configured layout page limit (1-indexed, inclusive) or null.
    ///
    /// This is primarily a debugging/testing hook so the host can verify whether
    /// lazy layout limits are preserved across editing operations.
    #[wasm_bindgen(js_name = layoutPageLimit)]
    pub fn layout_page_limit(&self) -> JsValue {
        match self.renderer.borrow().layout_page_limit() {
            Some(limit) => JsValue::from_f64(limit as f64),
            None => JsValue::NULL,
        }
    }

    #[wasm_bindgen(js_name = debugRendererStats)]
    pub fn debug_renderer_stats(&self) -> JsValue {
        #[derive(Serialize)]
        struct CacheStatsView {
            hits: u64,
            misses: u64,
            size: usize,
            capacity: usize,
        }

        #[derive(serde::Serialize)]
        struct ParagraphWrapCacheStatsView {
            hits: u64,
            misses: u64,
            table_hits: u64,
            table_misses: u64,
            size: usize,
            capacity: usize,
        }

        #[derive(Serialize)]
        struct TableAutofitCacheStatsView {
            enabled: bool,
            hits: u64,
            misses: u64,
            invalidations: u64,
            saved_paragraph_measures: u64,
            size: usize,
            capacity: usize,
        }

        #[derive(Serialize)]
        struct ParagraphLayoutStatsView {
            hits: u64,
            misses: u64,
            skipped_paragraphs: u64,
            cached_entries: usize,
            total_capacity: usize,
        }

        #[derive(Serialize)]
        struct ParagraphRenderStatsView {
            hits: u64,
            misses: u64,
            cached_entries: usize,
            total_capacity: usize,
        }

        #[derive(Serialize)]
        struct WordOracleStatsView {
            sized_advances: usize,
            text_advances: usize,
            paragraph_advances: usize,
            paragraph_counts: Vec<(usize, usize)>,
            report_x_offsets: usize,
        }

        #[derive(Serialize)]
        struct EastAsianStatsView {
            use_east_asian_layout: bool,
            character_spacing_control: Option<String>,
        }

        #[derive(Serialize)]
        struct ContextRebuildStatsView {
            context_rebuilds: u64,
            context_rebuild_skips: u64,
            canvas_inflates: u64,
            pages_seek_skipped: u64,
            context_creations: u64,
            context_creation_deferrals: u64,
            page_slot_recycles: u64,
            page_slot_reuses: u64,
            page_slot_pool_len: usize,
            hot_backing_recycles: u64,
            hot_backing_reuses: u64,
            hot_backing_pool_len: usize,
            hot_backing_pool_bytes: u64,
        }

        #[derive(Serialize)]
        struct WindowSeekStatsView {
            exact_hits: u64,
            fallback_hits: u64,
            misses: u64,
        }

        #[derive(Serialize)]
        struct LayoutExtensionSeekStatsView {
            enabled: bool,
            hits: u64,
            misses: u64,
            last_miss: String,
            last_resume_page: Option<usize>,
            last_resume_body_index: Option<usize>,
            last_visited_body_elements: u64,
            last_invalidated_checkpoints: u64,
            invalidated_checkpoints: u64,
        }

        #[derive(Serialize)]
        struct GlyphBatchStatsView {
            // Initial-render (immediate draw) path.
            batch_runs: u64,
            batch_glyphs: u64,
            fallback_runs: u64,
            fallback_glyphs: u64,
            // Draw-command replay path (cached paragraph redraw).
            replay_batch_runs: u64,
            replay_batch_glyphs: u64,
            replay_fallback_runs: u64,
            replay_fallback_glyphs: u64,
            // Whole-string `DrawCommand::Text` replays (font-grouped segment
            // fillText, never per-glyph batched). `glyphs` counts graphemes.
            replay_text_runs: u64,
            replay_text_glyphs: u64,
        }

        #[derive(Serialize)]
        struct BandRedrawStatsView {
            enabled: bool,
            eligible: u64,
            taken: u64,
            /// Fallback reason → count. Present so a probe can verify which gate
            /// rejected band mode at each depth.
            fallback: std::collections::BTreeMap<String, u64>,
            /// Dirty visible pages a lazy band edit left unpainted; their
            /// pixels are repainted when fetched.
            skipped_pages: u64,
        }

        #[derive(Serialize)]
        struct SpanSeekStatsView {
            enabled: bool,
            recorded: u64,
            consumed: u64,
            rejected_visible_start: u64,
            rejected_geometry: u64,
        }

        #[derive(Serialize)]
        struct DebugRendererStatsView {
            page_count: usize,
            layout_page_limit: Option<usize>,
            visible_page_range: Option<(usize, usize)>,
            band_redraw: BandRedrawStatsView,
            span_seek: SpanSeekStatsView,
            text_measure_cache: CacheStatsView,
            paragraph_wrap_cache: ParagraphWrapCacheStatsView,
            table_autofit_cache: TableAutofitCacheStatsView,
            paragraph_layout_cache: ParagraphLayoutStatsView,
            paragraph_render_cache: ParagraphRenderStatsView,
            word_oracle: WordOracleStatsView,
            east_asian: EastAsianStatsView,
            context_rebuild: ContextRebuildStatsView,
            window_seek: WindowSeekStatsView,
            layout_extension_seek: LayoutExtensionSeekStatsView,
            glyph_batch: GlyphBatchStatsView,
            last_render_body_element_count: u64,
            last_render_prefix_skips: u64,
        }

        let renderer = self.renderer.borrow();
        let visible_page_range = renderer
            .visible_page_range()
            .as_ref()
            .map(|r| (*r.start(), *r.end()));

        let text = renderer.text_measure_cache_stats();
        let wrap = renderer.paragraph_wrap_cache_stats();
        let table_autofit = renderer.table_autofit_cache_stats();
        let layout = renderer.paragraph_layout_cache_stats();
        let render = renderer.paragraph_render_cache_stats();
        let (sized_advances, text_advances, paragraph_advances, paragraph_counts) =
            renderer.word_oracle_advance_stats();
        let (use_east_asian_layout, character_spacing_control) =
            renderer.east_asian_debug_settings();
        let (
            context_rebuilds,
            context_rebuild_skips,
            canvas_inflates,
            pages_seek_skipped,
            context_creations,
            context_creation_deferrals,
        ) = renderer.context_rebuild_stats();
        // `page_slot_recycle_stats` also reports `context_creations`; the
        // authoritative copy above comes from `context_rebuild_stats`.
        let (
            _context_creations,
            page_slot_recycles,
            page_slot_reuses,
            page_slot_pool_len,
            hot_backing_recycles,
            hot_backing_reuses,
            hot_backing_pool_len,
            hot_backing_pool_bytes,
        ) = renderer.page_slot_recycle_stats();
        let (window_seek_exact_hits, window_seek_fallback_hits, window_seek_misses) =
            renderer.window_seek_stats();
        let (layout_extension_seek_hits, layout_extension_seek_misses) =
            renderer.layout_extension_seek_stats();
        let (
            layout_extension_seek_last_resume_page,
            layout_extension_seek_last_resume_body_index,
            layout_extension_seek_last_invalidated_checkpoints,
            layout_extension_seek_invalidated_checkpoints,
        ) = renderer.layout_extension_seek_frontier_stats();
        let (glyph_batch_runs, glyph_batch_glyphs, glyph_fallback_runs, glyph_fallback_glyphs) =
            renderer.glyph_batch_stats();
        let (band_eligible, band_taken, band_fallbacks) = renderer.band_redraw_stats();
        let (
            span_seek_recorded,
            span_seek_consumed,
            span_seek_rejected_visible_start,
            span_seek_rejected_geometry,
        ) = renderer.span_seek_stats();
        let (
            replay_batch_runs,
            replay_batch_glyphs,
            replay_fallback_runs,
            replay_fallback_glyphs,
            replay_text_runs,
            replay_text_glyphs,
        ) = crate::draw_command::executor::replay_glyph_batch_stats();

        let view = DebugRendererStatsView {
            page_count: renderer.page_count(),
            layout_page_limit: renderer.layout_page_limit(),
            visible_page_range,
            band_redraw: BandRedrawStatsView {
                enabled: renderer.band_redraw_enabled(),
                eligible: band_eligible,
                taken: band_taken,
                fallback: band_fallbacks
                    .into_iter()
                    .map(|(name, count)| (name.to_string(), count))
                    .collect(),
                skipped_pages: renderer.band_redraw_skipped_pages(),
            },
            span_seek: SpanSeekStatsView {
                enabled: renderer.span_seek_enabled(),
                recorded: span_seek_recorded,
                consumed: span_seek_consumed,
                rejected_visible_start: span_seek_rejected_visible_start,
                rejected_geometry: span_seek_rejected_geometry,
            },
            text_measure_cache: CacheStatsView {
                hits: text.hits,
                misses: text.misses,
                size: text.size,
                capacity: text.capacity,
            },
            paragraph_wrap_cache: ParagraphWrapCacheStatsView {
                hits: wrap.hits,
                misses: wrap.misses,
                table_hits: wrap.table_hits,
                table_misses: wrap.table_misses,
                size: wrap.size,
                capacity: wrap.capacity,
            },
            table_autofit_cache: TableAutofitCacheStatsView {
                enabled: renderer.table_autofit_cache_enabled(),
                hits: table_autofit.hits,
                misses: table_autofit.misses,
                invalidations: table_autofit.invalidations,
                saved_paragraph_measures: table_autofit.saved_paragraph_measures,
                size: table_autofit.size,
                capacity: table_autofit.capacity,
            },
            paragraph_layout_cache: ParagraphLayoutStatsView {
                hits: layout.hits,
                misses: layout.misses,
                skipped_paragraphs: layout.skipped_paragraphs,
                cached_entries: layout.cached_entries,
                total_capacity: layout.total_capacity,
            },
            paragraph_render_cache: ParagraphRenderStatsView {
                hits: render.hits,
                misses: render.misses,
                cached_entries: render.cached_entries,
                total_capacity: render.total_capacity,
            },
            word_oracle: WordOracleStatsView {
                sized_advances,
                text_advances,
                paragraph_advances,
                paragraph_counts,
                report_x_offsets: renderer.word_oracle_report_x_offset_count(),
            },
            east_asian: EastAsianStatsView {
                use_east_asian_layout,
                character_spacing_control,
            },
            context_rebuild: ContextRebuildStatsView {
                context_rebuilds,
                context_rebuild_skips,
                canvas_inflates,
                pages_seek_skipped,
                context_creations,
                context_creation_deferrals,
                page_slot_recycles,
                page_slot_reuses,
                page_slot_pool_len,
                hot_backing_recycles,
                hot_backing_reuses,
                hot_backing_pool_len,
                hot_backing_pool_bytes,
            },
            window_seek: WindowSeekStatsView {
                exact_hits: window_seek_exact_hits,
                fallback_hits: window_seek_fallback_hits,
                misses: window_seek_misses,
            },
            layout_extension_seek: LayoutExtensionSeekStatsView {
                enabled: renderer.layout_extension_seek_enabled(),
                hits: layout_extension_seek_hits,
                misses: layout_extension_seek_misses,
                last_miss: renderer.layout_extension_seek_last_miss().to_string(),
                last_resume_page: layout_extension_seek_last_resume_page,
                last_resume_body_index: layout_extension_seek_last_resume_body_index,
                last_visited_body_elements: renderer.last_render_body_element_count(),
                last_invalidated_checkpoints: layout_extension_seek_last_invalidated_checkpoints,
                invalidated_checkpoints: layout_extension_seek_invalidated_checkpoints,
            },
            glyph_batch: GlyphBatchStatsView {
                batch_runs: glyph_batch_runs,
                batch_glyphs: glyph_batch_glyphs,
                fallback_runs: glyph_fallback_runs,
                fallback_glyphs: glyph_fallback_glyphs,
                replay_batch_runs,
                replay_batch_glyphs,
                replay_fallback_runs,
                replay_fallback_glyphs,
                replay_text_runs,
                replay_text_glyphs,
            },
            last_render_body_element_count: renderer.last_render_body_element_count(),
            last_render_prefix_skips: renderer.last_render_prefix_skips(),
        };

        // Use the JSON-compatible serializer so map fields (e.g.
        // `band_redraw.fallback`) become plain JS objects rather than `Map`
        // instances, which `JSON.stringify` / `Object.keys` cannot read.
        view.serialize(&serde_wasm_bindgen::Serializer::json_compatible())
            .unwrap_or(JsValue::NULL)
    }

    /// Enables or disables the persistent table intrinsic/autofit cache. This
    /// is a diagnostics escape hatch used for cache-on/off parity probes.
    #[wasm_bindgen(js_name = setTableAutofitCacheEnabled)]
    pub fn set_table_autofit_cache_enabled(&self, enabled: bool) {
        self.renderer
            .borrow_mut()
            .set_table_autofit_cache_enabled(enabled);
    }

    /// Returns whether the last render pass completed the full document layout.
    ///
    /// In lazy layout mode (layout page limit set), this can be false until the host
    /// extends layout to completion via `ensureLayoutThroughPage`.
    #[wasm_bindgen(js_name = lastLayoutComplete)]
    pub fn last_layout_complete(&self) -> bool {
        self.renderer.borrow().last_layout_complete()
    }

    /// Returns the dimensions for a specific page as { width, height } or null.
    #[wasm_bindgen]
    pub fn page_dimensions(&self, page_index: usize) -> JsValue {
        let renderer = self.renderer.borrow();
        match renderer.page_dimensions(page_index) {
            Some((width, height)) => {
                to_value(&CanvasSize { width, height }).unwrap_or(JsValue::NULL)
            }
            None => JsValue::NULL,
        }
    }

    /// Completes image decoding and drawing before a lazy page is snapshotted.
    #[wasm_bindgen(js_name = waitForPendingImages)]
    pub async fn wait_for_pending_page_images(&self) -> Result<(), JsValue> {
        self.wait_for_pending_images().await
    }

    /// Creates an ImageBitmap for a specific page (non-destructive).
    /// Returns a Promise that resolves to the ImageBitmap.
    #[wasm_bindgen]
    pub fn create_page_bitmap(&self, page_index: usize) -> Result<js_sys::Promise, JsValue> {
        let renderer = self.renderer.borrow();
        renderer.create_page_bitmap(page_index)
    }

    /// Transfers an ImageBitmap for a specific page using `transferToImageBitmap()`.
    /// Synchronous and ~10x faster than `create_page_bitmap`, but **clears the
    /// canvas**. The page must be re-rendered before another bitmap can be taken.
    #[wasm_bindgen]
    pub fn transfer_page_bitmap(&self, page_index: usize) -> Result<ImageBitmap, JsValue> {
        let mut renderer = self.renderer.borrow_mut();
        renderer.transfer_page_bitmap(page_index)
    }

    /// Reports whether the given 0-based page currently holds a full-resolution
    /// backing with valid rendered pixels. Used by the bitmap pipeline to decide
    /// whether a page must be re-rendered before snapshotting.
    #[wasm_bindgen(js_name = pageHasRenderedPixels)]
    pub fn page_has_rendered_pixels(&self, page_index: usize) -> bool {
        let renderer = self.renderer.borrow();
        renderer.page_has_rendered_pixels(page_index)
    }

    /// Reports whether the given 0-based page is inside the renderer's keep
    /// window (its backing is kept at full resolution). In-window pages can be
    /// fetched non-destructively (`create_page_bitmap`) at zero extra peak
    /// memory, letting refetches of the caret/visible pages skip the
    /// transfer+shrink+re-inflate+re-render churn.
    #[wasm_bindgen(js_name = pageInKeepWindow)]
    pub fn page_in_keep_window(&self, page_index: usize) -> bool {
        let renderer = self.renderer.borrow();
        renderer.page_in_keep_window(page_index)
    }

    #[wasm_bindgen]
    pub fn clear_cached_pages(&self) {
        self.state.borrow_mut().paged_images.clear();
    }

    #[wasm_bindgen]
    pub fn caret_rect(&self) -> JsValue {
        if self.state.borrow().preview_mode {
            return JsValue::NULL;
        }

        let renderer = self.renderer.borrow();
        match renderer.caret_metrics() {
            Some(metrics) => to_value(&metrics).unwrap_or(JsValue::NULL),
            None => JsValue::NULL,
        }
    }

    /// Returns the *effective* formatting at the current caret position as a JS
    /// object containing both run and paragraph properties.  The returned shape
    /// is `{ run: RunProperties, paragraph: ParagraphProperties }`.
    ///
    /// Run properties are resolved by layering:
    ///   1. Paragraph-style run properties
    ///   2. Character-style run properties
    ///   3. Direct run properties
    ///   4. Pending inline-formatting overrides (collapsed selections)
    ///
    /// Paragraph properties are resolved by layering:
    ///   1. Paragraph-style paragraph properties
    ///   2. Direct paragraph properties
    #[wasm_bindgen(js_name = runPropertiesAtCaret)]
    pub fn run_properties_at_caret(&self) -> JsValue {
        if !self.body_commands_allowed() {
            // The ribbon reflects the header/footer line under the caret.
            let Some(state) = self.caret_hf_run_state() else {
                return JsValue::NULL;
            };
            let run = Object::new();
            let set = |key: &str, value: JsValue| {
                let _ = Reflect::set(&run, &JsValue::from_str(key), &value);
            };
            set("bold", JsValue::from_bool(state.all_bold));
            set("italic", JsValue::from_bool(state.all_italic));
            set("underline", JsValue::from_bool(state.all_underline));
            if let Some(kind) = state.underline_kind.as_deref() {
                set("underline_kind", JsValue::from_str(kind));
            }
            set(
                "strike",
                JsValue::from_bool(state.all_strike && !state.all_double_strike),
            );
            set("double_strike", JsValue::from_bool(state.all_double_strike));
            set(
                "baseline",
                JsValue::from_str(if state.all_superscript {
                    "superscript"
                } else if state.all_subscript {
                    "subscript"
                } else {
                    "baseline"
                }),
            );
            if let Some(size) = state.font_size {
                set("font_size", JsValue::from_f64(f64::from(size)));
            }
            if let Some(family) = state.font_family.as_deref() {
                set("font_family", JsValue::from_str(family));
            }
            if let Some(color) = state.color.as_deref() {
                set("color", JsValue::from_str(color));
            }
            if let Some(highlight) = state.highlight.as_deref() {
                set("highlight", JsValue::from_str(highlight));
            }
            let result = Object::new();
            let _ = Reflect::set(&result, &JsValue::from_str("run"), &run);
            let view = self.state.borrow();
            let direct = view.hf_edit_cursor.and_then(|cursor| {
                let (role, variant) = Self::hf_role_variant(view.editing_mode, view.hf_variant)?;
                view.word_doc
                    .as_ref()?
                    .hf_paragraph_properties(role, variant, cursor.line_index)
            });
            let mut paragraph = ParagraphProperties::default();
            if let Some(styles) = view
                .word_doc
                .as_ref()
                .and_then(|doc| doc.styles_part.as_ref())
            {
                let style_id = direct
                    .as_ref()
                    .and_then(|props| props.style.as_deref())
                    .or_else(|| styles.default_paragraph_style_id());
                if let Some(properties) =
                    style_id.and_then(|id| styles.resolve_paragraph_properties(id))
                {
                    merge_paragraph_properties_override(&mut paragraph, &properties);
                }
            }
            if let Some(direct) = direct.as_ref() {
                merge_paragraph_properties_override(&mut paragraph, direct);
            }
            // The cascade merges formatting, not the authored style identity.
            paragraph.style = direct
                .as_ref()
                .and_then(|properties| properties.style.clone());
            paragraph.alignment.get_or_insert_with(|| Rc::from("left"));
            let character_unit =
                view.word_doc
                    .as_ref()
                    .zip(view.hf_edit_cursor)
                    .and_then(|(doc, cursor)| {
                        let (role, variant) =
                            Self::hf_role_variant(view.editing_mode, view.hf_variant)?;
                        let story_paragraph =
                            doc.hf_ruler_paragraph(role, variant, cursor.line_index)?;
                        let document = doc.document.as_ref()?;
                        let section_index = self
                            .renderer
                            .borrow()
                            .section_index_for_zero_based_page(cursor.page_index);
                        let section = document
                            .sections
                            .get(section_index)
                            .and_then(|section| section.properties.as_ref());
                        Some(
                            jade_docx_layout::text_extract::ruler_character_unit_px(
                                document,
                                section,
                                &story_paragraph,
                                doc.styles_part.as_ref(),
                            ) * 0.75,
                        )
                    });
            if let Some(unit) = character_unit {
                let _ = Reflect::set(
                    &result,
                    &JsValue::from_str("rulerIndent"),
                    &resolved_ruler_indent(&paragraph, unit),
                );
            }
            let paragraph = to_value(&paragraph).unwrap_or(JsValue::NULL);
            let _ = Reflect::set(&result, &JsValue::from_str("paragraph"), &paragraph);
            drop(view);
            let _ = Reflect::set(
                &result,
                &JsValue::from_str("editingMode"),
                &JsValue::from_str(Self::hf_editing_mode_name(self.state.borrow().editing_mode)),
            );
            return result.into();
        }
        let state = self.state.borrow();
        if state.preview_mode {
            return JsValue::NULL;
        }

        // A freshly opened document has no caret yet. Word puts it at the
        // start of the document, so the ribbon shows that position's
        // formatting instead of hard-coded defaults. The payload then omits
        // `selection`, since nothing is selected.
        let (selection, implicit_caret) = match state.selection {
            Some(sel) => (sel, false),
            None => (SelectionRange::collapsed(TextPosition::new(0, 0)), true),
        };

        let word_doc = match state.word_doc.as_ref() {
            Some(doc) => doc,
            None => return JsValue::NULL,
        };

        // A non-collapsed selection ending exactly at a run boundary must
        // report the selected run's formatting, not the following run's.
        // Word's ribbon reflects the selection itself regardless of drag
        // direction, so use the normalized start as the formatting probe.
        let position = if selection.is_collapsed() {
            selection.focus
        } else {
            selection.normalized().0
        };
        let position = word_doc.rendered_to_model_text_position(position);
        let paragraph = match word_doc.paragraph(position.paragraph) {
            Some(p) => p,
            None => return JsValue::NULL,
        };

        // ── Run properties ──────────────────────────────────────────────

        // Walk runs to find the one containing `position.offset`.
        let mut run_start: usize = 0;
        let mut target_props: Option<&RunProperties> = None;
        for run in &paragraph.runs {
            let run_end = run_start + run.text.len();
            if position.offset < run_end || (position.offset == run_end && run_end == run_start) {
                target_props = run.properties.as_ref();
                break;
            }
            if position.offset == run_end {
                target_props = run.properties.as_ref();
            }
            run_start = run_end;
        }

        let para_style_id = paragraph
            .properties
            .as_ref()
            .and_then(|p| p.style.as_deref());

        // Layer 0: document defaults (`w:docDefaults/w:rPrDefault`). Without
        // this, body text with no explicit style reports an empty font
        // family/size and the ribbon readout goes stale.
        let mut effective_run = RunProperties::default();
        if let Some(styles) = word_doc.styles_part.as_ref() {
            if let Some(defaults) = styles.doc_default_run_properties() {
                merge_run_properties_override(&mut effective_run, defaults);
            }
        }

        // Layer 1: paragraph style run properties. Paragraphs without an
        // explicit `w:pStyle` use the document's default paragraph style,
        // matching the renderer's cascade.
        if let Some(styles) = word_doc.styles_part.as_ref() {
            let style_id = para_style_id.or_else(|| styles.default_paragraph_style_id());
            if let Some(sid) = style_id {
                if let Some(style_props) = styles.resolve_run_properties(sid) {
                    merge_run_properties_override(&mut effective_run, &style_props);
                }
            }
        }

        // Layer 2: character style run properties
        if let Some(styles) = word_doc.styles_part.as_ref() {
            if let Some(char_style_id) = target_props.and_then(|p| p.style.as_deref()) {
                if let Some(char_props) = styles.resolve_character_run_properties(char_style_id) {
                    merge_run_properties_override(&mut effective_run, &char_props);
                }
            }
        }

        // Layer 3: direct run properties
        if let Some(direct) = target_props {
            merge_run_properties_override(&mut effective_run, direct);
        }

        // Layer 4: pending inline formatting — only while the caret still
        // sits where the pending formatting was armed.
        if state.inline_formatting.is_armed_for(&selection) {
            let inline = &state.inline_formatting;
            let overrides = RunProperties {
                bold: inline.bold,
                italic: inline.italic,
                underline: inline.underline,
                strike: inline.strike,
                baseline: inline.baseline.as_deref().map(Rc::from),
                font_size: inline.font_size,
                font_family: inline.font_family.as_deref().map(Rc::from),
                color: inline.color.as_deref().map(Rc::from),
                highlight: inline.highlight.as_deref().map(Rc::from),
                ..RunProperties::default()
            };
            merge_run_properties_override(&mut effective_run, &overrides);
        }

        // ── Paragraph properties ────────────────────────────────────────

        // Layer 1: paragraph style paragraph properties
        let mut effective_para = ParagraphProperties::default();
        if let Some(styles) = word_doc.styles_part.as_ref() {
            // Paragraphs without an explicit `w:pStyle` use the document's
            // default paragraph style, matching the renderer's cascade.
            let style_id = para_style_id.or_else(|| styles.default_paragraph_style_id());
            if let Some(sid) = style_id {
                if let Some(style_para) = styles.resolve_paragraph_properties(sid) {
                    merge_paragraph_properties_override(&mut effective_para, &style_para);
                }
            }
        }

        // Layer 2: direct paragraph properties
        if let Some(direct) = paragraph.properties.as_ref() {
            merge_paragraph_properties_override(&mut effective_para, direct);
        }

        // Style resolution merges formatting fields, not the authored pStyle
        // identity. Keep it for galleries and selection controls.
        effective_para.style = paragraph.properties.as_ref().and_then(|p| p.style.clone());

        // ── Resolve numbering format ────────────────────────────────────
        let numbering_format: Option<String> =
            effective_para.numbering.as_ref().and_then(|numbering| {
                let num_id = numbering.id.as_deref()?;
                let level = numbering.level.unwrap_or(0);
                word_doc
                    .numbering_part
                    .as_ref()
                    .and_then(|np| np.get_level_info(num_id, level))
                    .and_then(|level| level.num_fmt.clone())
            });

        // ── Mixed-selection summary ─────────────────────────────────────
        // Word's ribbon reflects the whole selection: toggle buttons read
        // pressed only when ALL selected text carries the flag, and the
        // font/size/color boxes blank out when values are mixed.
        let selection_summary = Self::selection_run_summary(word_doc, &selection);
        if let Some(summary) = selection_summary.as_ref() {
            effective_run.bold = Some(summary.all_bold);
            effective_run.italic = Some(summary.all_italic);
            effective_run.underline = Some(summary.all_underline);
            effective_run.strike = Some(summary.all_strike);
            effective_run.baseline = summary
                .baseline
                .uniform()
                .map(|value| Rc::from(value.as_str()));
            effective_run.font_size = if summary.font_size.is_mixed() {
                None
            } else {
                effective_run.font_size
            };
            effective_run.font_family = if summary.font_family.is_mixed() {
                None
            } else {
                effective_run.font_family.take()
            };
            effective_run.color = if summary.color.is_mixed() {
                None
            } else {
                effective_run.color.take()
            };
            effective_run.highlight = if summary.highlight.is_mixed() {
                None
            } else {
                effective_run.highlight.take()
            };
        }

        // ── Build result object ─────────────────────────────────────────
        let result = Object::new();
        let run_js = to_value(&effective_run).unwrap_or(JsValue::NULL);
        let para_js = to_value(&effective_para).unwrap_or(JsValue::NULL);
        // Ruler edits use physical point values even when OOXML authored the
        // indent in character units. Resolve through the layout precedence.
        if let Some(document) = word_doc.document.as_ref() {
            let section = word_doc
                .section_index_for_paragraph(selection.focus.paragraph)
                .and_then(|index| document.sections.get(index))
                .and_then(|section| section.properties.as_ref());
            let unit = jade_docx_layout::text_extract::ruler_character_unit_px(
                document,
                section,
                paragraph,
                word_doc.styles_part.as_ref(),
            ) * 0.75;
            let _ = Reflect::set(
                &result,
                &JsValue::from_str("rulerIndent"),
                &resolved_ruler_indent(&effective_para, unit),
            );
        }
        let _ = Reflect::set(&result, &JsValue::from_str("run"), &run_js);
        let _ = Reflect::set(&result, &JsValue::from_str("paragraph"), &para_js);
        if let Some(summary) = selection_summary.as_ref() {
            let mixed = Object::new();
            let _ = Reflect::set(
                &mixed,
                &JsValue::from_str("font_size"),
                &JsValue::from_bool(summary.font_size.is_mixed()),
            );
            let _ = Reflect::set(
                &mixed,
                &JsValue::from_str("font_family"),
                &JsValue::from_bool(summary.font_family.is_mixed()),
            );
            let _ = Reflect::set(
                &mixed,
                &JsValue::from_str("color"),
                &JsValue::from_bool(summary.color.is_mixed()),
            );
            let _ = Reflect::set(
                &mixed,
                &JsValue::from_str("highlight"),
                &JsValue::from_bool(summary.highlight.is_mixed()),
            );
            let _ = Reflect::set(&result, &JsValue::from_str("mixed"), &mixed);
        }
        if !implicit_caret {
            if let Ok(selection_js) = to_value(&selection) {
                let _ = Reflect::set(&result, &JsValue::from_str("selection"), &selection_js);
            }
        }
        if let Some(fmt) = numbering_format {
            let _ = Reflect::set(
                &result,
                &JsValue::from_str("numberingFormat"),
                &JsValue::from_str(&fmt),
            );
        }
        result.into()
    }

    /// Registers a JS-backed decoration plugin.
    ///
    /// The callback receives one argument, the segment's UTF-8 text, and
    /// must return either a JSON string shaped like
    /// `Vec<DecoratedRange>` or `null`/`undefined` to mean "no
    /// decorations". See `crates/jade-preview-plugin-api` for the wire
    /// schema, and `crates/jade-plugins/circle-mark-plugin` for a reference
    /// guest WASM module the callback can delegate to.
    ///
    /// Registering a plugin does NOT trigger a repaint on its own — the
    /// next render pass picks it up. Call
    /// [`Self::set_plugins_enabled`] + a refresh to force immediate
    /// visibility, or let the next edit paint invalidation do it.
    ///
    /// Multiple plugins may share a name; [`Self::unregister_plugin`]
    /// removes every match.
    #[wasm_bindgen(js_name = registerPlugin)]
    pub fn register_plugin(
        &self,
        name: String,
        decorate_fn: js_sys::Function,
    ) -> Result<(), JsValue> {
        if name.is_empty() {
            return Err(JsValue::from_str("plugin name must not be empty"));
        }
        let plugin = crate::plugins::JsCallbackPlugin::new(name, decorate_fn);
        self.renderer.borrow_mut().register_plugin(Box::new(plugin));
        Ok(())
    }

    /// Removes every registered plugin whose name equals `name`. Returns
    /// `true` iff at least one plugin was removed.
    #[wasm_bindgen(js_name = unregisterPlugin)]
    pub fn unregister_plugin(&self, name: String) -> bool {
        self.renderer.borrow_mut().unregister_plugin(&name)
    }

    /// Enables or disables all registered decoration plugins wholesale
    /// without unregistering them. Useful for a "preview / production
    /// mode" toggle that mutes overlays temporarily.
    #[wasm_bindgen(js_name = setPluginsEnabled)]
    pub fn set_plugins_enabled(&self, enabled: bool) {
        self.renderer.borrow_mut().set_plugins_enabled(enabled);
    }

    /// Returns whether plugin invocations are currently enabled.
    #[wasm_bindgen(js_name = pluginsEnabled)]
    pub fn plugins_enabled(&self) -> bool {
        self.renderer.borrow().decoration_registry().is_enabled()
    }

    /// Reconfigures ephemeral spelling and repaints without changing document history.
    #[wasm_bindgen(js_name = configureSpelling)]
    pub fn configure_spelling(&self, enabled: bool, generation: u32) -> Result<JsValue, JsValue> {
        self.rerender_after_display_change(|renderer| {
            renderer
                .spelling_cache
                .borrow_mut()
                .configure(enabled, generation);
            renderer.clear_paragraph_render_cache();
        })
    }

    #[wasm_bindgen(js_name = takePendingSpelling)]
    pub fn take_pending_spelling(&self) -> Result<JsValue, JsValue> {
        let renderer = self.renderer.borrow();
        let mut cache = renderer.spelling_cache.borrow_mut();
        serde_json::json!({
            "generation": cache.generation, "requests": cache.take_pending(),
        })
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|err| JsValue::from_str(&err.to_string()))
    }

    #[wasm_bindgen(js_name = applySpellingResults)]
    pub fn apply_spelling_results(&self, generation: u32, json: &str) -> Result<JsValue, JsValue> {
        #[derive(serde::Deserialize)]
        struct Entry {
            key: String,
            matches: Vec<(usize, usize)>,
        }
        let entries: Vec<Entry> = serde_json::from_str(json)
            .map_err(|err| JsValue::from_str(&format!("invalid spelling results: {err}")))?;
        let mut changed_texts = Vec::new();
        {
            let renderer = self.renderer.borrow();
            let mut cache = renderer.spelling_cache.borrow_mut();
            for entry in entries {
                if let Some(text) = cache.apply(generation, &entry.key, &entry.matches) {
                    changed_texts.push(text);
                }
            }
        }
        if changed_texts.is_empty() {
            return Ok(JsValue::NULL);
        }
        self.rerender_decoration_texts(&changed_texts)
    }

    /// Drains paragraph cache misses for the embedder's decoration plugins.
    #[wasm_bindgen(js_name = takePendingDecorations)]
    pub fn take_pending_decorations(&self) -> Result<JsValue, JsValue> {
        let texts = self.renderer.borrow().take_pending_decorations();
        let arr = Array::new();
        for text in texts {
            let obj = js_sys::Object::new();
            Reflect::set(&obj, &JsValue::from_str("text"), &JsValue::from_str(&text))?;
            arr.push(&obj);
        }
        Ok(arr.into())
    }

    /// Returns whether any paragraph text is currently awaiting a plugin
    /// run. The async orchestrator polls this after responses are posted
    /// to decide whether to schedule a `takePendingDecorations` sweep.
    #[wasm_bindgen(js_name = hasPendingDecorations)]
    pub fn has_pending_decorations(&self) -> bool {
        self.renderer.borrow().has_pending_decorations()
    }

    /// Feeds plugin results back into the decoration cache and, if any
    /// entries changed, invalidates affected paragraph render caches and
    /// re-renders so the decorations become visible.
    ///
    /// `json` is a JSON string of the form
    /// `[{ "text": "<paragraph text>", "rangesJson": "[{...}]" }, ...]`
    /// where `rangesJson` is a JSON-encoded `Vec<DecoratedRange>`
    /// matching the wire schema from `jade-preview-plugin-api`.
    ///
    /// When at least one cache entry changed, this returns a JS object
    /// of the form `{ pages, dirtyRegions }` mirroring the payload shape
    /// produced by edit commands (`apply_bold`, `insert_text`, etc.).
    /// The caller forwards that payload to the main thread's bitmap
    /// applicator. When nothing changed (all results were byte-identical
    /// to what was already cached) this returns `null`.
    /// Throws away every cached decoration result and re-renders.
    ///
    /// Results are cached per paragraph text, on the assumption that the same
    /// text always decorates the same way. Reconfiguring a plugin breaks that
    /// assumption, so the cache has to go.
    ///
    /// Clearing alone is not enough. Plugins are consulted from the draw pass:
    /// a paragraph with no cached result is queued as pending, and the
    /// embedder drains that queue and pushes answers back. With nothing
    /// redrawing, nothing is ever queued and the plugin is never asked. So
    /// this re-renders and hands back the pages, the same way toggling
    /// revision display does.
    #[wasm_bindgen(js_name = clearDecorationCache)]
    pub fn clear_decoration_cache(&self) -> Result<JsValue, JsValue> {
        self.renderer.borrow().clear_decoration_cache();
        self.rerender_after_display_change(|_| {})
    }

    #[wasm_bindgen(js_name = applyDecorationResults)]
    pub fn apply_decoration_results(&self, json: &str) -> Result<JsValue, JsValue> {
        #[derive(serde::Deserialize)]
        struct ResultEntry {
            text: String,
            #[serde(rename = "rangesJson")]
            ranges_json: String,
        }
        let parsed: Vec<ResultEntry> = serde_json::from_str(json)
            .map_err(|err| JsValue::from_str(&format!("invalid results JSON: {err}")))?;

        let mut any_changed = false;
        let mut changed_texts: Vec<String> = Vec::new();
        {
            let renderer = self.renderer.borrow();
            for entry in parsed {
                let ranges: Vec<crate::plugins::DecoratedRange> =
                    if entry.ranges_json.is_empty() || entry.ranges_json == "[]" {
                        Vec::new()
                    } else {
                        serde_json::from_str(&entry.ranges_json).unwrap_or_default()
                    };
                let text_for_invalidation = entry.text.clone();
                if renderer.apply_decoration_result(entry.text, ranges) {
                    any_changed = true;
                    changed_texts.push(text_for_invalidation);
                }
            }
        }

        if !any_changed {
            return Ok(JsValue::NULL);
        }

        self.rerender_decoration_texts(&changed_texts)
    }

    fn rerender_decoration_texts(&self, changed_texts: &[String]) -> Result<JsValue, JsValue> {
        // Targeted invalidation + per-paragraph dirty rects.
        //
        // `invalidate_and_mark_dirty_rects_for_decoration_texts`:
        //   (a) wipes the DrawCommand cache for affected paragraphs so
        //       the next render re-runs their segment-draw loop,
        //   (b) pushes one dirty rect per affected paragraph (derived
        //       from the still-valid cached render geometry) to the
        //       bitmap dirty-region tracker.
        //
        // The bitmap update pipeline then sends partial page updates
        // instead of full-page bitmaps for every page — the dominant
        // wire cost when a decoration change affects a single
        // paragraph in a long document.
        //
        // Fall back to a full repaint only when:
        //   - the reverse index had no record for any changed text
        //     (e.g. `clearDecorationCache` ran between record and
        //     apply), OR
        //   - some targeted paragraph had no cached rect to derive
        //     from (it was never rendered, or the cache was cleared).
        // In either case the dominant cost (plugin JSON parsing, wasm
        // round-trip) has already been paid; conservatively repainting
        // every page keeps correctness guaranteed.
        let (snapshots, dirty_regions) = {
            let mut renderer_mut = self.renderer.borrow_mut();
            let outcome =
                renderer_mut.invalidate_and_mark_dirty_rects_for_decoration_texts(changed_texts);
            if outcome.invalidated_paragraphs == 0 {
                renderer_mut.clear_paragraph_render_cache();
                renderer_mut.mark_full_repaint();
            } else if !outcome.all_rects_resolved {
                renderer_mut.mark_full_repaint();
            }
            // Decoration results change pixels while the document hash stays
            // identical. Do not replay the entire unchanged body prefix: that
            // can skip nested text boxes and the endnote drawing pass.
            renderer_mut.skip_change_detection_once();
            renderer_mut.rerender_with_cached_state()?;
            self.capture_paged_images(&mut renderer_mut)?
        };

        if let Some(ref pages) = snapshots {
            self.store_paged_images(Some(pages.clone()));
        }

        let payload = js_sys::Object::new();
        if let Some(pages) = snapshots {
            let arr = Array::new();
            for page in pages {
                arr.push(&page);
            }
            Reflect::set(&payload, &JsValue::from_str("pages"), &arr)?;
        } else {
            Reflect::set(&payload, &JsValue::from_str("pages"), &Array::new())?;
        }
        Reflect::set(&payload, &JsValue::from_str("dirtyRegions"), &dirty_regions)?;
        Ok(payload.into())
    }

    #[wasm_bindgen]
    pub fn set_preview_mode(&self, preview: bool) {
        let mut state_mut = self.state.borrow_mut();
        if state_mut.preview_mode == preview {
            return;
        }
        state_mut.preview_mode = preview;
        state_mut.options.set_preview_mode(preview);
    }

    /// Switches the backing renderer between the interactive editor surface and
    /// a clean page-export surface. The original formatting-mark preference is
    /// retained so a PDF/PNG export cannot change the live editor configuration.
    #[wasm_bindgen(js_name = setPageExportMode)]
    pub fn set_page_export_mode(&self, enabled: bool, scale: Option<f64>) -> Result<(), JsValue> {
        // Change density before decoration state, so a rejected scale leaves
        // the interactive render options untouched.
        if enabled {
            if let Some(scale) = scale {
                if !scale.is_finite() || !(0.5..=4.0).contains(&scale) {
                    return Err(JsValue::from_str(
                        "Export raster scale must be between 0.5 and 4",
                    ));
                }
                let previous = self.renderer.borrow_mut().set_raster_scale(scale)?;
                self.state
                    .borrow_mut()
                    .page_export_original_raster_scale
                    .get_or_insert(previous);
            }
        } else if let Some(previous) = self.state.borrow().page_export_original_raster_scale {
            self.renderer.borrow_mut().set_raster_scale(previous)?;
        }
        if !enabled {
            self.state.borrow_mut().page_export_original_raster_scale = None;
        }
        let (show_formatting_marks, show_crop_marks) = {
            let mut state = self.state.borrow_mut();
            let CanvasDocumentState {
                options,
                page_export_original_formatting_marks,
                ..
            } = &mut *state;
            set_page_export_render_mode(options, page_export_original_formatting_marks, enabled)
        };
        let mut renderer = self.renderer.borrow_mut();
        renderer.set_page_export_render_options(show_formatting_marks, show_crop_marks);
        renderer.set_spelling_export_mode(enabled);
        Ok(())
    }

    /// Shows or hides spaces, paragraph marks, tabs, and other non-printing
    /// control marks without reloading the document or resetting history.
    #[wasm_bindgen(js_name = setShowFormattingMarks)]
    pub fn set_show_formatting_marks(&self, visible: bool) -> Result<JsValue, JsValue> {
        let should_rerender = {
            let mut state = self.state.borrow_mut();
            let current = state
                .page_export_original_formatting_marks
                .unwrap_or(state.options.show_formatting_marks);
            if current == visible {
                false
            } else {
                let export_active = state.page_export_original_formatting_marks.is_some();
                let CanvasDocumentState {
                    options,
                    page_export_original_formatting_marks,
                    ..
                } = &mut *state;
                set_formatting_marks_preference(
                    options,
                    page_export_original_formatting_marks,
                    visible,
                );
                !export_active
            }
        };

        if !should_rerender {
            return Ok(JsValue::NULL);
        }
        self.rerender_after_display_change(|renderer| {
            renderer.set_formatting_marks_flag(visible);
        })
    }

    /// Switches the live view between paged and pageless rendering on a newly
    /// transferred canvas without reloading the DOCX. The document model,
    /// current selection, and undo/redo executor stay in the existing view.
    ///
    /// With `lazy` (paged only), the view stays in lazy mode: the layout limit
    /// and visible window of the previous presentation carry over, only the
    /// visible pages are painted, and the result carries page dimensions with
    /// `lazy: true` instead of a bitmap for every page. The host then fetches
    /// pixels on demand exactly as after `loadDocxLazyUntil`, which is what
    /// keeps pages outside the viewport correct after later edits reflow onto
    /// them. An eager result leaves those pages without any fetch path once
    /// the host narrows the visible range.
    #[wasm_bindgen(js_name = switchRenderer)]
    pub fn switch_renderer(
        &self,
        canvas: OffscreenCanvas,
        pageless: bool,
        device_pixel_ratio: f64,
        dark_mode: bool,
        lazy: Option<bool>,
    ) -> Result<JsValue, JsValue> {
        let ratio = if device_pixel_ratio.is_finite() && device_pixel_ratio > 0.0 {
            device_pixel_ratio
        } else {
            1.0
        };
        let lazy = lazy.unwrap_or(false) && !pageless;
        let host = OffscreenRenderHost::new(canvas, ratio)?;
        let was_lazy = {
            let mut state = self.state.borrow_mut();
            let was_lazy = state.lazy_mode;
            state.options.set_pageless(pageless);
            state.options.set_dark_mode(dark_mode);
            state.lazy_mode = lazy;
            state.paged_images.clear();
            was_lazy
        };
        {
            let mut renderer = self.renderer.borrow_mut();
            let previous_limit = renderer.layout_page_limit();
            let previous_visible = renderer.visible_page_range();
            renderer.replace_render_host(Box::new(host), pageless)?;
            renderer.set_band_redraw_paints_band_page_only(lazy);
            if lazy {
                // A lazy session keeps laying out on demand; an eager one was
                // already complete, so it keeps no limit.
                let limit = if was_lazy { previous_limit } else { None };
                renderer.set_layout_page_limit(limit);
                let visible = previous_visible.unwrap_or_else(|| {
                    1..=limit
                        .unwrap_or(SWITCH_RENDERER_LAZY_BOOTSTRAP_PAGES)
                        .clamp(1, SWITCH_RENDERER_LAZY_BOOTSTRAP_PAGES)
                });
                renderer.set_visible_page_range(Some(visible));
            }
        }

        self.rerender_document()?;
        if lazy {
            return self.switch_renderer_lazy_result();
        }
        let (snapshots, dirty_regions, metadata, caret_metrics) = {
            let mut renderer = self.renderer.borrow_mut();
            let metadata = renderer.table_resize_metadata();
            let caret_metrics = renderer.caret_metrics();
            let (snapshots, dirty_regions) = self.capture_paged_images(&mut renderer)?;
            (snapshots, dirty_regions, metadata, caret_metrics)
        };
        self.store_paged_images(snapshots);
        self.state.borrow_mut().table_resize = metadata;

        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &self.canvas_size()?)?;
        Reflect::set(&result, &JsValue::from_str("pages"), &self.paged_images())?;
        Reflect::set(
            &result,
            &JsValue::from_str("caret"),
            &if self.state.borrow().preview_mode {
                JsValue::NULL
            } else {
                caret_metrics
                    .as_ref()
                    .and_then(|metrics| to_value(metrics).ok())
                    .unwrap_or(JsValue::NULL)
            },
        )?;
        Reflect::set(&result, &JsValue::from_str("dirtyRegions"), &dirty_regions)?;
        Self::write_history_flags(&result, self.history_flags())?;
        self.write_object_selection(&result)?;
        let selection = self.renderer.borrow().selection_range();
        {
            let renderer = self.renderer.borrow();
            Self::write_selection_rects(&result, &renderer, selection)?;
        }
        Ok(result.into())
    }

    fn switch_renderer_lazy_result(&self) -> Result<JsValue, JsValue> {
        let (page_dimensions, page_count, complete, metadata, caret_metrics) = {
            let renderer = self.renderer.borrow();
            // Pixels are fetched per page at the new layout generation, so
            // dirty regions of this pass carry no information for the host.
            renderer.clear_dirty_regions();
            let page_count = renderer.page_count();
            let page_dimensions: Vec<CanvasSize> = (0..page_count)
                .filter_map(|i| {
                    renderer.page_dimensions(i).map(|(w, h)| CanvasSize {
                        width: w,
                        height: h,
                    })
                })
                .collect();
            (
                page_dimensions,
                page_count,
                renderer.last_layout_complete(),
                renderer.table_resize_metadata(),
                renderer.caret_metrics(),
            )
        };
        self.state.borrow_mut().table_resize = metadata;

        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &self.canvas_size()?)?;
        Reflect::set(&result, &JsValue::from_str("lazy"), &JsValue::TRUE)?;
        Reflect::set(
            &result,
            &JsValue::from_str("pageCount"),
            &JsValue::from_f64(page_count as f64),
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("complete"),
            &JsValue::from_bool(complete),
        )?;
        let pages_array = Array::new();
        for dim in &page_dimensions {
            let page_obj = Object::new();
            Reflect::set(
                &page_obj,
                &JsValue::from_str("width"),
                &JsValue::from_f64(dim.width),
            )?;
            Reflect::set(
                &page_obj,
                &JsValue::from_str("height"),
                &JsValue::from_f64(dim.height),
            )?;
            pages_array.push(&page_obj);
        }
        Reflect::set(&result, &JsValue::from_str("pages"), &pages_array)?;
        Reflect::set(
            &result,
            &JsValue::from_str("caret"),
            &if self.state.borrow().preview_mode {
                JsValue::NULL
            } else {
                caret_metrics
                    .as_ref()
                    .and_then(|metrics| to_value(metrics).ok())
                    .unwrap_or(JsValue::NULL)
            },
        )?;
        Self::write_history_flags(&result, self.history_flags())?;
        self.write_object_selection(&result)?;
        let selection = self.renderer.borrow().selection_range();
        {
            let renderer = self.renderer.borrow();
            Self::write_selection_rects(&result, &renderer, selection)?;
        }
        Ok(result.into())
    }

    #[wasm_bindgen(getter = renderChanges)]
    pub fn render_changes(&self) -> bool {
        self.state.borrow().options.render_changes
    }

    #[wasm_bindgen(js_name = setRenderChanges)]
    /// Toggles the `render_changes` option and re-renders the cached document to
    /// avoid a full reload while keeping undo/redo state intact.
    pub fn set_render_changes(&self, value: bool) -> Result<JsValue, JsValue> {
        {
            let mut state_mut = self.state.borrow_mut();
            state_mut.options.revision_display_explicit = true;
            if state_mut.options.render_changes == value && !state_mut.options.render_original {
                return Ok(JsValue::NULL);
            }
            state_mut.options.render_changes = value;
            state_mut.options.render_original = false;
        }

        self.rerender_after_display_change(|renderer| {
            renderer.set_revision_display_flags(value, false);
        })
    }

    #[wasm_bindgen(js_name = setRevisionDisplay)]
    /// Switches among All Markup, No Markup, and Original without accepting or
    /// rejecting any revision in the document model.
    pub fn set_revision_display(&self, mode: &str) -> Result<JsValue, JsValue> {
        let (render_changes, render_original) = match mode {
            "all" => (true, false),
            "final" | "none" => (false, false),
            "original" => (false, true),
            _ => return Err(JsValue::from_str("unsupported revision display mode")),
        };
        {
            let mut state_mut = self.state.borrow_mut();
            state_mut.options.revision_display_explicit = true;
            if state_mut.options.render_changes == render_changes
                && state_mut.options.render_original == render_original
            {
                return Ok(JsValue::NULL);
            }
            state_mut.options.render_changes = render_changes;
            state_mut.options.render_original = render_original;
        }
        self.rerender_after_display_change(|renderer| {
            renderer.set_revision_display_flags(render_changes, render_original);
        })
    }

    /// Re-renders the cached document and builds the payload the embedder
    /// applies. `prepare` runs against the renderer just before the re-render.
    fn rerender_after_display_change(
        &self,
        prepare: impl FnOnce(&mut crate::canvas_renderer::CanvasRenderer),
    ) -> Result<JsValue, JsValue> {
        let (metadata, snapshots, caret_metrics) = {
            let mut renderer = self.renderer.borrow_mut();
            prepare(&mut renderer);
            // Display preferences can change pixels without mutating document
            // content. Explicitly dirty the visible pages so the caller gets
            // replacement bitmaps even after prior dirty regions were consumed.
            renderer.mark_full_repaint();
            renderer.skip_change_detection_once();
            renderer.rerender_with_cached_state()?;
            let metadata = renderer.table_resize_metadata();
            let caret_metrics = renderer.caret_metrics();
            let (snapshots, dirty_regions) = self.capture_paged_images(&mut renderer)?;
            (metadata, (snapshots, dirty_regions), caret_metrics)
        };

        self.store_paged_images(snapshots.0.clone());
        {
            let mut state_mut = self.state.borrow_mut();
            state_mut.table_resize = metadata;
        }

        let preview_mode = self.state.borrow().preview_mode;
        let size = self.canvas_size()?;
        let pages = self.paged_images();
        let caret_value = if preview_mode {
            JsValue::NULL
        } else {
            match caret_metrics {
                Some(metrics) => to_value(&metrics).unwrap_or(JsValue::NULL),
                None => JsValue::NULL,
            }
        };
        let history_flags = self.history_flags();
        let selection_after = { self.renderer.borrow().selection_range() };

        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &size)?;
        Reflect::set(&result, &JsValue::from_str("pages"), &pages)?;
        Reflect::set(&result, &JsValue::from_str("caret"), &caret_value)?;
        Reflect::set(&result, &JsValue::from_str("dirtyRegions"), &snapshots.1)?;
        Self::write_history_flags(&result, history_flags)?;
        self.write_object_selection(&result)?;
        {
            let renderer_ref = self.renderer.borrow();
            Self::write_selection_rects(&result, &renderer_ref, selection_after)?;
        }

        Ok(result.into())
    }

    /// Overrides the default comment decoration colors and re-renders the cached
    /// document so the new colors take effect immediately. Each argument is a CSS
    /// color string; pass `null`/`undefined` to keep the current value for that
    /// slot.
    #[wasm_bindgen(js_name = setCommentColors)]
    pub fn set_comment_colors(
        &self,
        highlight_fill: JsValue,
        marker_fill: JsValue,
        marker_outline: JsValue,
    ) -> Result<JsValue, JsValue> {
        let to_color = |value: JsValue| -> Option<Rc<str>> {
            value.as_string().and_then(|s| {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(Rc::from(trimmed))
                }
            })
        };

        let (metadata, snapshots, caret_metrics) = {
            let mut renderer = self.renderer.borrow_mut();
            renderer.set_comment_colors(
                to_color(highlight_fill),
                to_color(marker_fill),
                to_color(marker_outline),
            );
            renderer.skip_change_detection_once();
            renderer.rerender_with_cached_state()?;
            let metadata = renderer.table_resize_metadata();
            let caret_metrics = renderer.caret_metrics();
            let (snapshots, dirty_regions) = self.capture_paged_images(&mut renderer)?;
            (metadata, (snapshots, dirty_regions), caret_metrics)
        };

        self.store_paged_images(snapshots.0.clone());
        {
            let mut state_mut = self.state.borrow_mut();
            state_mut.table_resize = metadata;
        }

        let preview_mode = self.state.borrow().preview_mode;
        let size = self.canvas_size()?;
        let pages = self.paged_images();
        let caret_value = if preview_mode {
            JsValue::NULL
        } else {
            match caret_metrics {
                Some(metrics) => to_value(&metrics).unwrap_or(JsValue::NULL),
                None => JsValue::NULL,
            }
        };
        let history_flags = self.history_flags();
        let selection_after = { self.renderer.borrow().selection_range() };

        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &size)?;
        Reflect::set(&result, &JsValue::from_str("pages"), &pages)?;
        Reflect::set(&result, &JsValue::from_str("caret"), &caret_value)?;
        Reflect::set(&result, &JsValue::from_str("dirtyRegions"), &snapshots.1)?;
        Self::write_history_flags(&result, history_flags)?;
        self.write_object_selection(&result)?;
        {
            let renderer_ref = self.renderer.borrow();
            Self::write_selection_rects(&result, &renderer_ref, selection_after)?;
        }

        Ok(result.into())
    }

    #[wasm_bindgen(js_name = setRecordChanges)]
    pub fn set_record_changes(&self, enabled: bool, metadata: JsValue) -> Result<JsValue, JsValue> {
        let metadata = parse_revision_metadata(metadata)?;
        let mut session_to_apply: Option<RevisionSession> = None;

        {
            let mut state_mut = self.state.borrow_mut();
            state_mut.options.set_record_changes(enabled);

            if enabled {
                if let Some(author) = metadata.author.clone() {
                    state_mut.options.set_revision_author(Some(author));
                } else if state_mut.options.revision_author_internal().is_none() {
                    state_mut
                        .options
                        .set_revision_author(Some(DEFAULT_AUTHOR_NAME.to_string()));
                }

                if let Some(date) = metadata.date.clone() {
                    state_mut.options.set_revision_date(Some(date));
                }

                if let Some(date_utc) = metadata.date_utc.clone() {
                    state_mut.options.set_revision_date_utc(Some(date_utc));
                }

                let mut session = RevisionSession {
                    author: state_mut.options.revision_author_internal(),
                    date: state_mut.options.revision_date_internal(),
                    date_utc: state_mut.options.revision_date_utc_internal(),
                };

                if session.author.is_none() {
                    session.author = Some(DEFAULT_AUTHOR_NAME.to_string());
                }

                session_to_apply = Some(session);
            } else {
                state_mut.options.clear_revision_metadata();
            }

            if let Some(word_doc) = state_mut.word_doc.as_mut() {
                word_doc.set_revision_session(session_to_apply.clone());
            }
        }

        Ok(self.history_state())
    }

    /// Accept every tracked change in the document (Word の「すべての変更を反映」).
    /// Inserted runs become normal text and deleted runs are removed.
    #[wasm_bindgen(js_name = "acceptAllRevisions")]
    pub fn accept_all_revisions(&self) -> Result<JsValue, JsValue> {
        self.apply_revision_decision_all(true)
    }

    /// Reject every tracked change in the document (Word の「すべての変更を元に戻す」).
    /// Inserted runs are removed and deleted runs are restored.
    #[wasm_bindgen(js_name = "rejectAllRevisions")]
    pub fn reject_all_revisions(&self) -> Result<JsValue, JsValue> {
        self.apply_revision_decision_all(false)
    }

    /// Lists every tracked change in document order (Word の校閲ウィンドウ).
    ///
    /// The review workflow this exists for is per-change: a returned draft is
    /// worked through one revision at a time, and "accept everything" is the
    /// exception rather than the rule.
    #[wasm_bindgen(js_name = "listRevisions")]
    pub fn list_revisions(&self) -> Result<JsValue, JsValue> {
        let mut state = self.state.borrow_mut();
        let Some(doc) = state.word_doc.as_mut() else {
            return Ok(JsValue::NULL);
        };
        doc.ensure_roundtrip_xml()
            .map_err(|err| JsValue::from_str(&err.to_string()))?;
        let revisions: Vec<RevisionJs> = crate::headless_api::revision_snapshots(doc)
            .into_iter()
            .map(|snapshot| RevisionJs {
                id: snapshot.id,
                kind: snapshot.kind,
                author: snapshot.author,
                date: snapshot.date,
                date_utc: snapshot.date_utc,
                paragraph: snapshot.paragraph,
                start_offset: snapshot.start_offset,
                end_offset: snapshot.end_offset,
                text: snapshot.text,
                paragraph_mark: snapshot.paragraph_mark,
            })
            .collect();
        to_value(&revisions).map_err(|err| JsValue::from_str(&err.to_string()))
    }

    /// Lists the paragraph styles the document defines (Word のスタイル
    /// ギャラリー).
    ///
    /// Ordered the way the gallery reads: the default style first, then
    /// headings by outline level, then the rest by name. Styles are keyed by
    /// `w:styleId`, which is what `setParagraphStyle` takes.
    #[wasm_bindgen(js_name = "listParagraphStyles")]
    pub fn list_paragraph_styles(&self) -> JsValue {
        let state = self.state.borrow();
        let Some(doc) = state.word_doc.as_ref() else {
            return JsValue::NULL;
        };
        let Some(styles) = doc.styles_part.as_ref() else {
            return to_value::<Vec<ParagraphStyleJs>>(&Vec::new()).unwrap_or(JsValue::NULL);
        };
        to_value(&paragraph_style_options(styles)).unwrap_or(JsValue::NULL)
    }

    /// Lists the character styles the document defines (Word の文字スタイル).
    #[wasm_bindgen(js_name = "listCharacterStyles")]
    pub fn list_character_styles(&self) -> JsValue {
        let state = self.state.borrow();
        let Some(doc) = state.word_doc.as_ref() else {
            return JsValue::NULL;
        };
        let Some(styles) = doc.styles_part.as_ref() else {
            return to_value::<Vec<ParagraphStyleJs>>(&Vec::new()).unwrap_or(JsValue::NULL);
        };
        to_value(&character_style_options(styles)).unwrap_or(JsValue::NULL)
    }

    /// Applies a character style to the selection, or clears it when
    /// `style_id` is undefined (Word の「標準の文字」).
    ///
    /// Needs a real selection: a character style marks a span, and Word has no
    /// notion of applying one to a caret.
    #[wasm_bindgen(js_name = "setCharacterStyle")]
    pub fn set_character_style(&self, style_id: Option<String>) -> Result<JsValue, JsValue> {
        if self.state.borrow().preview_mode {
            return Ok(JsValue::NULL);
        }
        let selection = {
            let state_ref = self.state.borrow();
            state_ref
                .selection
                .or_else(|| self.renderer.borrow().selection_range())
        };
        let Some(selection) = selection else {
            return Err(JsValue::from_str(
                "文字スタイルを当てる範囲を選択してください",
            ));
        };
        let (start, end) = selection.normalized();
        if start.paragraph == end.paragraph && start.offset >= end.offset {
            return Err(JsValue::from_str(
                "文字スタイルを当てる範囲を選択してください",
            ));
        }
        let normalized = style_id
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        self.apply_paragraph_style_headless(move |_s, _e| HeadlessCommand::SetCharacterStyle {
            start_paragraph: start.paragraph,
            start_offset: start.offset,
            end_paragraph: end.paragraph,
            end_offset: end.offset,
            style_id: normalized.clone(),
        })
    }

    /// Accept one tracked change (Word の「この変更を反映」).
    #[wasm_bindgen(js_name = "acceptRevision")]
    pub fn accept_revision(&self, revision_id: String) -> Result<JsValue, JsValue> {
        self.apply_revision_decision_one(revision_id, true)
    }

    /// Reject one tracked change (Word の「この変更を元に戻す」).
    #[wasm_bindgen(js_name = "rejectRevision")]
    pub fn reject_revision(&self, revision_id: String) -> Result<JsValue, JsValue> {
        self.apply_revision_decision_one(revision_id, false)
    }

    /// Accepts (`accept = true`) or rejects the tracked changes a scope and
    /// filters select, as one undo step (Word の「この変更を反映」「表示されて
    /// いるすべての変更を反映」 and their 元に戻す twins).
    ///
    /// `options.scope = "selection"` (the default) takes every revision the
    /// selection overlaps, or the one at the caret; `"all"` takes every
    /// revision in the document. `kinds` and `authors` narrow either scope
    /// the way Word's 「変更履歴とコメントの表示」 filters do. With
    /// `moveToNext` the result names the revision to select afterwards.
    ///
    /// The payload carries `revisionDecision` with the counts. A decision
    /// that selects nothing changes nothing and records no undo step.
    #[wasm_bindgen(js_name = "decideRevisions")]
    pub fn decide_revisions(&self, accept: bool, options: JsValue) -> Result<JsValue, JsValue> {
        if self.state.borrow().preview_mode || !self.body_commands_allowed() {
            return Ok(JsValue::NULL);
        }
        let options: RevisionDecisionOptionsJs = if options.is_undefined() || options.is_null() {
            RevisionDecisionOptionsJs::default()
        } else {
            serde_wasm_bindgen::from_value(options)
                .map_err(|err| JsValue::from_str(&format!("invalid revision options: {err}")))?
        };
        let scope_all = match options.scope.as_deref() {
            None | Some("selection") => false,
            Some("all") => true,
            Some(other) => {
                return Err(JsValue::from_str(&format!(
                    "unsupported revision decision scope: {other}"
                )))
            }
        };
        if let Some(kind) = options.kinds.iter().flatten().find(|kind| {
            !matches!(
                kind.as_str(),
                "inserted" | "deleted" | "movedFrom" | "movedTo"
            )
        }) {
            return Err(JsValue::from_str(&format!(
                "unsupported revision kind: {kind}"
            )));
        }

        let selection_before = {
            let state = self.state.borrow();
            state
                .selection
                .or_else(|| self.renderer.borrow().selection_range())
                .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
        };
        let (render_changes, render_original) = {
            let state = self.state.borrow();
            (state.options.render_changes, state.options.render_original)
        };
        let (display_start, display_end) = selection_before.normalized();
        let forward = selection_before.anchor <= selection_before.focus;
        let selector = RevisionSelector {
            range: None,
            kinds: options.kinds.clone(),
            authors: options.authors.clone(),
        };
        let unfiltered_all = scope_all && options.kinds.is_none() && options.authors.is_none();
        // Everything below is read from the document before the decision:
        // which revisions it takes, and where the selection lands afterwards.
        let (selected, rebased_start, rebased_end) = {
            let mut state = self.state.borrow_mut();
            let doc = state
                .word_doc
                .as_mut()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;
            doc.ensure_roundtrip_xml()
                .map_err(|err| JsValue::from_str(&err.to_string()))?;
            let model_start = display_to_model_position(
                doc,
                display_start,
                render_changes,
                render_original,
                true,
            );
            let model_end = if display_start == display_end {
                model_start
            } else {
                display_to_model_position(doc, display_end, render_changes, render_original, false)
                    .max(model_start)
            };
            let snapshots = revision_snapshots(doc);
            let selector = RevisionSelector {
                range: (!scope_all).then_some((model_start, model_end)),
                ..selector.clone()
            };
            let selected: Vec<HeadlessRevisionSnapshot> = select_revisions(&snapshots, &selector)
                .into_iter()
                .cloned()
                .collect();
            let decided: Vec<&HeadlessRevisionSnapshot> = selected.iter().collect();
            let rebased_start = rebase_position_after_decision(doc, &decided, accept, model_start);
            let rebased_end = rebase_position_after_decision(doc, &decided, accept, model_end);
            (selected, rebased_start, rebased_end)
        };
        let decided = selected.len();

        let mut payload = JsValue::NULL;
        if decided > 0 {
            // The unfiltered whole-document decision is Word's 「すべての変更を
            // 反映」 and keeps the engine's whole-document path. Every narrower
            // one names its revisions by position and id: an id alone would
            // also reach any other revision the file gave the same id.
            let revision_targets = (!unfiltered_all)
                .then(|| revision_targets_for(&selected.iter().collect::<Vec<_>>()));
            let first_paragraph = selected.iter().map(|revision| revision.paragraph).min();
            let command = if accept {
                HeadlessCommand::AcceptRevisions {
                    kind: None,
                    start_paragraph: None,
                    end_paragraph: None,
                    revision_id: None,
                    revision_ids: None,
                    revision_targets,
                }
            } else {
                HeadlessCommand::RejectRevisions {
                    kind: None,
                    start_paragraph: None,
                    end_paragraph: None,
                    revision_id: None,
                    revision_ids: None,
                    revision_targets,
                }
            };
            // The selection stays where the user put it (and is what undo
            // restores); only its offsets follow the text the decision
            // removed. The repaint starts at the first decided paragraph,
            // which for a filtered decision can sit above the selection.
            payload = self.apply_paragraph_style_headless_many_with(
                move |_start, _end| vec![command],
                HeadlessApplyOverrides {
                    dirty_from: first_paragraph,
                    selection_after: Some(Box::new(move |doc: &WordDocument| {
                        let to_display = |position| {
                            model_to_display_position(
                                doc,
                                position,
                                render_changes,
                                render_original,
                            )
                        };
                        let (start, end) = (to_display(rebased_start), to_display(rebased_end));
                        Some(if forward {
                            SelectionRange::new(start, end)
                        } else {
                            SelectionRange::new(end, start)
                        })
                    })),
                },
            )?;
        }

        let next = if options.move_to_next {
            let mut state = self.state.borrow_mut();
            state.word_doc.as_mut().and_then(|doc| {
                doc.ensure_roundtrip_xml().ok()?;
                let snapshots = revision_snapshots(doc);
                next_revision_from(&snapshots, rebased_start, &selector).map(RevisionJs::from)
            })
        } else {
            None
        };

        let report = to_value(&RevisionDecisionJs { decided, next })
            .map_err(|err| JsValue::from_str(&err.to_string()))?;
        if !payload.is_object() {
            payload = Object::new().into();
        }
        Reflect::set(&payload, &JsValue::from_str("revisionDecision"), &report)?;
        Ok(payload)
    }

    fn apply_revision_decision_one(
        &self,
        revision_id: String,
        accept: bool,
    ) -> Result<JsValue, JsValue> {
        if self.state.borrow().preview_mode {
            return Ok(JsValue::NULL);
        }
        if revision_id.is_empty() {
            return Err(JsValue::from_str(
                "変更履歴の id を指定してください (id を持たない変更は個別に処理できません)。",
            ));
        }
        if let Some(doc) = self.state.borrow().word_doc.as_ref() {
            doc.ensure_revision_decision_supported_for(accept, None, Some(&revision_id))
                .map_err(|error| JsValue::from_str(&error))?;
        }

        // Same reasoning as `apply_revision_decision_all`: a single revision can
        // span runs on pages before the caret, and dropping deleted runs reflows
        // everything after it. Anchor the dirty region at the top so the
        // structural full-repaint path covers the whole document.
        {
            let mut state = self.state.borrow_mut();
            state.selection = Some(SelectionRange::collapsed(TextPosition::default()));
        }

        self.apply_paragraph_style_headless(move |_start, _end| {
            let revision_id = Some(revision_id.clone());
            if accept {
                HeadlessCommand::AcceptRevisions {
                    kind: None,
                    start_paragraph: None,
                    end_paragraph: None,
                    revision_id,
                    revision_ids: None,
                    revision_targets: None,
                }
            } else {
                HeadlessCommand::RejectRevisions {
                    kind: None,
                    start_paragraph: None,
                    end_paragraph: None,
                    revision_id,
                    revision_ids: None,
                    revision_targets: None,
                }
            }
        })
    }

    fn apply_revision_decision_all(&self, accept: bool) -> Result<JsValue, JsValue> {
        if self.state.borrow().preview_mode {
            return Ok(JsValue::NULL);
        }
        if let Some(doc) = self.state.borrow().word_doc.as_ref() {
            doc.ensure_revision_decision_supported_for(accept, None, None)
                .map_err(|error| JsValue::from_str(&error))?;
        }

        // Accept/Reject touches the whole document — paragraphs before the
        // caret can lose their deleted runs too. Anchor the dirty region at the
        // very top so the structural full-repaint path (see `is_structural_insert`)
        // covers every page rather than only the caret paragraph onward.
        {
            let mut state = self.state.borrow_mut();
            state.selection = Some(SelectionRange::collapsed(TextPosition::default()));
        }

        self.apply_paragraph_style_headless(move |_start, _end| {
            if accept {
                HeadlessCommand::AcceptRevisions {
                    kind: None,
                    start_paragraph: None,
                    end_paragraph: None,
                    revision_id: None,
                    revision_ids: None,
                    revision_targets: None,
                }
            } else {
                HeadlessCommand::RejectRevisions {
                    kind: None,
                    start_paragraph: None,
                    end_paragraph: None,
                    revision_id: None,
                    revision_ids: None,
                    revision_targets: None,
                }
            }
        })
    }

    /// Sets the page orientation of the default section (Word の「印刷の向き」).
    /// `landscape = true` makes the page wider than tall; `false` restores
    /// portrait. The whole document re-flows (full repaint).
    #[wasm_bindgen(js_name = "setPageOrientation")]
    pub fn set_page_orientation(&self, landscape: bool) -> Result<JsValue, JsValue> {
        self.apply_paragraph_style_headless(move |_start, _end| {
            HeadlessCommand::SetPageOrientation { landscape }
        })
    }

    /// Sets the page size (Word の「サイズ」). Dimensions are portrait points
    /// (height ≥ width); the current orientation is preserved. Full repaint.
    #[wasm_bindgen(js_name = "setPageSize")]
    pub fn set_page_size(
        &self,
        portrait_width: f32,
        portrait_height: f32,
    ) -> Result<JsValue, JsValue> {
        self.apply_paragraph_style_headless(move |_start, _end| HeadlessCommand::SetPageSize {
            portrait_width,
            portrait_height,
        })
    }

    /// Reads the same caret section that page setup mutations address.
    #[wasm_bindgen(js_name = getPageSetup)]
    pub fn get_page_setup(&self) -> JsValue {
        let section = self.ribbon_target_section();
        let state = self.state.borrow();
        state.word_doc.as_ref().map_or(JsValue::NULL, |document| {
            to_value(&document.section_page_setup(section)).unwrap_or(JsValue::NULL)
        })
    }

    /// Sets the page margins (Word の「余白」), in points. Full repaint.
    #[wasm_bindgen(js_name = "setPageMargins")]
    pub fn set_page_margins(
        &self,
        top: f32,
        bottom: f32,
        left: f32,
        right: f32,
    ) -> Result<JsValue, JsValue> {
        self.apply_paragraph_style_headless(move |_start, _end| HeadlessCommand::SetPageMargins {
            top: Some(top),
            bottom: Some(bottom),
            left: Some(left),
            right: Some(right),
            header: None,
            footer: None,
            gutter: None,
        })
    }

    /// Sets equal-width columns on the caret section. `space` is in
    /// points and `separator` draws a vertical rule in the gutter.
    #[wasm_bindgen(js_name = "setPageColumns")]
    pub fn set_page_columns(
        &self,
        count: u32,
        space: f32,
        separator: bool,
    ) -> Result<JsValue, JsValue> {
        self.apply_paragraph_style_headless(move |_start, _end| HeadlessCommand::SetPageColumns {
            count,
            space,
            separator,
        })
    }

    /// Reads the active section's page frame. Imported automatic colors
    /// are returned as `auto`; absent `offsetFrom` has Word's `text` semantics.
    #[wasm_bindgen(js_name = "getPageBorders")]
    pub fn get_page_borders(&self) -> JsValue {
        fn edge_state(edge: &jade_docx_reader::document::BorderEdge) -> PageBorderEdgeStateJs {
            PageBorderEdgeStateJs {
                style: edge.val.clone().unwrap_or_else(|| "none".to_string()),
                width: edge.size_eighths.map(|value| value as f32 / 8.0),
                color: edge.color.clone().unwrap_or_else(|| "auto".to_string()),
            }
        }

        let section = self.ribbon_target_section();
        let state = self.state.borrow();
        let Some(borders) = state
            .word_doc
            .as_ref()
            .and_then(|word_doc| word_doc.page_borders(section))
        else {
            return JsValue::NULL;
        };
        to_value(&PageBordersStateJs {
            top: borders.top.as_ref().map(edge_state),
            bottom: borders.bottom.as_ref().map(edge_state),
            left: borders.left.as_ref().map(edge_state),
            right: borders.right.as_ref().map(edge_state),
            offset_from: borders
                .offset_from
                .clone()
                .unwrap_or_else(|| "text".to_string()),
        })
        .unwrap_or(JsValue::NULL)
    }

    /// Replaces the final/default section's page frame. `null` clears it;
    /// `undefined`, arrays, and unknown fields are rejected at the boundary.
    #[wasm_bindgen(js_name = "setPageBorders")]
    pub fn set_page_borders(&self, value: JsValue) -> Result<JsValue, JsValue> {
        if value.is_undefined() {
            return Err(JsValue::from_str("page borders must be an object or null"));
        }
        let command = if value.is_null() {
            HeadlessCommand::SetPageBorders {
                clear: true,
                top: None,
                bottom: None,
                left: None,
                right: None,
                offset_from: None,
            }
        } else {
            let input: PageBordersInput = serde_wasm_bindgen::from_value(value)
                .map_err(|error| JsValue::from_str(&format!("invalid page borders: {error}")))?;
            HeadlessCommand::SetPageBorders {
                clear: false,
                top: input.top,
                bottom: input.bottom,
                left: input.left,
                right: input.right,
                offset_from: input.offset_from,
            }
        };
        self.apply_paragraph_style_headless(move |_start, _end| command)
    }

    /// Reads the normalized Word-compatible text watermark, or `null` when the
    /// document has none.
    #[wasm_bindgen(js_name = "getWatermark")]
    pub fn get_watermark(&self) -> JsValue {
        self.state
            .borrow()
            .word_doc
            .as_ref()
            .and_then(|word_doc| word_doc.document.as_deref())
            .and_then(|document| document.watermark_text.clone())
            .map(|text| JsValue::from_str(&text))
            .unwrap_or(JsValue::NULL)
    }

    /// Replaces the Word-compatible text watermark. A string sets it and
    /// `null` clears it; `undefined`, objects, arrays, and numbers are rejected
    /// rather than being silently stringified.
    #[wasm_bindgen(js_name = "setWatermark")]
    pub fn set_watermark(&self, value: JsValue) -> Result<JsValue, JsValue> {
        if value.is_undefined() {
            return Err(JsValue::from_str("watermark must be a string or null"));
        }
        let command = if value.is_null() {
            HeadlessCommand::SetWatermark {
                clear: true,
                text: None,
            }
        } else if let Some(text) = value.as_string() {
            HeadlessCommand::SetWatermark {
                clear: false,
                text: Some(text),
            }
        } else {
            return Err(JsValue::from_str("watermark must be a string or null"));
        };
        self.apply_paragraph_style_headless(move |_start, _end| command)
    }

    /// Reads the editable Word document theme, or `null` when the package has
    /// no theme relationship.
    #[wasm_bindgen(js_name = "getDocumentTheme")]
    pub fn get_document_theme(&self) -> JsValue {
        let state = self.state.borrow();
        state
            .word_doc
            .as_ref()
            .and_then(WordDocument::document_theme)
            .and_then(|theme| serde_wasm_bindgen::to_value(&theme).ok())
            .unwrap_or(JsValue::NULL)
    }

    /// Replaces the package-wide Word theme fonts and colors. Unknown fields
    /// are rejected at the WASM boundary.
    #[wasm_bindgen(js_name = "setDocumentTheme")]
    pub fn set_document_theme(&self, value: JsValue) -> Result<JsValue, JsValue> {
        if value.is_null() || value.is_undefined() {
            return Err(JsValue::from_str("document theme must be an object"));
        }
        let theme: HeadlessDocumentTheme = serde_wasm_bindgen::from_value(value)
            .map_err(|error| JsValue::from_str(&format!("invalid document theme: {error}")))?;
        self.apply_paragraph_style_headless(move |_start, _end| HeadlessCommand::SetDocumentTheme {
            theme,
        })
    }

    /// Inserts a section break at the collapsed body caret. The active
    /// section's complete properties are cloned onto the paragraph before the
    /// break; the following section receives `nextPage`, `continuous`,
    /// `evenPage`, or `oddPage`.
    #[wasm_bindgen(js_name = "insertSectionBreak")]
    pub fn insert_section_break(&self, break_type: &str) -> Result<JsValue, JsValue> {
        let break_type = match break_type.trim() {
            "nextPage" => HeadlessSectionBreakType::NextPage,
            "continuous" => HeadlessSectionBreakType::Continuous,
            "evenPage" => HeadlessSectionBreakType::EvenPage,
            "oddPage" => HeadlessSectionBreakType::OddPage,
            _ => {
                return Err(JsValue::from_str(
                    "section break type must be nextPage, continuous, evenPage, or oddPage",
                ));
            }
        };
        let selection = {
            let state_ref = self.state.borrow();
            state_ref
                .selection
                .or_else(|| self.renderer.borrow().selection_range())
                .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
        };
        if !selection.is_collapsed() {
            return Err(JsValue::from_str(
                "section breaks require a collapsed body caret",
            ));
        }
        let position = selection.focus;
        self.apply_paragraph_style_headless(move |_start, _end| {
            HeadlessCommand::InsertSectionBreak {
                paragraph: position.paragraph,
                offset: position.offset,
                break_type,
            }
        })
    }

    /// Sets the line spacing (Word の「行間」) of the selected paragraphs.
    /// `line` is a multiplier (1.0/1.5/2.0); ≤0 clears the explicit spacing.
    #[wasm_bindgen(js_name = "setLineSpacing")]
    pub fn set_line_spacing(&self, line: f32) -> Result<JsValue, JsValue> {
        let line = if line.is_finite() && line > 0.0 {
            Some(line)
        } else {
            None
        };
        self.apply_paragraph_style_headless(move |start, end| {
            HeadlessCommand::SetParagraphLineSpacing {
                start_paragraph: start,
                end_paragraph: end,
                // The editor's line-spacing control always writes the line
                // height, clearing it when the caller passes a non-positive
                // value.
                line: Some(line),
                line_rule: None,
                before: None,
                after: None,
            }
        })
    }

    /// Replaces explicit tab stops on the selected body or active story paragraph.
    #[wasm_bindgen(js_name = "setParagraphTabs")]
    pub fn set_paragraph_tabs(&self, tabs: JsValue) -> Result<JsValue, JsValue> {
        let tabs: Vec<jade_docx_headless_api::HeadlessDirectTabStop> =
            serde_wasm_bindgen::from_value(tabs)
                .map_err(|error| JsValue::from_str(&error.to_string()))?;
        self.apply_paragraph_style_headless(move |start, end| HeadlessCommand::SetParagraphTabs {
            start_paragraph: start,
            end_paragraph: end,
            tabs,
        })
    }

    /// Sets the selected paragraphs' left and right indentation in points.
    /// Point-based values intentionally replace `w:leftChars` / `w:rightChars`
    /// so a precise Layout-ribbon value is not shadowed by an earlier 1字 step.
    #[wasm_bindgen(js_name = "setParagraphIndent")]
    pub fn set_paragraph_indent(
        &self,
        left: f32,
        right: f32,
        first_line: Option<f32>,
    ) -> Result<JsValue, JsValue> {
        if !valid_paragraph_indents(left, right)
            || first_line.is_some_and(|value| !value.is_finite())
        {
            return Err(JsValue::from_str(
                "paragraph indents must be finite point values",
            ));
        }
        self.apply_paragraph_style_headless(move |start, end| HeadlessCommand::SetParagraphIndent {
            start_paragraph: start,
            end_paragraph: end,
            left: Some(left),
            right: Some(right),
            first_line: first_line.filter(|value| *value >= 0.0),
            hanging: first_line.filter(|value| *value < 0.0).map(f32::abs),
            left_chars: None,
            right_chars: None,
            first_line_chars: None,
            hanging_chars: None,
            clear_special: first_line == Some(0.0),
        })
    }

    /// Word's paragraph-dialog 特殊インデント control: `"firstLine"` (字下げ)
    /// or `"hanging"` (ぶら下げ) with a width in points, or `"none"` to remove
    /// both while keeping the left/right edges.
    #[wasm_bindgen(js_name = "setParagraphSpecialIndent")]
    pub fn set_paragraph_special_indent(
        &self,
        kind: String,
        amount: Option<f32>,
    ) -> Result<JsValue, JsValue> {
        let (first_line, hanging, clear_special) = match kind.as_str() {
            "none" => (None, None, true),
            "firstLine" | "hanging" => {
                let amount = amount.ok_or_else(|| {
                    JsValue::from_str("special indent requires an amount in points")
                })?;
                if !amount.is_finite() || amount < 0.0 {
                    return Err(JsValue::from_str(
                        "special indent must be a finite non-negative point value",
                    ));
                }
                if kind == "firstLine" {
                    (Some(amount), None, false)
                } else {
                    (None, Some(amount), false)
                }
            }
            _ => {
                return Err(JsValue::from_str(&format!(
                    "unknown special indent kind: {kind}"
                )))
            }
        };
        self.apply_paragraph_style_headless(move |start, end| HeadlessCommand::SetParagraphIndent {
            start_paragraph: start,
            end_paragraph: end,
            left: None,
            right: None,
            first_line,
            hanging,
            left_chars: None,
            right_chars: None,
            first_line_chars: None,
            hanging_chars: None,
            clear_special,
        })
    }

    /// Word's Ctrl+Q: strip direct paragraph formatting from the selected
    /// paragraphs while keeping their style, numbering, and run formatting.
    #[wasm_bindgen(js_name = "resetParagraphFormatting")]
    pub fn reset_paragraph_formatting(&self) -> Result<JsValue, JsValue> {
        self.apply_paragraph_style_headless(|start, end| {
            HeadlessCommand::ResetParagraphFormatting {
                start_paragraph: start,
                end_paragraph: end,
            }
        })
    }

    /// Sets the selected paragraphs' spacing before and after in points without
    /// changing their current line-height rule.
    #[wasm_bindgen(js_name = "setParagraphSpacing")]
    pub fn set_paragraph_spacing(&self, before: f32, after: f32) -> Result<JsValue, JsValue> {
        if !before.is_finite() || !after.is_finite() || before < 0.0 || after < 0.0 {
            return Err(JsValue::from_str(
                "paragraph spacing must be finite non-negative point values",
            ));
        }
        self.apply_paragraph_style_headless(move |start, end| {
            HeadlessCommand::SetParagraphLineSpacing {
                start_paragraph: start,
                end_paragraph: end,
                line: None,
                line_rule: None,
                before: Some(before),
                after: Some(after),
            }
        })
    }

    /// Applies the paragraph dialog's indent, spacing and 特殊インデント
    /// fields as one history operation, so one Undo restores the complete
    /// dialog submission. Like Word's dialog, an omitted (`undefined`) field
    /// is left as it is, keeping `w:leftChars`/`w:beforeLines` and the
    /// per-paragraph values of a multi-paragraph selection.
    #[wasm_bindgen(js_name = "setParagraphMetrics")]
    pub fn set_paragraph_metrics(
        &self,
        left: Option<f32>,
        right: Option<f32>,
        before: Option<f32>,
        after: Option<f32>,
        special_kind: Option<String>,
        special_amount: Option<f32>,
    ) -> Result<JsValue, JsValue> {
        if !valid_optional_paragraph_metrics(left, right, before, after) {
            return Err(JsValue::from_str(
                "paragraph indents must be finite and spacing must be finite non-negative point values",
            ));
        }
        let (first_line, hanging, clear_special) = match special_kind.as_deref() {
            None => (None, None, false),
            Some("none") => (None, None, true),
            Some(kind @ ("firstLine" | "hanging")) => {
                let amount = special_amount
                    .filter(|amount| amount.is_finite() && *amount >= 0.0)
                    .ok_or_else(|| {
                        JsValue::from_str(
                            "special indent must be a finite non-negative point value",
                        )
                    })?;
                if kind == "firstLine" {
                    (Some(amount), None, false)
                } else {
                    (None, Some(amount), false)
                }
            }
            Some(kind) => {
                return Err(JsValue::from_str(&format!(
                    "unknown special indent kind: {kind}"
                )))
            }
        };
        self.apply_paragraph_style_headless(move |start, end| {
            HeadlessCommand::SetParagraphMetrics {
                start_paragraph: start,
                end_paragraph: end,
                left,
                right,
                before,
                after,
                first_line,
                hanging,
                clear_special,
            }
        })
    }

    /// Sets a solid paragraph background fill, or clears direct shading when
    /// `color` is omitted/empty.
    #[wasm_bindgen(js_name = "setParagraphShading")]
    pub fn set_paragraph_shading(&self, color: Option<String>) -> Result<JsValue, JsValue> {
        let color = color
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if !self.body_commands_allowed() {
            return self.apply_hf_paragraph_commands(vec![HeadlessCommand::SetParagraphShading {
                start_paragraph: 0,
                end_paragraph: 0,
                color,
            }]);
        }
        self.apply_paragraph_style_headless(move |start, end| {
            HeadlessCommand::SetParagraphShading {
                start_paragraph: start,
                end_paragraph: end,
                color,
            }
        })
    }

    /// Replaces direct paragraph border sides. `sides` is any combination of
    /// `t`, `b`, `l`, `r`; an empty string clears the border box.
    #[wasm_bindgen(js_name = "setParagraphBorders")]
    pub fn set_paragraph_borders(
        &self,
        sides: &str,
        color: &str,
        width: f32,
        style: &str,
    ) -> Result<JsValue, JsValue> {
        let normalized_sides = sides.trim().to_ascii_lowercase();
        if normalized_sides
            .chars()
            .any(|side| !matches!(side, 't' | 'b' | 'l' | 'r'))
        {
            return Err(JsValue::from_str(
                "paragraph border sides must contain only t, b, l, r",
            ));
        }
        let edge = HeadlessBorderEdge {
            style: Some(style.trim().to_string()),
            width: Some(width),
            color: Some(color.trim().to_string()),
        };
        self.apply_paragraph_style_headless(move |start, end| {
            HeadlessCommand::SetParagraphBorders {
                start_paragraph: start,
                end_paragraph: end,
                top: normalized_sides.contains('t').then(|| edge.clone()),
                bottom: normalized_sides.contains('b').then(|| edge.clone()),
                left: normalized_sides.contains('l').then(|| edge.clone()),
                right: normalized_sides.contains('r').then(|| edge.clone()),
            }
        })
    }

    /// Sets the page background color (デザインタブの「ページの色」). Pass an
    /// empty string to clear it. Full repaint.
    #[wasm_bindgen(js_name = "setPageBackgroundColor")]
    pub fn set_page_background_color(&self, color: &str) -> Result<JsValue, JsValue> {
        let color = color.trim();
        let color = if color.is_empty() {
            None
        } else {
            Some(color.to_string())
        };
        self.apply_paragraph_style_headless(move |_start, _end| {
            HeadlessCommand::SetPageBackgroundColor { color }
        })
    }

    /// Sets the name written into comments and tracked changes.
    ///
    /// Word stores the editing user's name on every `w:comment` / `w:ins` /
    /// `w:del`, and review flows are unusable when every change claims the same
    /// author. Hosts call this once the signed-in user is known; passing `None`
    /// (or a blank string) clears it and falls back to the generic name.
    #[wasm_bindgen(js_name = "setAuthorName")]
    pub fn set_author_name(&self, name: Option<String>) {
        let mut state_mut = self.state.borrow_mut();
        state_mut.options.set_revision_author(name);
    }

    /// Sets the initials written on comments (ユーザー名の頭文字); `None` or
    /// a blank string takes them from the author name.
    #[wasm_bindgen(js_name = "setAuthorInitials")]
    pub fn set_author_initials(&self, initials: Option<String>) {
        self.state
            .borrow_mut()
            .options
            .set_author_initials(initials);
    }

    /// Author recorded on new comments. Shares the revision identity because
    /// Word takes both from the same user name.
    fn comment_author(&self) -> String {
        self.state.borrow().options.author_name_or_default()
    }

    /// The initials recorded on new comments beside [`Self::comment_author`].
    fn comment_initials(&self) -> String {
        self.state.borrow().options.author_initials_or_default()
    }

    /// Adds a comment anchored to the current (non-collapsed, single-paragraph)
    /// selection. Errors if the selection is empty or spans paragraphs.
    #[wasm_bindgen(js_name = "addComment")]
    pub fn add_comment(&self, text: &str) -> Result<JsValue, JsValue> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err(JsValue::from_str("コメント本文を入力してください"));
        }
        let selection = {
            let state_ref = self.state.borrow();
            state_ref
                .selection
                .or_else(|| self.renderer.borrow().selection_range())
        };
        let Some(selection) = selection else {
            return Err(JsValue::from_str("コメントを付ける範囲を選択してください"));
        };
        let (start, end) = selection.normalized();
        if start.paragraph != end.paragraph || start.offset >= end.offset {
            return Err(JsValue::from_str(
                "コメントは同一段落内の選択範囲に付けられます",
            ));
        }
        let start_offset = start.offset;
        let end_offset = end.offset;
        let author = self.comment_author();
        let initials = self.comment_initials();
        // Word と同じくコメントには付けた日時を書く。書かないと、取り込む側で 1970/01/01 のコメントになる。
        let date = current_field_datetime().to_word_comment_date();
        self.apply_paragraph_style_headless(move |start_para, _end_para| {
            HeadlessCommand::AddComment {
                start_paragraph: start_para,
                start_offset: Some(start_offset),
                end_paragraph: start_para,
                end_offset: Some(end_offset),
                text,
                author: Some(author),
                initials: Some(initials),
                date: Some(date),
            }
        })
    }

    /// Inserts a footnote reference at the caret and creates the corresponding
    /// `word/footnotes.xml` entry (ECMA-376 Part 1 §17.11.10 footnoteReference).
    #[wasm_bindgen(js_name = "insertFootnote")]
    pub fn insert_footnote(&self, text: Option<String>) -> Result<JsValue, JsValue> {
        self.insert_note(text, false)
    }

    /// Inserts an endnote reference at the caret and creates the corresponding
    /// `word/endnotes.xml` entry (ECMA-376 Part 1 §17.11.2 endnoteReference).
    #[wasm_bindgen(js_name = "insertEndnote")]
    pub fn insert_endnote(&self, text: Option<String>) -> Result<JsValue, JsValue> {
        self.insert_note(text, true)
    }

    /// Returns substantive endnotes in first-reference order for a host-side
    /// management dialog. Reserved separator entries never leak through.
    #[wasm_bindgen(js_name = "listEndnotes")]
    pub fn list_endnotes(&self) -> JsValue {
        let state = self.state.borrow();
        let endnotes = state
            .word_doc
            .as_ref()
            .map(WordDocument::endnote_infos)
            .unwrap_or_default();
        to_value(&endnotes).unwrap_or(JsValue::NULL)
    }

    /// Replaces one endnote body while preserving its id and body references.
    #[wasm_bindgen(js_name = "updateEndnote")]
    pub fn update_endnote(&self, endnote_id: String, text: String) -> Result<JsValue, JsValue> {
        let endnote_id = endnote_id.trim().to_string();
        if endnote_id.is_empty() {
            return Err(JsValue::from_str("endnote id must not be empty"));
        }
        self.apply_paragraph_style_headless(move |_start, _end| HeadlessCommand::UpdateEndnote {
            endnote_id,
            text,
        })
    }

    /// Removes one endnote entry and every reference to it atomically.
    #[wasm_bindgen(js_name = "deleteEndnote")]
    pub fn delete_endnote(&self, endnote_id: String) -> Result<JsValue, JsValue> {
        let endnote_id = endnote_id.trim().to_string();
        if endnote_id.is_empty() {
            return Err(JsValue::from_str("endnote id must not be empty"));
        }
        self.apply_paragraph_style_headless(move |_start, _end| HeadlessCommand::DeleteEndnote {
            endnote_id,
        })
    }

    /// Returns substantive footnotes in first-reference order (the footnote
    /// twin of `listEndnotes`).
    #[wasm_bindgen(js_name = "listFootnotes")]
    pub fn list_footnotes(&self) -> JsValue {
        let state = self.state.borrow();
        let footnotes = state
            .word_doc
            .as_ref()
            .map(WordDocument::footnote_infos)
            .unwrap_or_default();
        to_value(&footnotes).unwrap_or(JsValue::NULL)
    }

    /// Returns body reference ranges using the document's shared marker offsets.
    #[wasm_bindgen(js_name = "footnoteReferencePositions")]
    pub fn footnote_reference_positions(&self) -> JsValue {
        let state = self.state.borrow();
        let positions = state
            .word_doc
            .as_ref()
            .map(WordDocument::footnote_reference_positions)
            .unwrap_or_default();
        to_value(&positions).unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen(js_name = "endnoteReferencePositions")]
    pub fn endnote_reference_positions(&self) -> JsValue {
        let state = self.state.borrow();
        let positions = state
            .word_doc
            .as_ref()
            .map(WordDocument::endnote_reference_positions)
            .unwrap_or_default();
        to_value(&positions).unwrap_or(JsValue::NULL)
    }

    /// Replaces one footnote body while preserving its id and body references.
    #[wasm_bindgen(js_name = "updateFootnote")]
    pub fn update_footnote(&self, footnote_id: String, text: String) -> Result<JsValue, JsValue> {
        let footnote_id = footnote_id.trim().to_string();
        if footnote_id.is_empty() {
            return Err(JsValue::from_str("footnote id must not be empty"));
        }
        self.apply_paragraph_style_headless(move |_start, _end| HeadlessCommand::UpdateFootnote {
            footnote_id,
            text,
        })
    }

    /// Removes one footnote entry and every reference to it atomically.
    #[wasm_bindgen(js_name = "deleteFootnote")]
    pub fn delete_footnote(&self, footnote_id: String) -> Result<JsValue, JsValue> {
        let footnote_id = footnote_id.trim().to_string();
        if footnote_id.is_empty() {
            return Err(JsValue::from_str("footnote id must not be empty"));
        }
        self.apply_paragraph_style_headless(move |_start, _end| HeadlessCommand::DeleteFootnote {
            footnote_id,
        })
    }

    /// Converts a note and its references as one undoable package edit.
    #[wasm_bindgen(js_name = "convertFootnoteToEndnote")]
    pub fn convert_footnote_to_endnote(&self, footnote_id: String) -> Result<JsValue, JsValue> {
        let footnote_id = footnote_id.trim().to_string();
        if footnote_id.is_empty() {
            return Err(JsValue::from_str("note id must not be empty"));
        }
        self.apply_paragraph_style_headless(move |_start, _end| {
            HeadlessCommand::ConvertFootnoteToEndnote { footnote_id }
        })
    }

    /// Converts a note and its references as one undoable package edit.
    #[wasm_bindgen(js_name = "convertEndnoteToFootnote")]
    pub fn convert_endnote_to_footnote(&self, endnote_id: String) -> Result<JsValue, JsValue> {
        let endnote_id = endnote_id.trim().to_string();
        if endnote_id.is_empty() {
            return Err(JsValue::from_str("note id must not be empty"));
        }
        self.apply_paragraph_style_headless(move |_start, _end| {
            HeadlessCommand::ConvertEndnoteToFootnote { endnote_id }
        })
    }

    fn insert_note(&self, text: Option<String>, endnote: bool) -> Result<JsValue, JsValue> {
        if self.state.borrow().preview_mode || self.state.borrow().editing_mode != EditingMode::Body
        {
            return Ok(JsValue::NULL);
        }

        let selection_before = {
            let state_ref = self.state.borrow();
            state_ref
                .selection
                .or_else(|| self.renderer.borrow().selection_range())
                .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::default()))
        };
        let dirty_from = selection_before.normalized().0.paragraph;

        // See `apply_inline_style_with_scope`.
        let _document_cache = self.drop_renderer_document_cache();
        let (options, numbering_part, header_lines, footer_lines, new_selection) = {
            let mut state = self.state.borrow_mut();
            let mut word_doc = state
                .word_doc
                .take()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;
            state.selection = Some(selection_before);
            let mut command_executor = std::mem::take(&mut state.command_executor);
            let command_result = {
                let mut ctx = CommandContext::new(&mut word_doc, &mut state.selection);
                let command: Box<dyn Command> = if endnote {
                    Box::new(InsertEndnoteCommand::new(
                        selection_before,
                        text.unwrap_or_default(),
                    ))
                } else {
                    Box::new(InsertFootnoteCommand::new(
                        selection_before,
                        text.unwrap_or_default(),
                    ))
                };
                command_executor.execute(command, &mut ctx)
            };
            state.command_executor = command_executor;
            if let Err(err) = command_result {
                state.word_doc = Some(word_doc);
                return Err(JsValue::from_str(&err.to_string()));
            }
            state.paged_images.clear();

            let options = state.options.clone();
            let numbering_part = word_doc.numbering_part.clone();
            let header_lines = word_doc.header_lines.clone();
            let footer_lines = word_doc.footer_lines.clone();
            let new_selection = state.selection;
            state.word_doc = Some(word_doc);
            (
                options,
                numbering_part,
                header_lines,
                footer_lines,
                new_selection,
            )
        };

        let anchor_page = {
            let renderer = self.renderer.borrow();
            renderer
                .cached_paragraph_start_page(dirty_from)
                .or_else(|| renderer.visible_page_range().as_ref().map(|r| *r.end()))
                .or_else(|| renderer.layout_page_limit())
                .unwrap_or(1)
        };

        let mut renderer = self.renderer.borrow_mut();
        let lazy_mode = self.state.borrow().lazy_mode;
        Self::ensure_visible_page_range_for_edit(&mut renderer, lazy_mode, anchor_page);
        let prev_limit: Option<Option<usize>> = if lazy_mode {
            let desired = Self::lazy_visible_end(&renderer, anchor_page);
            let prev = renderer.layout_page_limit();
            renderer.set_layout_page_limit(Some(desired));
            Some(prev)
        } else {
            renderer.set_layout_page_limit(None);
            None
        };
        renderer.disable_seek_replay_once();
        renderer.invalidate_layout_from(dirty_from);
        renderer.invalidate_paragraph_wrap_cache_from(dirty_from);
        if lazy_mode {
            let end_page_1idx = Self::lazy_visible_end(&renderer, anchor_page);
            renderer.mark_dirty_from_paragraph_to_end_bounded(dirty_from, end_page_1idx);
        } else {
            renderer.mark_full_repaint();
        }

        {
            let state_ref = self.state.borrow();
            if let Some(word_doc) = state_ref.word_doc.as_ref() {
                if let Some(document) = word_doc.document.as_ref() {
                    renderer.render_with_numbering_rc(
                        Rc::clone(document),
                        &options,
                        numbering_part.as_ref(),
                        header_lines.as_deref(),
                        footer_lines.as_deref(),
                        Some(&word_doc.comments),
                    )?;
                }
            }
        }
        if let Some(prev) = prev_limit {
            renderer.set_layout_page_limit(prev);
        }
        if let Some(range) = new_selection {
            renderer.set_selection_range(Some(range));
        }
        let caret_metrics = renderer.caret_metrics();
        let selection_after = renderer.selection_range();
        let (snapshots, dirty_regions_js) = if lazy_mode {
            renderer.coalesce_dirty_regions();
            let dirty_regions = renderer.dirty_regions();
            let dirty_regions_js = Self::dirty_regions_to_js(&dirty_regions);
            renderer.clear_dirty_regions();
            (None, dirty_regions_js)
        } else {
            let (snapshots, dirty_regions) = self.capture_paged_images(&mut renderer)?;
            (snapshots, dirty_regions)
        };
        drop(renderer);

        self.store_paged_images(snapshots);
        let (preview_mode, history_flags) = {
            let mut state_mut = self.state.borrow_mut();
            state_mut.selection = selection_after;
            state_mut.object_selection = None;
            state_mut.caret_goal_x = None;
            (state_mut.preview_mode, state_mut.history_flags())
        };

        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &self.canvas_size()?)?;
        Reflect::set(&result, &JsValue::from_str("pages"), &self.paged_images())?;
        Reflect::set(
            &result,
            &JsValue::from_str("dirtyRegions"),
            &dirty_regions_js,
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("debugMutationKind"),
            &JsValue::from_str(if endnote {
                "insert-endnote"
            } else {
                "insert-footnote"
            }),
        )?;
        let caret_value = if preview_mode {
            JsValue::NULL
        } else {
            caret_metrics
                .as_ref()
                .map(|metrics| to_value(metrics).unwrap_or(JsValue::NULL))
                .unwrap_or(JsValue::NULL)
        };
        Reflect::set(&result, &JsValue::from_str("caret"), &caret_value)?;
        Self::write_history_flags(&result, history_flags)?;
        self.write_object_selection(&result)?;
        self.write_merge_state(&result)?;
        {
            let renderer_ref = self.renderer.borrow();
            Self::write_selection_rects(&result, &renderer_ref, selection_after)?;
        }
        Ok(result.into())
    }

    /// Adds a reply under an existing comment (Word の「返信」).
    ///
    /// Replying to a reply still answers the original point, so the document
    /// side re-roots the new comment at the thread's first comment; the panel
    /// only ever has to render two levels.
    #[wasm_bindgen(js_name = "replyToComment")]
    pub fn reply_to_comment(&self, comment_id: String, text: String) -> Result<JsValue, JsValue> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err(JsValue::from_str("返信の本文を入力してください"));
        }
        let author = self.comment_author();
        let initials = self.comment_initials();
        // 返信もコメントと同じく付けた日時を書く（`add_comment` 参照）。
        let date = current_field_datetime().to_word_comment_date();
        self.apply_paragraph_style_headless(move |_s, _e| HeadlessCommand::ReplyToComment {
            comment_id: comment_id.clone(),
            text: text.clone(),
            author: Some(author.clone()),
            initials: Some(initials.clone()),
            date: Some(date.clone()),
        })
    }

    /// Replaces the body text of an existing comment.
    #[wasm_bindgen(js_name = "updateComment")]
    pub fn update_comment(&self, comment_id: String, text: String) -> Result<JsValue, JsValue> {
        self.apply_paragraph_style_headless(move |_s, _e| HeadlessCommand::UpdateComment {
            comment_id,
            text,
        })
    }

    /// Marks a comment resolved/unresolved.
    #[wasm_bindgen(js_name = "resolveComment")]
    pub fn resolve_comment(&self, comment_id: String, resolved: bool) -> Result<JsValue, JsValue> {
        self.apply_paragraph_style_headless(move |_s, _e| HeadlessCommand::ResolveComment {
            comment_id,
            resolved,
        })
    }

    /// Deletes a comment, its replies and their anchor markers.
    #[wasm_bindgen(js_name = "deleteComment")]
    pub fn delete_comment(&self, comment_id: String) -> Result<JsValue, JsValue> {
        self.apply_paragraph_style_headless(move |_s, _e| HeadlessCommand::DeleteComment {
            comment_id,
        })
    }

    /// Lists every comment in document order (by the body paragraph of its first
    /// anchor) as `[{id, author, text, resolved, date, paragraph}]`.
    #[wasm_bindgen(js_name = "listComments")]
    pub fn list_comments(&self) -> JsValue {
        let state = self.state.borrow();
        let Some(doc) = state.word_doc.as_ref() else {
            return JsValue::NULL;
        };
        // First anchor (paragraph + byte range) per comment id, in doc order.
        let mut first_anchor: std::collections::HashMap<String, (usize, usize, usize)> =
            std::collections::HashMap::new();
        let total = doc.paragraph_count();
        for paragraph_index in 0..total {
            if let Some(paragraph) = doc.paragraph(paragraph_index) {
                for range in &paragraph.comment_ranges {
                    first_anchor.entry(range.id.clone()).or_insert((
                        paragraph_index,
                        range.start,
                        range.end,
                    ));
                }
            }
        }
        let mut comments: Vec<CommentJs> = doc
            .comments
            .values()
            .map(|c| {
                let anchor = first_anchor.get(&c.id).copied();
                CommentJs {
                    id: c.id.clone(),
                    author: c.author.clone(),
                    text: c.text.clone(),
                    resolved: c.resolved.unwrap_or(false),
                    date: c.date.clone(),
                    paragraph: anchor.map(|(p, _, _)| p),
                    start_offset: anchor.map(|(_, s, _)| s),
                    end_offset: anchor.map(|(_, _, e)| e),
                    thread_parent_id: c.thread_parent_id.clone(),
                }
            })
            .collect();
        // Document order; unanchored comments sink to the end, ties broken by id
        // for a stable list across renders.
        comments.sort_by(|a, b| {
            a.paragraph
                .unwrap_or(usize::MAX)
                .cmp(&b.paragraph.unwrap_or(usize::MAX))
                .then_with(|| a.id.cmp(&b.id))
        });
        to_value(&comments).unwrap_or(JsValue::NULL)
    }

    #[wasm_bindgen(js_name = "tableResizeDebug")]
    pub fn table_resize_debug(&self) -> JsValue {
        match self.resize_debug.borrow().as_ref() {
            Some(value) => JsValue::from_str(value),
            None => JsValue::NULL,
        }
    }

    #[wasm_bindgen]
    pub fn merge_selected_cells(&self) -> Result<JsValue, JsValue> {
        if self.state.borrow().preview_mode {
            return Ok(JsValue::NULL);
        }

        let dirty_from = {
            let selection = self.renderer.borrow().selection_range();
            selection
                .map(|range| range.anchor.paragraph.min(range.focus.paragraph))
                .unwrap_or(0)
        };

        let (table_order_index, selections) = match self.collect_table_selection() {
            Some(value) => value,
            None => return Ok(JsValue::NULL),
        };

        {
            let state = self.state.borrow();
            let word_doc = state
                .word_doc
                .as_ref()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;
            MergeTableCellsCommand::preview_plan(word_doc, table_order_index, &selections)
                .map_err(|err| JsValue::from_str(&err.to_string()))?;
        }

        // See `apply_inline_style_with_scope`.
        let _document_cache = self.drop_renderer_document_cache();
        let (options, numbering_part, header_lines, footer_lines, new_selection) = {
            let mut state = self.state.borrow_mut();
            let mut word_doc = state
                .word_doc
                .take()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;

            // Merge execution is delegated to headless API; preview validation stays local.
            let request = HeadlessEditRequest {
                commands: vec![HeadlessCommand::MergeTableCells {
                    table_order_index,
                    selected_cells: selections
                        .iter()
                        .map(|cell| HeadlessTableCellSelection {
                            row: cell.row,
                            column: cell.column,
                            column_span: cell.column_span,
                        })
                        .collect(),
                }],
            };
            let mut command_executor = std::mem::take(&mut state.command_executor);
            let command_result = apply_headless_commands_with_executor_to_document(
                &mut word_doc,
                &mut command_executor,
                state.selection,
                &request,
            );
            state.command_executor = command_executor;
            let selection_after = match command_result {
                Ok(selection) => selection,
                Err(err) => {
                    state.word_doc = Some(word_doc);
                    return Err(JsValue::from_str(&err.to_string()));
                }
            };
            state.selection = selection_after;
            state.paged_images.clear();

            let options = state.options.clone();
            let numbering_part = word_doc.numbering_part.clone();
            let header_lines = word_doc.header_lines.clone();
            let footer_lines = word_doc.footer_lines.clone();
            let new_selection = state.selection;

            state.word_doc = Some(word_doc);

            (
                options,
                numbering_part,
                header_lines,
                footer_lines,
                new_selection,
            )
        };

        let anchor_page = {
            let renderer = self.renderer.borrow();
            renderer
                .cached_paragraph_start_page(dirty_from)
                .or_else(|| renderer.visible_page_range().as_ref().map(|r| *r.end()))
                .or_else(|| renderer.layout_page_limit())
                .unwrap_or(1)
        };

        let mut renderer = self.renderer.borrow_mut();
        renderer.clear_cell_selection();
        renderer.set_incremental_dirty_from(dirty_from);
        let lazy_mode = self.state.borrow().lazy_mode;
        // Respect host-provided visible range if configured (viewport virtualization).
        // Ensure the edited page stays visible even if the range is stale.
        Self::ensure_visible_page_range_for_edit(&mut renderer, lazy_mode, anchor_page);
        // Table merges can reflow content, but in lazy mode we only lay out the visible window.
        let prev_limit: Option<Option<usize>> = if lazy_mode {
            let desired = Self::lazy_visible_end(&renderer, anchor_page);
            let prev = renderer.layout_page_limit();
            renderer.set_layout_page_limit(Some(desired));
            Some(prev)
        } else {
            renderer.set_layout_page_limit(None);
            None
        };
        {
            let state_ref = self.state.borrow();
            if let Some(word_doc) = state_ref.word_doc.as_ref() {
                if let Some(document) = word_doc.document.as_ref() {
                    renderer.render_with_numbering_rc(
                        Rc::clone(document),
                        &options,
                        numbering_part.as_ref(),
                        header_lines.as_deref(),
                        footer_lines.as_deref(),
                        Some(&word_doc.comments),
                    )?;
                }
            }
        }
        if let Some(prev) = prev_limit {
            renderer.set_layout_page_limit(prev);
        }

        if let Some(range) = new_selection {
            if renderer.set_selection_range(Some(range)) {
                renderer.skip_change_detection_once();
                renderer.rerender_with_cached_state()?;
            }
        }

        let caret_metrics = renderer.caret_metrics();
        let selection_after = renderer.selection_range();
        let skip_snapshots = lazy_mode;
        let (snapshots, dirty_regions) = if skip_snapshots {
            renderer.coalesce_dirty_regions();
            let dirty_regions = renderer.dirty_regions();
            let dirty_regions_js = Self::dirty_regions_to_js(&dirty_regions);
            renderer.clear_dirty_regions();
            (None, dirty_regions_js)
        } else {
            self.capture_paged_images(&mut renderer)?
        };
        drop(renderer);

        self.store_paged_images(snapshots);
        let (preview_mode, history_flags) = {
            let mut state_mut = self.state.borrow_mut();
            state_mut.selection = selection_after;
            state_mut.caret_goal_x = None;
            let flags = state_mut.history_flags();
            (state_mut.preview_mode, flags)
        };

        let size = self.canvas_size()?;
        let pages = self.paged_images();
        let caret_value = if preview_mode {
            JsValue::NULL
        } else {
            match caret_metrics {
                Some(metrics) => to_value(&metrics).unwrap_or(JsValue::NULL),
                None => JsValue::NULL,
            }
        };

        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &size)?;
        Reflect::set(&result, &JsValue::from_str("pages"), &pages)?;
        Reflect::set(&result, &JsValue::from_str("caret"), &caret_value)?;
        Reflect::set(&result, &JsValue::from_str("dirtyRegions"), &dirty_regions)?;
        Self::write_history_flags(&result, history_flags)?;
        self.write_object_selection(&result)?;
        self.write_merge_state(&result)?;
        {
            let renderer_ref = self.renderer.borrow();
            Self::write_selection_rects(&result, &renderer_ref, selection_after)?;
        }

        Ok(result.into())
    }

    #[wasm_bindgen]
    pub fn unmerge_selected_cells(&self) -> Result<JsValue, JsValue> {
        if self.state.borrow().preview_mode {
            return Ok(JsValue::NULL);
        }

        let dirty_from = {
            let selection = self.renderer.borrow().selection_range();
            selection
                .map(|range| range.anchor.paragraph.min(range.focus.paragraph))
                .unwrap_or(0)
        };

        let (table_order_index, selections) = match self.caret_or_selected_cells() {
            Some(value) => value,
            None => return Ok(JsValue::NULL),
        };

        {
            let state = self.state.borrow();
            let word_doc = state
                .word_doc
                .as_ref()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;
            UnmergeTableCellsCommand::preview_plan(word_doc, table_order_index, &selections)
                .map_err(|err| JsValue::from_str(&err.to_string()))?;
        }

        // See `apply_inline_style_with_scope`.
        let _document_cache = self.drop_renderer_document_cache();
        let (options, numbering_part, header_lines, footer_lines, new_selection) = {
            let mut state = self.state.borrow_mut();
            let mut word_doc = state
                .word_doc
                .take()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;

            let request = HeadlessEditRequest {
                commands: vec![HeadlessCommand::UnmergeTableCells {
                    table_order_index,
                    target_cells: selections
                        .iter()
                        .map(|cell| HeadlessTableCellSelection {
                            row: cell.row,
                            column: cell.column,
                            column_span: cell.column_span,
                        })
                        .collect(),
                }],
            };
            let mut command_executor = std::mem::take(&mut state.command_executor);
            let command_result = apply_headless_commands_with_executor_to_document(
                &mut word_doc,
                &mut command_executor,
                state.selection,
                &request,
            );
            state.command_executor = command_executor;
            let selection_after = match command_result {
                Ok(selection) => selection,
                Err(err) => {
                    state.word_doc = Some(word_doc);
                    return Err(JsValue::from_str(&err.to_string()));
                }
            };
            state.selection = selection_after;
            state.paged_images.clear();

            let options = state.options.clone();
            let numbering_part = word_doc.numbering_part.clone();
            let header_lines = word_doc.header_lines.clone();
            let footer_lines = word_doc.footer_lines.clone();
            let new_selection = state.selection;

            state.word_doc = Some(word_doc);

            (
                options,
                numbering_part,
                header_lines,
                footer_lines,
                new_selection,
            )
        };

        let anchor_page = {
            let renderer = self.renderer.borrow();
            renderer
                .cached_paragraph_start_page(dirty_from)
                .or_else(|| renderer.visible_page_range().as_ref().map(|r| *r.end()))
                .or_else(|| renderer.layout_page_limit())
                .unwrap_or(1)
        };

        let mut renderer = self.renderer.borrow_mut();
        renderer.clear_cell_selection();
        renderer.set_incremental_dirty_from(dirty_from);
        let lazy_mode = self.state.borrow().lazy_mode;
        Self::ensure_visible_page_range_for_edit(&mut renderer, lazy_mode, anchor_page);
        let prev_limit: Option<Option<usize>> = if lazy_mode {
            let desired = Self::lazy_visible_end(&renderer, anchor_page);
            let prev = renderer.layout_page_limit();
            renderer.set_layout_page_limit(Some(desired));
            Some(prev)
        } else {
            renderer.set_layout_page_limit(None);
            None
        };
        {
            let state_ref = self.state.borrow();
            if let Some(word_doc) = state_ref.word_doc.as_ref() {
                if let Some(document) = word_doc.document.as_ref() {
                    renderer.render_with_numbering_rc(
                        Rc::clone(document),
                        &options,
                        numbering_part.as_ref(),
                        header_lines.as_deref(),
                        footer_lines.as_deref(),
                        Some(&word_doc.comments),
                    )?;
                }
            }
        }
        if let Some(prev) = prev_limit {
            renderer.set_layout_page_limit(prev);
        }

        if let Some(range) = new_selection {
            if renderer.set_selection_range(Some(range)) {
                renderer.skip_change_detection_once();
                renderer.rerender_with_cached_state()?;
            }
        }

        let caret_metrics = renderer.caret_metrics();
        let selection_after = renderer.selection_range();
        let skip_snapshots = lazy_mode;
        let (snapshots, dirty_regions) = if skip_snapshots {
            renderer.coalesce_dirty_regions();
            let dirty_regions = renderer.dirty_regions();
            let dirty_regions_js = Self::dirty_regions_to_js(&dirty_regions);
            renderer.clear_dirty_regions();
            (None, dirty_regions_js)
        } else {
            self.capture_paged_images(&mut renderer)?
        };
        drop(renderer);

        self.store_paged_images(snapshots);
        let (preview_mode, history_flags) = {
            let mut state_mut = self.state.borrow_mut();
            state_mut.selection = selection_after;
            state_mut.caret_goal_x = None;
            let flags = state_mut.history_flags();
            (state_mut.preview_mode, flags)
        };

        let size = self.canvas_size()?;
        let pages = self.paged_images();
        let caret_value = if preview_mode {
            JsValue::NULL
        } else {
            match caret_metrics {
                Some(metrics) => to_value(&metrics).unwrap_or(JsValue::NULL),
                None => JsValue::NULL,
            }
        };

        let result = Object::new();
        Reflect::set(&result, &JsValue::from_str("size"), &size)?;
        Reflect::set(&result, &JsValue::from_str("pages"), &pages)?;
        Reflect::set(&result, &JsValue::from_str("caret"), &caret_value)?;
        Reflect::set(&result, &JsValue::from_str("dirtyRegions"), &dirty_regions)?;
        Self::write_history_flags(&result, history_flags)?;
        self.write_object_selection(&result)?;
        self.write_merge_state(&result)?;
        {
            let renderer_ref = self.renderer.borrow();
            Self::write_selection_rects(&result, &renderer_ref, selection_after)?;
        }

        Ok(result.into())
    }

    /// Returns text measurement cache statistics for debugging/monitoring.
    /// Returns an object with { hits, misses, size, capacity, hitRate }.
    #[wasm_bindgen(js_name = textMeasureCacheStats)]
    pub fn text_measure_cache_stats(&self) -> JsValue {
        let renderer = self.renderer.borrow();
        let stats = renderer.text_measure_cache_stats();
        let result = Object::new();
        let _ = Reflect::set(
            &result,
            &JsValue::from_str("hits"),
            &JsValue::from_f64(stats.hits as f64),
        );
        let _ = Reflect::set(
            &result,
            &JsValue::from_str("misses"),
            &JsValue::from_f64(stats.misses as f64),
        );
        let _ = Reflect::set(
            &result,
            &JsValue::from_str("size"),
            &JsValue::from_f64(stats.size as f64),
        );
        let _ = Reflect::set(
            &result,
            &JsValue::from_str("capacity"),
            &JsValue::from_f64(stats.capacity as f64),
        );
        let _ = Reflect::set(
            &result,
            &JsValue::from_str("hitRate"),
            &JsValue::from_f64(stats.hit_rate()),
        );
        result.into()
    }

    /// Clears the text measurement cache.
    #[wasm_bindgen(js_name = clearTextMeasureCache)]
    pub fn clear_text_measure_cache(&self) {
        let mut renderer = self.renderer.borrow_mut();
        renderer.clear_text_measure_cache();
    }

    /// Installs Word COM TRACE character advances for browser oracle runs.
    ///
    /// This is intentionally an explicit test/debug hook: normal browser
    /// rendering keeps using Canvas measurements, while
    /// `tests/playwright/browser-word-oracle.mjs` can replay the same
    /// per-character advances recorded in the committed Word oracle traces.
    #[wasm_bindgen(js_name = setWordOracleAdvances)]
    pub fn set_word_oracle_advances(&self, advances: JsValue) -> Result<usize, JsValue> {
        let advances: Vec<crate::canvas_renderer::WordOracleAdvanceInput> =
            serde_wasm_bindgen::from_value(advances)
                .map_err(|err| JsValue::from_str(&err.to_string()))?;
        let mut renderer = self.renderer.borrow_mut();
        let count = renderer.set_word_oracle_advances(advances);
        Ok(count)
    }

    /// Enables or disables the opt-in "band redraw" fast path (URL param
    /// `band_redraw=1`). Default disabled; when disabled, rendering is
    /// byte-for-byte identical to a build without this feature.
    #[wasm_bindgen(js_name = setBandRedrawEnabled)]
    pub fn set_band_redraw_enabled(&self, enabled: bool) {
        self.renderer.borrow_mut().set_band_redraw_enabled(enabled);
    }

    /// Enables or disables the spanning-paragraph seek fast path. Default
    /// enabled; URL param `span_seek=0` disables it. When disabled, the lazy
    /// visible-window seek is byte-for-byte identical to a build without this
    /// feature (spanning snapshots are recorded but never consumed).
    #[wasm_bindgen(js_name = setSpanSeekEnabled)]
    pub fn set_span_seek_enabled(&self, enabled: bool) {
        self.renderer.borrow_mut().set_span_seek_enabled(enabled);
    }

    /// Enables or disables the layout-extension seek (resuming
    /// `ensureLayoutThroughPage` from the last page-start snapshot). Enabled
    /// by default; URL param `layout_extension_seek=0` disables it, under which
    /// every extension walks from the document start as before.
    #[wasm_bindgen(js_name = setLayoutExtensionSeekEnabled)]
    pub fn set_layout_extension_seek_enabled(&self, enabled: bool) {
        self.renderer
            .borrow_mut()
            .set_layout_extension_seek_enabled(enabled);
    }

    /// Enables the bounded full-resolution tier in the recycled lazy page-slot
    /// pool. This is configured before the first render by the browser runtime.
    #[wasm_bindgen(js_name = setHotPageBackingPoolEnabled)]
    pub fn set_hot_page_backing_pool_enabled(&self, enabled: bool) {
        self.renderer
            .borrow_mut()
            .set_hot_page_backing_pool_enabled(enabled);
    }

    /// Returns paragraph layout cache statistics for debugging/monitoring.
    /// Returns an object with { hits, misses, skippedParagraphs, cachedEntries, totalCapacity, hitRate }.
    #[wasm_bindgen(js_name = paragraphLayoutCacheStats)]
    pub fn paragraph_layout_cache_stats(&self) -> JsValue {
        let renderer = self.renderer.borrow();
        let stats = renderer.paragraph_layout_cache_stats();
        let result = Object::new();
        let _ = Reflect::set(
            &result,
            &JsValue::from_str("hits"),
            &JsValue::from_f64(stats.hits as f64),
        );
        let _ = Reflect::set(
            &result,
            &JsValue::from_str("misses"),
            &JsValue::from_f64(stats.misses as f64),
        );
        let _ = Reflect::set(
            &result,
            &JsValue::from_str("skippedParagraphs"),
            &JsValue::from_f64(stats.skipped_paragraphs as f64),
        );
        let _ = Reflect::set(
            &result,
            &JsValue::from_str("cachedEntries"),
            &JsValue::from_f64(stats.cached_entries as f64),
        );
        let _ = Reflect::set(
            &result,
            &JsValue::from_str("totalCapacity"),
            &JsValue::from_f64(stats.total_capacity as f64),
        );
        let _ = Reflect::set(
            &result,
            &JsValue::from_str("hitRate"),
            &JsValue::from_f64(stats.hit_rate()),
        );
        result.into()
    }

    /// Returns paragraph render cache statistics for debugging/monitoring.
    /// Returns an object with { hits, misses, cachedEntries, totalCapacity, hitRate }.
    #[wasm_bindgen(js_name = paragraphRenderCacheStats)]
    pub fn paragraph_render_cache_stats(&self) -> JsValue {
        let renderer = self.renderer.borrow();
        let stats = renderer.paragraph_render_cache_stats();
        let result = Object::new();
        let _ = Reflect::set(
            &result,
            &JsValue::from_str("hits"),
            &JsValue::from_f64(stats.hits as f64),
        );
        let _ = Reflect::set(
            &result,
            &JsValue::from_str("misses"),
            &JsValue::from_f64(stats.misses as f64),
        );
        let _ = Reflect::set(
            &result,
            &JsValue::from_str("cachedEntries"),
            &JsValue::from_f64(stats.cached_entries as f64),
        );
        let _ = Reflect::set(
            &result,
            &JsValue::from_str("totalCapacity"),
            &JsValue::from_f64(stats.total_capacity as f64),
        );
        let _ = Reflect::set(
            &result,
            &JsValue::from_str("hitRate"),
            &JsValue::from_f64(stats.hit_rate()),
        );
        result.into()
    }

    /// Resets paragraph render cache hit/miss counters (entries are preserved).
    #[wasm_bindgen(js_name = resetParagraphRenderCacheStats)]
    pub fn reset_paragraph_render_cache_stats(&self) {
        let mut renderer = self.renderer.borrow_mut();
        renderer.reset_paragraph_render_cache_stats();
    }

    /// Rebuilds paragraph paint without discarding shared layout results.
    #[wasm_bindgen(js_name = clearParagraphRenderCache)]
    pub fn clear_paragraph_render_cache(&self) {
        self.renderer.borrow_mut().clear_paragraph_render_cache();
    }

    /// Clears the paragraph layout cache.
    #[wasm_bindgen(js_name = clearParagraphLayoutCache)]
    pub fn clear_paragraph_layout_cache(&self) {
        let mut renderer = self.renderer.borrow_mut();
        renderer.clear_paragraph_layout_cache();
    }

    /// Enables or disables incremental layout.
    /// When enabled, the renderer caches paragraph layout results and uses
    /// content hashing to detect changes, potentially skipping unchanged paragraphs.
    #[wasm_bindgen(js_name = setIncrementalLayoutEnabled)]
    pub fn set_incremental_layout_enabled(&self, enabled: bool) {
        let mut renderer = self.renderer.borrow_mut();
        renderer.set_incremental_layout_enabled(enabled);
    }

    /// Returns whether incremental layout is enabled.
    #[wasm_bindgen(js_name = isIncrementalLayoutEnabled)]
    pub fn is_incremental_layout_enabled(&self) -> bool {
        let renderer = self.renderer.borrow();
        renderer.is_incremental_layout_enabled()
    }

    // =========================================================================
    // Dirty Region Tracking API
    // =========================================================================

    /// Returns whether there are any dirty regions.
    #[wasm_bindgen(js_name = hasDirtyRegions)]
    pub fn has_dirty_regions(&self) -> bool {
        let renderer = self.renderer.borrow();
        renderer.has_dirty_regions()
    }

    /// Returns whether a full repaint is needed.
    #[wasm_bindgen(js_name = needsFullRepaint)]
    pub fn needs_full_repaint(&self) -> bool {
        let renderer = self.renderer.borrow();
        renderer.needs_full_repaint()
    }

    /// Returns all dirty regions as an array of objects.
    /// Each object has: { x, y, width, height, page }
    #[wasm_bindgen(js_name = getDirtyRegions)]
    pub fn get_dirty_regions(&self) -> JsValue {
        let renderer = self.renderer.borrow();
        let regions = renderer.dirty_regions();
        let result = Array::new();

        for rect in regions {
            let obj = Object::new();
            let _ = Reflect::set(&obj, &JsValue::from_str("x"), &JsValue::from_f64(rect.x));
            let _ = Reflect::set(&obj, &JsValue::from_str("y"), &JsValue::from_f64(rect.y));
            let _ = Reflect::set(
                &obj,
                &JsValue::from_str("width"),
                &JsValue::from_f64(rect.width),
            );
            let _ = Reflect::set(
                &obj,
                &JsValue::from_str("height"),
                &JsValue::from_f64(rect.height),
            );
            let _ = Reflect::set(
                &obj,
                &JsValue::from_str("page"),
                &JsValue::from_f64(rect.page as f64),
            );
            result.push(&obj);
        }

        result.into()
    }

    /// Clears all dirty regions.
    #[wasm_bindgen(js_name = clearDirtyRegions)]
    pub fn clear_dirty_regions(&self) {
        let renderer = self.renderer.borrow();
        renderer.clear_dirty_regions();
    }

    /// Marks a rectangular region as dirty.
    #[wasm_bindgen(js_name = markDirtyRect)]
    pub fn mark_dirty_rect(&self, x: f64, y: f64, width: f64, height: f64, page: usize) {
        use jade_docx_layout::layout_cache::DirtyRect;
        let renderer = self.renderer.borrow();
        renderer.mark_dirty_rect(DirtyRect::new(x, y, width, height, page));
    }

    /// Marks the entire document for full repaint.
    #[wasm_bindgen(js_name = markFullRepaint)]
    pub fn mark_full_repaint(&self) {
        let renderer = self.renderer.borrow();
        renderer.mark_full_repaint();
    }

    /// Returns the current change ID from the dirty region tracker.
    #[wasm_bindgen(js_name = dirtyChangeId)]
    pub fn dirty_change_id(&self) -> f64 {
        let renderer = self.renderer.borrow();
        renderer.dirty_change_id() as f64
    }

    // =========================================================================
    // Selection Overlay APIs
    // =========================================================================

    /// Returns all selection rectangles for overlay rendering.
    /// Each object has: { x, y, width, height, page }
    #[wasm_bindgen(js_name = getSelectionRects)]
    pub fn get_selection_rects(&self) -> JsValue {
        if self.state.borrow().editing_mode != EditingMode::Body {
            let result = Object::new();
            if self.write_hf_selection_rects(&result).is_err() {
                return Array::new().into();
            }
            return Reflect::get(&result, &JsValue::from_str("selectionRects"))
                .unwrap_or_else(|_| Array::new().into());
        }
        let renderer = self.renderer.borrow();
        let rects = renderer.get_selection_rects();
        let result = Array::new();

        for rect in rects {
            let obj = Object::new();
            let _ = Reflect::set(&obj, &JsValue::from_str("x"), &JsValue::from_f64(rect.x));
            let _ = Reflect::set(&obj, &JsValue::from_str("y"), &JsValue::from_f64(rect.y));
            let _ = Reflect::set(
                &obj,
                &JsValue::from_str("width"),
                &JsValue::from_f64(rect.width),
            );
            let _ = Reflect::set(
                &obj,
                &JsValue::from_str("height"),
                &JsValue::from_f64(rect.height),
            );
            let _ = Reflect::set(
                &obj,
                &JsValue::from_str("page"),
                &JsValue::from_f64(rect.page as f64),
            );
            result.push(&obj);
        }

        result.into()
    }

    /// Updates the selection overlay with current selection state.
    #[wasm_bindgen(js_name = updateSelectionOverlay)]
    pub fn update_selection_overlay(&self) {
        let renderer = self.renderer.borrow();
        renderer.update_selection_overlay();
    }

    /// Returns whether the selection overlay needs re-rendering.
    #[wasm_bindgen(js_name = isSelectionOverlayDirty)]
    pub fn is_selection_overlay_dirty(&self) -> bool {
        let renderer = self.renderer.borrow();
        let overlay = renderer.selection_overlay();
        let is_dirty = overlay.is_dirty();
        drop(overlay);
        is_dirty
    }

    /// Clears the selection overlay dirty flag.
    #[wasm_bindgen(js_name = clearSelectionOverlayDirty)]
    pub fn clear_selection_overlay_dirty(&self) {
        let renderer = self.renderer.borrow();
        renderer.clear_selection_overlay_dirty();
    }

    /// Returns whether JS-side selection overlay is being used.
    #[wasm_bindgen(js_name = useSelectionOverlay)]
    pub fn use_selection_overlay(&self) -> bool {
        let renderer = self.renderer.borrow();
        renderer.use_selection_overlay()
    }

    /// Sets whether to use JS-side selection overlay.
    /// When true, selection highlight is not drawn on the main canvas.
    #[wasm_bindgen(js_name = setUseSelectionOverlay)]
    pub fn set_use_selection_overlay(&self, value: bool) {
        let mut renderer = self.renderer.borrow_mut();
        renderer.set_use_selection_overlay(value);
    }

    /// Sets the range of pages that should be drawn. Pages outside this range
    /// will have their layout computed but drawing commands will be skipped.
    /// Pass start and end page numbers (1-indexed, inclusive).
    #[wasm_bindgen(js_name = setVisiblePageRange)]
    pub fn set_visible_page_range(&self, start: usize, end: usize) {
        let mut renderer = self.renderer.borrow_mut();
        renderer.set_visible_page_range(Some(start..=end));
    }

    /// Clears the visible page range, enabling drawing for all pages.
    #[wasm_bindgen(js_name = clearVisiblePageRange)]
    pub fn clear_visible_page_range(&self) {
        let mut renderer = self.renderer.borrow_mut();
        renderer.clear_visible_page_range();
    }

    /// Sets a hard layout limit in pages (1-indexed, inclusive).
    #[wasm_bindgen(js_name = setLayoutPageLimit)]
    pub fn set_layout_page_limit(&self, limit: usize) {
        let mut renderer = self.renderer.borrow_mut();
        renderer.set_layout_page_limit(Some(limit.max(1)));
    }

    /// Clears the layout page limit (renders full layout).
    #[wasm_bindgen(js_name = clearLayoutPageLimit)]
    pub fn clear_layout_page_limit(&self) {
        let mut renderer = self.renderer.borrow_mut();
        renderer.set_layout_page_limit(None);
    }
}

#[cfg(test)]
mod tests {
    /// No Markup and Original drop one side of each revision from layout, so
    /// a selection's display offsets have to skip those runs before they can
    /// be compared with revision offsets. A boundary next to a hidden run
    /// lands on the outer side for both selection ends.
    #[test]
    fn display_offsets_skip_revisions_the_markup_mode_hides() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/review-kinds.docx"),
        )
        .expect("read review-kinds fixture");
        let doc = futures::executor::block_on(crate::word_document::WordDocument::load(
            &bytes,
            &crate::Options::default(),
        ))
        .expect("load review-kinds fixture");
        // Model: "Base " [0,5) ins "inserted-A" [5,15) " mid " [15,20)
        //        del "deleted-B" [20,29) " end" [29,33)
        let map = |offset, render_changes, render_original, after_hidden| {
            super::display_to_model_position(
                &doc,
                TextPosition::new(0, offset),
                render_changes,
                render_original,
                after_hidden,
            )
            .offset
        };
        // All Markup hides nothing.
        assert_eq!(map(22, true, false, true), 22);
        // No Markup: "Base inserted-A mid  end" — the deletion is gone.
        assert_eq!(map(20, false, false, true), 29);
        assert_eq!(map(20, false, false, false), 20);
        assert_eq!(map(21, false, false, true), 30);
        assert_eq!(map(7, false, false, true), 7);
        assert_eq!(map(24, false, false, true), 33);
        // Original: "Base  mid deleted-B end" — the insertion is gone.
        assert_eq!(map(5, false, true, true), 15);
        assert_eq!(map(5, false, true, false), 5);
        assert_eq!(map(10, false, true, true), 20);
        assert_eq!(map(12, false, true, false), 22);
    }

    /// The selection a review decision leaves behind goes back to display
    /// space through `model_to_display_position`; it has to undo
    /// `display_to_model_position` and clamp past-the-end positions.
    #[test]
    fn model_offsets_map_back_to_display_offsets() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../public/fixtures/review-kinds.docx"),
        )
        .expect("read review-kinds fixture");
        let doc = futures::executor::block_on(crate::word_document::WordDocument::load(
            &bytes,
            &crate::Options::default(),
        ))
        .expect("load review-kinds fixture");
        let back = |paragraph, offset, render_changes, render_original| {
            let position = super::model_to_display_position(
                &doc,
                TextPosition::new(paragraph, offset),
                render_changes,
                render_original,
            );
            (position.paragraph, position.offset)
        };
        assert_eq!(back(0, 22, true, false), (0, 22));
        // No Markup hides del#2 [20, 29): positions inside it collapse to 20.
        assert_eq!(back(0, 24, false, false), (0, 20));
        assert_eq!(back(0, 30, false, false), (0, 21));
        // Original hides ins#1 [5, 15).
        assert_eq!(back(0, 22, false, true), (0, 12));
        for offset in [0, 3, 16, 21, 33] {
            let model = super::display_to_model_position(
                &doc,
                TextPosition::new(0, offset.min(24)),
                false,
                false,
                true,
            );
            assert_eq!(
                back(model.paragraph, model.offset, false, false),
                (0, offset.min(24))
            );
        }
        // Past the end of the document clamps to the end of the last paragraph.
        let last = doc.paragraph_count() - 1;
        let last_len = doc.paragraph_text_len_at(last).unwrap();
        assert_eq!(back(last + 3, 0, true, false), (last, last_len));
        assert_eq!(back(0, 999, true, false), (0, 33));
    }

    #[test]
    fn header_footer_commands_preserve_advanced_character_formats() {
        let clear = super::HeadlessCommand::ClearFormatting {
            start_paragraph: 0,
            start_offset: 0,
            end_paragraph: 0,
            end_offset: 4,
        };
        assert!(
            super::OffscreenDocumentView::hf_run_format_for_command(&clear)
                .unwrap()
                .clear_visual
        );
        let strike = super::HeadlessCommand::ApplyStrike {
            start_paragraph: 0,
            start_offset: 0,
            end_paragraph: 0,
            end_offset: 4,
            enabled: true,
            double: true,
        };
        let format = super::OffscreenDocumentView::hf_run_format_for_command(&strike)
            .expect("double strike is supported in headers and footers");
        assert_eq!(format.strike, Some(false));
        assert_eq!(format.double_strike, Some(true));
        let underline = super::HeadlessCommand::ApplyUnderline {
            start_paragraph: 0,
            start_offset: 0,
            end_paragraph: 0,
            end_offset: 4,
            enabled: true,
            kind: Some("double".into()),
        };
        let format = super::OffscreenDocumentView::hf_run_format_for_command(&underline).unwrap();
        assert_eq!(format.underline_kind.as_deref(), Some("double"));
        for enabled in [true, false] {
            for (command, baseline) in [
                (
                    super::HeadlessCommand::ApplySuperscript {
                        start_paragraph: 0,
                        start_offset: 0,
                        end_paragraph: 0,
                        end_offset: 4,
                        enabled,
                    },
                    "superscript",
                ),
                (
                    super::HeadlessCommand::ApplySubscript {
                        start_paragraph: 0,
                        start_offset: 0,
                        end_paragraph: 0,
                        end_offset: 4,
                        enabled,
                    },
                    "subscript",
                ),
            ] {
                let format =
                    super::OffscreenDocumentView::hf_run_format_for_command(&command).unwrap();
                assert_eq!(
                    format.baseline.as_deref(),
                    Some(if enabled { baseline } else { "baseline" })
                );
            }
        }
    }

    #[test]
    fn page_dimension_runs_preserve_section_boundaries() {
        assert!(super::page_dimension_runs([]).is_empty());
        assert_eq!(
            super::page_dimension_runs([(600.0, 800.0); 1000]),
            vec![(1000, 600.0, 800.0)]
        );
        assert_eq!(
            super::page_dimension_runs([
                (600.0, 800.0),
                (600.0, 800.0),
                (800.0, 600.0),
                (600.0, 800.0)
            ]),
            vec![(2, 600.0, 800.0), (1, 800.0, 600.0), (1, 600.0, 800.0)]
        );
    }

    use super::{
        character_style_options, column_resize_delta, paragraph_style_options,
        selected_grid_columns, selected_grid_rows, set_formatting_marks_preference,
        set_page_export_render_mode, stepped_start_indent_chars, track_index_after_delete,
        valid_optional_paragraph_metrics, valid_paragraph_indents, InlineFormattingState,
        OffscreenDocumentView, TableCellSelection,
    };
    use crate::document::Indent;
    use crate::Options;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};
    use proptest::prelude::*;

    /// Paragraphs `A`, `B` (list items of numId 1), `C` (`w:numId="0"`, so
    /// no list item) and `D` (plain).
    fn list_document() -> crate::word_document::WordDocument {
        use crate::document::{
            Body, BodyElement, Document, Numbering, Paragraph, ParagraphProperties, Run,
        };
        let paragraph = |text: &str, num_id: Option<&str>| Paragraph {
            runs: vec![Run {
                text: text.to_string(),
                properties: None,
                image: None,
                text_box: None,
                math: None,
                field: None,
                hyperlink: None,
                node_id: None,
                text_node_id: None,
                revision_node_id: None,
                comment_reference_ids: Vec::new(),
                footnote_reference_ids: Vec::new(),
                endnote_reference_ids: Vec::new(),
                note_separator: None,
                has_footnote_ref: false,
                has_endnote_ref: false,
                revision: None,
            }],
            properties: num_id.map(|id| ParagraphProperties {
                numbering: Some(Numbering {
                    id: Some(id.to_string()),
                    level: Some(0),
                }),
                ..ParagraphProperties::default()
            }),
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };
        crate::word_document::WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![
                    BodyElement::from_paragraph(paragraph("A", Some("1"))),
                    BodyElement::from_paragraph(paragraph("B", Some("1"))),
                    BodyElement::from_paragraph(paragraph("C", Some("0"))),
                    BodyElement::from_paragraph(paragraph("D", None)),
                ],
            }),
            ..Document::default()
        })
    }

    fn range(anchor: (usize, usize), focus: (usize, usize)) -> SelectionRange {
        SelectionRange::new(
            TextPosition::new(anchor.0, anchor.1),
            TextPosition::new(focus.0, focus.1),
        )
    }

    #[test]
    fn num_id_zero_is_not_a_list_item() {
        // `w:numId="0"` switches numbering off: Backspace, Enter and Tab at
        // its start behave as in body text.
        let document = list_document();
        assert_eq!(
            super::list_item_at_caret(&document, range((0, 0), (0, 0))),
            Some((0, false))
        );
        assert_eq!(
            super::list_item_at_caret(&document, range((2, 0), (2, 0))),
            None
        );
        assert_eq!(
            super::list_item_at_caret(&document, range((3, 0), (3, 0))),
            None
        );
        assert_eq!(
            super::list_item_at_caret(&document, range((0, 1), (0, 1))),
            None
        );
    }

    #[test]
    fn tab_over_selected_list_items_changes_their_levels() {
        // Word's Tab / Shift+Tab with list items selected re-levels them all
        // instead of replacing the selection with a tab.
        let document = list_document();
        let tab = |anchor, focus| super::list_tab_paragraphs(&document, range(anchor, focus));
        assert_eq!(tab((1, 0), (1, 0)), Some((1, 1)));
        assert_eq!(tab((0, 1), (1, 1)), Some((0, 1)));
        assert_eq!(tab((1, 1), (0, 0)), Some((0, 1)));
        assert_eq!(tab((1, 0), (1, 1)), Some((1, 1)));
        // Text inside one item, a selection reaching a non-list paragraph,
        // and a caret in body text keep typing a tab.
        assert_eq!(tab((1, 1), (1, 1)), None);
        assert_eq!(tab((0, 1), (0, 1)), None);
        assert_eq!(tab((1, 1), (2, 1)), None);
        assert_eq!(tab((3, 0), (3, 0)), None);
    }

    #[test]
    fn triple_click_selects_the_paragraph_mark() {
        // Word includes the paragraph mark, so Delete removes the paragraph.
        // The last paragraph of a story has no following paragraph to reach.
        let document = list_document();
        assert_eq!(
            super::paragraph_selection_end(&document, 1, 1),
            TextPosition::new(2, 0)
        );
        assert_eq!(
            super::paragraph_selection_end(&document, 3, 1),
            TextPosition::new(3, 1)
        );
    }

    #[test]
    fn selection_summary_uses_word_ribbon_semantics_for_mixed_runs() {
        use crate::document::{Body, BodyElement, Document, Paragraph, Run, RunProperties};
        use crate::word_document::WordDocument;

        let run = |text: &str, props: Option<RunProperties>| Run {
            text: text.to_string(),
            properties: props,
            image: None,
            text_box: None,
            math: None,
            field: None,
            hyperlink: None,
            node_id: None,
            text_node_id: None,
            revision_node_id: None,
            comment_reference_ids: Vec::new(),
            footnote_reference_ids: Vec::new(),
            endnote_reference_ids: Vec::new(),
            note_separator: None,
            has_footnote_ref: false,
            has_endnote_ref: false,
            revision: None,
        };
        let document = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![
                        run(
                            "He",
                            Some(RunProperties {
                                bold: Some(true),
                                font_size: Some(20.0),
                                color: Some("#FF0000".into()),
                                ..RunProperties::default()
                            }),
                        ),
                        run(
                            "llo",
                            Some(RunProperties {
                                bold: Some(true),
                                italic: Some(true),
                                font_size: Some(12.0),
                                color: Some("#FF0000".into()),
                                ..RunProperties::default()
                            }),
                        ),
                    ],
                    properties: None,
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            ..Document::default()
        });

        let selection = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 5));
        let summary = OffscreenDocumentView::selection_run_summary(&document, &selection)
            .expect("selection covers text");

        assert!(summary.all_bold, "every run is bold");
        assert!(!summary.all_italic, "only part of the selection is italic");
        assert!(
            summary.font_size.is_mixed(),
            "20pt and 12pt runs must read as a mixed size"
        );
        assert!(!summary.color.is_mixed(), "a shared color stays uniform");
        assert_eq!(summary.color.uniform().map(String::as_str), Some("#FF0000"));

        // A collapsed selection has no whole-selection summary.
        let collapsed = SelectionRange::collapsed(TextPosition::new(0, 2));
        assert!(OffscreenDocumentView::selection_run_summary(&document, &collapsed).is_none());
    }

    #[test]
    fn pending_inline_formatting_dies_when_the_caret_moves() {
        let mut state = InlineFormattingState::default();
        state.set_bold(true);
        state.armed_at = Some(TextPosition::new(1, 4));

        let armed = SelectionRange::collapsed(TextPosition::new(1, 4));
        let moved = SelectionRange::collapsed(TextPosition::new(2, 0));
        let ranged = SelectionRange::new(TextPosition::new(1, 4), TextPosition::new(1, 6));

        assert_eq!(
            state
                .pending_run_properties_for(&armed)
                .and_then(|props| props.bold),
            Some(true),
            "pending bold applies while the caret has not moved"
        );
        assert!(
            state.pending_run_properties_for(&moved).is_none(),
            "a caret move discards pending formatting, like Word"
        );
        assert!(
            state.pending_run_properties_for(&ranged).is_none(),
            "a range selection never consumes pending caret formatting"
        );

        // Consumption disarms so a later return to the same position cannot
        // revive stale formatting.
        assert!(state.take_pending_for(&armed).is_some());
        assert!(state.pending_run_properties_for(&armed).is_none());
    }

    #[test]
    fn caret_bold_and_italic_include_complex_script_properties() {
        let mut formatting = InlineFormattingState::default();
        formatting.set_bold(true);
        formatting.set_italic(true);

        let properties = formatting.as_run_properties().expect("inline properties");
        assert_eq!(properties.bold, Some(true));
        assert_eq!(properties.bold_complex, Some(true));
        assert_eq!(properties.italic, Some(true));
        assert_eq!(properties.italic_complex, Some(true));
    }

    #[test]
    fn indent_step_targets_the_logical_start_edge() {
        let indent = Indent {
            left_chars: Some(100),
            right_chars: Some(300),
            ..Indent::default()
        };

        assert_eq!(
            stepped_start_indent_chars(Some(&indent), false, 100),
            (Some(200), None)
        );
        assert_eq!(
            stepped_start_indent_chars(Some(&indent), true, 100),
            (None, Some(400))
        );
        assert_eq!(
            stepped_start_indent_chars(Some(&indent), true, -500),
            (None, Some(0))
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        #[test]
        fn paragraph_metric_validation_accepts_signed_indents_but_not_negative_spacing(
            left in any::<f32>(),
            right in any::<f32>(),
            before in any::<f32>(),
            after in any::<f32>(),
        ) {
            prop_assert_eq!(
                valid_paragraph_indents(left, right),
                left.is_finite() && right.is_finite(),
            );
            prop_assert_eq!(
                valid_optional_paragraph_metrics(Some(left), Some(right), Some(before), Some(after)),
                left.is_finite()
                    && right.is_finite()
                    && before.is_finite()
                    && before >= 0.0
                    && after.is_finite()
                    && after >= 0.0,
            );
            // An omitted field is "leave as is" and never invalid.
            prop_assert!(valid_optional_paragraph_metrics(None, None, None, None));
        }

        #[test]
        fn arbitrary_indent_steps_only_change_the_logical_start_edge(
            left in 0_i32..1_000_000,
            right in 0_i32..1_000_000,
            delta in -1_000_000_i32..1_000_000,
            bidi in any::<bool>(),
        ) {
            let indent = Indent {
                left_chars: Some(left),
                right_chars: Some(right),
                ..Indent::default()
            };
            let (next_left, next_right) = stepped_start_indent_chars(Some(&indent), bidi, delta);
            let current = if bidi { right } else { left };
            let expected = current.saturating_add(delta).max(0);
            if bidi {
                prop_assert_eq!(next_left, None);
                prop_assert_eq!(next_right, Some(expected));
            } else {
                prop_assert_eq!(next_left, Some(expected));
                prop_assert_eq!(next_right, None);
            }
        }

        #[test]
        fn formatting_mark_preference_survives_arbitrary_export_and_toggle_sequences(
            initial in any::<bool>(),
            operations in proptest::collection::vec((any::<bool>(), any::<bool>()), 0..128),
        ) {
            let mut options = Options {
                show_formatting_marks: initial,
                ..Options::default()
            };
            let mut saved = None;
            let mut expected_preference = initial;
            let mut export_active = false;

            for (is_preference_change, value) in operations {
                if is_preference_change {
                    expected_preference = value;
                    let rendered = set_formatting_marks_preference(
                        &mut options,
                        &mut saved,
                        value,
                    );
                    prop_assert_eq!(rendered, if export_active { false } else { value });
                } else {
                    export_active = value;
                    let (marks, crop) = set_page_export_render_mode(
                        &mut options,
                        &mut saved,
                        value,
                    );
                    prop_assert_eq!(marks, if export_active { false } else { expected_preference });
                    prop_assert_eq!(crop, !export_active);
                }

                prop_assert_eq!(saved.is_some(), export_active);
                prop_assert_eq!(
                    saved.unwrap_or(options.show_formatting_marks),
                    expected_preference,
                );
                prop_assert_eq!(
                    options.show_formatting_marks,
                    if export_active { false } else { expected_preference },
                );
            }
        }
    }

    /// The gallery has to come back in the same order every time. The styles
    /// live in a `HashMap`, so without the sort the buttons would move around
    /// between calls — and the outline levels are what put 見出し before body
    /// styles regardless of how the file happens to list them.
    #[test]
    fn paragraph_style_options_order_default_then_headings_then_names() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:styleId="Betsuhyo"><w:name w:val="別表"/></w:style>
  <w:style w:type="paragraph" w:styleId="H2"><w:name w:val="heading 2"/><w:pPr><w:outlineLvl w:val="1"/></w:pPr></w:style>
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
  <w:style w:type="paragraph" w:styleId="H1"><w:name w:val="heading 1"/><w:pPr><w:outlineLvl w:val="0"/></w:pPr></w:style>
  <w:style w:type="paragraph" w:styleId="Honbun"><w:name w:val="本文"/><w:pPr><w:outlineLvl w:val="9"/></w:pPr></w:style>
</w:styles>
"#;
        let styles = jade_docx_reader::styles::StylesPart::from_xml(xml).expect("parse styles");
        let options = paragraph_style_options(&styles);
        let order: Vec<&str> = options.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(
            order,
            vec!["Normal", "heading 1", "heading 2", "別表", "本文"]
        );

        // `w:outlineLvl 9` is body text in the file format; the gallery treats
        // it the same as no level, so 本文 sorts with the non-headings.
        let honbun = options.iter().find(|o| o.name == "本文").expect("本文");
        assert_eq!(honbun.outline_level, None);
        assert!(options[0].is_default, "既定スタイルが先頭に来る");
    }

    #[test]
    fn character_style_options_hide_package_implementation_styles() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:styleId="Heading1">
    <w:name w:val="heading 1"/><w:link w:val="Heading1Char"/>
  </w:style>
  <w:style w:type="character" w:styleId="Heading1Char">
    <w:name w:val="Heading 1 Char"/><w:link w:val="Heading1"/>
  </w:style>
  <w:style w:type="character" w:styleId="Hyperlink"><w:name w:val="Hyperlink"/></w:style>
  <w:style w:type="character" w:styleId="FollowedHyperlink"><w:name w:val="FollowedHyperlink"/></w:style>
  <w:style w:type="character" w:styleId="Default Paragraph Font"><w:name w:val="Default Paragraph Font"/></w:style>
  <w:style w:type="character" w:styleId="Hidden"><w:name w:val="Hidden"/><w:semiHidden/></w:style>
  <w:style w:type="character" w:styleId="VisibleExplicit"><w:name w:val="Visible Explicit"/><w:semiHidden w:val="0"/></w:style>
  <w:style w:type="character" w:styleId="Strong"><w:name w:val="Strong"/></w:style>
</w:styles>
"#;
        let styles = jade_docx_reader::styles::StylesPart::from_xml(xml).expect("parse styles");
        let options = character_style_options(&styles);
        let ids: Vec<&str> = options
            .iter()
            .map(|option| option.style_id.as_str())
            .collect();
        assert_eq!(ids, vec!["Strong", "VisibleExplicit"]);

        assert_eq!(
            styles.resolve_character_style_id("Hyperlink"),
            Some("Hyperlink")
        );
        assert_eq!(
            styles.resolve_character_style_id("Heading 1 Char"),
            Some("Heading1Char")
        );
    }

    #[test]
    fn bidi_visual_column_resize_reverses_physical_pointer_delta() {
        assert_eq!(column_resize_delta(18.0, false), 18.0);
        assert_eq!(column_resize_delta(18.0, true), -18.0);
    }

    #[test]
    fn section_field_placeholder_is_one_header_footer_caret_unit() {
        let text = format!("A{}B", crate::document::SECTION_NUMBER_FIELD_PLACEHOLDER);
        let placeholder_end = 1 + crate::document::SECTION_NUMBER_FIELD_PLACEHOLDER.len();

        // Header/footer editing treats semantic field placeholders atomically.
        // A caret inside `{SECTION}` would let text edits corrupt the field code.
        assert_eq!(
            OffscreenDocumentView::hf_caret_boundaries(&text),
            vec![0, 1, placeholder_end, text.len()]
        );
    }

    #[test]
    fn header_footer_caret_boundaries_keep_graphemes_and_fields_atomic() {
        for unit in [
            "e\u{301}",
            "葛\u{e0100}",
            "🇯🇵",
            "👩🏽‍💻",
            crate::document::PAGE_FIELD_PLACEHOLDER,
            crate::document::PAGE_COUNT_FIELD_PLACEHOLDER,
            crate::document::SECTION_PAGE_COUNT_FIELD_PLACEHOLDER,
            crate::document::SECTION_NUMBER_FIELD_PLACEHOLDER,
        ] {
            let text = format!("A{unit}B");
            assert_eq!(
                OffscreenDocumentView::hf_caret_boundaries(&text),
                vec![0, 1, 1 + unit.len(), text.len()],
                "{unit}"
            );
        }
    }

    #[test]
    fn header_footer_package_targets_keep_story_byte_offsets_and_reject_ambiguous_lines() {
        use super::{EditingMode, HfEditCursor, HfSegment, HfVariant, WordDocument};
        let data = include_bytes!("../../../public/fixtures/hello.docx");
        for mode in [EditingMode::Header, EditingMode::Footer] {
            for variant in [HfVariant::Default, HfVariant::First, HfVariant::Even] {
                let (role, hf_variant) =
                    OffscreenDocumentView::hf_role_variant(mode, variant).unwrap();
                let mut doc =
                    futures::executor::block_on(WordDocument::load(data, &Options::default()))
                        .unwrap();
                doc.ensure_hf_part_for_edit(role, hf_variant).unwrap();
                OffscreenDocumentView::replace_hf_line(&mut doc, mode, variant, 0, "A日本Z")
                    .unwrap();
                doc.set_hf_paragraph_alignment(role, hf_variant, 0, "center")
                    .unwrap();
                let anchor = HfEditCursor {
                    page_index: 2,
                    line_index: 0,
                    segment: HfSegment::Center,
                    offset: 7,
                };
                let focus = HfEditCursor {
                    offset: 1,
                    ..anchor
                };
                let target = OffscreenDocumentView::hf_package_target_for_selection(
                    &doc, mode, variant, anchor, focus,
                )
                .unwrap();
                assert_eq!(target.range.anchor, TextPosition::new(0, 7));
                assert_eq!(target.range.focus, TextPosition::new(0, 1));
                assert_eq!(
                    Some(target.part_path),
                    doc.active_hf_reference(role, hf_variant)
                        .and_then(|id| doc.hf_part_path_for_reference(&id))
                );
                for invalid in [
                    HfEditCursor { offset: 2, ..focus },
                    HfEditCursor {
                        offset: 99,
                        ..focus
                    },
                    HfEditCursor {
                        page_index: 1,
                        ..focus
                    },
                    HfEditCursor {
                        segment: HfSegment::Right,
                        ..focus
                    },
                    HfEditCursor {
                        line_index: 1,
                        ..focus
                    },
                ] {
                    assert!(OffscreenDocumentView::hf_package_target_for_selection(
                        &doc, mode, variant, anchor, invalid
                    )
                    .is_none());
                }
                OffscreenDocumentView::replace_hf_line(&mut doc, mode, variant, 0, "A\tB").unwrap();
                assert!(OffscreenDocumentView::hf_package_target_for_selection(
                    &doc, mode, variant, focus, focus
                )
                .is_none());
            }
        }
        let doc = futures::executor::block_on(WordDocument::load(
            include_bytes!("../../../public/fixtures/header.docx"),
            &Options::default(),
        ))
        .unwrap();
        let cursor = HfEditCursor {
            page_index: 0,
            line_index: 0,
            segment: HfSegment::Left,
            offset: 0,
        };
        assert!(OffscreenDocumentView::hf_package_target_for_selection(
            &doc,
            EditingMode::Header,
            HfVariant::Default,
            cursor,
            cursor
        )
        .is_none());
        assert!(OffscreenDocumentView::hf_package_target_for_selection(
            &doc,
            EditingMode::Body,
            HfVariant::Default,
            cursor,
            cursor
        )
        .is_none());
    }

    #[test]
    fn header_footer_paragraph_merge_targets_follow_surviving_alignment() {
        use super::{EditingMode, HfEditCursor, HfSegment, HfVariant, WordDocument};
        use crate::word_document::{HeaderFooterRole, HeaderFooterVariant};

        let data = include_bytes!("../../../public/fixtures/hello.docx");
        for mode in [EditingMode::Header, EditingMode::Footer] {
            let (role, variant) =
                OffscreenDocumentView::hf_role_variant(mode, HfVariant::Default).unwrap();
            let mut doc =
                futures::executor::block_on(WordDocument::load(data, &Options::default())).unwrap();
            doc.ensure_hf_part_for_edit(role, variant).unwrap();
            OffscreenDocumentView::replace_hf_line(&mut doc, mode, HfVariant::Default, 0, "ABC")
                .unwrap();
            doc.set_hf_paragraph_alignment(role, variant, 0, "center")
                .unwrap();
            doc.split_hf_paragraph_at_display_offset(role, variant, 0, 3, 1)
                .unwrap();
            OffscreenDocumentView::replace_hf_line(&mut doc, mode, HfVariant::Default, 1, "DEF")
                .unwrap();
            doc.set_hf_paragraph_alignment(role, variant, 1, "right")
                .unwrap();
            let previous_end = HfEditCursor {
                page_index: 2,
                line_index: 0,
                segment: HfSegment::Center,
                offset: 3,
            };
            let current_start = HfEditCursor {
                line_index: 1,
                segment: HfSegment::Right,
                offset: 0,
                ..previous_end
            };
            for (cursor, backward) in [(previous_end, false), (current_start, true)] {
                assert_eq!(
                    OffscreenDocumentView::hf_paragraph_merge_target(
                        &doc,
                        mode,
                        HfVariant::Default,
                        cursor,
                        backward,
                    ),
                    Some((1, previous_end))
                );
                for invalid in [
                    HfEditCursor {
                        offset: 1,
                        ..cursor
                    },
                    HfEditCursor {
                        segment: HfSegment::Left,
                        ..cursor
                    },
                ] {
                    assert!(OffscreenDocumentView::hf_paragraph_merge_target(
                        &doc,
                        mode,
                        HfVariant::Default,
                        invalid,
                        backward,
                    )
                    .is_none());
                }
            }
            for (cursor, backward) in [
                (
                    HfEditCursor {
                        offset: 0,
                        ..previous_end
                    },
                    true,
                ),
                (
                    HfEditCursor {
                        offset: 3,
                        ..current_start
                    },
                    false,
                ),
            ] {
                assert!(OffscreenDocumentView::hf_paragraph_merge_target(
                    &doc,
                    mode,
                    HfVariant::Default,
                    cursor,
                    backward,
                )
                .is_none());
            }
            OffscreenDocumentView::replace_hf_line(&mut doc, mode, HfVariant::Default, 1, "D\tEF")
                .unwrap();
            assert!(OffscreenDocumentView::hf_paragraph_merge_target(
                &doc,
                mode,
                HfVariant::Default,
                previous_end,
                false,
            )
            .is_none());
        }

        let doc = futures::executor::block_on(WordDocument::load(
            include_bytes!("../../../public/fixtures/header.docx"),
            &Options::default(),
        ))
        .unwrap();
        assert!(doc
            .hf_plain_paragraph_text(HeaderFooterRole::Header, HeaderFooterVariant::Default, 0,)
            .is_none());
    }

    fn cell(row: usize, column: usize, column_span: usize) -> TableCellSelection {
        TableCellSelection {
            row,
            column,
            column_span,
        }
    }

    /// Word's 行の削除 works on the selected rows, so the ribbon has to know
    /// which grid rows a cell range covers — each one exactly once, in order.
    #[test]
    fn selected_rows_are_ordered_and_deduplicated() {
        let cells = vec![cell(2, 0, 1), cell(2, 1, 1), cell(1, 0, 1), cell(1, 1, 1)];
        assert_eq!(selected_grid_rows(&cells), vec![1, 2]);
    }

    /// A merged cell occupies every grid slot of its `gridSpan`; deleting the
    /// selected columns has to remove all of them.
    #[test]
    fn selected_columns_expand_merged_cells() {
        let cells = vec![cell(0, 0, 2), cell(1, 0, 1), cell(1, 1, 1)];
        assert_eq!(selected_grid_columns(&cells), vec![0, 1]);
        assert_eq!(selected_grid_columns(&[cell(0, 3, 3)]), vec![3, 4, 5]);
    }

    /// After 行の削除 Word leaves the caret in the row that moved into the
    /// deleted slot; deleting the last row(s) leaves it in the new last row.
    #[test]
    fn caret_row_after_delete_follows_word() {
        assert_eq!(track_index_after_delete(&[1], 4), Some(1));
        assert_eq!(track_index_after_delete(&[1, 2], 4), Some(1));
        assert_eq!(track_index_after_delete(&[3], 4), Some(2));
        assert_eq!(track_index_after_delete(&[2, 3], 4), Some(1));
        assert_eq!(track_index_after_delete(&[0], 1), None);
        assert_eq!(track_index_after_delete(&[0, 1, 2], 3), None);
    }
}
