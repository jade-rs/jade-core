//! Word's page layers for floating objects (`wp:anchor`).
//!
//! Word paints a page bottom to top as: header and footer, objects behind
//! text (`behindDoc="1"`), the body text, then every other floating object —
//! within each object layer in `relativeHeight` order, whichever paragraph
//! anchors the object. The renderer reaches an object while painting its
//! anchor paragraph, so painting it there lets later paragraphs on the page
//! cover an object in front of text, and an object behind text cover the text
//! of earlier paragraphs.
//!
//! The walk stays as it is. While an object is painted its drawing is
//! captured ([`jade_painter::capture`]) instead of reaching the page, and so
//! is the body itself on a page of a document that has objects behind text.
//! When the page is finished — after its footnotes and footer — the held
//! drawing is painted in layer order: objects behind text, the body, objects
//! in front. Paragraph render caches are unaffected: a cached paragraph's
//! replay is drawing like any other and lands in the body capture.
//!
//! Hosts opt in with
//! [`CanvasRenderHost::supports_deferred_painting`](crate::render_host::CanvasRenderHost::supports_deferred_painting);
//! others keep painting objects with their anchors.

use super::*;
use jade_painter::capture::{self, CaptureSink, PaintOp, RoutedPainter};
use std::rc::Weak;

/// A weak key does not keep an old document alive or add a strong owner that
/// would force a deep clone on the next edit. `Rc::make_mut` changes this
/// identity when only weak references remain, so an edited document cannot
/// reuse the old answer.
#[derive(Default)]
struct BehindTextObjectsCache {
    document: Weak<Document>,
    value: bool,
    #[cfg(test)]
    scans: usize,
}

impl BehindTextObjectsCache {
    fn get(&mut self, document: &Rc<Document>) -> bool {
        let identity = Rc::downgrade(document);
        if !self.document.ptr_eq(&identity) {
            self.value = document
                .body
                .as_ref()
                .is_some_and(|body| elements_have_behind_text_objects(&body.elements));
            self.document = identity;
            #[cfg(test)]
            {
                self.scans += 1;
            }
        }
        self.value
    }
}

/// A host image draw made while capturing, painted again at replay.
struct HeldHostImage {
    data_uri: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    crop: Option<ImageCropMapping>,
}

/// A floating object's captured drawing.
struct HeldObject {
    behind_text: bool,
    relative_height: u32,
    sequence: usize,
    ops: Vec<PaintOp>,
}

/// What the current page holds back until it is finished.
#[derive(Default)]
pub(super) struct PageLayers {
    /// Active captures, innermost last; drawing goes to the last one.
    stack: RefCell<Vec<CaptureSink>>,
    /// Host image draws referenced by [`PaintOp::Marker`].
    images: RefCell<Vec<HeldHostImage>>,
    /// The body's drawing, once the page started holding it.
    body: RefCell<Option<CaptureSink>>,
    /// Whether the body capture is still receiving drawing.
    body_open: Cell<bool>,
    objects: RefCell<Vec<HeldObject>>,
    next_sequence: Cell<usize>,
    /// Whether the document has an object behind text in its body.
    holds_body: Cell<bool>,
    behind_text_cache: BehindTextObjectsCache,
}

impl PageLayers {
    fn active_sink(&self) -> Option<CaptureSink> {
        self.stack.borrow().last().cloned()
    }

    fn is_empty(&self) -> bool {
        self.body.borrow().is_none() && self.objects.borrow().is_empty()
    }

    fn reset(&self) {
        self.stack.borrow_mut().clear();
        self.images.borrow_mut().clear();
        *self.body.borrow_mut() = None;
        self.body_open.set(false);
        self.objects.borrow_mut().clear();
    }
}

/// An object capture begun by [`CanvasRenderer::begin_float_layer`].
pub(super) struct FloatLayerToken {
    sink: CaptureSink,
    behind_text: bool,
    relative_height: u32,
}

impl<P: Painter> RenderHostSlot<P>
where
    P::Error: PainterError,
{
    pub(super) fn new(value: Option<BoxedRenderHost<P>>) -> Self {
        Self {
            value,
            layers: PageLayers::default(),
        }
    }

    /// The current page's painter, recording while a capture is active.
    ///
    /// Shadows the host's own `painter` so every draw the renderer makes goes
    /// through the page layers.
    pub(super) fn painter(&self) -> RoutedPainter<P> {
        RoutedPainter::new((**self).painter(), self.layers.active_sink())
    }

    /// The host's `set_font`, also recorded while capturing: the renderer
    /// tracks the font as applied to the paint surface too.
    pub(super) fn set_font(&self, font_descriptor: &str) {
        if let Some(sink) = self.layers.active_sink() {
            sink.borrow_mut()
                .push(PaintOp::SetFont(Rc::from(font_descriptor)));
        }
        (**self).set_font(font_descriptor);
    }

    /// The host's image draw, held as a marker while capturing.
    pub(super) fn draw_image_data_uri(
        &self,
        data_uri: &str,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        crop: Option<ImageCropMapping>,
    ) -> bool {
        let Some(sink) = self.layers.active_sink() else {
            return (**self).draw_image_data_uri(data_uri, x, y, width, height, crop);
        };
        let mut images = self.layers.images.borrow_mut();
        sink.borrow_mut().push(PaintOp::Marker(images.len()));
        images.push(HeldHostImage {
            data_uri: data_uri.to_string(),
            x,
            y,
            width,
            height,
            crop,
        });
        true
    }

    /// Paints `ops` onto the page itself.
    fn replay_onto_page(&self, ops: &[PaintOp]) -> Result<(), P::Error> {
        let page = (**self).painter();
        page.save();
        let images = self.layers.images.borrow();
        let result = capture::replay(ops, &page, |id| {
            if let Some(image) = images.get(id) {
                (**self).draw_image_data_uri(
                    &image.data_uri,
                    image.x,
                    image.y,
                    image.width,
                    image.height,
                    image.crop,
                );
            }
        });
        page.restore();
        result
    }
}

/// True when any body paragraph, at any table depth, anchors an object
/// behind text.
fn elements_have_behind_text_objects(elements: &[BodyElement]) -> bool {
    elements.iter().any(|element| match element {
        BodyElement::Paragraph(paragraph) => paragraph.runs.iter().any(|run| {
            let behind = |layout: &RunImageLayout| {
                layout.kind == RunImageKind::Floating && layout.behind_doc
            };
            run.image
                .as_ref()
                .is_some_and(|image| behind(&image.layout))
                || run
                    .text_box
                    .as_ref()
                    .is_some_and(|text_box| behind(&text_box.layout))
        }),
        BodyElement::Table(table) => table.rows.iter().any(|row| {
            row.cells
                .iter()
                .any(|cell| elements_have_behind_text_objects(&cell.blocks))
        }),
        BodyElement::Math(_) => false,
    })
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    fn document(behind: bool) -> Rc<Document> {
        let mut run: Run = serde_json::from_value(serde_json::json!({ "text": "text" })).unwrap();
        run.image = Some(RunImage {
            layout: RunImageLayout {
                kind: RunImageKind::Floating,
                behind_doc: behind,
                ..Default::default()
            },
            ..Default::default()
        });
        Rc::new(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![run],
                    properties: None,
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            ..Default::default()
        })
    }

    #[test]
    fn page_layer_detection_scans_each_immutable_document_only_once() {
        let mut cache = BehindTextObjectsCache::default();
        for behind in [false, true] {
            let document = document(behind);
            let before = cache.scans;
            for _ in 0..100 {
                assert_eq!(cache.get(&document), behind);
            }
            assert_eq!(cache.scans - before, 1);
            assert_eq!(
                Rc::strong_count(&document),
                1,
                "cache must not force a document clone"
            );
        }
    }

    #[test]
    fn page_layer_detection_follows_in_place_edits_and_undo_snapshots() {
        let mut document = document(false);
        let mut cache = BehindTextObjectsCache::default();
        assert!(!cache.get(&document));
        // With only a weak cache key, make_mut moves rather than cloning the
        // document, but it still invalidates the cache's identity.
        let body = Rc::make_mut(&mut document).body.as_mut().unwrap();
        let BodyElement::Paragraph(paragraph) = &mut body.elements[0] else {
            unreachable!()
        };
        paragraph.runs[0].image.as_mut().unwrap().layout.behind_doc = true;
        assert!(cache.get(&document));
        let undo = Rc::clone(&document);
        Rc::make_mut(&mut document)
            .body
            .as_mut()
            .unwrap()
            .elements
            .clear();
        assert!(!cache.get(&document));
        assert!(cache.get(&undo));
        assert_eq!(cache.scans, 4);
    }

    #[test]
    fn page_layer_detection_keeps_nested_table_objects() {
        let source = document(true);
        let paragraph = source.body.as_ref().unwrap().elements[0].clone();
        let mut nested = paragraph;
        for _ in 0..3 {
            nested = BodyElement::Table(Box::new(Table {
                rows: vec![TableRow {
                    cells: vec![TableCell {
                        blocks: vec![nested],
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }));
        }
        let nested = Rc::new(Document {
            body: Some(Body {
                elements: vec![nested],
            }),
            ..Default::default()
        });
        let mut cache = BehindTextObjectsCache::default();
        assert!(cache.get(&nested));
        assert!(!cache.get(&Rc::new(Document::default())));
        assert!(cache.get(&nested));
    }

    #[test]
    fn clearing_page_layer_cache_preserves_the_next_edit_allocation() {
        let mut document = document(false);
        let mut cache = BehindTextObjectsCache::default();
        assert!(!cache.get(&document));
        let pointer = Rc::as_ptr(&document);
        // Mirrors drop_document_cache before document_mut. Without releasing
        // the weak key, make_mut moves the document and invalidates unrelated
        // pointer-keyed renderer caches on every keystroke.
        cache = BehindTextObjectsCache::default();
        assert_eq!(Rc::weak_count(&document), 0);
        Rc::make_mut(&mut document).body = None;
        assert_eq!(Rc::as_ptr(&document), pointer);
        assert!(!cache.get(&document));
    }
}

impl<HostPainter> CanvasRenderer<HostPainter>
where
    HostPainter: Painter,
    HostPainter::Error: PainterError,
{
    /// Starts a render pass: nothing is held, and the body is held on pages
    /// of `document` only when it has objects behind text.
    pub(super) fn reset_page_layers(&mut self, document: &Rc<Document>) {
        self.host.layers.reset();
        // Layout-only passes and hosts without page captures never consume the
        // answer. Bitmap refreshes of the same immutable document share it.
        let holds_body = !self.render_skip_draw
            && !self.pageless_mode
            && self.host.supports_deferred_painting()
            && self.host.layers.behind_text_cache.get(document);
        self.host.layers.holds_body.set(holds_body);
    }

    pub(super) fn invalidate_page_layers_document(&mut self) {
        // Release the weak key before an edit as well: make_mut can then keep
        // the same allocation, preserving other document-identity caches.
        self.host.layers.behind_text_cache = BehindTextObjectsCache::default();
    }

    /// Whether drawing on the current page may be held back.
    fn page_layers_available(&self) -> bool {
        !self.pageless_mode
            && !self.rendering_page_furniture
            && !self.measuring_keep_next
            // A band redraw repaints one paragraph inside a clip and keeps the
            // rest of the page, so there is no page to put back together.
            && self.band_redraw_plan.is_none()
            && self.host.supports_deferred_painting()
            && self.should_draw_current_page_for_render()
    }

    /// Called once the header is on the page: on a page that may get objects
    /// behind text, hold the body so they can go under it.
    pub(super) fn begin_page_body_layer(&mut self) {
        if !self.host.layers.holds_body.get()
            || self.host.layers.body.borrow().is_some()
            || !self.page_layers_available()
        {
            return;
        }
        let sink = capture::new_sink();
        self.host.layers.stack.borrow_mut().push(sink.clone());
        *self.host.layers.body.borrow_mut() = Some(sink);
        self.host.layers.body_open.set(true);
        self.invalidate_context_state_tracking();
    }

    /// Called before the footer: the body is complete.
    pub(super) fn end_page_body_layer(&mut self) {
        if !self.host.layers.body_open.replace(false) {
            return;
        }
        let body = self.host.layers.body.borrow().clone();
        let mut stack = self.host.layers.stack.borrow_mut();
        if let Some(position) =
            body.and_then(|body| stack.iter().rposition(|sink| Rc::ptr_eq(sink, &body)))
        {
            stack.truncate(position);
        }
        drop(stack);
        self.invalidate_context_state_tracking();
    }

    /// Starts holding one floating object's drawing, or returns `None` to
    /// paint it in place.
    ///
    /// An object behind text is held only while the body is: painted after
    /// the page's text it would cover it.
    pub(super) fn begin_float_layer(&mut self, layout: &RunImageLayout) -> Option<FloatLayerToken> {
        if !self.page_layers_available() {
            return None;
        }
        let behind_text = layout.behind_doc;
        if behind_text && !self.host.layers.body_open.get() {
            return None;
        }
        let sink = capture::new_sink();
        self.host.layers.stack.borrow_mut().push(sink.clone());
        self.invalidate_context_state_tracking();
        Some(FloatLayerToken {
            sink,
            behind_text,
            relative_height: layout
                .relative_height
                .unwrap_or(DEFAULT_FLOAT_RELATIVE_HEIGHT),
        })
    }

    /// Stops holding the object begun with `token`.
    pub(super) fn end_float_layer(&mut self, token: Option<FloatLayerToken>) {
        let Some(token) = token else {
            return;
        };
        {
            let mut stack = self.host.layers.stack.borrow_mut();
            if let Some(position) = stack.iter().rposition(|sink| Rc::ptr_eq(sink, &token.sink)) {
                stack.truncate(position);
            }
        }
        let ops = std::mem::take(&mut *token.sink.borrow_mut());
        if !ops.is_empty() {
            let sequence = self.host.layers.next_sequence.get();
            self.host.layers.next_sequence.set(sequence + 1);
            self.host.layers.objects.borrow_mut().push(HeldObject {
                behind_text: token.behind_text,
                relative_height: token.relative_height,
                sequence,
                ops,
            });
        }
        self.invalidate_context_state_tracking();
    }

    /// Paints what the current page held, in Word's layer order, onto the
    /// page: objects behind text, the body, objects in front of text.
    pub(super) fn composite_page_layers(&mut self) -> Result<(), JsValue> {
        self.end_page_body_layer();
        let layers = &self.host.layers;
        if layers.is_empty() {
            layers.reset();
            return Ok(());
        }
        layers.stack.borrow_mut().clear();
        let mut objects = std::mem::take(&mut *layers.objects.borrow_mut());
        objects.sort_by_key(|object| (object.relative_height, object.sequence));
        let body = layers.body.borrow_mut().take();
        let mut result = Ok(());
        for object in objects.iter().filter(|object| object.behind_text) {
            result = result.and(self.host.replay_onto_page(&object.ops));
        }
        if let Some(body) = body {
            result = result.and(self.host.replay_onto_page(&body.borrow()));
        }
        for object in objects.iter().filter(|object| !object.behind_text) {
            result = result.and(self.host.replay_onto_page(&object.ops));
        }
        self.host.layers.reset();
        self.invalidate_context_state_tracking();
        result.map_err(PainterError::into_js_value)
    }
}

#[cfg(test)]
mod tests {
    use super::super::recording_host::recording_renderer;
    use super::*;
    use jade_painter::recording::Op;

    fn run(text: &str) -> Run {
        Run {
            text: text.into(),
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
        }
    }

    fn paragraph(text: &str) -> Paragraph {
        Paragraph {
            runs: vec![run(text)],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        }
    }

    /// A filled rectangle shape over the top of the page, in front of or
    /// behind the text.
    fn shape_paragraph(
        text: &str,
        fill: &str,
        behind_doc: bool,
        relative_height: u32,
    ) -> Paragraph {
        let mut paragraph = paragraph(text);
        paragraph.runs.push(Run {
            text: String::new(),
            text_box: Some(RunTextBox {
                width_px: Some(300.0),
                height_px: Some(200.0),
                fill_color: Some(fill.into()),
                layout: RunImageLayout {
                    kind: RunImageKind::Floating,
                    wrap: WrapMode::None,
                    behind_doc,
                    relative_height: Some(relative_height),
                    position_h: Some(ImagePosition {
                        relative_from: Some("page".into()),
                        offset_px: Some(50.0),
                        align: None,
                    }),
                    position_v: Some(ImagePosition {
                        relative_from: Some("page".into()),
                        offset_px: Some(50.0),
                        align: None,
                    }),
                    ..RunImageLayout::default()
                },
                ..RunTextBox::default()
            }),
            ..run("")
        });
        paragraph
    }

    fn document(paragraphs: Vec<Paragraph>) -> Document {
        Document {
            body: Some(Body {
                elements: paragraphs
                    .into_iter()
                    .map(BodyElement::from_paragraph)
                    .collect(),
            }),
            ..Document::default()
        }
    }

    /// Position in the op log of the first fill in `color` and of the first
    /// glyph of `text`.
    fn order(ops: &[Op], color: &str, text: &str) -> (usize, usize) {
        let fill = ops
            .iter()
            .position(
                |op| matches!(op, Op::SetFillStyle(value) if value.eq_ignore_ascii_case(color)),
            )
            .unwrap_or_else(|| panic!("no {color} fill in {ops:?}"));
        let glyph = ops
            .iter()
            .position(
                |op| matches!(op, Op::FillText(value, _, _) if text.starts_with(value.as_str())),
            )
            .unwrap_or_else(|| panic!("no {text} glyph"));
        (fill, glyph)
    }

    fn render(document: &Document) -> Vec<Op> {
        let mut renderer = recording_renderer();
        renderer.render(document, &Options::default()).unwrap();
        let painter = renderer.host.painter();
        painter.inner().ops()
    }

    #[test]
    fn an_object_in_front_of_text_covers_later_paragraphs() {
        let ops = render(&document(vec![
            shape_paragraph("Anchor", "#ff0000", false, 10),
            paragraph("Later"),
        ]));
        let (fill, later) = order(&ops, "#ff0000", "Later");
        assert!(
            later < fill,
            "the shape is painted after the later paragraph"
        );
    }

    #[test]
    fn an_object_behind_text_goes_under_earlier_paragraphs() {
        let ops = render(&document(vec![
            paragraph("Earlier"),
            shape_paragraph("Anchor", "#00ff00", true, 10),
        ]));
        let (fill, earlier) = order(&ops, "#00ff00", "Earlier");
        assert!(
            fill < earlier,
            "the shape is painted before the earlier paragraph"
        );
    }

    #[test]
    fn objects_stack_by_relative_height_across_paragraphs() {
        let ops = render(&document(vec![
            shape_paragraph("One", "#0000ff", false, 300),
            shape_paragraph("Two", "#ffff00", false, 100),
        ]));
        let blue = order(&ops, "#0000ff", "One").0;
        let yellow = order(&ops, "#ffff00", "Two").0;
        assert!(yellow < blue, "the lower relativeHeight is painted first");
    }
}
