//! A DOCX open for editing and painting, with no browser.
//!
//! Wraps the three things that have to move together — the document model, the
//! renderer, and the page surfaces — and owns the repaint sequence that makes
//! an edit cheap. That sequence is not obvious and is easy to get wrong in ways
//! that still produce a picture, so it lives here rather than in each caller.

use crate::SkiaRenderHost;
use futures::executor::block_on;
pub use jade_docx_editor::canvas_renderer::CaretAffinity;
use jade_docx_editor::canvas_renderer::{CanvasRenderer, FullLayoutDebugSnapshot};
use jade_docx_editor::commands::transform_object::TransformObjectCommand;
use jade_docx_editor::commands::{
    Command, CommandContext, CommandExecutor, HistoryRenderFootprint,
};
use jade_docx_editor::field_datetime::FieldDateTime;
use jade_docx_editor::fields::{field_code_display, FieldCodeView};
use jade_docx_editor::headless_api::{
    apply_headless_commands_with_executor_to_document, execute_headless_queries_on_document,
};
use jade_docx_editor::word_document::{
    FloatingObjectKind, FloatingObjectTarget, FloatingObjectTransform, WordDocument,
};
use jade_docx_editor::Options;
use jade_docx_headless_api::{
    HeadlessBorderEdge, HeadlessCommand, HeadlessDeleteDirection, HeadlessEditRequest,
    HeadlessQuery, HeadlessQueryRequest, HeadlessQueryResult, HeadlessRunStyle,
};
use jade_selection::{SelectionRange, TextPosition};
use jade_skia_painter::SkiaPainter;
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
    /// A view's page box in place of the sections' (Web レイアウト, 閲覧
    /// モード); kept here so a renderer built again keeps it.
    view_geometry: Option<jade_docx_editor::canvas_renderer::ViewGeometry>,
    /// アウトライン's indent for each paragraph, while it shows.
    outline_indents: Option<Rc<std::collections::HashMap<usize, f64>>>,
    /// The math face the renderers bind (Cambria Math), when installed.
    math_face: Option<FontFace>,
}

/// Binds the math face among `faces` (Cambria Math, from Office's fonts) to
/// the renderer, so equations use its MATH table and outlines. Without it they
/// are laid out on Cambria Math's constants and drawn through the painter.
fn register_math_face(renderer: &mut CanvasRenderer<SkiaPainter>, faces: &[FontFace]) {
    let Some(face) = math_face_of(faces) else {
        return;
    };
    let Ok(bytes) = std::fs::read(&face.path) else {
        return;
    };
    let _ = renderer.register_math_face(
        crate::system_fonts::MATH_FAMILY.to_string(),
        bytes,
        face.index,
    );
}

fn math_face_of(faces: &[FontFace]) -> Option<FontFace> {
    faces
        .iter()
        .find(|face| {
            face.family
                .eq_ignore_ascii_case(crate::system_fonts::MATH_FAMILY)
        })
        .cloned()
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
        let mut options = Options::default();

        let word_document = block_on(WordDocument::load(docx_bytes, &options))
            .map_err(|err| format!("parse failed: {err}"))?;
        let Some(document) = word_document.document.as_ref() else {
            return Err("no main document part".to_string());
        };
        // Tracked changes as Word shows them: with their markup — insertions
        // coloured and underlined, deletions struck through — unless the
        // document was saved with a view that leaves them out.
        options.set_render_changes(document.display_content_revisions.unwrap_or(true));

        // Section properties decide the page box, and it is not necessarily the
        // one the caller asked for — this fixture is A4 while the shell asks
        // for Letter. A host sized to the wrong box draws each page short and
        // maps every page after the first to the wrong place, so find out
        // first and build to what the document wants.
        let (page_width, page_height) =
            Self::layout_page_size_of(&word_document, &options, faces, page_width, page_height)?;

        let mut host = SkiaRenderHost::new(page_width, page_height)
            .ok_or_else(|| "could not allocate the raster surface".to_string())?;
        register_faces(&mut host, faces)?;
        let fonts = host.font_environment();
        let pages = host.pages();
        let rendered_pages = host.rendered_pages();
        let pass_covers_whole_document = host.pass_covers_whole_document();
        let dark_paper = host.dark_paper();
        let mut renderer = CanvasRenderer::<SkiaPainter>::with_host(Box::new(host), false)
            .map_err(|_| "renderer setup failed".to_string())?;
        register_math_face(&mut renderer, faces);
        renderer.sync_word_document(&word_document);

        let document = word_document
            .document
            .as_ref()
            .expect("checked above")
            .clone();
        // The renderer defaults to letting a JS overlay draw the selection,
        // which leaves a native host with no highlight at all. There is no
        // overlay here; the renderer paints it.
        renderer.set_use_selection_overlay(false);
        // Declared up front: without it the first render on a fresh renderer
        // leaves the first paragraph of every page after the first unpainted —
        // a later full repaint draws it, so the content is laid out, just not
        // drawn. Costs nothing here because everything is dirty anyway.
        renderer.mark_full_repaint();
        renderer
            .render_with_numbering(
                &document,
                &options,
                word_document.numbering_part.as_ref(),
                word_document.header_lines.as_deref(),
                word_document.footer_lines.as_deref(),
                Some(&word_document.comments),
            )
            .map_err(|_| "render failed".to_string())?;
        if let Some(error) = renderer.take_render_error() {
            return Err(error);
        }
        drop(document);

        Ok(Self {
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
            math_face: math_face_of(faces),
            view_geometry: None,
            outline_indents: None,
        })
    }

    /// Serialises the document back to a .docx.
    ///
    /// Round-trips through jade's writer, so parts the edit never touched keep
    /// their original XML — tracked changes, comments and field codes survive.
    pub fn export_docx(&self) -> Result<Vec<u8>, String> {
        self.word_document
            .export_docx()
            .map_err(|err| format!("export failed: {err:?}"))
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
            register_math_face(&mut renderer, self.math_face.as_slice());
            renderer.sync_word_document(&self.word_document);
            renderer.set_spelling_export_mode(true);
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
        let document = self
            .word_document
            .document
            .as_ref()
            .ok_or_else(|| "document went missing".to_string())?
            .clone();
        let started = Instant::now();
        self.renderer.sync_word_document(&self.word_document);
        self.refresh_field_display();
        // History can restore different paragraph contents at an address
        // previously used by another snapshot. Recompute address-keyed hashes
        // before deciding whether old paragraph paint can be replayed.
        self.renderer.invalidate_paragraph_content_hash_cache();
        self.renderer.mark_full_repaint();
        self.renderer
            .render_with_numbering(
                &document,
                &self.options,
                self.word_document.numbering_part.as_ref(),
                self.word_document.header_lines.as_deref(),
                self.word_document.footer_lines.as_deref(),
                Some(&self.word_document.comments),
            )
            .map_err(|_| "repaint failed".to_string())?;
        if let Some(error) = self.renderer.take_render_error() {
            return Err(error);
        }
        self.follow_page_box()?;
        Ok(started.elapsed())
    }

    /// When an edit has changed the page box — the page turned, another
    /// paper — the host and the renderer are built again to it, as opening
    /// the document builds them. Built to the old box, the renderer went on
    /// wrapping lines to its width: turned to landscape, the text kept its
    /// portrait width, the pages their portrait pitch, and lines ran over
    /// each other.
    fn follow_page_box(&mut self) -> Result<(), String> {
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
        register_math_face(&mut renderer, self.math_face.as_slice());
        renderer.sync_word_document(&self.word_document);
        renderer.set_use_selection_overlay(false);
        renderer.set_view_geometry(self.view_geometry);
        renderer.set_outline_indents(self.outline_indents.clone());
        renderer.set_selection_range(self.selection);
        renderer.set_current_datetime(self.field_datetime.map(renderer_datetime));
        if self.field_codes != FieldCodeView::default() {
            renderer.set_field_code_display(Some(field_code_display(
                &self.word_document,
                &self.field_codes,
            )));
        }
        renderer.mark_full_repaint();
        let document = self
            .word_document
            .document
            .as_ref()
            .ok_or_else(|| "document went missing".to_string())?
            .clone();
        renderer
            .render_with_numbering(
                &document,
                &self.options,
                self.word_document.numbering_part.as_ref(),
                self.word_document.header_lines.as_deref(),
                self.word_document.footer_lines.as_deref(),
                Some(&self.word_document.comments),
            )
            .map_err(|_| "repaint failed".to_string())?;
        if let Some(error) = renderer.take_render_error() {
            return Err(error);
        }
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
        self.selection = Some(range);
        self.renderer.set_selection_range(Some(range));
        let (from, _) = range.normalized();
        self.repaint_bounded_to(from.paragraph, visible_end_page_1idx)
            .map(|_| ())
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
        self.selection = self
            .edit_in_place(|this| {
                apply_headless_commands_with_executor_to_document(
                    &mut this.word_document,
                    &mut this.executor,
                    this.selection.take(),
                    &HeadlessEditRequest {
                        commands: vec![command],
                    },
                )
            })
            .map_err(|error| format!("header/footer edit failed: {error:?}"))?;
        self.repaint_all()?;
        self.header_footer_undo.push(HeaderFooterEdit { previous });
        self.header_footer_redo.clear();
        self.redo_history.clear();
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

    /// Repaints from `paragraph` up to `visible_end_page_1idx`, drawing every
    /// page in that span.
    ///
    /// The same sequence a keystroke uses, minus the narrowing that only a
    /// change confined to one page can afford.
    fn repaint_bounded_to(
        &mut self,
        paragraph: usize,
        visible_end_page_1idx: usize,
    ) -> Result<Duration, String> {
        self.repaint_bounded_inner(paragraph, visible_end_page_1idx, false)
    }

    /// As above, drawing only `visible_end_page_1idx` itself.
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
            visible_end_page_1idx,
        );
        self.renderer.clear_visible_page_range();
        self.pass_covers_whole_document.set(true);
        result.map_err(|_| "repaint failed".to_string())?;
        if let Some(error) = self.renderer.take_render_error() {
            return Err(error);
        }
        Ok(started.elapsed())
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

    /// Which fields show their codes.
    pub fn field_code_view(&self) -> &FieldCodeView {
        &self.field_codes
    }

    /// Shows the codes of the fields `view` names (⌥F9 every field, ⇧F9 the
    /// selected ones) and repaints.
    pub fn set_field_code_view(&mut self, view: FieldCodeView) -> Result<(), String> {
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
        if query.is_empty() {
            return Vec::new();
        }
        let Ok(response) = execute_headless_queries_on_document(
            &self.word_document,
            &HeadlessQueryRequest {
                queries: vec![HeadlessQuery::SearchText {
                    query: query.to_string(),
                    start_paragraph: 0,
                    max_results: 500,
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
        let hits = self.find(query, false);
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
    /// Word's own vocabulary: `line` is in 240ths of a line under the `auto`
    /// rule, which is what the ribbon's 1.0 / 1.15 / 1.5 / 2.0 mean.
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
                line: Some(Some(multiple * 240.0)),
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
        let selection = self.selection;
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
        self.renderer.set_selection_range(selection);
        rect
    }

    /// The text position under a point, in the renderer's document coordinate
    /// space.
    ///
    /// Populated by rendering, so it reflects the last paint — which is what a
    /// click should hit.
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

    /// The document coordinate of a point on `page_index`.
    pub fn document_y(&self, page_index: usize, y_on_page: f64) -> f64 {
        y_on_page + page_index as f64 * self.page_pitch()
    }

    /// Which page a document coordinate falls on, and where on it.
    pub fn page_and_offset(&self, document_y: f64) -> (usize, f64) {
        let page = (document_y / self.page_pitch()).floor().max(0.0) as usize;
        (page, document_y - page as f64 * self.page_pitch())
    }

    /// The page box the document asks for, found by laying it out once.
    ///
    /// Costs a render, which is why it happens only at open. The page size is
    /// only known after layout has read the section properties, and everything
    /// that maps between screen and document depends on it.
    fn layout_page_size_of(
        word_document: &WordDocument,
        options: &Options,
        faces: &[FontFace],
        fallback_width: f64,
        fallback_height: f64,
    ) -> Result<(f64, f64), String> {
        let mut host = SkiaRenderHost::new(fallback_width, fallback_height)
            .ok_or_else(|| "could not allocate the probe surface".to_string())?;
        register_faces(&mut host, faces)?;
        let mut renderer = CanvasRenderer::<SkiaPainter>::with_host(Box::new(host), false)
            .map_err(|_| "renderer setup failed".to_string())?;
        register_math_face(&mut renderer, faces);
        renderer.sync_word_document(word_document);
        let document = word_document
            .document
            .as_ref()
            .ok_or_else(|| "no main document part".to_string())?
            .clone();
        renderer
            .render_with_numbering(
                &document,
                options,
                word_document.numbering_part.as_ref(),
                word_document.header_lines.as_deref(),
                word_document.footer_lines.as_deref(),
                Some(&word_document.comments),
            )
            .map_err(|_| "probe render failed".to_string())?;
        if let Some(error) = renderer.take_render_error() {
            return Err(error);
        }
        let (width, height) = renderer.layout_page_size();
        Ok(if width > 1.0 && height > 1.0 {
            (width, height)
        } else {
            (fallback_width, fallback_height)
        })
    }

    pub fn page_count(&self) -> usize {
        // The host's surfaces are a pool that a shrinking document leaves
        // entries in, so the count comes from what the last render produced.
        self.rendered_pages.get()
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
        let pages = self.page_count().max(1);
        self.apply_and_repaint(
            paragraph,
            HeadlessCommand::InsertParagraphBreak {
                paragraph,
                offset,
                count: 1,
            },
            pages,
        )
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

    // ---- for a shell's own commands ------------------------------------------

    /// The document model, for reading what a command needs to address —
    /// styles, numbering, table dimensions.
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
        let before = self.selection;
        self.redo_history.clear();
        self.history
            .push(HistoryStep::Command { executor_steps: 1 });
        let after = self.edit_in_place(|this| {
            apply_headless_commands_with_executor_to_document(
                &mut this.word_document,
                &mut this.executor,
                this.selection.take(),
                &HeadlessEditRequest {
                    commands: vec![command],
                },
            )
        });
        let after = match after {
            Ok(after) => after,
            Err(err) => {
                // Nothing was recorded, so neither is the step.
                self.history.pop();
                self.selection = before;
                return Err(format!("edit failed: {err:?}"));
            }
        };
        self.selection = if keep_selection {
            before
        } else {
            after.or(before)
        };
        self.renderer.set_selection_range(self.selection);
        self.repaint_all().map(|_| ())
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
        self.renderer.drop_document_cache();
        let result = edit(self);
        if let Some(document) = &self.word_document.document {
            self.renderer.restore_document_cache(Rc::clone(document));
        }
        result
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
        let before = self.selection;
        let recorded = self.executor.history_revision();
        self.executor.begin_group();
        let after = self.edit_in_place(|this| {
            let result = apply_headless_commands_with_executor_to_document(
                &mut this.word_document,
                &mut this.executor,
                this.selection.take(),
                &HeadlessEditRequest { commands },
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
        self.renderer.set_selection_range(self.selection);
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

    /// As [`Self::apply_and_repaint`], for a command that leaves more than one
    /// entry on the executor's stack.
    fn apply_and_repaint_grouped(
        &mut self,
        paragraph: usize,
        command: HeadlessCommand,
        visible_end_page_1idx: usize,
        executor_steps: usize,
    ) -> Result<Duration, String> {
        let old_height = matches!(&command, HeadlessCommand::InsertText { .. })
            .then(|| self.renderer.cached_paragraph_height(paragraph))
            .flatten();
        let text_edit_cache = matches!(&command, HeadlessCommand::InsertText { .. })
            .then(|| self.renderer.local_text_edit_cache());
        let recorded = self.executor.history_revision();
        // Through the executor so the edit lands on the undo stack. The plain
        // apply does the same edit without recording anything.
        // A fresh edit drops the redo branch, the same way the executor does.
        self.redo_history.clear();
        self.history.push(HistoryStep::Command { executor_steps });
        self.selection = self
            .edit_in_place(|this| {
                apply_headless_commands_with_executor_to_document(
                    &mut this.word_document,
                    &mut this.executor,
                    this.selection.take(),
                    &HeadlessEditRequest {
                        commands: vec![command],
                    },
                )
            })
            .map_err(|err| format!("edit failed: {err:?}"))?;

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
        if executor_steps == 1
            && self.executor.history_revision() != recorded
            && self.executor.next_undo_render_footprint()
                == Some(HistoryRenderFootprint::SingleParagraph { index: paragraph })
        {
            if let Some(cache) = text_edit_cache {
                self.renderer
                    .restore_local_text_edit_cache(cache, &self.word_document, paragraph);
            }
            if let Some(height) = old_height {
                self.renderer
                    .prepare_completed_pagination_edit(paragraph, height);
            }
        }
        // Bounded to what the caller can see, so the pages it stops at are not
        // the document's page count.
        self.pass_covers_whole_document
            .set(visible_end_page_1idx >= self.page_count());
        // The sequence lives on the renderer, so this shell and the browser one
        // cannot drift apart on it. Reproducing it here is what produced three
        // repaints that ran and drew nothing.
        self.renderer
            .prepare_incremental_repaint(paragraph, visible_end_page_1idx);
        // The other parts go with the body. Passing `None` for them renders a
        // document with no headers, no footers and no list numbering, which
        // looks like a layout difference rather than parts left behind.
        let result = self.renderer.render_bounded_to_page_with_parts(
            edited,
            &self.options,
            self.word_document.numbering_part.as_ref(),
            self.word_document.header_lines.as_deref(),
            self.word_document.footer_lines.as_deref(),
            visible_end_page_1idx,
        );
        // Whatever the pass was, the next one is a whole-document render until
        // something says otherwise.
        self.pass_covers_whole_document.set(true);
        result.map_err(|_| "repaint failed".to_string())?;
        if let Some(error) = self.renderer.take_render_error() {
            return Err(error);
        }

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
