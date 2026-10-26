use jade_canvas_painter::{Canvas2dPainter, PainterError};
use jade_painter::{Painter, TextMetrics};
use js_sys::{Object, Promise, Reflect};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    CanvasRenderingContext2d, HtmlCanvasElement, HtmlElement, ImageBitmap, ImageData,
    OffscreenCanvas, OffscreenCanvasRenderingContext2d,
};

// Layout caches store alphabetic ascent/descent. Canvas measureText otherwise
// returns baseline-relative values, including the "top" baseline used in cells.
fn measure_alphabetic_text_metrics<P: Painter>(
    painter: &P,
    text: &str,
) -> Result<TextMetrics, P::Error> {
    painter.save();
    painter.set_text_baseline("alphabetic");
    let result = painter.measure_text_metrics(text);
    painter.restore();
    result
}

/// Vertical gap between pages in the renderer's document coordinate space.
///
/// Public because anything mapping a point on a page back to a document
/// coordinate has to account for it: the pitch between pages is the page height
/// *plus* this, and assuming otherwise drifts by one gap per page.
pub const PAGE_GAP_PX: f64 = 16.0;

/// A boxed host, defaulting to the canvas backend.
///
/// Every use of `dyn CanvasRenderHost` goes through this alias. The default
/// parameter is what lets the browser build keep writing `BoxedRenderHost` and
/// `CanvasRenderer` with no type arguments, while a native build names its own
/// painter instead of editing either.
pub type BoxedRenderHost<P = Canvas2dPainter<CanvasRenderingContext2d>> =
    Box<dyn CanvasRenderHost<Painter = P>>;

/// Normalized source and destination rectangles for a cropped image.
///
/// Keeping both rectangles lets negative DrawingML `srcRect` edges reserve
/// transparent space in the destination instead of being clamped away.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageCropMapping {
    pub source_x: f64,
    pub source_y: f64,
    pub source_width: f64,
    pub source_height: f64,
    pub destination_x: f64,
    pub destination_y: f64,
    pub destination_width: f64,
    pub destination_height: f64,
}

/// Abstraction layer that hides DOM-specific canvas manipulation behind a
/// trait. This allows the renderer to run against different environments such
/// as regular DOM canvases, `OffscreenCanvas`, or headless test doubles.
pub trait CanvasRenderHost {
    /// The surface this host paints on.
    ///
    /// An associated type rather than a concrete one, so the backend is fixed
    /// at compile time with no dispatch: a native host names its own painter
    /// and the renderer monomorphises against it. It carries no lifetime, which
    /// is what keeps `CanvasRenderHost` usable as `dyn` — and is why the
    /// painter owns its handle to the surface rather than borrowing the host.
    /// Borrowing would also block `&mut self` while the painter is alive, which
    /// table rendering cannot accept.
    ///
    /// `PainterError` on the error type lets a fallible backend (canvas) and an
    /// infallible one (a rasteriser) both report through the renderer's own
    /// `JsValue`.
    type Painter: Painter<Error: PainterError>;

    /// Returns the current device pixel ratio used for canvas scaling.
    fn device_pixel_ratio(&self) -> f64;
    /// Refreshes the cached device pixel ratio and rebuilds the drawing
    /// context when the value changes. Returns `true` if the ratio changed.
    fn update_device_pixel_ratio(&mut self) -> Result<bool, JsValue>;
    /// Changes raster density while preserving logical page geometry.
    fn set_device_pixel_ratio(&mut self, _ratio: f64) -> Result<(), JsValue> {
        Err(JsValue::from_str(
            "Raster scale changes are unsupported by this host",
        ))
    }
    /// Prepares the host for a new render pass. When `reuse_pages` is true,
    /// implementations may keep existing page canvases to preserve pixels.
    fn begin_render(&mut self, _reuse_pages: bool) -> Result<(), JsValue> {
        self.reset_pages()
    }
    /// Removes any previously created page canvases and restores the base
    /// canvas to its initial state.
    fn reset_pages(&mut self) -> Result<(), JsValue>;
    /// Configures the base canvas element with the desired size and initial
    /// scaling. Pageless hosts may use a bounded raster with a growing logical height.
    fn configure_base_canvas(
        &mut self,
        width: f64,
        _height: f64,
        pageless: bool,
        margin_top: f64,
    ) -> Result<(), JsValue>;
    /// Clears the current canvas contents. In pageless mode only the visible
    /// portion is reset to avoid unnecessary growth.
    fn clear(&mut self, page_width: f64, pageless: bool, full_height: f64) -> Result<(), JsValue>;
    /// Extends the pageless document's logical height including the trailing margin.
    fn ensure_pageless_height(
        &mut self,
        required_height: f64,
        margin_bottom: f64,
    ) -> Result<(), JsValue>;
    fn pageless_viewport(&self) -> Option<(f64, f64)> {
        None
    }
    /// Moves the bounded pageless raster within the document's logical space.
    /// Hosts without viewport rendering keep their existing full-surface behavior.
    fn set_pageless_viewport(&mut self, _top: f64) -> Result<(), JsValue> {
        Ok(())
    }
    /// Allocates and prepares a new canvas that represents the next physical
    /// page.
    fn start_new_page(&mut self, width: f64, height: f64, margin_top: f64) -> Result<(), JsValue>;
    /// Advances the page cursor past a clean prefix page **without touching its
    /// canvas** (no activation, no resize, no context rebuild). This is the fast
    /// path for incremental edits deep in a document: pages entirely before the
    /// dirty window keep their pixels and need no per-page canvas work, so the
    /// renderer only has to keep the page bookkeeping (cursor, logical-size
    /// vector) consistent.
    ///
    /// Returns `true` when the page was advanced without touching. Returns
    /// `false` when the host cannot skip this page — the slot does not exist yet
    /// (page must be allocated) or the page's logical size differs from the
    /// cached one (backing must be resized). In that case the caller must fall
    /// back to [`Self::start_new_page`].
    ///
    /// The active drawing context is left unchanged, so the caller must ensure
    /// the *last* page it lands on (the seek target / a drawn page) is activated
    /// through [`Self::start_new_page`], not this method.
    ///
    /// Default: hosts that always keep every page materialized (DOM / test
    /// doubles) return `false` so the caller always uses `start_new_page`.
    fn advance_page_without_touch(&mut self, _width: f64, _height: f64) -> bool {
        false
    }
    /// Moves the host cursor directly to an already allocated clean page.
    /// `page_number` is 1-based and includes the base page. The active drawing
    /// context is intentionally left unchanged; callers must activate a page
    /// before drawing it.
    ///
    /// This is the batched counterpart of [`Self::advance_page_without_touch`]
    /// for seek-snapshot restores. Implementations return `false` when they
    /// cannot prove that every skipped page slot already exists.
    fn jump_to_page_without_touch(&mut self, _page_number: usize) -> bool {
        false
    }
    /// Allocates the next out-of-window page as a lightweight placeholder and
    /// advances the page cursor without creating or activating a 2d context.
    /// This is the first-render counterpart of
    /// [`Self::advance_page_without_touch`]: lazy layout still needs a stable
    /// page slot and logical dimensions, but an offscreen page that will not be
    /// painted yet does not need a drawing context.
    ///
    /// Returns `true` when the placeholder was allocated. Returns `false` when
    /// the host does not support deferred contexts or the page must be kept
    /// materialized, in which case the caller falls back to
    /// [`Self::start_new_page`].
    fn advance_page_with_placeholder(
        &mut self,
        _width: f64,
        _height: f64,
    ) -> Result<bool, JsValue> {
        Ok(false)
    }
    /// Activates the canvas of the page the cursor currently sits on, without
    /// advancing the cursor.
    ///
    /// The render walk reaches a page it will not draw through
    /// [`Self::advance_page_without_touch`] /
    /// [`Self::advance_page_with_placeholder`], which leave the drawing context
    /// on the previously drawn page. When a cache miss later promotes that page
    /// to dirty mid-walk, the renderer calls this so its draws land on the
    /// promoted page instead of overpainting the page it left. A shrunk or
    /// placeholder backing is inflated to the page's logical size first.
    ///
    /// Default no-op: hosts that activate every page through
    /// [`Self::start_new_page`] never leave the context on a stale page.
    fn activate_current_page(&mut self) -> Result<(), JsValue> {
        Ok(())
    }
    /// Returns an immutable reference to the active 2d drawing context.
    ///
    /// Prefer [`Self::painter`] for drawing. This stays for the parts of the
    /// renderer that still need canvas-specific types (image compositing,
    /// snapshot transfer).
    fn context(&self) -> &CanvasRenderingContext2d;
    /// Returns the surface the renderer paints on.
    ///
    /// Every painting call in the renderer now reaches its surface through
    /// here, rather than each site wrapping [`Self::context`] itself. The
    /// return type is still concrete, so this does not yet *choose* a backend —
    /// but it collapses the choice from dozens of construction sites down to
    /// one signature, which is the part that has to be true before a
    /// non-canvas backend is possible.
    ///
    /// Prefer to bind the result once per loop rather than per iteration: this
    /// is a trait method on a `dyn` host, so calling it per glyph pays virtual
    /// dispatch per glyph.
    ///
    /// The returned painter borrows the host, so a caller that needs it to
    /// outlive a `&mut self` call clones the context and builds one from that;
    /// [`Self::context`] is still there for those.
    fn painter(&self) -> Self::Painter;

    /// Backend failures that cannot be represented as a native `JsValue`.
    /// Native callers check this after a render before returning its output.
    fn take_render_error(&self) -> Option<String> {
        None
    }

    // ---- measurement -------------------------------------------------------
    //
    // Measuring is the one surface capability the renderer needs *before* it
    // paints anything: wrapping, line heights and pagination are all decided
    // from advance widths. A host that cannot produce a canvas — the native
    // shaping backend, or a test double — must still be able to answer these,
    // so they are host methods with canvas defaults rather than calls the
    // renderer makes on a painter it assumes exists.
    //
    // Font application is deliberately separate from measuring. Measurement is
    // by far the most frequent call during wrapping and the renderer tracks
    // whether the descriptor actually changed; folding the two together would
    // re-apply the font on every measurement, which `measure_text_cached`
    // documents as measurably worse.

    /// Applies `font_descriptor` to the surface subsequent measurement will use.
    ///
    /// Called only when the renderer has determined the descriptor changed, and
    /// only from the measurement path — painting applies the font to its own
    /// [`Painter`] instead. For the canvas host those are the same surface, so
    /// one application serves both. A host where they differ must make this
    /// reach both, since the renderer tracks only *that* a descriptor was
    /// applied, not where.
    fn set_font(&self, font_descriptor: &str) {
        self.painter().set_font(font_descriptor);
    }

    /// Advance width of `text` under the font last given to [`Self::set_font`].
    fn measure_text_width(&self, text: &str) -> Result<f64, JsValue> {
        self.painter()
            .measure_text_width(text)
            .map_err(PainterError::into_js_value)
    }

    /// Draws an image from a data URI, if the host can.
    ///
    /// Returns `false` when it cannot, and the renderer falls back to the
    /// browser path — `createImageBitmap`, a promise, and a redraw when it
    /// resolves. That path needs a canvas, so a host without one must handle
    /// this or the renderer reaches for a context it does not have.
    ///
    /// Rotation and flips are the caller's to apply around this: a host that
    /// only blits still gets an image on the page, which is the difference
    /// between a document that renders and one that panics.
    fn draw_image_data_uri(
        &self,
        _data_uri: &str,
        _x: f64,
        _y: f64,
        _width: f64,
        _height: f64,
        _crop: Option<ImageCropMapping>,
    ) -> bool {
        false
    }

    /// Whether drawing may be held back and painted later in the page.
    ///
    /// The renderer paints a page's floating objects in Word's layer order —
    /// objects behind text, the text, then objects in front of text — by
    /// holding them until the page is finished. That needs every draw to go
    /// through [`Self::painter`] or [`Self::draw_image_data_uri`]; a host that
    /// falls back to its canvas context for images cannot hold them, and keeps
    /// the anchor-order painting.
    fn supports_deferred_painting(&self) -> bool {
        false
    }

    /// Alphabetic-baseline metrics under the font last given to [`Self::set_font`].
    ///
    /// Line box height and trailing overhang need the bounding boxes, not just
    /// the advance, so a host overriding [`Self::measure_text_width`] has to
    /// override this too or pagination will disagree with wrapping.
    fn measure_text_metrics(&self, text: &str) -> Result<TextMetrics, JsValue> {
        measure_alphabetic_text_metrics(&self.painter(), text).map_err(PainterError::into_js_value)
    }
    /// Returns a mutable reference to the active 2d drawing context.
    fn context_mut(&mut self) -> &mut CanvasRenderingContext2d;
    /// Converts a device coordinate into the CSS space used by layout logic.
    fn device_to_css(&self, value: f64) -> f64;
    /// Returns the current canvas size in CSS pixels.
    fn canvas_css_size(&self) -> (f64, f64);
    /// Captures paged rendering results as transferable snapshots when available.
    fn take_paged_snapshots(&mut self) -> Result<Option<Vec<JsValue>>, JsValue>;
    /// Captures paged rendering results for a subset of pages. When `pages` is `None`,
    /// captures all pages.
    ///
    /// Page indices are 0-indexed.
    fn take_paged_snapshots_for_pages(
        &mut self,
        pages: Option<&[usize]>,
    ) -> Result<Option<Vec<JsValue>>, JsValue>;
    /// Returns the total number of pages (1 for base canvas + extra canvases).
    fn page_count(&self) -> usize;
    /// Returns the dimensions (width, height) for a specific page index.
    fn page_dimensions(&self, page_index: usize) -> Option<(f64, f64)>;

    fn take_page_text_capture(&self, _page_index: usize) -> JsValue {
        JsValue::NULL
    }
    /// Truncates extra page canvases so the total page count matches `total_pages`.
    fn truncate_pages(&mut self, _total_pages: usize) -> Result<(), JsValue> {
        Ok(())
    }
    /// Creates an ImageBitmap for a specific page without destroying the canvas.
    /// This is non-destructive unlike `take_paged_snapshots`.
    fn create_page_bitmap(&self, page_index: usize) -> Result<Promise, JsValue>;
    /// Transfers an ImageBitmap for a specific page using `transferToImageBitmap()`.
    /// This is synchronous and much faster than `create_page_bitmap`, but
    /// **clears the canvas contents** — callers must re-render the page afterwards
    /// if the pixels are still needed.
    fn transfer_page_bitmap(&mut self, page_index: usize) -> Result<ImageBitmap, JsValue> {
        let _ = page_index;
        Err(JsValue::from_str("transfer_page_bitmap not supported"))
    }

    /// Declares the inclusive 1-based page-number window whose canvas backings
    /// must stay materialized at full resolution. Pages outside the window may
    /// have their backing stores shrunk to free memory (lazy/windowed mode).
    /// `None` keeps every page materialized (non-lazy / full-document render).
    ///
    /// Default no-op: hosts that always keep every page (DOM / test doubles)
    /// ignore this.
    fn set_page_keep_window(&mut self, _window: Option<std::ops::RangeInclusive<usize>>) {}

    /// Enables a small bounded pool of recycled page slots that retain their
    /// full-resolution backing store. Implementations that do not recycle
    /// offscreen page canvases may ignore this setting.
    fn set_hot_page_backing_pool_enabled(&mut self, _enabled: bool) {}

    /// Reports whether the given 0-based page is inside the current keep window
    /// (i.e. its backing is guaranteed to be kept at full resolution). Pages in
    /// the window can be snapshotted non-destructively (`create_page_bitmap`)
    /// with no extra peak memory, because their full-resolution backing already
    /// exists — this lets the bitmap pipeline skip the shrink+re-inflate+re-render
    /// churn on repeated fetches of the caret/visible pages.
    ///
    /// Default `true`: hosts that never shrink page backings keep every page.
    fn page_in_keep_window(&self, _page_index: usize) -> bool {
        true
    }

    /// Reports whether the given 0-based page currently holds a full-resolution
    /// backing store with valid rendered pixels. Pages whose backing was shrunk
    /// to save memory return `false` so the bitmap pipeline re-renders them
    /// before snapshotting (otherwise a snapshot would be blank). Default `true`
    /// for hosts that never shrink page backings.
    fn page_has_rendered_pixels(&self, _page_index: usize) -> bool {
        true
    }

    /// Records that the given 0-based page was (re)painted in the current render
    /// pass, so its backing now holds valid pixels. Paired with the backing being
    /// invalidated on shrink/inflate. Default no-op.
    fn mark_page_rendered(&mut self, _page_index: usize) {}

    /// Records that the given 0-based page's backing no longer holds pixels
    /// for the current layout, so `page_has_rendered_pixels` reports `false`
    /// until a render repaints it. Default no-op.
    fn forget_page_pixels(&mut self, _page_index: usize) {}

    /// Diagnostic counters exposed through `debugRendererStats`:
    /// `(context_rebuilds, context_rebuild_skips, canvas_inflates,
    /// pages_seek_skipped, context_creations, context_creation_deferrals)`.
    ///
    /// * `context_rebuilds`: how many times a page activation actually re-applied
    ///   the identity transform + dpr scale + default font/fill (a WASM→JS cost).
    /// * `context_rebuild_skips`: how many activations skipped the rebuild because
    ///   the backing store was unchanged and the context already held the correct
    ///   transform.
    /// * `canvas_inflates`: how many times a shrunk (1x1) page backing was
    ///   re-inflated to full resolution.
    /// * `pages_seek_skipped`: how many no-touch cursor operations skipped a
    ///   clean prefix (single-page advances or batched seek jumps).
    /// * `context_creations`: how many 2d contexts have actually been created.
    /// * `context_creation_deferrals`: how many first-render page placeholders
    ///   were allocated without creating a 2d context.
    ///
    /// Default zeros for hosts that do not track this.
    fn context_rebuild_stats(&self) -> (u64, u64, u64, u64, u64, u64) {
        (0, 0, 0, 0, 0, 0)
    }

    /// Diagnostic counters for bounded OffscreenCanvas page-slot recycling:
    /// `(context_creations, page_slot_recycles, page_slot_reuses, pool_len,
    /// hot_backing_recycles, hot_backing_reuses, hot_backing_pool_len,
    /// hot_backing_pool_bytes)`.
    fn page_slot_recycle_stats(&self) -> (u64, u64, u64, usize, u64, u64, usize, u64) {
        (0, 0, 0, 0, 0, 0, 0, 0)
    }
}

/// DOM-backed implementation of [`CanvasRenderHost`]. All interactions with
/// `web_sys` elements are encapsulated here so that the renderer can remain
/// agnostic of the runtime environment.
pub struct DomRenderHost {
    base_canvas: HtmlCanvasElement,
    current_canvas: HtmlCanvasElement,
    context: CanvasRenderingContext2d,
    document: web_sys::Document,
    parent: Option<web_sys::Element>,
    page_style: Option<String>,
    extra_canvases: Vec<HtmlCanvasElement>,
    page_cursor: usize,
    device_pixel_ratio: f64,
}

fn snap_css_length_to_device_pixels(value: f64, device_pixel_ratio: f64) -> f64 {
    let dpr = if device_pixel_ratio.is_finite() && device_pixel_ratio > 0.0 {
        device_pixel_ratio
    } else {
        1.0
    };
    let safe_value = if value.is_finite() && value > 0.0 {
        value
    } else {
        1.0
    };
    ((safe_value * dpr).round().max(1.0)) / dpr
}

impl DomRenderHost {
    /// Creates a new host that controls the provided canvas element. The
    /// device pixel ratio is captured at construction time and used to scale
    /// the drawing context accordingly.
    pub fn new(canvas: HtmlCanvasElement) -> Result<Self, JsValue> {
        let document = canvas
            .owner_document()
            .ok_or_else(|| JsValue::from_str("Canvas has no owner document"))?;
        let mut device_pixel_ratio = web_sys::window()
            .map(|window| window.device_pixel_ratio())
            .unwrap_or(1.0);
        if !device_pixel_ratio.is_finite() || device_pixel_ratio <= 0.0 {
            device_pixel_ratio = 1.0;
        }

        let context = Self::create_scaled_context(&canvas, device_pixel_ratio)?;
        let parent = canvas.parent_element();
        let page_style = canvas.get_attribute("style");

        Ok(Self {
            base_canvas: canvas.clone(),
            current_canvas: canvas,
            context,
            document,
            parent,
            page_style,
            extra_canvases: Vec::new(),
            page_cursor: 0,
            device_pixel_ratio,
        })
    }

    fn create_scaled_context(
        canvas: &HtmlCanvasElement,
        device_pixel_ratio: f64,
    ) -> Result<CanvasRenderingContext2d, JsValue> {
        let raw_context = canvas
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("Failed to obtain 2d context"))?;

        let context = match raw_context.dyn_into::<CanvasRenderingContext2d>() {
            Ok(ctx) => ctx,
            Err(raw) => {
                let offscreen_ctx: OffscreenCanvasRenderingContext2d =
                    raw.dyn_into().map_err(|_| {
                        JsValue::from_str("Failed to obtain OffscreenCanvasRenderingContext2D")
                    })?;
                offscreen_ctx.unchecked_into()
            }
        };

        let _ = context.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
        if device_pixel_ratio.is_finite() && device_pixel_ratio > 0.0 {
            context.scale(device_pixel_ratio, device_pixel_ratio)?;
        }

        context.set_font("12pt 'Times New Roman', serif");
        context.set_fill_style_str("#000000");

        Ok(context)
    }

    fn set_canvas_css_size(
        &self,
        canvas: &HtmlCanvasElement,
        width: f64,
        height: f64,
    ) -> Result<(), JsValue> {
        let snapped_width = snap_css_length_to_device_pixels(width, self.device_pixel_ratio);
        let snapped_height = snap_css_length_to_device_pixels(height, self.device_pixel_ratio);
        let device_width = (snapped_width * self.device_pixel_ratio).round().max(1.0) as u32;
        let device_height = (snapped_height * self.device_pixel_ratio).round().max(1.0) as u32;
        // Skip set_width/set_height when the canvas already has the desired size.
        // Setting canvas dimensions clears the content and resets the context,
        // which is expensive (~160ms in profiling).
        if canvas.width() != device_width || canvas.height() != device_height {
            canvas.set_width(device_width);
            canvas.set_height(device_height);
        }

        let html_element: HtmlElement = canvas.clone().dyn_into()?;
        let style = html_element.style();
        style.set_property("width", &format!("{snapped_width:.2}px"))?;
        style.set_property("height", &format!("{snapped_height:.2}px"))?;
        Ok(())
    }

    fn set_canvas_css_height(
        &self,
        canvas: &HtmlCanvasElement,
        height: f64,
    ) -> Result<(), JsValue> {
        let snapped_height = snap_css_length_to_device_pixels(height, self.device_pixel_ratio);
        let device_height = (snapped_height * self.device_pixel_ratio).round().max(1.0) as u32;
        // Only resize when the height actually changed to avoid clearing canvas.
        if canvas.height() != device_height {
            canvas.set_height(device_height);
        }

        let html_element: HtmlElement = canvas.clone().dyn_into()?;
        let style = html_element.style();
        style.set_property("height", &format!("{snapped_height:.2}px"))?;
        Ok(())
    }

    fn rebuild_context(&mut self) -> Result<(), JsValue> {
        self.context = Self::create_scaled_context(&self.current_canvas, self.device_pixel_ratio)?;
        Ok(())
    }
}

impl CanvasRenderHost for DomRenderHost {
    type Painter = Canvas2dPainter<CanvasRenderingContext2d>;

    fn painter(&self) -> Self::Painter {
        Canvas2dPainter::new(self.context().clone())
    }

    fn device_pixel_ratio(&self) -> f64 {
        self.device_pixel_ratio
    }

    fn update_device_pixel_ratio(&mut self) -> Result<bool, JsValue> {
        let mut ratio = web_sys::window()
            .map(|window| window.device_pixel_ratio())
            .unwrap_or(1.0);
        if !ratio.is_finite() || ratio <= 0.0 {
            ratio = 1.0;
        }

        if (ratio - self.device_pixel_ratio).abs() > 1e-6 {
            self.device_pixel_ratio = ratio;
            self.rebuild_context()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn begin_render(&mut self, reuse_pages: bool) -> Result<(), JsValue> {
        if reuse_pages {
            self.current_canvas = self.base_canvas.clone();
            self.page_cursor = 0;
            self.rebuild_context()?;
            Ok(())
        } else {
            self.reset_pages()
        }
    }

    fn reset_pages(&mut self) -> Result<(), JsValue> {
        if let Some(parent) = &self.parent {
            for canvas in self.extra_canvases.drain(..) {
                let _ = parent.remove_child(&canvas);
            }
        }
        self.current_canvas = self.base_canvas.clone();
        self.page_cursor = 0;
        self.rebuild_context()?;
        Ok(())
    }

    fn configure_base_canvas(
        &mut self,
        width: f64,
        height: f64,
        pageless: bool,
        margin_top: f64,
    ) -> Result<(), JsValue> {
        let initial_height = if pageless {
            (margin_top + 1.0).ceil().max(1.0)
        } else {
            height.max(1.0)
        };

        self.set_canvas_css_size(&self.base_canvas, width, initial_height)?;
        self.current_canvas = self.base_canvas.clone();
        self.rebuild_context()?;
        Ok(())
    }

    fn clear(&mut self, width: f64, pageless: bool, full_height: f64) -> Result<(), JsValue> {
        let height = if pageless {
            self.device_to_css(self.current_canvas.height() as f64)
        } else {
            full_height
        };
        self.context.clear_rect(0.0, 0.0, width, height);
        Ok(())
    }

    fn ensure_pageless_height(
        &mut self,
        required_height: f64,
        margin_bottom: f64,
    ) -> Result<(), JsValue> {
        let current_height_device = self.current_canvas.height() as f64;
        let current_height_css = self.device_to_css(current_height_device);
        if required_height > current_height_css {
            let mut new_height_css = (required_height + margin_bottom).ceil();
            if new_height_css <= current_height_css {
                new_height_css = current_height_css;
            }

            if new_height_css > current_height_css {
                let width = self.current_canvas.width();
                let prev_height = self.current_canvas.height();
                let image_data: Option<ImageData> = if width > 0 && prev_height > 0 {
                    self.context
                        .get_image_data(0.0, 0.0, width as f64, prev_height as f64)
                        .ok()
                } else {
                    None
                };

                self.set_canvas_css_height(&self.current_canvas, new_height_css)?;
                self.rebuild_context()?;
                if let Some(data) = image_data {
                    let _ = self.context.put_image_data(&data, 0.0, 0.0);
                }
            }
        }
        Ok(())
    }

    fn start_new_page(&mut self, width: f64, height: f64, margin_top: f64) -> Result<(), JsValue> {
        self.page_cursor = self.page_cursor.saturating_add(1);
        if self.page_cursor <= self.extra_canvases.len() {
            // Reuse existing canvas to preserve pixels on clean pages.
            let canvas = self.extra_canvases[self.page_cursor - 1].clone();
            self.set_canvas_css_size(&canvas, width, height.max(1.0))?;
            self.current_canvas = canvas;
            self.rebuild_context()?;
            if margin_top > 0.0 {
                self.context.translate(0.0, 0.0)?;
            }
            return Ok(());
        }

        let parent = self
            .parent
            .as_ref()
            .ok_or_else(|| JsValue::from_str("Canvas has no parent element"))?
            .clone();

        let new_canvas = self
            .document
            .create_element("canvas")?
            .dyn_into::<HtmlCanvasElement>()?;
        if let Some(style) = &self.page_style {
            let mut style_value = style.clone();
            if !style_value.trim().is_empty() && !style_value.trim_end().ends_with(';') {
                style_value.push(';');
            }
            style_value.push_str(&format!("margin-top: {}px;", PAGE_GAP_PX));
            new_canvas.set_attribute("style", &style_value)?;
        } else {
            new_canvas.set_attribute("style", &format!("margin-top: {}px;", PAGE_GAP_PX))?;
        }

        if let Some(index) = self.base_canvas.get_attribute("data-view-index") {
            new_canvas.set_attribute("data-view-index", &index)?;
        }

        let key = JsValue::from_str("__jade_view");
        if let Ok(view_value) = Reflect::get(self.base_canvas.as_ref(), &key) {
            let _ = Reflect::set(new_canvas.as_ref(), &key, &view_value);
        }

        self.set_canvas_css_size(&new_canvas, width, height.max(1.0))?;

        parent.append_child(&new_canvas)?;

        self.current_canvas = new_canvas.clone();
        self.context = Self::create_scaled_context(&new_canvas, self.device_pixel_ratio)?;
        self.extra_canvases.push(new_canvas);

        // Ensure the first drawing operation will start after the top margin
        if margin_top > 0.0 {
            self.context.translate(0.0, 0.0)?; // no-op but keeps API consistent
        }

        Ok(())
    }

    fn context(&self) -> &CanvasRenderingContext2d {
        &self.context
    }

    fn context_mut(&mut self) -> &mut CanvasRenderingContext2d {
        &mut self.context
    }

    fn device_to_css(&self, value: f64) -> f64 {
        if self.device_pixel_ratio > 0.0 {
            value / self.device_pixel_ratio
        } else {
            value
        }
    }

    fn canvas_css_size(&self) -> (f64, f64) {
        let width = self.device_to_css(self.current_canvas.width() as f64);
        let height = self.device_to_css(self.current_canvas.height() as f64);
        (width, height)
    }

    fn take_paged_snapshots(&mut self) -> Result<Option<Vec<JsValue>>, JsValue> {
        Ok(None)
    }

    fn take_paged_snapshots_for_pages(
        &mut self,
        _pages: Option<&[usize]>,
    ) -> Result<Option<Vec<JsValue>>, JsValue> {
        Ok(None)
    }

    fn page_count(&self) -> usize {
        1 + self.extra_canvases.len()
    }

    fn take_page_text_capture(&self, page_index: usize) -> JsValue {
        let canvas = if page_index == 0 {
            Some(&self.base_canvas)
        } else {
            self.extra_canvases.get(page_index - 1)
        };
        let context = canvas
            .and_then(|canvas| canvas.get_context("2d").ok().flatten())
            .and_then(|context| context.dyn_into::<CanvasRenderingContext2d>().ok());
        context.map_or(JsValue::NULL, |context| {
            jade_canvas_painter::text_batch::take_text_capture(&context, self.device_pixel_ratio)
        })
    }

    fn page_dimensions(&self, page_index: usize) -> Option<(f64, f64)> {
        if page_index == 0 {
            Some((
                self.device_to_css(self.base_canvas.width() as f64),
                self.device_to_css(self.base_canvas.height() as f64),
            ))
        } else {
            self.extra_canvases.get(page_index - 1).map(|canvas| {
                (
                    self.device_to_css(canvas.width() as f64),
                    self.device_to_css(canvas.height() as f64),
                )
            })
        }
    }

    fn truncate_pages(&mut self, total_pages: usize) -> Result<(), JsValue> {
        let desired = total_pages.saturating_sub(1);
        if desired >= self.extra_canvases.len() {
            return Ok(());
        }
        if let Some(parent) = &self.parent {
            for canvas in self.extra_canvases.drain(desired..) {
                let _ = parent.remove_child(&canvas);
            }
        } else {
            self.extra_canvases.truncate(desired);
        }
        Ok(())
    }

    fn create_page_bitmap(&self, _page_index: usize) -> Result<Promise, JsValue> {
        // DomRenderHost doesn't support creating ImageBitmaps
        Err(JsValue::from_str(
            "DomRenderHost does not support create_page_bitmap",
        ))
    }
}

impl Default for DomRenderHost {
    fn default() -> Self {
        panic!("DomRenderHost::default should not be used directly");
    }
}

/// OffscreenCanvas 実装のレンダーホスト。Pageless / Paged の両方をサポートし、
/// Paged の場合はページ単位の `ImageBitmap` を生成してメインスレッドに返す。
const MAX_RECYCLED_PAGE_SLOTS: usize = 32;
const MAX_HOT_PAGE_BACKING_SLOTS: usize = 4;
const MAX_HOT_PAGE_BACKING_BYTES: u64 = 32 * 1024 * 1024;

struct RecycledOffscreenPageSlot {
    canvas: OffscreenCanvas,
    context: CanvasRenderingContext2d,
    retains_full_backing: bool,
}

pub struct OffscreenRenderHost {
    base_canvas: OffscreenCanvas,
    base_logical_size: (f64, f64),
    current_canvas: OffscreenCanvas,
    base_context: CanvasRenderingContext2d,
    context: CanvasRenderingContext2d,
    extra_canvases: Vec<OffscreenCanvas>,
    /// Lazily-created contexts parallel to `extra_canvases`. Out-of-window
    /// first-render pages keep `None` until they are actually painted.
    extra_contexts: Vec<Option<CanvasRenderingContext2d>>,
    /// Logical (CSS) size per extra page, parallel to `extra_canvases`. Kept
    /// separate from the canvas backing dimensions so `page_dimensions` reports
    /// the true page size even when a backing has been shrunk to free memory.
    page_logical_sizes: Vec<(f64, f64)>,
    /// Whether each extra page currently holds a full-resolution backing with
    /// rendered pixels (vs. a shrunk 1x1 placeholder). Parallel to
    /// `extra_canvases`.
    page_materialized: Vec<bool>,
    /// Whether the base canvas (page 0 / page number 1) currently holds the
    /// fully-painted page content. The base canvas is never shrunk, but a
    /// destructive `transfer_to_image_bitmap()` (snapshot / bitmap fetch) leaves
    /// it BLANK until the next render repaints it. `page_has_rendered_pixels(0)`
    /// must reflect this so band redraw (which only clears+repaints the edited
    /// band and preserves the rest of the canvas) is not allowed to run on a
    /// blanked base canvas — that would leave every non-edited paragraph white.
    /// This is exactly the IME compositionEnd blanking: the composition-preview
    /// render transfers page 1's pixels away, then the commit's band redraw
    /// clears only the band on the now-blank base canvas. Set `false` on every
    /// base transfer, `true` on `mark_page_rendered(0)`.
    base_has_pixels: bool,
    /// Inclusive 1-based page-number window to keep materialized; `None` keeps
    /// every page at full resolution.
    keep_window: Option<(usize, usize)>,
    page_cursor: usize,
    pageless_mode: bool,
    pageless_top: f64,
    pageless_height: f64,
    device_pixel_ratio: f64,
    /// Whether the base canvas context needs its transform/scale re-applied on
    /// the next activation. `false` means the context already holds the correct
    /// dpr transform (created fresh or rebuilt with no intervening backing
    /// change), so `rebuild_context` would be pure waste. Set true whenever the
    /// base backing store is resized/cleared (which implicitly resets the
    /// context transform) or a bitmap transfer clears it.
    base_context_needs_rebuild: bool,
    /// Per-extra-page rebuild flags, parallel to `extra_canvases` /
    /// `extra_contexts`. Same semantics as `base_context_needs_rebuild`.
    extra_context_needs_rebuild: Vec<bool>,
    /// Recently truncated slots retained for immediate lazy-layout
    /// re-extension. Pooled slots are excluded from the logical page vectors.
    recycled_page_slots: Vec<RecycledOffscreenPageSlot>,
    /// When enabled, up to `MAX_HOT_PAGE_BACKING_SLOTS` recycled slots retain
    /// their full-resolution backing. The rest still collapse to 1x1, keeping
    /// total memory bounded while avoiding hot-path backing reallocation.
    hot_page_backing_pool_enabled: bool,
    hot_page_backing_pool_len: usize,
    hot_page_backing_pool_bytes: u64,
    /// Diagnostic counters (see `context_rebuild_stats`).
    context_rebuilds: u64,
    context_rebuild_skips: u64,
    canvas_inflates: u64,
    /// How many clean prefix pages were advanced via
    /// `advance_page_without_touch` (no canvas activation) this session.
    pages_seek_skipped: u64,
    /// Total 2d contexts created, including the base canvas context.
    context_creations: u64,
    /// First-render page slots allocated without creating a 2d context.
    context_creation_deferrals: u64,
    /// Truncated page slots accepted into the bounded recycle pool.
    page_slot_recycles: u64,
    /// Recycled slots consumed by a later render pass.
    page_slot_reuses: u64,
    /// Full-resolution slots accepted into / reused from the hot tier.
    hot_page_backing_recycles: u64,
    hot_page_backing_reuses: u64,
}

/// Bound both the raster height and its pixel area independently
/// of document length. Logical geometry continues to cover the whole document.
fn pageless_raster_height(width: f64, ratio: f64) -> f64 {
    let ratio = if ratio.is_finite() && ratio > 0.0 {
        ratio
    } else {
        1.0
    };
    let width_px = (width.max(1.0) * ratio).ceil();
    let height_px = (16_777_216.0 / width_px).floor().clamp(1.0, 8192.0);
    (4096.0 * ratio).min(height_px) / ratio
}

impl OffscreenRenderHost {
    pub fn new(canvas: OffscreenCanvas, device_pixel_ratio: f64) -> Result<Self, JsValue> {
        let ratio = if device_pixel_ratio.is_finite() && device_pixel_ratio > 0.0 {
            device_pixel_ratio
        } else {
            1.0
        };

        let context = Self::create_scaled_context(&canvas, ratio)?;

        Ok(Self {
            base_logical_size: (
                canvas.width() as f64 / ratio,
                canvas.height() as f64 / ratio,
            ),
            base_canvas: canvas.clone(),
            current_canvas: canvas,
            base_context: context.clone(),
            context,
            extra_canvases: Vec::new(),
            extra_contexts: Vec::new(),
            page_logical_sizes: Vec::new(),
            page_materialized: Vec::new(),
            // The base canvas starts blank (no page painted yet); the first
            // render marks it via `mark_page_rendered(0)`.
            base_has_pixels: false,
            keep_window: None,
            page_cursor: 0,
            pageless_mode: true,
            pageless_top: 0.0,
            pageless_height: 1.0,
            device_pixel_ratio: ratio,
            // `create_scaled_context` above already applied the identity
            // transform + dpr scale + default font/fill, so the base context
            // starts in the correct state.
            base_context_needs_rebuild: false,
            extra_context_needs_rebuild: Vec::new(),
            recycled_page_slots: Vec::new(),
            hot_page_backing_pool_enabled: false,
            hot_page_backing_pool_len: 0,
            hot_page_backing_pool_bytes: 0,
            context_rebuilds: 0,
            context_rebuild_skips: 0,
            canvas_inflates: 0,
            pages_seek_skipped: 0,
            context_creations: 1,
            context_creation_deferrals: 0,
            page_slot_recycles: 0,
            page_slot_reuses: 0,
            hot_page_backing_recycles: 0,
            hot_page_backing_reuses: 0,
        })
    }

    fn recycle_page_slot(&mut self, mut slot: RecycledOffscreenPageSlot) {
        if self.recycled_page_slots.len() < MAX_RECYCLED_PAGE_SLOTS {
            let has_full_backing = slot.canvas.width() > 1 && slot.canvas.height() > 1;
            let backing_bytes = Self::canvas_backing_bytes(&slot.canvas);
            let retain_full_backing = self.hot_page_backing_pool_enabled
                && has_full_backing
                && self.hot_page_backing_pool_len < MAX_HOT_PAGE_BACKING_SLOTS
                && backing_bytes
                    <= MAX_HOT_PAGE_BACKING_BYTES.saturating_sub(self.hot_page_backing_pool_bytes);
            if retain_full_backing {
                slot.retains_full_backing = true;
                self.hot_page_backing_pool_len += 1;
                self.hot_page_backing_pool_bytes += backing_bytes;
                self.hot_page_backing_recycles += 1;
            } else {
                // Retain the expensive 2d context, not the full-resolution pixel
                // backing. Resizing the canvas also resets its context state before
                // the slot can represent another logical page.
                Self::shrink_canvas_backing(&slot.canvas);
                slot.retains_full_backing = false;
            }
            self.recycled_page_slots.push(slot);
            self.page_slot_recycles += 1;
        }
    }

    fn take_recycled_page_slot(
        &mut self,
        target_width: u32,
        target_height: u32,
    ) -> Option<RecycledOffscreenPageSlot> {
        if self.recycled_page_slots.is_empty() {
            return None;
        }
        let wants_full_backing = target_width > 1 && target_height > 1;
        let preferred_index = if wants_full_backing {
            self.recycled_page_slots.iter().rposition(|slot| {
                slot.retains_full_backing
                    && slot.canvas.width() == target_width
                    && slot.canvas.height() == target_height
            })
        } else {
            self.recycled_page_slots
                .iter()
                .rposition(|slot| !slot.retains_full_backing)
        };
        let index = preferred_index.unwrap_or_else(|| self.recycled_page_slots.len() - 1);
        let slot = self.recycled_page_slots.swap_remove(index);
        if slot.retains_full_backing {
            self.hot_page_backing_pool_len = self.hot_page_backing_pool_len.saturating_sub(1);
            self.hot_page_backing_pool_bytes = self
                .hot_page_backing_pool_bytes
                .saturating_sub(Self::canvas_backing_bytes(&slot.canvas));
            if slot.canvas.width() == target_width && slot.canvas.height() == target_height {
                self.hot_page_backing_reuses += 1;
            }
        }
        self.page_slot_reuses += 1;
        Some(slot)
    }

    fn recycle_active_page_slots_from(&mut self, desired: usize) {
        while desired < self.extra_canvases.len() {
            let canvas = self.extra_canvases.pop().expect("page canvas exists");
            let context = self.extra_contexts.pop().expect("page context exists");
            self.page_logical_sizes.pop();
            self.page_materialized.pop();
            self.extra_context_needs_rebuild
                .pop()
                .expect("page context rebuild flag exists");
            // Deferred placeholder slots never created a 2d context; there is
            // nothing worth pooling in their 1x1 canvas, so drop them.
            if let Some(context) = context {
                self.recycle_page_slot(RecycledOffscreenPageSlot {
                    canvas,
                    context,
                    retains_full_backing: false,
                });
            }
        }
    }

    fn initialize_scaled_context(
        context: &CanvasRenderingContext2d,
        device_pixel_ratio: f64,
    ) -> Result<(), JsValue> {
        // Reset any existing transform before applying the DPR scale.
        //
        // `get_context("2d")` on an OffscreenCanvas returns the *same* context
        // object whose transform matrix survives across calls.  When
        // `rebuild_context` is invoked on a reused canvas whose backing-store
        // dimensions have not changed (so `set_canvas_css_size` is a no-op and
        // the context is NOT implicitly reset), the previous
        // `scale(dpr, dpr)` is still in effect.  Without this explicit reset
        // the new `scale()` compounds with the old one, rendering content at
        // dpr² on the base (first-page) canvas while extra-page canvases
        // — which are created fresh via `OffscreenCanvas::new` — remain at the
        // correct dpr.
        //
        // DomRenderHost already performs this reset (see `set_transform` above).
        let _ = context.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
        if device_pixel_ratio.is_finite() && device_pixel_ratio > 0.0 {
            context.scale(device_pixel_ratio, device_pixel_ratio)?;
        }

        context.set_font("12pt 'Times New Roman', serif");
        context.set_fill_style_str("#000000");
        Ok(())
    }

    fn create_scaled_context(
        canvas: &OffscreenCanvas,
        device_pixel_ratio: f64,
    ) -> Result<CanvasRenderingContext2d, JsValue> {
        let raw_context = canvas
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("Failed to obtain 2d context"))?;

        // OffscreenCanvas always returns OffscreenCanvasRenderingContext2d.
        // Use unchecked_into to skip the expensive instanceof checks that
        // dyn_into would perform (first for CanvasRenderingContext2d which
        // always fails, then for OffscreenCanvasRenderingContext2d).
        let context: CanvasRenderingContext2d = raw_context.unchecked_into();

        Self::initialize_scaled_context(&context, device_pixel_ratio)?;
        Ok(context)
    }

    /// Resizes the canvas backing store when needed. Returns `true` when the
    /// backing dimensions actually changed (which implicitly clears the canvas
    /// and resets its context transform), so callers can flag the context for a
    /// rebuild. Returns `false` for a no-op resize (pixels + transform intact).
    fn set_canvas_css_size(canvas: &OffscreenCanvas, ratio: f64, width: f64, height: f64) -> bool {
        let device_width = (width * ratio).round().max(1.0) as u32;
        let device_height = (height * ratio).round().max(1.0) as u32;
        // Skip when dimensions are unchanged to avoid clearing canvas content
        // and resetting the rendering context.
        if canvas.width() != device_width || canvas.height() != device_height {
            canvas.set_width(device_width);
            canvas.set_height(device_height);
            true
        } else {
            false
        }
    }

    /// Shrinks a page canvas backing store to 1x1, releasing the large
    /// device-resolution buffer (deviceW*deviceH*4 bytes). The canvas object and
    /// its slot are retained so page-index alignment is preserved; the backing is
    /// restored on demand via `set_canvas_css_size` when the page re-enters the
    /// keep window.
    /// Returns `true` when the backing was actually shrunk (i.e. it was not
    /// already 1x1), which resets the canvas context transform.
    fn shrink_canvas_backing(canvas: &OffscreenCanvas) -> bool {
        let mut changed = false;
        if canvas.width() != 1 {
            canvas.set_width(1);
            changed = true;
        }
        if canvas.height() != 1 {
            canvas.set_height(1);
            changed = true;
        }
        changed
    }

    fn canvas_backing_bytes(canvas: &OffscreenCanvas) -> u64 {
        u64::from(canvas.width())
            .saturating_mul(u64::from(canvas.height()))
            .saturating_mul(4)
    }

    /// Whether the given 1-based page number must stay materialized at full
    /// resolution. `None` keep window => keep every page (non-lazy render).
    fn page_should_stay_materialized(&self, page_no: usize) -> bool {
        match self.keep_window {
            Some((start, end)) => page_no >= start && page_no <= end,
            None => true,
        }
    }

    fn rebuild_context(&mut self) -> Result<(), JsValue> {
        Self::initialize_scaled_context(&self.context, self.device_pixel_ratio)?;
        if self.pageless_mode {
            self.context.translate(0.0, -self.pageless_top)?;
        }
        Ok(())
    }

    fn activate_base_canvas(&mut self) -> Result<(), JsValue> {
        self.current_canvas = self.base_canvas.clone();
        self.context = self.base_context.clone();
        // Only re-apply the identity transform + dpr scale + default font/fill
        // when the base backing store was resized/cleared since the last
        // rebuild. A no-op resize leaves the context transform intact, so a
        // rebuild would just re-establish the exact same state (pure waste).
        if self.base_context_needs_rebuild {
            self.rebuild_context()?;
            self.base_context_needs_rebuild = false;
            self.context_rebuilds += 1;
        } else {
            self.context_rebuild_skips += 1;
        }
        Ok(())
    }

    fn activate_extra_canvas(&mut self, index: usize) -> Result<(), JsValue> {
        let canvas = self
            .extra_canvases
            .get(index)
            .ok_or_else(|| JsValue::from_str(&format!("Page index {} out of range", index + 1)))?
            .clone();
        let existing_context = self
            .extra_contexts
            .get(index)
            .ok_or_else(|| JsValue::from_str(&format!("Page context {} out of range", index + 1)))?
            .clone();
        let context = if let Some(context) = existing_context {
            context
        } else {
            // This slot was created as a 1x1 lazy-layout placeholder. Its
            // backing has already been inflated by `start_new_page`; create the
            // context only now that the page is about to be painted.
            let context = Self::create_scaled_context(&canvas, self.device_pixel_ratio)?;
            self.extra_contexts[index] = Some(context.clone());
            self.extra_context_needs_rebuild[index] = false;
            self.context_creations += 1;
            context
        };
        self.current_canvas = canvas;
        self.context = context;
        // Each extra page owns a persistent context whose transform survives
        // across passes. Rebuild only when this page's backing was resized
        // (inflated from 1x1 / shrunk) since its last rebuild — otherwise the
        // context still holds the correct dpr transform.
        let needs_rebuild = self
            .extra_context_needs_rebuild
            .get(index)
            .copied()
            .unwrap_or(true);
        if needs_rebuild {
            self.rebuild_context()?;
            if let Some(flag) = self.extra_context_needs_rebuild.get_mut(index) {
                *flag = false;
            }
            self.context_rebuilds += 1;
        } else {
            self.context_rebuild_skips += 1;
        }
        Ok(())
    }

    fn base_css_size(&self) -> (f64, f64) {
        self.base_logical_size
    }
}

impl CanvasRenderHost for OffscreenRenderHost {
    type Painter = Canvas2dPainter<CanvasRenderingContext2d>;

    fn painter(&self) -> Self::Painter {
        Canvas2dPainter::new(self.context().clone())
    }

    fn device_pixel_ratio(&self) -> f64 {
        self.device_pixel_ratio
    }

    fn update_device_pixel_ratio(&mut self) -> Result<bool, JsValue> {
        // The worker context does not automatically expose devicePixelRatio.
        // Callers are expected to provide the ratio explicitly when constructing the host.
        Ok(false)
    }

    fn set_device_pixel_ratio(&mut self, ratio: f64) -> Result<(), JsValue> {
        if !ratio.is_finite() || ratio <= 0.0 {
            return Err(JsValue::from_str("Invalid raster scale"));
        }
        if self.device_pixel_ratio == ratio {
            return Ok(());
        }
        self.device_pixel_ratio = ratio;
        let (width, height) = self.base_logical_size;
        Self::set_canvas_css_size(&self.base_canvas, ratio, width, height);
        self.base_context_needs_rebuild = true;
        self.base_has_pixels = false;
        // Keep offscreen pages lightweight; their logical sizes and slots
        // survive, and the ordinary visible-window repaint inflates them.
        for (index, canvas) in self.extra_canvases.iter().enumerate() {
            Self::shrink_canvas_backing(canvas);
            self.extra_context_needs_rebuild[index] = true;
            self.page_materialized[index] = false;
        }
        self.recycled_page_slots.clear();
        self.hot_page_backing_pool_len = 0;
        self.hot_page_backing_pool_bytes = 0;
        Ok(())
    }

    fn reset_pages(&mut self) -> Result<(), JsValue> {
        // Reset the logical page list immediately, but retain a tiny bounded
        // tail for the full repaint that commonly follows.
        self.recycle_active_page_slots_from(0);
        self.page_cursor = 0;
        self.activate_base_canvas()
    }

    fn begin_render(&mut self, reuse_pages: bool) -> Result<(), JsValue> {
        if reuse_pages {
            self.page_cursor = 0;
            self.activate_base_canvas()
        } else {
            self.reset_pages()
        }
    }

    fn configure_base_canvas(
        &mut self,
        width: f64,
        height: f64,
        pageless: bool,
        margin_top: f64,
    ) -> Result<(), JsValue> {
        self.pageless_mode = pageless;

        let target_height = if pageless {
            self.pageless_height = (margin_top + 1.0).ceil().max(1.0);
            pageless_raster_height(width, self.device_pixel_ratio)
        } else {
            height.max(1.0)
        };

        self.base_logical_size = (width, target_height);

        if Self::set_canvas_css_size(
            &self.base_canvas,
            self.device_pixel_ratio,
            width,
            target_height,
        ) {
            // Resizing the base backing cleared its context transform.
            self.base_context_needs_rebuild = true;
        }
        self.activate_base_canvas()?;
        self.extra_canvases.clear();
        self.extra_contexts.clear();
        self.page_logical_sizes.clear();
        self.page_materialized.clear();
        self.extra_context_needs_rebuild.clear();
        Ok(())
    }

    fn clear(&mut self, width: f64, pageless: bool, full_height: f64) -> Result<(), JsValue> {
        let height = if pageless {
            self.device_to_css(self.current_canvas.height() as f64)
        } else {
            full_height
        };
        self.context.clear_rect(
            0.0,
            if pageless { self.pageless_top } else { 0.0 },
            width,
            height,
        );
        Ok(())
    }

    fn ensure_pageless_height(
        &mut self,
        required_height: f64,
        margin_bottom: f64,
    ) -> Result<(), JsValue> {
        if !self.pageless_mode {
            return Ok(());
        }

        self.pageless_height = self
            .pageless_height
            .max((required_height + margin_bottom).ceil());
        Ok(())
    }

    fn pageless_viewport(&self) -> Option<(f64, f64)> {
        self.pageless_mode.then(|| {
            (
                self.pageless_top,
                self.device_to_css(self.base_canvas.height() as f64),
            )
        })
    }

    fn set_pageless_viewport(&mut self, top: f64) -> Result<(), JsValue> {
        if !self.pageless_mode || !top.is_finite() || top < 0.0 {
            return Err(JsValue::from_str("Invalid pageless viewport"));
        }
        self.pageless_top = top;
        self.base_context_needs_rebuild = true;
        self.activate_base_canvas()
    }

    fn start_new_page(&mut self, width: f64, height: f64, _margin_top: f64) -> Result<(), JsValue> {
        if self.pageless_mode {
            return Ok(());
        }

        self.page_cursor = self.page_cursor.saturating_add(1);
        // The base canvas is page 1, so the n-th `start_new_page` materializes
        // page (n + 1). Pages outside the keep window are kept as a 1x1
        // placeholder to bound memory on large lazily-scrolled documents.
        let page_no = self.page_cursor + 1;
        let keep = self.page_should_stay_materialized(page_no);

        if self.page_cursor <= self.extra_canvases.len() {
            // Reuse existing slot. `set_canvas_css_size` is a no-op when the
            // backing dimensions are unchanged, so in-window clean pages keep
            // their pixels; out-of-window pages collapse to 1x1 to free memory.
            let canvas_index = self.page_cursor - 1;
            self.page_logical_sizes[canvas_index] = (width, height);
            let canvas = self.extra_canvases[canvas_index].clone();
            let target_device = (
                (width * self.device_pixel_ratio).round().max(1.0) as u32,
                (height.max(1.0) * self.device_pixel_ratio).round().max(1.0) as u32,
            );
            // Resizing a canvas clears its pixels. So `materialized` (= "backing
            // holds valid pixels") must drop to false whenever the backing is
            // resized (inflated from 1x1) or shrunk; a no-op resize keeps the
            // preserved pixels and therefore the previous `materialized` state.
            // `mark_page_rendered` flips it back to true once the renderer
            // actually (re)paints the page this pass.
            let backing_changed =
                canvas.width() != target_device.0 || canvas.height() != target_device.1;
            if keep {
                // Count an inflate when re-materializing a shrunk (1x1)
                // placeholder to full resolution.
                let was_shrunk = canvas.width() <= 1 && canvas.height() <= 1;
                let resized = Self::set_canvas_css_size(
                    &canvas,
                    self.device_pixel_ratio,
                    width,
                    height.max(1.0),
                );
                if resized {
                    // Resizing cleared this page's context transform: it must be
                    // rebuilt on activation below.
                    if let Some(flag) = self.extra_context_needs_rebuild.get_mut(canvas_index) {
                        *flag = true;
                    }
                }
                if backing_changed {
                    self.page_materialized[canvas_index] = false;
                    if was_shrunk && resized {
                        self.canvas_inflates += 1;
                    }
                }
            } else if Self::shrink_canvas_backing(&canvas) {
                // Shrinking to 1x1 also resets the context transform.
                if let Some(flag) = self.extra_context_needs_rebuild.get_mut(canvas_index) {
                    *flag = true;
                }
                self.page_materialized[canvas_index] = false;
            } else {
                // Already 1x1: backing unchanged, but keep materialized=false so
                // the shrunk placeholder is never treated as holding pixels.
                self.page_materialized[canvas_index] = false;
            }
            return self.activate_extra_canvas(canvas_index);
        }

        let (device_width, device_height) = if keep {
            (
                (width * self.device_pixel_ratio).round().max(1.0),
                (height * self.device_pixel_ratio).round().max(1.0),
            )
        } else {
            // Allocate a placeholder for an out-of-window page; it is inflated to
            // full resolution by the reuse branch above once it re-enters the
            // keep window.
            (1.0, 1.0)
        };
        if let Some(recycled) =
            self.take_recycled_page_slot(device_width as u32, device_height as u32)
        {
            if recycled.canvas.width() != device_width as u32
                || recycled.canvas.height() != device_height as u32
            {
                recycled.canvas.set_width(device_width as u32);
                recycled.canvas.set_height(device_height as u32);
            }
            // The slot may represent another logical page. Reset clears pixels
            // and the complete drawing-state stack (including clips) before the
            // standard DPR/default state is restored.
            recycled.context.reset();
            Self::initialize_scaled_context(&recycled.context, self.device_pixel_ratio)?;
            self.context_rebuilds += 1;

            self.extra_canvases.push(recycled.canvas.clone());
            self.extra_contexts.push(Some(recycled.context.clone()));
            self.page_logical_sizes.push((width, height));
            self.page_materialized.push(false);
            self.extra_context_needs_rebuild.push(false);
            self.current_canvas = recycled.canvas;
            self.context = recycled.context;
            return Ok(());
        }

        let new_canvas = OffscreenCanvas::new(device_width as u32, device_height as u32)?;
        let context = Self::create_scaled_context(&new_canvas, self.device_pixel_ratio)?;
        self.context_creations += 1;

        self.extra_canvases.push(new_canvas.clone());
        self.extra_contexts.push(Some(context.clone()));
        self.page_logical_sizes.push((width, height));
        // Freshly allocated backing has no pixels yet; `mark_page_rendered` sets
        // this true once the page is painted this pass.
        self.page_materialized.push(false);
        // `create_scaled_context` above initialized this context with the
        // identity transform + dpr scale + default font/fill, so it needs no
        // rebuild on its first activation.
        self.extra_context_needs_rebuild.push(false);
        self.current_canvas = new_canvas;
        self.context = context;
        Ok(())
    }

    fn advance_page_without_touch(&mut self, width: f64, height: f64) -> bool {
        // Pageless mode has no per-page canvases to advance past.
        if self.pageless_mode {
            return false;
        }
        // The prospective page occupies slot `page_cursor` (0-based),
        // materializing 1-based page `page_cursor + 2`.
        let canvas_index = self.page_cursor;
        // The slot must already exist. If not, the page must be allocated via
        // `start_new_page` (we never allocate here).
        if canvas_index >= self.extra_canvases.len() {
            return false;
        }
        // Only skip when this page's logical size is unchanged. A mid-document
        // section page-size change must fall back to `start_new_page` so the
        // backing is resized (invariant: pages whose logical size changes still
        // resize).
        let (cached_w, cached_h) = self.page_logical_sizes[canvas_index];
        if (cached_w - width).abs() > f64::EPSILON || (cached_h - height).abs() > f64::EPSILON {
            return false;
        }
        // Preserve the keep-window boundedness invariant (bug #676): a page that
        // has left the keep window must still be shrunk. `start_new_page`'s reuse
        // branch does this lazily when it walks a page; because we skip that walk
        // here, apply the same eviction locally for out-of-window pages. In-window
        // (kept) clean pages retain their full-resolution backing untouched.
        let page_no = canvas_index + 2; // page 1 is the base canvas
        if !self.page_should_stay_materialized(page_no) {
            let canvas = &self.extra_canvases[canvas_index];
            if Self::shrink_canvas_backing(canvas) {
                // Shrinking to 1x1 resets the context transform and drops pixels.
                if let Some(flag) = self.extra_context_needs_rebuild.get_mut(canvas_index) {
                    *flag = true;
                }
            }
            if let Some(flag) = self.page_materialized.get_mut(canvas_index) {
                *flag = false;
            }
        }
        // Advance the cursor. We deliberately do NOT activate the canvas or
        // rebuild the context here:
        //   * Drawing is gated out for this page (the caller only skips pages
        //     outside the drawn window), so the context need not point at it.
        //   * An in-window kept page keeps its previous `page_materialized`
        //     value, so its pixels/state stay truthful; an out-of-window page is
        //     evicted above just as `start_new_page` would have.
        self.page_cursor = self.page_cursor.saturating_add(1);
        // The logical size already matches within EPSILON; leave the vector as-is
        // (it stays authoritative and byte-identical to what `start_new_page`
        // would have written for an unchanged page).
        self.pages_seek_skipped += 1;
        true
    }

    fn jump_to_page_without_touch(&mut self, page_number: usize) -> bool {
        if self.pageless_mode || page_number == 0 {
            return false;
        }
        let target_cursor = page_number.saturating_sub(1);
        if target_cursor < self.page_cursor || target_cursor > self.extra_canvases.len() {
            return false;
        }
        if target_cursor == self.page_cursor {
            return true;
        }
        // `set_page_keep_window` eagerly evicts pages as they leave the bounded
        // window. Existing clean slots therefore need no per-page work here.
        self.page_cursor = target_cursor;
        self.pages_seek_skipped += 1;
        true
    }

    fn advance_page_with_placeholder(&mut self, width: f64, height: f64) -> Result<bool, JsValue> {
        if self.pageless_mode {
            return Ok(false);
        }

        let canvas_index = self.page_cursor;
        // Existing slots are handled by `advance_page_without_touch`; refusing
        // them here keeps size-change and eviction behavior in one place.
        if canvas_index != self.extra_canvases.len() {
            return Ok(false);
        }
        let page_no = canvas_index + 2;
        if self.page_should_stay_materialized(page_no) {
            return Ok(false);
        }
        // A pooled cold slot already owns a 2d context; letting
        // `start_new_page` reuse it is cheaper than deferring here and paying
        // a fresh `getContext` when this page is eventually painted. Hot
        // full-backing slots are excluded so an offscreen placeholder never
        // consumes (and shrinks) a backing kept warm for in-window pages.
        if self
            .recycled_page_slots
            .iter()
            .any(|slot| !slot.retains_full_backing)
        {
            return Ok(false);
        }

        let placeholder = OffscreenCanvas::new(1, 1)?;
        self.extra_canvases.push(placeholder);
        self.extra_contexts.push(None);
        self.page_logical_sizes.push((width, height));
        self.page_materialized.push(false);
        // No context exists yet, so there is no transform to rebuild. The
        // eventual context is initialized after the backing is inflated.
        self.extra_context_needs_rebuild.push(false);
        self.page_cursor = self.page_cursor.saturating_add(1);
        self.context_creation_deferrals += 1;
        Ok(true)
    }

    fn activate_current_page(&mut self) -> Result<(), JsValue> {
        if self.pageless_mode {
            return Ok(());
        }
        if self.page_cursor == 0 {
            return self.activate_base_canvas();
        }
        let canvas_index = self.page_cursor - 1;
        let canvas = self
            .extra_canvases
            .get(canvas_index)
            .ok_or_else(|| {
                JsValue::from_str(&format!("Page index {} out of range", canvas_index + 1))
            })?
            .clone();
        let (width, height) = self
            .page_logical_sizes
            .get(canvas_index)
            .copied()
            .unwrap_or_else(|| {
                (
                    self.device_to_css(canvas.width() as f64),
                    self.device_to_css(canvas.height() as f64),
                )
            });
        // A page skipped at page start may hold a 1x1 placeholder or a shrunk
        // backing; inflate it before anything is painted on it.
        let was_shrunk = canvas.width() <= 1 && canvas.height() <= 1;
        if Self::set_canvas_css_size(&canvas, self.device_pixel_ratio, width, height.max(1.0)) {
            if let Some(flag) = self.extra_context_needs_rebuild.get_mut(canvas_index) {
                *flag = true;
            }
            if let Some(flag) = self.page_materialized.get_mut(canvas_index) {
                *flag = false;
            }
            if was_shrunk {
                self.canvas_inflates += 1;
            }
        }
        self.activate_extra_canvas(canvas_index)
    }

    fn context(&self) -> &CanvasRenderingContext2d {
        &self.context
    }

    fn context_mut(&mut self) -> &mut CanvasRenderingContext2d {
        &mut self.context
    }

    fn device_to_css(&self, value: f64) -> f64 {
        if self.device_pixel_ratio > 0.0 {
            value / self.device_pixel_ratio
        } else {
            value
        }
    }

    fn canvas_css_size(&self) -> (f64, f64) {
        if self.pageless_mode {
            let width = self.device_to_css(self.current_canvas.width() as f64);
            (width, self.pageless_height)
        } else {
            self.base_css_size()
        }
    }

    fn take_paged_snapshots(&mut self) -> Result<Option<Vec<JsValue>>, JsValue> {
        if self.pageless_mode {
            return Ok(None);
        }

        let mut snapshots = Vec::new();

        let base_width = self.device_to_css(self.base_canvas.width() as f64);
        let base_height = self.device_to_css(self.base_canvas.height() as f64);
        let base_bitmap = self.base_canvas.transfer_to_image_bitmap()?;
        // The transfer emptied the base canvas; band redraw must not run on it
        // until a render repaints it (see `base_has_pixels`).
        self.base_has_pixels = false;
        let base_obj = Object::new();
        Reflect::set(
            &base_obj,
            &JsValue::from_str("bitmap"),
            base_bitmap.as_ref(),
        )?;
        Reflect::set(
            &base_obj,
            &JsValue::from_str("width"),
            &JsValue::from_f64(base_width),
        )?;
        Reflect::set(
            &base_obj,
            &JsValue::from_str("height"),
            &JsValue::from_f64(base_height),
        )?;
        snapshots.push(base_obj.into());

        let ratio = if self.device_pixel_ratio > 0.0 {
            self.device_pixel_ratio
        } else {
            1.0
        };

        for extra in self.extra_canvases.drain(..) {
            let width = extra.width() as f64 / ratio;
            let height = extra.height() as f64 / ratio;
            let bitmap = extra.transfer_to_image_bitmap()?;
            let obj = Object::new();
            Reflect::set(&obj, &JsValue::from_str("bitmap"), bitmap.as_ref())?;
            Reflect::set(&obj, &JsValue::from_str("width"), &JsValue::from_f64(width))?;
            Reflect::set(
                &obj,
                &JsValue::from_str("height"),
                &JsValue::from_f64(height),
            )?;
            snapshots.push(obj.into());
        }
        self.extra_contexts.clear();
        self.extra_context_needs_rebuild.clear();

        // Transferring the base bitmap reset its context transform.
        self.base_context_needs_rebuild = true;
        // After transferring bitmaps the contexts are reset. Re-initialise base canvas for future renders.
        self.activate_base_canvas()?;

        Ok(Some(snapshots))
    }

    fn take_paged_snapshots_for_pages(
        &mut self,
        pages: Option<&[usize]>,
    ) -> Result<Option<Vec<JsValue>>, JsValue> {
        if self.pageless_mode {
            return Ok(None);
        }

        let total_pages = 1 + self.extra_canvases.len();
        let mut requested: Vec<usize> = match pages {
            None => (0..total_pages).collect(),
            Some(list) => list.iter().copied().filter(|p| *p < total_pages).collect(),
        };
        requested.sort_unstable();
        requested.dedup();

        if requested.is_empty() {
            return Ok(Some(Vec::new()));
        }

        let ratio = if self.device_pixel_ratio > 0.0 {
            self.device_pixel_ratio
        } else {
            1.0
        };

        let mut snapshots = Vec::with_capacity(requested.len());
        let mut transferred_base = false;

        for page_index in requested {
            let (width, height, bitmap) = if page_index == 0 {
                transferred_base = true;
                let width = self.device_to_css(self.base_canvas.width() as f64);
                let height = self.device_to_css(self.base_canvas.height() as f64);
                let bitmap = self.base_canvas.transfer_to_image_bitmap()?;
                (width, height, bitmap)
            } else {
                let canvas = self.extra_canvases.get(page_index - 1).ok_or_else(|| {
                    JsValue::from_str(&format!("Page index {} out of range", page_index))
                })?;
                let (width, height) = self
                    .page_logical_sizes
                    .get(page_index - 1)
                    .copied()
                    .unwrap_or_else(|| {
                        (
                            canvas.width() as f64 / ratio,
                            canvas.height() as f64 / ratio,
                        )
                    });
                let bitmap = canvas.transfer_to_image_bitmap()?;
                // Subset snapshots are used for lazy edit/bitmap refresh paths.
                // transferToImageBitmap clears the source canvas; shrink the
                // now-empty extra-page backing immediately so repeated late-page
                // snapshots do not retain full-resolution worker GPU buffers.
                Self::shrink_canvas_backing(canvas);
                if let Some(flag) = self.page_materialized.get_mut(page_index - 1) {
                    *flag = false;
                }
                // transferToImageBitmap + shrink both reset this page's context
                // transform, so it must be rebuilt on next activation.
                if let Some(flag) = self.extra_context_needs_rebuild.get_mut(page_index - 1) {
                    *flag = true;
                }
                (width, height, bitmap)
            };

            let obj = Object::new();
            Reflect::set(&obj, &JsValue::from_str("bitmap"), bitmap.as_ref())?;
            Reflect::set(&obj, &JsValue::from_str("width"), &JsValue::from_f64(width))?;
            Reflect::set(
                &obj,
                &JsValue::from_str("height"),
                &JsValue::from_f64(height),
            )?;
            Reflect::set(
                &obj,
                &JsValue::from_str("pageIndex"),
                &JsValue::from_f64(page_index as f64),
            )?;
            snapshots.push(obj.into());
        }

        if transferred_base {
            // Transferring the base bitmap reset its context transform.
            self.base_context_needs_rebuild = true;
            // The transfer also emptied the base canvas: band redraw must not run
            // on it until a render repaints it (see `base_has_pixels`).
            self.base_has_pixels = false;
            // After transferring the base bitmap the context is reset. Re-initialise base canvas for future renders.
            self.activate_base_canvas()?;
        }

        Ok(Some(snapshots))
    }

    fn page_count(&self) -> usize {
        if self.pageless_mode {
            1
        } else {
            1 + self.extra_canvases.len()
        }
    }

    fn take_page_text_capture(&self, page_index: usize) -> JsValue {
        let context = if page_index == 0 {
            Some(&self.base_context)
        } else {
            self.extra_contexts
                .get(page_index - 1)
                .and_then(Option::as_ref)
        };
        context.map_or(JsValue::NULL, |context| {
            jade_canvas_painter::text_batch::take_text_capture(context, self.device_pixel_ratio)
        })
    }

    fn page_dimensions(&self, page_index: usize) -> Option<(f64, f64)> {
        if self.pageless_mode {
            if page_index == 0 {
                Some(self.canvas_css_size())
            } else {
                None
            }
        } else if page_index == 0 {
            Some(self.base_logical_size)
        } else {
            // Report the logical page size, not the backing-store size: a page
            // whose backing was shrunk to 1x1 to free memory still occupies its
            // full logical footprint in the layout.
            self.page_logical_sizes
                .get(page_index - 1)
                .copied()
                .or_else(|| {
                    self.extra_canvases.get(page_index - 1).map(|canvas| {
                        (
                            self.device_to_css(canvas.width() as f64),
                            self.device_to_css(canvas.height() as f64),
                        )
                    })
                })
        }
    }

    fn truncate_pages(&mut self, total_pages: usize) -> Result<(), JsValue> {
        if self.pageless_mode {
            return Ok(());
        }
        let desired = total_pages.saturating_sub(1);
        self.recycle_active_page_slots_from(desired);
        Ok(())
    }

    fn set_page_keep_window(&mut self, window: Option<std::ops::RangeInclusive<usize>>) {
        let next = window.map(|range| (*range.start(), *range.end()));
        if self.keep_window == next {
            return;
        }
        self.keep_window = next;

        // Evict when the window changes rather than while the render cursor
        // walks the prefix. This makes a later seek jump O(1), while retaining
        // the invariant that pages outside the bounded window do not keep large
        // backing stores. Window changes are infrequent relative to extension
        // chunks; an unchanged window returns above without scanning pages.
        if self.keep_window.is_some() {
            for index in 0..self.extra_canvases.len() {
                let page_no = index + 2;
                if self.page_should_stay_materialized(page_no) {
                    continue;
                }
                if Self::shrink_canvas_backing(&self.extra_canvases[index]) {
                    if let Some(flag) = self.extra_context_needs_rebuild.get_mut(index) {
                        *flag = true;
                    }
                }
                if let Some(flag) = self.page_materialized.get_mut(index) {
                    *flag = false;
                }
            }
        }
    }

    fn set_hot_page_backing_pool_enabled(&mut self, enabled: bool) {
        self.hot_page_backing_pool_enabled = enabled;
        if !enabled && self.hot_page_backing_pool_len > 0 {
            for slot in &mut self.recycled_page_slots {
                if slot.retains_full_backing {
                    Self::shrink_canvas_backing(&slot.canvas);
                    slot.retains_full_backing = false;
                }
            }
            self.hot_page_backing_pool_len = 0;
            self.hot_page_backing_pool_bytes = 0;
        }
    }

    fn page_in_keep_window(&self, page_index: usize) -> bool {
        if self.pageless_mode || page_index == 0 {
            // The base canvas (page 0) is never shrunk, so it is effectively
            // always kept.
            return true;
        }
        // `page_should_stay_materialized` takes a 1-based page number; page_index
        // is 0-based (page 0 = base canvas), so extra page slot i corresponds to
        // 1-based page (i + 1).
        self.page_should_stay_materialized(page_index + 1)
    }

    fn page_has_rendered_pixels(&self, page_index: usize) -> bool {
        if self.pageless_mode {
            return true;
        }
        if page_index == 0 {
            // The base canvas is never shrunk, but a destructive bitmap transfer
            // (snapshot / fetch) blanks it until the next render. Band redraw must
            // see that so it does not clear+repaint only the band on a blank page.
            return self.base_has_pixels;
        }
        self.page_materialized
            .get(page_index - 1)
            .copied()
            .unwrap_or(false)
    }

    fn forget_page_pixels(&mut self, page_index: usize) {
        if page_index == 0 {
            self.base_has_pixels = false;
        } else if let Some(flag) = self.page_materialized.get_mut(page_index - 1) {
            *flag = false;
        }
    }

    fn mark_page_rendered(&mut self, page_index: usize) {
        if page_index == 0 {
            // The base canvas (page 0) was (re)painted this render: it now holds
            // valid full-page pixels again (see `base_has_pixels`).
            self.base_has_pixels = true;
            return;
        }
        if let Some(flag) = self.page_materialized.get_mut(page_index - 1) {
            *flag = true;
        }
    }

    fn create_page_bitmap(&self, page_index: usize) -> Result<Promise, JsValue> {
        if self.pageless_mode {
            return Err(JsValue::from_str(
                "create_page_bitmap is not supported in pageless mode",
            ));
        }

        let canvas: &OffscreenCanvas = if page_index == 0 {
            &self.base_canvas
        } else {
            self.extra_canvases.get(page_index - 1).ok_or_else(|| {
                JsValue::from_str(&format!("Page index {} out of range", page_index))
            })?
        };

        // Use createImageBitmap which is non-destructive (unlike transferToImageBitmap)
        let global = js_sys::global();
        let create_image_bitmap = Reflect::get(&global, &JsValue::from_str("createImageBitmap"))?;
        let create_fn: js_sys::Function = create_image_bitmap
            .dyn_into()
            .map_err(|_| JsValue::from_str("createImageBitmap is not a function"))?;

        let promise = create_fn.call1(&global, canvas)?;
        Ok(promise.unchecked_into())
    }

    fn transfer_page_bitmap(&mut self, page_index: usize) -> Result<ImageBitmap, JsValue> {
        if self.pageless_mode {
            return Err(JsValue::from_str(
                "transfer_page_bitmap is not supported in pageless mode",
            ));
        }

        let (canvas, extra_index): (&OffscreenCanvas, Option<usize>) = if page_index == 0 {
            (&self.base_canvas, None)
        } else {
            (
                self.extra_canvases.get(page_index - 1).ok_or_else(|| {
                    JsValue::from_str(&format!("Page index {} out of range", page_index))
                })?,
                Some(page_index - 1),
            )
        };

        let bitmap = canvas.transfer_to_image_bitmap()?;
        if let Some(index) = extra_index {
            // transferToImageBitmap clears the source canvas. Keep the logical
            // page slot but release the now-empty full-resolution backing so
            // late-page bitmap fetches do not accumulate large GPU buffers in
            // the worker. The next fetch/render inflates and repaints it.
            Self::shrink_canvas_backing(canvas);
            if let Some(flag) = self.page_materialized.get_mut(index) {
                *flag = false;
            }
            // transfer + shrink both reset this page's context transform.
            if let Some(flag) = self.extra_context_needs_rebuild.get_mut(index) {
                *flag = true;
            }
        } else {
            // Transferring the base bitmap reset its context transform.
            self.base_context_needs_rebuild = true;
            // The transfer emptied the base canvas; band redraw must not run on it
            // until a render repaints it (see `base_has_pixels`).
            self.base_has_pixels = false;
        }
        Ok(bitmap)
    }

    fn context_rebuild_stats(&self) -> (u64, u64, u64, u64, u64, u64) {
        (
            self.context_rebuilds,
            self.context_rebuild_skips,
            self.canvas_inflates,
            self.pages_seek_skipped,
            self.context_creations,
            self.context_creation_deferrals,
        )
    }

    fn page_slot_recycle_stats(&self) -> (u64, u64, u64, usize, u64, u64, usize, u64) {
        (
            self.context_creations,
            self.page_slot_recycles,
            self.page_slot_reuses,
            self.recycled_page_slots.len(),
            self.hot_page_backing_recycles,
            self.hot_page_backing_reuses,
            self.hot_page_backing_pool_len,
            self.hot_page_backing_pool_bytes,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{measure_alphabetic_text_metrics, snap_css_length_to_device_pixels};
    use jade_painter::recording::{Op, RecordingPainter};
    use jade_painter::Painter;

    #[test]
    fn text_metrics_use_alphabetic_baseline_and_preserve_cell_drawing_state() {
        let painter = RecordingPainter::with_width_per_char(7.0);
        painter.set_text_baseline("top");
        let metrics = measure_alphabetic_text_metrics(&painter, "After table").unwrap();
        assert_eq!(metrics.width, 77.0);
        assert_eq!(
            painter.ops(),
            vec![
                Op::SetTextBaseline("top".into()),
                Op::Save,
                Op::SetTextBaseline("alphabetic".into()),
                Op::MeasureText("After table".into()),
                Op::Restore,
            ]
        );
    }

    #[test]
    fn snap_css_length_to_device_pixels_rounds_to_backing_store_multiple() {
        let snapped = snap_css_length_to_device_pixels(816.33, 2.0);
        assert!((snapped - 816.5).abs() < 1e-6);
    }
}

#[cfg(test)]
mod pageless_viewport_tests {
    use super::pageless_raster_height;

    #[test]
    fn raster_memory_is_bounded_at_high_dpi_and_wide_document_sizes() {
        for ratio in [1.0, 2.0, 3.0, 4.0] {
            for width in [800.0, 1600.0, 3200.0] {
                let height = pageless_raster_height(width, ratio);
                assert!(height > 0.0);
                assert!(height * ratio <= 8192.0);
                assert!((width * ratio).ceil() * (height * ratio).round() <= 16_777_216.0);
            }
        }
    }
}
