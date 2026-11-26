//! What the browser shell does after a pass, on a paged recording host: a
//! footer's `PAGE of NUMPAGES` drawn before a lazily opened document was laid
//! out whole, and a header's STYLEREF drawn before the headings below it
//! moved, are drawn again and reported dirty ([`settle_drawn_pages`]); a
//! STYLEREF that moves its own heading stops after [`MAX_SETTLE_PAINTS`]; and
//! a pass that drew them right paints nothing. These are also the first
//! direct tests of [`extend_renderer_layout`] and
//! [`complete_renderer_layout`], what `ensureLayoutThroughPage` and
//! `complete_layout` run.

use super::*;
use jade_painter::recording::{Op, RecordingPainter};
use jade_painter::Painter as _;
use std::io::Write;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// A paged host recording each page's drawing apart, and measuring every
/// character as 10px so pagination does not depend on installed fonts.
struct PagedRecordingHost {
    pages: Rc<RefCell<Vec<RecordingPainter>>>,
    /// Pages laid out after the first.
    extra_pages: usize,
    cursor: usize,
    /// Pages (0-based) whose pixels were forgotten and not drawn since.
    forgotten: Rc<RefCell<std::collections::BTreeSet<usize>>>,
    /// While set, starting a page fails, as a lost canvas context does.
    failing: Rc<std::cell::Cell<bool>>,
}

impl PagedRecordingHost {
    fn page(&self, index: usize) -> RecordingPainter {
        let mut pages = self.pages.borrow_mut();
        while pages.len() <= index {
            pages.push(RecordingPainter::new());
        }
        pages[index].clone()
    }
}

impl crate::render_host::CanvasRenderHost for PagedRecordingHost {
    type Painter = RecordingPainter;

    fn painter(&self) -> Self::Painter {
        self.page(self.cursor)
    }
    fn device_pixel_ratio(&self) -> f64 {
        1.0
    }
    fn update_device_pixel_ratio(&mut self) -> Result<bool, JsValue> {
        Ok(false)
    }
    fn begin_render(&mut self, reuse_pages: bool) -> Result<(), JsValue> {
        if reuse_pages {
            self.cursor = 0;
            Ok(())
        } else {
            self.reset_pages()
        }
    }
    fn reset_pages(&mut self) -> Result<(), JsValue> {
        self.extra_pages = 0;
        self.cursor = 0;
        Ok(())
    }
    fn configure_base_canvas(
        &mut self,
        _width: f64,
        _height: f64,
        _pageless: bool,
        _margin_top: f64,
    ) -> Result<(), JsValue> {
        Ok(())
    }
    fn clear(&mut self, _page_width: f64, _pageless: bool, _full: f64) -> Result<(), JsValue> {
        Ok(())
    }
    fn ensure_pageless_height(&mut self, _h: f64, _mb: f64) -> Result<(), JsValue> {
        Ok(())
    }
    fn start_new_page(&mut self, _w: f64, _h: f64, _mt: f64) -> Result<(), JsValue> {
        if self.failing.get() {
            return Err(JsValue::NULL);
        }
        self.cursor += 1;
        self.extra_pages = self.extra_pages.max(self.cursor);
        Ok(())
    }
    fn device_to_css(&self, value: f64) -> f64 {
        value
    }
    fn canvas_css_size(&self) -> (f64, f64) {
        (816.0, 1056.0)
    }
    fn page_count(&self) -> usize {
        1 + self.extra_pages
    }
    fn truncate_pages(&mut self, total_pages: usize) -> Result<(), JsValue> {
        self.extra_pages = self.extra_pages.min(total_pages.saturating_sub(1));
        Ok(())
    }
    fn page_dimensions(&self, page_index: usize) -> Option<(f64, f64)> {
        (page_index < self.page_count()).then_some((816.0, 1056.0))
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
    fn context(&self) -> &web_sys::CanvasRenderingContext2d {
        unreachable!("recording host: context() must not be called")
    }
    fn context_mut(&mut self) -> &mut web_sys::CanvasRenderingContext2d {
        unreachable!("recording host: context_mut() must not be called")
    }
    fn create_page_bitmap(&self, _page_index: usize) -> Result<js_sys::Promise, JsValue> {
        unreachable!("recording host: bitmap snapshots are browser-side")
    }
    fn draw_image_data_uri(
        &self,
        _data_uri: &str,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        _crop: Option<crate::render_host::ImageCropMapping>,
    ) -> bool {
        self.painter().fill_rect(x, y, width, height);
        true
    }
    fn measure_text_width(&self, text: &str) -> Result<f64, JsValue> {
        Ok(text.chars().count() as f64 * 10.0)
    }
    fn page_has_rendered_pixels(&self, page_index: usize) -> bool {
        !self.forgotten.borrow().contains(&page_index)
    }
    fn mark_page_rendered(&mut self, page_index: usize) {
        self.forgotten.borrow_mut().remove(&page_index);
    }
    fn forget_page_pixels(&mut self, page_index: usize) {
        self.forgotten.borrow_mut().insert(page_index);
    }
    fn measure_text_metrics(&self, text: &str) -> Result<jade_painter::TextMetrics, JsValue> {
        let width = text.chars().count() as f64 * 10.0;
        Ok(jade_painter::TextMetrics {
            width,
            font_bounding_box_ascent: 8.0,
            font_bounding_box_descent: 2.0,
            actual_bounding_box_ascent: 8.0,
            actual_bounding_box_descent: 2.0,
            actual_bounding_box_left: 0.0,
            actual_bounding_box_right: width,
        })
    }
}

/// The browser shell's renderer over a paged recording host, and the
/// host's page recordings.
struct Shell {
    renderer: CanvasRenderer<RecordingPainter>,
    pages: Rc<RefCell<Vec<RecordingPainter>>>,
    forgotten: Rc<RefCell<std::collections::BTreeSet<usize>>>,
    /// The view's kept pages ([`KeptPages`]).
    kept: KeptPages,
    failing: Rc<std::cell::Cell<bool>>,
    options: Options,
}

impl Shell {
    fn new() -> Self {
        let pages = Rc::new(RefCell::new(Vec::new()));
        let forgotten = Rc::new(RefCell::new(std::collections::BTreeSet::new()));
        let failing = Rc::new(std::cell::Cell::new(false));
        let renderer = CanvasRenderer::with_host(
            Box::new(PagedRecordingHost {
                pages: Rc::clone(&pages),
                extra_pages: 0,
                cursor: 0,
                forgotten: Rc::clone(&forgotten),
                failing: Rc::clone(&failing),
            }),
            false,
        )
        .expect("paged recording renderer");
        Self {
            renderer,
            pages,
            forgotten,
            kept: KeptPages::new(),
            failing,
            options: Options::default(),
        }
    }

    /// Forgets what every page drew so far.
    fn forget_drawing(&self) {
        for page in self.pages.borrow().iter() {
            page.clear();
        }
    }

    /// The text page `page` (0-based) drew since [`Self::forget_drawing`],
    /// formatting marks left out.
    fn page_text(&self, page: usize) -> String {
        self.pages
            .borrow()
            .get(page)
            .map(|painter| {
                painter
                    .ops()
                    .iter()
                    .filter_map(|op| match op {
                        Op::FillText(text, ..) => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<String>()
            })
            .unwrap_or_default()
            .replace(['\u{b7}', '\u{b6}', '\u{21b5}', '\u{21d3}'], " ")
    }

    /// What `loadDocxLazyUntil` does: lays out and draws `end_page` pages,
    /// then settles them.
    fn open_lazily(&mut self, document: &WordDocument, end_page: usize) -> Settled {
        self.renderer.reset_page_totals();
        self.renderer.sync_word_document(document);
        self.renderer.set_layout_page_limit(Some(end_page));
        self.renderer.set_visible_page_range(Some(1..=end_page));
        self.render(document);
        settle_drawn_pages(&mut self.renderer).expect("settle")
    }

    /// What an edit's render does before its payload is captured: draws the
    /// visible pages of `document`, which may have changed.
    fn render(&mut self, document: &WordDocument) {
        self.renderer.sync_word_document(document);
        self.renderer
            .render_with_numbering_rc(
                Rc::clone(document.document.as_ref().expect("document")),
                &self.options,
                document.numbering_part.as_ref(),
                document.header_lines.as_deref(),
                document.footer_lines.as_deref(),
                Some(&document.comments),
            )
            .expect("render");
    }

    /// An edit that changes the document to `document`, and the settling
    /// the capture of its payload does.
    fn edit(&mut self, document: &WordDocument) -> Settled {
        self.renderer.note_style_reference_edit();
        self.render(document);
        let settled = settle_drawn_pages(&mut self.renderer).expect("settle");
        self.renderer.clear_dirty_regions();
        settled
    }

    /// A lazy-mode edit: the render, then the dirty pages its payload
    /// reports ([`take_lazy_payload_dirty_regions`]).
    fn lazy_edit(&mut self, document: &WordDocument) -> std::collections::BTreeSet<usize> {
        self.renderer.note_style_reference_edit();
        self.render(document);
        take_lazy_payload_dirty_regions(&mut self.renderer, &mut self.kept)
            .expect("payload")
            .into_iter()
            .map(|rect| rect.page)
            .collect()
    }

    /// The pages the next lazy payload reports, with no render before it.
    fn lazy_payload(&mut self) -> std::collections::BTreeSet<usize> {
        take_lazy_payload_dirty_regions(&mut self.renderer, &mut self.kept)
            .expect("payload")
            .into_iter()
            .map(|rect| rect.page)
            .collect()
    }

    fn complete_layout(&mut self, document: &WordDocument) -> (bool, Settled) {
        let options = self.options.clone();
        complete_renderer_layout(&mut self.renderer, &options, document).expect("complete")
    }
}

/// The `n of m` a page's footer drew last, as `(n, m)`.
fn page_of_total(text: &str) -> Option<(usize, usize)> {
    let at = text.rfind(" of ")?;
    let before: String = text[..at]
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let after: String = text[at + 4..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    Some((before.parse().ok()?, after.parse().ok()?))
}

/// A package of `body` with the default header `header` and footer `footer`
/// (a paragraph's inner XML each, `None` for none), and Heading1.
fn package(body: &str, header: Option<&str>, footer: Option<&str>) -> Vec<u8> {
    let header_reference = header.map_or(String::new(), |_| {
        r#"<w:headerReference w:type="default" r:id="rIdHeader"/>"#.to_string()
    });
    let footer_reference = footer.map_or(String::new(), |_| {
        r#"<w:footerReference w:type="default" r:id="rIdFooter"/>"#.to_string()
    });
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}" xmlns:r="{R}"><w:body>{body}<w:sectPr>{header_reference}{footer_reference}<w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:body></w:document>"#
    );
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="{W}"><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:rPr><w:b/></w:rPr></w:style></w:styles>"#
    );
    let mut parts = vec![
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/><Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/></Types>"#.to_string(),
        ),
        (
            "_rels/.rels",
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}/officeDocument" Target="word/document.xml"/></Relationships>"#
            ),
        ),
        (
            "word/_rels/document.xml.rels",
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdStyles" Type="{R}/styles" Target="styles.xml"/><Relationship Id="rIdHeader" Type="{R}/header" Target="header1.xml"/><Relationship Id="rIdFooter" Type="{R}/footer" Target="footer1.xml"/></Relationships>"#
            ),
        ),
        ("word/document.xml", document),
        ("word/styles.xml", styles),
    ];
    if let Some(header) = header {
        parts.push((
            "word/header1.xml",
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:hdr xmlns:w="{W}" xmlns:r="{R}"><w:p>{header}</w:p></w:hdr>"#
            ),
        ));
    }
    if let Some(footer) = footer {
        parts.push((
            "word/footer1.xml",
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:ftr xmlns:w="{W}" xmlns:r="{R}"><w:p>{footer}</w:p></w:ftr>"#
            ),
        ));
    }
    let mut bytes = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in parts {
            zip.start_file(name, options).expect("start entry");
            zip.write_all(content.as_bytes()).expect("write entry");
        }
        zip.finish().expect("finish package");
    }
    bytes
}

fn load(bytes: &[u8]) -> WordDocument {
    futures::executor::block_on(WordDocument::load(bytes, &Options::default())).expect("load")
}

/// A complex field `code` whose saved result is `cached`.
fn field(code: &str, cached: &str) -> String {
    format!(
        r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> {code} </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>{cached}</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#
    )
}

/// `pages` pages, each starting with a page break, under the footer
/// `PAGE of NUMPAGES` whose saved results are both 1.
fn page_of_numpages(pages: usize) -> WordDocument {
    let body: String = (0..pages)
        .map(|page| {
            let break_before = if page == 0 {
                ""
            } else {
                "<w:pPr><w:pageBreakBefore/></w:pPr>"
            };
            format!("<w:p>{break_before}<w:r><w:t>Body {page}</w:t></w:r></w:p>")
        })
        .collect();
    let footer = format!(
        r#"{}<w:r><w:t xml:space="preserve"> of </w:t></w:r>{}"#,
        field("PAGE", "1"),
        field("NUMPAGES", "1"),
    );
    load(&package(&body, None, Some(&footer)))
}

/// A lazy open of one page draws `1 of 1`; laying the rest out learns there
/// are three, and page 1 is drawn again and reported dirty, as the native
/// host draws its window again. Typing that leaves the count as it is then
/// paints nothing more.
#[test]
fn a_lazy_open_redraws_page_one_once_the_layout_counts_the_pages() {
    let document = page_of_numpages(3);
    let mut shell = Shell::new();
    let opened = shell.open_lazily(&document, 1);
    assert!(!shell.renderer.last_layout_complete(), "one page of three");
    assert_eq!(opened.paints, 0, "an incomplete pass knows no total");
    assert_eq!(page_of_total(&shell.page_text(0)), Some((1, 1)));

    shell.forget_drawing();
    let (complete, settled) = shell.complete_layout(&document);
    assert!(complete);
    assert_eq!(shell.renderer.page_count(), 3);
    assert!(settled.pages.contains(&0), "page 1 is dirty: {settled:?}");
    assert!(
        (1..=MAX_SETTLE_PAINTS).contains(&settled.paints),
        "{settled:?}"
    );
    assert_eq!(
        page_of_total(&shell.page_text(0)),
        Some((1, 3)),
        "{:?}",
        shell.page_text(0)
    );
    // The lazy-layout limit stays the host's.
    assert_eq!(shell.renderer.layout_page_limit(), Some(1));
    assert!(shell.renderer.last_layout_complete() || shell.renderer.page_count() == 3);

    // Typing that leaves three pages paints nothing more.
    for _ in 0..3 {
        shell.forget_drawing();
        let typed = shell.edit(&document);
        assert!(typed.pages.is_empty(), "{typed:?}");
        assert_eq!(typed.paints, 0);
        assert_eq!(page_of_total(&shell.page_text(0)), Some((1, 3)));
    }
}

/// `ensureLayoutThroughPage` in chunks: the chunk that reaches the last page
/// settles page 1, which no chunk before it drew.
#[test]
fn the_chunk_that_reaches_the_last_page_settles_page_one() {
    let document = page_of_numpages(4);
    let mut shell = Shell::new();
    shell.open_lazily(&document, 1);
    shell.forget_drawing();
    let options = shell.options.clone();
    let mut dirty = std::collections::BTreeSet::new();
    for end_page in 2..=5 {
        let settled =
            extend_renderer_layout(&mut shell.renderer, Some(end_page), &options, &document)
                .expect("extend");
        assert!(settled.paints <= MAX_SETTLE_PAINTS, "{settled:?}");
        dirty.extend(settled.pages);
    }
    assert!(shell.renderer.last_layout_complete());
    assert!(dirty.contains(&0), "{dirty:?}");
    assert_eq!(page_of_total(&shell.page_text(0)), Some((1, 4)));
    // Nothing is left dirty for the next render to draw.
    assert!(!shell.renderer.has_dirty_regions());
}

/// `heading 1` Alpha, two lines, Beta, then Gamma after a page break; Beta
/// has a page break before it when `beta_moves`. The header shows
/// `STYLEREF "heading 1" \l`, the last heading on its page.
fn last_heading_header(beta_moves: bool) -> WordDocument {
    let heading = |text: &str, break_before: bool| {
        let break_before = if break_before {
            "<w:pageBreakBefore/>"
        } else {
            ""
        };
        format!(
            r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/>{break_before}</w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#
        )
    };
    let body = format!(
        "{}<w:p><w:r><w:t>one</w:t></w:r></w:p><w:p><w:r><w:t>two</w:t></w:r></w:p>{}{}",
        heading("Alpha", false),
        heading("Beta", beta_moves),
        heading("Gamma", true),
    );
    let header = format!(
        r#"<w:r><w:t xml:space="preserve">Now: </w:t></w:r>{}"#,
        field(r#"STYLEREF "heading 1" \l"#, "stale")
    );
    load(&package(&body, Some(&header), None))
}

/// The heading a page's header drew last after `Now: `.
fn header_heading(text: &str) -> Option<String> {
    let at = text.rfind("Now: ")?;
    Some(
        text[at + 5..]
            .chars()
            .take_while(char::is_ascii_alphabetic)
            .collect(),
    )
}

/// A header is drawn before the body below it: the open draws page 1's
/// STYLEREF from no headings, and the edit that moves Beta to the next page
/// draws it from where Beta was. Each time the page is drawn again with its
/// own last heading and reported dirty.
#[test]
fn a_header_styleref_settles_when_its_heading_moves_to_another_page() {
    let before = last_heading_header(false);
    let after = last_heading_header(true);
    let mut shell = Shell::new();
    let opened = shell.open_lazily(&before, 3);
    assert!(opened.pages.contains(&0), "{opened:?}");
    assert_eq!(header_heading(&shell.page_text(0)).as_deref(), Some("Beta"));
    let (complete, _) = shell.complete_layout(&before);
    assert!(complete);

    shell.forget_drawing();
    let moved = shell.edit(&after);
    assert_eq!(shell.renderer.page_count(), 3);
    assert!(moved.pages.contains(&0), "{moved:?}");
    assert!((1..=MAX_SETTLE_PAINTS).contains(&moved.paints), "{moved:?}");
    assert_eq!(
        header_heading(&shell.page_text(0)).as_deref(),
        Some("Alpha"),
        "{:?}",
        shell.page_text(0)
    );
    assert_eq!(header_heading(&shell.page_text(1)).as_deref(), Some("Beta"));

    // Typing that moves no heading paints nothing more.
    shell.forget_drawing();
    let typed = shell.edit(&after);
    assert!(typed.pages.is_empty(), "{typed:?}");
    assert_eq!(typed.paints, 0);
}

/// A header showing only `STYLEREF "heading 1" \l` above a heading A,
/// filler and a long heading B kept together: showing B, the header wraps
/// and pushes B to the next page, where the header shows A, shrinks, and
/// lets B back. No value is right; the shell's paints stop at the cap all
/// the same, the pass after them paints nothing, and an edit starts the
/// count again.
#[test]
fn a_styleref_that_moves_its_own_heading_stops_at_the_cap() {
    let long = "B ".to_string() + &"wrapping header text ".repeat(40);
    let header = field(r#"STYLEREF "heading 1" \l"#, "stale");
    let mut oscillated = 0;
    for filler in 20..60 {
        let mut body = String::from(
            r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>A</w:t></w:r></w:p>"#,
        );
        for index in 0..filler {
            body.push_str(&format!("<w:p><w:r><w:t>Line {index}</w:t></w:r></w:p>"));
        }
        body.push_str(&format!(
            r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/><w:keepLines/></w:pPr><w:r><w:t>{long}</w:t></w:r></w:p><w:p><w:r><w:t>After</w:t></w:r></w:p>"#
        ));
        let document = load(&package(&body, Some(&header), None));
        let mut shell = Shell::new();
        shell.renderer.set_visible_page_range(None);
        shell.renderer.sync_word_document(&document);
        for edit in 0..2 {
            let settled = shell.edit(&document);
            assert!(
                settled.paints <= MAX_SETTLE_PAINTS,
                "filler {filler}, edit {edit}: {settled:?}"
            );
            if settled.paints == MAX_SETTLE_PAINTS {
                oscillated += 1;
                // The capped flags are taken: the next pass that is no edit
                // paints nothing for them.
                for round in 0..3 {
                    shell.render(&document);
                    let again = settle_drawn_pages(&mut shell.renderer).expect("settle");
                    shell.renderer.clear_dirty_regions();
                    assert_eq!(again.paints, 0, "filler {filler}, round {round}: {again:?}");
                }
            }
        }
    }
    assert!(oscillated > 0, "some filler moves B with its own header");
}

/// The fixture whose two one-page sections each end with `Page n of section
/// m` (SECTIONPAGES split across runs): a lazy open and the layout of the
/// rest leave each footer with its own section's count, and a later pass
/// paints nothing more.
#[test]
fn the_sectionpages_fixture_settles_its_footers() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/header-footer-sectionpages.docx"
    );
    let document = load(&std::fs::read(path).expect("fixture"));
    let mut shell = Shell::new();
    let opened = shell.open_lazily(&document, 1);
    assert!(opened.paints <= MAX_SETTLE_PAINTS, "{opened:?}");
    let (complete, settled) = shell.complete_layout(&document);
    assert!(complete);
    assert!(settled.paints <= MAX_SETTLE_PAINTS, "{settled:?}");
    assert_eq!(shell.renderer.page_count(), 2);
    shell.renderer.set_visible_page_range(Some(1..=2));
    shell.renderer.set_layout_page_limit(None);
    shell.forget_drawing();
    let typed = shell.edit(&document);
    assert_eq!(typed.paints, 0, "{typed:?}");
    for page in 0..2 {
        let text = shell.page_text(page);
        assert!(text.contains("of section 1"), "page {page}: {text:?}");
    }
}

/// A lazy-mode edit builds its payload from the dirty regions without a
/// snapshot capture; it settles all the same. Moving Beta to the next page
/// reports page 1, drawn again with its own last heading.
#[test]
fn a_lazy_edit_that_moves_a_styleref_heading_reports_its_page() {
    let before = last_heading_header(false);
    let after = last_heading_header(true);
    let mut shell = Shell::new();
    shell.open_lazily(&before, 3);
    shell.complete_layout(&before);
    shell.forget_drawing();
    let dirty = shell.lazy_edit(&after);
    assert!(dirty.contains(&0), "{dirty:?}");
    assert_eq!(
        header_heading(&shell.page_text(0)).as_deref(),
        Some("Alpha"),
        "{:?}",
        shell.page_text(0)
    );
    assert!(!shell.renderer.has_dirty_regions());
}

/// A selection past the laid-out pages extends the layout without
/// reporting it (`extend_layout_for_selection`): the `1 of N` page it
/// settles is still reported, by the payload of the render that follows.
#[test]
fn a_page_settled_by_an_unreported_extension_is_reported_later() {
    let document = page_of_numpages(3);
    let mut shell = Shell::new();
    shell.open_lazily(&document, 1);
    let options = shell.options.clone();
    let settled =
        extend_renderer_layout(&mut shell.renderer, Some(3), &options, &document).expect("extend");
    assert!(settled.pages.contains(&0), "{settled:?}");
    keep_for_next_payload(&mut shell.kept, &settled.pages);
    assert_eq!(page_of_total(&shell.page_text(0)), Some((1, 3)));
    // The extension leaves no dirty regions of its own; the next payload
    // carries page 1 all the same.
    let reported = shell.lazy_payload();
    assert!(reported.contains(&0), "{reported:?}");
    assert!(shell.kept.is_empty(), "the payload took them");
}

/// `set_selection_range_surface_async`: the extension keeps page 1, then
/// the selection's render is captured without a report
/// (`capture_pages_after_render`), which clears the dirty tracker. The kept
/// page outlives it, as it would a `clearDirtyRegions`, and the next
/// reported payload carries it.
#[test]
fn a_kept_page_outlives_an_unreported_capture() {
    let document = page_of_numpages(3);
    let mut shell = Shell::new();
    shell.open_lazily(&document, 1);
    let options = shell.options.clone();
    let settled =
        extend_renderer_layout(&mut shell.renderer, Some(3), &options, &document).expect("extend");
    assert!(settled.pages.contains(&0), "{settled:?}");
    keep_for_next_payload(&mut shell.kept, &settled.pages);

    // The selection's render and its unreported capture: it settles
    // nothing new, keeps what it settled, and clears the tracker.
    shell.renderer.skip_change_detection_once();
    shell.render(&document);
    let unreported = settle_drawn_pages(&mut shell.renderer).expect("settle");
    keep_for_next_payload(&mut shell.kept, &unreported.pages);
    shell.renderer.clear_dirty_regions();
    assert!(shell.kept.contains(&0), "{:?}", shell.kept);

    let reported = shell.lazy_payload();
    assert!(reported.contains(&0), "{reported:?}");
    assert!(shell.kept.is_empty());
    // Reported once: the payload after it does not carry page 1 again.
    let next = shell.lazy_payload();
    assert!(!next.contains(&0), "{next:?}");
}

/// A page outside the visible range is not drawn when its total settles;
/// its pixels are left stale, so a bitmap fetch draws it again rather than
/// handing out `1 of 1`.
#[test]
fn a_settled_page_outside_the_window_is_left_stale() {
    let document = page_of_numpages(3);
    let mut shell = Shell::new();
    shell.open_lazily(&document, 1);
    assert!(shell.renderer.page_has_rendered_pixels(0));
    // Scrolled to the last page before the layout completes.
    shell.renderer.set_visible_page_range(Some(3..=3));
    shell.forget_drawing();
    let (complete, settled) = shell.complete_layout(&document);
    assert!(complete);
    assert!(settled.pages.contains(&0), "{settled:?}");
    assert!(
        shell.page_text(0).is_empty(),
        "page 1 is outside the window"
    );
    assert!(
        !shell.renderer.page_has_rendered_pixels(0),
        "page 1 is stale: {:?}",
        shell.forgotten.borrow()
    );
    // Drawn again when it is fetched, with the settled total.
    shell.renderer.set_visible_page_range(Some(1..=1));
    shell.renderer.skip_change_detection_once();
    shell.renderer.mark_full_repaint();
    shell.render(&document);
    assert_eq!(page_of_total(&shell.page_text(0)), Some((1, 3)));
    assert!(shell.renderer.page_has_rendered_pixels(0));
}

/// A lazy edit's pass is bounded: the pages past where it stopped have no
/// page box, though they still exist. Kept pages there stay kept, and the
/// payload of a pass that reaches them reports them.
#[test]
fn kept_pages_past_a_bounded_pass_wait_for_a_payload_that_reaches_them() {
    let document = page_of_numpages(3);
    let mut shell = Shell::new();
    shell.open_lazily(&document, 1);
    let (complete, _) = shell.complete_layout(&document);
    assert!(complete);
    let _ = shell.lazy_payload();
    keep_for_next_payload(&mut shell.kept, &(0..3).collect());

    shell.renderer.set_layout_page_limit(Some(1));
    shell.renderer.skip_change_detection_once();
    shell.render(&document);
    assert!(!shell.renderer.last_layout_complete());
    assert!(
        shell.renderer.page_count() < 3,
        "the bounded pass stops short: {}",
        shell.renderer.page_count()
    );
    let first = shell.lazy_payload();
    assert!(first.contains(&0), "{first:?}");
    assert!(!shell.kept.is_empty(), "pages past the bound stay kept");
    assert!(first.is_disjoint(&shell.kept), "{first:?} {:?}", shell.kept);

    shell.renderer.set_layout_page_limit(None);
    shell.renderer.skip_change_detection_once();
    shell.render(&document);
    assert!(shell.renderer.last_layout_complete());
    let second = shell.lazy_payload();
    let reported: std::collections::BTreeSet<usize> = first.union(&second).copied().collect();
    assert!(
        (0..3).all(|page| reported.contains(&page)),
        "{first:?} then {second:?}"
    );
    assert!(shell.kept.is_empty());
}

/// Past the end of a complete layout a kept page no longer exists: the
/// payload drops it rather than keeping it for ever.
#[test]
fn a_kept_page_past_a_complete_layout_is_dropped() {
    let document = page_of_numpages(3);
    let mut shell = Shell::new();
    shell.renderer.sync_word_document(&document);
    shell.render(&document);
    assert!(shell.renderer.last_layout_complete());
    let _ = shell.lazy_payload();
    keep_for_next_payload(&mut shell.kept, &[1, 7].into_iter().collect());
    let reported = shell.lazy_payload();
    assert!(reported.contains(&1), "{reported:?}");
    assert!(!reported.contains(&7), "{reported:?}");
    assert!(shell.kept.is_empty(), "{:?}", shell.kept);
}

/// A payload whose settling fails leaves the kept pages as they were, for
/// the next payload to report.
#[test]
fn a_failed_settle_leaves_the_kept_pages() {
    let document = page_of_numpages(3);
    let mut shell = Shell::new();
    shell.open_lazily(&document, 1);
    keep_for_next_payload(&mut shell.kept, &[0].into_iter().collect());
    // A layout-only pass to the end proves page 1's total wrong, which the
    // next payload settles by drawing it again.
    let options = shell.options.clone();
    shell.renderer.set_layout_page_limit(None);
    shell
        .renderer
        .render_layout_with_numbering_rc(
            Rc::clone(document.document.as_ref().expect("document")),
            &options,
            document.numbering_part.as_ref(),
            document.header_lines.as_deref(),
            document.footer_lines.as_deref(),
            Some(&document.comments),
        )
        .expect("layout");
    shell.failing.set(true);
    assert!(
        take_lazy_payload_dirty_regions(&mut shell.renderer, &mut shell.kept).is_err(),
        "the paint fails"
    );
    assert_eq!(shell.kept, [0].into_iter().collect::<KeptPages>());
    shell.failing.set(false);
    let reported = shell.lazy_payload();
    assert!(reported.contains(&0), "{reported:?}");
    assert!(shell.kept.is_empty());
}
