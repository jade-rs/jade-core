//! Drives `jade-docx-editor`'s renderer with no browser.
//!
//! The renderer is the same code the browser runs, so what it lays out here is
//! what Word parity was earned against. Only the two things it reaches outside
//! itself are swapped: it paints through [`SkiaPainter`] instead of a canvas,
//! and it measures through `jade-docx-layout`'s shaping measurer instead of
//! `measureText`.
//!
//! ## What a non-canvas host has to answer, and what it does not
//!
//! `CanvasRenderHost` is shaped around a browser, but most of it turned out to
//! be bookkeeping rather than drawing. Three groups:
//!
//! * **Painting and measuring** — the parts that decide the output. Answered
//!   here for real.
//! * **Page geometry** — sizes and counts. Answered from the page this host
//!   was built for.
//! * **Canvas handles and bitmap snapshots** — `context()`, `context_mut()`
//!   and `create_page_bitmap()`. These exist for the browser's page-bitmap
//!   cache and have no meaning without a canvas, so they are `unreachable!`.
//!   A panic here would mean the renderer took a browser-only path, which is
//!   worth failing loudly rather than stubbing into silence.
//!
//! ## Errors
//!
//! The renderer reports `JsValue`, and wasm-bindgen cannot construct one off
//! the browser — `JsValue::from_str` panics. Nothing here builds one: the
//! painter is infallible, so every error this host returns is `Ok`. It does
//! mean a failure *inside* the renderer would panic rather than return, which
//! is a real limitation of rendering natively until the renderer's error type
//! stops being `JsValue`.

pub mod document;
pub use document::{
    CaretRect, CharacterFormat, FaceSpec, NativeDocument, ObjectHit, PdfExportOptions,
    PdfRenderingMode, SearchHit, TableCell,
};

use jade_docx_editor::render_host::{CanvasRenderHost, ImageCropMapping};
use jade_docx_layout::shaping_measurer::{ShapingMeasurer, ShapingOptions};
use jade_painter::{Painter, TextMetrics};
use jade_skia_painter::SkiaPainter;
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use wasm_bindgen::JsValue;

/// One raster surface per page, plus the measurement the renderer needs before
/// it can lay anything out.
pub struct SkiaRenderHost {
    /// A surface per page, in document order. The renderer asks for a new page
    /// as it paginates; giving each its own surface is what keeps page 2 from
    /// being painted on top of page 1.
    /// Shared so the caller can still read the pages after the host has been
    /// moved into the renderer, which takes ownership of it.
    pages: Rc<RefCell<Vec<SkiaPainter>>>,
    page_sizes: Vec<(f64, f64)>,
    /// Index into `pages` that `painter()` currently hands out.
    current_page: Cell<usize>,
    /// How many pages the last render produced. Surfaces past that are a pool
    /// kept for the next one rather than pages of the document: allocating a
    /// page-sized raster and filling it white was a quarter of every repaint.
    rendered_pages: Rc<Cell<usize>>,
    /// Whether the render in progress paginates the whole document.
    ///
    /// A bounded repaint stops at the page it was given, so the pages it
    /// produced are not the document's page count — believing them shrinks a
    /// three-page document to one the moment someone types on page 1.
    pass_covers_whole_document: Rc<Cell<bool>>,
    measurer: Rc<ShapingMeasurer>,
    recording: bool,
    pdf_options: PdfExportOptions,
    /// Page count already used by the live host to resolve early NUMPAGES fields.
    page_count_hint: usize,
    render_error: RefCell<Option<String>>,
    /// Parsed faces kept so a page created mid-render can be given the same
    /// ones; a fresh Skia surface starts with none, and a page whose faces were
    /// missing would lay out correctly and then draw nothing.
    faces: RefCell<Vec<(String, u16, jade_skia_painter::Typeface)>>,
    /// How many times the renderer asked this host to measure, and how long
    /// that took in total. Rendering natively is measurement-heavy, and a
    /// repaint that is not getting cheaper is usually not spending its time in
    /// the rasteriser — this makes that checkable rather than guessed at.
    shared_measure_calls: Rc<Cell<u64>>,
    shared_measure_nanos: Rc<Cell<u128>>,
    page_width: f64,
    page_height: f64,
    /// Device pixels per CSS pixel for the live raster pages: the window's
    /// backing scale, so a 2x screen is painted at 2x instead of shown a 1x
    /// page stretched to fit. Recording hosts ignore it and use their PDF
    /// options instead.
    raster_scale: f64,
    /// Whether the live raster pages show the document on a dark page. Shared
    /// so the document can switch it after the renderer has taken the host;
    /// recording hosts never set it, so print and export keep the document's
    /// own colours.
    dark_paper: Rc<Cell<bool>>,
}

#[derive(Clone)]
pub(crate) struct FontEnvironment {
    measurer: Rc<ShapingMeasurer>,
    faces: Vec<(String, u16, jade_skia_painter::Typeface)>,
}

impl SkiaRenderHost {
    /// Builds a host whose pages are `page_width` x `page_height` CSS pixels.
    pub fn new(page_width: f64, page_height: f64) -> Option<Self> {
        let first = SkiaPainter::new(page_width.ceil() as i32, page_height.ceil() as i32)?;
        Some(Self {
            pages: Rc::new(RefCell::new(vec![first])),
            page_sizes: vec![(page_width, page_height)],
            current_page: Cell::new(0),
            rendered_pages: Rc::new(Cell::new(1)),
            pass_covers_whole_document: Rc::new(Cell::new(true)),
            measurer: Rc::new(ShapingMeasurer::with_options(
                12.0,
                ShapingOptions::CHROMIUM_PARITY,
            )),
            recording: false,
            pdf_options: PdfExportOptions::default(),
            page_count_hint: 0,
            render_error: RefCell::new(None),
            faces: RefCell::new(Vec::new()),
            shared_measure_calls: Rc::new(Cell::new(0)),
            shared_measure_nanos: Rc::new(Cell::new(0)),
            page_width,
            page_height,
            raster_scale: 1.0,
            dark_paper: Rc::new(Cell::new(false)),
        })
    }

    fn font_environment(&self) -> FontEnvironment {
        FontEnvironment {
            measurer: Rc::clone(&self.measurer),
            faces: self.faces.borrow().clone(),
        }
    }

    fn recording_with_fonts(
        width: f64,
        height: f64,
        fonts: &FontEnvironment,
        options: PdfExportOptions,
        page_count_hint: usize,
    ) -> Option<Self> {
        let first = match options.rendering {
            PdfRenderingMode::Editor => {
                SkiaPainter::new_editor_recording(width, height, options.scale)?
            }
            PdfRenderingMode::Vector => SkiaPainter::new_recording(width, height)?,
        };
        for (family, weight, face) in &fonts.faces {
            first.register_face(family, *weight, face.clone());
        }
        Some(Self {
            pages: Rc::new(RefCell::new(vec![first])),
            page_sizes: vec![(width, height)],
            current_page: Cell::new(0),
            rendered_pages: Rc::new(Cell::new(1)),
            pass_covers_whole_document: Rc::new(Cell::new(true)),
            measurer: Rc::clone(&fonts.measurer),
            recording: true,
            pdf_options: options,
            page_count_hint,
            render_error: RefCell::new(None),
            faces: RefCell::new(fonts.faces.clone()),
            shared_measure_calls: Rc::new(Cell::new(0)),
            shared_measure_nanos: Rc::new(Cell::new(0)),
            page_width: width,
            page_height: height,
            raster_scale: 1.0,
            dark_paper: Rc::new(Cell::new(false)),
        })
    }

    /// The pages, shared with the host so they stay readable after the
    /// renderer has taken ownership of it.
    pub fn pages(&self) -> Rc<RefCell<Vec<SkiaPainter>>> {
        Rc::clone(&self.pages)
    }

    /// How many of those pages the last render actually produced.
    ///
    /// `pages()` is a pool and can be longer: a document that shrinks leaves
    /// surfaces behind for the next render to reuse, and counting them as pages
    /// would report a page that is no longer there.
    pub fn rendered_pages(&self) -> Rc<Cell<usize>> {
        Rc::clone(&self.rendered_pages)
    }

    /// Whether the next render paginates the whole document.
    ///
    /// Set it to `false` around a repaint bounded to a page: the page count
    /// then keeps what a whole-document render last found, instead of being
    /// cut down to the bound.
    pub fn pass_covers_whole_document(&self) -> Rc<Cell<bool>> {
        Rc::clone(&self.pass_covers_whole_document)
    }

    /// Whether the live pages are drawn on dark paper. Read when a surface is
    /// made, so a page added later matches the ones already there.
    pub fn dark_paper(&self) -> Rc<Cell<bool>> {
        Rc::clone(&self.dark_paper)
    }

    fn new_surface(&self, width: f64, height: f64) -> Option<SkiaPainter> {
        let surface = if self.recording {
            match self.pdf_options.rendering {
                PdfRenderingMode::Editor => {
                    SkiaPainter::new_editor_recording(width, height, self.pdf_options.scale)?
                }
                PdfRenderingMode::Vector => SkiaPainter::new_recording(width, height)?,
            }
        } else {
            let surface = SkiaPainter::new_scaled(width, height, self.raster_scale)?;
            surface.set_dark_paper(self.dark_paper.get());
            surface
        };
        // Parsed faces, not bytes. A new Skia surface starts with none, and a
        // document that paginates mid-render creates surfaces on the hot path —
        // re-parsing megabytes of font file per page put half a repaint in
        // `memmove`. A `Typeface` is a refcounted handle, so this is a refcount.
        for (family, weight, typeface) in self.faces.borrow().iter() {
            surface.register_face(family.as_str(), *weight, typeface.clone());
        }
        Some(surface)
    }

    fn ensure_page_size(&mut self, index: usize, width: f64, height: f64) {
        if self.page_sizes.get(index) == Some(&(width, height)) {
            return;
        }
        let surface = self
            .new_surface(width, height)
            .expect("valid layout page size");
        if index == self.page_sizes.len() {
            self.pages.borrow_mut().push(surface);
            self.page_sizes.push((width, height));
        } else {
            self.pages.borrow_mut()[index] = surface;
            self.page_sizes[index] = (width, height);
        }
    }

    /// Registers one face with **both** backends under the same family name.
    ///
    /// They have to agree: the measurer decides where a line breaks and the
    /// painter decides where the glyphs land, so a face known to only one of
    /// them produces a page whose text does not sit where layout put it.
    pub fn register_font(
        &mut self,
        family: &str,
        weight: u16,
        path: impl AsRef<Path>,
    ) -> Result<(), String> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|err| format!("{}: {err}", path.display()))?;
        // Every existing surface, and `new_surface` copies from the first, so a
        // page created later still knows the faces.
        for page in self.pages.borrow().iter() {
            page.register_font_bytes(family, weight, &bytes)
                .ok_or_else(|| format!("{}: skia rejected the face", path.display()))?;
        }
        // Taken back from the surface that just parsed them, so the bytes are
        // read and parsed exactly once.
        let just_registered = self
            .pages
            .borrow()
            .first()
            .and_then(|page| page.registered_faces().pop());
        if let Some((_, _, typeface)) = just_registered {
            self.faces
                .borrow_mut()
                .push((family.to_string(), weight, typeface));
        }
        Rc::get_mut(&mut self.measurer)
            .ok_or_else(|| "register fonts before sharing the font environment".to_string())?
            .register_font_path(
                family,
                weight,
                jade_docx_layout::font_measurer::RegisteredFontStyle::Normal,
                path,
            )
            .map_err(|err| format!("{}: {err:?}", path.display()))?;
        Ok(())
    }

    /// PNG bytes of `page_index`.
    pub fn encode_page_png(&self, page_index: usize) -> Option<Vec<u8>> {
        self.pages.borrow().get(page_index)?.encode_png()
    }

    /// A closure that reads the counters after the host has moved into the
    /// renderer, which takes ownership of it.
    pub fn measurement_handle(&self) -> impl Fn() -> (u64, std::time::Duration) {
        let calls = Rc::clone(&self.shared_measure_calls);
        let nanos = Rc::clone(&self.shared_measure_nanos);
        move || {
            (
                calls.get(),
                std::time::Duration::from_nanos(nanos.get() as u64),
            )
        }
    }

    /// Measurement calls made so far, and the time they took.
    pub fn measurement_stats(&self) -> (u64, std::time::Duration) {
        (
            self.shared_measure_calls.get(),
            std::time::Duration::from_nanos(self.shared_measure_nanos.get() as u64),
        )
    }

    /// How many pages the renderer has asked for so far.
    pub fn rendered_page_count(&self) -> usize {
        self.pages.borrow().len()
    }
}

impl CanvasRenderHost for SkiaRenderHost {
    type Painter = SkiaPainter;

    fn painter(&self) -> Self::Painter {
        // Cloning shares a surface, so every call within a page paints onto the
        // same pixels; the index is what makes it the *current* page.
        let pages = self.pages.borrow();
        let index = self.current_page.get().min(pages.len().saturating_sub(1));
        pages[index].clone()
    }

    // ---- measurement -------------------------------------------------------

    fn set_font(&self, font_descriptor: &str) {
        self.painter().set_font(font_descriptor);
    }

    fn draw_image_data_uri(
        &self,
        data_uri: &str,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        crop: Option<ImageCropMapping>,
    ) -> bool {
        // Decoded here rather than through the browser's `createImageBitmap`,
        // which is asynchronous and needs a canvas. A document with a picture
        // used to reach `context()` and panic.
        let Some(bytes) = decode_data_uri(data_uri) else {
            self.render_error
                .replace(Some("invalid image data URI".to_string()));
            return true;
        };
        let (source, destination) = match crop {
            Some(crop) => (
                Some([
                    crop.source_x,
                    crop.source_y,
                    crop.source_width,
                    crop.source_height,
                ]),
                (
                    x + width * crop.destination_x,
                    y + height * crop.destination_y,
                    width * crop.destination_width,
                    height * crop.destination_height,
                ),
            ),
            None => (None, (x, y, width, height)),
        };
        let drawn = self.painter().draw_encoded_image_rect(
            &bytes,
            source,
            destination.0,
            destination.1,
            destination.2,
            destination.3,
        );
        if !drawn {
            self.render_error
                .replace(Some("could not decode embedded image".to_string()));
        }
        // This backend owns image decoding, including failures. Falling back
        // to a browser canvas would panic and discard the actionable error.
        true
    }

    fn take_render_error(&self) -> Option<String> {
        self.render_error.borrow_mut().take()
    }

    fn measure_text_width(&self, text: &str) -> Result<f64, JsValue> {
        use jade_docx_layout::layout_cache::TextMeasurer;
        // Deliberately the measurer, not the painter: `jade-docx-layout` is the
        // single source of truth for advances, and it is what the Chromium
        // parity fixtures pin. Asking Skia would reintroduce a second opinion.
        let started = std::time::Instant::now();
        let width = self
            .measurer
            .measure_text(text, &self.painter().font_descriptor())
            .unwrap_or(0.0);
        self.shared_measure_calls
            .set(self.shared_measure_calls.get() + 1);
        self.shared_measure_nanos
            .set(self.shared_measure_nanos.get() + started.elapsed().as_nanos());
        Ok(width)
    }

    fn measure_text_metrics(&self, text: &str) -> Result<TextMetrics, JsValue> {
        let descriptor = self.painter().font_descriptor();
        let width = self.measure_text_width(text)?;
        let (font_ascent, font_descent) = self
            .measurer
            .font_bounding_box(&descriptor)
            .unwrap_or((0.0, 0.0));
        let (ink_ascent, ink_descent) = self
            .measurer
            .measure_font_vertical_metrics(&descriptor)
            .unwrap_or((font_ascent, font_descent));
        Ok(TextMetrics {
            width,
            font_bounding_box_ascent: font_ascent,
            font_bounding_box_descent: font_descent,
            actual_bounding_box_ascent: ink_ascent,
            actual_bounding_box_descent: ink_descent,
            actual_bounding_box_right: width,
        })
    }

    // ---- page geometry -----------------------------------------------------

    fn device_pixel_ratio(&self) -> f64 {
        if self.recording {
            match self.pdf_options.rendering {
                PdfRenderingMode::Editor => self.pdf_options.scale,
                PdfRenderingMode::Vector => 1.0,
            }
        } else {
            self.raster_scale
        }
    }

    fn update_device_pixel_ratio(&mut self) -> Result<bool, JsValue> {
        Ok(false)
    }

    fn set_device_pixel_ratio(&mut self, ratio: f64) -> Result<(), JsValue> {
        // Never `Err`: a `JsValue` cannot be built off the browser. The caller
        // validates the ratio; a recording host keeps the density its PDF
        // options chose.
        if self.recording || !ratio.is_finite() || ratio <= 0.0 || ratio == self.raster_scale {
            return Ok(());
        }
        self.raster_scale = ratio;
        // Every surface in the pool, not only the rendered ones: a page reused
        // later must not come back at the old density. The renderer marks a
        // full repaint after changing the scale, so nothing has to be copied.
        let surfaces = self
            .page_sizes
            .iter()
            .map(|&(width, height)| self.new_surface(width, height))
            .collect::<Option<Vec<_>>>()
            .expect("valid layout page size");
        *self.pages.borrow_mut() = surfaces;
        Ok(())
    }

    fn device_to_css(&self, value: f64) -> f64 {
        value / self.device_pixel_ratio()
    }

    fn canvas_css_size(&self) -> (f64, f64) {
        (self.page_width, self.page_height)
    }

    fn page_count(&self) -> usize {
        self.pages.borrow().len().max(self.page_count_hint)
    }

    fn page_dimensions(&self, page_index: usize) -> Option<(f64, f64)> {
        self.page_sizes.get(page_index).copied()
    }

    // ---- page lifecycle ----------------------------------------------------

    fn reset_pages(&mut self) -> Result<(), JsValue> {
        self.render_error.borrow_mut().take();
        // The surfaces are kept rather than dropped. The renderer clears a page
        // before drawing it, and only the pages it will draw, so a page it
        // skips still holds exactly what belongs on it — the page-bitmap cache
        // the browser pairs with a narrowed visible range, which this host has
        // in the form of one surface per page.
        self.current_page.set(0);
        if self.pass_covers_whole_document.get() {
            // Only a render that paginates everything may shrink the count.
            self.rendered_pages.set(1);
        }
        Ok(())
    }

    fn configure_base_canvas(
        &mut self,
        width: f64,
        height: f64,
        _pageless: bool,
        _margin_top: f64,
    ) -> Result<(), JsValue> {
        self.ensure_page_size(0, width, height);
        self.page_width = width;
        self.page_height = height;
        Ok(())
    }

    fn clear(
        &mut self,
        _page_width: f64,
        _pageless: bool,
        _full_height: f64,
    ) -> Result<(), JsValue> {
        Ok(())
    }

    fn ensure_pageless_height(
        &mut self,
        _required_height: f64,
        _margin_bottom: f64,
    ) -> Result<(), JsValue> {
        Ok(())
    }

    fn start_new_page(&mut self, w: f64, h: f64, _margin_top: f64) -> Result<(), JsValue> {
        // A surface of its own, or the page would be painted over the previous
        // one — but the one this page had last render will do. Only a document
        // that has grown needs a new one; `new_surface` re-registers the faces.
        let index = self.current_page.get() + 1;
        self.ensure_page_size(index, w, h);
        self.current_page.set(index);
        self.rendered_pages
            .set(self.rendered_pages.get().max(index + 1));
        Ok(())
    }

    // ---- browser-only ------------------------------------------------------

    fn context(&self) -> &web_sys::CanvasRenderingContext2d {
        unreachable!("native host: the renderer must not reach for a canvas")
    }

    fn context_mut(&mut self) -> &mut web_sys::CanvasRenderingContext2d {
        unreachable!("native host: the renderer must not reach for a canvas")
    }

    fn create_page_bitmap(&self, _page_index: usize) -> Result<js_sys::Promise, JsValue> {
        unreachable!("native host: page bitmaps are a browser-side cache")
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
}

/// The bytes behind a `data:` URI, if it carries base64.
///
/// Only the base64 form appears in a .docx: images are embedded as parts and
/// jade encodes them this way on the way to the renderer. Anything else — a
/// percent-encoded or malformed payload — returns `None` so the caller can
/// decline rather than draw garbage.
fn decode_data_uri(data_uri: &str) -> Option<Vec<u8>> {
    let (_, payload) = data_uri.split_once("base64,")?;
    let mut out = Vec::with_capacity(payload.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for byte in payload.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' => continue,
            _ => return None,
        } as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::decode_data_uri;
    use super::{CanvasRenderHost, SkiaRenderHost};

    #[test]
    fn measurement_follows_painter_font_changes_and_restore() {
        use jade_painter::Painter;

        let mut host = SkiaRenderHost::new(300.0, 300.0).unwrap();
        host.register_font(
            "Noto Sans JP",
            400,
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fonts/Noto_Sans_JP/static/NotoSansJP-Regular.ttf"),
        )
        .unwrap();
        host.set_font("10pt 'Noto Sans JP'");
        let small = host.measure_text_width("PDF 日本語").unwrap();
        assert!(small > 0.0);
        let painter = host.painter();
        painter.save();
        // Renderer painting sets the painter directly. Its next measurement
        // may reuse the same tracked font without calling the host setter.
        painter.set_font("20pt 'Noto Sans JP'");
        assert!((host.measure_text_width("PDF 日本語").unwrap() - 2.0 * small).abs() < 1e-6);
        host.set_font("30pt 'Noto Sans JP'");
        painter.restore();
        assert!((host.measure_text_width("PDF 日本語").unwrap() - small).abs() < 1e-6);
    }

    #[test]
    fn page_surfaces_follow_renderer_geometry_and_reallocate_when_it_changes() {
        let mut host = SkiaRenderHost::new(816.0, 1056.0).unwrap();
        host.configure_base_canvas(793.6, 1122.4, false, 0.0)
            .unwrap();
        host.start_new_page(1122.4, 793.6, 0.0).unwrap();
        assert_eq!(host.page_dimensions(0), Some((793.6, 1122.4)));
        assert_eq!(host.page_dimensions(1), Some((1122.4, 793.6)));
        assert_eq!(host.page_dimensions(2), None);
        let (_, w, h) = host.pages.borrow()[1].read_bgra_premultiplied().unwrap();
        assert_eq!((w, h), (1123, 794));
        host.reset_pages().unwrap();
        host.configure_base_canvas(600.5, 800.25, false, 0.0)
            .unwrap();
        host.start_new_page(600.5, 800.25, 0.0).unwrap();
        assert_eq!(host.page_dimensions(1), Some((600.5, 800.25)));
        let (_, w, h) = host.pages.borrow()[1].read_bgra_premultiplied().unwrap();
        assert_eq!((w, h), (601, 801));
    }

    #[test]
    fn a_raster_scale_multiplies_the_pixels_and_keeps_css_geometry() {
        let mut host = SkiaRenderHost::new(600.5, 800.25).unwrap();
        host.start_new_page(600.5, 800.25, 0.0).unwrap();
        host.set_device_pixel_ratio(2.0).unwrap();
        assert_eq!(host.device_pixel_ratio(), 2.0);
        assert_eq!(host.page_dimensions(1), Some((600.5, 800.25)));
        for page in host.pages.borrow().iter() {
            let (_, w, h) = page.read_bgra_premultiplied().unwrap();
            assert_eq!((w, h), (1201, 1601));
        }
        // A page the renderer adds afterwards comes at the same density.
        host.start_new_page(600.5, 800.25, 0.0).unwrap();
        let (_, w, h) = host.pages.borrow()[2].read_bgra_premultiplied().unwrap();
        assert_eq!((w, h), (1201, 1601));
        // Drawing stays in CSS pixels: a 10px square covers 20 device pixels.
        use jade_painter::Painter;
        let painter = host.painter();
        painter.set_fill_style_str("#000000");
        painter.fill_rect(0.0, 0.0, 10.0, 10.0);
        assert_eq!(painter.pixel(19, 19), Some((0, 0, 0, 255)));
        assert_eq!(painter.pixel(21, 21), Some((255, 255, 255, 255)));
    }

    #[test]
    fn a_base64_payload_decodes_to_its_bytes() {
        // "Man" is the canonical base64 example: three bytes, four characters,
        // no padding.
        assert_eq!(
            decode_data_uri("data:image/png;base64,TWFu").unwrap(),
            b"Man"
        );
        // One padding character means the last group carries two bytes.
        assert_eq!(
            decode_data_uri("data:image/png;base64,TWE=").unwrap(),
            b"Ma"
        );
        // Two mean it carries one.
        assert_eq!(decode_data_uri("data:image/png;base64,TQ==").unwrap(), b"M");
    }

    #[test]
    fn a_payload_that_is_not_base64_is_declined() {
        assert!(decode_data_uri("data:image/png,%89PNG").is_none());
        assert!(decode_data_uri("data:image/png;base64,not base64!").is_none());
    }
}
