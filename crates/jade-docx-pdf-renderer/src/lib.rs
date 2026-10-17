use flate2::Compression;
use flate2::write::ZlibEncoder;
use futures::executor::block_on;
use jade_docx_extract::Paragraph;
use jade_docx_layout::font_measurer::RegisteredFontStyle;
use jade_docx_layout::font_resolver::{ResolvedRunStyle, resolve_run_style_for_wrap};
use jade_docx_layout::line_wrap::LineWrapContext;
use jade_docx_layout::shaping_measurer::{ShapingLineWrapContext, ShapingMeasurer, ShapingOptions};
use jade_docx_layout::text_extract::{
    DocumentLineLayout, LineInfo, layout_docx_lines_from_package,
};
use jade_docx_layout::units::points_to_px;
use jade_docx_reader::{
    BodyElement, CommentsPart, Document as DocxDocument, DocumentParser, Note as DocxNote,
    OpenXmlPackage, Paragraph as DocxParagraph, Relationship, RelationshipTypes, RunImageKind,
    RunTextBox as DocxTextBox, Table as DocxTable, TextBoxBorderStyle,
};
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, StringFormat, dictionary};
use quick_xml::Reader;
use quick_xml::escape::unescape;
use quick_xml::events::Event;
use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::io::Write;
use std::rc::Rc;
use std::sync::OnceLock;
use subsetter::{Profile, subset};

const CSS_PX_PER_PT: f64 = 96.0 / 72.0;
const EMBEDDED_FONT_BYTES: &[u8] =
    include_bytes!("../../../fonts/Noto_Sans_JP/static/NotoSansJP-Regular.ttf");
const EMBEDDED_FONT_NAME: &str = "NotoSansJP-Regular";
const EMBEDDED_BOLD_FONT_BYTES: &[u8] =
    include_bytes!("../../../fonts/Noto_Sans_JP/static/NotoSansJP-Bold.ttf");
const EMBEDDED_BOLD_FONT_NAME: &str = "NotoSansJP-Bold";
/// Horizontal shear applied to italic runs: Skia's synthetic oblique
/// (`SK_Scalar1/4`, an ~14° slant), which is what Chromium paints for a
/// family without an italic face.
const SYNTHETIC_OBLIQUE_SHEAR: f64 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PdfConformance {
    PdfA2u,
    Standard,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderOptions {
    pub conformance: PdfConformance,
    pub embed_fonts: bool,
    pub raster_dpi: u16,
    pub fail_on_warnings: bool,
    /// How line-breaking widths are measured. See [`TextMeasurement`].
    pub text_measurement: TextMeasurement,
}

/// Width source used when the PDF renderer wraps text.
///
/// The renderer decides its own line breaks, so this choice moves them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextMeasurement {
    /// Raw `hmtx` advances from the single embedded face. No shaping, so no
    /// kerning and no full-width punctuation handling, and the document's own
    /// font stack is ignored. This is the long-standing behaviour and stays the
    /// default so existing PDF output is unchanged.
    #[default]
    EmbeddedFontAdvances,
    /// `ShapingMeasurer` in Chromium-parity mode, so the PDF wraps on the same
    /// widths the browser renderer wraps on. Still limited to the embedded face,
    /// which is registered under the family names Word documents use.
    BrowserParityShaping,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            conformance: PdfConformance::PdfA2u,
            embed_fonts: true,
            raster_dpi: 144,
            fail_on_warnings: false,
            text_measurement: TextMeasurement::EmbeddedFontAdvances,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PipelineStage {
    ParseDocx,
    BuildLayout,
    ResolveFonts,
    DrawPdf,
    ValidateOutput,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderReport {
    pub pages: usize,
    pub rendered_lines: usize,
    pub rendered_images: usize,
    pub rendered_header_footer_lines: usize,
    pub rendered_tables: usize,
    pub rendered_text_boxes: usize,
    pub rendered_notes: usize,
    pub rendered_math: usize,
    pub rendered_hyperlinks: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RenderError {
    EmptyInput,
    Layout(String),
    DocxExtract(String),
    DocxReadiness(String),
    ReadinessWarnings(Vec<String>),
    PdfBackend(String),
}

#[derive(Debug, Clone, PartialEq)]
struct RenderLine {
    page: usize,
    text: String,
    x: f64,
    y: f64,
    baseline_y: f64,
    width: f64,
    spans: Vec<RenderTextSpan>,
    source: RenderLineSource,
}

#[derive(Debug, Clone, PartialEq)]
struct RenderTextSpan {
    text: String,
    x_offset: f64,
    width: f64,
    font_size_pt: f64,
    baseline_shift_pt: f64,
    char_spacing_pt: f64,
    color: PdfColor,
    background_color: Option<PdfColor>,
    underline: bool,
    strike: bool,
    double_strike: bool,
    bold: bool,
    italic: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct RenderLink {
    page: usize,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderLineSource {
    Body,
    HeaderFooter,
    TextBox,
    Note,
    Math,
}

#[derive(Debug, Clone, PartialEq)]
struct RenderImage {
    page: usize,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    pixels: PdfImagePixels,
}

#[derive(Debug, Clone, PartialEq)]
struct PdfImagePixels {
    width: u32,
    height: u32,
    data: PdfImageData,
}

#[derive(Debug, Clone, PartialEq)]
struct RenderTextBox {
    page: usize,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    fill_color: Option<PdfColor>,
    border: PdfBorder,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PdfColor {
    r: f64,
    g: f64,
    b: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct PdfBorder {
    width: f64,
    color: PdfColor,
    style: TextBoxBorderStyle,
}

#[derive(Debug, Clone, PartialEq)]
enum PdfImageData {
    FlateRgb {
        rgb_zlib: Vec<u8>,
        alpha_zlib: Option<Vec<u8>>,
    },
    Jpeg {
        bytes: Vec<u8>,
        color_space: JpegColorSpace,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JpegColorSpace {
    Gray,
    Rgb,
    Cmyk { inverted: bool },
}

fn default_pdf_text_color() -> PdfColor {
    PdfColor {
        r: 0.0,
        g: 0.0,
        b: 0.0,
    }
}

#[derive(Debug, Clone, PartialEq)]
struct RenderTable {
    page: usize,
    x: f64,
    y: f64,
    column_widths: Vec<f64>,
    row_heights: Vec<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ReadinessFeatures {
    tables: usize,
    images: usize,
    text_boxes: usize,
    math: usize,
    headers_or_footers: usize,
    line_numbering_sections: usize,
    comments: usize,
    revisions: usize,
    hyperlinks: usize,
    unsupported_fields: usize,
    unsupported_run_styling: usize,
    notes: usize,
    requested_font_names: BTreeSet<String>,
}

/// Family names Word documents ask for, all served by the embedded face so a
/// shaping-based measurement still resolves every descriptor.
const EMBEDDED_FONT_ALIASES: &[&str] = &[
    "Noto Sans JP",
    "MS Gothic",
    "\u{ff2d}\u{ff33} \u{30b4}\u{30b7}\u{30c3}\u{30af}",
    "MS Mincho",
    "\u{ff2d}\u{ff33} \u{660e}\u{671d}",
];

enum PdfLayoutContext {
    EmbeddedAdvances,
    Shaping(Box<ShapingLineWrapContext>),
}

impl PdfLayoutContext {
    fn new(measurement: TextMeasurement) -> Self {
        match measurement {
            TextMeasurement::EmbeddedFontAdvances => Self::EmbeddedAdvances,
            TextMeasurement::BrowserParityShaping => {
                let mut measurer =
                    ShapingMeasurer::with_options(12.0, ShapingOptions::CHROMIUM_PARITY);
                for family in EMBEDDED_FONT_ALIASES {
                    // Bold runs are painted with the real embedded bold face,
                    // so bold descriptors must measure with that face's
                    // advances too — matching both what this renderer paints
                    // and a browser that has the bold face installed. (The
                    // regular-face fallback locked by jade-docx-layout
                    // tests/chromium_fake_bold.rs only applies to families
                    // that register no bold face at all.)
                    for (weight, bytes) in
                        [(400, EMBEDDED_FONT_BYTES), (700, EMBEDDED_BOLD_FONT_BYTES)]
                    {
                        if measurer
                            .register_font_bytes(
                                *family,
                                weight,
                                RegisteredFontStyle::Normal,
                                bytes,
                            )
                            .is_err()
                        {
                            return Self::EmbeddedAdvances;
                        }
                    }
                }
                Self::Shaping(Box::new(ShapingLineWrapContext::new(measurer, None)))
            }
        }
    }
}

impl LineWrapContext for PdfLayoutContext {
    type Error = Infallible;

    fn resolve_run_style(
        &self,
        properties: Option<&jade_docx_reader::RunProperties>,
    ) -> Result<Rc<ResolvedRunStyle>, Self::Error> {
        Ok(resolve_run_style_for_wrap(properties, None))
    }

    fn measure_text(&self, text: &str, font_descriptor: &str) -> Result<f64, Self::Error> {
        match self {
            Self::EmbeddedAdvances => {
                let size_pt = parse_font_size_pt(font_descriptor).unwrap_or(12.0);
                let bold = font_descriptor_requests_bold(font_descriptor);
                Ok(
                    (embedded_pdf_text_width_pt(text, f64::from(size_pt), bold) * CSS_PX_PER_PT)
                        .max(0.0),
                )
            }
            // `ShapingLineWrapContext` only fails when a font fails to register,
            // which `new` already ruled out, so the width is always available.
            Self::Shaping(context) => {
                Ok(context.measure_text(text, font_descriptor).unwrap_or(0.0))
            }
        }
    }

    fn measure_line_box_height(&self, font_descriptor: &str) -> Result<Option<f64>, Self::Error> {
        match self {
            Self::EmbeddedAdvances => Ok(None),
            Self::Shaping(context) => Ok(context
                .measure_line_box_height(font_descriptor)
                .ok()
                .flatten()),
        }
    }

    fn measure_font_vertical_metrics(
        &self,
        font_descriptor: &str,
    ) -> Result<Option<(f64, f64)>, Self::Error> {
        match self {
            Self::EmbeddedAdvances => Ok(None),
            Self::Shaping(context) => Ok(context
                .measure_font_vertical_metrics(font_descriptor)
                .ok()
                .flatten()),
        }
    }
}

pub fn default_pipeline() -> [PipelineStage; 5] {
    [
        PipelineStage::ParseDocx,
        PipelineStage::BuildLayout,
        PipelineStage::ResolveFonts,
        PipelineStage::DrawPdf,
        PipelineStage::ValidateOutput,
    ]
}

pub fn render_docx_to_pdf(
    docx_bytes: &[u8],
    options: &RenderOptions,
) -> Result<Vec<u8>, RenderError> {
    render_docx_to_pdf_with_report(docx_bytes, options).map(|(pdf, _)| pdf)
}

pub fn render_docx_to_pdf_with_report(
    docx_bytes: &[u8],
    options: &RenderOptions,
) -> Result<(Vec<u8>, RenderReport), RenderError> {
    if docx_bytes.is_empty() {
        return Err(RenderError::EmptyInput);
    }

    let context = RenderDocxContext::load(docx_bytes)?;
    let render_layout = build_render_layout(&context, options.text_measurement)?;
    let readiness = scan_readiness_features(&context)?;
    let pdf = build_pdf_bytes(&render_layout, docx_bytes.len(), options)?;
    validate_pdf_output(&pdf, render_layout.page_count())?;
    let report = build_render_report(&render_layout, &readiness, options);
    if options.fail_on_warnings && !report.warnings.is_empty() {
        return Err(RenderError::ReadinessWarnings(report.warnings.clone()));
    }
    Ok((pdf, report))
}

#[derive(Debug, Clone, PartialEq)]
struct RenderLayout {
    lines: Vec<RenderLine>,
    images: Vec<RenderImage>,
    tables: Vec<RenderTable>,
    text_boxes: Vec<RenderTextBox>,
    links: Vec<RenderLink>,
    rendered_header_footer_parts: usize,
    rendered_notes: usize,
    rendered_math: usize,
    page_width: f64,
    page_height: f64,
    page_sizes: Vec<(f64, f64)>,
    used_jade_docx_layout: bool,
}

struct RenderDocxContext {
    package: OpenXmlPackage,
    doc_target: String,
    document: DocxDocument,
    relationships: Vec<Relationship>,
}

impl RenderDocxContext {
    fn load(docx_bytes: &[u8]) -> Result<Self, RenderError> {
        let package = block_on(OpenXmlPackage::load(docx_bytes))
            .map_err(|err| RenderError::DocxReadiness(err.to_string()))?;
        let doc_target = main_document_target(&package)?;
        let xml = package.get(&doc_target).ok_or_else(|| {
            RenderError::DocxReadiness(format!("missing document part {doc_target}"))
        })?;
        let mut document = DocumentParser::new()
            .parse_document_xml(&String::from_utf8_lossy(xml))
            .map_err(|err| RenderError::DocxReadiness(err.to_string()))?;
        if let Some(settings_xml) = package.get("word/settings.xml")
            && let Ok(settings) =
                jade_docx_reader::DocumentSettings::from_xml(&String::from_utf8_lossy(settings_xml))
        {
            document.default_tab_stop = settings.default_tab_stop;
        }
        let relationships = document_relationships(&package, &doc_target)?;
        Ok(Self {
            package,
            doc_target,
            document,
            relationships,
        })
    }
}

impl RenderLayout {
    fn page_count(&self) -> usize {
        self.lines
            .iter()
            .map(|line| line.page)
            .chain(self.images.iter().map(|image| image.page))
            .chain(self.tables.iter().map(|table| table.page))
            .chain(self.text_boxes.iter().map(|text_box| text_box.page))
            .chain(self.links.iter().map(|link| link.page))
            .max()
            .unwrap_or(1)
    }

    fn page_size(&self, page: usize) -> (f64, f64) {
        self.page_sizes
            .get(page.saturating_sub(1))
            .copied()
            .or_else(|| self.page_sizes.last().copied())
            .unwrap_or((self.page_width, self.page_height))
    }
}

fn body_element_layout_slots(element: &BodyElement) -> usize {
    match element {
        BodyElement::Paragraph(_) | BodyElement::Math(_) => 1,
        BodyElement::Table(table) => table
            .rows
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .flat_map(|cell| &cell.blocks)
                    .map(body_element_layout_slots)
                    .sum::<usize>()
                    + 1
            })
            .sum(),
    }
}

fn document_page_sizes(
    document: &DocxDocument,
    layout: &DocumentLineLayout,
    default_width: f64,
    default_height: f64,
) -> Vec<(f64, f64)> {
    let page_count = layout.get_lines().map(|line| line.page).max().unwrap_or(1);
    let Some(body) = document.body.as_ref() else {
        return vec![(default_width, default_height); page_count];
    };
    if document.sections.is_empty() {
        return vec![(default_width, default_height); page_count];
    }

    let mut element_slot_ends = Vec::with_capacity(body.elements.len());
    let mut slots = 0;
    for element in &body.elements {
        slots += body_element_layout_slots(element);
        element_slot_ends.push(slots);
    }

    let default_settings = document.page_settings.as_ref();
    let mut sizes = vec![(default_width, default_height); page_count];
    let mut first_page = 1;
    for section in &document.sections {
        let settings = section
            .properties
            .as_ref()
            .and_then(|properties| properties.page_settings.as_ref());
        let width = points_to_px(
            settings
                .and_then(|value| value.width)
                .or_else(|| default_settings.and_then(|value| value.width))
                .unwrap_or((default_width / CSS_PX_PER_PT) as f32),
        );
        let height = points_to_px(
            settings
                .and_then(|value| value.height)
                .or_else(|| default_settings.and_then(|value| value.height))
                .unwrap_or((default_height / CSS_PX_PER_PT) as f32),
        );
        let section_end_slot = section
            .end
            .checked_sub(1)
            .and_then(|index| element_slot_ends.get(index))
            .copied()
            .unwrap_or(slots);
        let last_page = layout
            .get_lines()
            .filter(|line| line.paragraph_index < section_end_slot)
            .map(|line| line.page)
            .max()
            .unwrap_or(first_page)
            .max(first_page)
            .min(page_count);
        for page in first_page..=last_page {
            sizes[page - 1] = (width, height);
        }
        first_page = last_page.saturating_add(1);
    }
    sizes
}

fn build_render_layout(
    context: &RenderDocxContext,
    measurement: TextMeasurement,
) -> Result<RenderLayout, RenderError> {
    if let Ok(layout) = layout_lines_from_docx(context, measurement) {
        let external_link_targets = external_hyperlink_targets(context);
        let numbering_markers = accepted_numbering_markers(context);
        let (mut lines, links) =
            collect_layout_render_lines(&layout, &external_link_targets, &numbering_markers);
        if !lines.is_empty() {
            let page_sizes = document_page_sizes(
                &context.document,
                &layout,
                layout.page_width(),
                layout.page_height(),
            );
            let images = collect_supported_render_images(context, &layout).unwrap_or_default();
            let tables = collect_table_render_grids(context, &layout);
            let (text_boxes, text_box_lines) =
                collect_text_box_render_items(context, &layout, &lines);
            lines.extend(text_box_lines);
            let (rendered_notes, note_lines) =
                collect_note_render_lines(context, &layout).unwrap_or_default();
            lines.extend(note_lines);
            let (rendered_math, math_lines) = collect_math_render_lines(context, &layout, &lines);
            lines.extend(math_lines);
            let page_count = lines.iter().map(|line| line.page).max().unwrap_or(1);
            let (rendered_header_footer_parts, header_footer_lines) =
                collect_header_footer_render_lines(
                    context,
                    page_count,
                    layout.page_width(),
                    layout.page_height(),
                )
                .unwrap_or_default();
            lines.extend(header_footer_lines);
            return Ok(RenderLayout {
                lines,
                images,
                tables,
                text_boxes,
                links,
                rendered_header_footer_parts,
                rendered_notes,
                rendered_math,
                page_width: layout.page_width(),
                page_height: layout.page_height(),
                page_sizes,
                used_jade_docx_layout: true,
            });
        }
    }

    let paragraphs = extract_paragraphs_from_context(context)?;
    let lines = paginate_unpaged_lines(collect_extract_render_lines(&paragraphs));
    Ok(RenderLayout {
        lines,
        images: Vec::new(),
        tables: Vec::new(),
        text_boxes: Vec::new(),
        links: Vec::new(),
        rendered_header_footer_parts: 0,
        rendered_notes: 0,
        rendered_math: 0,
        page_width: 595.0 * CSS_PX_PER_PT,
        page_height: 842.0 * CSS_PX_PER_PT,
        page_sizes: vec![(595.0 * CSS_PX_PER_PT, 842.0 * CSS_PX_PER_PT)],
        used_jade_docx_layout: false,
    })
}

fn build_render_report(
    layout: &RenderLayout,
    readiness: &ReadinessFeatures,
    options: &RenderOptions,
) -> RenderReport {
    let mut warnings = Vec::new();
    if !layout.used_jade_docx_layout {
        warnings.push("PDF_LAYOUT_FALLBACK: jade-docx-layout produced no lines; used plain text pagination fallback".to_string());
    }
    if layout.lines.is_empty() {
        warnings.push("PDF_NO_VISIBLE_TEXT: no visible text lines were emitted".to_string());
    }
    if !options.embed_fonts {
        warnings.push("PDF_FONT_NOT_EMBEDDED: using a non-embedded Type0 CID font resource; output depends on PDF viewer fonts".to_string());
    }
    if !options.embed_fonts && layout.lines.iter().any(|line| !line.text.is_ascii()) {
        warnings.push("PDF_NON_ASCII_TEXT_USES_VIEWER_FONT: non-ASCII text is preserved, but visual output depends on PDF viewer CID font support".to_string());
    }
    if !readiness.requested_font_names.is_empty() {
        let sample = readiness
            .requested_font_names
            .iter()
            .take(4)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        warnings.push(format!(
            "PDF_REQUESTED_FONTS_NOT_EMBEDDED: {count} requested font(s) detected ({sample}); PDF output currently uses bundled {embedded} instead of per-run font selection",
            count = readiness.requested_font_names.len(),
            embedded = EMBEDDED_FONT_NAME
        ));
    }
    let rendered_tables = layout.tables.len();
    let unsupported_tables = readiness.tables.saturating_sub(rendered_tables);
    if unsupported_tables > 0 {
        warnings.push(format!(
            "PDF_UNSUPPORTED_TABLE_DRAWING: {count} unsupported table(s) detected; simple top-level table grids are currently drawn as PDF primitives",
            count = unsupported_tables
        ));
    }
    let unsupported_images = readiness.images.saturating_sub(layout.images.len());
    if unsupported_images > 0 {
        warnings.push(format!(
            "PDF_UNSUPPORTED_IMAGE_DRAWING: {count} unsupported image(s) detected; PNG and JPEG images are currently embedded in PDF output",
            count = unsupported_images
        ));
    }
    let rendered_text_boxes = layout.text_boxes.len();
    let unsupported_text_boxes = readiness.text_boxes.saturating_sub(rendered_text_boxes);
    if unsupported_text_boxes > 0 {
        warnings.push(format!(
            "PDF_UNSUPPORTED_TEXT_BOX_DRAWING: {count} unsupported text box(es) detected; simple text boxes are currently drawn as PDF primitives",
            count = unsupported_text_boxes
        ));
    }
    let unsupported_math = readiness.math.saturating_sub(layout.rendered_math);
    if unsupported_math > 0 {
        warnings.push(format!(
            "PDF_UNSUPPORTED_MATH_DRAWING: {count} math object(s) detected; OMML is emitted only through text fallback/layout extraction where available",
            count = unsupported_math
        ));
    }
    if layout.rendered_math > 0 {
        warnings.push(format!(
            "PDF_MATH_TEXT_FALLBACK: {count} math object(s) rendered as plain text fallback; OMML visual layout is not yet reproduced",
            count = layout.rendered_math
        ));
    }
    let rendered_header_footer_lines = layout
        .lines
        .iter()
        .filter(|line| matches!(line.source, RenderLineSource::HeaderFooter))
        .count();
    let unsupported_header_footer_parts = readiness
        .headers_or_footers
        .saturating_sub(layout.rendered_header_footer_parts);
    if unsupported_header_footer_parts > 0 {
        warnings.push(format!(
            "PDF_UNSUPPORTED_HEADER_FOOTER_DRAWING: {count} unsupported substantive header/footer part(s) detected; simple text header/footer parts are currently drawn in PDF output",
            count = unsupported_header_footer_parts
        ));
    }
    if readiness.line_numbering_sections > 0 {
        warnings.push(format!(
            "PDF_UNSUPPORTED_LINE_NUMBERING: {count} section(s) request margin line numbers; PDF output does not draw line numbers yet",
            count = readiness.line_numbering_sections
        ));
    }
    if readiness.comments > 0 {
        warnings.push(format!(
            "PDF_UNSUPPORTED_COMMENT_RENDERING: {count} comment(s) detected; PDF output does not render comment markers or comment balloons yet",
            count = readiness.comments
        ));
    }
    if readiness.revisions > 0 {
        warnings.push(format!(
            "PDF_UNSUPPORTED_REVISION_MARKUP: {count} tracked revision run(s) detected; PDF output does not render insertion/deletion markup yet",
            count = readiness.revisions
        ));
    }
    let unsupported_hyperlinks = readiness.hyperlinks.saturating_sub(layout.links.len());
    if unsupported_hyperlinks > 0 {
        warnings.push(format!(
            "PDF_UNSUPPORTED_HYPERLINK_ANNOTATION: {count} hyperlink run(s) detected without external URI annotation support; PDF output preserves link text and annotates resolved external URI links",
            count = unsupported_hyperlinks
        ));
    }
    if readiness.unsupported_fields > 0 {
        warnings.push(format!(
            "PDF_UNSUPPORTED_FIELD_RENDERING: {count} unsupported field instruction(s) detected; PDF output preserves cached field text where available but does not update these fields",
            count = readiness.unsupported_fields
        ));
    }
    if readiness.unsupported_run_styling > 0 {
        warnings.push(format!(
            "PDF_UNSUPPORTED_RUN_STYLING: {count} visible run(s) contain direct text styling that PDF output does not render yet",
            count = readiness.unsupported_run_styling
        ));
    }
    let unsupported_notes = readiness.notes.saturating_sub(layout.rendered_notes);
    if unsupported_notes > 0 {
        warnings.push(format!(
            "PDF_UNSUPPORTED_NOTE_DRAWING: {count} unsupported footnote/endnote item(s) detected; simple note text is currently drawn in PDF output",
            count = unsupported_notes
        ));
    }

    RenderReport {
        pages: layout.page_count(),
        rendered_lines: layout.lines.len(),
        rendered_images: layout.images.len(),
        rendered_header_footer_lines,
        rendered_tables,
        rendered_text_boxes,
        rendered_notes: layout.rendered_notes,
        rendered_math: layout.rendered_math,
        rendered_hyperlinks: layout.links.len(),
        warnings,
    }
}

fn scan_readiness_features(context: &RenderDocxContext) -> Result<ReadinessFeatures, RenderError> {
    let package = &context.package;
    let doc_target = &context.doc_target;
    let document = &context.document;
    let document_xml = package
        .get(doc_target)
        .ok_or_else(|| RenderError::DocxReadiness(format!("missing document part {doc_target}")))?;
    let mut features = ReadinessFeatures {
        headers_or_footers: count_substantive_header_footer_references(
            package, doc_target, document,
        )?,
        line_numbering_sections: count_line_numbering_sections(document),
        comments: count_substantive_comments_in_package(package, document)?,
        revisions: count_revision_runs(document),
        hyperlinks: count_hyperlink_runs(document),
        unsupported_fields: count_unsupported_field_instructions(&String::from_utf8_lossy(
            document_xml,
        )),
        notes: count_substantive_notes_in_package(package)?,
        ..ReadinessFeatures::default()
    };
    if let Some(body) = document.body.as_ref() {
        scan_body_elements(&body.elements, &mut features);
    }
    Ok(features)
}

fn count_line_numbering_sections(document: &jade_docx_reader::Document) -> usize {
    let section_count = document
        .sections
        .iter()
        .filter_map(|section| section.properties.as_ref())
        .filter_map(|section| section.line_numbering.as_ref())
        .filter(|line_numbering| !line_numbering.is_empty())
        .count();
    if section_count > 0 {
        return section_count;
    }
    usize::from(
        document
            .default_section
            .as_ref()
            .and_then(|section| section.line_numbering.as_ref())
            .is_some_and(|line_numbering| !line_numbering.is_empty()),
    )
}

fn count_unsupported_field_instructions(xml: &str) -> usize {
    collect_field_instruction_names(xml)
        .into_iter()
        .filter(|name| !is_supported_pdf_field_instruction(name))
        .count()
}

fn collect_field_instruction_names(xml: &str) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut names = Vec::new();
    let mut in_instruction = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => match event.name().as_ref() {
                b"w:instrText" | b"instrText" => in_instruction = true,
                b"w:fldSimple" | b"fldSimple" => {
                    if let Some(instr) = field_simple_instruction(&event)
                        && let Some(name) = field_instruction_name(&instr)
                    {
                        names.push(name);
                    }
                }
                _ => {}
            },
            Ok(Event::Empty(event)) => {
                if matches!(event.name().as_ref(), b"w:fldSimple" | b"fldSimple")
                    && let Some(instr) = field_simple_instruction(&event)
                    && let Some(name) = field_instruction_name(&instr)
                {
                    names.push(name);
                }
            }
            Ok(Event::End(event)) => {
                if matches!(event.name().as_ref(), b"w:instrText" | b"instrText") {
                    in_instruction = false;
                }
            }
            Ok(Event::Text(text)) => {
                if in_instruction
                    && let Some(value) = xml_text_to_string(&text)
                    && let Some(name) = field_instruction_name(&value)
                {
                    names.push(name);
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    names
}

fn field_simple_instruction(event: &quick_xml::events::BytesStart<'_>) -> Option<String> {
    event.attributes().flatten().find_map(|attr| {
        if matches!(attr.key.as_ref(), b"w:instr" | b"instr") {
            Some(String::from_utf8_lossy(attr.value.as_ref()).to_string())
        } else {
            None
        }
    })
}

fn field_instruction_name(instruction: &str) -> Option<String> {
    instruction
        .split_whitespace()
        .find(|token| !token.starts_with('\\') && !token.starts_with('"'))
        .map(|token| token.trim_matches('"').to_ascii_uppercase())
        .filter(|token| !token.is_empty())
}

fn is_supported_pdf_field_instruction(name: &str) -> bool {
    matches!(
        name,
        "PAGE"
            | "NUMPAGES"
            | "PAGEREF"
            | "REF"
            | "SEQ"
            | "DATE"
            | "CREATEDATE"
            | "SAVEDATE"
            | "PRINTDATE"
            | "TIME"
    )
}

fn count_hyperlink_runs(document: &jade_docx_reader::Document) -> usize {
    let Some(body) = document.body.as_ref() else {
        return 0;
    };
    count_hyperlink_runs_in_elements(&body.elements)
}

fn count_hyperlink_runs_in_elements(elements: &[BodyElement]) -> usize {
    elements
        .iter()
        .map(|element| match element {
            BodyElement::Paragraph(paragraph) => paragraph
                .runs
                .iter()
                .filter(|run| run.hyperlink.is_some())
                .count(),
            BodyElement::Table(table) => table
                .rows
                .iter()
                .flat_map(|row| &row.cells)
                .map(|cell| count_hyperlink_runs_in_elements(&cell.blocks))
                .sum(),
            BodyElement::Math(_) => 0,
        })
        .sum()
}

fn count_revision_runs(document: &jade_docx_reader::Document) -> usize {
    let Some(body) = document.body.as_ref() else {
        return 0;
    };
    count_revision_runs_in_elements(&body.elements)
}

fn count_revision_runs_in_elements(elements: &[BodyElement]) -> usize {
    elements
        .iter()
        .map(|element| match element {
            BodyElement::Paragraph(paragraph) => paragraph
                .runs
                .iter()
                .filter(|run| run.revision.is_some())
                .count(),
            BodyElement::Table(table) => table
                .rows
                .iter()
                .flat_map(|row| &row.cells)
                .map(|cell| count_revision_runs_in_elements(&cell.blocks))
                .sum(),
            BodyElement::Math(_) => 0,
        })
        .sum()
}

fn count_substantive_comments_in_package(
    package: &OpenXmlPackage,
    document: &jade_docx_reader::Document,
) -> Result<usize, RenderError> {
    let mut referenced_ids = BTreeSet::new();
    if let Some(body) = document.body.as_ref() {
        collect_comment_ids_from_elements(&body.elements, &mut referenced_ids);
    }

    let Some(xml) = package.get("word/comments.xml") else {
        return Ok(referenced_ids.len());
    };
    let parser = DocumentParser::new();
    let comments = CommentsPart::from_xml(&parser, &String::from_utf8_lossy(xml))
        .map_err(|err| RenderError::DocxReadiness(err.to_string()))?;
    let substantive_ids = comments
        .comments
        .iter()
        .filter(|(_, comment)| !comment.text.trim().is_empty())
        .map(|(id, _)| id.clone())
        .collect::<BTreeSet<_>>();

    if referenced_ids.is_empty() {
        return Ok(substantive_ids.len());
    }

    Ok(referenced_ids
        .iter()
        .filter(|id| substantive_ids.contains(*id) || !id.trim().is_empty())
        .count())
}

fn collect_comment_ids_from_elements(elements: &[BodyElement], ids: &mut BTreeSet<String>) {
    for element in elements {
        match element {
            BodyElement::Paragraph(paragraph) => {
                ids.extend(
                    paragraph
                        .comment_ranges
                        .iter()
                        .map(|range| range.id.clone()),
                );
                ids.extend(
                    paragraph
                        .comment_markers
                        .iter()
                        .map(|marker| marker.id.clone()),
                );
                for run in &paragraph.runs {
                    ids.extend(run.comment_reference_ids.iter().cloned());
                }
            }
            BodyElement::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        collect_comment_ids_from_elements(&cell.blocks, ids);
                    }
                }
            }
            BodyElement::Math(_) => {}
        }
    }
}

fn count_substantive_header_footer_references(
    package: &OpenXmlPackage,
    doc_target: &str,
    document: &jade_docx_reader::Document,
) -> Result<usize, RenderError> {
    let relationships = document_relationships(package, doc_target)?;
    let mut parts = BTreeSet::new();
    for (rel_id, rel_type) in header_footer_reference_ids(document) {
        if let Some(target) = relationship_target(&relationships, &rel_id, rel_type) {
            parts.insert(resolve_part_target(doc_target, target));
        }
    }

    let mut count = 0;
    for part in parts {
        let Some(xml) = package.get(&part) else {
            continue;
        };
        if header_footer_part_has_substantive_content(&String::from_utf8_lossy(xml)) {
            count += 1;
        }
    }
    Ok(count)
}

fn header_footer_reference_ids(
    document: &jade_docx_reader::Document,
) -> Vec<(String, &'static str)> {
    let mut ids = Vec::new();
    collect_optional_header_footer_reference(
        &mut ids,
        document.header_reference.as_deref(),
        RelationshipTypes::Header.as_str(),
    );
    collect_optional_header_footer_reference(
        &mut ids,
        document.footer_reference.as_deref(),
        RelationshipTypes::Footer.as_str(),
    );
    if let Some(section) = document.default_section.as_ref() {
        collect_section_header_footer_references(&mut ids, section);
    }
    for section in &document.sections {
        if let Some(properties) = section.properties.as_ref() {
            collect_section_header_footer_references(&mut ids, properties);
        }
    }
    ids
}

fn collect_section_header_footer_references(
    ids: &mut Vec<(String, &'static str)>,
    section: &jade_docx_reader::SectionProperties,
) {
    for rel_id in [
        section.header_reference.as_deref(),
        section.header_reference_first.as_deref(),
        section.header_reference_even.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        collect_optional_header_footer_reference(
            ids,
            Some(rel_id),
            RelationshipTypes::Header.as_str(),
        );
    }
    for rel_id in [
        section.footer_reference.as_deref(),
        section.footer_reference_first.as_deref(),
        section.footer_reference_even.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        collect_optional_header_footer_reference(
            ids,
            Some(rel_id),
            RelationshipTypes::Footer.as_str(),
        );
    }
}

fn collect_optional_header_footer_reference(
    ids: &mut Vec<(String, &'static str)>,
    rel_id: Option<&str>,
    rel_type: &'static str,
) {
    let Some(rel_id) = rel_id else {
        return;
    };
    ids.push((rel_id.to_string(), rel_type));
}

fn header_footer_part_has_substantive_content(xml: &str) -> bool {
    if !extract_header_footer_text(xml, 1).trim().is_empty() {
        return true;
    }

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) | Ok(Event::Empty(event)) => {
                if matches!(
                    event.name().as_ref(),
                    b"w:drawing"
                        | b"drawing"
                        | b"w:pict"
                        | b"pict"
                        | b"w:object"
                        | b"object"
                        | b"w:tbl"
                        | b"tbl"
                        | b"w:txbxContent"
                        | b"txbxContent"
                        | b"m:oMath"
                        | b"oMath"
                        | b"m:oMathPara"
                        | b"oMathPara"
                ) {
                    return true;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => return true,
            _ => {}
        }
    }
    false
}

fn count_substantive_notes_in_package(package: &OpenXmlPackage) -> Result<usize, RenderError> {
    Ok(
        parse_substantive_notes(package, "word/footnotes.xml", b"w:footnote")?.len()
            + parse_substantive_notes(package, "word/endnotes.xml", b"w:endnote")?.len(),
    )
}

fn scan_body_elements(elements: &[BodyElement], features: &mut ReadinessFeatures) {
    for element in elements {
        match element {
            BodyElement::Paragraph(paragraph) => scan_paragraph(paragraph, features),
            BodyElement::Table(table) => {
                features.tables += 1;
                for row in &table.rows {
                    for cell in &row.cells {
                        scan_body_elements(&cell.blocks, features);
                    }
                }
            }
            BodyElement::Math(_) => {
                features.math += 1;
            }
        }
    }
}

fn scan_paragraph(paragraph: &jade_docx_reader::Paragraph, features: &mut ReadinessFeatures) {
    for run in &paragraph.runs {
        scan_run_fonts(run, features);
        scan_run_visual_styling(run, features);
        if run.image.is_some() {
            features.images += 1;
        }
        if let Some(text_box) = run.text_box.as_ref() {
            features.text_boxes += 1;
            scan_body_elements(&text_box.blocks, features);
        }
        if run.math.is_some() {
            features.math += 1;
        }
    }
}

fn scan_run_visual_styling(run: &jade_docx_reader::Run, features: &mut ReadinessFeatures) {
    let Some(properties) = run.properties.as_ref() else {
        return;
    };
    if run.text.trim().is_empty() && run.math.is_none() {
        return;
    }
    if run_has_unsupported_pdf_visual_styling(properties) {
        features.unsupported_run_styling += 1;
    }
}

fn run_has_unsupported_pdf_visual_styling(properties: &jade_docx_reader::RunProperties) -> bool {
    properties.caps == Some(true)
        || properties.small_caps == Some(true)
        || properties.shadow == Some(true)
        || properties.outline == Some(true)
        || properties.emboss == Some(true)
        || properties.imprint == Some(true)
        || properties
            .emphasis
            .as_deref()
            .is_some_and(|value| !value.trim().eq_ignore_ascii_case("none"))
        || properties
            .baseline
            .as_deref()
            .is_some_and(|value| !is_supported_pdf_baseline(value))
        || properties
            .text_position
            .is_some_and(|position| !position.is_finite())
        || properties.kerning.is_some()
        || properties
            .text_scale
            .is_some_and(|scale| !scale.is_finite() || scale <= 0.0)
        || properties.fit_text.is_some()
        || properties
            .color
            .as_deref()
            .is_some_and(|color| color != "#000000" && parse_hex_color(color).is_none())
        || properties.border.is_some()
}

fn scan_run_fonts(run: &jade_docx_reader::Run, features: &mut ReadinessFeatures) {
    let Some(properties) = run.properties.as_ref() else {
        return;
    };
    if run.text.trim().is_empty() && run.math.is_none() {
        return;
    }

    if let Some(font_family) = properties.font_family.as_deref() {
        insert_requested_font_name(&mut features.requested_font_names, font_family);
    }
    if let Some(fonts) = properties.fonts.as_ref() {
        for font_name in [
            fonts.ascii.as_deref(),
            fonts.h_ansi.as_deref(),
            fonts.east_asia.as_deref(),
            fonts.complex.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            insert_requested_font_name(&mut features.requested_font_names, font_name);
        }
        for theme_name in [
            fonts.ascii_theme.as_deref(),
            fonts.h_ansi_theme.as_deref(),
            fonts.east_asia_theme.as_deref(),
            fonts.complex_theme.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            let trimmed = theme_name.trim();
            if !trimmed.is_empty() {
                features
                    .requested_font_names
                    .insert(format!("theme:{trimmed}"));
            }
        }
    }
}

fn insert_requested_font_name(names: &mut BTreeSet<String>, font_name: &str) {
    let trimmed = font_name.trim();
    if trimmed.is_empty() || is_bundled_pdf_font_name(trimmed) {
        return;
    }
    names.insert(trimmed.to_string());
}

fn is_bundled_pdf_font_name(font_name: &str) -> bool {
    matches!(
        font_name.to_ascii_lowercase().as_str(),
        "noto sans jp" | "notosansjp-regular" | "noto sans cjk jp"
    )
}

fn collect_note_render_lines(
    context: &RenderDocxContext,
    layout: &DocumentLineLayout,
) -> Result<(usize, Vec<RenderLine>), RenderError> {
    let refs = collect_note_references(
        context
            .document
            .body
            .as_ref()
            .map(|body| body.elements.as_slice()),
    );
    let footnotes = parse_substantive_notes(&context.package, "word/footnotes.xml", b"w:footnote")?;
    let endnotes = parse_substantive_notes(&context.package, "word/endnotes.xml", b"w:endnote")?;

    let page_width = layout.page_width();
    let page_height = layout.page_height();
    let left = 72.0 * CSS_PX_PER_PT;
    let note_width = (page_width - 144.0 * CSS_PX_PER_PT).max(1.0);
    let mut lines = Vec::new();
    let mut rendered = 0usize;
    let paragraph_pages = layout
        .get_lines()
        .map(|line| (line.paragraph_index, line.page))
        .collect::<BTreeMap<_, _>>();

    let footnote_lines = note_lines_for_refs(
        &refs.footnotes,
        &footnotes,
        Some(&paragraph_pages),
        1,
        left,
        page_height - 96.0 * CSS_PX_PER_PT,
        note_width,
    );
    rendered += footnote_lines.len();
    lines.extend(footnote_lines);

    let endnote_page = layout.get_lines().map(|line| line.page).max().unwrap_or(1);
    let endnote_lines = note_lines_for_refs(
        &refs.endnotes,
        &endnotes,
        None,
        endnote_page,
        left,
        page_height - 96.0 * CSS_PX_PER_PT,
        note_width,
    );
    rendered += endnote_lines.len();
    lines.extend(endnote_lines);

    Ok((rendered, lines))
}

#[derive(Default)]
struct NoteReferences {
    footnotes: Vec<(String, usize)>,
    endnotes: Vec<(String, usize)>,
}

fn collect_note_references(elements: Option<&[BodyElement]>) -> NoteReferences {
    let mut refs = NoteReferences::default();
    if let Some(elements) = elements {
        let mut paragraph_index = 0usize;
        collect_note_references_from_elements(elements, &mut paragraph_index, &mut refs);
    }
    refs
}

fn collect_note_references_from_elements(
    elements: &[BodyElement],
    paragraph_index: &mut usize,
    refs: &mut NoteReferences,
) {
    for element in elements {
        match element {
            BodyElement::Paragraph(paragraph) => {
                let current_paragraph = *paragraph_index;
                for run in &paragraph.runs {
                    for id in &run.footnote_reference_ids {
                        if !refs.footnotes.iter().any(|(existing, _)| existing == id) {
                            refs.footnotes.push((id.clone(), current_paragraph));
                        }
                    }
                    for id in &run.endnote_reference_ids {
                        if !refs.endnotes.iter().any(|(existing, _)| existing == id) {
                            refs.endnotes.push((id.clone(), current_paragraph));
                        }
                    }
                }
                *paragraph_index = paragraph_index.saturating_add(1);
            }
            BodyElement::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        collect_note_references_from_elements(&cell.blocks, paragraph_index, refs);
                    }
                }
            }
            BodyElement::Math(_) => {
                *paragraph_index = paragraph_index.saturating_add(1);
            }
        }
    }
}

fn parse_substantive_notes(
    package: &OpenXmlPackage,
    path: &str,
    note_tag: &[u8],
) -> Result<BTreeMap<String, DocxNote>, RenderError> {
    let Some(xml) = package.get(path) else {
        return Ok(BTreeMap::new());
    };
    let notes = DocumentParser::new()
        .parse_notes_xml(&String::from_utf8_lossy(xml), note_tag)
        .map_err(|err| RenderError::DocxReadiness(err.to_string()))?;
    Ok(notes
        .into_iter()
        .filter(|note| note.note_type.is_none())
        .filter(|note| note_plain_text(note).is_some())
        .map(|note| (note.id.clone(), note))
        .collect())
}

fn note_lines_for_refs(
    refs: &[(String, usize)],
    notes: &BTreeMap<String, DocxNote>,
    paragraph_pages: Option<&BTreeMap<usize, usize>>,
    default_page: usize,
    x: f64,
    y_start: f64,
    width: f64,
) -> Vec<RenderLine> {
    let mut seen = BTreeSet::new();
    let mut page_line_counts = BTreeMap::<usize, usize>::new();
    refs.iter()
        .filter_map(|(id, paragraph_index)| {
            if !seen.insert(id.clone()) {
                return None;
            }
            let note = notes.get(id)?;
            let text = note_plain_text(note)?;
            let page = paragraph_pages
                .and_then(|pages| pages.get(paragraph_index).copied())
                .unwrap_or(default_page);
            let (page, y) =
                allocate_note_position(&mut page_line_counts, page, y_start, 72.0 * CSS_PX_PER_PT);
            Some((id, text, page, y))
        })
        .map(|(id, text, page, y)| RenderLine {
            page,
            text: format!("{id}. {text}"),
            x,
            y,
            baseline_y: y + points_to_px(12.0) * 0.8,
            width,
            spans: Vec::new(),
            source: RenderLineSource::Note,
        })
        .collect()
}

fn allocate_note_position(
    page_line_counts: &mut BTreeMap<usize, usize>,
    start_page: usize,
    y_start: f64,
    min_y: f64,
) -> (usize, f64) {
    const NOTE_LINE_HEIGHT_PX: f64 = 16.0;
    let capacity = (((y_start - min_y).max(0.0) / NOTE_LINE_HEIGHT_PX).floor() as usize + 1).max(1);
    let mut page = start_page.max(1);
    loop {
        let line_count = page_line_counts.entry(page).or_default();
        if *line_count < capacity {
            let y = (y_start - *line_count as f64 * NOTE_LINE_HEIGHT_PX).max(min_y);
            *line_count += 1;
            return (page, y);
        }
        page += 1;
    }
}

fn note_plain_text(note: &DocxNote) -> Option<String> {
    let mut lines = Vec::new();
    for block in &note.body {
        let BodyElement::Paragraph(paragraph) = block else {
            return None;
        };
        let text = paragraph
            .runs
            .iter()
            .filter(|run| !run.has_footnote_ref && !run.has_endnote_ref)
            .map(|run| run.text.as_str())
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !text.is_empty() {
            lines.push(text);
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join(" "))
    }
}

fn collect_math_render_lines(
    context: &RenderDocxContext,
    layout: &DocumentLineLayout,
    existing_lines: &[RenderLine],
) -> (usize, Vec<RenderLine>) {
    let mut previews = Vec::new();
    let mut paragraph_index = 0usize;
    if let Some(body) = context.document.body.as_ref() {
        collect_math_previews_from_elements(&body.elements, &mut paragraph_index, &mut previews);
    }
    let page_width = layout.page_width();
    let page_height = layout.page_height();
    let existing_text = existing_lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>();
    let positions_by_paragraph = layout
        .get_lines()
        .map(|line| {
            (
                line.paragraph_index,
                (line.page, line.x, line.y, line.width),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut lines = Vec::new();
    for (preview, paragraph_index) in &previews {
        if existing_text.iter().any(|line| line.contains(preview)) {
            continue;
        }
        let index = lines.len();
        let (page, x, y, width) = positions_by_paragraph
            .get(paragraph_index)
            .copied()
            .unwrap_or_else(|| {
                (
                    layout.get_lines().map(|line| line.page).max().unwrap_or(1),
                    72.0 * CSS_PX_PER_PT,
                    (page_height - 144.0 * CSS_PX_PER_PT + index as f64 * 16.0)
                        .min(page_height - 72.0 * CSS_PX_PER_PT),
                    (page_width - 144.0 * CSS_PX_PER_PT).max(1.0),
                )
            });
        lines.push(RenderLine {
            page,
            text: preview.clone(),
            x,
            y,
            baseline_y: y + points_to_px(12.0) * 0.8,
            width,
            spans: Vec::new(),
            source: RenderLineSource::Math,
        });
    }
    (previews.len(), lines)
}

fn collect_math_previews_from_elements(
    elements: &[BodyElement],
    paragraph_index: &mut usize,
    previews: &mut Vec<(String, usize)>,
) {
    for element in elements {
        match element {
            BodyElement::Paragraph(paragraph) => {
                for run in &paragraph.runs {
                    if let Some(math) = run.math.as_ref()
                        && let Some(preview) = math.preview_text(160)
                    {
                        previews.push((preview, *paragraph_index));
                    }
                }
                *paragraph_index += 1;
            }
            BodyElement::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        collect_math_previews_from_elements(
                            &cell.blocks,
                            paragraph_index,
                            previews,
                        );
                    }
                }
            }
            BodyElement::Math(math) => {
                if let Some(preview) = math.preview_text(160) {
                    previews.push((preview, *paragraph_index));
                }
                *paragraph_index += 1;
            }
        }
    }
}

fn layout_lines_from_docx(
    docx: &RenderDocxContext,
    measurement: TextMeasurement,
) -> Result<DocumentLineLayout, RenderError> {
    let layout_context = PdfLayoutContext::new(measurement);
    layout_docx_lines_from_package(&docx.package, &layout_context)
        .map_err(|err| RenderError::Layout(format!("{err:?}")))
}

fn collect_layout_render_lines(
    layout: &DocumentLineLayout,
    external_link_targets: &BTreeMap<String, String>,
    numbering_markers: &BTreeMap<usize, String>,
) -> (Vec<RenderLine>, Vec<RenderLink>) {
    let mut links = Vec::new();
    let lines = layout
        .get_lines()
        .filter_map(|line| {
            let trimmed = line.text.trim();
            if trimmed.is_empty() {
                None
            } else {
                for span in &line.hyperlinks {
                    let Some(rel_id) = span.hyperlink.relationship_id.as_deref() else {
                        continue;
                    };
                    let Some(target) = external_link_targets.get(rel_id) else {
                        continue;
                    };
                    links.push(RenderLink {
                        page: line.page,
                        x: line.x + span.x_offset,
                        y: line.y,
                        width: span.width,
                        height: line.height,
                        target: target.clone(),
                    });
                }
                let numbering_marker = (line.line_index_in_paragraph == 0)
                    .then(|| numbering_markers.get(&line.paragraph_index))
                    .flatten();
                let text = numbering_marker
                    .map(|marker| format!("{marker}{trimmed}"))
                    .unwrap_or_else(|| trimmed.to_string());
                let mut spans = render_text_spans_for_layout_line(line);
                if let (Some(marker), Some(first_span)) = (numbering_marker, spans.first_mut()) {
                    first_span.text.insert_str(0, marker);
                }
                Some(RenderLine {
                    page: line.page,
                    text,
                    x: line.x,
                    y: line.y,
                    baseline_y: line.baseline,
                    width: line.width,
                    spans,
                    source: RenderLineSource::Body,
                })
            }
        })
        .collect();
    (lines, links)
}

fn accepted_numbering_markers(context: &RenderDocxContext) -> BTreeMap<usize, String> {
    extract_paragraphs_from_context(context)
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .filter_map(|(paragraph_index, paragraph)| {
            (!paragraph.accepted_numbering_text.is_empty())
                .then_some((paragraph_index, paragraph.accepted_numbering_text))
        })
        .collect()
}

fn extract_paragraphs_from_context(
    context: &RenderDocxContext,
) -> Result<Vec<Paragraph>, RenderError> {
    let document_xml = context.package.get(&context.doc_target).ok_or_else(|| {
        RenderError::DocxExtract(format!("missing document part {}", context.doc_target))
    })?;
    let numbering_xml = context.package.get("word/numbering.xml");
    let styles_xml = context.package.get("word/styles.xml");
    jade_docx_extract::extract_from_xml_parts(
        &String::from_utf8_lossy(document_xml),
        numbering_xml
            .map(|xml| String::from_utf8_lossy(xml))
            .as_deref(),
        styles_xml
            .map(|xml| String::from_utf8_lossy(xml))
            .as_deref(),
    )
    .map_err(|error| RenderError::DocxExtract(error.to_string()))
}

fn render_text_spans_for_layout_line(line: &LineInfo) -> Vec<RenderTextSpan> {
    if !line.spans.iter().any(|span| {
        span.properties
            .as_ref()
            .is_some_and(run_properties_have_supported_pdf_text_styling)
    }) {
        return Vec::new();
    }
    line.spans
        .iter()
        .filter(|span| !span.text.is_empty() && span.width > 0.0)
        .map(|span| {
            let properties = span.properties.as_ref();
            RenderTextSpan {
                text: span.text.clone(),
                x_offset: span.x_offset,
                width: span.width,
                font_size_pt: properties.map(run_pdf_font_size_pt).unwrap_or(12.0),
                baseline_shift_pt: properties.map(run_pdf_baseline_shift_pt).unwrap_or(0.0),
                char_spacing_pt: properties
                    .and_then(|props| props.char_spacing)
                    .filter(|spacing| spacing.is_finite())
                    .unwrap_or(0.0) as f64,
                color: properties
                    .and_then(|props| props.color.as_deref())
                    .and_then(parse_hex_color)
                    .unwrap_or_else(default_pdf_text_color),
                background_color: properties.and_then(run_background_color),
                underline: properties.and_then(|props| props.underline) == Some(true),
                strike: properties.and_then(|props| props.strike) == Some(true),
                double_strike: properties.and_then(|props| props.double_strike) == Some(true),
                bold: properties.is_some_and(run_pdf_bold),
                italic: properties.is_some_and(run_pdf_italic),
            }
        })
        .collect()
}

/// Bold in either the Latin (`w:b`) or complex-script (`w:bCs`) slot selects
/// the embedded bold face at paint time.
fn run_pdf_bold(properties: &jade_docx_reader::RunProperties) -> bool {
    properties.bold == Some(true) || properties.bold_complex == Some(true)
}

/// Italic in either the Latin (`w:i`) or complex-script (`w:iCs`) slot slants
/// the run with a synthetic oblique shear at paint time — the bundled family
/// has no italic faces, matching a browser that falls back to fake italic.
fn run_pdf_italic(properties: &jade_docx_reader::RunProperties) -> bool {
    properties.italic == Some(true) || properties.italic_complex == Some(true)
}

fn run_properties_have_supported_pdf_text_styling(
    properties: &jade_docx_reader::RunProperties,
) -> bool {
    run_pdf_bold(properties)
        || run_pdf_italic(properties)
        || properties
            .font_size
            .is_some_and(|size| (size - 12.0).abs() > f32::EPSILON)
        || properties
            .color
            .as_deref()
            .is_some_and(|color| color != "#000000" && parse_hex_color(color).is_some())
        || properties.underline == Some(true)
        || properties.strike == Some(true)
        || properties.double_strike == Some(true)
        || run_background_color(properties).is_some()
        || properties
            .char_spacing
            .is_some_and(|spacing| spacing.is_finite() && spacing.abs() > f32::EPSILON)
        || properties.text_scale.is_some_and(|scale| {
            scale.is_finite() && scale > 0.0 && (scale - 100.0).abs() > f32::EPSILON
        })
        || properties
            .text_position
            .is_some_and(|position| position.is_finite() && position.abs() > f32::EPSILON)
        || properties
            .baseline
            .as_deref()
            .is_some_and(|value| baseline_shift_kind(value).is_some())
}

fn run_background_color(properties: &jade_docx_reader::RunProperties) -> Option<PdfColor> {
    properties
        .highlight
        .as_deref()
        .and_then(highlight_color)
        .or_else(|| {
            properties
                .shading
                .as_ref()
                .and_then(|shading| shading.resolved_background_color())
                .as_deref()
                .and_then(parse_hex_color)
        })
}

fn run_pdf_font_size_pt(properties: &jade_docx_reader::RunProperties) -> f64 {
    let base = properties.font_size.unwrap_or(12.0) as f64;
    if properties.baseline.as_deref().is_some_and(|value| {
        matches!(
            baseline_shift_kind(value),
            Some(BaselineShiftKind::Super | BaselineShiftKind::Sub)
        )
    }) {
        base * 0.66
    } else {
        base
    }
}

fn run_pdf_baseline_shift_pt(properties: &jade_docx_reader::RunProperties) -> f64 {
    let base = properties.font_size.unwrap_or(12.0) as f64;
    let explicit_shift = properties
        .text_position
        .filter(|position| position.is_finite())
        .unwrap_or(0.0) as f64;
    let align_shift = match properties.baseline.as_deref().and_then(baseline_shift_kind) {
        Some(BaselineShiftKind::Super) => base * 0.33,
        Some(BaselineShiftKind::Sub) => -base * 0.14,
        None => 0.0,
    };
    explicit_shift + align_shift
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BaselineShiftKind {
    Super,
    Sub,
}

fn baseline_shift_kind(value: &str) -> Option<BaselineShiftKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "superscript" | "super" => Some(BaselineShiftKind::Super),
        "subscript" | "sub" => Some(BaselineShiftKind::Sub),
        _ => None,
    }
}

fn is_supported_pdf_baseline(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case("baseline") || baseline_shift_kind(value).is_some()
}

fn external_hyperlink_targets(context: &RenderDocxContext) -> BTreeMap<String, String> {
    context
        .relationships
        .iter()
        .filter(|rel| {
            rel.r#type == RelationshipTypes::Hyperlink.as_str()
                && rel
                    .target_mode
                    .as_deref()
                    .is_some_and(|mode| mode.eq_ignore_ascii_case("External"))
        })
        .map(|rel| (rel.id.clone(), rel.target.clone()))
        .collect()
}

fn collect_header_footer_render_lines(
    context: &RenderDocxContext,
    page_count: usize,
    page_width: f64,
    page_height: f64,
) -> Result<(usize, Vec<RenderLine>), RenderError> {
    let header_part = context
        .document
        .header_reference
        .as_deref()
        .and_then(|id| {
            relationship_target(
                &context.relationships,
                id,
                RelationshipTypes::Header.as_str(),
            )
        })
        .map(|target| resolve_part_target(&context.doc_target, target));
    let footer_part = context
        .document
        .footer_reference
        .as_deref()
        .and_then(|id| {
            relationship_target(
                &context.relationships,
                id,
                RelationshipTypes::Footer.as_str(),
            )
        })
        .map(|target| resolve_part_target(&context.doc_target, target));

    let mut rendered_parts = BTreeSet::new();
    let mut lines = Vec::new();
    if let Some(part) = header_part.as_deref()
        && let Some(xml) = context.package.get(part)
    {
        let text = extract_header_footer_text(&String::from_utf8_lossy(xml), 1);
        if !text.trim().is_empty() {
            rendered_parts.insert(part.to_string());
        }
    }
    if let Some(part) = footer_part.as_deref()
        && let Some(xml) = context.package.get(part)
    {
        let text = extract_header_footer_text(&String::from_utf8_lossy(xml), 1);
        if !text.trim().is_empty() {
            rendered_parts.insert(part.to_string());
        }
    }
    for page in 1..=page_count {
        if let Some(part) = header_part.as_deref()
            && let Some(xml) = context.package.get(part)
        {
            let text = extract_header_footer_text(&String::from_utf8_lossy(xml), page);
            if !text.trim().is_empty() {
                lines.push(RenderLine {
                    page,
                    text,
                    x: (72.0 * CSS_PX_PER_PT).min(page_width * 0.2),
                    y: 36.0 * CSS_PX_PER_PT,
                    baseline_y: 36.0 * CSS_PX_PER_PT + points_to_px(12.0) * 0.8,
                    width: page_width * 0.8,
                    spans: Vec::new(),
                    source: RenderLineSource::HeaderFooter,
                });
            }
        }
        if let Some(part) = footer_part.as_deref()
            && let Some(xml) = context.package.get(part)
        {
            let text = extract_header_footer_text(&String::from_utf8_lossy(xml), page);
            if !text.trim().is_empty() {
                lines.push(RenderLine {
                    page,
                    text,
                    x: (72.0 * CSS_PX_PER_PT).min(page_width * 0.2),
                    y: (page_height - 36.0 * CSS_PX_PER_PT).max(0.0),
                    baseline_y: (page_height - 36.0 * CSS_PX_PER_PT).max(0.0)
                        + points_to_px(12.0) * 0.8,
                    width: page_width * 0.8,
                    spans: Vec::new(),
                    source: RenderLineSource::HeaderFooter,
                });
            }
        }
    }
    Ok((rendered_parts.len(), lines))
}

fn collect_table_render_grids(
    context: &RenderDocxContext,
    layout: &DocumentLineLayout,
) -> Vec<RenderTable> {
    let page_settings = context.document.page_settings.as_ref();
    let content_left = points_to_px(page_settings.and_then(|s| s.margin_left).unwrap_or(72.0));
    let page_width = points_to_px(page_settings.and_then(|s| s.width).unwrap_or(595.0));
    let margin_right = points_to_px(page_settings.and_then(|s| s.margin_right).unwrap_or(72.0));
    let available_width = (page_width - margin_right - content_left).max(1.0);
    let positions_by_paragraph = layout
        .get_lines()
        .map(|line| (line.paragraph_index, (line.page, line.y)))
        .collect::<BTreeMap<_, _>>();

    let mut tables = Vec::new();
    let mut paragraph_index = 0usize;
    if let Some(body) = context.document.body.as_ref() {
        for element in &body.elements {
            match element {
                BodyElement::Table(table) => {
                    if let Some(&(page, y)) = positions_by_paragraph.get(&paragraph_index)
                        && let Some(render_table) = render_grid_for_top_level_table(
                            table,
                            page,
                            content_left,
                            y,
                            available_width,
                        )
                    {
                        tables.push(render_table);
                    }
                    paragraph_index += count_table_paragraphs(table);
                }
                BodyElement::Paragraph(_) => paragraph_index += 1,
                BodyElement::Math(_) => {
                    paragraph_index += 1;
                }
            }
        }
    }
    tables
}

fn render_grid_for_top_level_table(
    table: &DocxTable,
    page: usize,
    x: f64,
    y: f64,
    available_width: f64,
) -> Option<RenderTable> {
    if table.rows.is_empty() || !is_simple_rectangular_table(table) {
        return None;
    }
    let mut column_widths = table
        .columns
        .iter()
        .filter_map(|column| column.width.map(points_to_px))
        .collect::<Vec<_>>();
    if column_widths.is_empty() {
        let column_count = table
            .rows
            .iter()
            .map(|row| row.cells.len())
            .max()
            .unwrap_or(1)
            .max(1);
        column_widths = vec![available_width / column_count as f64; column_count];
    }
    let row_heights = table
        .rows
        .iter()
        .map(|row| {
            row.height
                .map(points_to_px)
                .unwrap_or_else(|| points_to_px(12.0) * 1.2 + points_to_px(8.0))
        })
        .collect::<Vec<_>>();
    if column_widths.is_empty() || row_heights.is_empty() {
        return None;
    }
    Some(RenderTable {
        page,
        x,
        y,
        column_widths,
        row_heights,
    })
}

fn count_table_paragraphs(table: &DocxTable) -> usize {
    table
        .rows
        .iter()
        .flat_map(|row| &row.cells)
        .map(|cell| count_body_paragraphs(&cell.blocks))
        .sum()
}

fn count_body_paragraphs(elements: &[BodyElement]) -> usize {
    elements
        .iter()
        .map(|element| match element {
            BodyElement::Paragraph(_) | BodyElement::Math(_) => 1,
            BodyElement::Table(table) => count_table_paragraphs(table),
        })
        .sum()
}

fn is_simple_rectangular_table(table: &DocxTable) -> bool {
    let Some(first_row) = table.rows.first() else {
        return false;
    };
    let column_count = first_row.cells.len();
    column_count > 0
        && table.rows.iter().all(|row| {
            row.grid_before.is_none()
                && row.grid_after.is_none()
                && row.cells.len() == column_count
                && row.cells.iter().all(|cell| {
                    cell.span.unwrap_or(1) == 1
                        && cell.vertical_merge.is_none()
                        && !cell
                            .blocks
                            .iter()
                            .any(|block| matches!(block, BodyElement::Table(_)))
                })
        })
}

fn collect_text_box_render_items(
    context: &RenderDocxContext,
    layout: &DocumentLineLayout,
    existing_lines: &[RenderLine],
) -> (Vec<RenderTextBox>, Vec<RenderLine>) {
    let line_positions = layout
        .get_lines()
        .map(|line| (line.page, line.x, line.y))
        .collect::<Vec<_>>();
    let existing_text = existing_lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>();
    let mut position_index = 0usize;
    let mut boxes = Vec::new();
    let mut fallback_lines = Vec::new();
    if let Some(body) = context.document.body.as_ref() {
        collect_text_box_render_items_from_elements(
            &body.elements,
            &line_positions,
            &existing_text,
            &mut position_index,
            &mut boxes,
            &mut fallback_lines,
        );
    }
    (boxes, fallback_lines)
}

fn collect_text_box_render_items_from_elements(
    elements: &[BodyElement],
    line_positions: &[(usize, f64, f64)],
    existing_text: &[&str],
    position_index: &mut usize,
    boxes: &mut Vec<RenderTextBox>,
    fallback_lines: &mut Vec<RenderLine>,
) {
    for element in elements {
        match element {
            BodyElement::Paragraph(paragraph) => collect_text_box_render_items_from_paragraph(
                paragraph,
                line_positions,
                existing_text,
                position_index,
                boxes,
                fallback_lines,
            ),
            BodyElement::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        collect_text_box_render_items_from_elements(
                            &cell.blocks,
                            line_positions,
                            existing_text,
                            position_index,
                            boxes,
                            fallback_lines,
                        );
                    }
                }
            }
            BodyElement::Math(_) => {}
        }
    }
}

fn collect_text_box_render_items_from_paragraph(
    paragraph: &DocxParagraph,
    line_positions: &[(usize, f64, f64)],
    existing_text: &[&str],
    position_index: &mut usize,
    boxes: &mut Vec<RenderTextBox>,
    fallback_lines: &mut Vec<RenderLine>,
) {
    for run in &paragraph.runs {
        let Some(text_box) = run.text_box.as_ref() else {
            continue;
        };
        if !is_simple_renderable_text_box(text_box) {
            continue;
        }
        let (page, x, y) = text_box_position(text_box, line_positions, position_index);
        let width = text_box.width_px.unwrap_or(160.0);
        let height = text_box.height_px.unwrap_or(60.0);
        boxes.push(RenderTextBox {
            page,
            x,
            y,
            width,
            height,
            fill_color: text_box.fill_color.as_deref().and_then(parse_hex_color),
            border: PdfBorder {
                width: text_box.border.width_px.unwrap_or(1.0).max(0.0),
                color: text_box
                    .border
                    .color
                    .as_deref()
                    .and_then(parse_hex_color)
                    .unwrap_or(PdfColor {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                    }),
                style: text_box.border.style.clone(),
            },
        });
        add_missing_text_box_lines(text_box, page, x, y, width, existing_text, fallback_lines);
    }
}

fn is_simple_renderable_text_box(text_box: &DocxTextBox) -> bool {
    !text_box.text.trim().is_empty()
        && text_box.blocks.iter().all(|block| match block {
            BodyElement::Paragraph(_) => true,
            BodyElement::Table(_) | BodyElement::Math(_) => false,
        })
}

fn text_box_position(
    text_box: &DocxTextBox,
    line_positions: &[(usize, f64, f64)],
    position_index: &mut usize,
) -> (usize, f64, f64) {
    match text_box.layout.kind {
        RunImageKind::Inline => {
            let position = line_positions.get(*position_index).copied().unwrap_or((
                1,
                72.0 * CSS_PX_PER_PT,
                72.0 * CSS_PX_PER_PT,
            ));
            *position_index += 1;
            position
        }
        RunImageKind::Floating => floating_image_position(&text_box.layout, line_positions),
    }
}

fn add_missing_text_box_lines(
    text_box: &DocxTextBox,
    page: usize,
    x: f64,
    y: f64,
    width: f64,
    existing_text: &[&str],
    fallback_lines: &mut Vec<RenderLine>,
) {
    let missing_lines = text_box
        .text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| {
            !existing_text
                .iter()
                .any(|existing| existing.contains(*line))
        })
        .collect::<Vec<_>>();
    for (index, line) in missing_lines.into_iter().enumerate() {
        fallback_lines.push(RenderLine {
            page,
            text: line.to_string(),
            x: x + 6.0,
            y: y + 16.0 + index as f64 * 16.0,
            baseline_y: y + 16.0 + index as f64 * 16.0 + points_to_px(12.0) * 0.8,
            width: (width - 12.0).max(1.0),
            spans: Vec::new(),
            source: RenderLineSource::TextBox,
        });
    }
}

fn parse_hex_color(value: &str) -> Option<PdfColor> {
    let hex = value.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.is_ascii() || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(PdfColor {
        r: f64::from(r) / 255.0,
        g: f64::from(g) / 255.0,
        b: f64::from(b) / 255.0,
    })
}

fn highlight_color(value: &str) -> Option<PdfColor> {
    match value.trim().to_ascii_lowercase().as_str() {
        "yellow" => parse_hex_color("#FFFF00"),
        "green" => parse_hex_color("#00FF00"),
        "cyan" => parse_hex_color("#00FFFF"),
        "magenta" => parse_hex_color("#FF00FF"),
        "blue" => parse_hex_color("#0000FF"),
        "red" => parse_hex_color("#FF0000"),
        "darkblue" => parse_hex_color("#000080"),
        "darkcyan" => parse_hex_color("#008080"),
        "darkgreen" => parse_hex_color("#008000"),
        "darkmagenta" => parse_hex_color("#800080"),
        "darkred" => parse_hex_color("#800000"),
        "darkyellow" => parse_hex_color("#808000"),
        "darkgray" | "darkgrey" => parse_hex_color("#808080"),
        "lightgray" | "lightgrey" => parse_hex_color("#C0C0C0"),
        "black" => parse_hex_color("#000000"),
        "white" => parse_hex_color("#FFFFFF"),
        "none" => None,
        _ => None,
    }
}

fn relationship_target<'a>(
    relationships: &'a [Relationship],
    id: &str,
    rel_type: &str,
) -> Option<&'a str> {
    relationships
        .iter()
        .find(|rel| rel.id == id && rel.r#type == rel_type)
        .map(|rel| rel.target.as_str())
}

fn extract_header_footer_text(xml: &str, page: usize) -> String {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut output = String::new();
    let mut in_text = false;
    let mut in_instruction = false;
    let mut instruction = String::new();
    let mut suppress_complex_result = false;
    let mut simple_page_depth = 0usize;

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => match event.name().as_ref() {
                b"w:t" | b"t" => in_text = true,
                b"w:instrText" | b"instrText" => in_instruction = true,
                b"w:fldSimple" | b"fldSimple" => {
                    if field_simple_instruction(&event)
                        .as_deref()
                        .and_then(field_instruction_name)
                        .is_some_and(|name| name == "PAGE")
                    {
                        output.push_str(&page.to_string());
                        simple_page_depth += 1;
                    }
                }
                _ => {}
            },
            Ok(Event::Empty(event)) => match event.name().as_ref() {
                b"w:tab" | b"tab" => output.push(' '),
                b"w:fldSimple" | b"fldSimple" => {
                    if field_simple_instruction(&event)
                        .as_deref()
                        .and_then(field_instruction_name)
                        .is_some_and(|name| name == "PAGE")
                    {
                        output.push_str(&page.to_string());
                    }
                }
                b"w:fldChar" | b"fldChar" => {
                    let field_type = event.attributes().flatten().find_map(|attr| {
                        matches!(attr.key.as_ref(), b"w:fldCharType" | b"fldCharType")
                            .then(|| String::from_utf8_lossy(attr.value.as_ref()).into_owned())
                    });
                    match field_type.as_deref() {
                        Some("begin") => {
                            instruction.clear();
                            suppress_complex_result = false;
                        }
                        Some("separate") => {
                            suppress_complex_result = field_instruction_name(&instruction)
                                .is_some_and(|name| name == "PAGE");
                            if suppress_complex_result {
                                output.push_str(&page.to_string());
                            }
                        }
                        Some("end") => {
                            instruction.clear();
                            suppress_complex_result = false;
                        }
                        _ => {}
                    }
                }
                _ => {}
            },
            Ok(Event::End(event)) => match event.name().as_ref() {
                b"w:t" | b"t" => in_text = false,
                b"w:instrText" | b"instrText" => in_instruction = false,
                b"w:fldSimple" | b"fldSimple" if simple_page_depth > 0 => {
                    simple_page_depth -= 1;
                }
                b"w:p" | b"p" => {
                    if !output.ends_with(' ') {
                        output.push(' ');
                    }
                }
                _ => {}
            },
            Ok(Event::Text(text)) => {
                if let Some(value) = xml_text_to_string(&text) {
                    if in_text && simple_page_depth == 0 && !suppress_complex_result {
                        output.push_str(&value);
                    } else if in_instruction {
                        instruction.push_str(&value);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn xml_text_to_string(event: &quick_xml::events::BytesText<'_>) -> Option<String> {
    let decoded = event.decode().ok()?;
    let unescaped = unescape(decoded.as_ref()).ok()?;
    Some(unescaped.into_owned())
}

fn collect_supported_render_images(
    context: &RenderDocxContext,
    layout: &DocumentLineLayout,
) -> Result<Vec<RenderImage>, RenderError> {
    let mut positions_by_paragraph: BTreeMap<usize, Vec<(usize, f64, f64)>> = BTreeMap::new();
    for line in layout.get_lines() {
        positions_by_paragraph
            .entry(line.paragraph_index)
            .or_default()
            .push((line.page, line.x, line.y));
    }
    let mut paragraph_index = 0usize;
    let mut images = Vec::new();

    if let Some(body) = context.document.body.as_ref() {
        collect_supported_render_images_from_elements(
            &body.elements,
            &context.package,
            &context.doc_target,
            &context.relationships,
            &positions_by_paragraph,
            &mut paragraph_index,
            &mut images,
        )?;
    }
    Ok(images)
}

fn main_document_target(package: &OpenXmlPackage) -> Result<String, RenderError> {
    package
        .relationships
        .iter()
        .find(|rel| rel.r#type == RelationshipTypes::OfficeDocument.as_str())
        .map(|rel| rel.target.clone())
        .ok_or_else(|| RenderError::DocxReadiness("missing main document relationship".to_string()))
}

fn document_relationships(
    package: &OpenXmlPackage,
    doc_target: &str,
) -> Result<Vec<Relationship>, RenderError> {
    let rels_path = relationship_part_path(doc_target);
    let Some(rels) = package.get(&rels_path) else {
        return Ok(Vec::new());
    };
    package
        .parse_relationships(rels)
        .map_err(|err| RenderError::DocxReadiness(err.to_string()))
}

fn relationship_part_path(part_path: &str) -> String {
    let Some((dir, file)) = part_path.rsplit_once('/') else {
        return format!("_rels/{part_path}.rels");
    };
    format!("{dir}/_rels/{file}.rels")
}

fn resolve_part_target(base_part: &str, target: &str) -> String {
    if target.starts_with('/') {
        return target.trim_start_matches('/').to_string();
    }
    let base_dir = base_part
        .rsplit_once('/')
        .map(|(dir, _)| dir)
        .unwrap_or_default();
    normalize_package_path(&format!("{base_dir}/{target}"))
}

fn normalize_package_path(path: &str) -> String {
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    parts.join("/")
}

fn collect_supported_render_images_from_elements(
    elements: &[BodyElement],
    package: &OpenXmlPackage,
    base_part: &str,
    relationships: &[Relationship],
    positions_by_paragraph: &BTreeMap<usize, Vec<(usize, f64, f64)>>,
    paragraph_index: &mut usize,
    images: &mut Vec<RenderImage>,
) -> Result<(), RenderError> {
    for element in elements {
        match element {
            BodyElement::Paragraph(paragraph) => {
                let line_positions = positions_by_paragraph
                    .get(paragraph_index)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                collect_supported_render_images_from_paragraph(
                    paragraph,
                    package,
                    base_part,
                    relationships,
                    line_positions,
                    images,
                )?;
                *paragraph_index = paragraph_index.saturating_add(1);
            }
            BodyElement::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        collect_supported_render_images_from_elements(
                            &cell.blocks,
                            package,
                            base_part,
                            relationships,
                            positions_by_paragraph,
                            paragraph_index,
                            images,
                        )?;
                    }
                }
            }
            BodyElement::Math(_) => {
                *paragraph_index = paragraph_index.saturating_add(1);
            }
        }
    }
    Ok(())
}

fn collect_supported_render_images_from_paragraph(
    paragraph: &DocxParagraph,
    package: &OpenXmlPackage,
    base_part: &str,
    relationships: &[Relationship],
    image_positions: &[(usize, f64, f64)],
    images: &mut Vec<RenderImage>,
) -> Result<(), RenderError> {
    let mut position_index = 0usize;
    for run in &paragraph.runs {
        let Some(image) = run.image.as_ref() else {
            continue;
        };
        let Some(rel_id) = image.relationship_id.as_deref() else {
            continue;
        };
        let (page, x, y) = match image.layout.kind {
            RunImageKind::Inline => {
                let Some(position) = image_positions
                    .get(position_index)
                    .or_else(|| image_positions.first())
                    .copied()
                else {
                    continue;
                };
                position_index += 1;
                position
            }
            RunImageKind::Floating => floating_image_position(&image.layout, image_positions),
        };
        let Some(rel) = relationships
            .iter()
            .find(|rel| rel.id == rel_id && rel.r#type == RelationshipTypes::Image.as_str())
        else {
            continue;
        };
        let target = resolve_part_target(base_part, &rel.target);
        let Some(bytes) = package.get(&target) else {
            continue;
        };
        let Ok(Some(pixels)) = decode_supported_image_for_pdf(&target, bytes) else {
            continue;
        };
        let width = image.width_px.unwrap_or(f64::from(pixels.width));
        let height = image.height_px.unwrap_or(f64::from(pixels.height));
        images.push(RenderImage {
            page,
            x,
            y,
            width,
            height,
            pixels,
        });
    }
    Ok(())
}

fn decode_supported_image_for_pdf(
    target: &str,
    bytes: &[u8],
) -> Result<Option<PdfImagePixels>, RenderError> {
    let target = target.to_ascii_lowercase();
    if target.ends_with(".png") {
        return decode_png_for_pdf(bytes).map(Some);
    }
    if target.ends_with(".jpg") || target.ends_with(".jpeg") || looks_like_jpeg(bytes) {
        return decode_jpeg_for_pdf(bytes).map(Some);
    }
    Ok(None)
}

fn floating_image_position(
    layout: &jade_docx_reader::RunImageLayout,
    line_positions: &[(usize, f64, f64)],
) -> (usize, f64, f64) {
    let (page, base_x, base_y) =
        line_positions
            .first()
            .copied()
            .unwrap_or((1, 72.0 * CSS_PX_PER_PT, 72.0 * CSS_PX_PER_PT));
    let x_offset = layout
        .position_h
        .as_ref()
        .and_then(|position| position.offset_px)
        .unwrap_or(0.0);
    let y_offset = layout
        .position_v
        .as_ref()
        .and_then(|position| position.offset_px)
        .unwrap_or(0.0);
    (page, base_x + x_offset, base_y + y_offset)
}

fn decode_png_for_pdf(bytes: &[u8]) -> Result<PdfImagePixels, RenderError> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|err| RenderError::PdfBackend(format!("failed to decode PNG info: {err}")))?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|err| RenderError::PdfBackend(format!("failed to decode PNG pixels: {err}")))?;
    let data = &buffer[..info.buffer_size()];
    let (rgb, alpha) = match info.color_type {
        png::ColorType::Rgb => (data.to_vec(), None),
        png::ColorType::Rgba => {
            let mut rgb = Vec::with_capacity((info.width * info.height * 3) as usize);
            let mut alpha = Vec::with_capacity((info.width * info.height) as usize);
            for pixel in data.chunks_exact(4) {
                rgb.extend_from_slice(&pixel[..3]);
                alpha.push(pixel[3]);
            }
            (rgb, Some(alpha))
        }
        png::ColorType::Grayscale => {
            let mut rgb = Vec::with_capacity((info.width * info.height * 3) as usize);
            for value in data {
                rgb.extend_from_slice(&[*value, *value, *value]);
            }
            (rgb, None)
        }
        png::ColorType::GrayscaleAlpha => {
            let mut rgb = Vec::with_capacity((info.width * info.height * 3) as usize);
            let mut alpha = Vec::with_capacity((info.width * info.height) as usize);
            for pixel in data.chunks_exact(2) {
                rgb.extend_from_slice(&[pixel[0], pixel[0], pixel[0]]);
                alpha.push(pixel[1]);
            }
            (rgb, Some(alpha))
        }
        png::ColorType::Indexed => {
            return Err(RenderError::PdfBackend(
                "indexed PNG images are not supported in PDF output yet".to_string(),
            ));
        }
    };
    Ok(PdfImagePixels {
        width: info.width,
        height: info.height,
        data: PdfImageData::FlateRgb {
            rgb_zlib: zlib_compress(&rgb)?,
            alpha_zlib: alpha.as_deref().map(zlib_compress).transpose()?,
        },
    })
}

fn decode_jpeg_for_pdf(bytes: &[u8]) -> Result<PdfImagePixels, RenderError> {
    let (width, height, color_space) = jpeg_metadata(bytes).ok_or_else(|| {
        RenderError::PdfBackend("failed to read JPEG image dimensions".to_string())
    })?;
    Ok(PdfImagePixels {
        width,
        height,
        data: PdfImageData::Jpeg {
            bytes: bytes.to_vec(),
            color_space,
        },
    })
}

fn looks_like_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xff, 0xd8])
}

fn jpeg_metadata(bytes: &[u8]) -> Option<(u32, u32, JpegColorSpace)> {
    if !looks_like_jpeg(bytes) {
        return None;
    }
    let mut index = 2usize;
    let mut adobe_cmyk = false;
    while index + 4 <= bytes.len() {
        while index < bytes.len() && bytes[index] == 0xff {
            index += 1;
        }
        if index >= bytes.len() {
            return None;
        }
        let marker = bytes[index];
        index += 1;
        if marker == 0xd9 || marker == 0xda {
            return None;
        }
        if index + 2 > bytes.len() {
            return None;
        }
        let segment_len = u16::from_be_bytes([bytes[index], bytes[index + 1]]) as usize;
        if segment_len < 2 || index + segment_len > bytes.len() {
            return None;
        }
        if marker == 0xee && segment_len >= 14 && bytes.get(index + 2..index + 7) == Some(b"Adobe")
        {
            adobe_cmyk = true;
        }
        if is_jpeg_start_of_frame(marker) {
            if segment_len < 7 {
                return None;
            }
            let height = u16::from_be_bytes([bytes[index + 3], bytes[index + 4]]) as u32;
            let width = u16::from_be_bytes([bytes[index + 5], bytes[index + 6]]) as u32;
            let color_space = match bytes.get(index + 7).copied()? {
                1 => JpegColorSpace::Gray,
                3 => JpegColorSpace::Rgb,
                4 => JpegColorSpace::Cmyk {
                    inverted: adobe_cmyk,
                },
                _ => return None,
            };
            if width > 0 && height > 0 {
                return Some((width, height, color_space));
            }
            return None;
        }
        index += segment_len;
    }
    None
}

fn is_jpeg_start_of_frame(marker: u8) -> bool {
    matches!(
        marker,
        0xc0 | 0xc1 | 0xc2 | 0xc3 | 0xc5 | 0xc6 | 0xc7 | 0xc9 | 0xca | 0xcb | 0xcd | 0xce | 0xcf
    )
}

fn zlib_compress(bytes: &[u8]) -> Result<Vec<u8>, RenderError> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(bytes)
        .map_err(|err| RenderError::PdfBackend(format!("failed to compress image data: {err}")))?;
    encoder
        .finish()
        .map_err(|err| RenderError::PdfBackend(format!("failed to finish image data: {err}")))
}

fn collect_extract_render_lines(paragraphs: &[Paragraph]) -> Vec<String> {
    paragraphs
        .iter()
        .filter_map(|para| {
            let line = format!("{}{}", para.accepted_numbering_text, para.accepted_text);
            let trimmed = line.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect()
}

fn paginate_unpaged_lines(lines: Vec<String>) -> Vec<RenderLine> {
    const LINES_PER_PAGE: usize = 40;
    lines
        .into_iter()
        .enumerate()
        .map(|(index, text)| RenderLine {
            page: (index / LINES_PER_PAGE) + 1,
            text,
            x: 72.0 * CSS_PX_PER_PT,
            y: (72.0 + (index % LINES_PER_PAGE) as f64 * 18.0) * CSS_PX_PER_PT,
            baseline_y: (72.0 + (index % LINES_PER_PAGE) as f64 * 18.0) * CSS_PX_PER_PT
                + points_to_px(12.0) * 0.8,
            width: 0.0,
            spans: Vec::new(),
            source: RenderLineSource::Body,
        })
        .collect()
}

fn normalize_pdf_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\n' | '\r' | '\t' => out.push(' '),
            _ if !ch.is_control() => out.push(ch),
            _ => {}
        }
    }
    out
}

#[derive(Debug, Clone, Default)]
struct PdfCidMap {
    non_bmp_to_cid: BTreeMap<char, u16>,
    cid_to_non_bmp: BTreeMap<u16, char>,
}

impl PdfCidMap {
    fn from_lines(lines: &[RenderLine]) -> Result<Self, RenderError> {
        let non_bmp = lines
            .iter()
            .flat_map(|line| normalize_pdf_text(&line.text).chars().collect::<Vec<_>>())
            .filter(|ch| u32::from(*ch) > u32::from(u16::MAX))
            .collect::<BTreeSet<_>>();
        if non_bmp.len() > 0x800 {
            return Err(RenderError::PdfBackend(
                "PDF output contains more than 2048 distinct non-BMP characters".to_string(),
            ));
        }
        let mut map = Self::default();
        for (offset, ch) in non_bmp.into_iter().enumerate() {
            let cid = 0xd800_u16 + offset as u16;
            map.non_bmp_to_cid.insert(ch, cid);
            map.cid_to_non_bmp.insert(cid, ch);
        }
        Ok(map)
    }

    fn cid_for_char(&self, ch: char) -> u16 {
        self.non_bmp_to_cid
            .get(&ch)
            .copied()
            .unwrap_or_else(|| u16::try_from(u32::from(ch)).unwrap_or(u16::from(b'?')))
    }

    fn char_for_cid(&self, cid: u16) -> char {
        self.cid_to_non_bmp
            .get(&cid)
            .copied()
            .or_else(|| char::from_u32(u32::from(cid)))
            .unwrap_or('\0')
    }
}

fn pdf_text_object(input: &str, cid_map: Option<&PdfCidMap>) -> Object {
    let text = normalize_pdf_text(input);
    let bytes = if let Some(cid_map) = cid_map {
        text.chars()
            .map(|ch| cid_map.cid_for_char(ch))
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>()
    } else {
        text.encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>()
    };
    Object::String(bytes, StringFormat::Hexadecimal)
}

fn embedded_pdf_text_width_pt(text: &str, font_size_pt: f64, bold: bool) -> f64 {
    static FACE: OnceLock<ttf_parser::Face<'static>> = OnceLock::new();
    static BOLD_FACE: OnceLock<ttf_parser::Face<'static>> = OnceLock::new();
    let face = if bold {
        BOLD_FACE.get_or_init(|| {
            ttf_parser::Face::parse(EMBEDDED_BOLD_FONT_BYTES, 0)
                .expect("bundled bold PDF font must remain valid")
        })
    } else {
        FACE.get_or_init(|| {
            ttf_parser::Face::parse(EMBEDDED_FONT_BYTES, 0)
                .expect("bundled PDF font must remain valid")
        })
    };
    let units_per_em = f64::from(face.units_per_em());
    let advance_units = text
        .chars()
        .map(|ch| {
            face.glyph_index(ch)
                .and_then(|glyph| face.glyph_hor_advance(glyph))
                .map(f64::from)
                .unwrap_or(units_per_em)
        })
        .sum::<f64>();
    (advance_units / units_per_em * font_size_pt).max(0.0)
}

fn horizontal_scale_percent(target_width_pt: f64, natural_width_pt: f64) -> f64 {
    if target_width_pt <= 0.0 || natural_width_pt <= 0.0 {
        return 100.0;
    }
    ((target_width_pt / natural_width_pt) * 100.0).clamp(10.0, 1000.0)
}

fn ascii_cid_widths() -> Object {
    Object::Array(vec![
        32.into(),
        Object::Array((32..=126).map(|_| 550.into()).collect()),
    ])
}

/// PDF default glyph advance width (1000 units = 1 em), used for characters the
/// bundled font does not cover so a missing glyph never fails the whole render.
const DEFAULT_GLYPH_WIDTH: i64 = 1000;

struct EmbeddedFontProgram {
    face: ttf_parser::Face<'static>,
    bytes: &'static [u8],
    units_per_em: f32,
    ascent: i64,
    descent: i64,
    cap_height: i64,
    bbox: [i64; 4],
}

impl EmbeddedFontProgram {
    fn parse() -> Result<Self, RenderError> {
        Self::parse_bytes(EMBEDDED_FONT_BYTES)
    }

    fn parse_bold() -> Result<Self, RenderError> {
        Self::parse_bytes(EMBEDDED_BOLD_FONT_BYTES)
    }

    fn parse_bytes(bytes: &'static [u8]) -> Result<Self, RenderError> {
        // The bundled font program lives in `'static` memory, so the parsed
        // `Face` can be held for the lifetime of the program and reused for
        // every glyph/width lookup instead of re-parsing the multi-megabyte
        // font once per character.
        let face = ttf_parser::Face::parse(bytes, 0).map_err(|err| {
            RenderError::PdfBackend(format!("failed to parse bundled PDF font: {err:?}"))
        })?;
        let units_per_em = f32::from(face.units_per_em()).max(1.0);
        let scale = 1000.0 / units_per_em;
        let bbox = face.global_bounding_box();
        Ok(Self {
            ascent: (f32::from(face.ascender()) * scale).round() as i64,
            descent: (f32::from(face.descender()) * scale).round() as i64,
            cap_height: face
                .capital_height()
                .map(|height| (f32::from(height) * scale).round() as i64)
                .unwrap_or_else(|| (f32::from(face.ascender()) * scale * 0.7).round() as i64),
            bbox: [
                (f32::from(bbox.x_min) * scale).round() as i64,
                (f32::from(bbox.y_min) * scale).round() as i64,
                (f32::from(bbox.x_max) * scale).round() as i64,
                (f32::from(bbox.y_max) * scale).round() as i64,
            ],
            face,
            bytes,
            units_per_em,
        })
    }

    /// Maps a character to its glyph id, falling back to `.notdef` (glyph 0) for
    /// characters the font does not cover. This keeps an unusual character from
    /// aborting the entire PDF render; it renders as a missing-glyph box instead.
    fn glyph_id_for_char(&self, ch: char) -> u16 {
        self.face.glyph_index(ch).map_or(0, |glyph_id| glyph_id.0)
    }

    fn width_for_char(&self, ch: char) -> i64 {
        let Some(glyph_id) = self.face.glyph_index(ch) else {
            return DEFAULT_GLYPH_WIDTH;
        };
        let advance = self
            .face
            .glyph_hor_advance(glyph_id)
            .unwrap_or(self.face.units_per_em());
        (f32::from(advance) * 1000.0 / self.units_per_em).round() as i64
    }
}

fn used_cids(lines: &[RenderLine], cid_map: &PdfCidMap) -> Vec<u16> {
    used_cids_from_texts(lines.iter().map(|line| line.text.as_str()), cid_map)
}

fn used_cids_from_texts<'a>(texts: impl Iterator<Item = &'a str>, cid_map: &PdfCidMap) -> Vec<u16> {
    let mut cids = texts
        .flat_map(|text| normalize_pdf_text(text).chars().collect::<Vec<_>>())
        .map(|ch| cid_map.cid_for_char(ch))
        .collect::<Vec<_>>();
    cids.sort_unstable();
    cids.dedup();
    cids
}

fn cid_widths_for_embedded_font(
    cids: &[u16],
    font: &EmbeddedFontProgram,
    cid_map: &PdfCidMap,
) -> Object {
    let mut widths = Vec::with_capacity(cids.len() * 2);
    for cid in cids {
        let ch = cid_map.char_for_cid(*cid);
        widths.push(i64::from(*cid).into());
        widths.push(Object::Array(vec![font.width_for_char(ch).into()]));
    }
    Object::Array(widths)
}

/// Collects the unique glyph ids used by the document (plus `.notdef`) so the
/// bundled font can be subset down to only those glyphs.
fn used_glyph_ids(cids: &[u16], font: &EmbeddedFontProgram, cid_map: &PdfCidMap) -> Vec<u16> {
    let mut glyph_ids = vec![0_u16];
    for cid in cids {
        let ch = cid_map.char_for_cid(*cid);
        glyph_ids.push(font.glyph_id_for_char(ch));
    }
    glyph_ids.sort_unstable();
    glyph_ids.dedup();
    glyph_ids
}

/// Subsets the bundled font to only the glyphs the document actually uses so
/// each PDF embeds tens of kilobytes instead of the multi-megabyte full font.
/// `subsetter` keeps glyph ids stable (it only strips unused outlines/tables),
/// so the `CIDToGIDMap` built from the original font stays valid. Returns the
/// font program and whether subsetting actually happened (the boolean is
/// `false` only on the fallback path where the full program is embedded).
fn subset_embedded_font(font_bytes: &[u8], glyph_ids: &[u16]) -> (Vec<u8>, bool) {
    match subset(font_bytes, 0, Profile::pdf(glyph_ids)) {
        Ok(bytes) => (bytes, true),
        Err(_) => (font_bytes.to_vec(), false),
    }
}

/// Builds the PDF font name for an embedded program. Subset programs carry the
/// spec-mandated 6-uppercase-letter tag prefix (e.g. `ABCDEF+NotoSansJP-Regular`)
/// so viewers and PDF/A validators recognize the font as a subset. The tag is
/// derived deterministically from the glyph set so distinct subsets get distinct
/// names (avoiding resource collisions when PDFs are merged).
fn embedded_font_name(base_name: &str, glyph_ids: &[u16], is_subset: bool) -> String {
    if !is_subset {
        return base_name.to_string();
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for gid in glyph_ids {
        hash ^= u64::from(*gid);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let mut tag = String::with_capacity(7);
    for _ in 0..6 {
        tag.push((b'A' + (hash % 26) as u8) as char);
        hash /= 26;
    }
    tag.push('+');
    tag.push_str(base_name);
    tag
}

fn cid_to_gid_map(cids: &[u16], font: &EmbeddedFontProgram, cid_map: &PdfCidMap) -> Vec<u8> {
    let max_cid = cids.iter().copied().max().unwrap_or(0) as usize;
    let mut map = vec![0_u8; (max_cid + 1) * 2];
    for cid in cids {
        let ch = cid_map.char_for_cid(*cid);
        let glyph_id = font.glyph_id_for_char(ch);
        let offset = *cid as usize * 2;
        map[offset..offset + 2].copy_from_slice(&glyph_id.to_be_bytes());
    }
    map
}

fn link_annotation_rect(link: &RenderLink, page_height_pt: f64) -> Object {
    let left = px_to_pt(link.x);
    let right = px_to_pt(link.x + link.width);
    let top = page_height_pt - px_to_pt(link.y);
    let bottom = page_height_pt - px_to_pt(link.y + link.height);
    Object::Array(vec![
        Object::Real(left as f32),
        Object::Real(bottom as f32),
        Object::Real(right as f32),
        Object::Real(top as f32),
    ])
}

fn add_embedded_type0_font(
    document: &mut Document,
    lines: &[RenderLine],
    cid_map: &PdfCidMap,
) -> Result<lopdf::ObjectId, RenderError> {
    let font = EmbeddedFontProgram::parse()?;
    let cids = used_cids(lines, cid_map);
    add_embedded_type0_font_from_cids(document, &font, EMBEDDED_FONT_NAME, &cids, cid_map)
}

/// Embeds the bundled bold face, subset down to the characters that bold spans
/// actually use, so bold text costs kilobytes instead of a second full CJK
/// font program.
fn add_embedded_bold_type0_font(
    document: &mut Document,
    lines: &[RenderLine],
    cid_map: &PdfCidMap,
) -> Result<lopdf::ObjectId, RenderError> {
    let font = EmbeddedFontProgram::parse_bold()?;
    let cids = used_cids_from_texts(
        lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .filter(|span| span.bold)
            .map(|span| span.text.as_str()),
        cid_map,
    );
    add_embedded_type0_font_from_cids(document, &font, EMBEDDED_BOLD_FONT_NAME, &cids, cid_map)
}

fn add_embedded_type0_font_from_cids(
    document: &mut Document,
    font: &EmbeddedFontProgram,
    base_name: &str,
    cids: &[u16],
    cid_map: &PdfCidMap,
) -> Result<lopdf::ObjectId, RenderError> {
    let glyph_ids = used_glyph_ids(cids, font, cid_map);
    let (font_program, is_subset) = subset_embedded_font(font.bytes, &glyph_ids);
    let font_name = embedded_font_name(base_name, &glyph_ids, is_subset);
    let font_file_id = document.add_object(Stream::new(
        dictionary! {
            // FontFile2 `Length1` is the length of the decompressed (subset)
            // TrueType program; the stream itself is Flate-compressed.
            "Length1" => font_program.len() as i64,
            "Filter" => "FlateDecode",
        },
        zlib_compress(&font_program)?,
    ));
    let font_descriptor_id = document.add_object(dictionary! {
        "Type" => "FontDescriptor",
        "FontName" => font_name.as_str(),
        "Flags" => 4,
        "FontBBox" => vec![font.bbox[0].into(), font.bbox[1].into(), font.bbox[2].into(), font.bbox[3].into()],
        "ItalicAngle" => 0,
        "Ascent" => font.ascent,
        "Descent" => font.descent,
        "CapHeight" => font.cap_height,
        "StemV" => 80,
        "FontFile2" => font_file_id,
    });
    let cid_to_gid_id = document.add_object(Stream::new(
        dictionary! {},
        cid_to_gid_map(cids, font, cid_map),
    ));
    let descendant_font_id = document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "CIDFontType2",
        "BaseFont" => font_name.as_str(),
        "CIDSystemInfo" => dictionary! {
            "Registry" => Object::string_literal("Adobe"),
            "Ordering" => Object::string_literal("Identity"),
            "Supplement" => 0,
        },
        "DW" => 1000,
        "W" => cid_widths_for_embedded_font(cids, font, cid_map),
        "CIDToGIDMap" => cid_to_gid_id,
        "FontDescriptor" => font_descriptor_id,
    });
    let to_unicode_id = document.add_object(Stream::new(
        dictionary! {},
        identity_to_unicode_cmap(Some(cid_map)),
    ));
    Ok(document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type0",
        "BaseFont" => font_name.as_str(),
        "Encoding" => "Identity-H",
        "DescendantFonts" => vec![descendant_font_id.into()],
        "ToUnicode" => to_unicode_id,
    }))
}

fn add_viewer_cid_font(document: &mut Document) -> lopdf::ObjectId {
    let font_descriptor_id = document.add_object(dictionary! {
        "Type" => "FontDescriptor",
        "FontName" => "KozMinPr6N-Regular",
        "Flags" => 4,
        "FontBBox" => vec![(-1000).into(), (-300).into(), 1000.into(), 1000.into()],
        "ItalicAngle" => 0,
        "Ascent" => 880,
        "Descent" => -120,
        "CapHeight" => 700,
        "StemV" => 80,
    });
    let descendant_font_id = document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "CIDFontType0",
        "BaseFont" => "KozMinPr6N-Regular",
        "CIDSystemInfo" => dictionary! {
            "Registry" => Object::string_literal("Adobe"),
            "Ordering" => Object::string_literal("Japan1"),
            "Supplement" => 6,
        },
        "DW" => 1000,
        "W" => ascii_cid_widths(),
        "FontDescriptor" => font_descriptor_id,
    });
    let to_unicode_id =
        document.add_object(Stream::new(dictionary! {}, identity_to_unicode_cmap(None)));
    document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type0",
        "BaseFont" => "KozMinPr6N-Regular",
        "Encoding" => "UniJIS-UTF16-H",
        "DescendantFonts" => vec![descendant_font_id.into()],
        "ToUnicode" => to_unicode_id,
    })
}

/// CSS-style font descriptors spell weight before the size token, either as the
/// `bold` keyword or as a numeric weight (700 and up is bold territory).
fn font_descriptor_requests_bold(font_descriptor: &str) -> bool {
    font_descriptor
        .split_whitespace()
        .take_while(|token| !token.ends_with("pt"))
        .any(|token| {
            token.eq_ignore_ascii_case("bold")
                || token.parse::<u32>().is_ok_and(|weight| weight >= 600)
        })
}

fn parse_font_size_pt(font_descriptor: &str) -> Option<f32> {
    for token in font_descriptor.split_whitespace() {
        if let Some(value) = token.strip_suffix("pt")
            && let Ok(size) = value.parse::<f32>()
        {
            return Some(size);
        }
    }
    None
}

fn add_pdf_image_objects(document: &mut Document, images: &[RenderImage]) -> Vec<lopdf::ObjectId> {
    images
        .iter()
        .map(|image| {
            let mut dict = image_xobject_dictionary(
                image,
                match &image.pixels.data {
                    PdfImageData::FlateRgb { .. } => "FlateDecode",
                    PdfImageData::Jpeg { .. } => "DCTDecode",
                },
            );
            let data = match &image.pixels.data {
                PdfImageData::FlateRgb {
                    rgb_zlib,
                    alpha_zlib,
                } => {
                    if let Some(smask) = alpha_zlib.as_ref().map(|alpha| {
                        document.add_object(Stream::new(
                            dictionary! {
                                "Type" => "XObject",
                                "Subtype" => "Image",
                                "Width" => image.pixels.width as i64,
                                "Height" => image.pixels.height as i64,
                                "ColorSpace" => "DeviceGray",
                                "BitsPerComponent" => 8,
                                "Filter" => "FlateDecode",
                            },
                            alpha.clone(),
                        ))
                    }) {
                        dict.set("SMask", smask);
                    }
                    rgb_zlib.clone()
                }
                PdfImageData::Jpeg { bytes, color_space } => {
                    dict.set(
                        "ColorSpace",
                        match color_space {
                            JpegColorSpace::Gray => "DeviceGray",
                            JpegColorSpace::Rgb => "DeviceRGB",
                            JpegColorSpace::Cmyk { .. } => "DeviceCMYK",
                        },
                    );
                    if matches!(color_space, JpegColorSpace::Cmyk { inverted: true }) {
                        dict.set(
                            "Decode",
                            Object::Array(vec![
                                1.into(),
                                0.into(),
                                1.into(),
                                0.into(),
                                1.into(),
                                0.into(),
                                1.into(),
                                0.into(),
                            ]),
                        );
                    }
                    bytes.clone()
                }
            };
            document.add_object(Stream::new(dict, data))
        })
        .collect()
}

fn image_xobject_dictionary(image: &RenderImage, filter: &str) -> lopdf::Dictionary {
    dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => image.pixels.width as i64,
        "Height" => image.pixels.height as i64,
        "ColorSpace" => "DeviceRGB",
        "BitsPerComponent" => 8,
        "Filter" => filter,
    }
}

fn build_pdf_bytes(
    layout: &RenderLayout,
    input_size: usize,
    options: &RenderOptions,
) -> Result<Vec<u8>, RenderError> {
    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let page_count = layout.page_count();
    let lines = &layout.lines;
    let images = &layout.images;
    let tables = &layout.tables;
    let text_boxes = &layout.text_boxes;
    let links = &layout.links;

    let cid_map = if options.embed_fonts {
        Some(PdfCidMap::from_lines(lines)?)
    } else {
        None
    };
    let font_id = if let Some(cid_map) = cid_map.as_ref() {
        add_embedded_type0_font(&mut document, lines, cid_map)?
    } else {
        add_viewer_cid_font(&mut document)
    };
    let document_has_bold_spans = lines
        .iter()
        .any(|line| line.spans.iter().any(|span| span.bold));
    let bold_font_id = match (cid_map.as_ref(), document_has_bold_spans) {
        (Some(cid_map), true) => Some(add_embedded_bold_type0_font(&mut document, lines, cid_map)?),
        _ => None,
    };
    let image_objects = add_pdf_image_objects(&mut document, images);

    let mut kids = Vec::with_capacity(page_count);
    for page_index in 0..page_count {
        let (page_width, page_height) = layout.page_size(page_index + 1);
        let page_width_pt = px_to_pt(page_width);
        let page_height_pt = px_to_pt(page_height);
        let font_size_pt = 12.0;
        let mut operations = Vec::new();
        let mut text_background_operations = Vec::new();
        let mut text_decoration_operations = Vec::new();
        for text_box in text_boxes
            .iter()
            .filter(|text_box| text_box.page == page_index + 1)
        {
            push_text_box_operations(&mut operations, text_box, page_height_pt);
        }
        for table in tables.iter().filter(|table| table.page == page_index + 1) {
            push_table_grid_operations(&mut operations, table, page_height_pt);
        }
        operations.push(Operation::new("BT", vec![]));
        for line in lines.iter().filter(|line| line.page == page_index + 1) {
            if line.spans.is_empty() {
                push_pdf_text_run_operations(
                    &mut operations,
                    &line.text,
                    line.x,
                    line.baseline_y,
                    line.width,
                    font_size_pt,
                    0.0,
                    0.0,
                    default_pdf_text_color(),
                    page_height_pt,
                    cid_map.as_ref(),
                    false,
                    false,
                );
            } else {
                for span in &line.spans {
                    push_pdf_text_background_operations(
                        &mut text_background_operations,
                        span,
                        line.x + span.x_offset,
                        line.y,
                        page_height_pt,
                    );
                    push_pdf_text_run_operations(
                        &mut operations,
                        &span.text,
                        line.x + span.x_offset,
                        line.baseline_y,
                        span.width,
                        span.font_size_pt,
                        span.baseline_shift_pt,
                        span.char_spacing_pt,
                        span.color,
                        page_height_pt,
                        cid_map.as_ref(),
                        // Only switch to the bold resource when it was actually
                        // added — the viewer-font fallback has no /F2.
                        span.bold && bold_font_id.is_some(),
                        // The shear needs no extra resource, so italic slants
                        // even on the viewer-font fallback path.
                        span.italic,
                    );
                    push_pdf_text_decoration_operations(
                        &mut text_decoration_operations,
                        span,
                        line.x + span.x_offset,
                        line.y,
                        page_height_pt,
                    );
                }
            }
        }
        let mut ordered_text_operations =
            Vec::with_capacity(text_background_operations.len() + operations.len());
        ordered_text_operations.extend(text_background_operations);
        ordered_text_operations.extend(operations);
        operations = ordered_text_operations;
        operations.push(Operation::new("ET", vec![]));
        operations.extend(text_decoration_operations);
        for (image_index, image) in images
            .iter()
            .enumerate()
            .filter(|(_, image)| image.page == page_index + 1)
        {
            let image_name = format!("Im{}", image_index + 1);
            operations.push(Operation::new("q", vec![]));
            operations.push(Operation::new(
                "cm",
                vec![
                    Object::Real(px_to_pt(image.width) as f32),
                    0.into(),
                    0.into(),
                    Object::Real(px_to_pt(image.height) as f32),
                    Object::Real(px_to_pt(image.x) as f32),
                    Object::Real((page_height_pt - px_to_pt(image.y + image.height)) as f32),
                ],
            ));
            operations.push(Operation::new(
                "Do",
                vec![Object::Name(image_name.into_bytes())],
            ));
            operations.push(Operation::new("Q", vec![]));
        }
        let content = Content { operations };

        let content_bytes = content
            .encode()
            .map_err(|err| RenderError::PdfBackend(err.to_string()))?;
        let content_id = document.add_object(Stream::new(dictionary! {}, content_bytes));

        let mut xobjects = lopdf::Dictionary::new();
        for (image_index, image_id) in image_objects
            .iter()
            .enumerate()
            .filter(|(image_index, _)| images[*image_index].page == page_index + 1)
        {
            xobjects.set(format!("Im{}", image_index + 1), *image_id);
        }

        let mut font_resources = dictionary! {
            "F1" => font_id,
        };
        if let Some(bold_font_id) = bold_font_id {
            font_resources.set("F2", bold_font_id);
        }
        let mut resources = dictionary! {
            "Font" => font_resources,
        };
        if !xobjects.is_empty() {
            resources.set("XObject", xobjects);
        }

        let annots = links
            .iter()
            .filter(|link| link.page == page_index + 1)
            .map(|link| {
                document.add_object(dictionary! {
                    "Type" => "Annot",
                    "Subtype" => "Link",
                    "Rect" => link_annotation_rect(link, page_height_pt),
                    "Border" => vec![0.into(), 0.into(), 0.into()],
                    "A" => dictionary! {
                        "S" => "URI",
                        "URI" => Object::string_literal(link.target.as_str()),
                    },
                })
            })
            .collect::<Vec<_>>();

        let mut page_dict = dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![
                0.into(),
                0.into(),
                Object::Real(page_width_pt as f32),
                Object::Real(page_height_pt as f32),
            ],
            "Resources" => resources,
            "Contents" => content_id,
        };
        if !annots.is_empty() {
            page_dict.set(
                "Annots",
                Object::Array(annots.into_iter().map(Object::Reference).collect()),
            );
        }
        let page_id = document.add_object(page_dict);
        kids.push(page_id.into());
    }

    let pages = dictionary! {
        "Type" => "Pages",
        "Kids" => kids,
        "Count" => page_count as i64,
    };
    document.objects.insert(pages_id, Object::Dictionary(pages));

    let metadata_id = document.add_object(Stream::new(
        dictionary! {
            "Type" => "Metadata",
            "Subtype" => "XML",
        },
        xmp_metadata_packet(),
    ));
    let catalog_id = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
        "Metadata" => metadata_id,
    });
    document.trailer.set("Root", catalog_id);

    let info_id = document.add_object(dictionary! {
        "Producer" => Object::string_literal("jade-docx-pdf-renderer"),
        "Title" => Object::string_literal("DOCX render"),
        "Subject" => Object::string_literal(format!(
            "input-bytes:{input_size};lines:{};pages:{page_count}",
            lines.len()
        )),
    });
    document.trailer.set("Info", info_id);

    // A trailer /ID is required by PDF/A and expected by virtually every PDF
    // consumer. It is derived deterministically from the document's structural
    // fingerprint (no clock) so byte-for-byte regression hashing stays stable.
    let doc_id = deterministic_doc_id(input_size, lines.len(), page_count);
    document.trailer.set(
        "ID",
        Object::Array(vec![
            Object::String(doc_id.clone(), StringFormat::Hexadecimal),
            Object::String(doc_id, StringFormat::Hexadecimal),
        ]),
    );

    let mut out = Vec::new();
    document
        .save_to(&mut out)
        .map_err(|err| RenderError::PdfBackend(err.to_string()))?;
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn push_pdf_text_run_operations(
    operations: &mut Vec<Operation>,
    text: &str,
    x_px: f64,
    y_px: f64,
    width_px: f64,
    font_size_pt: f64,
    baseline_shift_pt: f64,
    char_spacing_pt: f64,
    color: PdfColor,
    page_height_pt: f64,
    cid_map: Option<&PdfCidMap>,
    bold: bool,
    italic: bool,
) {
    push_fill_color(operations, color);
    let font_resource_name: &[u8] = if bold { b"F2" } else { b"F1" };
    operations.push(Operation::new(
        "Tf",
        vec![
            Object::Name(font_resource_name.to_vec()),
            Object::Real(font_size_pt as f32),
        ],
    ));
    let has_char_spacing = char_spacing_pt.abs() > f64::EPSILON;
    if has_char_spacing {
        operations.push(Operation::new(
            "Tc",
            vec![Object::Real(char_spacing_pt as f32)],
        ));
    }
    let scale_percent = horizontal_scale_percent(
        glyph_target_width_pt(text, px_to_pt(width_px), char_spacing_pt),
        embedded_pdf_text_width_pt(text, font_size_pt, bold),
    );
    operations.push(Operation::new(
        "Tz",
        vec![Object::Real(scale_percent as f32)],
    ));
    // The bundled family has no italic faces, so italic runs are slanted the
    // way Chromium slants fake italic: a pure horizontal shear that leaves
    // advance widths untouched (locked by jade-docx-layout
    // tests/chromium_fake_italic.rs). 0.25 is Skia's synthetic oblique
    // (SK_Scalar1/4, ~14 degrees).
    let skew: Object = if italic {
        Object::Real(SYNTHETIC_OBLIQUE_SHEAR as f32)
    } else {
        0.into()
    };
    operations.push(Operation::new(
        "Tm",
        vec![
            1.into(),
            0.into(),
            skew,
            1.into(),
            Object::Real(px_to_pt(x_px) as f32),
            Object::Real((page_height_pt - px_to_pt(y_px) + baseline_shift_pt) as f32),
        ],
    ));
    operations.push(Operation::new("Tj", vec![pdf_text_object(text, cid_map)]));
    if has_char_spacing {
        operations.push(Operation::new("Tc", vec![Object::Real(0.0)]));
    }
}

fn glyph_target_width_pt(text: &str, total_width_pt: f64, char_spacing_pt: f64) -> f64 {
    let gap_count = text.chars().count().saturating_sub(1) as f64;
    let spacing_width = char_spacing_pt * gap_count;
    (total_width_pt - spacing_width).max(0.1)
}

fn push_pdf_text_background_operations(
    operations: &mut Vec<Operation>,
    span: &RenderTextSpan,
    x_px: f64,
    y_px: f64,
    page_height_pt: f64,
) {
    let Some(color) = span.background_color else {
        return;
    };
    let left = px_to_pt(x_px);
    let width = px_to_pt(span.width);
    if width <= 0.0 {
        return;
    }
    let height = span.font_size_pt * 1.25;
    let bottom =
        page_height_pt - px_to_pt(y_px) + span.baseline_shift_pt - span.font_size_pt * 0.25;
    operations.push(Operation::new("q", vec![]));
    push_fill_color(operations, color);
    operations.push(Operation::new(
        "re",
        vec![
            Object::Real(left as f32),
            Object::Real(bottom as f32),
            Object::Real(width as f32),
            Object::Real(height as f32),
        ],
    ));
    operations.push(Operation::new("f", vec![]));
    operations.push(Operation::new("Q", vec![]));
}

fn push_pdf_text_decoration_operations(
    operations: &mut Vec<Operation>,
    span: &RenderTextSpan,
    x_px: f64,
    y_px: f64,
    page_height_pt: f64,
) {
    if !(span.underline || span.strike || span.double_strike) {
        return;
    }
    let left = px_to_pt(x_px);
    let right = px_to_pt(x_px + span.width);
    if right <= left {
        return;
    }
    let baseline = page_height_pt - px_to_pt(y_px) + span.baseline_shift_pt;
    let stroke_width = (span.font_size_pt * 0.055).clamp(0.4, 1.2);
    if span.underline {
        push_pdf_colored_line(
            operations,
            left,
            baseline - span.font_size_pt * 0.12,
            right,
            span.color,
            stroke_width,
        );
    }
    if span.strike {
        push_pdf_colored_line(
            operations,
            left,
            baseline + span.font_size_pt * 0.28,
            right,
            span.color,
            stroke_width,
        );
    }
    if span.double_strike {
        let center = baseline + span.font_size_pt * 0.28;
        let gap = (span.font_size_pt * 0.09).max(stroke_width * 1.5);
        push_pdf_colored_line(
            operations,
            left,
            center - gap,
            right,
            span.color,
            stroke_width,
        );
        push_pdf_colored_line(
            operations,
            left,
            center + gap,
            right,
            span.color,
            stroke_width,
        );
    }
}

fn push_pdf_colored_line(
    operations: &mut Vec<Operation>,
    left: f64,
    y: f64,
    right: f64,
    color: PdfColor,
    stroke_width: f64,
) {
    operations.push(Operation::new("q", vec![]));
    push_stroke_color(operations, color);
    operations.push(Operation::new("w", vec![Object::Real(stroke_width as f32)]));
    operations.push(Operation::new(
        "m",
        vec![Object::Real(left as f32), Object::Real(y as f32)],
    ));
    operations.push(Operation::new(
        "l",
        vec![Object::Real(right as f32), Object::Real(y as f32)],
    ));
    operations.push(Operation::new("S", vec![]));
    operations.push(Operation::new("Q", vec![]));
}

fn push_text_box_operations(
    operations: &mut Vec<Operation>,
    text_box: &RenderTextBox,
    page_height_pt: f64,
) {
    let x = px_to_pt(text_box.x);
    let y = page_height_pt - px_to_pt(text_box.y + text_box.height);
    let width = px_to_pt(text_box.width);
    let height = px_to_pt(text_box.height);
    operations.push(Operation::new("q", vec![]));
    if let Some(fill) = text_box.fill_color {
        push_fill_color(operations, fill);
        operations.push(Operation::new(
            "re",
            vec![
                Object::Real(x as f32),
                Object::Real(y as f32),
                Object::Real(width as f32),
                Object::Real(height as f32),
            ],
        ));
        operations.push(Operation::new("f", vec![]));
    }
    if text_box.border.style != TextBoxBorderStyle::None && text_box.border.width > 0.0 {
        operations.push(Operation::new(
            "w",
            vec![Object::Real(px_to_pt(text_box.border.width) as f32)],
        ));
        push_stroke_color(operations, text_box.border.color);
        push_border_dash(operations, &text_box.border.style);
        operations.push(Operation::new(
            "re",
            vec![
                Object::Real(x as f32),
                Object::Real(y as f32),
                Object::Real(width as f32),
                Object::Real(height as f32),
            ],
        ));
        operations.push(Operation::new("S", vec![]));
    }
    operations.push(Operation::new("Q", vec![]));
}

fn push_fill_color(operations: &mut Vec<Operation>, color: PdfColor) {
    operations.push(Operation::new(
        "rg",
        vec![
            Object::Real(color.r as f32),
            Object::Real(color.g as f32),
            Object::Real(color.b as f32),
        ],
    ));
}

fn push_stroke_color(operations: &mut Vec<Operation>, color: PdfColor) {
    operations.push(Operation::new(
        "RG",
        vec![
            Object::Real(color.r as f32),
            Object::Real(color.g as f32),
            Object::Real(color.b as f32),
        ],
    ));
}

fn push_border_dash(operations: &mut Vec<Operation>, style: &TextBoxBorderStyle) {
    let pattern = match style {
        TextBoxBorderStyle::Dashed => vec![6.into(), 3.into()],
        TextBoxBorderStyle::Dotted => vec![1.into(), 2.into()],
        TextBoxBorderStyle::DashDot => vec![6.into(), 3.into(), 1.into(), 3.into()],
        TextBoxBorderStyle::DashDotDot => {
            vec![6.into(), 3.into(), 1.into(), 3.into(), 1.into(), 3.into()]
        }
        TextBoxBorderStyle::Solid | TextBoxBorderStyle::Double | TextBoxBorderStyle::None => {
            Vec::new()
        }
    };
    operations.push(Operation::new(
        "d",
        vec![Object::Array(pattern), Object::Integer(0)],
    ));
}

fn push_table_grid_operations(
    operations: &mut Vec<Operation>,
    table: &RenderTable,
    page_height_pt: f64,
) {
    let x0 = px_to_pt(table.x);
    let y0 = page_height_pt - px_to_pt(table.y);
    let table_width = px_to_pt(table.column_widths.iter().sum::<f64>());
    let table_height = px_to_pt(table.row_heights.iter().sum::<f64>());
    operations.push(Operation::new("q", vec![]));
    operations.push(Operation::new("w", vec![Object::Real(0.5)]));
    operations.push(Operation::new("RG", vec![0.into(), 0.into(), 0.into()]));

    let mut x = x0;
    push_pdf_line(operations, x, y0, x, y0 - table_height);
    for width in &table.column_widths {
        x += px_to_pt(*width);
        push_pdf_line(operations, x, y0, x, y0 - table_height);
    }

    let mut y = y0;
    push_pdf_line(operations, x0, y, x0 + table_width, y);
    for height in &table.row_heights {
        y -= px_to_pt(*height);
        push_pdf_line(operations, x0, y, x0 + table_width, y);
    }
    operations.push(Operation::new("Q", vec![]));
}

fn push_pdf_line(operations: &mut Vec<Operation>, x1: f64, y1: f64, x2: f64, y2: f64) {
    operations.push(Operation::new(
        "m",
        vec![Object::Real(x1 as f32), Object::Real(y1 as f32)],
    ));
    operations.push(Operation::new(
        "l",
        vec![Object::Real(x2 as f32), Object::Real(y2 as f32)],
    ));
    operations.push(Operation::new("S", vec![]));
}

fn validate_pdf_output(pdf_bytes: &[u8], expected_page_count: usize) -> Result<(), RenderError> {
    let document = Document::load_mem(pdf_bytes)
        .map_err(|err| RenderError::PdfBackend(format!("generated PDF failed to parse: {err}")))?;
    let actual_page_count = document.get_pages().len();
    if actual_page_count != expected_page_count {
        return Err(RenderError::PdfBackend(format!(
            "generated PDF page count mismatch: expected {expected_page_count}, got {actual_page_count}"
        )));
    }
    document
        .trailer
        .get(b"Root")
        .map_err(|err| RenderError::PdfBackend(format!("generated PDF missing Root: {err}")))?;
    document
        .trailer
        .get(b"Info")
        .map_err(|err| RenderError::PdfBackend(format!("generated PDF missing Info: {err}")))?;
    Ok(())
}

/// Builds a deterministic 16-byte document identifier from the document's
/// structural fingerprint. Deterministic (clock-free) so identical input
/// produces identical PDF bytes, keeping hash-based regression checks valid.
fn deterministic_doc_id(input_size: usize, line_count: usize, page_count: usize) -> Vec<u8> {
    let digest = |salt: u64| -> [u8; 8] {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325 ^ salt;
        for value in [input_size as u64, line_count as u64, page_count as u64] {
            hash ^= value;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash.to_be_bytes()
    };
    let mut id = Vec::with_capacity(16);
    id.extend_from_slice(&digest(0xa5a5_a5a5_a5a5_a5a5));
    id.extend_from_slice(&digest(0x5a5a_5a5a_5a5a_5a5a));
    id
}

/// Emits a minimal, valid XMP metadata packet for the document catalog. Kept
/// clock-free for deterministic output. Note: this does not stamp PDF/A
/// identification (`pdfaid`) because full PDF/A conformance additionally
/// requires an OutputIntent with an embedded ICC profile, which is not yet
/// emitted (tracked in the renderer roadmap).
fn xmp_metadata_packet() -> Vec<u8> {
    format!(
        "<?xpacket begin=\"{bom}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n\
 <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
  <rdf:Description rdf:about=\"\"\n\
   xmlns:dc=\"http://purl.org/dc/elements/1.1/\"\n\
   xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\"\n\
   xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\">\n\
   <dc:format>application/pdf</dc:format>\n\
   <dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">DOCX render</rdf:li></rdf:Alt></dc:title>\n\
   <pdf:Producer>jade-docx-pdf-renderer</pdf:Producer>\n\
   <xmp:CreatorTool>jade-docx-pdf-renderer</xmp:CreatorTool>\n\
  </rdf:Description>\n\
 </rdf:RDF>\n\
</x:xmpmeta>\n\
<?xpacket end=\"w\"?>",
        bom = '\u{feff}'
    )
    .into_bytes()
}

fn px_to_pt(px: f64) -> f64 {
    px / CSS_PX_PER_PT
}

fn identity_to_unicode_cmap(cid_map: Option<&PdfCidMap>) -> Vec<u8> {
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def
/CMapName /Adobe-Identity-UCS def
/CMapType 2 def
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
248 beginbfrange
",
    );
    for high in (0_u16..=255).filter(|high| !(0xd8..=0xdf).contains(high)) {
        use std::fmt::Write;
        let start = high << 8;
        let end = start | 0x00ff;
        writeln!(cmap, "<{start:04X}> <{end:04X}> <{start:04X}>")
            .expect("writing to a String cannot fail");
    }
    cmap.push_str("endbfrange\n");
    if let Some(cid_map) = cid_map {
        for entries in cid_map
            .cid_to_non_bmp
            .iter()
            .collect::<Vec<_>>()
            .chunks(100)
        {
            use std::fmt::Write;
            writeln!(cmap, "{} beginbfchar", entries.len())
                .expect("writing to a String cannot fail");
            for (cid, ch) in entries {
                let mut utf16 = [0_u16; 2];
                let encoded = ch.encode_utf16(&mut utf16);
                write!(cmap, "<{cid:04X}> <").expect("writing to a String cannot fail");
                for unit in encoded {
                    write!(cmap, "{unit:04X}").expect("writing to a String cannot fail");
                }
                cmap.push_str(">\n");
            }
            cmap.push_str("endbfchar\n");
        }
    }
    cmap.push_str(
        "endcmap
CMapName currentdict /CMap defineresource pop
end
end",
    );
    cmap.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_ascii_hex_color_is_rejected_without_panicking() {
        assert!(parse_hex_color("ああ").is_none());
        assert!(parse_hex_color("12GG56").is_none());
        assert!(parse_hex_color("#123456").is_some());
    }

    #[test]
    fn to_unicode_ranges_only_vary_the_low_byte() {
        let cmap = String::from_utf8(identity_to_unicode_cmap(None)).expect("ASCII CMap");
        assert!(!cmap.contains("<0000> <FFFF> <0000>"));
        assert!(cmap.contains("<0000> <00FF> <0000>"));
        assert!(cmap.contains("<FF00> <FFFF> <FF00>"));
    }

    #[test]
    fn header_footer_page_fields_replace_cached_results() {
        let xml = r#"<w:hdr xmlns:w="urn:test"><w:p>
          <w:fldSimple w:instr=" PAGE "><w:r><w:t>99</w:t></w:r></w:fldSimple>
          <w:r><w:t>/</w:t></w:r>
          <w:r><w:fldChar w:fldCharType="begin"/></w:r>
          <w:r><w:instrText> PAGE </w:instrText></w:r>
          <w:r><w:fldChar w:fldCharType="separate"/></w:r>
          <w:r><w:t>88</w:t></w:r>
          <w:r><w:fldChar w:fldCharType="end"/></w:r>
        </w:p></w:hdr>"#;

        assert_eq!(extract_header_footer_text(xml, 7), "7/7");
    }

    fn jpeg_with_components(components: u8, adobe: bool) -> Vec<u8> {
        let mut bytes = vec![0xff, 0xd8];
        if adobe {
            bytes.extend_from_slice(&[
                0xff, 0xee, 0x00, 0x0e, b'A', b'd', b'o', b'b', b'e', 0, 100, 0, 0, 0, 0, 0,
            ]);
        }
        bytes.extend_from_slice(&[
            0xff, 0xc0, 0x00, 0x08, 0x08, 0x00, 0x02, 0x00, 0x03, components,
        ]);
        bytes.extend_from_slice(&[0xff, 0xd9]);
        bytes
    }

    #[test]
    fn jpeg_metadata_preserves_gray_rgb_and_cmyk_color_spaces() {
        assert_eq!(
            jpeg_metadata(&jpeg_with_components(1, false)),
            Some((3, 2, JpegColorSpace::Gray))
        );
        assert_eq!(
            jpeg_metadata(&jpeg_with_components(3, false)),
            Some((3, 2, JpegColorSpace::Rgb))
        );
        assert_eq!(
            jpeg_metadata(&jpeg_with_components(4, false)),
            Some((3, 2, JpegColorSpace::Cmyk { inverted: false }))
        );
        assert_eq!(
            jpeg_metadata(&jpeg_with_components(4, true)),
            Some((3, 2, JpegColorSpace::Cmyk { inverted: true }))
        );
    }

    #[test]
    fn default_options_are_pdf_a_with_embedded_fonts() {
        let options = RenderOptions::default();
        assert_eq!(options.conformance, PdfConformance::PdfA2u);
        assert!(options.embed_fonts);
        assert_eq!(options.raster_dpi, 144);
        assert!(!options.fail_on_warnings);
    }

    #[test]
    fn pipeline_has_expected_stage_order() {
        assert_eq!(
            default_pipeline(),
            [
                PipelineStage::ParseDocx,
                PipelineStage::BuildLayout,
                PipelineStage::ResolveFonts,
                PipelineStage::DrawPdf,
                PipelineStage::ValidateOutput
            ]
        );
    }

    #[test]
    fn embedded_font_name_tags_subsets_and_passes_through_full_font() {
        let full = embedded_font_name(EMBEDDED_FONT_NAME, &[0, 1, 2], false);
        assert_eq!(full, EMBEDDED_FONT_NAME);

        let subset_name = embedded_font_name(EMBEDDED_FONT_NAME, &[0, 5, 9], true);
        let (tag, base) = subset_name
            .split_once('+')
            .expect("subset name should carry a tag prefix");
        assert_eq!(base, EMBEDDED_FONT_NAME);
        assert_eq!(tag.len(), 6, "tag must be 6 letters: {tag:?}");
        assert!(
            tag.bytes().all(|b| b.is_ascii_uppercase()),
            "tag must be uppercase letters: {tag:?}"
        );
        assert_eq!(
            subset_name,
            embedded_font_name(EMBEDDED_FONT_NAME, &[0, 5, 9], true)
        );
        assert_ne!(
            subset_name,
            embedded_font_name(EMBEDDED_FONT_NAME, &[0, 5, 10], true)
        );
        assert!(
            embedded_font_name(EMBEDDED_BOLD_FONT_NAME, &[0, 5, 9], true)
                .ends_with(EMBEDDED_BOLD_FONT_NAME)
        );
    }

    #[test]
    fn deterministic_doc_id_is_stable_and_sixteen_bytes() {
        let a = deterministic_doc_id(1024, 10, 2);
        assert_eq!(a, deterministic_doc_id(1024, 10, 2));
        assert_eq!(a.len(), 16);
        assert_ne!(a, deterministic_doc_id(2048, 10, 2));
    }

    #[test]
    fn missing_glyph_falls_back_to_notdef() {
        let font = EmbeddedFontProgram::parse().expect("font should parse");
        // A Private Use Area code point is absent from the bundled Japanese font
        // but is still a valid BMP character.
        let pua = '\u{E000}';
        assert_eq!(font.glyph_id_for_char(pua), 0, "uncovered char -> .notdef");
        assert_eq!(font.width_for_char(pua), DEFAULT_GLYPH_WIDTH);
        // A covered character resolves to a real glyph.
        assert_ne!(font.glyph_id_for_char('A'), 0);
    }

    #[test]
    fn non_bmp_text_uses_one_cid_instead_of_surrogate_cids() {
        let mut cid_map = PdfCidMap::default();
        cid_map.non_bmp_to_cid.insert('😀', 0xd800);
        cid_map.cid_to_non_bmp.insert(0xd800, '😀');
        let Object::String(bytes, StringFormat::Hexadecimal) =
            pdf_text_object("😀", Some(&cid_map))
        else {
            panic!("hexadecimal PDF text string");
        };
        assert_eq!(bytes.len(), 2, "one Unicode scalar must map to one CID");
        assert_eq!(bytes, 0xd800_u16.to_be_bytes());
        let cmap = String::from_utf8(identity_to_unicode_cmap(Some(&cid_map))).expect("CMap");
        assert!(cmap.contains("<D800> <D83DDE00>"), "{cmap}");
    }

    #[test]
    fn embedded_font_is_subset_and_compressed_far_below_full_font() {
        let input = include_bytes!("../../../public/fixtures/yokohama_page6.docx");
        let pdf =
            render_docx_to_pdf(input, &RenderOptions::default()).expect("render should succeed");
        // A multi-page Japanese document embedding the full bundled font would
        // exceed its raw multi-megabyte size; subsetting + Flate compression
        // must keep the whole PDF well under 1 MB.
        assert!(
            pdf.len() < 1_000_000,
            "subset PDF should be well under 1 MB, got {} bytes",
            pdf.len()
        );
        assert!(
            pdf.len() < EMBEDDED_FONT_BYTES.len(),
            "subset PDF ({} bytes) should be smaller than the full font ({} bytes)",
            pdf.len(),
            EMBEDDED_FONT_BYTES.len()
        );

        let document = Document::load_mem(&pdf).expect("pdf should parse");
        assert!(
            document.trailer.get(b"ID").is_ok(),
            "trailer should carry an /ID"
        );
        let has_subset_basefont = document.objects.values().any(|object| {
            object
                .as_dict()
                .ok()
                .and_then(|dict| dict.get(b"BaseFont").ok())
                .and_then(|base| base.as_name().ok())
                .map(String::from_utf8_lossy)
                .is_some_and(|name| name.ends_with(EMBEDDED_FONT_NAME) && name.contains('+'))
        });
        assert!(
            has_subset_basefont,
            "embedded font BaseFont should carry a subset tag prefix"
        );
    }

    #[test]
    fn browser_parity_measurement_applies_kerning_and_punctuation_trimming() {
        let embedded = PdfLayoutContext::new(TextMeasurement::EmbeddedFontAdvances);
        let shaping = PdfLayoutContext::new(TextMeasurement::BrowserParityShaping);
        let descriptor = "10.50pt 'Noto Sans JP'";
        let em = 14.0_f64; // 10.5pt -> 14px

        // Raw advances cannot collapse adjacent full-width punctuation; the
        // browser does, and so must the parity backend.
        let embedded_pair = embedded
            .measure_text("\u{300d}\u{300c}", descriptor)
            .unwrap();
        let shaping_pair = shaping
            .measure_text("\u{300d}\u{300c}", descriptor)
            .unwrap();
        assert!(
            (embedded_pair - 2.0 * em).abs() < 1e-6,
            "got {embedded_pair}"
        );
        assert!((shaping_pair - 1.5 * em).abs() < 1e-6, "got {shaping_pair}");

        // Latin kerning is likewise invisible to raw advances.
        let embedded_av = embedded.measure_text("AV", descriptor).unwrap();
        let shaping_av = shaping.measure_text("AV", descriptor).unwrap();
        assert!(
            shaping_av < embedded_av - 1e-6,
            "AV should kern: {shaping_av} vs {embedded_av}"
        );
    }

    #[test]
    fn both_measurement_backends_measure_bold_with_the_bold_face() {
        let text = "Hello fluffy 1,000";
        let regular_descriptor = "10.50pt 'Noto Sans JP'";
        let bold_descriptor = "bold 10.50pt 'Noto Sans JP'";
        for measurement in [
            TextMeasurement::EmbeddedFontAdvances,
            TextMeasurement::BrowserParityShaping,
        ] {
            let context = PdfLayoutContext::new(measurement);
            let regular = context.measure_text(text, regular_descriptor).unwrap();
            let bold = context.measure_text(text, bold_descriptor).unwrap();
            assert!(
                (regular - bold).abs() > 0.01,
                "{measurement:?}: bold runs are painted with the embedded bold \
                 face, so they must wrap on its advances (regular={regular}, \
                 bold={bold})"
            );
        }
    }

    #[test]
    fn text_measurement_defaults_to_the_existing_embedded_advances() {
        assert_eq!(
            RenderOptions::default().text_measurement,
            TextMeasurement::EmbeddedFontAdvances
        );
    }

    #[test]
    fn browser_parity_measurement_still_renders_the_fixture() {
        let docx = include_bytes!("../../../public/fixtures/hello.docx");
        let options = RenderOptions {
            text_measurement: TextMeasurement::BrowserParityShaping,
            ..RenderOptions::default()
        };

        let pdf = render_docx_to_pdf(docx, &options).expect("render with shaping measurement");

        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn renders_pdf_for_fixture_docx() {
        let input = include_bytes!("../../../public/fixtures/hello.docx");
        let result =
            render_docx_to_pdf(input, &RenderOptions::default()).expect("render should succeed");
        let text = String::from_utf8_lossy(&result);
        let document = Document::load_mem(&result).expect("pdf should parse");

        assert!(text.starts_with("%PDF-"));
        assert!(!document.get_pages().is_empty());
        assert!(document.trailer.get(b"Root").is_ok());
        assert!(document.trailer.get(b"Info").is_ok());
    }

    #[test]
    fn rejects_empty_input() {
        let result = render_docx_to_pdf(&[], &RenderOptions::default());
        assert_eq!(result, Err(RenderError::EmptyInput));
    }

    #[test]
    fn page_count_increases_with_many_lines() {
        let lines = (0..95)
            .map(|idx| RenderLine {
                page: (idx / 40) + 1,
                text: format!("line-{idx}"),
                x: 72.0,
                y: 72.0 + idx as f64 * 18.0,
                baseline_y: 84.8 + idx as f64 * 18.0,
                width: 100.0,
                spans: Vec::new(),
                source: RenderLineSource::Body,
            })
            .collect::<Vec<_>>();
        let result = build_pdf_bytes(
            &RenderLayout {
                lines,
                images: Vec::new(),
                tables: Vec::new(),
                text_boxes: Vec::new(),
                links: Vec::new(),
                rendered_header_footer_parts: 0,
                rendered_notes: 0,
                rendered_math: 0,
                page_width: 595.0 * CSS_PX_PER_PT,
                page_height: 842.0 * CSS_PX_PER_PT,
                page_sizes: Vec::new(),
                used_jade_docx_layout: true,
            },
            1024,
            &RenderOptions::default(),
        )
        .expect("build pdf");
        let document = Document::load_mem(&result).expect("pdf should parse");
        assert_eq!(document.get_pages().len(), 3);
    }

    #[test]
    fn parses_font_size_from_descriptor() {
        assert_eq!(
            parse_font_size_pt("bold italic 10.5pt Noto Sans"),
            Some(10.5)
        );
        assert_eq!(parse_font_size_pt("Noto Sans"), None);
    }

    #[test]
    fn normalize_pdf_text_does_not_truncate_long_lines() {
        let input = "a".repeat(256);
        assert_eq!(normalize_pdf_text(&input), input);
    }

    #[test]
    fn build_pdf_emits_external_uri_link_annotations() {
        let pdf = build_pdf_bytes(
            &RenderLayout {
                lines: vec![RenderLine {
                    page: 1,
                    text: "example.com".to_string(),
                    x: 72.0,
                    y: 72.0,
                    baseline_y: 84.8,
                    width: 90.0,
                    spans: Vec::new(),
                    source: RenderLineSource::Body,
                }],
                images: Vec::new(),
                tables: Vec::new(),
                text_boxes: Vec::new(),
                links: vec![RenderLink {
                    page: 1,
                    x: 72.0,
                    y: 72.0,
                    width: 90.0,
                    height: 18.0,
                    target: "https://example.com".to_string(),
                }],
                rendered_header_footer_parts: 0,
                rendered_notes: 0,
                rendered_math: 0,
                page_width: 595.0,
                page_height: 842.0,
                page_sizes: Vec::new(),
                used_jade_docx_layout: true,
            },
            1024,
            &RenderOptions::default(),
        )
        .expect("build pdf");
        let document = Document::load_mem(&pdf).expect("pdf should parse");
        let mut found_uri = false;
        for object in document.objects.values() {
            let Ok(dict) = object.as_dict() else {
                continue;
            };
            if dict
                .get(b"Subtype")
                .ok()
                .and_then(|object| object.as_name().ok())
                != Some(b"Link".as_slice())
            {
                continue;
            }
            let action = dict
                .get(b"A")
                .and_then(Object::as_dict)
                .expect("link action dictionary");
            let uri = action
                .get(b"URI")
                .and_then(Object::as_str)
                .expect("link uri");
            found_uri = uri == b"https://example.com";
        }

        assert!(found_uri);
    }

    #[test]
    fn render_report_surfaces_viewer_font_warnings_when_embedding_is_disabled() {
        let layout = RenderLayout {
            lines: vec![RenderLine {
                page: 1,
                text: "契約ABC".to_string(),
                x: 0.0,
                y: 0.0,
                baseline_y: 12.8,
                width: 42.0,
                spans: Vec::new(),
                source: RenderLineSource::Body,
            }],
            images: Vec::new(),
            tables: Vec::new(),
            text_boxes: Vec::new(),
            links: Vec::new(),
            rendered_header_footer_parts: 0,
            rendered_notes: 0,
            rendered_math: 0,
            page_width: 595.0,
            page_height: 842.0,
            page_sizes: Vec::new(),
            used_jade_docx_layout: true,
        };
        let options = RenderOptions {
            embed_fonts: false,
            ..RenderOptions::default()
        };
        let report = build_render_report(&layout, &ReadinessFeatures::default(), &options);

        assert_eq!(report.pages, 1);
        assert_eq!(report.rendered_lines, 1);
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.starts_with("PDF_FONT_NOT_EMBEDDED"))
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.starts_with("PDF_NON_ASCII_TEXT_USES_VIEWER_FONT"))
        );
    }

    #[test]
    fn fail_on_warnings_allows_text_only_embedded_font_output() {
        let input = include_bytes!("../../../public/fixtures/tab.docx");
        let options = RenderOptions {
            fail_on_warnings: true,
            ..RenderOptions::default()
        };
        let (_, report) = render_docx_to_pdf_with_report(input, &options)
            .expect("embedded text-only fixture should pass strict readiness");

        assert!(
            report.warnings.is_empty(),
            "text-only embedded font fixture should have no warnings: {:?}",
            report.warnings
        );
    }

    #[test]
    fn fail_on_warnings_rejects_disabled_font_embedding() {
        let input = include_bytes!("../../../public/fixtures/hello.docx");
        let options = RenderOptions {
            embed_fonts: false,
            fail_on_warnings: true,
            ..RenderOptions::default()
        };
        let err = render_docx_to_pdf_with_report(input, &options)
            .expect_err("disabled font embedding should fail strict mode");

        assert!(
            matches!(err, RenderError::ReadinessWarnings(warnings) if warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_FONT_NOT_EMBEDDED")))
        );
    }

    #[test]
    fn render_report_surfaces_unsupported_feature_warnings() {
        let layout = RenderLayout {
            lines: vec![RenderLine {
                page: 1,
                text: "table image math".to_string(),
                x: 0.0,
                y: 0.0,
                baseline_y: 12.8,
                width: 42.0,
                spans: Vec::new(),
                source: RenderLineSource::Body,
            }],
            images: Vec::new(),
            tables: Vec::new(),
            text_boxes: Vec::new(),
            links: Vec::new(),
            rendered_header_footer_parts: 1,
            rendered_notes: 0,
            rendered_math: 0,
            page_width: 595.0,
            page_height: 842.0,
            page_sizes: Vec::new(),
            used_jade_docx_layout: true,
        };
        let readiness = ReadinessFeatures {
            tables: 1,
            images: 2,
            text_boxes: 3,
            math: 4,
            headers_or_footers: 2,
            line_numbering_sections: 6,
            comments: 7,
            revisions: 8,
            hyperlinks: 9,
            unsupported_fields: 10,
            unsupported_run_styling: 11,
            notes: 5,
            requested_font_names: BTreeSet::from(["MS Mincho".to_string()]),
        };
        let report = build_render_report(&layout, &readiness, &RenderOptions::default());

        for prefix in [
            "PDF_REQUESTED_FONTS_NOT_EMBEDDED",
            "PDF_UNSUPPORTED_TABLE_DRAWING",
            "PDF_UNSUPPORTED_IMAGE_DRAWING",
            "PDF_UNSUPPORTED_TEXT_BOX_DRAWING",
            "PDF_UNSUPPORTED_MATH_DRAWING",
            "PDF_UNSUPPORTED_HEADER_FOOTER_DRAWING",
            "PDF_UNSUPPORTED_LINE_NUMBERING",
            "PDF_UNSUPPORTED_COMMENT_RENDERING",
            "PDF_UNSUPPORTED_REVISION_MARKUP",
            "PDF_UNSUPPORTED_HYPERLINK_ANNOTATION",
            "PDF_UNSUPPORTED_FIELD_RENDERING",
            "PDF_UNSUPPORTED_RUN_STYLING",
            "PDF_UNSUPPORTED_NOTE_DRAWING",
        ] {
            assert!(
                report
                    .warnings
                    .iter()
                    .any(|warning| warning.starts_with(prefix)),
                "missing warning prefix {prefix}: {:?}",
                report.warnings
            );
        }
    }

    fn test_run(
        text: &str,
        properties: Option<jade_docx_reader::RunProperties>,
    ) -> jade_docx_reader::Run {
        jade_docx_reader::Run {
            text: text.to_string(),
            properties,
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

    #[test]
    fn glyph_target_width_subtracts_pdf_character_spacing_gaps() {
        assert!((glyph_target_width_pt("Hello", 30.0, 2.0) - 22.0).abs() <= f64::EPSILON);
        assert!((glyph_target_width_pt("H", 30.0, 2.0) - 30.0).abs() <= f64::EPSILON);
        assert!((glyph_target_width_pt("Hello", 3.0, 2.0) - 0.1).abs() <= f64::EPSILON);
    }

    #[test]
    fn pdf_text_width_uses_embedded_font_advances() {
        let face = ttf_parser::Face::parse(EMBEDDED_FONT_BYTES, 0).expect("embedded font");
        let units_per_em = f64::from(face.units_per_em());
        let expected = "Hello"
            .chars()
            .map(|ch| {
                face.glyph_index(ch)
                    .and_then(|glyph| face.glyph_hor_advance(glyph))
                    .map(f64::from)
                    .unwrap_or(units_per_em)
            })
            .sum::<f64>()
            / units_per_em
            * 12.0;

        assert!(
            (embedded_pdf_text_width_pt("Hello", 12.0, false) - expected).abs() < 0.001,
            "PDF width must use the embedded font's real advances"
        );
    }

    #[test]
    fn bold_pdf_text_width_uses_the_bold_face_advances() {
        let regular = embedded_pdf_text_width_pt("Hello fluffy 1,000", 12.0, false);
        let bold = embedded_pdf_text_width_pt("Hello fluffy 1,000", 12.0, true);
        assert!(
            (regular - bold).abs() > 0.01,
            "the real bold face has its own Latin advances (regular={regular}, bold={bold})"
        );

        // CJK glyphs are full-width in both faces, so the em-square advances
        // must stay identical — bold must not reflow Japanese body text.
        let regular_cjk = embedded_pdf_text_width_pt("秘密保持契約", 12.0, false);
        let bold_cjk = embedded_pdf_text_width_pt("秘密保持契約", 12.0, true);
        assert!(
            (regular_cjk - bold_cjk).abs() < 1e-9,
            "full-width CJK advances must match between faces"
        );
    }

    #[test]
    fn font_descriptor_bold_detection_reads_keyword_and_numeric_weights() {
        assert!(font_descriptor_requests_bold("bold 10.50pt 'Noto Sans JP'"));
        assert!(font_descriptor_requests_bold("bold italic 12pt serif"));
        assert!(font_descriptor_requests_bold("700 12pt 'Noto Sans JP'"));
        assert!(!font_descriptor_requests_bold("12pt 'Noto Sans JP'"));
        assert!(!font_descriptor_requests_bold("400 12pt 'Noto Sans JP'"));
        // A size token ends the weight prefix: family names after the size
        // must not be misread as weights.
        assert!(!font_descriptor_requests_bold("12pt 'Bold Ideas'"));
    }

    #[test]
    fn note_positions_stay_inside_page_and_spill_forward() {
        let mut counts = BTreeMap::new();
        let y_start = 700.0;
        let min_y = 100.0;
        let positions = (0..100)
            .map(|_| allocate_note_position(&mut counts, 2, y_start, min_y))
            .collect::<Vec<_>>();

        assert!(positions.iter().all(|(_, y)| *y >= min_y && *y <= y_start));
        assert!(positions.iter().all(|(page, _)| *page >= 2));
        assert!(
            positions.iter().any(|(page, _)| *page > 2),
            "overflowing notes should continue on a later page"
        );
    }

    #[test]
    fn run_visual_styling_scanner_counts_only_visible_unsupported_direct_styling() {
        let mut features = ReadinessFeatures::default();
        let styled_run = test_run(
            "styled",
            Some(jade_docx_reader::RunProperties {
                caps: Some(true),
                color: Some("#FF0000".into()),
                ..jade_docx_reader::RunProperties::default()
            }),
        );
        let default_like_run = test_run(
            "plain",
            Some(jade_docx_reader::RunProperties {
                bold: Some(false),
                font_size: Some(12.0),
                color: Some("#000000".into()),
                ..jade_docx_reader::RunProperties::default()
            }),
        );
        // Bold is painted with the real embedded bold face, and italic as a
        // synthetic oblique — neither counts as unsupported styling anymore.
        let bold_run = test_run(
            "bold",
            Some(jade_docx_reader::RunProperties {
                bold: Some(true),
                bold_complex: Some(true),
                italic: Some(true),
                italic_complex: Some(true),
                ..jade_docx_reader::RunProperties::default()
            }),
        );
        let supported_style_run = test_run(
            "colored",
            Some(jade_docx_reader::RunProperties {
                font_size: Some(16.0),
                color: Some("#FF0000".into()),
                highlight: Some("yellow".into()),
                char_spacing: Some(2.0),
                text_scale: Some(80.0),
                baseline: Some("superscript".into()),
                text_position: Some(2.0),
                ..jade_docx_reader::RunProperties::default()
            }),
        );
        let hidden_empty_run = test_run(
            " ",
            Some(jade_docx_reader::RunProperties {
                underline: Some(true),
                ..jade_docx_reader::RunProperties::default()
            }),
        );

        scan_run_visual_styling(&styled_run, &mut features);
        scan_run_visual_styling(&default_like_run, &mut features);
        scan_run_visual_styling(&bold_run, &mut features);
        scan_run_visual_styling(&supported_style_run, &mut features);
        scan_run_visual_styling(&hidden_empty_run, &mut features);

        assert_eq!(features.unsupported_run_styling, 1);
    }

    #[test]
    fn field_instruction_scanner_counts_only_unsupported_fields() {
        let xml = r#"
            <w:p>
              <w:r><w:instrText> PAGE </w:instrText></w:r>
              <w:r><w:instrText> TOC \o "1-3" \h </w:instrText></w:r>
              <w:fldSimple w:instr=" AUTONUM  \* Arabic "/>
              <w:fldSimple w:instr=" SEQ Figure "/>
            </w:p>
        "#;

        assert_eq!(count_unsupported_field_instructions(xml), 2);
    }

    #[test]
    fn table_grid_renderer_rejects_merged_or_nested_tables() {
        let simple = DocxTable {
            rows: vec![
                jade_docx_reader::TableRow {
                    cells: vec![
                        jade_docx_reader::TableCell::default(),
                        jade_docx_reader::TableCell::default(),
                    ],
                    ..jade_docx_reader::TableRow::default()
                },
                jade_docx_reader::TableRow {
                    cells: vec![
                        jade_docx_reader::TableCell::default(),
                        jade_docx_reader::TableCell::default(),
                    ],
                    ..jade_docx_reader::TableRow::default()
                },
            ],
            ..DocxTable::default()
        };
        let merged = DocxTable {
            rows: vec![jade_docx_reader::TableRow {
                cells: vec![jade_docx_reader::TableCell {
                    span: Some(2),
                    ..jade_docx_reader::TableCell::default()
                }],
                ..jade_docx_reader::TableRow::default()
            }],
            ..DocxTable::default()
        };
        let nested = DocxTable {
            rows: vec![jade_docx_reader::TableRow {
                cells: vec![jade_docx_reader::TableCell {
                    blocks: vec![BodyElement::Table(Box::default())],
                    ..jade_docx_reader::TableCell::default()
                }],
                ..jade_docx_reader::TableRow::default()
            }],
            ..DocxTable::default()
        };

        assert!(render_grid_for_top_level_table(&simple, 1, 0.0, 0.0, 100.0).is_some());
        assert!(render_grid_for_top_level_table(&merged, 1, 0.0, 0.0, 100.0).is_none());
        assert!(render_grid_for_top_level_table(&nested, 1, 0.0, 0.0, 100.0).is_none());
    }

    #[test]
    fn text_box_renderer_rejects_nested_tables_and_math() {
        let simple = DocxTextBox {
            text: "Hello".to_string(),
            blocks: vec![BodyElement::Paragraph(Box::new(DocxParagraph {
                runs: Vec::new(),
                properties: None,
                node_id: None,
                comment_ranges: Vec::new(),
                comment_markers: Vec::new(),
            }))],
            ..DocxTextBox::default()
        };
        let nested_table = DocxTextBox {
            text: "Hello".to_string(),
            blocks: vec![BodyElement::Table(Box::default())],
            ..DocxTextBox::default()
        };
        let math = DocxTextBox {
            text: "Hello".to_string(),
            blocks: vec![BodyElement::Math(Box::default())],
            ..DocxTextBox::default()
        };

        assert!(is_simple_renderable_text_box(&simple));
        assert!(!is_simple_renderable_text_box(&nested_table));
        assert!(!is_simple_renderable_text_box(&math));
    }

    #[test]
    fn jpeg_dimension_scanner_reads_start_of_frame_size() {
        let bytes = [
            0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01, 0x01, 0x00,
            0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x00, 0x07, 0x00,
            0x0b, 0x03, 0x01, 0x11, 0x00, 0x02, 0x11, 0x00, 0x03, 0x11, 0x00, 0xff, 0xd9,
        ];

        assert_eq!(jpeg_metadata(&bytes), Some((11, 7, JpegColorSpace::Rgb)));
        assert_eq!(jpeg_metadata(&bytes[..10]), None);
        assert_eq!(jpeg_metadata(b"not jpeg"), None);
    }

    #[test]
    fn note_plain_text_rejects_complex_note_bodies() {
        let paragraph_note = DocxNote {
            id: "1".to_string(),
            note_type: None,
            body: vec![BodyElement::Paragraph(Box::new(DocxParagraph {
                runs: vec![jade_docx_reader::Run {
                    text: " Hello note ".to_string(),
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
                properties: None,
                node_id: None,
                comment_ranges: Vec::new(),
                comment_markers: Vec::new(),
            }))],
        };
        let table_note = DocxNote {
            id: "2".to_string(),
            note_type: None,
            body: vec![BodyElement::Table(Box::default())],
        };

        assert_eq!(
            note_plain_text(&paragraph_note).as_deref(),
            Some("Hello note")
        );
        assert!(note_plain_text(&table_note).is_none());
    }

    #[test]
    fn validate_pdf_output_rejects_invalid_bytes() {
        let err = validate_pdf_output(b"not a pdf", 1).expect_err("invalid bytes should fail");
        assert!(
            matches!(err, RenderError::PdfBackend(message) if message.contains("failed to parse"))
        );
    }

    #[test]
    fn validate_pdf_output_rejects_page_count_mismatch() {
        let lines = vec![RenderLine {
            page: 1,
            text: "hello".to_string(),
            x: 72.0,
            y: 72.0,
            baseline_y: 84.8,
            width: 42.0,
            spans: Vec::new(),
            source: RenderLineSource::Body,
        }];
        let pdf = build_pdf_bytes(
            &RenderLayout {
                lines,
                images: Vec::new(),
                tables: Vec::new(),
                text_boxes: Vec::new(),
                links: Vec::new(),
                rendered_header_footer_parts: 0,
                rendered_notes: 0,
                rendered_math: 0,
                page_width: 595.0,
                page_height: 842.0,
                page_sizes: Vec::new(),
                used_jade_docx_layout: true,
            },
            1024,
            &RenderOptions::default(),
        )
        .expect("build pdf");
        let err = validate_pdf_output(&pdf, 2).expect_err("wrong page count should fail");
        assert!(
            matches!(err, RenderError::PdfBackend(message) if message.contains("page count mismatch"))
        );
    }
}
