//! A DOCX open for editing and painting, with no browser.
//!
//! Wraps the three things that have to move together — the document model, the
//! renderer, and the page surfaces — and owns the repaint sequence that makes
//! an edit cheap. That sequence is not obvious and is easy to get wrong in ways
//! that still produce a picture, so it lives here rather than in each caller.

use crate::{FontEnvironment, SkiaRenderHost};
use futures::executor::block_on;
pub use jade_docx_editor::canvas_renderer::CaretAffinity;
use jade_docx_editor::canvas_renderer::{CanvasRenderer, FieldShading, FullLayoutDebugSnapshot};
use jade_docx_editor::commands::transform_object::TransformObjectCommand;
use jade_docx_editor::commands::{
    Command, CommandContext, CommandError, CommandExecutor, HistoryRenderFootprint,
};
use jade_docx_editor::document::BodyElement;
use jade_docx_editor::field_datetime::FieldDateTime;
use jade_docx_editor::fields::{field_code_display, FieldCodeView};
use jade_docx_editor::headless_api::{
    apply_headless_commands_with_executor_to_document,
    apply_headless_commands_with_resize_measurement, execute_headless_queries_on_document,
};
use jade_docx_editor::word_document::{
    FloatingObjectKind, FloatingObjectTarget, FloatingObjectTransform, WordDocument,
};
use jade_docx_editor::Options;
use jade_docx_headless_api::{
    HeadlessBorderEdge, HeadlessCommand, HeadlessDeleteDirection, HeadlessEditRequest,
    HeadlessQuery, HeadlessQueryRequest, HeadlessQueryResult, HeadlessRunStyle,
};
use jade_docx_layout::line_metrics::{word_layout_platform, WordLayoutPlatform};
use jade_selection::{SelectionRange, TextPosition};
use jade_skia_painter::SkiaPainter;
use std::borrow::Cow;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// The renderer's form of a field clock reading.
fn renderer_datetime(datetime: FieldDateTime) -> jade_docx_editor::canvas_renderer::FieldDateTime {
    jade_docx_editor::canvas_renderer::FieldDateTime {
        year: datetime.year,
        month: datetime.month,
        day: datetime.day,
        hour: datetime.hour,
        minute: datetime.minute,
        second: datetime.second,
        weekday: datetime.weekday,
    }
}

/// A face to register, as a family name and a font file.
pub struct FaceSpec<'a> {
    pub family: &'a str,
    pub weight: u16,
    pub path: &'a Path,
}

/// A face to register under a family name: an upright or italic face, from a
/// font file or one face of a collection (`.ttc`) — what a document's own
/// fonts, found installed, are registered as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFace {
    pub family: String,
    pub weight: u16,
    pub italic: bool,
    pub path: PathBuf,
    pub index: u32,
}

impl From<&FaceSpec<'_>> for FontFace {
    fn from(face: &FaceSpec<'_>) -> Self {
        Self {
            family: face.family.to_string(),
            weight: face.weight,
            italic: false,
            path: face.path.to_path_buf(),
            index: 0,
        }
    }
}

/// Registers `faces`, passing over one that cannot be read — a damaged or
/// unreadable installed font must not keep the document from opening — and
/// failing only when none could be.
fn register_faces(host: &mut SkiaRenderHost, faces: &[FontFace]) -> Result<(), String> {
    let mut first_error = None;
    let mut registered = 0;
    for face in faces {
        match host.register_font_styled(
            &face.family,
            face.weight,
            face.italic,
            &face.path,
            face.index,
        ) {
            Ok(()) => registered += 1,
            Err(error) => {
                eprintln!("font {} skipped: {error}", face.family);
                first_error.get_or_insert(error);
            }
        }
    }
    match first_error {
        Some(error) if registered == 0 => Err(error),
        _ => Ok(()),
    }
}

/// How a PDF represents the editor's painted page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PdfRenderingMode {
    /// Preserve live raster coverage and colors, with searchable Unicode text.
    #[default]
    Editor,
    /// Keep scalable vector text and paths; reader antialiasing can differ.
    Vector,
}

/// Native PDF export controls. Page geometry always comes from shared layout.
#[derive(Clone, Copy, Debug)]
pub struct PdfExportOptions {
    pub rendering: PdfRenderingMode,
    /// Pixels per CSS pixel for editor appearance mode, from 0.5 through 4.
    pub scale: f64,
    /// Whether tracked changes and comments print (Word's 変更履歴/コメント
    /// の印刷): `Some(false)` prints the document as it reads with every
    /// change accepted and no comments; `None` prints what the view shows.
    pub markup: Option<bool>,
    /// A tagged PDF: its structure tree — headings, paragraphs, tables,
    /// figures with their alternative text — in reading order, bookmarks
    /// from the headings and the document's title. Tagged pages are vector.
    pub tagged: bool,
}

impl Default for PdfExportOptions {
    fn default() -> Self {
        Self {
            rendering: PdfRenderingMode::Editor,
            scale: 1.0,
            markup: None,
            tagged: false,
        }
    }
}

/// Header/footer package and model state, including rich story XML.
#[derive(Clone)]
struct HeaderFooterEdit {
    previous: jade_docx_editor::word_document::WordDocumentSnapshot,
}

/// Which undo stack a step is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HistoryStep {
    /// A command on the executor's stack, and how many of the executor's own
    /// entries it takes. Usually one, but replace-all runs an insert per match:
    /// one user action, so one undo has to take all of them back.
    Command {
        executor_steps: usize,
    },
    HeaderFooter,
}

/// A floating object (image or shape) and where it sits on the page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObjectHit {
    /// Which sort of floating object it is, which the transform command needs
    /// to find it again in the document.
    pub kind: FloatingObjectKind,
    pub paragraph: usize,
    pub run: usize,
    pub page: usize,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Where the caret is inside a table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableCell {
    pub table_order_index: usize,
    pub row: usize,
    pub column: usize,
    pub rows: usize,
    pub columns: usize,
}

/// Where a search hit is, as byte offsets into its paragraph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub paragraph: usize,
    pub start_offset: usize,
    pub end_offset: usize,
}

/// A character format that can be applied across a selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharacterFormat {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Superscript,
    Subscript,
    /// Back to the paragraph style's own formatting.
    Clear,
}

/// Where the caret sits, in the renderer's document coordinate space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaretRect {
    pub x: f64,
    /// Distance from the top of the *document*, so a caller that stacks pages
    /// can work out which page it is on.
    pub document_y: f64,
    pub width: f64,
    pub height: f64,
}

pub struct NativeDocument {
    /// The page box the host and the renderer were built to: see
    /// [`Self::follow_page_box`].
    page_width: f64,
    page_height: f64,
    /// Edit history. Edits go through this so they can be undone; the
    /// executor records the inverse of each command as it applies it.
    executor: CommandExecutor,
    /// Header and footer edits, newest last.
    ///
    /// They bypass `CommandExecutor` — the headless API applies them straight
    /// to the document part — so they leave nothing on its undo stack. The
    /// browser runtime keeps a separate stack for exactly this reason; this
    /// mirrors it, and `history` records which stack each step came from so
    /// undo pops them in the order they happened.
    header_footer_undo: Vec<HeaderFooterEdit>,
    header_footer_redo: Vec<HeaderFooterEdit>,
    /// Which stack each undoable step is on, newest last.
    history: Vec<HistoryStep>,
    /// Steps taken off `history` by undo, so redo can group the same way.
    redo_history: Vec<HistoryStep>,
    /// The selection, which is also the caret when it is collapsed. Held here
    /// rather than in the shell because commands read and update it.
    selection: Option<SelectionRange>,
    word_document: WordDocument,
    renderer: CanvasRenderer<SkiaPainter>,
    pages: Rc<RefCell<Vec<SkiaPainter>>>,
    /// How many of those surfaces the last render used. See
    /// [`SkiaRenderHost::rendered_pages`].
    rendered_pages: Rc<std::cell::Cell<usize>>,
    /// Set to `false` around a bounded repaint, so it does not report the pages
    /// it stopped at as the document's page count.
    pass_covers_whole_document: Rc<std::cell::Cell<bool>>,
    /// See [`Self::set_dark_paper`].
    dark_paper: Rc<std::cell::Cell<bool>>,
    options: Options,
    fonts: crate::FontEnvironment,
    /// The clock DATE and TIME show; `None` shows their saved results.
    field_datetime: Option<FieldDateTime>,
    /// Which fields show their codes (⌥F9, ⇧F9).
    field_codes: FieldCodeView,
    /// フィールドの網かけ, on screen only.
    field_shading: FieldShading,
    /// A view's page box in place of the sections' (Web レイアウト, 閲覧
    /// モード); kept here so a renderer built again keeps it.
    view_geometry: Option<jade_docx_editor::canvas_renderer::ViewGeometry>,
    /// アウトライン's indent for each paragraph, while it shows.
    outline_indents: Option<Rc<std::collections::HashMap<usize, f64>>>,
    /// Configured and fallback math faces, retained across renderer rebuilds.
    math_faces: Vec<FontFace>,
    /// The pages (0-based) a whole-document repaint draws at once: the ones a
    /// shell shows. `None` draws every page. See [`Self::set_paint_window`].
    paint_window: Option<std::ops::Range<usize>>,
    /// Pages laid out since they were last drawn: their surfaces hold what
    /// they showed before. [`Self::paint_stale_pages`] draws them.
    stale_pages: std::collections::BTreeSet<usize>,
    /// Whether a bounded pass (an edit, a selection) has run since the last
    /// whole-document layout. While it has, [`Self::paint_stale_pages`] lays
    /// the document out from the start rather than resuming at a page.
    bounded_pass_since_full_layout: bool,
    /// The round-trip tree was built after the layout the pages show, which
    /// has no node ids for tables and links: the next whole repaint gives
    /// them theirs, and a bounded edit runs one first.
    roundtrip_relayout_pending: bool,
    /// Why the round-trip tree could not be built, kept so it is not
    /// parsed again on every edit.
    roundtrip_xml_error: Option<String>,
}

/// How far before the first stale page [`NativeDocument::paint_stale_pages`]
/// looks for a page it can resume layout at, before it lays the document out
/// from the start instead.
const SEEK_BACK_PAGES: usize = 8;

/// Where a paragraph's lines begin and end, and how many there are: what a
/// one-paragraph edit has to keep for every later paragraph to stay put.
#[derive(Clone, Copy, PartialEq, Debug)]
struct ParagraphExtent {
    lines: usize,
    top: f64,
    bottom: f64,
}

thread_local! {
    static SURFACE_DOCUMENT_RENDERS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn count_surface_document_render() {
    SURFACE_DOCUMENT_RENDERS.with(|renders| renders.set(renders.get() + 1));
}

/// How many whole-document renders into a [`NativeDocument`]'s page surfaces
/// have run on this thread: the one at open, every full repaint and every
/// rebuild for a new page box. Bounded edit repaints, PDF export and print
/// are not counted. A diagnostic for tests that pin how much work an
/// operation does.
#[doc(hidden)]
pub fn surface_document_renders() -> usize {
    SURFACE_DOCUMENT_RENDERS.with(std::cell::Cell::get)
}

thread_local! {
    static ROUNDTRIP_XML_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many times a [`NativeDocument`] on this thread has built the
/// round-trip XML tree that saving patches, after opening. Opening builds
/// none (a document whose first paint needs it is loaded with it,
/// uncounted), nor does export; the first edit or an explicit
/// [`NativeDocument::ensure_roundtrip_xml`] builds one. A diagnostic for
/// tests that pin how much work an operation does.
#[doc(hidden)]
pub fn roundtrip_xml_builds() -> usize {
    ROUNDTRIP_XML_BUILDS.with(std::cell::Cell::get)
}

/// Binds only the selected math faces, so each family uses its own MATH
/// table and outlines instead of the renderer's fallback constants.
fn register_math_faces(renderer: &mut CanvasRenderer<SkiaPainter>, faces: &[FontFace]) {
    for face in faces {
        let Ok(bytes) = std::fs::read(&face.path) else {
            continue;
        };
        let _ = renderer.register_math_face(face.family.clone(), bytes, face.index);
    }
}

fn math_faces_of(faces: &[FontFace], word_document: &WordDocument) -> Vec<FontFace> {
    crate::system_fonts::math_faces_for(
        faces,
        word_document
            .document
            .as_ref()
            .and_then(|document| document.math_properties.math_font.as_deref()),
    )
}

impl NativeDocument {
    /// Opens `docx_bytes` and paints it once.
    ///
    /// `faces` are registered with both the painter and the measurer, which
    /// have to agree: one decides where lines break, the other where glyphs
    /// land.
    /// `faces` should include one covering the marks Word uses for line and
    /// page breaks (U+21B5, U+21D3). A Japanese text face carries the pilcrow
    /// but not those, and the painter's per-character fallback can only reach a
    /// face that was registered.
    pub fn open(
        docx_bytes: &[u8],
        page_width: f64,
        page_height: f64,
        faces: &[FaceSpec<'_>],
    ) -> Result<Self, String> {
        let faces: Vec<FontFace> = faces.iter().map(FontFace::from).collect();
        Self::open_with_font_faces(docx_bytes, page_width, page_height, &faces)
    }

    /// [`Self::open`] with faces that may be italic or in a collection.
    pub fn open_with_font_faces(
        docx_bytes: &[u8],
        page_width: f64,
        page_height: f64,
        faces: &[FontFace],
    ) -> Result<Self, String> {
        Self::open_drawing(docx_bytes, page_width, page_height, faces, None)
    }

    /// [`Self::open`] for a shell that shows `window` (0-based pages) first:
    /// every page is laid out, but only those are drawn before this returns.
    /// The rest are left stale for [`Self::paint_stale_pages`], and later
    /// whole-document repaints keep to the same window (see
    /// [`Self::set_paint_window`]). An empty window draws every page, as
    /// [`Self::open`] does.
    pub fn open_with_paint_window(
        docx_bytes: &[u8],
        page_width: f64,
        page_height: f64,
        faces: &[FaceSpec<'_>],
        window: std::ops::Range<usize>,
    ) -> Result<Self, String> {
        let faces: Vec<FontFace> = faces.iter().map(FontFace::from).collect();
        Self::open_with_font_faces_and_paint_window(
            docx_bytes,
            page_width,
            page_height,
            &faces,
            window,
        )
    }

    /// [`Self::open_with_paint_window`] with faces that may be italic or in a
    /// collection.
    pub fn open_with_font_faces_and_paint_window(
        docx_bytes: &[u8],
        page_width: f64,
        page_height: f64,
        faces: &[FontFace],
        window: std::ops::Range<usize>,
    ) -> Result<Self, String> {
        Self::open_drawing(docx_bytes, page_width, page_height, faces, Some(window))
    }

    /// Opens the document, lays every page out once and draws those in
    /// `window` (every page when `None` or empty).
    fn open_drawing(
        docx_bytes: &[u8],
        page_width: f64,
        page_height: f64,
        faces: &[FontFace],
        window: Option<std::ops::Range<usize>>,
    ) -> Result<Self, String> {
        let window = window.filter(|window| !window.is_empty());
        let mut options = Options::default();

        // The model alone, as the browser opens: the round-trip XML tree that
        // saving patches is a second pass over `document.xml` the first paint
        // does not need. The shell builds it in idle time, and every edit
        // builds it first if it comes sooner (see `Self::ensure_roundtrip_xml`).
        let word_document = block_on(WordDocument::load_for_initial_render(docx_bytes, &options))
            .map_err(|err| format!("parse failed: {err}"))?;
        let Some(document) = word_document.document.as_ref() else {
            return Err("no main document part".to_string());
        };
        // Tracked changes as Word shows them: with their markup — insertions
        // coloured and underlined, deletions struck through — unless the
        // document was saved with a view that leaves them out.
        options.set_render_changes(document.display_content_revisions.unwrap_or(true));
        // The model's own snapshot, shared rather than copied: the renderer
        // keeps it as its last document, and identity-keyed caches then hit.
        let document = Rc::clone(document);

        // Section properties decide the page box, and it is not necessarily the
        // one the caller asked for — this fixture is A4 while the shell asks
        // for Letter. A host sized to the wrong box draws each page short and
        // maps every page after the first to the wrong place, so build to
        // what the document wants. Layout ends on the last section's box, read
        // here from the section properties without laying anything out.
        let (page_width, page_height) =
            Self::layout_page_size_of(&document).unwrap_or((page_width, page_height));

        let mut host = SkiaRenderHost::new(page_width, page_height)
            .ok_or_else(|| "could not allocate the raster surface".to_string())?;
        register_faces(&mut host, faces)?;
        let fonts = host.font_environment();
        let math_faces = math_faces_of(faces, &word_document);
        let pages = host.pages();
        let rendered_pages = host.rendered_pages();
        let pass_covers_whole_document = host.pass_covers_whole_document();
        let dark_paper = host.dark_paper();
        let mut renderer = CanvasRenderer::<SkiaPainter>::with_host(Box::new(host), false)
            .map_err(|_| "renderer setup failed".to_string())?;
        register_math_faces(&mut renderer, &math_faces);
        renderer.sync_word_document(&word_document);

        // The renderer defaults to letting a JS overlay draw the selection,
        // which leaves a native host with no highlight at all. There is no
        // overlay here; the renderer paints it.
        renderer.set_use_selection_overlay(false);
        // Declared up front: without it the first render on a fresh renderer
        // leaves the first paragraph of every page after the first unpainted —
        // a later full repaint draws it, so the content is laid out, just not
        // drawn. Costs nothing here because everything is dirty anyway.
        renderer.mark_full_repaint();
        if let Some(window) = &window {
            renderer.set_visible_page_range(Some(window.start + 1..=window.end));
        }
        count_surface_document_render();
        let result = renderer.render_with_numbering_rc(
            document,
            &options,
            word_document.numbering_part.as_ref(),
            word_document.header_lines.as_deref(),
            word_document.footer_lines.as_deref(),
            Some(&word_document.comments),
        );
        renderer.clear_visible_page_range();
        result.map_err(|_| "render failed".to_string())?;
        if let Some(error) = renderer.take_render_error() {
            return Err(error);
        }
        renderer.clear_dirty_regions();
        let page_count = rendered_pages.get();
        let stale_pages = match &window {
            Some(window) => (0..page_count)
                .filter(|page| !window.contains(page))
                .collect(),
            None => std::collections::BTreeSet::new(),
        };

        let mut opened = Self {
            // The box the document asked for, not the one the caller guessed.
            page_width,
            page_height,
            executor: CommandExecutor::default(),
            header_footer_undo: Vec::new(),
            header_footer_redo: Vec::new(),
            history: Vec::new(),
            redo_history: Vec::new(),
            selection: None,
            word_document,
            renderer,
            pages,
            rendered_pages,
            pass_covers_whole_document,
            dark_paper,
            options,
            fonts,
            field_datetime: None,
            field_codes: FieldCodeView::default(),
            field_shading: FieldShading::Never,
            math_faces,
            view_geometry: None,
            outline_indents: None,
            paint_window: window,
            stale_pages,
            bounded_pass_since_full_layout: false,
            roundtrip_relayout_pending: false,
            roundtrip_xml_error: None,
        };
        // Should layout end on a box the section properties did not predict,
        // the host is built again to it, drawing the same window.
        let window = opened.paint_window.clone();
        opened.follow_page_box(window)?;
        opened.redraw_wrong_page_totals()?;
        Ok(opened)
    }

    /// Draws the paint window once more when the pass just run drew a
    /// NUMPAGES or SECTIONPAGES total other than the one it ended with, as
    /// the first pass over a document does: a footer on page 1 is drawn
    /// before the last page is reached. The renderer shows the settled
    /// totals the second time.
    ///
    /// So it does when the pass drew a header's or footer's STYLEREF on a
    /// page of the window other than the page's own headings show: a header
    /// is drawn before the body below it is laid out, and the first pass has
    /// no headings yet. Wrong pages outside the window are left stale.
    fn redraw_wrong_page_totals(&mut self) -> Result<(), String> {
        let window = self.paint_window.clone();
        let mut style_refs_drawn_wrong = false;
        if self.renderer.has_style_reference_wrong_pages() {
            let wrong = self.renderer.take_style_reference_wrong_pages();
            style_refs_drawn_wrong = wrong
                .iter()
                .any(|page| window.as_ref().is_none_or(|window| window.contains(page)));
            if !style_refs_drawn_wrong {
                self.mark_pages_stale(wrong);
            }
        }
        if self.renderer.take_page_totals_changed() || style_refs_drawn_wrong {
            self.repaint_all_drawing(window, false)?;
        }
        Ok(())
    }

    /// Leaves stale the pages whose STYLEREF the pass just run drew other
    /// than the headings it laid out show, so [`Self::paint_stale_pages`]
    /// draws them from those headings.
    #[inline]
    fn mark_wrong_style_references_stale(&mut self) {
        if self.renderer.has_style_reference_wrong_pages() {
            let wrong = self.renderer.take_style_reference_wrong_pages();
            self.mark_pages_stale(wrong);
        }
    }

    #[cold]
    fn mark_pages_stale(&mut self, pages: std::collections::BTreeSet<usize>) {
        let count = self.page_count();
        self.stale_pages
            .extend(pages.into_iter().filter(|&page| page < count));
    }

    /// Leaves every page stale when the pass just run drew a NUMPAGES or
    /// SECTIONPAGES total other than the one it ended with — an edit that
    /// changed the page count, a page box that rebuilt the renderer — so
    /// [`Self::paint_stale_pages`] draws them with the settled totals.
    fn mark_wrong_page_totals_stale(&mut self) {
        if self.renderer.take_page_totals_changed() {
            self.stale_pages.extend(0..self.page_count());
        }
        self.mark_wrong_style_references_stale();
    }

    /// Limits what a whole-document repaint draws to `window` (0-based
    /// pages): every page is still laid out, but only these are drawn, and
    /// the rest are left stale for [`Self::paint_stale_pages`]. A paragraph
    /// split or an undo near the start of a long document re-flows every page
    /// after it, and drawing all of them is most of what it costs; a shell
    /// shows a few. `None` (the default) draws every page, as before.
    pub fn set_paint_window(&mut self, window: Option<std::ops::Range<usize>>) {
        self.paint_window = window.filter(|window| !window.is_empty());
    }

    /// Whether page `page_index` was laid out since it was last drawn, so its
    /// pixels are out of date: the old pixels after a re-flow, or blank for a
    /// page that has never been drawn (outside the window at open, or after
    /// the surfaces were built again for a new page box).
    pub fn is_page_stale(&self, page_index: usize) -> bool {
        self.stale_pages.contains(&page_index)
    }

    /// Whether any page waits to be drawn.
    pub fn has_stale_pages(&self) -> bool {
        self.stale_pages
            .first()
            .is_some_and(|&page| page < self.page_count())
    }

    /// Draws the stale pages among `pages` (0-based) — all of them in one pass
    /// over the span from the first to the last. Returns whether any was
    /// drawn. A shell calls it for the pages it is about to show, and in idle
    /// time for the rest.
    pub fn paint_stale_pages(&mut self, pages: std::ops::Range<usize>) -> Result<bool, String> {
        let count = self.page_count();
        self.stale_pages.retain(|&page| page < count);
        let mut stale = self.stale_pages.range(pages.start..pages.end.min(count));
        let (Some(&first), last) = (stale.next(), stale.next_back()) else {
            return Ok(false);
        };
        let last = last.copied().unwrap_or(first);
        if self.paint_stale_span_from_page_start(first, last)? {
            return Ok(true);
        }
        // Nothing changed since the pass that left them stale, so the pages
        // outside this span keep pixels that are still right.
        self.repaint_all_drawing(Some(first..last + 1), false)?;
        self.bounded_pass_since_full_layout = false;
        // A bounded edit may have left the document longer than the count
        // this span was cut from. The relayout found the pages it added;
        // nothing has drawn them, and those asked for are drawn now.
        if self.mark_added_pages_stale(count) {
            self.paint_stale_pages(pages)?;
        }
        Ok(true)
    }

    /// Marks the pages after `count_before` stale, and returns whether there
    /// were any: a relayout that lengthened the document drew only the span
    /// it was asked for.
    fn mark_added_pages_stale(&mut self, count_before: usize) -> bool {
        let count = self.page_count();
        self.stale_pages.extend(count_before..count);
        count > count_before
    }

    /// Draws pages `first..=last` (0-based) by resuming layout at the start of
    /// `first` and stopping after `last`, so the cost follows the pages drawn
    /// rather than the document. Nothing changed since the pass that left
    /// them stale: the caches that pass filled still hold, and pagination is
    /// kept as it is. Returns `false`, having done nothing, when the renderer
    /// has no page-start snapshot it can seek to at `first` or a few pages
    /// before (page 0, a document whose notes are shown, snapshots that no
    /// longer fit); the caller then lays the document out from the start.
    fn paint_stale_span_from_page_start(
        &mut self,
        first: usize,
        last: usize,
    ) -> Result<bool, String> {
        // A page that starts inside a table or a paragraph carried over from
        // the page before may have no snapshot of its own. The nearest earlier
        // page that has one does as well: the pages between are laid out
        // again but, outside the visible range, not drawn.
        // A bounded pass since the last whole layout (an edit, a selection)
        // stops at its bound: the snapshots, the line registry hit-testing
        // reads and the regions after it may belong to a layout that no
        // longer exists, and only a whole layout rebuilds them. So may a
        // layout from before the round-trip tree gave the model its node ids.
        if self.bounded_pass_since_full_layout || self.roundtrip_relayout_pending {
            return Ok(false);
        }
        // With a selection showing, a seek drew some highlighted pages
        // differently from a whole layout (a selection running to the end of
        // a long document); the whole layout draws them as a plain open does.
        if self
            .selection
            .is_some_and(|selection| !selection.is_collapsed())
        {
            return Ok(false);
        }
        // Fetched before anything is prepared, so an early return leaves the
        // renderer as it was.
        let document = Rc::clone(
            self.word_document
                .document
                .as_ref()
                .ok_or_else(|| "document went missing".to_string())?,
        );
        if !(1..=first)
            .rev()
            .take(SEEK_BACK_PAGES)
            .any(|page| self.renderer.try_prepare_rerender_for_page(page))
        {
            return Ok(false);
        }
        let pages_before = self.page_count();
        // The pass stops after `last`, so its last page is not the
        // document's page count.
        self.pass_covers_whole_document.set(false);
        self.renderer
            .set_visible_page_range(Some(first + 1..=last + 1));
        let previous_limit = self.renderer.layout_page_limit();
        self.renderer.set_layout_page_limit(Some(last + 1));
        let result = self.renderer.render_with_numbering_rc(
            document,
            &self.options,
            self.word_document.numbering_part.as_ref(),
            self.word_document.header_lines.as_deref(),
            self.word_document.footer_lines.as_deref(),
            Some(&self.word_document.comments),
        );
        self.renderer.set_layout_page_limit(previous_limit);
        self.renderer.clear_visible_page_range();
        self.pass_covers_whole_document.set(true);
        result.map_err(|_| "repaint failed".to_string())?;
        if let Some(error) = self.renderer.take_render_error() {
            return Err(error);
        }
        debug_assert_eq!(self.page_count(), pages_before, "pagination is kept");
        for page in first..=last {
            self.stale_pages.remove(&page);
        }
        self.mark_wrong_style_references_stale();
        self.renderer.clear_dirty_regions();
        Ok(true)
    }

    /// Serialises the document back to a .docx.
    ///
    /// Round-trips through jade's writer, so parts the edit never touched keep
    /// their original XML — tracked changes, comments and field codes survive.
    pub fn export_docx(&self) -> Result<Vec<u8>, String> {
        let grids = if Self::has_windows_nested_body_tables(&self.word_document) {
            if !self.can_reuse_table_layout() {
                return self.export_docx_with_fresh_autofit_grids();
            }
            self.renderer.autofit_grids_for_docx()
        } else {
            Vec::new()
        };
        self.word_document
            .export_docx_with_table_grids(&grids)
            .map_err(|err| format!("export failed: {err:?}"))
    }

    fn has_windows_nested_body_tables(word_document: &WordDocument) -> bool {
        word_layout_platform() == WordLayoutPlatform::Windows
            && word_document
                .document
                .as_ref()
                .and_then(|document| document.body.as_ref())
                .is_some_and(|body| {
                    body.elements.iter().any(|element| {
                        let BodyElement::Table(table) = element else {
                            return false;
                        };
                        table.rows.iter().flat_map(|row| &row.cells).any(|cell| {
                            cell.blocks
                                .iter()
                                .any(|block| matches!(block, BodyElement::Table(_)))
                        })
                    })
                })
    }

    fn can_reuse_table_layout(&self) -> bool {
        self.word_document.raw_document.is_some()
            && self.renderer.last_layout_complete()
            && !self.bounded_pass_since_full_layout
            && !self.roundtrip_relayout_pending
            && !self.options.render_original
            && self.options.collapsed_blocks().is_empty()
    }

    fn export_docx_with_fresh_autofit_grids(&self) -> Result<Vec<u8>, String> {
        // Layout may be bounded to the edited pages, or the opening pass
        // may still carry no round-trip IDs. Keep this save pass independent
        // of the live document, renderer and history.
        let mut document = Cow::Borrowed(&self.word_document);
        if document.raw_document.is_none() {
            document
                .to_mut()
                .ensure_roundtrip_xml()
                .map_err(|error| format!("round-trip XML failed: {error}"))?;
        }
        let renderer = Self::measure_tables(
            &document,
            (self.page_width, self.page_height),
            &self.fonts,
            &self.math_faces,
            self.field_datetime,
            &self.options,
        )?;
        document
            .export_docx_with_table_grids(&renderer.autofit_grids_for_docx())
            .map_err(|err| format!("export failed: {err:?}"))
    }

    fn measure_tables(
        document: &WordDocument,
        page_size: (f64, f64),
        fonts: &FontEnvironment,
        math_faces: &[FontFace],
        field_datetime: Option<FieldDateTime>,
        options: &Options,
    ) -> Result<CanvasRenderer<SkiaPainter>, String> {
        // Use the current model and fonts without drawing or sharing mutable
        // layout state with the live view.
        let host = SkiaRenderHost::with_fonts(page_size.0, page_size.1, fonts, 1.0)
            .ok_or_else(|| "could not allocate the table measurement surface".to_string())?;
        let mut renderer = CanvasRenderer::<SkiaPainter>::with_host(Box::new(host), false)
            .map_err(|_| "table measurement setup failed".to_string())?;
        register_math_faces(&mut renderer, math_faces);
        renderer.sync_word_document(document);
        renderer.set_current_datetime(field_datetime.map(renderer_datetime));
        renderer.set_visible_page_range(Some(0..=0));
        let mut options = options.clone();
        options.render_original = false;
        options.set_collapsed_blocks(Vec::new());
        renderer
            .render_with_numbering_rc(
                document
                    .document
                    .clone()
                    .ok_or_else(|| "no document".to_string())?,
                &options,
                document.numbering_part.as_ref(),
                document.header_lines.as_deref(),
                document.footer_lines.as_deref(),
                Some(&document.comments),
            )
            .map_err(|_| "table measurement layout failed".to_string())?;
        if let Some(error) = renderer.take_render_error() {
            return Err(error);
        }
        Ok(renderer)
    }

    /// Exports the live document through the editor's painter and shared layout.
    /// The independent recording host preserves editing pixels, selection and
    /// history. The default preserves live raster appearance and searchable text.
    pub fn export_pdf(&self) -> Result<Vec<u8>, String> {
        self.export_pdf_with_options(PdfExportOptions::default())
    }

    /// Exports at a chosen raster density or with scalable vector text.
    pub fn export_pdf_with_options(
        &self,
        mut pdf_options: PdfExportOptions,
    ) -> Result<Vec<u8>, String> {
        if pdf_options.tagged {
            // Marks survive only in vector drawing.
            pdf_options.rendering = PdfRenderingMode::Vector;
        }
        if !pdf_options.scale.is_finite() || !(0.5..=4.0).contains(&pdf_options.scale) {
            return Err("PDF scale must be finite and between 0.5 and 4".to_string());
        }
        let (width, height) = self.layout_page_size();
        let mut options = self.options.clone();
        options.show_formatting_marks = false;
        options.set_formatting_mark_kinds(Default::default());
        options.show_crop_marks = false;
        options.show_text_boundaries = false;
        if let Some(markup) = pdf_options.markup {
            options.render_changes = markup;
            options.render_comments = markup;
            if !markup {
                options.render_original = false;
            }
        }
        // Export can hide revisions or formatting marks and paginate differently
        // from the live view. Repaint early NUMPAGES fields only when that
        // changes the total; retained raster surfaces are never a page count.
        let mut page_count_hint = self.page_count();
        let mut attempts = 0;
        let (mut renderer, pages, count) = loop {
            // A fresh recording prevents text and structure tags from earlier
            // pagination attempts from remaining in the searchable PDF.
            let host = SkiaRenderHost::recording_with_fonts(
                width,
                height,
                &self.fonts,
                pdf_options,
                page_count_hint,
            )
            .ok_or_else(|| "could not create PDF recording pages".to_string())?;
            let pages = host.pages();
            let count = host.rendered_pages();
            let mut renderer = CanvasRenderer::<SkiaPainter>::with_host(Box::new(host), false)
                .map_err(|_| "PDF renderer setup failed".to_string())?;
            register_math_faces(&mut renderer, &self.math_faces);
            renderer.sync_word_document(&self.word_document);
            if pdf_options.rendering == PdfRenderingMode::Vector {
                renderer.set_page_export_mode(true);
            } else {
                // Editor mode promises the live raster, including table grid
                // guides. Vector mode is the clean, printable page surface.
                renderer.set_spelling_export_mode(true);
            }
            renderer.set_structure_tagging(pdf_options.tagged);
            renderer.mark_full_repaint();
            renderer
                .render_with_numbering(
                    self.word_document
                        .document
                        .as_ref()
                        .ok_or_else(|| "no document".to_string())?,
                    &options,
                    self.word_document.numbering_part.as_ref(),
                    self.word_document.header_lines.as_deref(),
                    self.word_document.footer_lines.as_deref(),
                    Some(&self.word_document.comments),
                )
                .map_err(|_| "PDF render failed".to_string())?;
            if let Some(error) = renderer.take_render_error() {
                return Err(error);
            }
            if count.get() == page_count_hint {
                break (renderer, pages, count);
            }
            attempts += 1;
            if attempts == 4 {
                return Err("PDF page count did not stabilize".to_string());
            }
            page_count_hint = count.get();
        };
        let bookmarks = renderer.page_bookmarks(&self.word_document);
        let links = (0..count.get())
            .flat_map(|page| renderer.page_hyperlink_regions(page))
            .filter_map(|link| {
                let target = if link
                    .target_mode
                    .as_deref()
                    .is_some_and(|mode| mode.eq_ignore_ascii_case("Internal"))
                    || link.target.is_none()
                {
                    let anchor = link.anchor.as_deref()?.trim_start_matches('#');
                    let bookmark = bookmarks.iter().find(|bookmark| bookmark.name == anchor)?;
                    jade_skia_painter::PdfLinkTarget::Page {
                        page_index: bookmark.page_index,
                        x: bookmark.x,
                        y: bookmark.y,
                    }
                } else {
                    jade_skia_painter::PdfLinkTarget::Uri(link.target?)
                };
                Some(jade_skia_painter::PdfLink {
                    page_index: link.page,
                    rect: (link.x, link.y, link.width, link.height),
                    target,
                    tooltip: link.tooltip,
                })
            })
            .collect::<Vec<_>>();
        if pdf_options.rendering == PdfRenderingMode::Vector
            && jade_docx_layout::line_metrics::word_layout_platform()
                == jade_docx_layout::line_metrics::WordLayoutPlatform::Windows
        {
            for page in &pages.borrow()[..count.get()] {
                let logical = page
                    .recording_size_points()
                    .ok_or_else(|| "missing PDF recording dimensions".to_string())?;
                page.set_pdf_page_size_points(Some(windows_word_pdf_paper_size(logical)))?;
            }
        }
        let output = if pdf_options.tagged {
            let entries = renderer.take_structure_entries();
            let tagging = crate::tagging::tagging(&self.word_document, &entries);
            SkiaPainter::encode_tagged_pdf(&pages.borrow()[..count.get()], &links, &tagging)
        } else {
            SkiaPainter::encode_pdf_with_links(&pages.borrow()[..count.get()], &links)
        };
        output
    }

    /// Each page's section (from 0) and the number its `PAGE` field shows:
    /// what a print range such as `p1s2-p3s3` names.
    pub fn page_labels(&self) -> Vec<(usize, usize)> {
        let mut labels = self.renderer.page_labels();
        labels.truncate(self.page_count());
        labels
    }

    pub fn can_undo(&self) -> bool {
        !self.history.is_empty() || self.executor.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_history.is_empty() || self.executor.can_redo()
    }

    /// How many undoable steps there are: a mark for [`Self::merge_history_since`].
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Makes every command step taken since `mark` one step, so a single
    /// undo takes them all back — what formatting several ranges at once,
    /// or a drag-and-drop made of several edits, needs. Steps of another
    /// kind (a header edit) are left as they are.
    pub fn merge_history_since(&mut self, mark: usize) {
        if mark >= self.history.len() {
            return;
        }
        let tail = &self.history[mark..];
        if !tail
            .iter()
            .all(|step| matches!(step, HistoryStep::Command { .. }))
        {
            return;
        }
        let executor_steps = tail
            .iter()
            .map(|step| match step {
                HistoryStep::Command { executor_steps } => *executor_steps,
                HistoryStep::HeaderFooter => 0,
            })
            .sum();
        self.history.truncate(mark);
        self.history.push(HistoryStep::Command { executor_steps });
    }

    /// Undoes the last edit and repaints, reporting the selection it restored.
    ///
    /// The whole document is repainted: an undo can move content anywhere,
    /// including onto pages the shell is not looking at.
    pub fn undo(&mut self) -> Result<Option<(usize, usize)>, String> {
        self.step_history(true)
    }

    pub fn redo(&mut self) -> Result<Option<(usize, usize)>, String> {
        self.step_history(false)
    }

    fn step_history(&mut self, backwards: bool) -> Result<Option<(usize, usize)>, String> {
        self.build_roundtrip_xml()?;
        // Header and footer edits are on their own stack, so undo has to pop
        // whichever stack the last step came from.
        let step = if backwards {
            self.history.last()
        } else {
            self.redo_history.last()
        };
        if matches!(step, Some(HistoryStep::HeaderFooter)) {
            let current = self
                .word_document
                .snapshot_including_package()
                .ok_or_else(|| "document went missing".to_string())?;
            let edit = if backwards {
                self.header_footer_undo.pop()
            } else {
                self.header_footer_redo.pop()
            }
            .ok_or_else(|| "header/footer history went missing".to_string())?;
            self.word_document.restore_snapshot(edit.previous);
            self.renderer.note_style_reference_edit();
            if backwards {
                self.history.pop();
                self.redo_history.push(HistoryStep::HeaderFooter);
                self.header_footer_redo
                    .push(HeaderFooterEdit { previous: current });
            } else {
                self.redo_history.pop();
                self.history.push(HistoryStep::HeaderFooter);
                self.header_footer_undo
                    .push(HeaderFooterEdit { previous: current });
            }
            self.repaint_all()?;
            return Ok(None);
        }

        // How many executor entries this one user action left behind. Undoing
        // one of a replace-all's inserts would leave the document half replaced
        // and the rest of the group waiting for further undos.
        let steps = match if backwards {
            self.history.last()
        } else {
            self.redo_history.last()
        } {
            Some(HistoryStep::Command { executor_steps }) => (*executor_steps).max(1),
            _ => 1,
        };

        let mut selection = self.selection.take();
        let moved = self.edit_in_place(|this| {
            let mut ctx = CommandContext::new(&mut this.word_document, &mut selection);
            let mut moved = false;
            for _ in 0..steps {
                let stepped = if backwards {
                    this.executor.undo(&mut ctx)
                } else {
                    this.executor.redo(&mut ctx)
                }
                .map_err(|err| format!("history failed: {err:?}"))?;
                if !stepped {
                    break;
                }
                moved = true;
            }
            Ok::<bool, String>(moved)
        });
        self.selection = selection;
        let moved = moved?;
        if !moved {
            return Ok(None);
        }
        // Moved between the stacks, so a redo knows the group size too.
        if backwards {
            if let Some(step) = self.history.pop() {
                self.redo_history.push(step);
            }
        } else if let Some(step) = self.redo_history.pop() {
            self.history.push(step);
        }
        self.repaint_all()?;
        Ok(self
            .selection
            .map(|range| (range.focus.paragraph, range.focus.offset)))
    }

    /// Repaints every page from scratch.
    pub fn repaint_all(&mut self) -> Result<Duration, String> {
        // Whatever asked for it — an edit, an undo, a view that lays the
        // document out otherwise (collapsed headings, markup, field codes) —
        // may leave STYLEREFs wrong afresh.
        self.renderer.note_style_reference_edit();
        let elapsed = self.repaint_all_drawing(self.paint_window.clone(), true)?;
        self.bounded_pass_since_full_layout = false;
        Ok(elapsed)
    }

    /// Lays every page out again and draws those in `drawn` (0-based; every
    /// page when `None`). With `others_changed`, the pages not drawn are
    /// marked stale; without, they are known to be unchanged.
    fn repaint_all_drawing(
        &mut self,
        drawn: Option<std::ops::Range<usize>>,
        others_changed: bool,
    ) -> Result<Duration, String> {
        // The model's own snapshot, shared: a copy would cost a deep clone of
        // the document and leave the renderer's identity caches missing.
        let document = Rc::clone(
            self.word_document
                .document
                .as_ref()
                .ok_or_else(|| "document went missing".to_string())?,
        );
        let started = Instant::now();
        self.renderer.sync_word_document(&self.word_document);
        self.renderer.set_selection_range_for_render(self.selection);
        self.refresh_field_display();
        // History can restore different paragraph contents at an address
        // previously used by another snapshot. Recompute address-keyed hashes
        // before deciding whether old paragraph paint can be replayed.
        self.renderer.invalidate_paragraph_content_hash_cache();
        self.renderer.mark_full_repaint();
        let pages_before = self.page_count();
        if let Some(drawn) = &drawn {
            self.renderer
                .set_visible_page_range(Some(drawn.start + 1..=drawn.end));
        }
        count_surface_document_render();
        let result = self.renderer.render_with_numbering_rc(
            document,
            &self.options,
            self.word_document.numbering_part.as_ref(),
            self.word_document.header_lines.as_deref(),
            self.word_document.footer_lines.as_deref(),
            Some(&self.word_document.comments),
        );
        self.renderer.clear_visible_page_range();
        result.map_err(|_| "repaint failed".to_string())?;
        if let Some(error) = self.renderer.take_render_error() {
            return Err(error);
        }
        self.roundtrip_relayout_pending = false;
        let pages = self.page_count().max(pages_before);
        match &drawn {
            Some(drawn) => {
                if others_changed {
                    self.stale_pages
                        .extend((0..pages).filter(|page| !drawn.contains(page)));
                }
                for page in drawn.start..drawn.end.min(pages) {
                    self.stale_pages.remove(&page);
                }
            }
            None => self.stale_pages.clear(),
        }
        // The native host has consumed this pass directly into retained page
        // surfaces. Leaving the tracker full-dirty disables incremental drawing
        // on every subsequent keystroke (the browser consumes it in its payload).
        self.renderer.clear_dirty_regions();
        self.follow_page_box(drawn)?;
        self.mark_wrong_page_totals_stale();
        Ok(started.elapsed())
    }

    /// When an edit has changed the page box — the page turned, another
    /// paper — the host and the renderer are built again to it, as opening
    /// the document builds them. Built to the old box, the renderer went on
    /// wrapping lines to its width: turned to landscape, the text kept its
    /// portrait width, the pages their portrait pitch, and lines ran over
    /// each other.
    ///
    /// The new surfaces draw `drawn` (0-based; every page when `None`), and
    /// the pages outside it are left stale: they start blank.
    fn follow_page_box(&mut self, drawn: Option<std::ops::Range<usize>>) -> Result<(), String> {
        let (width, height) = self.renderer.layout_page_size();
        if (width, height) == (self.page_width, self.page_height) || width <= 1.0 || height <= 1.0 {
            return Ok(());
        }
        // The density the pages were painted at, from the renderer about to
        // be let go.
        let raster_scale = self
            .renderer
            .set_raster_scale(1.0)
            .map_err(|_| "raster scale lookup failed".to_string())?;
        let host = SkiaRenderHost::with_fonts(width, height, &self.fonts, raster_scale)
            .ok_or_else(|| "could not allocate the raster surface".to_string())?;
        host.dark_paper().set(self.dark_paper.get());
        if let Some(first) = host.pages().borrow().first() {
            first.set_dark_paper(self.dark_paper.get());
        }
        let pages = host.pages();
        let rendered_pages = host.rendered_pages();
        let pass_covers_whole_document = host.pass_covers_whole_document();
        let dark_paper = host.dark_paper();
        let mut renderer = CanvasRenderer::<SkiaPainter>::with_host(Box::new(host), false)
            .map_err(|_| "renderer setup failed".to_string())?;
        register_math_faces(&mut renderer, &self.math_faces);
        renderer.sync_word_document(&self.word_document);
        renderer.set_use_selection_overlay(false);
        renderer.set_view_geometry(self.view_geometry);
        renderer.set_outline_indents(self.outline_indents.clone());
        renderer.set_selection_range_for_render(self.selection);
        renderer.set_current_datetime(self.field_datetime.map(renderer_datetime));
        renderer.set_field_shading(self.field_shading.clone());
        if self.field_codes != FieldCodeView::default() {
            renderer.set_field_code_display(Some(field_code_display(
                &self.word_document,
                &self.field_codes,
            )));
        }
        renderer.mark_full_repaint();
        let document = Rc::clone(
            self.word_document
                .document
                .as_ref()
                .ok_or_else(|| "document went missing".to_string())?,
        );
        if let Some(drawn) = &drawn {
            renderer.set_visible_page_range(Some(drawn.start + 1..=drawn.end));
        }
        count_surface_document_render();
        let result = renderer.render_with_numbering_rc(
            document,
            &self.options,
            self.word_document.numbering_part.as_ref(),
            self.word_document.header_lines.as_deref(),
            self.word_document.footer_lines.as_deref(),
            Some(&self.word_document.comments),
        );
        renderer.clear_visible_page_range();
        result.map_err(|_| "repaint failed".to_string())?;
        if let Some(error) = renderer.take_render_error() {
            return Err(error);
        }
        renderer.clear_dirty_regions();
        let page_count = rendered_pages.get();
        self.stale_pages = match &drawn {
            Some(drawn) => (0..page_count)
                .filter(|page| !drawn.contains(page))
                .collect(),
            None => std::collections::BTreeSet::new(),
        };
        self.renderer = renderer;
        self.pages = pages;
        self.rendered_pages = rendered_pages;
        self.pass_covers_whole_document = pass_covers_whole_document;
        self.dark_paper = dark_paper;
        self.page_width = width;
        self.page_height = height;
        Ok(())
    }

    /// Rasterises the pages at `scale` device pixels per CSS pixel — the
    /// window's backing scale — and repaints if that changed it.
    ///
    /// Layout, hit-testing and the caret stay in CSS pixels; only
    /// [`Self::page_pixels`] grows. Returns whether the pages were repainted.
    pub fn set_raster_scale(&mut self, scale: f64) -> Result<bool, String> {
        if !scale.is_finite() || !(0.5..=4.0).contains(&scale) {
            return Err("raster scale must be finite and between 0.5 and 4".to_string());
        }
        let previous = self
            .renderer
            .set_raster_scale(scale)
            .map_err(|_| "raster scale change failed".to_string())?;
        if previous == scale {
            return Ok(false);
        }
        self.repaint_all()?;
        Ok(true)
    }

    /// Shows the pages on dark paper, or on the document's own, and repaints if
    /// that changed anything. Returns whether the pages were repainted.
    ///
    /// Only what [`Self::page_pixels`] reads changes: the document keeps its
    /// colours, and [`Self::export_pdf`] — and so printing — draws them, since
    /// it paints on surfaces of its own.
    pub fn set_dark_paper(&mut self, on: bool) -> Result<bool, String> {
        if self.dark_paper.replace(on) == on {
            return Ok(false);
        }
        // The whole pool, not only the rendered pages: a surface reused when
        // the document grows must not come back on the other paper.
        for page in self.pages.borrow().iter() {
            page.set_dark_paper(on);
        }
        self.repaint_all()?;
        Ok(true)
    }

    /// Whether the pages are shown on dark paper.
    pub fn dark_paper(&self) -> bool {
        self.dark_paper.get()
    }

    /// Selects from `anchor` to `focus` and repaints so the highlight appears.
    ///
    /// The renderer paints the selection, so this costs a repaint — unlike
    /// moving a collapsed caret, which the shell draws itself.
    ///
    /// A selection changes no layout, only the highlight, so only the pages
    /// the old or the new selection is on are drawn again, and of those only
    /// the ones up to `visible_end_page_1idx`; the ones after it are left
    /// stale for [`Self::paint_stale_pages`]. The pass lays out no further
    /// than the last page it draws, and keeps the hit-test, caret and region
    /// registries the last layout built: rebuilt from a pass that stopped at
    /// the bound, they covered only the pages before it, and clicks and
    /// carets on later pages went wrong until the next full repaint.
    pub fn select(
        &mut self,
        anchor: (usize, usize),
        focus: (usize, usize),
        visible_end_page_1idx: usize,
    ) -> Result<(), String> {
        let range = SelectionRange::new(
            TextPosition::new(anchor.0, anchor.1),
            TextPosition::new(focus.0, focus.1),
        );
        // Read from the last pass's layout, before the selection changes.
        let pages = [self.selection, Some(range)]
            .into_iter()
            .flatten()
            .map(|range| self.selection_pages(range))
            .try_fold(None, |acc: Option<(usize, usize)>, span| {
                let (first, last) = span?;
                Some(Some(
                    acc.map_or((first, last), |(a, b)| (a.min(first), b.max(last))),
                ))
            })
            .flatten();
        self.selection = Some(range);
        self.renderer.set_selection_range(Some(range));
        let visible_end = visible_end_page_1idx.max(1);
        let Some((first, last)) = pages else {
            // An end of a selection the last pass did not lay out (one
            // bounded to the pages on screen): the layout is not whole. Lay
            // every page out again, draw the pages on screen, and leave every
            // other page stale, since the pages may have moved.
            // `visible_end` may mean "no bound" (`usize::MAX`); pages past
            // the old count are left stale by the layout itself.
            let drawn = 0..visible_end.min(self.page_count().max(1));
            self.repaint_all_drawing(Some(drawn), true)?;
            return Ok(());
        };
        let drawn = first..(last + 1).min(visible_end);
        if !drawn.is_empty() {
            if self.renderer.any_section_valign_enabled() {
                // A vertically aligned section's offsets come from the used
                // height of every page, which a pass over a few pages does
                // not measure: lay every page out, drawing the same pages.
                self.repaint_all_drawing(Some(drawn), false)?;
            } else {
                self.repaint_unchanged_layout(drawn)?;
            }
        }
        // The layout did not change, so the pages read before it still hold.
        let count = self.page_count();
        self.stale_pages
            .extend((first.max(visible_end)..=last).filter(|&page| page < count));
        Ok(())
    }

    /// Draws `drawn` (0-based pages) again over a layout that has not
    /// changed since the last pass, which laid out every page. The walk stops
    /// after the last drawn page, and the registries the last pass built
    /// (hit-testing, carets, cells, links, pagination) are kept, since they
    /// describe the same layout.
    fn repaint_unchanged_layout(&mut self, drawn: std::ops::Range<usize>) -> Result<(), String> {
        let document = Rc::clone(
            self.word_document
                .document
                .as_ref()
                .ok_or_else(|| "document went missing".to_string())?,
        );
        self.renderer.set_selection_range_for_render(self.selection);
        count_surface_document_render();
        let result = self.renderer.render_pages_over_unchanged_layout(
            document,
            &self.options,
            self.word_document.numbering_part.as_ref(),
            self.word_document.header_lines.as_deref(),
            self.word_document.footer_lines.as_deref(),
            Some(&self.word_document.comments),
            drawn.start + 1..=drawn.end,
        );
        result.map_err(|_| "repaint failed".to_string())?;
        if let Some(error) = self.renderer.take_render_error() {
            return Err(error);
        }
        for page in drawn {
            self.stale_pages.remove(&page);
        }
        self.mark_wrong_style_references_stale();
        self.renderer.clear_dirty_regions();
        Ok(())
    }

    /// The first and last page (0-based) the lines of `range` are on in the
    /// last pass's layout, or `None` when an end of it was not laid out.
    fn selection_pages(&self, range: SelectionRange) -> Option<(usize, usize)> {
        let (start, end) = range.normalized();
        let paragraphs = self.renderer.selection_manager().paragraphs();
        // The ends must be laid out; a paragraph between them without lines
        // (hidden text) draws nothing.
        for paragraph in [start.paragraph, end.paragraph] {
            paragraphs
                .get(paragraph)
                .filter(|layout| !layout.lines.is_empty())?;
        }
        let mut span: Option<(usize, usize)> = None;
        for layout in &paragraphs[start.paragraph..=end.paragraph] {
            for line in &layout.lines {
                let bottom = (line.absolute_top + line.height - 0.01).max(line.absolute_top);
                for y in [line.absolute_top, bottom] {
                    let page = self.renderer.page_for_y(y.max(0.0));
                    span = Some(span.map_or((page, page), |(a, b)| (a.min(page), b.max(page))));
                }
            }
        }
        span
    }

    /// Selects whole cells, as Word does when a drag crosses from one cell
    /// into another or a row, a column or the table is selected: the
    /// rectangle the two cells span, by their ids in the last paint's cell
    /// registry. The text range runs from the first cell's start to the last
    /// one's end, which is what commands act on; the highlight is the cells.
    pub fn select_cells(&mut self, anchor_cell: usize, focus_cell: usize) -> Result<(), String> {
        let range = self
            .renderer
            .cell_selection_range(anchor_cell, focus_cell)
            .ok_or_else(|| "the cells are not in one table".to_string())?;
        self.selection = Some(range);
        self.renderer.set_selection_range(Some(range));
        self.renderer.set_cell_selection(anchor_cell, focus_cell);
        self.repaint_all().map(|_| ())
    }

    /// Sets the text of a header or footer paragraph.
    ///
    /// Headers and footers are their own parts, addressed by paragraph within
    /// the part rather than by document position — the caret cannot be in one,
    /// so there is nothing to derive them from.
    pub fn set_header_footer_text(
        &mut self,
        footer: bool,
        paragraph: usize,
        text: &str,
    ) -> Result<(), String> {
        self.build_roundtrip_xml()?;
        let previous = self
            .word_document
            .snapshot_including_package()
            .ok_or_else(|| "document went missing".to_string())?;
        let command = if footer {
            HeadlessCommand::SetFooterParagraphText {
                paragraph,
                text: text.to_string(),
            }
        } else {
            HeadlessCommand::SetHeaderParagraphText {
                paragraph,
                text: text.to_string(),
            }
        };
        let after = self.edit_in_place(|this| {
            apply_headless_commands_with_executor_to_document(
                &mut this.word_document,
                &mut this.executor,
                this.selection,
                &HeadlessEditRequest {
                    commands: vec![command],
                },
            )
        });
        self.selection = match after {
            Ok(after) => after,
            Err(error) => {
                // Creating a missing story can change the package before an
                // invalid paragraph is rejected. Failed edits keep both the
                // original package and the selection, without losing redo.
                self.edit_in_place(|this| this.word_document.restore_snapshot(previous));
                self.repaint_all()?;
                return Err(format!("header/footer edit failed: {error:?}"));
            }
        };
        self.repaint_all()?;
        self.header_footer_undo.push(HeaderFooterEdit { previous });
        self.header_footer_redo.clear();
        self.redo_history.clear();
        // This edit bypasses the executor's command stack, but forks its redo
        // branch just as an ordinary body command does.
        while self.executor.can_redo() {
            self.executor.discard_newest_redo();
        }
        self.history.push(HistoryStep::HeaderFooter);
        Ok(())
    }

    /// Moves and resizes a floating object.
    ///
    /// Goes through the executor, so it lands on the undo stack like a text
    /// edit — an image dragged to the wrong place has to be recoverable the
    /// same way a mistyped word is.
    pub fn transform_object(
        &mut self,
        object: ObjectHit,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> Result<(), String> {
        self.transform_object_inner(object, (x, y, width, height), false, None)
    }

    /// Replaces the previous transform instead of stacking on top of it.
    ///
    /// A drag arrives as a stream of moves. Recording every one of them would
    /// take as many undos to get back to where the picture started, so the
    /// previous transform is rolled off the stack first: one drag, one undo.
    /// Only for the moves of a drag that is still in progress — the first move
    /// has nothing to replace and must use [`Self::transform_object`].
    pub fn retransform_object(
        &mut self,
        object: ObjectHit,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> Result<(), String> {
        self.transform_object_inner(object, (x, y, width, height), true, Some(object.page))
    }

    /// `rect` is the object's new (x, y, width, height); `replace_previous`
    /// rolls the last transform off the stack first, and `bound_to_page` limits
    /// the repaint to one page.
    fn transform_object_inner(
        &mut self,
        object: ObjectHit,
        rect: (f64, f64, f64, f64),
        replace_previous: bool,
        bound_to_page: Option<usize>,
    ) -> Result<(), String> {
        self.ensure_roundtrip_xml()?;
        let (x, y, width, height) = rect;
        if replace_previous {
            // Rolled back without a repaint: the transform below redraws, and
            // painting the intermediate state would flicker the picture back to
            // where it was between every pair of moves.
            let mut selection = self.selection.take();
            self.edit_in_place(|this| {
                let mut ctx = CommandContext::new(&mut this.word_document, &mut selection);
                this.executor.undo(&mut ctx)
            })
            .map_err(|err| format!("history failed: {err:?}"))?;
            self.selection = selection;
            self.history.pop();
        }
        let target = FloatingObjectTarget {
            paragraph_index: object.paragraph,
            run_index: object.run,
            kind: object.kind,
        };
        // A move or resize keeps the object's rotation.
        let rotation_deg = self
            .word_document
            .paragraph(object.paragraph)
            .and_then(|paragraph| paragraph.runs.get(object.run))
            .and_then(|run| {
                run.image
                    .as_ref()
                    .map(|image| &image.layout)
                    .or_else(|| run.text_box.as_ref().map(|text_box| &text_box.layout))
            })
            .and_then(|layout| layout.rotation_deg)
            .unwrap_or(0.0);
        let transform = FloatingObjectTransform {
            x,
            y,
            width,
            height,
            rotation_deg,
        };
        let mut selection = self.selection.take();
        self.edit_in_place(|this| {
            let mut ctx = CommandContext::new(&mut this.word_document, &mut selection);
            this.executor.execute(
                Box::new(TransformObjectCommand::new(target, transform)),
                &mut ctx,
            )
        })
        .map_err(|err| format!("transform failed: {err:?}"))?;
        self.selection = selection;
        self.redo_history.clear();
        self.history
            .push(HistoryStep::Command { executor_steps: 1 });
        match bound_to_page {
            // A drag is a stream of moves, and a full repaint costs about a
            // millisecond per page — fine for one edit, not for sixty a second
            // on a document with pages. Bounded, a move costs one page.
            Some(page) => self.repaint_bounded(object.paragraph, page).map(|_| ()),
            None => self.repaint_all().map(|_| ()),
        }
    }

    /// Repaints from `paragraph` up to `visible_end_page_1idx`, drawing only
    /// `visible_end_page_1idx` itself.
    fn repaint_bounded(
        &mut self,
        paragraph: usize,
        visible_end_page_1idx: usize,
    ) -> Result<Duration, String> {
        self.repaint_bounded_inner(paragraph, visible_end_page_1idx, true)
    }

    fn repaint_bounded_inner(
        &mut self,
        paragraph: usize,
        visible_end_page_1idx: usize,
        only_the_bound_page: bool,
    ) -> Result<Duration, String> {
        let document = self
            .word_document
            .document
            .as_ref()
            .ok_or_else(|| "document went missing".to_string())?
            .clone();
        let started = Instant::now();
        self.refresh_field_display();
        self.bounded_pass_since_full_layout = true;
        self.pass_covers_whole_document.set(false);
        self.renderer
            .prepare_incremental_repaint(paragraph, visible_end_page_1idx);
        // With `only_the_bound_page`, the pages before it still have to be laid
        // out — pagination decides where this one starts — but their pixels are
        // already on their own surfaces and redrawing them is work with no
        // result. The browser pairs this narrowing with its page-bitmap cache;
        // here the cache is the surfaces themselves. Only safe when the change
        // cannot have touched an earlier page.
        if only_the_bound_page {
            self.renderer
                .set_visible_page_range(Some(visible_end_page_1idx..=visible_end_page_1idx));
        }
        let result = self.renderer.render_bounded_to_page_with_parts(
            document,
            &self.options,
            self.word_document.numbering_part.as_ref(),
            self.word_document.header_lines.as_deref(),
            self.word_document.footer_lines.as_deref(),
            Some(&self.word_document.comments),
            visible_end_page_1idx,
        );
        self.renderer.clear_visible_page_range();
        self.pass_covers_whole_document.set(true);
        result.map_err(|_| "repaint failed".to_string())?;
        if let Some(error) = self.renderer.take_render_error() {
            return Err(error);
        }
        // A moved object can re-wrap text past the bound; a selection changes
        // no layout, so the pages after it keep pixels that are still right.
        if only_the_bound_page {
            self.mark_pages_past_bound_stale(visible_end_page_1idx, false);
        }
        self.mark_wrong_style_references_stale();
        // The native host has consumed this pass directly into retained page
        // surfaces. Leaving the tracker full-dirty disables incremental drawing
        // on every subsequent keystroke (the browser consumes it in its payload).
        self.renderer.clear_dirty_regions();
        Ok(started.elapsed())
    }

    /// After a bounded pass that changed content: the pages after
    /// `bound_1idx` still show — and their lines still answer hit-testing
    /// with — the layout from before the edit, unless the pass reached the
    /// end of the document, proved the pagination after the edit unchanged,
    /// or `downstream_stood` (the edit's own proof, from the lines it laid
    /// out). Otherwise they are marked stale, so [`Self::paint_stale_pages`]
    /// lays the document out again when a shell draws them. A keystroke that
    /// keeps everything after its paragraph in place — typing, Backspace,
    /// Delete, in a table cell or not — marks nothing.
    fn mark_pages_past_bound_stale(&mut self, bound_1idx: usize, downstream_stood: bool) {
        if downstream_stood
            || self.renderer.last_layout_complete()
            || self.renderer.last_completed_pagination_edit_preserved()
        {
            return;
        }
        let count = self.page_count();
        if bound_1idx < count {
            self.stale_pages.extend(bound_1idx..count);
        }
    }

    /// Lays the document out again when a bounded pass since the last whole
    /// layout stopped before `paragraph`: such a pass registers lines only up
    /// to its bound, and a caret query past it would land on the last line it
    /// registered. Draws only the last page; the rest stay as they are, stale
    /// or not.
    fn ensure_layout_reaches(&mut self, paragraph: usize) -> Result<(), String> {
        // An entry without lines counts as missing: past a pass's bound the
        // registry can hold entries with none (a document with footnotes
        // does).
        if !self.bounded_pass_since_full_layout
            || self
                .renderer
                .selection_manager()
                .paragraphs()
                .get(paragraph)
                .is_some_and(|layout| !layout.lines.is_empty())
        {
            return Ok(());
        }
        let count = self.page_count();
        let last = count.saturating_sub(1);
        // Drawn: the last page, or the effective layout limit would end the
        // walk at the drawn window.
        self.repaint_all_drawing(Some(last..last + 1), false)?;
        self.bounded_pass_since_full_layout = false;
        self.mark_added_pages_stale(count);
        Ok(())
    }

    /// Where paragraph `paragraph`'s lines begin and end in the last layout,
    /// and how many there are; `None` when it has none registered.
    fn paragraph_extent(&self, paragraph: usize) -> Option<ParagraphExtent> {
        let lines = &self
            .renderer
            .selection_manager()
            .paragraphs()
            .get(paragraph)?
            .lines;
        let (first, last) = (lines.first()?, lines.last()?);
        Some(ParagraphExtent {
            lines: lines.len(),
            top: first.absolute_top,
            bottom: last.absolute_top + last.height,
        })
    }

    /// The floating object under a point, in document coordinates.
    ///
    /// Images and shapes are not in the text flow, so hit testing for them is
    /// separate from hit testing for a caret position — a click inside a
    /// picture still resolves to some paragraph, and answering with that would
    /// put the caret behind the image instead of selecting it.
    pub fn object_at(&self, x: f64, y: f64) -> Option<ObjectHit> {
        let info = self.renderer.hit_test_object(x, y)?;
        Some(ObjectHit {
            kind: info.kind,
            paragraph: info.paragraph_index,
            run: info.run_index,
            page: info.page,
            x: info.x,
            y: info.y,
            width: info.width,
            height: info.height,
        })
    }

    /// Adds a comment on the selection, or on the caret's paragraph.
    ///
    /// Comments live in a separate part of the .docx, which is why editing a
    /// document jade did not write still keeps the ones already in it: the part
    /// is carried through untouched unless a command changes it.
    pub fn add_comment(&mut self, text: &str, author: &str) -> Result<(), String> {
        let (start, end) = match self.selection {
            Some(range) if !range.is_collapsed() => {
                let (start, end) = range.normalized();
                (
                    (start.paragraph, Some(start.offset)),
                    (end.paragraph, Some(end.offset)),
                )
            }
            _ => {
                let paragraph = self.selection_paragraph();
                ((paragraph, None), (paragraph, None))
            }
        };
        let pages = self.page_count().max(1);
        self.apply_and_repaint(
            start.0,
            HeadlessCommand::AddComment {
                start_paragraph: start.0,
                start_offset: start.1,
                end_paragraph: end.0,
                end_offset: end.1,
                text: text.to_string(),
                author: Some(author.to_string()),
                initials: None,
                date: None,
            },
            pages,
        )
        .map(|_| ())
    }

    /// How many comments the document carries.
    pub fn comment_count(&self) -> usize {
        let Ok(response) = execute_headless_queries_on_document(
            &self.word_document,
            &HeadlessQueryRequest {
                queries: vec![HeadlessQuery::Comments {
                    start_comment: 0,
                    max_comments: 1000,
                    author: None,
                    resolved: None,
                }],
            },
        ) else {
            return 0;
        };
        response
            .results
            .into_iter()
            .find_map(|result| match result {
                HeadlessQueryResult::Comments { comments, .. } => Some(comments.len()),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// Turns change tracking on or off.
    ///
    /// With it on, every subsequent edit is recorded as a revision rather than
    /// applied outright — which is how a contract is marked up for the other
    /// side to review.
    pub fn set_change_tracking(&mut self, on: bool, author: &str) -> Result<(), String> {
        let command = if on {
            HeadlessCommand::StartChangeTracking {
                author: Some(author.to_string()),
                initials: None,
                date: None,
                date_utc: None,
            }
        } else {
            HeadlessCommand::StopChangeTracking
        };
        let pages = self.page_count().max(1);
        self.apply_and_repaint(self.selection_paragraph(), command, pages)
            .map(|_| ())
    }

    /// The table cell the caret is in, if it is in one.
    ///
    /// Table commands address a table by its order in the body and a cell by
    /// row and column, none of which the caret knows — this is the same walk
    /// the browser runtime does to bridge the two.
    pub fn caret_table_cell(&self) -> Option<TableCell> {
        let range = self.selection?;
        let paragraph = range.focus.paragraph;
        let cell_id = self.renderer.cell_id_for_paragraph(paragraph)?;
        // By registry id: `CellLayout::order` restarts at every table, so
        // matching on it only finds cells of the first one.
        let layout = self.renderer.cell_layout(cell_id)?;
        let table_order_index = self.renderer.table_order_for(layout.table_id)?;
        let (rows, columns) = self.word_document.table_dimensions(table_order_index)?;
        Some(TableCell {
            table_order_index,
            row: layout.row,
            column: layout.column,
            rows,
            columns,
        })
    }

    /// Inserts a row below the caret's row, or deletes the caret's row.
    ///
    /// A table edit re-flows everything after it, so the whole document is
    /// repainted rather than the visible page.
    pub fn edit_table_row(&mut self, insert: bool) -> Result<(), String> {
        let cell = self
            .caret_table_cell()
            .ok_or_else(|| "the caret is not in a table".to_string())?;
        if !insert && cell.rows <= 1 {
            return Err("a table needs at least one row".to_string());
        }
        let command = if insert {
            HeadlessCommand::InsertTableRow {
                table_order_index: cell.table_order_index,
                row_index: cell.row + 1,
                template_row: Some(cell.row),
            }
        } else {
            HeadlessCommand::DeleteTableRow {
                table_order_index: cell.table_order_index,
                row_index: cell.row,
            }
        };
        let pages = self.page_count().max(1);
        self.apply_and_repaint(self.selection_paragraph(), command, pages)
            .map(|_| ())
    }

    /// Inserts a table after the paragraph the caret is in.
    pub fn insert_table(&mut self, rows: usize, columns: usize) -> Result<(), String> {
        let after = self.selection_paragraph();
        let pages = self.page_count().max(1);
        self.apply_and_repaint(
            after,
            HeadlessCommand::InsertTable {
                after_paragraph: after,
                rows,
                columns,
                column_widths: Vec::new(),
                caret_offset: None,
            },
            pages,
        )
        .map(|_| ())
    }

    /// Inserts a page break at the caret.
    pub fn insert_blank_page(&mut self) -> Result<(), String> {
        let (paragraph, offset) = match self.selection {
            Some(range) => {
                let (start, _) = range.normalized();
                (start.paragraph, start.offset)
            }
            None => (self.selection_paragraph(), 0),
        };
        let pages = self.page_count().max(1) + 1;
        self.apply_and_repaint(
            paragraph,
            HeadlessCommand::InsertBlankPage {
                start_paragraph: paragraph,
                start_offset: offset,
                end_paragraph: paragraph,
                end_offset: offset,
            },
            pages,
        )
        .map(|_| ())
    }

    /// Inserts a table of contents after the caret's paragraph.
    pub fn insert_table_of_contents(&mut self) -> Result<(), String> {
        let after = self.selection_paragraph();
        let pages = self.page_count().max(1);
        self.apply_and_repaint(
            after,
            HeadlessCommand::InsertTableOfContents {
                after_paragraph: after,
                outline_min: 1,
                outline_max: 3,
            },
            pages,
        )
        .map(|_| ())
    }

    /// Accepts or rejects every tracked change in the document.
    pub fn resolve_revisions(&mut self, accept: bool) -> Result<(), String> {
        let pages = self.page_count().max(1);
        let command = if accept {
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
        };
        self.apply_and_repaint(0, command, pages).map(|_| ())
    }

    /// 変更内容の表示 and 変更履歴とコメントの表示: whether tracked changes
    /// are marked up (`markup`), the original shown instead of the final text
    /// (`original`), and the view's change bars, reviewer colours and
    /// filters. Changes only what the pages show; repaints when it changed.
    pub fn set_revision_display(
        &mut self,
        markup: bool,
        original: bool,
        view: jade_docx_editor::revision_view::RevisionView,
    ) -> Result<bool, String> {
        let original = original && !markup;
        if self.options.render_changes == markup
            && self.options.render_original == original
            && *self.options.revision_view() == view
        {
            return Ok(false);
        }
        self.options.render_changes = markup;
        self.options.render_original = original;
        self.options.set_revision_view(view);
        self.repaint_all()?;
        Ok(true)
    }

    /// Whether tracked changes are marked up, and whether the original is
    /// shown in place of the final text.
    pub fn revision_display(&self) -> (bool, bool) {
        (self.options.render_changes, self.options.render_original)
    }

    /// 入力時にスペルチェックを行う: marks misspellings and grammar on the
    /// pages, or takes the marks off. Each change starts afresh.
    pub fn set_proofing(&mut self, on: bool) -> Result<(), String> {
        let (_, generation) = self.renderer.spelling_state();
        self.renderer
            .configure_spelling(on, generation.wrapping_add(1));
        self.repaint_all().map(|_| ())
    }

    /// Opened 読み取り専用 (the file recommends it, or it is marked final):
    /// no edit runs until this is turned off.
    pub fn set_opened_read_only(&mut self, read_only: bool) {
        self.word_document.set_opened_read_only(read_only);
    }

    /// Whether the pages are marked for spelling and grammar.
    pub fn proofing(&self) -> bool {
        self.renderer.spelling_state().0
    }

    /// Asks `check` about every span drawn since the last call — its text,
    /// the span's byte range in it, and its `w:lang` — and draws what it
    /// finds. Returns whether the pages changed.
    #[allow(clippy::type_complexity)]
    pub fn proof_pending(
        &mut self,
        check: &mut dyn FnMut(
            &str,
            usize,
            usize,
            Option<&str>,
        ) -> Vec<jade_docx_editor::plugins::spelling::ProofingIssue>,
    ) -> Result<bool, String> {
        let (on, generation) = self.renderer.spelling_state();
        if !on {
            return Ok(false);
        }
        let mut changed = false;
        for request in self.renderer.take_pending_spelling() {
            let issues = check(
                &request.text,
                request.start,
                request.end,
                request.language.as_deref(),
            );
            changed |= self
                .renderer
                .apply_proofing_issues(generation, &request.key, &issues);
        }
        if changed {
            self.repaint_all()?;
        }
        Ok(changed)
    }

    /// Shows or hides Word's formatting marks.
    pub fn set_formatting_marks(&mut self, show: bool) -> Result<(), String> {
        self.options.show_formatting_marks = show;
        self.repaint_all().map(|_| ())
    }

    /// Shows the document with its tracked changes and comments, or as it
    /// reads with every change accepted (Word's 変更履歴なし), and repaints.
    pub fn set_markup_visible(&mut self, visible: bool) -> Result<(), String> {
        self.options.render_changes = visible;
        self.options.render_comments = visible;
        if !visible {
            self.options.render_original = false;
        }
        self.repaint_all().map(|_| ())
    }

    /// Lays the document out in a view's page box — Web レイアウト's strips
    /// the width of the window, 閲覧モード's screen-sized columns — or, with
    /// `None`, in its sections' pages again. Everything is laid out anew.
    pub fn set_view_geometry(
        &mut self,
        geometry: Option<jade_docx_editor::canvas_renderer::ViewGeometry>,
    ) -> Result<bool, String> {
        if self.view_geometry == geometry {
            return Ok(false);
        }
        self.view_geometry = geometry;
        self.renderer.set_view_geometry(geometry);
        self.repaint_all()?;
        Ok(true)
    }

    /// アウトライン: each paragraph set in by its outline level (CSS pixels
    /// from the margin), or `None` for its own indents.
    pub fn set_outline_indents(
        &mut self,
        indents: Option<std::collections::HashMap<usize, f64>>,
    ) -> Result<bool, String> {
        let indents = indents.map(Rc::new);
        if self.outline_indents == indents {
            return Ok(false);
        }
        self.outline_indents = indents.clone();
        self.renderer.set_outline_indents(indents);
        self.repaint_all()?;
        Ok(true)
    }

    /// The view's page box, while one is set.
    pub fn view_geometry(&self) -> Option<jade_docx_editor::canvas_renderer::ViewGeometry> {
        self.view_geometry
    }

    /// Whether formatting marks are being drawn.
    pub fn formatting_marks(&self) -> bool {
        self.options.show_formatting_marks
    }

    /// Shows the formatting marks of some kinds whatever the ¶ toggle says
    /// (Word のオプション → 表示 → 常に画面に表示する編集記号).
    pub fn set_formatting_mark_kinds(
        &mut self,
        kinds: jade_docx_editor::FormattingMarkKinds,
    ) -> Result<(), String> {
        if self.options.formatting_mark_kinds() == kinds {
            return Ok(());
        }
        self.options.set_formatting_mark_kinds(kinds);
        self.repaint_all().map(|_| ())
    }

    /// The kinds shown beside the ¶ toggle.
    pub fn formatting_mark_kinds(&self) -> jade_docx_editor::FormattingMarkKinds {
        self.options.formatting_mark_kinds()
    }

    /// Sets the clock DATE and TIME fields show from the next repaint on.
    /// Without one they show the result Word saved.
    pub fn set_field_datetime(&mut self, datetime: Option<FieldDateTime>) {
        self.field_datetime = datetime;
    }

    /// フィールドの網かけ: which fields' text is drawn on Word's grey (on
    /// screen; a PDF leaves it out). Repaints when it changed; whether it
    /// did.
    pub fn set_field_shading(&mut self, shading: FieldShading) -> Result<bool, String> {
        if self.field_shading == shading {
            return Ok(false);
        }
        self.field_shading = shading.clone();
        self.renderer.set_field_shading(shading);
        self.repaint_all()?;
        Ok(true)
    }

    /// Which fields are drawn shaded.
    pub fn field_shading(&self) -> &FieldShading {
        &self.field_shading
    }

    /// Which fields show their codes.
    pub fn field_code_view(&self) -> &FieldCodeView {
        &self.field_codes
    }

    /// Shows the codes of the fields `view` names (⌥F9 every field, ⇧F9 the
    /// selected ones) and repaints.
    pub fn set_field_code_view(&mut self, view: FieldCodeView) -> Result<(), String> {
        self.build_roundtrip_xml()?;
        self.field_codes = view;
        self.renderer.set_field_code_display(None);
        self.repaint_all().map(|_| ())
    }

    /// Gives the renderer the clock and, while codes are shown, what each
    /// field's runs show. Edits move runs, so the codes are found afresh.
    fn refresh_field_display(&mut self) {
        self.renderer
            .set_current_datetime(self.field_datetime.map(renderer_datetime));
        if self.field_codes == FieldCodeView::default() {
            return;
        }
        let display = field_code_display(&self.word_document, &self.field_codes);
        self.renderer.set_field_code_display(Some(display));
    }

    /// Hides the top-level body elements under collapsed headings (Word's
    /// 見出しの折りたたみ) and repaints; empty shows them all again.
    pub fn set_collapsed_blocks(&mut self, blocks: Vec<usize>) -> Result<(), String> {
        self.options.set_collapsed_blocks(blocks);
        self.repaint_all().map(|_| ())
    }

    /// The top-level body elements collapsed headings hide.
    pub fn collapsed_blocks(&self) -> &[usize] {
        self.options.collapsed_blocks()
    }

    /// Shows or hides the crop marks around the document's content area.
    pub fn set_crop_marks(&mut self, show: bool) -> Result<(), String> {
        self.options.show_crop_marks = show;
        self.repaint_all().map(|_| ())
    }

    /// Whether the crop marks (トリムマーク) are drawn.
    pub fn crop_marks(&self) -> bool {
        self.options.show_crop_marks
    }

    /// Shows or hides 文字列の範囲, the dotted line round each page's text
    /// area. On screen only: a PDF leaves it out.
    pub fn set_text_boundaries(&mut self, show: bool) -> Result<(), String> {
        self.options.show_text_boundaries = show;
        self.repaint_all().map(|_| ())
    }

    /// Whether 文字列の範囲 is drawn.
    pub fn text_boundaries(&self) -> bool {
        self.options.show_text_boundaries
    }

    /// A table's model, as the headless API reports it.
    ///
    /// What the renderer lays out from, so a page that looks wrong can be
    /// compared against the model that produced it rather than guessed at.
    pub fn table_snapshot(
        &self,
        table_order_index: usize,
    ) -> Option<jade_docx_headless_api::HeadlessTableSnapshot> {
        let response = execute_headless_queries_on_document(
            &self.word_document,
            &HeadlessQueryRequest {
                queries: vec![HeadlessQuery::Table { table_order_index }],
            },
        )
        .ok()?;
        response
            .results
            .into_iter()
            .find_map(|result| match result {
                HeadlessQueryResult::Table { value, .. } => value,
                _ => None,
            })
    }

    /// The run style at a position — what the toolbar shows for the caret, and
    /// what a test can assert a formatting command actually changed.
    ///
    /// A family or a colour need not change a pixel: every family here is
    /// aliased to the one bundled face, so the model is the only place the
    /// change is visible.
    pub fn run_style_at(&self, paragraph: usize, offset: usize) -> Option<HeadlessRunStyle> {
        let response = execute_headless_queries_on_document(
            &self.word_document,
            &HeadlessQueryRequest {
                queries: vec![HeadlessQuery::CharacterStyleAt { paragraph, offset }],
            },
        )
        .ok()?;
        response
            .results
            .into_iter()
            .find_map(|result| match result {
                HeadlessQueryResult::CharacterStyleAt { value, .. } => {
                    value.map(|snapshot| snapshot.style)
                }
                _ => None,
            })
    }

    /// A paragraph's `w:pStyle`, or `None` when it has the default style.
    pub fn paragraph_style(&self, paragraph: usize) -> Option<String> {
        let response = execute_headless_queries_on_document(
            &self.word_document,
            &HeadlessQueryRequest {
                queries: vec![HeadlessQuery::Paragraph { paragraph }],
            },
        )
        .ok()?;
        response
            .results
            .into_iter()
            .find_map(|result| match result {
                HeadlessQueryResult::Paragraph { value, .. } => value.and_then(|p| p.style_id),
                _ => None,
            })
    }

    /// How many paragraphs the document has.
    ///
    /// Selecting everything needs the last one, and counting by walking
    /// [`Self::paragraph_text`] until it returns `None` would parse the whole
    /// document to answer.
    pub fn paragraph_count(&self) -> usize {
        let Ok(response) = execute_headless_queries_on_document(
            &self.word_document,
            &HeadlessQueryRequest {
                queries: vec![HeadlessQuery::ParagraphCount],
            },
        ) else {
            return 0;
        };
        response
            .results
            .into_iter()
            .find_map(|result| match result {
                HeadlessQueryResult::ParagraphCount { paragraph_count } => Some(paragraph_count),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// Finds `query` in the document.
    ///
    /// Byte offsets into the paragraph text, so a hit can be selected directly.
    pub fn find(&self, query: &str, case_insensitive: bool) -> Vec<SearchHit> {
        self.find_with_limit(query, case_insensitive, 500)
    }

    fn find_with_limit(
        &self,
        query: &str,
        case_insensitive: bool,
        max_results: usize,
    ) -> Vec<SearchHit> {
        if query.is_empty() {
            return Vec::new();
        }
        let Ok(response) = execute_headless_queries_on_document(
            &self.word_document,
            &HeadlessQueryRequest {
                queries: vec![HeadlessQuery::SearchText {
                    query: query.to_string(),
                    start_paragraph: 0,
                    max_results,
                    context_before_chars: 0,
                    context_after_chars: 0,
                    case_insensitive,
                }],
            },
        ) else {
            return Vec::new();
        };
        response
            .results
            .into_iter()
            .find_map(|result| match result {
                HeadlessQueryResult::SearchText { matches, .. } => Some(matches),
                _ => None,
            })
            .unwrap_or_default()
            .into_iter()
            .map(|hit| SearchHit {
                paragraph: hit.paragraph,
                start_offset: hit.start_offset,
                end_offset: hit.end_offset,
            })
            .collect()
    }

    /// Replaces one found hit, leaving the caret after the replacement.
    ///
    /// Offsets in a `SearchHit` are those of the document as it was searched,
    /// so replace one hit and the rest of that paragraph's hits are stale.
    pub fn replace_one(
        &mut self,
        hit: SearchHit,
        replacement: &str,
        visible_end_page_1idx: usize,
    ) -> Result<(), String> {
        self.replace_range(
            hit.paragraph,
            hit.start_offset,
            hit.end_offset,
            replacement,
            visible_end_page_1idx,
        )
        .map(|_| ())
    }

    /// Replaces every occurrence of `query`, and answers how many.
    ///
    /// One command, so one undo step puts the whole document back — replacing
    /// hit by hit would bury the previous edit under as many undos as there
    /// were matches. Matching is exact, the same as Word's find with match case
    /// on.
    pub fn replace_all(
        &mut self,
        query: &str,
        replacement: &str,
        visible_end_page_1idx: usize,
    ) -> Result<usize, String> {
        if query.is_empty() {
            return Ok(0);
        }
        // Counted before the edit: afterwards the old text is gone, and if the
        // replacement contains the query, counting after would find the new
        // occurrences instead.
        // Every replacement leaves an executor entry. The interactive search
        // limit must not truncate either the count or this undo group's size.
        let hits = self.find_with_limit(query, false, usize::MAX);
        if hits.is_empty() {
            return Ok(0);
        }
        let first = hits.iter().map(|hit| hit.paragraph).min().unwrap_or(0);
        self.apply_and_repaint_grouped(
            first,
            HeadlessCommand::ReplacePlaceholder {
                placeholder: query.to_string(),
                text: replacement.to_string(),
                max_replacements: None,
            },
            visible_end_page_1idx,
            hits.len(),
        )?;
        // The edit can move text onto later pages, and the bounded repaint above
        // only redraws as far as the caller can see. Everything after that is
        // redrawn when it scrolls into view.
        Ok(hits.len())
    }

    /// Sets the paragraph alignment of the paragraphs the selection touches.
    ///
    /// `alignment` is Word's own vocabulary: `left`, `center`, `right`, `both`.
    pub fn set_alignment(
        &mut self,
        alignment: &str,
        visible_end_page_1idx: usize,
    ) -> Result<(), String> {
        let (start, end) = self.selected_paragraph_span();
        self.apply_and_repaint(
            start,
            HeadlessCommand::SetParagraphAlignment {
                start_paragraph: start,
                end_paragraph: end,
                alignment: alignment.to_string(),
            },
            visible_end_page_1idx,
        )
        .map(|_| ())
    }

    /// Shifts the left indent of the paragraphs the selection touches.
    pub fn shift_indent(
        &mut self,
        points: f32,
        visible_end_page_1idx: usize,
    ) -> Result<(), String> {
        let (start, end) = self.selected_paragraph_span();
        self.apply_and_repaint(
            start,
            HeadlessCommand::SetParagraphIndent {
                start_paragraph: start,
                end_paragraph: end,
                left: Some(points),
                right: None,
                first_line: None,
                hanging: None,
                left_chars: None,
                right_chars: None,
                first_line_chars: None,
                hanging_chars: None,
                clear_special: false,
            },
            visible_end_page_1idx,
        )
        .map(|_| ())
    }

    /// The paragraphs the selection covers, or the caret's paragraph twice.
    ///
    /// Paragraph formatting applies to whole paragraphs, so a selection that
    /// covers part of one still formats all of it — the same as Word.
    fn selected_paragraph_span(&self) -> (usize, usize) {
        match self.selection {
            Some(range) if !range.is_collapsed() => {
                let (start, end) = range.normalized();
                (start.paragraph, end.paragraph)
            }
            _ => (self.selection_paragraph(), self.selection_paragraph()),
        }
    }

    fn selection_paragraph(&self) -> usize {
        self.selection.map(|r| r.focus.paragraph).unwrap_or(0)
    }

    /// Applies a character format across the selection.
    ///
    /// Formatting is a range command like any other, so it goes on the undo
    /// stack and repaints the same way an edit does.
    pub fn format_selection(
        &mut self,
        format: CharacterFormat,
        visible_end_page_1idx: usize,
    ) -> Result<(), String> {
        let Some(range) = self.selection.filter(|range| !range.is_collapsed()) else {
            return Err("no selection to format".to_string());
        };
        let (start, end) = range.normalized();
        let command = match format {
            CharacterFormat::Bold => HeadlessCommand::ApplyBold {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                enabled: true,
            },
            CharacterFormat::Italic => HeadlessCommand::ApplyItalic {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                enabled: true,
            },
            CharacterFormat::Underline => HeadlessCommand::ApplyUnderline {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                kind: None,
                enabled: true,
            },
            CharacterFormat::Strikethrough => HeadlessCommand::ApplyStrike {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                double: false,
                enabled: true,
            },
            CharacterFormat::Superscript => HeadlessCommand::ApplySuperscript {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                enabled: true,
            },
            CharacterFormat::Subscript => HeadlessCommand::ApplySubscript {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                enabled: true,
            },
            CharacterFormat::Clear => HeadlessCommand::ClearFormatting {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
            },
        };
        self.apply_and_repaint(start.paragraph, command, visible_end_page_1idx)?;
        // The command consumed the selection state; put it back so a second
        // format applies to the same run rather than to nothing.
        self.selection = Some(range);
        self.renderer.set_selection_range(Some(range));
        Ok(())
    }

    /// Sets the selection's font size, in points.
    pub fn set_font_size(
        &mut self,
        points: f32,
        visible_end_page_1idx: usize,
    ) -> Result<(), String> {
        self.format_selection_with(visible_end_page_1idx, |start, end| {
            HeadlessCommand::SetFontSize {
                start_paragraph: start.0,
                start_offset: start.1,
                end_paragraph: end.0,
                end_offset: end.1,
                font_size: points,
            }
        })
    }

    /// Sets the selection's font family.
    pub fn set_font_family(
        &mut self,
        family: &str,
        visible_end_page_1idx: usize,
    ) -> Result<(), String> {
        let family = family.to_string();
        self.format_selection_with(visible_end_page_1idx, move |start, end| {
            HeadlessCommand::SetFontFamily {
                start_paragraph: start.0,
                start_offset: start.1,
                end_paragraph: end.0,
                end_offset: end.1,
                font_family: family.clone(),
            }
        })
    }

    /// Sets the selection's text colour, as `#RRGGBB` or `RRGGBB`.
    ///
    /// Word writes `w:color w:val="C00000"` and CSS wants `#C00000`; what
    /// reaches the canvas is a CSS colour, and a canvas given the bare form
    /// keeps the colour it already had — which reads as "the command did
    /// nothing" rather than as a rejected value.
    pub fn set_font_color(
        &mut self,
        color: &str,
        visible_end_page_1idx: usize,
    ) -> Result<(), String> {
        let color = normalize_css_color(color);
        self.format_selection_with(visible_end_page_1idx, move |start, end| {
            HeadlessCommand::SetFontColor {
                start_paragraph: start.0,
                start_offset: start.1,
                end_paragraph: end.0,
                end_offset: end.1,
                color: color.clone(),
            }
        })
    }

    /// Highlights the selection. Word's own highlight names — `yellow`,
    /// `green`, `cyan`, `none` to remove it.
    pub fn set_highlight_color(
        &mut self,
        color: &str,
        visible_end_page_1idx: usize,
    ) -> Result<(), String> {
        let color = color.to_string();
        self.format_selection_with(visible_end_page_1idx, move |start, end| {
            HeadlessCommand::SetHighlightColor {
                start_paragraph: start.0,
                start_offset: start.1,
                end_paragraph: end.0,
                end_offset: end.1,
                color: color.clone(),
            }
        })
    }

    /// Applies a run-level command to the selection and keeps the selection.
    ///
    /// The command consumes the selection state, and a toolbar applies one
    /// thing after another to the same run — bold, then a size, then a colour.
    fn format_selection_with(
        &mut self,
        visible_end_page_1idx: usize,
        command: impl FnOnce((usize, usize), (usize, usize)) -> HeadlessCommand,
    ) -> Result<(), String> {
        let Some(range) = self.selection.filter(|range| !range.is_collapsed()) else {
            return Err("no selection to format".to_string());
        };
        let (start, end) = range.normalized();
        let command = command((start.paragraph, start.offset), (end.paragraph, end.offset));
        self.apply_and_repaint(start.paragraph, command, visible_end_page_1idx)?;
        self.selection = Some(range);
        self.renderer.set_selection_range(Some(range));
        Ok(())
    }

    /// Sets the line height of the paragraphs the selection touches, as a
    /// multiple of a single line.
    ///
    /// The command accepts the multiplier directly; DOCX serialization converts
    /// it to 240ths of a line under the `auto` rule.
    pub fn set_line_spacing(
        &mut self,
        multiple: f32,
        visible_end_page_1idx: usize,
    ) -> Result<(), String> {
        let (start, end) = self.selected_paragraph_span();
        self.apply_and_repaint(
            start,
            HeadlessCommand::SetParagraphLineSpacing {
                start_paragraph: start,
                end_paragraph: end,
                line: Some(Some(multiple)),
                line_rule: Some("auto".to_string()),
                before: None,
                after: None,
            },
            visible_end_page_1idx,
        )
        .map(|_| ())
    }

    /// Sets the space before and after the paragraphs the selection touches,
    /// in points.
    pub fn set_paragraph_spacing(
        &mut self,
        before: Option<f32>,
        after: Option<f32>,
        visible_end_page_1idx: usize,
    ) -> Result<(), String> {
        let (start, end) = self.selected_paragraph_span();
        self.apply_and_repaint(
            start,
            HeadlessCommand::SetParagraphLineSpacing {
                start_paragraph: start,
                end_paragraph: end,
                line: None,
                line_rule: None,
                before,
                after,
            },
            visible_end_page_1idx,
        )
        .map(|_| ())
    }

    /// Shades the paragraphs the selection touches, or clears the shading with
    /// `None`.
    pub fn set_paragraph_shading(
        &mut self,
        color: Option<&str>,
        visible_end_page_1idx: usize,
    ) -> Result<(), String> {
        let (start, end) = self.selected_paragraph_span();
        self.apply_and_repaint(
            start,
            HeadlessCommand::SetParagraphShading {
                start_paragraph: start,
                end_paragraph: end,
                color: color.map(normalize_css_color),
            },
            visible_end_page_1idx,
        )
        .map(|_| ())
    }

    /// Puts a box around the paragraphs the selection touches, or takes it off.
    pub fn set_paragraph_borders(
        &mut self,
        boxed: bool,
        visible_end_page_1idx: usize,
    ) -> Result<(), String> {
        let (start, end) = self.selected_paragraph_span();
        let edge = boxed.then(|| HeadlessBorderEdge {
            style: Some("single".to_string()),
            width: Some(0.5),
            color: Some("#000000".to_string()),
        });
        self.apply_and_repaint(
            start,
            HeadlessCommand::SetParagraphBorders {
                start_paragraph: start,
                end_paragraph: end,
                top: edge.clone(),
                bottom: edge.clone(),
                left: edge.clone(),
                right: edge,
            },
            visible_end_page_1idx,
        )
        .map(|_| ())
    }

    /// Sets the page colour, or clears it with `None`.
    pub fn set_page_background(&mut self, color: Option<&str>) -> Result<(), String> {
        let pages = self.page_count().max(1);
        self.apply_and_repaint(
            0,
            HeadlessCommand::SetPageBackgroundColor {
                color: color.map(normalize_css_color),
            },
            pages,
        )
        .map(|_| ())
    }

    /// Puts a border around every page, or clears it.
    pub fn set_page_borders(&mut self, boxed: bool) -> Result<(), String> {
        let pages = self.page_count().max(1);
        let edge = boxed.then(|| HeadlessBorderEdge {
            style: Some("single".to_string()),
            width: Some(1.0),
            color: Some("#000000".to_string()),
        });
        self.apply_and_repaint(
            0,
            HeadlessCommand::SetPageBorders {
                clear: !boxed,
                top: edge.clone(),
                bottom: edge.clone(),
                left: edge.clone(),
                right: edge,
                offset_from: Some("text".to_string()),
            },
            pages,
        )
        .map(|_| ())
    }

    /// Applies a paragraph style — `Normal`, `Heading1`, and so on — to the
    /// paragraphs the selection touches.
    pub fn set_paragraph_style(
        &mut self,
        style_id: &str,
        visible_end_page_1idx: usize,
    ) -> Result<(), String> {
        let (start, end) = self.selected_paragraph_span();
        self.apply_and_repaint(
            start,
            HeadlessCommand::SetParagraphStyle {
                start_paragraph: start,
                end_paragraph: end,
                style_id: Some(style_id.to_string()),
            },
            visible_end_page_1idx,
        )
        .map(|_| ())
    }

    /// Turns the paragraphs the selection touches into a list, or back into
    /// plain paragraphs with `None`.
    pub fn set_numbering(
        &mut self,
        num_id: Option<&str>,
        visible_end_page_1idx: usize,
    ) -> Result<(), String> {
        let (start, end) = self.selected_paragraph_span();
        self.apply_and_repaint(
            start,
            HeadlessCommand::SetNumbering {
                start_paragraph: start,
                end_paragraph: end,
                num_id: num_id.map(str::to_string),
                level: num_id.map(|_| 0),
            },
            visible_end_page_1idx,
        )
        .map(|_| ())
    }

    /// The text of the current selection, for the clipboard.
    ///
    /// Selections spanning paragraphs join with newlines, which is what a
    /// paste back into the document turns into paragraph breaks.
    pub fn selected_text(&self) -> Option<String> {
        let range = self.selection.filter(|range| !range.is_collapsed())?;
        let (start, end) = range.normalized();
        if start.paragraph == end.paragraph {
            let text = self.paragraph_text(start.paragraph)?;
            return Some(text.get(start.offset..end.offset)?.to_string());
        }
        let mut parts = Vec::new();
        parts.push(
            self.paragraph_text(start.paragraph)?
                .get(start.offset..)?
                .to_string(),
        );
        for paragraph in (start.paragraph + 1)..end.paragraph {
            parts.push(self.paragraph_text(paragraph).unwrap_or_default());
        }
        parts.push(
            self.paragraph_text(end.paragraph)?
                .get(..end.offset)?
                .to_string(),
        );
        Some(parts.join("\n"))
    }

    /// The current selection as `(anchor, focus)`, or `None` when it is
    /// collapsed to a caret.
    pub fn selection(&self) -> Option<((usize, usize), (usize, usize))> {
        let range = self.selection?;
        (!range.is_collapsed()).then_some((
            (range.anchor.paragraph, range.anchor.offset),
            (range.focus.paragraph, range.focus.offset),
        ))
    }

    /// Replaces the selection with `text` (empty deletes it) and repaints.
    ///
    /// Returns where the caret ends up. Selections that span paragraphs go
    /// through the same range command, which is what makes a multi-paragraph
    /// delete one edit rather than a loop.
    pub fn replace_selection(
        &mut self,
        text: &str,
        visible_end_page_1idx: usize,
    ) -> Result<(usize, usize), String> {
        let Some(range) = self.selection.filter(|range| !range.is_collapsed()) else {
            return Err("no selection to replace".to_string());
        };
        let (start, end) = range.normalized();
        self.apply_and_repaint(
            start.paragraph,
            HeadlessCommand::ReplaceRange {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                text: text.to_string(),
            },
            visible_end_page_1idx,
        )?;
        // A cross-paragraph replacement pulls the tail up, so the caret lands
        // in the paragraph the selection started in.
        let caret = (start.paragraph, start.offset + text.len());
        self.selection = Some(SelectionRange::collapsed(TextPosition::new(
            caret.0, caret.1,
        )));
        self.renderer.set_selection_range(self.selection);
        Ok(caret)
    }

    /// Moves the caret to `(paragraph, offset)` and reports where it landed on
    /// the page, or `None` if that position has no geometry.
    ///
    /// The caret is not painted onto the page — it is returned so the shell can
    /// draw it as an overlay, which is what lets it move and blink without
    /// repainting the document.
    pub fn place_caret(&mut self, paragraph: usize, offset: usize) -> Option<CaretRect> {
        self.place_caret_with_affinity(paragraph, offset, CaretAffinity::Upstream)
    }

    /// [`Self::place_caret`], saying which line a caret on a soft wrap is
    /// drawn on: downstream for a click at a line's start, upstream for one
    /// past a line's end.
    pub fn place_caret_with_affinity(
        &mut self,
        paragraph: usize,
        offset: usize,
        affinity: CaretAffinity,
    ) -> Option<CaretRect> {
        // A query has no error to report; on failure it answers from the
        // layout there is.
        let _ = self.ensure_layout_reaches(paragraph);
        self.renderer.set_caret_affinity(affinity);
        let range = SelectionRange::collapsed(TextPosition::new(paragraph, offset));
        self.selection = Some(range);
        self.renderer.set_selection_range(Some(range));
        self.renderer.caret_metrics().map(|metrics| CaretRect {
            x: metrics.x,
            document_y: metrics.absolute_top,
            width: metrics.width.max(1.0),
            height: metrics.height,
        })
    }

    /// Where a caret at `(paragraph, offset)` would be, without moving the
    /// caret or the selection there — for an overlay that marks a place,
    /// such as the box around a master document's subdocument.
    pub fn position_rect(&mut self, paragraph: usize, offset: usize) -> Option<CaretRect> {
        let _ = self.ensure_layout_reaches(paragraph);
        let selection = self.renderer.selection_range();
        self.renderer
            .set_selection_range(Some(SelectionRange::collapsed(TextPosition::new(
                paragraph, offset,
            ))));
        let rect = self.renderer.caret_metrics().map(|metrics| CaretRect {
            x: metrics.x,
            document_y: metrics.absolute_top,
            width: metrics.width.max(1.0),
            height: metrics.height,
        });
        // A structural edit can leave a range referring to the previous
        // paragraph contents. A geometry query must restore it exactly, not
        // clamp it and make the renderer disagree with the shell's selection.
        self.renderer.set_selection_range_for_render(selection);
        rect
    }

    /// The text position under a point, in the renderer's document coordinate
    /// space.
    ///
    /// Populated by rendering, so it reflects the last paint — which is what a
    /// click should hit. It cannot lay the document out on demand: on a page
    /// past a bounded edit's bound it answers from the layout before the
    /// edit until that page is drawn again ([`Self::paint_stale_pages`]),
    /// or a caret query lays the document out.
    pub fn position_at(&self, x: f64, y: f64) -> Option<(usize, usize)> {
        self.renderer
            .hit_test_position(x, y)
            .map(|position| (position.paragraph, position.offset))
    }

    /// Total `fill_text` calls on a page, for diagnosing a repaint that runs
    /// but does not draw.
    pub fn page_fill_text_calls(&self, page_index: usize) -> u64 {
        self.pages
            .borrow()
            .get(page_index)
            .map(|p| p.fill_text_calls())
            .unwrap_or(0)
    }

    /// Distance between the tops of two consecutive pages, in the renderer's
    /// document coordinate space.
    ///
    /// Not the page height: the renderer leaves a gap between pages, so a
    /// mapping that uses the height alone drifts by one gap per page — correct
    /// on page 1 and wrong everywhere after it.
    /// The page box layout is using, which is also the size of every surface.
    pub fn layout_page_size(&self) -> (f64, f64) {
        self.renderer.layout_page_size()
    }

    pub fn page_pitch(&self) -> f64 {
        self.page_height + jade_docx_editor::render_host::PAGE_GAP_PX
    }

    /// The box of page `page_index` as it was laid out: a section turned
    /// landscape, or on other paper, has pages of its own size.
    pub fn page_box(&self, page_index: usize) -> (f64, f64) {
        self.renderer.page_box(page_index)
    }

    /// The document coordinate of a point on `page_index`: the pages before
    /// it count each at its own height.
    pub fn document_y(&self, page_index: usize, y_on_page: f64) -> f64 {
        y_on_page + self.renderer.page_top_y(page_index)
    }

    /// Which page a document coordinate falls on, and where on it.
    pub fn page_and_offset(&self, document_y: f64) -> (usize, f64) {
        let page = self.renderer.page_for_y(document_y.max(0.0));
        (page, document_y - self.renderer.page_top_y(page))
    }

    /// The page box layout ends on: the last section's, resolved as the
    /// renderer resolves it (the section's own page settings, then the
    /// document default section's, then the document's), folded for two
    /// pages per sheet or a booklet. `None` when the properties give no
    /// usable box; [`Self::follow_page_box`] still catches any box layout
    /// ends on that this did not predict.
    fn layout_page_size_of(document: &jade_docx_reader::Document) -> Option<(f64, f64)> {
        use jade_docx_layout::section_transition::{
            folded_page_size, DEFAULT_PAGE_HEIGHT_PT, DEFAULT_PAGE_WIDTH_PT,
        };
        let settings = document
            .sections
            .last()
            .and_then(|section| section.properties.as_ref())
            .and_then(|properties| properties.page_settings.as_ref())
            .or_else(|| {
                document
                    .default_section
                    .as_ref()
                    .and_then(|properties| properties.page_settings.as_ref())
            })
            .or(document.page_settings.as_ref());
        let (width_pt, height_pt) = folded_page_size(
            settings
                .and_then(|settings| settings.width)
                .unwrap_or(DEFAULT_PAGE_WIDTH_PT),
            settings
                .and_then(|settings| settings.height)
                .unwrap_or(DEFAULT_PAGE_HEIGHT_PT),
            document.print_two_on_one,
            document.book_fold_printing,
        );
        let to_px = |points: f32| f64::from(points) * 96.0 / 72.0;
        let (width, height) = (to_px(width_pt), to_px(height_pt));
        (width > 1.0 && height > 1.0).then_some((width, height))
    }

    pub fn page_count(&self) -> usize {
        // The host's surfaces are a pool that a shrinking document leaves
        // entries in, so the count comes from what the last render produced.
        self.rendered_pages.get()
    }

    /// Identifies the current raster contents of a rendered page.
    pub fn page_raster_generation(&self, page_index: usize) -> Option<u32> {
        self.pages.borrow().get(page_index)?.raster_generation()
    }

    /// A page's pixels as BGRA with premultiplied alpha, plus its size.
    pub fn page_pixels(&self, page_index: usize) -> Option<(Vec<u8>, i32, i32)> {
        self.pages
            .borrow()
            .get(page_index)?
            .read_bgra_premultiplied()
    }

    pub fn insert_text(
        &mut self,
        paragraph: usize,
        offset: usize,
        text: &str,
        visible_end_page_1idx: usize,
    ) -> Result<Duration, String> {
        self.apply_and_repaint(
            paragraph,
            HeadlessCommand::InsertText {
                paragraph,
                offset,
                text: text.to_string(),
            },
            visible_end_page_1idx,
        )
    }

    /// Replaces a byte range within one paragraph and repaints.
    ///
    /// Composition needs this: a preedit is replaced in place as the reading
    /// changes, so an insert-only API would leave every intermediate behind.
    pub fn replace_range(
        &mut self,
        paragraph: usize,
        start_offset: usize,
        end_offset: usize,
        text: &str,
        visible_end_page_1idx: usize,
    ) -> Result<Duration, String> {
        self.apply_and_repaint(
            paragraph,
            HeadlessCommand::ReplaceRange {
                start_paragraph: paragraph,
                start_offset,
                end_paragraph: paragraph,
                end_offset,
                text: text.to_string(),
            },
            visible_end_page_1idx,
        )
    }

    /// Deletes `count` characters before or after `offset` and repaints.
    ///
    /// Backspace and Delete differ only in direction, and both are counted in
    /// characters rather than bytes — deleting three bytes off a Japanese
    /// character would leave a broken one behind.
    pub fn delete_text(
        &mut self,
        paragraph: usize,
        offset: usize,
        backwards: bool,
        count: usize,
        visible_end_page_1idx: usize,
    ) -> Result<Duration, String> {
        self.apply_and_repaint(
            paragraph,
            HeadlessCommand::DeleteText {
                paragraph,
                offset,
                direction: if backwards {
                    HeadlessDeleteDirection::Backward
                } else {
                    HeadlessDeleteDirection::Forward
                },
                count,
            },
            visible_end_page_1idx,
        )
    }

    /// Splits the paragraph at `offset` and repaints.
    ///
    /// A break re-flows everything after it, so unlike an in-paragraph edit
    /// this cannot be bounded to the visible page without the pages below going
    /// stale. The caller gets the whole document laid out again.
    pub fn insert_paragraph_break(
        &mut self,
        paragraph: usize,
        offset: usize,
    ) -> Result<Duration, String> {
        self.apply_and_repaint(
            paragraph,
            HeadlessCommand::InsertParagraphBreak {
                paragraph,
                offset,
                count: 1,
            },
            // Splitting can create more pages than the previous pagination.
            // The old count would stop before the newly displaced tail.
            usize::MAX,
        )
    }

    /// Where the paragraph a split of `paragraph` made is: the next one of
    /// its story, past the paragraphs of the notes it references (they
    /// follow their reference), as `InsertParagraphBreak` puts the caret.
    pub fn paragraph_after_split(&self, paragraph: usize) -> usize {
        let document = &self.word_document;
        if !document.is_body_paragraph_surface(paragraph) {
            return paragraph + 1;
        }
        (paragraph + 1..document.paragraph_count())
            .find(|&index| document.is_body_paragraph_surface(index))
            .unwrap_or(paragraph + 1)
    }

    /// The plain text of one paragraph.
    ///
    /// An IME works over a run of text with offsets into it, and a paragraph is
    /// the natural unit: it is what an edit command addresses, and a
    /// composition does not span one.
    pub fn paragraph_text(&self, paragraph: usize) -> Option<String> {
        let response = execute_headless_queries_on_document(
            &self.word_document,
            &HeadlessQueryRequest {
                queries: vec![HeadlessQuery::ParagraphSlice {
                    start_paragraph: paragraph,
                    max_paragraphs: 1,
                }],
            },
        )
        .ok()?;
        response
            .results
            .into_iter()
            .find_map(|result| match result {
                HeadlessQueryResult::ParagraphSlice { paragraphs, .. } => paragraphs
                    .into_iter()
                    .find(|entry| entry.paragraph == paragraph)
                    .map(|entry| entry.text),
                _ => None,
            })
    }

    /// One paragraph's text in the caret's offsets: an inline picture, text
    /// box or equation is one object character (U+FFFC), as Word counts it,
    /// so the caret steps over it in one.
    pub fn caret_text(&self, paragraph: usize) -> Option<String> {
        self.word_document
            .paragraph(paragraph)
            .map(|_| self.word_document.rendered_paragraph_text(paragraph))
    }

    // ---- for a shell's own commands ------------------------------------------

    /// The document model, for reading what a command needs to address —
    /// styles, numbering, table dimensions.
    ///
    /// Until the round-trip tree is built (the first edit or
    /// [`Self::ensure_roundtrip_xml`]), readers backed by the XML — content
    /// controls, legacy form fields, fields, tables of contents,
    /// cross-reference targets, revisions, headings collapsed by default,
    /// editable ranges, alt text, rendered paragraph lengths, the XML half of
    /// paragraph and table formats — are empty or wrong. Callers that read
    /// those call [`Self::ensure_roundtrip_xml`] first.
    pub fn word_document(&self) -> &WordDocument {
        &self.word_document
    }

    /// The renderer, for what only the last paint knows: which table cell a
    /// paragraph was laid out in.
    pub fn renderer(&self) -> &CanvasRenderer<SkiaPainter> {
        &self.renderer
    }

    /// Capture a from-start layout pass, including painted auxiliary stories
    /// such as numbering labels and ruby guides. This deliberately expensive
    /// diagnostic bypasses lazy layout; callers must inspect the snapshot's
    /// completeness and uncaptured-story fields before treating it as coverage.
    pub fn capture_full_layout_debug(&mut self) -> Result<FullLayoutDebugSnapshot, String> {
        self.renderer
            .capture_full_layout_debug()
            .map_err(|_| "full layout capture failed".to_owned())
    }

    /// Capture painted repeated-table-header glyphs without changing ordinary
    /// diagnostic capture or the rendered document.
    pub fn capture_full_layout_debug_painted_table_headers(
        &mut self,
    ) -> Result<FullLayoutDebugSnapshot, String> {
        self.renderer
            .capture_full_layout_debug_painted_table_headers()
            .map_err(|_| "painted table header capture failed".to_owned())
    }

    /// Capture original table glyph origins even when painter clipping hides
    /// them, for comparison with Word's complete EMF text records.
    pub fn capture_full_layout_debug_unclipped_tables(
        &mut self,
    ) -> Result<FullLayoutDebugSnapshot, String> {
        self.renderer
            .capture_full_layout_debug_unclipped_tables()
            .map_err(|_| "unclipped table layout capture failed".to_owned())
    }

    /// The clock `DATE` and `TIME` fields show from the next repaint, as
    /// Word updates them when a document is opened or printed; `None` keeps
    /// their saved results.
    pub fn set_field_clock(
        &mut self,
        now: Option<jade_docx_editor::canvas_renderer::FieldDateTime>,
    ) {
        self.renderer.set_current_datetime(now);
    }

    /// The selection as the engine holds it, a caret when collapsed.
    pub fn selection_range(&self) -> Option<SelectionRange> {
        self.selection
    }

    /// Sets the selection without repainting: a highlight that was painted
    /// stays until the next repaint.
    pub fn set_selection_range(&mut self, range: Option<SelectionRange>) {
        self.selection = range;
        self.renderer.set_selection_range(range);
    }

    /// Applies any headless command as one undo step and repaints the whole
    /// document — what a ribbon command needs, whose effect can reach any
    /// page. With `keep_selection` the selection it acted on survives, so the
    /// next command acts on the same text; otherwise it is wherever the
    /// command left it.
    pub fn apply_command(
        &mut self,
        command: HeadlessCommand,
        keep_selection: bool,
    ) -> Result<(), String> {
        self.apply_commands(vec![command], keep_selection)
    }

    /// Runs `edit` on the document with the renderer's hold on it let go,
    /// then points the renderer at the result.
    ///
    /// The renderer keeps the document it last laid out, so changing a
    /// paragraph through the shared `Rc` copied the whole document first —
    /// on every keystroke, at a cost that grew with the document. Let go of,
    /// the document is changed where it is, as `canvas_view` does. Letting go
    /// also forgets the paragraph hashes the renderer keys by address, which
    /// an edit in place would otherwise leave stale; the copy forgot them too.
    fn edit_in_place<R>(&mut self, edit: impl FnOnce(&mut Self) -> R) -> R {
        debug_assert!(
            self.word_document.raw_document.is_some(),
            "an edit ran before ensure_roundtrip_xml"
        );
        self.renderer.drop_document_cache();
        self.renderer.note_style_reference_edit();
        let result = edit(self);
        if let Some(document) = &self.word_document.document {
            self.renderer.restore_document_cache(Rc::clone(document));
        }
        result
    }

    /// Builds the round-trip XML tree that saving patches, which opening
    /// leaves out, and repaints once so the layouts carry the node ids it
    /// attaches. The first paint needs only the model, and the tree is a
    /// second pass over `document.xml`; every edit and undo builds it first,
    /// and a shell calls this in idle time after the first frame so the first
    /// keystroke does not pay for it. Returns whether it did anything — built
    /// the tree or laid the document out again — so a shell knows whether
    /// the model's readers or the page images it holds may have changed. The
    /// pixels do not: the pages are drawn again, none is left stale. Does
    /// nothing once built; a tree that failed to parse fails again without
    /// being parsed again.
    ///
    /// Until it runs, readers of [`Self::word_document`] backed by the tree
    /// see nothing of it: content controls, legacy form fields, fields,
    /// tables of contents, cross-reference targets, revisions, headings
    /// collapsed by default, editable ranges, alt text, rendered paragraph
    /// lengths, and the XML half of paragraph and table formats. A caller
    /// that reads those, or reads anything the open set from them, calls
    /// this first.
    pub fn ensure_roundtrip_xml(&mut self) -> Result<bool, String> {
        let built = self.build_roundtrip_xml()?;
        if !self.roundtrip_relayout_pending {
            return Ok(built);
        }
        // Only node ids changed, so the pages left stale and those not are
        // as they were: lay everything out, draw the window, mark nothing.
        self.repaint_all_drawing(self.paint_window.clone(), false)?;
        self.bounded_pass_since_full_layout = false;
        Ok(true)
    }

    /// [`Self::ensure_roundtrip_xml`] without the repaint, for an edit that
    /// repaints the whole document after it anyway.
    fn build_roundtrip_xml(&mut self) -> Result<bool, String> {
        if self.word_document.raw_document.is_some() {
            return Ok(false);
        }
        if let Some(error) = &self.roundtrip_xml_error {
            return Err(error.clone());
        }
        // Attaching node ids writes to the model; with the renderer still
        // holding it, that would copy the whole document first.
        self.renderer.drop_document_cache();
        let built = self.word_document.ensure_roundtrip_xml();
        if let Some(document) = &self.word_document.document {
            self.renderer.restore_document_cache(Rc::clone(document));
        }
        match built {
            Ok(true) => {
                ROUNDTRIP_XML_BUILDS.with(|builds| builds.set(builds.get() + 1));
                self.roundtrip_relayout_pending = true;
                Ok(true)
            }
            Ok(false) => Ok(false),
            Err(error) => {
                let error = format!("round-trip XML failed: {error}");
                self.roundtrip_xml_error = Some(error.clone());
                Err(error)
            }
        }
    }

    /// [`Self::apply_command`] for several commands that are one action — a
    /// paste of formatted runs, a replace of matches the engine's own replace
    /// cannot find — applied in order and undone as one step. When one of
    /// them fails, the ones before it are taken back off the document and
    /// nothing is recorded.
    pub fn apply_commands(
        &mut self,
        commands: Vec<HeadlessCommand>,
        keep_selection: bool,
    ) -> Result<(), String> {
        if commands.is_empty() {
            return Ok(());
        }
        self.build_roundtrip_xml()?;
        let before = self.selection;
        let recorded = self.executor.history_revision();
        let reuse_first_layout = self.can_reuse_table_layout();
        self.executor.begin_group();
        let after = self.edit_in_place(|this| {
            let mut measure = |document: &WordDocument, index, table_order| {
                let Some(id) = document
                    .table_by_order(table_order)
                    .and_then(|table| table.node_id)
                else {
                    return Ok(None);
                };
                if index == 0 && reuse_first_layout {
                    return Ok(this.renderer.nested_column_resize_geometry_for_node(id));
                }
                if !Self::has_windows_nested_body_tables(document) {
                    return Ok(None);
                }
                let renderer = Self::measure_tables(
                    document,
                    (this.page_width, this.page_height),
                    &this.fonts,
                    &this.math_faces,
                    this.field_datetime,
                    &this.options,
                )
                .map_err(CommandError::Custom)?;
                Ok(renderer.nested_column_resize_geometry_for_node(id))
            };
            let result = apply_headless_commands_with_resize_measurement(
                &mut this.word_document,
                &mut this.executor,
                this.selection.take(),
                &HeadlessEditRequest { commands },
                &mut measure,
            );
            match result {
                Ok(after) => {
                    this.executor.end_group();
                    Ok(after)
                }
                Err(err) => {
                    let mut selection = before;
                    let mut ctx = CommandContext::new(&mut this.word_document, &mut selection);
                    this.executor.abort_group(&mut ctx);
                    Err(err)
                }
            }
        });
        let after = match after {
            Ok(after) => after,
            Err(err) => {
                self.selection = before;
                self.repaint_all()?;
                return Err(format!("edit failed: {err:?}"));
            }
        };
        self.finish_recorded_edit(recorded, before, after, keep_selection)
    }

    /// Executes a command the shell built itself — one the headless API has
    /// no form for, such as a picture with its bytes — as one undo step, and
    /// repaints the whole document.
    pub fn execute_command(
        &mut self,
        command: Box<dyn Command>,
        keep_selection: bool,
    ) -> Result<(), String> {
        self.build_roundtrip_xml()?;
        let before = self.selection;
        let recorded = self.executor.history_revision();
        let mut selection = self.selection;
        let result = self.edit_in_place(|this| {
            let mut ctx = CommandContext::new(&mut this.word_document, &mut selection);
            this.executor.execute(command, &mut ctx)
        });
        if let Err(err) = result {
            self.selection = before;
            return Err(format!("edit failed: {err:?}"));
        }
        self.finish_recorded_edit(recorded, before, selection, keep_selection)
    }

    /// The tail of an edit through the executor: one step on this history
    /// when it recorded anything, the selection, and a whole repaint.
    fn finish_recorded_edit(
        &mut self,
        recorded: u64,
        before: Option<SelectionRange>,
        after: Option<SelectionRange>,
        keep_selection: bool,
    ) -> Result<(), String> {
        if self.executor.history_revision() != recorded {
            self.redo_history.clear();
            self.history
                .push(HistoryStep::Command { executor_steps: 1 });
        }
        self.selection = if keep_selection {
            before
        } else {
            after.or(before)
        };
        self.repaint_all().map(|_| ())
    }

    /// Applies one command and repaints, returning how long the repaint took.
    ///
    /// `visible_end_page_1idx` is the last page on screen. Bounding the work to
    /// it is most of why an edit is cheap — but it also means the edited
    /// paragraph has to be on a page within that bound, or the repaint redraws
    /// unchanged pages and nothing appears.
    fn apply_and_repaint(
        &mut self,
        paragraph: usize,
        command: HeadlessCommand,
        visible_end_page_1idx: usize,
    ) -> Result<Duration, String> {
        self.apply_and_repaint_grouped(paragraph, command, visible_end_page_1idx, 1)
    }

    /// As [`Self::apply_and_repaint`], grouping a command's multiple executor
    /// edits before they can evict older undo entries.
    fn apply_and_repaint_grouped(
        &mut self,
        paragraph: usize,
        command: HeadlessCommand,
        visible_end_page_1idx: usize,
        executor_steps: usize,
    ) -> Result<Duration, String> {
        self.ensure_roundtrip_xml()?;
        // IME updates replace the previous marked text in the same paragraph.
        // Capture the same reuse candidates as typing; the recorded command's
        // footprint below must still prove that no other paragraph/part changed.
        let local_text_edit = match &command {
            HeadlessCommand::InsertText { .. } => true,
            // Backspace and Delete within the paragraph; one that joins two
            // paragraphs fails the footprint check below.
            HeadlessCommand::DeleteText {
                paragraph: deleted_in,
                ..
            } => *deleted_in == paragraph,
            HeadlessCommand::ReplaceRange {
                start_paragraph,
                end_paragraph,
                ..
            } => *start_paragraph == paragraph && *end_paragraph == paragraph,
            _ => false,
        };
        let old_height = local_text_edit
            .then(|| self.renderer.cached_paragraph_height(paragraph))
            .flatten();
        let extent_before = self.paragraph_extent(paragraph);
        let text_edit_cache = local_text_edit.then(|| {
            self.renderer
                .local_text_edit_cache_for_paragraph(&self.word_document, paragraph)
        });
        let recorded = self.executor.history_revision();
        let before = self.selection;
        let group_commands = executor_steps > 1;
        // Through the executor so the edit lands on the undo stack. The plain
        // apply does the same edit without recording anything.
        let after = self.edit_in_place(|this| {
            if group_commands {
                this.executor.begin_group();
            }
            let result = apply_headless_commands_with_executor_to_document(
                &mut this.word_document,
                &mut this.executor,
                this.selection.take(),
                &HeadlessEditRequest {
                    commands: vec![command],
                },
            );
            if group_commands {
                match result {
                    Ok(after) => {
                        this.executor.end_group();
                        Ok(after)
                    }
                    Err(err) => {
                        let mut selection = before;
                        let mut ctx = CommandContext::new(&mut this.word_document, &mut selection);
                        this.executor.abort_group(&mut ctx);
                        Err(err)
                    }
                }
            } else {
                result
            }
        });
        self.selection = match after {
            Ok(after) => after,
            Err(err) => {
                self.selection = before;
                if group_commands {
                    self.repaint_all()?;
                }
                return Err(format!("edit failed: {err:?}"));
            }
        };
        // Rejected and no-op edits preserve the redo branch and group sizes,
        // just as the executor does. Only an actual edit records a shell step.
        if self.executor.history_revision() != recorded {
            self.redo_history.clear();
            self.history
                .push(HistoryStep::Command { executor_steps: 1 });
        }
        self.renderer.set_selection_range_for_render(self.selection);
        // Structural commands can renumber every following table paragraph.
        // Their executor footprint requires a full render, just like history
        // replay; a seek from the caret can retain stale earlier table pixels.
        if self.executor.history_revision() != recorded
            && self.executor.next_undo_render_footprint()
                == Some(HistoryRenderFootprint::Structural)
        {
            return self.repaint_all();
        }

        let edited = self
            .word_document
            .document
            .as_ref()
            .ok_or_else(|| "document went missing".to_string())?
            .clone();

        let started = Instant::now();
        // Use the same geometry-checked reuse as browser typing. The recorded
        // command must prove it only touched this paragraph: InsertText can
        // instead invoke a content-control or form-field action.
        let single_paragraph_edit = executor_steps == 1
            && self.executor.history_revision() != recorded
            && self.executor.next_undo_render_footprint()
                == Some(HistoryRenderFootprint::SingleParagraph { index: paragraph });
        if single_paragraph_edit {
            // Native pages retain their pixels between edits. Let the renderer
            // redraw only this paragraph's band when its geometry is stable;
            // unsupported layouts and height changes fall back to a full page.
            // Hyperlink registration is idempotent against the regions the
            // render retained for paragraphs before the edit, so replaying them
            // adds nothing, and the edited paragraph's regions were dropped by
            // the `dirty_from` retain and register afresh. Linked documents can
            // take the band too.
            let band_safe = local_text_edit;
            self.renderer.set_band_redraw_enabled(band_safe);
            if let Some(cache) = text_edit_cache {
                self.renderer
                    .restore_local_text_edit_cache(cache, &self.word_document, paragraph);
            }
            if let Some(height) = old_height {
                self.renderer
                    .prepare_completed_pagination_edit(paragraph, height);
            }
        }
        // The caller's bound, raised to the last page the edited paragraph
        // reached: the lines it carries onto the next page reflow too, and
        // are laid out and drawn with the rest of it.
        let visible_end_page_1idx = match extent_before.filter(|_| single_paragraph_edit) {
            Some(extent) => visible_end_page_1idx
                .max(self.renderer.page_for_y((extent.bottom - 0.5).max(0.0)) + 1),
            None => visible_end_page_1idx,
        };
        // Bounded to what the caller can see, so the pages it stops at are not
        // the document's page count.
        self.pass_covers_whole_document
            .set(visible_end_page_1idx >= self.page_count());
        self.bounded_pass_since_full_layout = true;
        let bounded = visible_end_page_1idx < self.page_count();
        // A pass that stops at its bound drops the line registry, the cells
        // and the regions of the paragraphs after it; they are put back if
        // the layout after the edit turns out to stand.
        if single_paragraph_edit && bounded {
            self.renderer.set_aside_edit_downstream(paragraph);
        }
        // The sequence lives on the renderer, so this shell and the browser one
        // cannot drift apart on it. Reproducing it here is what produced three
        // repaints that ran and drew nothing.
        self.renderer
            .prepare_incremental_repaint(paragraph, visible_end_page_1idx);
        // The walk stops once it has crossed the bound, after laying out the
        // first lines of the next page; drawn, they would leave that page
        // cleared but for them. It is not drawn, and keeps what it showed.
        if bounded {
            self.renderer
                .set_visible_page_range(Some(1..=visible_end_page_1idx.max(1)));
        }
        // The other parts go with the body. Passing `None` for them renders a
        // document with no headers, no footers and no list numbering, which
        // looks like a layout difference rather than parts left behind.
        let result = self.renderer.render_bounded_to_page_with_parts(
            edited,
            &self.options,
            self.word_document.numbering_part.as_ref(),
            self.word_document.header_lines.as_deref(),
            self.word_document.footer_lines.as_deref(),
            Some(&self.word_document.comments),
            visible_end_page_1idx,
        );
        if bounded {
            self.renderer.clear_visible_page_range();
        }
        self.renderer.set_band_redraw_enabled(false);
        // Whatever the pass was, the next one is a whole-document render until
        // something says otherwise.
        self.pass_covers_whole_document.set(true);
        result.map_err(|_| "repaint failed".to_string())?;
        if let Some(error) = self.renderer.take_render_error() {
            return Err(error);
        }
        // The layout after the edit stands when the renderer proved the
        // pagination unchanged, or when the edited paragraph's lines begin and
        // end where they did and every paragraph after it that the pass laid
        // out again kept its lines.
        // A repeated table header row or a STYLEREF shows the edited text on
        // other pages too, which neither lines staying put nor a proven
        // pagination vouch for.
        let shown_elsewhere =
            single_paragraph_edit && self.renderer.paragraph_text_shows_elsewhere(paragraph);
        let downstream_stood = self.renderer.take_edit_downstream().is_some_and(|saved| {
            let stood = extent_before.is_some()
                && !shown_elsewhere
                && self.paragraph_extent(paragraph) == extent_before
                && self.renderer.edit_downstream_kept_its_lines(&saved);
            if stood {
                self.renderer.restore_edit_downstream(saved);
            }
            stood
        });
        if shown_elsewhere {
            // A STYLEREF also shows the text on the paragraph's own page, or
            // on the pages before it, in a header the pass did not draw again.
            let count = self.page_count();
            let from = self
                .renderer
                .first_page_showing_style_ref_of(paragraph)
                .map_or(visible_end_page_1idx, |page| {
                    page.min(visible_end_page_1idx)
                });
            self.stale_pages.extend(from.min(count)..count);
        } else {
            self.mark_pages_past_bound_stale(visible_end_page_1idx, downstream_stood);
        }
        self.mark_wrong_page_totals_stale();
        // The native host has consumed this pass directly into retained page
        // surfaces. Leaving the tracker full-dirty disables incremental drawing
        // on every subsequent keystroke (the browser consumes it in its payload).
        self.renderer.clear_dirty_regions();

        Ok(started.elapsed())
    }
}

/// Puts a colour into the `#RRGGBB` form the canvas understands.
///
/// Word writes `w:color w:val="C00000"` and CSS wants `#C00000`. A canvas given
/// the bare form keeps the colour it already had, which reads as "the command
/// did nothing" rather than as a rejected value. Anything already prefixed, or
/// not six hex digits — a named highlight like `yellow` — is left alone.
fn normalize_css_color(color: &str) -> String {
    let trimmed = color.trim();
    let is_hex = trimmed.len() == 6 && trimmed.bytes().all(|b| b.is_ascii_hexdigit());
    if is_hex {
        format!("#{trimmed}")
    } else {
        trimmed.to_string()
    }
}

// Original Windows Word PDF paper uses the same 600-dpi print grid as its
// vector font realization. Recover authored integer twips before rounding so
// CSS-point floating arithmetic cannot flip exact half-device-pixel cases.
fn windows_word_pdf_paper_size(logical: (f64, f64)) -> (f64, f64) {
    let extent = |points: f64| {
        let twips = (points * 20.0).round();
        let pixels = ((twips * 5.0 + 6.0) / 12.0).floor();
        if pixels.is_finite() && pixels >= 1.0 {
            pixels * 0.12
        } else {
            points
        }
    };
    (extent(logical.0), extent(logical.1))
}

#[cfg(test)]
mod windows_pdf_paper_tests {
    use super::windows_word_pdf_paper_size;

    #[test]
    fn actual_word_paper_bounds_keep_both_half_pixel_boundaries() {
        // Word 16.0.19127.20936 original PDF media boxes, including ties.
        for (logical, expected) in [
            ((595.1, 841.9), (595.08, 841.92)),
            ((595.3, 841.9), (595.32, 841.92)),
            ((595.5, 841.9), (595.56, 841.92)),
            ((595.3, (16842.0 / 15.0) * 0.75), (595.32, 842.16)),
            ((1584.0, 841.9), (1584.0, 841.92)),
        ] {
            let actual = windows_word_pdf_paper_size(logical);
            assert!((actual.0 - expected.0).abs() < 1e-9);
            assert!((actual.1 - expected.1).abs() < 1e-9);
        }
    }
}
