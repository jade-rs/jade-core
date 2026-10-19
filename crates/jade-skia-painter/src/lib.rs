//! Skia backend for [`jade_painter::Painter`].
//!
//! This is the first backend that actually rasterises. `RecordingPainter`
//! proves the drawing code *runs* without a browser; this proves the trait
//! carries enough information to produce pixels.
//!
//! Skia is the engine Chromium's own canvas is built on, so the mapping is
//! close to one-to-one. The places it is not are the interesting ones, and each
//! is handled explicitly below:
//!
//! - **State stack.** `Canvas2D.save()` saves styles, font, and line width
//!   alongside the matrix and clip; `SkCanvas::save` saves only the matrix and
//!   clip. This backend therefore keeps its own style stack in step with
//!   Skia's.
//! - **Current path.** Canvas has one implicit path that `fill`/`stroke`/`clip`
//!   consume; Skia takes an explicit `Path`, so one is accumulated here.
//! - **Rotation units.** Canvas rotates in radians, `SkCanvas::rotate` in
//!   degrees.
//!
//! The crate lives outside the workspace: it pulls Skia's prebuilt C++
//! binaries, which should not land in everyone's `cargo test --all` builds.

use jade_painter::{Painter, TextMetrics};
use lopdf::dictionary;
use skia_safe::RoundOut;
use std::collections::HashMap;
mod pdf_text;
pub use skia_safe::Typeface;

/// A link in the painter's logical CSS-pixel page coordinate space.
pub struct PdfLink {
    pub page_index: usize,
    pub rect: (f64, f64, f64, f64),
    pub target: PdfLinkTarget,
    pub tooltip: Option<String>,
}

pub enum PdfLinkTarget {
    Uri(String),
    Page { page_index: usize, x: f64, y: f64 },
}

use skia_safe::{
    path::AddPathMode, surfaces, Canvas, Color, Color4f, Font, FontMgr, Matrix, Paint, Path,
    Picture, PictureRecorder, Rect, Surface,
};
use std::cell::RefCell;
use std::convert::Infallible;
use std::rc::Rc;

/// Style state that `Canvas2D.save()` preserves but `SkCanvas::save` does not.
#[derive(Clone)]
struct StyleState {
    fill: Color4f,
    stroke: Color4f,
    line_width: f32,
    /// Multiplies the alpha of every paint. Canvas keeps this in the saved
    /// state, and `StyleState` is what `save`/`restore` stack here, so it is
    /// restored with the rest.
    global_alpha: f32,
    dash: Vec<f32>,
    font_size_px: f32,
    font_index: usize,
    font_descriptor: Rc<str>,
    /// Canvas's shadow, which Word's shadow, emboss and imprint character
    /// effects are drawn with. Transparent means no shadow, as on a canvas.
    shadow_color: Color4f,
    shadow_blur: f32,
    shadow_offset: (f32, f32),
    blur_filter: f32,
    /// What the `y` given to `fill_text` means, as canvas's `textBaseline`.
    /// Skia always draws from the alphabetic baseline, so this decides how far
    /// the `y` has to be moved before the glyphs land where the caller meant.
    text_baseline: TextBaseline,
}

/// The canvas text baselines the renderer sets.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum TextBaseline {
    #[default]
    Alphabetic,
    Top,
    Hanging,
    Middle,
    Ideographic,
    Bottom,
}

impl TextBaseline {
    fn parse(value: &str) -> Self {
        match value {
            "top" => Self::Top,
            "hanging" => Self::Hanging,
            "middle" => Self::Middle,
            "ideographic" => Self::Ideographic,
            "bottom" => Self::Bottom,
            _ => Self::Alphabetic,
        }
    }

    /// How far below the given `y` the alphabetic baseline sits, for a font
    /// whose metrics these are. Skia reports the ascent as a negative offset.
    fn baseline_offset(self, metrics: &skia_safe::FontMetrics) -> f32 {
        match self {
            Self::Alphabetic | Self::Ideographic => 0.0,
            Self::Top => -metrics.ascent,
            // Canvas puts `hanging` at about 80% of the ascent, which is where
            // Chromium puts it when a face carries no baseline table.
            Self::Hanging => -metrics.ascent * 0.8,
            Self::Middle => (-metrics.ascent - metrics.descent) / 2.0,
            Self::Bottom => -metrics.descent,
        }
    }
}

impl Default for StyleState {
    fn default() -> Self {
        Self {
            fill: Color4f::new(0.0, 0.0, 0.0, 1.0),
            stroke: Color4f::new(0.0, 0.0, 0.0, 1.0),
            line_width: 1.0,
            global_alpha: 1.0,
            dash: Vec::new(),
            font_size_px: 16.0,
            font_index: 0,
            font_descriptor: Rc::from("16px sans-serif"),
            shadow_color: Color4f::new(0.0, 0.0, 0.0, 0.0),
            shadow_blur: 0.0,
            shadow_offset: (0.0, 0.0),
            blur_filter: 0.0,
            text_baseline: TextBaseline::Alphabetic,
        }
    }
}

struct RegisteredTypeface {
    family: String,
    weight: u16,
    typeface: Typeface,
}

/// Runs `f` against a process-wide `FontMgr`.
///
/// Building one is not cheap the way a constructor usually is: on macOS
/// `SkFontMgr_New_CoreText` calls `CTFontManagerCopyAvailableFontFamilyNames`,
/// which enumerates every installed family over XPC and sorts it with
/// locale-aware string comparison. Creating one per registered face put 76% of
/// a repaint inside CoreText, found by sampling rather than by reading the code
/// — nothing at the call site suggests `FontMgr::new()` talks to another
/// process.
///
/// Thread-local rather than a global because `FontMgr` is reference-counted and
/// not `Send`.
fn with_font_mgr<R>(f: impl FnOnce(&FontMgr) -> R) -> R {
    thread_local! {
        static FONT_MGR: FontMgr = FontMgr::new();
    }
    FONT_MGR.with(|mgr| f(mgr))
}

/// Paints onto an in-memory Skia raster surface.
///
/// Cloning is cheap and every clone paints onto the *same* surface, including
/// the shared style stack and path. That is required rather than convenient:
/// `CanvasRenderHost::painter` hands the renderer a painter by value, many
/// times per page, so a backend whose painter cannot be handed out repeatedly
/// cannot be a host's painter at all. Copying the surface per call would also
/// be wrong, not just slow — each call would paint onto its own pixels.
#[derive(Clone)]
pub struct SkiaPainter {
    shared: Rc<SharedSurface>,
}

/// The state every clone of a [`SkiaPainter`] shares.
struct SharedSurface {
    surface: RefCell<PaintSurface>,
    style: RefCell<StyleState>,
    stack: RefCell<Vec<StyleState>>,
    path: RefCell<Path>,
    /// Interior mutability so faces can be registered through a clone; the
    /// renderer only ever reads them.
    fonts: RefCell<Vec<RegisteredTypeface>>,
    images: RefCell<HashMap<String, skia_safe::Image>>,
    fill_text_calls: std::cell::Cell<u64>,
    effect_text: RefCell<Vec<pdf_text::RecordedText>>,
    /// Whether colours are mapped onto a dark page as they are drawn. See
    /// [`SkiaPainter::set_dark_paper`].
    dark_paper: std::cell::Cell<bool>,
}

enum PaintSurface {
    Raster(Surface),
    Recording {
        recorder: PictureRecorder,
        size: (f64, f64),
        picture: Option<Picture>,
        raster_scale: Option<f64>,
    },
}

impl SkiaPainter {
    /// The font on the current surface, including painter changes and the
    /// descriptor restored by `save` / `restore`.
    pub fn font_descriptor(&self) -> Rc<str> {
        self.shared.style.borrow().font_descriptor.clone()
    }

    /// Creates a `width` x `height` surface filled with opaque white.
    pub fn new(width: i32, height: i32) -> Option<Self> {
        let mut surface = surfaces::raster_n32_premul((width, height))?;
        surface.canvas().clear(Color::WHITE);
        Some(Self::with_surface(PaintSurface::Raster(surface)))
    }

    /// Creates a surface for a `width` x `height` CSS-pixel page with `scale`
    /// device pixels per CSS pixel, filled with opaque white.
    ///
    /// Drawing stays in CSS pixels: the scale is the canvas's base transform,
    /// below anything the renderer saves and restores, so a Retina screen gets
    /// glyphs rasterised at its own density rather than a 1x page stretched
    /// to fit — which is what makes text look soft.
    pub fn new_scaled(width: f64, height: f64, scale: f64) -> Option<Self> {
        if !width.is_finite() || !height.is_finite() || !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let mut surface = surfaces::raster_n32_premul((
            (width * scale).ceil() as i32,
            (height * scale).ceil() as i32,
        ))?;
        let canvas = surface.canvas();
        canvas.clear(Color::WHITE);
        canvas.scale((scale as f32, scale as f32));
        Some(Self::with_surface(PaintSurface::Raster(surface)))
    }

    /// Shows the page dark for reading on screen, or as the document has it.
    ///
    /// Every colour drawn from then on goes through
    /// [`jade_painter::paper::to_dark_paper`], and clearing paints the dark
    /// paper rather than white; images are drawn as they are. The surface is
    /// cleared to the new paper, so the caller repaints after changing it. A
    /// surface for print or export is never switched, which is what keeps the
    /// author's colours there.
    pub fn set_dark_paper(&self, on: bool) {
        if self.shared.dark_paper.replace(on) == on {
            return;
        }
        let paper = self.paper();
        self.with_canvas(|canvas| {
            canvas.clear(paper.to_color());
        });
    }

    /// The colour a cleared page shows.
    fn paper(&self) -> Color4f {
        if self.shared.dark_paper.get() {
            let [r, g, b] = jade_painter::paper::dark_paper();
            Color4f::new(r, g, b, 1.0)
        } else {
            Color4f::new(1.0, 1.0, 1.0, 1.0)
        }
    }

    /// `color` as it is drawn on this page.
    fn on_paper(&self, color: Color4f) -> Color4f {
        if !self.shared.dark_paper.get() {
            return color;
        }
        let [r, g, b] = jade_painter::paper::to_dark_paper([color.r, color.g, color.b]);
        Color4f::new(r, g, b, color.a)
    }

    /// Records the same drawing commands without allocating page pixels.
    /// Recorded pages can be sealed into a vector PDF with [`Self::encode_pdf`].
    pub fn new_recording(width: f64, height: f64) -> Option<Self> {
        Self::recording(width, height, None)
    }

    /// Records the live raster painter's commands for an appearance-preserving
    /// PDF page. Text stays searchable through an original-Unicode text layer.
    pub fn new_editor_recording(width: f64, height: f64, scale: f64) -> Option<Self> {
        if !scale.is_finite() || !(0.5..=4.0).contains(&scale) {
            return None;
        }
        Self::recording(width, height, Some(scale))
    }

    fn recording(width: f64, height: f64, raster_scale: Option<f64>) -> Option<Self> {
        if !width.is_finite()
            || !height.is_finite()
            || width < 1.0
            || height < 1.0
            || width > f32::MAX as f64
            || height > f32::MAX as f64
        {
            return None;
        }
        let mut recorder = PictureRecorder::new();
        let scale = raster_scale.unwrap_or(1.0);
        let canvas = recorder.begin_recording(
            Rect::from_xywh(
                0.0,
                0.0,
                (width * scale).ceil() as f32,
                (height * scale).ceil() as f32,
            ),
            None,
        );
        canvas.clear(Color::WHITE);
        canvas.scale((scale as f32, scale as f32));
        Some(Self::with_surface(PaintSurface::Recording {
            recorder,
            size: (width, height),
            picture: None,
            raster_scale,
        }))
    }

    fn with_surface(surface: PaintSurface) -> Self {
        Self {
            shared: Rc::new(SharedSurface {
                surface: RefCell::new(surface),
                style: RefCell::new(StyleState::default()),
                stack: RefCell::new(Vec::new()),
                path: RefCell::new(Path::new()),
                fonts: RefCell::new(Vec::new()),
                images: RefCell::new(HashMap::new()),
                fill_text_calls: std::cell::Cell::new(0),
                effect_text: RefCell::new(Vec::new()),
                dark_paper: std::cell::Cell::new(false),
            }),
        }
    }

    /// Seals recording pages into a PDF, preserving CSS-pixel page geometry.
    /// The pages must no longer be painted after this call. Text, fonts, paths,
    /// clips, and embedded images are replayed by Skia's PDF backend.
    pub fn encode_pdf(pages: &[Self]) -> Result<Vec<u8>, String> {
        Self::encode_pdf_with_links(pages, &[])
    }

    /// Adds link annotations without changing the recorded page drawing.
    pub fn encode_pdf_with_links(pages: &[Self], links: &[PdfLink]) -> Result<Vec<u8>, String> {
        if pages.is_empty() {
            return Err("no PDF pages".to_string());
        }
        let mut bytes = Vec::new();
        let mut sizes = Vec::with_capacity(pages.len());
        let metadata = skia_safe::pdf::Metadata {
            creator: "Jade".into(),
            raster_dpi: Some(96.0),
            ..Default::default()
        };
        let mut document = skia_safe::pdf::new_document(&mut bytes, Some(&metadata));
        for painter in pages {
            let mut surface = painter.shared.surface.borrow_mut();
            let PaintSurface::Recording {
                recorder,
                size,
                picture,
                raster_scale,
            } = &mut *surface
            else {
                return Err("PDF export requires recording pages".to_string());
            };
            if picture.is_none() {
                *picture = recorder.finish_recording_as_picture(None);
            }
            let picture = picture
                .as_ref()
                .ok_or_else(|| "could not seal the PDF page".to_string())?;
            sizes.push((size.0 * 0.75, size.1 * 0.75));
            let mut page =
                document.begin_page(((size.0 * 0.75) as f32, (size.1 * 0.75) as f32), None);
            let canvas = page.canvas();
            if let Some(scale) = raster_scale {
                let mut sampled = surfaces::raster_n32_premul((
                    (size.0 * *scale).ceil() as i32,
                    (size.1 * *scale).ceil() as i32,
                ))
                .ok_or("could not allocate the PDF page raster")?;
                sampled.canvas().clear(Color::WHITE);
                sampled.canvas().draw_picture(picture, None, None);
                canvas.scale(((0.75 / *scale) as f32, (0.75 / *scale) as f32));
                canvas.draw_image(sampled.image_snapshot(), (0.0, 0.0), None);
            } else {
                canvas.scale((0.75, 0.75));
                canvas.draw_picture(picture, None, None);
            }
            document = page.end_page();
        }
        document.close();
        // Skia rounds MediaBox to whole points. Restore the layout's exact
        // size and move its top-origin canvas by the corresponding delta.
        let mut pdf = lopdf::Document::load_mem(&bytes).map_err(|err| err.to_string())?;
        let page_ids = pdf.get_pages().values().copied().collect::<Vec<_>>();
        for (id, (width, height)) in page_ids.iter().copied().zip(sizes.iter().copied()) {
            let page = pdf
                .get_object(id)
                .and_then(lopdf::Object::as_dict)
                .map_err(|err| err.to_string())?;
            let old_height = page
                .get(b"MediaBox")
                .and_then(lopdf::Object::as_array)
                .and_then(|bounds| bounds[3].as_float())
                .map_err(|err| err.to_string())?;
            let transform = format!("q\n1 0 0 1 0 {} cm\n", height - f64::from(old_height));
            let mut content = transform.into_bytes();
            content.extend(pdf.get_page_content(id).map_err(|err| err.to_string())?);
            content.extend(b"\nQ\n");
            let stream = pdf.add_object(lopdf::Stream::new(lopdf::Dictionary::new(), content));
            let page = pdf
                .get_object_mut(id)
                .and_then(lopdf::Object::as_dict_mut)
                .map_err(|err| err.to_string())?;
            page.set("Contents", stream);
            page.set(
                "MediaBox",
                vec![
                    0.into(),
                    0.into(),
                    (width as f32).into(),
                    (height as f32).into(),
                ],
            );
        }
        for ((id, (_, height)), painter) in page_ids.iter().zip(&sizes).zip(pages) {
            pdf_text::append(&mut pdf, *id, *height, &painter.shared.effect_text.borrow())?;
        }
        for link in links {
            let &(page_width, page_height) =
                sizes.get(link.page_index).ok_or("invalid PDF link page")?;
            let (x, y, width, height) = link.rect;
            if ![x, y, width, height].iter().all(|value| value.is_finite())
                || width <= 0.0
                || height <= 0.0
            {
                return Err("invalid PDF link rectangle".into());
            }
            let left = (x * 0.75).max(0.0);
            let right = ((x + width) * 0.75).min(page_width);
            let top = page_height - (y * 0.75).max(0.0);
            let bottom = page_height - ((y + height) * 0.75).min(page_height);
            if right <= left || top <= bottom {
                continue;
            }
            let mut annotation = lopdf::dictionary! {
                "Type" => "Annot", "Subtype" => "Link",
                "Rect" => vec![(left as f32).into(), (bottom as f32).into(), (right as f32).into(), (top as f32).into()],
                "Border" => vec![0.into(), 0.into(), 0.into()],
            };
            match &link.target {
                PdfLinkTarget::Uri(target) => {
                    annotation.set("A", lopdf::dictionary! { "S" => "URI", "URI" => lopdf::Object::string_literal(target.as_bytes()) });
                }
                PdfLinkTarget::Page { page_index, x, y } => {
                    let id = page_ids
                        .get(*page_index)
                        .ok_or("invalid PDF destination page")?;
                    if !x.is_finite() || !y.is_finite() {
                        return Err("invalid PDF destination coordinates".into());
                    }
                    annotation.set(
                        "Dest",
                        vec![
                            (*id).into(),
                            lopdf::Object::Name(b"XYZ".to_vec()),
                            ((*x * 0.75) as f32).into(),
                            ((sizes[*page_index].1 - *y * 0.75) as f32).into(),
                            lopdf::Object::Null,
                        ],
                    );
                }
            }
            if let Some(tooltip) = &link.tooltip {
                let utf16 = [0xfe, 0xff]
                    .into_iter()
                    .chain(tooltip.encode_utf16().flat_map(u16::to_be_bytes))
                    .collect::<Vec<_>>();
                annotation.set(
                    "Contents",
                    lopdf::Object::String(utf16, lopdf::StringFormat::Hexadecimal),
                );
            }
            let id = pdf.add_object(annotation);
            let page = pdf
                .get_object_mut(page_ids[link.page_index])
                .and_then(lopdf::Object::as_dict_mut)
                .map_err(|err| err.to_string())?;
            if !page.has(b"Annots") {
                page.set("Annots", Vec::<lopdf::Object>::new());
            }
            page.get_mut(b"Annots")
                .and_then(lopdf::Object::as_array_mut)
                .map_err(|err| err.to_string())?
                .push(id.into());
        }
        pdf.prune_objects();
        pdf.compress();
        bytes.clear();
        pdf.save_to(&mut bytes).map_err(|err| err.to_string())?;
        Ok(bytes)
    }

    /// Registers a font file under a family name and weight, the same way the
    /// measurement backend does, so a CSS font shorthand can be resolved.
    pub fn register_font_bytes(
        &self,
        family: impl Into<String>,
        weight: u16,
        bytes: &[u8],
    ) -> Option<()> {
        let typeface = with_font_mgr(|mgr| mgr.new_from_data(bytes, None))?;
        self.shared.fonts.borrow_mut().push(RegisteredTypeface {
            family: family.into(),
            weight,
            typeface,
        });
        Some(())
    }

    /// The faces registered here, ready to hand to another surface.
    ///
    /// A `Typeface` is a refcounted handle to already-parsed font data, so
    /// carrying these to a new surface costs a refcount. Re-registering from
    /// bytes instead re-parses and re-copies the file — which is megabytes per
    /// face, and showed up as half a repaint once a document paginated mid
    /// render.
    pub fn registered_faces(&self) -> Vec<(String, u16, Typeface)> {
        self.shared
            .fonts
            .borrow()
            .iter()
            .map(|entry| (entry.family.clone(), entry.weight, entry.typeface.clone()))
            .collect()
    }

    /// Registers an already-parsed face, as returned by [`Self::registered_faces`].
    pub fn register_face(&self, family: impl Into<String>, weight: u16, typeface: Typeface) {
        self.shared.fonts.borrow_mut().push(RegisteredTypeface {
            family: family.into(),
            weight,
            typeface,
        });
    }

    /// Calls to `fill_text` so far.
    pub fn fill_text_calls(&self) -> u64 {
        self.shared.fill_text_calls.get()
    }

    /// Where the alphabetic baseline goes for a `y` given under the current
    /// `textBaseline`.
    ///
    /// Canvas lets the caller say what the `y` it passes means; Skia only ever
    /// draws from the alphabetic baseline. The renderer draws table cells with
    /// `textBaseline = "top"`, so ignoring this drew every cell's text an
    /// ascent too high — above the row's top border, cut in half by it.
    fn baseline_y(&self, font: &Font, y: f64) -> f32 {
        let baseline = self.shared.style.borrow().text_baseline;
        if baseline == TextBaseline::Alphabetic {
            return y as f32;
        }
        let (_, metrics) = font.metrics();
        y as f32 + baseline.baseline_offset(&metrics)
    }

    fn draw_text(&self, text: &str, font: &Font, x: f32, y: f32, paint: &Paint) {
        if text.is_empty() {
            return;
        }
        let glyphs = font.str_to_glyphs_vec(text);
        let mut builder = skia_safe::TextBlobBuilder::new();
        let (run, utf8, clusters) =
            builder.alloc_run_text(font, glyphs.len(), (0.0, 0.0), text.len(), None);
        run.copy_from_slice(&glyphs);
        utf8.copy_from_slice(text.as_bytes());
        for (cluster, (offset, _)) in clusters.iter_mut().zip(text.char_indices()) {
            *cluster = offset as u32;
        }
        let Some(blob) = builder.make() else {
            return;
        };
        let editor_scale = match &*self.shared.surface.borrow() {
            PaintSurface::Recording { raster_scale, .. } => *raster_scale,
            _ => None,
        };
        let path_text = editor_scale.is_none()
            && matches!(
                &*self.shared.surface.borrow(),
                PaintSurface::Recording { .. }
            )
            && (glyphs.contains(&0)
                || paint.style() != skia_safe::paint::Style::Fill
                || paint.image_filter().is_some()
                || paint.mask_filter().is_some());
        self.with_canvas(|canvas| {
            if !path_text && editor_scale.is_none() {
                canvas.draw_text_blob(&blob, (x, y), paint);
                return;
            }
            // PDF text operators cannot express every canvas text effect, and
            // Skia omits missing glyphs' Unicode. Preserve the same glyph paths
            // and advances alongside an invisible original-Unicode text layer.
            let mut widths = vec![0.0; glyphs.len()];
            font.get_widths(&glyphs, &mut widths);
            let (_, metrics) = font.metrics();
            let [a, b, c, d, e, f] = canvas
                .local_to_device_as_3x3()
                .to_affine()
                .expect("2D painter transform")
                .map(|value| value / editor_scale.unwrap_or(1.0) as f32);
            self.shared
                .effect_text
                .borrow_mut()
                .push(pdf_text::RecordedText {
                    text: text.to_string(),
                    font_size: font.size(),
                    widths: widths.clone(),
                    ascent: -metrics.ascent,
                    descent: metrics.descent,
                    transform: [a, b, c, d, a * x + c * y + e, b * x + d * y + f],
                });
            if editor_scale.is_some() {
                canvas.draw_text_blob(&blob, (x, y), paint);
                return;
            }
            if paint.style() == skia_safe::paint::Style::Stroke {
                // A stroked text blob uses Skia's glyph rasterizer, whose
                // coverage differs from a PDF path stroke. Sample only this
                // effect through the live painter, retaining searchable text.
                let matrix = canvas.local_to_device_as_3x3();
                let style = self.shared.style.borrow();
                let padding = style.line_width / 2.0
                    + 3.0 * (style.shadow_blur + style.blur_filter)
                    + style.shadow_offset.0.abs().max(style.shadow_offset.1.abs())
                    + 1.0;
                let bounds: skia_safe::IRect = matrix
                    .map_rect(
                        blob.bounds()
                            .with_offset((x, y))
                            .with_outset((padding, padding)),
                    )
                    .0
                    .round_out();
                drop(style);
                if let Some(mut sampled) =
                    surfaces::raster_n32_premul((bounds.width(), bounds.height()))
                {
                    sampled.canvas().clear(Color::TRANSPARENT);
                    sampled
                        .canvas()
                        .translate((-bounds.left as f32, -bounds.top as f32));
                    sampled.canvas().concat(&matrix);
                    sampled.canvas().draw_text_blob(&blob, (x, y), paint);
                    let image = sampled.image_snapshot();
                    canvas.save();
                    canvas.reset_matrix();
                    canvas.draw_image(&image, (bounds.left as f32, bounds.top as f32), None);
                    canvas.restore();
                    return;
                }
            }
            let mut path = Path::new();
            let mut advance = 0.0;
            for (glyph, width) in glyphs.into_iter().zip(widths) {
                if let Some(glyph_path) = font.get_path(glyph) {
                    path.add_path(&glyph_path, (advance, 0.0), None);
                }
                advance += width;
            }
            canvas.save();
            canvas.translate((x, y));
            canvas.draw_path(&path, paint);
            canvas.restore();
        });
    }

    /// PNG bytes of everything painted so far.
    pub fn encode_png(&self) -> Option<Vec<u8>> {
        let mut target = self.shared.surface.borrow_mut();
        let PaintSurface::Raster(surface) = &mut *target else {
            return None;
        };
        let image = surface.image_snapshot();
        let data = image.encode(None, skia_safe::EncodedImageFormat::PNG, 100)?;
        Some(data.as_bytes().to_vec())
    }

    /// Draws encoded image bytes (PNG, JPEG, ...) into the given rectangle.
    ///
    /// Returns `false` when Skia cannot decode them, so a caller can fall back
    /// rather than silently drawing nothing.
    pub fn draw_encoded_image(
        &self,
        bytes: &[u8],
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> bool {
        self.draw_encoded_image_rect(bytes, None, x, y, width, height)
    }

    /// Draws a normalized source rectangle from encoded image bytes.
    pub fn draw_encoded_image_rect(
        &self,
        bytes: &[u8],
        source: Option<[f64; 4]>,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> bool {
        let data = skia_safe::Data::new_copy(bytes);
        let Some(image) = skia_safe::Image::from_encoded(data) else {
            return self.draw_svg_image_rect(bytes, source, x, y, width, height);
        };
        let source = source.map(|[source_x, source_y, source_width, source_height]| {
            Rect::from_xywh(
                (source_x * image.width() as f64) as f32,
                (source_y * image.height() as f64) as f32,
                (source_width * image.width() as f64) as f32,
                (source_height * image.height() as f64) as f32,
            )
        });
        let destination = Rect::from_xywh(x as f32, y as f32, width as f32, height as f32);
        let paint = self.fill_paint();
        let recording = matches!(
            *self.shared.surface.borrow(),
            PaintSurface::Recording {
                raster_scale: None,
                ..
            }
        );
        self.with_canvas(|canvas| {
            let source = source
                .as_ref()
                .map(|source| (source, skia_safe::canvas::SrcRectConstraint::Strict));
            if recording {
                // PDF readers choose their own downsampling for embedded
                // images. Sample raster pictures through the live Skia painter
                // first, so tiny borders and cropped details cannot change.
                // Text, paths and SVG still remain vector drawing commands.
                let matrix = canvas.local_to_device_as_3x3();
                let style = self.shared.style.borrow();
                let padding = 3.0 * (style.shadow_blur + style.blur_filter)
                    + style.shadow_offset.0.abs().max(style.shadow_offset.1.abs());
                let bounds: skia_safe::IRect = matrix
                    .map_rect(destination.with_outset((padding, padding)))
                    .0
                    .round_out();
                drop(style);
                let Some(mut sampled) =
                    surfaces::raster_n32_premul((bounds.width(), bounds.height()))
                else {
                    return false;
                };
                sampled.canvas().clear(Color::TRANSPARENT);
                sampled
                    .canvas()
                    .translate((-bounds.left as f32, -bounds.top as f32));
                sampled.canvas().concat(&matrix);
                sampled
                    .canvas()
                    .draw_image_rect(&image, source, destination, &paint);
                let image = sampled.image_snapshot();
                canvas.save();
                canvas.reset_matrix();
                canvas.draw_image(&image, (bounds.left as f32, bounds.top as f32), None);
                canvas.restore();
            } else {
                canvas.draw_image_rect(&image, source, destination, &paint);
            }
            true
        })
    }

    fn draw_decoded_image(
        &self,
        image: &skia_safe::Image,
        source: Option<[f64; 4]>,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> bool {
        let source = source.map(|[source_x, source_y, source_width, source_height]| {
            Rect::from_xywh(
                (source_x * image.width() as f64) as f32,
                (source_y * image.height() as f64) as f32,
                (source_width * image.width() as f64) as f32,
                (source_height * image.height() as f64) as f32,
            )
        });
        let destination = Rect::from_xywh(x as f32, y as f32, width as f32, height as f32);
        let paint = self.fill_paint();
        self.with_canvas(|canvas| {
            canvas.draw_image_rect(
                image,
                source
                    .as_ref()
                    .map(|source| (source, skia_safe::canvas::SrcRectConstraint::Strict)),
                destination,
                &paint,
            );
        });
        true
    }

    fn draw_svg_image_rect(
        &self,
        bytes: &[u8],
        source: Option<[f64; 4]>,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> bool {
        let Ok(mut dom) = with_font_mgr(|mgr| skia_safe::svg::Dom::from_bytes(bytes, mgr.clone()))
        else {
            return false;
        };
        let intrinsic = dom.root().intrinsic_size();
        let svg_width = if intrinsic.width > 0.0 {
            intrinsic.width as f64
        } else {
            width
        };
        let svg_height = if intrinsic.height > 0.0 {
            intrinsic.height as f64
        } else {
            height
        };
        let [sx, sy, sw, sh] = source.unwrap_or([0.0, 0.0, 1.0, 1.0]);
        if sw <= 0.0 || sh <= 0.0 || svg_width <= 0.0 || svg_height <= 0.0 {
            return false;
        }
        dom.set_container_size((svg_width as f32, svg_height as f32));
        let alpha = self.shared.style.borrow().global_alpha;
        self.with_canvas(|canvas| {
            canvas.save();
            canvas.clip_rect(
                Rect::from_xywh(x as f32, y as f32, width as f32, height as f32),
                None,
                true,
            );
            canvas.translate((x as f32, y as f32));
            canvas.scale((
                (width / (svg_width * sw)) as f32,
                (height / (svg_height * sh)) as f32,
            ));
            canvas.translate(((-sx * svg_width) as f32, (-sy * svg_height) as f32));
            if alpha < 1.0 {
                canvas.save_layer_alpha_f(None, alpha);
            }
            dom.render(canvas);
            if alpha < 1.0 {
                canvas.restore();
            }
            canvas.restore();
        });
        true
    }

    /// The surface's pixels as BGRA with premultiplied alpha, plus its size.
    ///
    /// That layout is not arbitrary: it is what a GPU compositor wants on
    /// Apple platforms, and what gpui's `RenderImage` documents itself as
    /// holding. Reading in the compositor's own order avoids a per-pixel swap
    /// on the way to the screen.
    pub fn read_bgra_premultiplied(&self) -> Option<(Vec<u8>, i32, i32)> {
        let mut target = self.shared.surface.borrow_mut();
        let PaintSurface::Raster(surface) = &mut *target else {
            return None;
        };
        let image = surface.image_snapshot();
        let (width, height) = (image.width(), image.height());
        let info = skia_safe::ImageInfo::new(
            (width, height),
            skia_safe::ColorType::BGRA8888,
            skia_safe::AlphaType::Premul,
            None,
        );
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        image
            .read_pixels(
                &info,
                &mut pixels,
                (width * 4) as usize,
                (0, 0),
                skia_safe::image::CachingHint::Allow,
            )
            .then_some((pixels, width, height))
    }

    /// Colour of one pixel as `(r, g, b, a)`, for assertions in tests.
    pub fn pixel(&self, x: i32, y: i32) -> Option<(u8, u8, u8, u8)> {
        let mut target = self.shared.surface.borrow_mut();
        let PaintSurface::Raster(surface) = &mut *target else {
            return None;
        };
        let image = surface.image_snapshot();
        let info = skia_safe::ImageInfo::new(
            (1, 1),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Unpremul,
            None,
        );
        let mut pixels = [0u8; 4];
        image
            .read_pixels(
                &info,
                &mut pixels,
                4,
                (x, y),
                skia_safe::image::CachingHint::Allow,
            )
            .then(|| (pixels[0], pixels[1], pixels[2], pixels[3]))
    }

    fn with_canvas<R>(&self, f: impl FnOnce(&Canvas) -> R) -> R {
        let mut surface = self.shared.surface.borrow_mut();
        match &mut *surface {
            PaintSurface::Raster(surface) => f(surface.canvas()),
            PaintSurface::Recording { recorder, .. } => f(recorder
                .recording_canvas()
                .expect("recording page must not be painted after PDF sealing")),
        }
    }

    fn fill_paint(&self) -> Paint {
        let style = self.shared.style.borrow();
        let mut color = self.on_paper(style.fill);
        color.a *= style.global_alpha;
        let mut paint = Paint::new(color, None);
        paint.set_anti_alias(true);
        Self::apply_shadow(&mut paint, &style);
        Self::apply_blur_filter(&mut paint, &style);
        paint
    }

    /// Puts the canvas shadow on a paint, if one is set.
    ///
    /// Word's shadow, emboss and imprint character effects are drawn as canvas
    /// shadows, so a backend that ignores them draws those runs plain. Canvas
    /// blur is a diameter and Skia takes a standard deviation, which is half of
    /// it — the same relation Chromium uses.
    fn apply_shadow(paint: &mut Paint, style: &StyleState) {
        let has_offset = style.shadow_offset.0 != 0.0 || style.shadow_offset.1 != 0.0;
        if style.shadow_color.a <= 0.0 || (!has_offset && style.shadow_blur <= 0.0) {
            return;
        }
        let sigma = (style.shadow_blur / 2.0).max(0.0);
        let mut color = style.shadow_color;
        color.a *= style.global_alpha;
        paint.set_image_filter(skia_safe::image_filters::drop_shadow(
            style.shadow_offset,
            (sigma, sigma),
            color.to_color(),
            None,
            None,
            skia_safe::image_filters::CropRect::NO_CROP_RECT,
        ));
    }

    fn apply_blur_filter(paint: &mut Paint, style: &StyleState) {
        if style.blur_filter > 0.0 && style.blur_filter.is_finite() {
            paint.set_mask_filter(skia_safe::MaskFilter::blur(
                skia_safe::BlurStyle::Normal,
                style.blur_filter,
                true,
            ));
        }
    }

    fn stroke_paint(&self) -> Paint {
        let style = self.shared.style.borrow();
        let mut color = self.on_paper(style.stroke);
        color.a *= style.global_alpha;
        let mut paint = Paint::new(color, None);
        paint.set_anti_alias(true);
        paint.set_stroke(true);
        paint.set_stroke_width(style.line_width);
        if !style.dash.is_empty() {
            paint.set_path_effect(skia_safe::PathEffect::dash(&style.dash, 0.0));
        }
        Self::apply_shadow(&mut paint, &style);
        Self::apply_blur_filter(&mut paint, &style);
        paint
    }

    /// The styled font, or a registered face that can actually draw `text`.
    ///
    /// Text is drawn one grapheme at a time, so this is a per-character
    /// fallback — the same thing the measurer does, and the same thing a
    /// browser does. Without it a glyph the styled face lacks comes out as a
    /// tofu box: formatting marks are the common case, since a Japanese text
    /// face carries the pilcrow but not the arrows Word uses for line and page
    /// breaks.
    fn font_covering(&self, text: &str) -> Font {
        let font = self.font();
        let Some(first) = text.chars().next() else {
            return font;
        };
        if font.typeface().unichar_to_glyph(first as i32) != 0 {
            return font;
        }
        let size_px = self.shared.style.borrow().font_size_px;
        let fonts = self.shared.fonts.borrow();
        for entry in fonts.iter() {
            if entry.typeface.unichar_to_glyph(first as i32) != 0 {
                return Font::from_typeface(entry.typeface.clone(), size_px);
            }
        }
        font
    }

    /// `text` split where the face drawing it changes: each character in
    /// [`Self::font_covering`]'s face for it — so a run mixing scripts is
    /// measured and drawn as a browser does, each in a face that has it.
    fn segments<'t>(&self, text: &'t str) -> Vec<(Font, &'t str)> {
        let mut segments: Vec<(Font, &'t str)> = Vec::new();
        let mut start = 0;
        for (at, ch) in text.char_indices() {
            let end = at + ch.len_utf8();
            let font = self.font_covering(&text[at..end]);
            match segments.last_mut() {
                Some((current, run))
                    if current.typeface().unique_id() == font.typeface().unique_id() =>
                {
                    *run = &text[start..end];
                }
                _ => {
                    start = at;
                    segments.push((font, &text[at..end]));
                }
            }
        }
        segments
    }

    /// Draws `text` from `x`, each segment in its face, on the first
    /// segment's baseline.
    fn draw_segments(&self, text: &str, x: f64, y: f64, paint: &Paint) {
        let segments = self.segments(text);
        let Some((first, _)) = segments.first() else {
            return;
        };
        let baseline_y = self.baseline_y(first, y);
        let mut pen = x as f32;
        for (font, run) in &segments {
            self.draw_text(run, font, pen, baseline_y, paint);
            pen += font.measure_str(run, Some(paint)).0;
        }
    }

    fn font(&self) -> Font {
        let style = self.shared.style.borrow();
        match self.shared.fonts.borrow().get(style.font_index) {
            Some(entry) => Font::from_typeface(entry.typeface.clone(), style.font_size_px),
            None => Font::default(),
        }
    }

    /// Resolves a CSS font shorthand to a registered face and a px size.
    ///
    /// Only the parts jade emits are handled: an optional `bold`/`italic`
    /// prefix, a `<n>pt` size, and a comma-separated family list. The first
    /// family with a registered face wins, which mirrors how the measurement
    /// backend walks the stack.
    fn resolve_font(&self, descriptor: &str) -> (usize, f32) {
        let lower = descriptor.to_ascii_lowercase();
        let weight: i32 = if lower.contains("bold") { 700 } else { 400 };

        let mut size_px = 16.0_f32;
        let mut families = String::new();
        for (index, token) in descriptor.split_whitespace().enumerate() {
            if let Some(value) = token.strip_suffix("pt") {
                if let Ok(points) = value.parse::<f32>() {
                    size_px = points * 96.0 / 72.0;
                    families = descriptor
                        .split_whitespace()
                        .skip(index + 1)
                        .collect::<Vec<_>>()
                        .join(" ");
                    break;
                }
            }
        }

        for family in families.split(',') {
            let name = family.trim().trim_matches('\'').trim_matches('"');
            if name.is_empty() {
                continue;
            }
            let fonts = self.shared.fonts.borrow();
            let best = fonts
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.family.eq_ignore_ascii_case(name))
                .min_by_key(|(_, entry)| (entry.weight as i32 - weight).abs());
            if let Some((index, _)) = best {
                return (index, size_px);
            }
        }
        (0, size_px)
    }
}

/// Parses the CSS colour strings jade emits: `#rgb`, `#rrggbb`, `#rrggbbaa`,
/// and `rgb()` / `rgba()`. Anything unrecognised paints opaque black rather
/// than failing, matching how a browser treats an invalid style assignment as
/// a no-op rather than an error.
fn parse_css_color(value: &str) -> Color4f {
    let text = value.trim();
    if let Some(hex) = text.strip_prefix('#') {
        let expand = |c: u8| -> f32 {
            let digit = (c as char).to_digit(16).unwrap_or(0) as f32;
            digit / 15.0
        };
        let bytes = hex.as_bytes();
        let channel = |pair: &[u8]| -> f32 {
            u8::from_str_radix(std::str::from_utf8(pair).unwrap_or("0"), 16).unwrap_or(0) as f32
                / 255.0
        };
        return match bytes.len() {
            3 => Color4f::new(expand(bytes[0]), expand(bytes[1]), expand(bytes[2]), 1.0),
            6 => Color4f::new(
                channel(&bytes[0..2]),
                channel(&bytes[2..4]),
                channel(&bytes[4..6]),
                1.0,
            ),
            8 => Color4f::new(
                channel(&bytes[0..2]),
                channel(&bytes[2..4]),
                channel(&bytes[4..6]),
                channel(&bytes[6..8]),
            ),
            _ => Color4f::new(0.0, 0.0, 0.0, 1.0),
        };
    }
    if let Some(inner) = text
        .strip_prefix("rgba(")
        .or_else(|| text.strip_prefix("rgb("))
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let parts: Vec<f32> = inner
            .split(',')
            .filter_map(|part| part.trim().parse::<f32>().ok())
            .collect();
        if parts.len() >= 3 {
            return Color4f::new(
                parts[0] / 255.0,
                parts[1] / 255.0,
                parts[2] / 255.0,
                parts.get(3).copied().unwrap_or(1.0),
            );
        }
    }
    Color4f::new(0.0, 0.0, 0.0, 1.0)
}

/// Splits a device-space extent into the partially covered end pixels and
/// fully covered interior, preserving the same coverage as an AA rectangle.
fn pdf_rect_axis_spans(start: f32, end: f32) -> Vec<(f32, f32, f32)> {
    let first = start.floor();
    let last = end.ceil();
    if last - first <= 1.0 {
        return vec![(first, last, end - start)];
    }
    let mut spans = Vec::with_capacity(3);
    let inside_start = start.ceil();
    let inside_end = end.floor();
    if inside_start > start {
        spans.push((first, inside_start, inside_start - start));
    }
    if inside_end > inside_start {
        spans.push((inside_start, inside_end, 1.0));
    }
    if end > inside_end {
        spans.push((inside_end, last, end - inside_end));
    }
    spans
}

impl Painter for SkiaPainter {
    fn draw_image(
        &self,
        image: jade_painter::ImageSource<'_>,
        source: Option<[f64; 4]>,
        destination: jade_painter::ImageRect,
    ) -> Result<bool, Self::Error> {
        let cached = self.shared.images.borrow().get(image.key).cloned();
        let decoded = cached.or_else(|| {
            let decoded = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(image.bytes))?;
            self.shared
                .images
                .borrow_mut()
                .insert(image.key.to_string(), decoded.clone());
            Some(decoded)
        });
        Ok(if let Some(decoded) = decoded {
            self.draw_decoded_image(
                &decoded,
                source,
                destination.x,
                destination.y,
                destination.width,
                destination.height,
            )
        } else {
            self.draw_svg_image_rect(
                image.bytes,
                source,
                destination.x,
                destination.y,
                destination.width,
                destination.height,
            )
        })
    }

    type Error = Infallible;

    fn save(&self) {
        self.shared
            .stack
            .borrow_mut()
            .push(self.shared.style.borrow().clone());
        self.with_canvas(|canvas| {
            canvas.save();
        });
    }

    fn restore(&self) {
        if let Some(style) = self.shared.stack.borrow_mut().pop() {
            *self.shared.style.borrow_mut() = style;
        }
        self.with_canvas(|canvas| {
            canvas.restore();
        });
    }

    fn translate(&self, x: f64, y: f64) -> Result<(), Self::Error> {
        self.with_canvas(|canvas| {
            canvas.translate((x as f32, y as f32));
        });
        Ok(())
    }

    fn scale(&self, x: f64, y: f64) -> Result<(), Self::Error> {
        self.with_canvas(|canvas| {
            canvas.scale((x as f32, y as f32));
        });
        Ok(())
    }

    fn rotate(&self, angle: f64) -> Result<(), Self::Error> {
        // Canvas rotates in radians, Skia in degrees.
        self.with_canvas(|canvas| {
            canvas.rotate(angle.to_degrees() as f32, None);
        });
        Ok(())
    }

    fn set_fill_style_str(&self, value: &str) {
        self.shared.style.borrow_mut().fill = parse_css_color(value);
    }

    fn set_stroke_style_str(&self, value: &str) {
        self.shared.style.borrow_mut().stroke = parse_css_color(value);
    }

    fn set_line_width(&self, width: f64) {
        self.shared.style.borrow_mut().line_width = width as f32;
    }

    fn set_global_alpha(&self, alpha: f64) {
        self.shared.style.borrow_mut().global_alpha = alpha as f32;
    }

    fn set_line_dash(&self, segments: &[f64]) {
        self.shared.style.borrow_mut().dash = segments.iter().map(|value| *value as f32).collect();
    }

    fn set_shadow_color(&self, value: &str) {
        self.shared.style.borrow_mut().shadow_color = parse_css_color(value);
    }

    fn set_shadow_blur(&self, value: f64) {
        self.shared.style.borrow_mut().shadow_blur = value as f32;
    }

    fn set_shadow_offset_x(&self, value: f64) {
        self.shared.style.borrow_mut().shadow_offset.0 = value as f32;
    }

    fn set_shadow_offset_y(&self, value: f64) {
        self.shared.style.borrow_mut().shadow_offset.1 = value as f32;
    }

    fn set_blur_filter(&self, radius_px: f64) {
        self.shared.style.borrow_mut().blur_filter = radius_px as f32;
    }

    fn set_font(&self, value: &str) {
        let (font_index, font_size_px) = self.resolve_font(value);
        let mut style = self.shared.style.borrow_mut();
        style.font_index = font_index;
        style.font_size_px = font_size_px;
        style.font_descriptor = Rc::from(value);
    }

    fn set_text_baseline(&self, value: &str) {
        self.shared.style.borrow_mut().text_baseline = TextBaseline::parse(value);
    }

    fn fill_text(&self, text: &str, x: f64, y: f64) -> Result<(), Self::Error> {
        self.shared
            .fill_text_calls
            .set(self.shared.fill_text_calls.get() + 1);
        let paint = self.fill_paint();
        self.draw_segments(text, x, y, &paint);
        Ok(())
    }

    fn stroke_text(&self, text: &str, x: f64, y: f64) -> Result<(), Self::Error> {
        let paint = self.stroke_paint();
        self.draw_segments(text, x, y, &paint);
        Ok(())
    }

    fn measure_text_width(&self, text: &str) -> Result<f64, Self::Error> {
        let paint = self.fill_paint();
        Ok(self
            .segments(text)
            .iter()
            .map(|(font, run)| font.measure_str(run, Some(&paint)).0 as f64)
            .sum())
    }

    /// Mirrors what Chromium's canvas reports.
    ///
    /// The font bounding box comes from the face's `hhea` ascender and
    /// descender with **each half rounded to a whole pixel independently** —
    /// rounding the sum gives a different answer, and Chromium rounds the
    /// halves. The actual bounding box is the ink extent of this particular
    /// string, which Skia returns alongside the advance.
    fn measure_text_metrics(&self, text: &str) -> Result<TextMetrics, Self::Error> {
        let font = self.font();
        let paint = self.fill_paint();
        // Each segment in its face, its ink box at its pen position.
        let (mut width, mut top, mut bottom, mut right) = (0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32);
        for (segment_font, run) in self.segments(text) {
            let (advance, ink) = segment_font.measure_str(run, Some(&paint));
            top = top.min(ink.top);
            bottom = bottom.max(ink.bottom);
            right = right.max(width + ink.right);
            width += advance;
        }
        let (_, metrics) = font.metrics();
        Ok(TextMetrics {
            width: width as f64,
            // Skia reports the ascent as a negative offset from the baseline.
            font_bounding_box_ascent: (-metrics.ascent).round() as f64,
            font_bounding_box_descent: metrics.descent.round() as f64,
            actual_bounding_box_ascent: (-top) as f64,
            actual_bounding_box_descent: bottom as f64,
            actual_bounding_box_right: right as f64,
        })
    }

    fn fill_rect(&self, x: f64, y: f64, width: f64, height: f64) {
        let paint = self.fill_paint();
        let recording = matches!(
            *self.shared.surface.borrow(),
            PaintSurface::Recording {
                raster_scale: None,
                ..
            }
        );
        self.with_canvas(|canvas| {
            let rect = Rect::from_xywh(x as f32, y as f32, width as f32, height as f32);
            let matrix = canvas.local_to_device_as_3x3();
            if recording
                && matrix.rect_stays_rect()
                && !rect.is_empty()
                && paint.image_filter().is_none()
                && paint.mask_filter().is_none()
            {
                // PDF readers can snap opaque fractional rectangles to whole
                // pixels, erasing a table border beneath a paragraph background.
                // Encode the live Skia painter's pixel coverage at each edge as
                // opacity on integer device rectangles. Interiors remain vector
                // fills; text, paths and transformed/effect drawing are untouched.
                let device = matrix.map_rect(rect).0;
                let xs = pdf_rect_axis_spans(device.left, device.right);
                let ys = pdf_rect_axis_spans(device.top, device.bottom);
                canvas.save();
                canvas.reset_matrix();
                for (left, right, x_coverage) in &xs {
                    for (top, bottom, y_coverage) in &ys {
                        let mut covered = paint.clone();
                        covered.set_anti_alias(false);
                        covered.set_alpha_f(paint.alpha_f() * x_coverage * y_coverage);
                        canvas.draw_rect(Rect::new(*left, *top, *right, *bottom), &covered);
                    }
                }
                canvas.restore();
            } else {
                canvas.draw_rect(rect, &paint);
            }
        });
    }

    fn stroke_rect(&self, x: f64, y: f64, width: f64, height: f64) {
        // Deliberately not the current path: canvas `strokeRect` leaves any
        // path being built untouched.
        let paint = self.stroke_paint();
        self.with_canvas(|canvas| {
            canvas.draw_rect(
                Rect::from_xywh(x as f32, y as f32, width as f32, height as f32),
                &paint,
            );
        });
    }

    fn fill_horizontal_gradient_rect(
        &self,
        rect: jade_painter::ImageRect,
        start: [u8; 4],
        end: [u8; 4],
    ) -> Result<(), Self::Error> {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return Ok(());
        }
        let colors = [start, end].map(|color| {
            let [r, g, b] = if self.shared.dark_paper.get() {
                jade_painter::paper::to_dark_paper_u8([color[0], color[1], color[2]])
            } else {
                [color[0], color[1], color[2]]
            };
            skia_safe::Color::from_argb(color[3], r, g, b)
        });
        if let Some(shader) = skia_safe::Shader::linear_gradient(
            (
                (rect.x as f32, rect.y as f32),
                ((rect.x + rect.width) as f32, rect.y as f32),
            ),
            colors.as_slice(),
            None,
            skia_safe::TileMode::Clamp,
            None,
            None,
        ) {
            let mut paint = self.fill_paint();
            paint.set_shader(shader);
            paint.set_alpha_f(self.shared.style.borrow().global_alpha);
            self.with_canvas(|canvas| {
                canvas.draw_rect(
                    Rect::from_xywh(
                        rect.x as f32,
                        rect.y as f32,
                        rect.width as f32,
                        rect.height as f32,
                    ),
                    &paint,
                );
            });
        }
        Ok(())
    }

    fn clear_rect(&self, x: f64, y: f64, width: f64, height: f64) {
        let mut paint = Paint::new(self.paper(), None);
        paint.set_blend_mode(skia_safe::BlendMode::Src);
        self.with_canvas(|canvas| {
            canvas.draw_rect(
                Rect::from_xywh(x as f32, y as f32, width as f32, height as f32),
                &paint,
            );
        });
    }

    fn begin_path(&self) {
        *self.shared.path.borrow_mut() = Path::new();
    }

    fn close_path(&self) {
        self.shared.path.borrow_mut().close();
    }

    fn move_to(&self, x: f64, y: f64) {
        self.shared.path.borrow_mut().move_to((x as f32, y as f32));
    }

    fn line_to(&self, x: f64, y: f64) {
        let mut path = self.shared.path.borrow_mut();
        // Canvas treats `lineTo` with no current point as `moveTo`.
        if path.is_empty() {
            path.move_to((x as f32, y as f32));
        } else {
            path.line_to((x as f32, y as f32));
        }
    }

    fn rect(&self, x: f64, y: f64, width: f64, height: f64) {
        self.shared.path.borrow_mut().add_rect(
            Rect::from_xywh(x as f32, y as f32, width as f32, height as f32),
            None,
        );
    }

    fn arc(
        &self,
        x: f64,
        y: f64,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
    ) -> Result<(), Self::Error> {
        self.ellipse(x, y, radius, radius, 0.0, start_angle, end_angle)
    }

    fn quadratic_curve_to(&self, cp_x: f64, cp_y: f64, x: f64, y: f64) {
        self.shared
            .path
            .borrow_mut()
            .quad_to((cp_x as f32, cp_y as f32), (x as f32, y as f32));
    }

    fn bezier_curve_to(&self, cp1_x: f64, cp1_y: f64, cp2_x: f64, cp2_y: f64, x: f64, y: f64) {
        self.shared.path.borrow_mut().cubic_to(
            (cp1_x as f32, cp1_y as f32),
            (cp2_x as f32, cp2_y as f32),
            (x as f32, y as f32),
        );
    }

    fn ellipse(
        &self,
        x: f64,
        y: f64,
        radius_x: f64,
        radius_y: f64,
        rotation: f64,
        start_angle: f64,
        end_angle: f64,
    ) -> Result<(), Self::Error> {
        self.ellipse_with_anticlockwise(
            x,
            y,
            radius_x,
            radius_y,
            rotation,
            start_angle,
            end_angle,
            false,
        )
    }

    fn ellipse_with_anticlockwise(
        &self,
        x: f64,
        y: f64,
        radius_x: f64,
        radius_y: f64,
        rotation: f64,
        start_angle: f64,
        end_angle: f64,
        anticlockwise: bool,
    ) -> Result<(), Self::Error> {
        let oval = Rect::from_xywh(
            (x - radius_x) as f32,
            (y - radius_y) as f32,
            (radius_x * 2.0) as f32,
            (radius_y * 2.0) as f32,
        );
        let start = start_angle.to_degrees();
        let sweep = (end_angle - start_angle).to_degrees();
        let sweep = if !anticlockwise && sweep >= 360.0 {
            360.0
        } else if anticlockwise && sweep <= -360.0 {
            -360.0
        } else if anticlockwise {
            -(-sweep).rem_euclid(360.0)
        } else {
            sweep.rem_euclid(360.0)
        };
        // Skia's `arcTo` treats the sweep modulo 360, so a full turn would
        // collapse to a single point; split it into two half turns instead.
        let add_arc = |path: &mut Path| {
            if sweep.abs() >= 360.0 {
                let half = sweep / 2.0;
                path.arc_to(oval, start as f32, half as f32, false);
                path.arc_to(oval, (start + half) as f32, half as f32, false);
            } else {
                path.arc_to(oval, start as f32, sweep as f32, false);
            }
        };
        let mut path = self.shared.path.borrow_mut();
        if rotation == 0.0 {
            add_arc(&mut path);
        } else {
            // Build the unrotated arc, then rotate it about the centre.
            // `Extend` joins it to the current point with a line, as canvas does.
            let mut arc = Path::new();
            add_arc(&mut arc);
            let matrix =
                Matrix::rotate_deg_pivot(rotation.to_degrees() as f32, (x as f32, y as f32));
            path.add_path_matrix(&arc, &matrix, AddPathMode::Extend);
        }
        Ok(())
    }

    fn fill(&self) {
        let paint = self.fill_paint();
        let path = self.shared.path.borrow().clone();
        self.with_canvas(|canvas| {
            canvas.draw_path(&path, &paint);
        });
    }

    fn stroke(&self) {
        let paint = self.stroke_paint();
        let path = self.shared.path.borrow().clone();
        self.with_canvas(|canvas| {
            canvas.draw_path(&path, &paint);
        });
    }

    fn clip(&self) {
        let path = self.shared.path.borrow().clone();
        self.with_canvas(|canvas| {
            canvas.clip_path(&path, None, true);
        });
    }

    fn use_fill_style_for_stroke(&self) {
        let fill = self.shared.style.borrow().fill;
        self.shared.style.borrow_mut().stroke = fill;
    }
}
