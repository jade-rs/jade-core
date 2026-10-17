use futures::executor::block_on;
use jade_docx_layout::font_resolver::{ResolvedRunStyle, resolve_run_style_for_wrap};
use jade_docx_layout::line_wrap::LineWrapContext;
use jade_docx_layout::text_extract::layout_docx_lines_from_bytes;
use jade_docx_layout::units::points_to_px;
use jade_docx_pdf_renderer::{RenderOptions, render_docx_to_pdf, render_docx_to_pdf_with_report};
use lopdf::Document;
use lopdf::content::Content;
use std::convert::Infallible;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::PathBuf;
use std::rc::Rc;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

const CSS_PX_PER_PT: f64 = 96.0 / 72.0;
const TEST_JPEG_1X1_RGB: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x00, 0x01, 0x00, 0x01, 0x03, 0x01, 0x11,
    0x00, 0x02, 0x11, 0x00, 0x03, 0x11, 0x00, 0xff, 0xd9,
];

#[derive(Clone, Copy)]
struct TestLayoutContext;

impl LineWrapContext for TestLayoutContext {
    type Error = Infallible;

    fn resolve_run_style(
        &self,
        properties: Option<&jade_docx_reader::RunProperties>,
    ) -> Result<Rc<ResolvedRunStyle>, Self::Error> {
        Ok(resolve_run_style_for_wrap(properties, None))
    }

    fn measure_text(&self, text: &str, font_descriptor: &str) -> Result<f64, Self::Error> {
        let size_pt = font_descriptor
            .split_whitespace()
            .find_map(|token| token.strip_suffix("pt")?.parse::<f32>().ok())
            .unwrap_or(12.0);
        let size_px = points_to_px(size_pt);
        let weighted_chars = text
            .chars()
            .map(|ch| if ch.is_ascii() { 0.55 } else { 1.0 })
            .sum::<f64>();
        Ok((weighted_chars * size_px).max(0.0))
    }
}

#[derive(Debug)]
struct PdfTextPosition {
    page: usize,
    text: String,
    x_pt: f64,
    y_pt: f64,
    font_size_pt: f64,
    fill_color: (f64, f64, f64),
    scale_percent: f64,
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures")
        .join(name)
}

fn parse_document(pdf_bytes: &[u8]) -> Document {
    Document::load_mem(pdf_bytes).expect("pdf should parse")
}

fn parse_page_count(pdf_bytes: &[u8]) -> usize {
    parse_document(pdf_bytes).get_pages().len()
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
    out.chars().take(160).collect()
}

fn decode_pdf_text_bytes(bytes: &[u8]) -> String {
    let units = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    String::from_utf16(&units).expect("pdf text should be UTF-16BE")
}

fn extract_pdf_text_positions(pdf_bytes: &[u8]) -> Vec<PdfTextPosition> {
    let document = parse_document(pdf_bytes);
    let mut positions = Vec::new();
    for (page_number, page_id) in document.get_pages() {
        let content_bytes = document
            .get_page_content(page_id)
            .expect("read page content stream");
        let content = Content::decode(&content_bytes).expect("decode page content stream");
        let mut current_x = 0.0;
        let mut current_y = 0.0;
        let mut current_font_size = 12.0;
        let mut current_fill_color = (0.0, 0.0, 0.0);
        let mut current_scale = 100.0;
        for operation in content.operations {
            match operation.operator.as_str() {
                "rg" => {
                    current_fill_color = (
                        operation.operands[0].as_float().expect("rg r") as f64,
                        operation.operands[1].as_float().expect("rg g") as f64,
                        operation.operands[2].as_float().expect("rg b") as f64,
                    );
                }
                "Tf" => {
                    current_font_size = operation.operands[1].as_float().expect("Tf size") as f64;
                }
                "Tz" => {
                    current_scale = operation.operands[0].as_float().expect("Tz scale") as f64;
                }
                "Tm" => {
                    current_x = operation.operands[4].as_float().expect("Tm e") as f64;
                    current_y = operation.operands[5].as_float().expect("Tm f") as f64;
                }
                "Tj" => {
                    let bytes = operation.operands[0].as_str().expect("Tj text");
                    positions.push(PdfTextPosition {
                        page: page_number as usize,
                        text: decode_pdf_text_bytes(bytes),
                        x_pt: current_x,
                        y_pt: current_y,
                        font_size_pt: current_font_size,
                        fill_color: current_fill_color,
                        scale_percent: current_scale,
                    });
                }
                _ => {}
            }
        }
    }
    positions
}

fn pdf_contains_uri_annotation(pdf: &[u8], expected_uri: &str) -> bool {
    let document = Document::load_mem(pdf).expect("pdf should parse");
    document.objects.values().any(|object| {
        let Ok(dict) = object.as_dict() else {
            return false;
        };
        if dict
            .get(b"Subtype")
            .ok()
            .and_then(|object| object.as_name().ok())
            != Some(b"Link".as_slice())
        {
            return false;
        }
        let Ok(action) = dict.get(b"A").and_then(lopdf::Object::as_dict) else {
            return false;
        };
        action
            .get(b"URI")
            .and_then(lopdf::Object::as_str)
            .is_ok_and(|uri| uri == expected_uri.as_bytes())
    })
}

fn count_pdf_page_operator(pdf: &[u8], operator: &str) -> usize {
    let document = parse_document(pdf);
    document
        .get_pages()
        .into_values()
        .map(|page_id| {
            let content_bytes = document
                .get_page_content(page_id)
                .expect("read page content stream");
            let content = Content::decode(&content_bytes).expect("decode page content stream");
            content
                .operations
                .iter()
                .filter(|operation| operation.operator == operator)
                .count()
        })
        .sum()
}

fn pdf_page_operator_numbers(pdf: &[u8], operator: &str) -> Vec<f64> {
    let document = parse_document(pdf);
    document
        .get_pages()
        .into_values()
        .flat_map(|page_id| {
            let content_bytes = document
                .get_page_content(page_id)
                .expect("read page content stream");
            let content = Content::decode(&content_bytes).expect("decode page content stream");
            content
                .operations
                .into_iter()
                .filter(move |operation| operation.operator == operator)
                .flat_map(|operation| {
                    operation
                        .operands
                        .into_iter()
                        .filter_map(|operand| operand.as_float().ok().map(|value| value as f64))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn estimated_pdf_text_width_pt(text: &str, font_size_pt: f64) -> f64 {
    let weighted_chars = text
        .chars()
        .map(|ch| if ch.is_ascii() { 0.55 } else { 1.0 })
        .sum::<f64>();
    (weighted_chars * font_size_pt).max(0.0)
}

fn first_page_font_resource(document: &Document) -> lopdf::Dictionary {
    let (_, page_id) = document
        .get_pages()
        .into_iter()
        .next()
        .expect("pdf should have a first page");
    let page = document
        .get_object(page_id)
        .expect("first page object")
        .as_dict()
        .expect("first page dictionary");
    let resources = page
        .get(b"Resources")
        .expect("page resources")
        .as_dict()
        .expect("resource dictionary");
    let font_ref = resources
        .get(b"Font")
        .expect("font resources")
        .as_dict()
        .expect("font resource dictionary")
        .get(b"F1")
        .expect("F1 font")
        .as_reference()
        .expect("F1 font reference");
    document
        .get_object(font_ref)
        .expect("font object")
        .as_dict()
        .expect("font dictionary")
        .clone()
}

fn first_page_xobject_resources(document: &Document) -> lopdf::Dictionary {
    let (_, page_id) = document
        .get_pages()
        .into_iter()
        .next()
        .expect("pdf should have a first page");
    let page = document
        .get_object(page_id)
        .expect("first page object")
        .as_dict()
        .expect("first page dictionary");
    let resources = page
        .get(b"Resources")
        .expect("page resources")
        .as_dict()
        .expect("resource dictionary");
    resources
        .get(b"XObject")
        .expect("xobject resources")
        .as_dict()
        .expect("xobject dictionary")
        .clone()
}

fn first_page_operations(pdf_bytes: &[u8]) -> Vec<lopdf::content::Operation> {
    page_operations(pdf_bytes, 1)
}

fn page_operations(pdf_bytes: &[u8], page_number: u32) -> Vec<lopdf::content::Operation> {
    let document = parse_document(pdf_bytes);
    let page_id = document
        .get_pages()
        .into_iter()
        .find_map(|(number, page_id)| (number == page_number).then_some(page_id))
        .unwrap_or_else(|| panic!("pdf should have page {page_number}"));
    let content_bytes = document
        .get_page_content(page_id)
        .expect("read page content stream");
    Content::decode(&content_bytes)
        .expect("decode page content stream")
        .operations
}

fn inline_image_docx_with_jpeg_media() -> Vec<u8> {
    let bytes = fs::read(fixture_path("image-inline.docx")).expect("read image fixture");
    let mut input = ZipArchive::new(Cursor::new(bytes)).expect("open fixture zip");
    let mut output = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut output);
    for index in 0..input.len() {
        let mut file = input.by_index(index).expect("zip entry");
        let name = file.name().to_string();
        if name == "word/media/image1.png" {
            continue;
        }
        let mut data = Vec::new();
        file.read_to_end(&mut data).expect("read zip entry");
        if name == "word/_rels/document.xml.rels" {
            data = String::from_utf8(data)
                .expect("rels xml")
                .replace("media/image1.png", "media/image1.jpg")
                .into_bytes();
        } else if name == "[Content_Types].xml" {
            let xml = String::from_utf8(data).expect("content types xml");
            data = if xml.contains("Extension=\"jpg\"") {
                xml.into_bytes()
            } else {
                xml.replace(
                    "</Types>",
                    r#"<Default Extension="jpg" ContentType="image/jpeg"/></Types>"#,
                )
                .into_bytes()
            };
        }
        writer
            .start_file(name, SimpleFileOptions::default())
            .expect("start zip entry");
        writer.write_all(&data).expect("write zip entry");
    }
    writer
        .start_file("word/media/image1.jpg", SimpleFileOptions::default())
        .expect("start jpeg entry");
    writer
        .write_all(TEST_JPEG_1X1_RGB)
        .expect("write jpeg entry");
    writer.finish().expect("finish zip");
    output.into_inner()
}

fn prepend_paragraph_to_docx(bytes: &[u8], text: &str) -> Vec<u8> {
    prepend_paragraphs_to_docx(bytes, std::iter::once(text))
}

fn mixed_page_size_docx() -> Vec<u8> {
    let bytes = fs::read(fixture_path("hello.docx")).expect("read hello fixture");
    let mut input = ZipArchive::new(Cursor::new(bytes)).expect("open fixture zip");
    let mut output = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut output);
    for index in 0..input.len() {
        let mut file = input.by_index(index).expect("zip entry");
        let name = file.name().to_string();
        let mut data = Vec::new();
        file.read_to_end(&mut data).expect("read zip entry");
        if name == "word/document.xml" {
            let xml = String::from_utf8(data).expect("document xml utf8");
            data = xml
                .replace("<w:br w:type=\"page\"/>", "")
                .replacen(
                    "<w:bidi w:val=\"0\"/></w:pPr>",
                    "<w:bidi w:val=\"0\"/><w:sectPr><w:pgSz w:w=\"15840\" w:h=\"12240\" w:orient=\"landscape\"/></w:sectPr></w:pPr>",
                    1,
                )
                .into_bytes();
        }
        writer
            .start_file(name, SimpleFileOptions::default())
            .expect("start zip entry");
        writer.write_all(&data).expect("write zip entry");
    }
    writer.finish().expect("finish zip");
    output.into_inner()
}

fn prepend_paragraphs_to_docx<'a>(
    bytes: &[u8],
    texts: impl IntoIterator<Item = &'a str>,
) -> Vec<u8> {
    let prefix = texts
        .into_iter()
        .map(|text| format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>"))
        .collect::<String>();
    let mut input = ZipArchive::new(Cursor::new(bytes)).expect("open fixture zip");
    let mut output = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut output);
    let options = SimpleFileOptions::default();
    for index in 0..input.len() {
        let mut file = input.by_index(index).expect("zip entry");
        let name = file.name().to_string();
        let mut data = Vec::new();
        file.read_to_end(&mut data).expect("read zip entry");
        if name == "word/document.xml" {
            let xml = String::from_utf8(data).expect("document xml utf8");
            data = xml
                .replacen("<w:body>", &format!("<w:body>{prefix}"), 1)
                .into_bytes();
        }
        writer.start_file(name, options).expect("start zip entry");
        writer.write_all(&data).expect("write zip entry");
    }
    writer.finish().expect("finish zip");
    output.into_inner()
}

fn run_styles_docx_with_highlight() -> Vec<u8> {
    let bytes = fs::read(fixture_path("run-styles.docx")).expect("read run-styles fixture");
    let mut input = ZipArchive::new(Cursor::new(bytes)).expect("open fixture zip");
    let mut output = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut output);
    for index in 0..input.len() {
        let mut file = input.by_index(index).expect("zip entry");
        let name = file.name().to_string();
        let mut data = Vec::new();
        file.read_to_end(&mut data).expect("read zip entry");
        if name == "word/document.xml" {
            data = String::from_utf8(data)
                .expect("document xml")
                .replace(
                    r#"<w:color w:val="ff2f92"/>"#,
                    r#"<w:color w:val="ff2f92"/><w:highlight w:val="yellow"/>"#,
                )
                .into_bytes();
        }
        writer
            .start_file(name, SimpleFileOptions::default())
            .expect("start zip entry");
        writer.write_all(&data).expect("write zip entry");
    }
    writer.finish().expect("finish zip");
    output.into_inner()
}

fn run_styles_docx_with_char_spacing() -> Vec<u8> {
    let bytes = fs::read(fixture_path("run-styles.docx")).expect("read run-styles fixture");
    let mut input = ZipArchive::new(Cursor::new(bytes)).expect("open fixture zip");
    let mut output = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut output);
    for index in 0..input.len() {
        let mut file = input.by_index(index).expect("zip entry");
        let name = file.name().to_string();
        let mut data = Vec::new();
        file.read_to_end(&mut data).expect("read zip entry");
        if name == "word/document.xml" {
            data = String::from_utf8(data)
                .expect("document xml")
                .replace(
                    r#"<w:color w:val="ff2f92"/>"#,
                    r#"<w:color w:val="ff2f92"/><w:spacing w:val="40"/>"#,
                )
                .into_bytes();
        }
        writer
            .start_file(name, SimpleFileOptions::default())
            .expect("start zip entry");
        writer.write_all(&data).expect("write zip entry");
    }
    writer.finish().expect("finish zip");
    output.into_inner()
}

fn run_styles_docx_with_text_scale(scale_percent: u32) -> Vec<u8> {
    let bytes = fs::read(fixture_path("run-styles.docx")).expect("read run-styles fixture");
    let mut input = ZipArchive::new(Cursor::new(bytes)).expect("open fixture zip");
    let mut output = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut output);
    for index in 0..input.len() {
        let mut file = input.by_index(index).expect("zip entry");
        let name = file.name().to_string();
        let mut data = Vec::new();
        file.read_to_end(&mut data).expect("read zip entry");
        if name == "word/document.xml" {
            data = String::from_utf8(data)
                .expect("document xml")
                .replace(
                    r#"<w:color w:val="ff2f92"/>"#,
                    &format!(r#"<w:color w:val="ff2f92"/><w:w w:val="{scale_percent}"/>"#),
                )
                .into_bytes();
        }
        writer
            .start_file(name, SimpleFileOptions::default())
            .expect("start zip entry");
        writer.write_all(&data).expect("write zip entry");
    }
    writer.finish().expect("finish zip");
    output.into_inner()
}

fn run_styles_docx_with_run_property(property_xml: &str) -> Vec<u8> {
    let bytes = fs::read(fixture_path("run-styles.docx")).expect("read run-styles fixture");
    let mut input = ZipArchive::new(Cursor::new(bytes)).expect("open fixture zip");
    let mut output = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut output);
    for index in 0..input.len() {
        let mut file = input.by_index(index).expect("zip entry");
        let name = file.name().to_string();
        let mut data = Vec::new();
        file.read_to_end(&mut data).expect("read zip entry");
        if name == "word/document.xml" {
            data = String::from_utf8(data)
                .expect("document xml")
                .replace(
                    r#"<w:color w:val="ff2f92"/>"#,
                    &format!(r#"<w:color w:val="ff2f92"/>{property_xml}"#),
                )
                .into_bytes();
        }
        writer
            .start_file(name, SimpleFileOptions::default())
            .expect("start zip entry");
        writer.write_all(&data).expect("write zip entry");
    }
    writer.finish().expect("finish zip");
    output.into_inner()
}

#[test]
fn renders_pdf_from_hello_fixture() {
    let bytes = fs::read(fixture_path("hello.docx")).expect("read hello fixture");
    let pdf = render_docx_to_pdf(&bytes, &RenderOptions::default()).expect("render pdf");
    let text = String::from_utf8_lossy(&pdf);
    let document = parse_document(&pdf);

    assert!(text.starts_with("%PDF-"));
    assert!(parse_page_count(&pdf) >= 1);
    assert!(document.trailer.get(b"Root").is_ok());
    assert!(document.trailer.get(b"Info").is_ok());
}

#[test]
fn hello_fixture_matches_word_page_break_and_text_origins() {
    // Word for Mac 16.113.2 on macOS 15.4.1 exported the unchanged hello.docx
    // to two A4 pages. The PDF text matrices placed Hello and World at
    // (56.64, 773.76) pt on their respective pages. This is a narrow measured
    // comparison, not full line-box oracle evidence.
    let bytes = fs::read(fixture_path("hello.docx")).expect("read hello fixture");
    let pdf = render_docx_to_pdf(&bytes, &RenderOptions::default()).expect("render pdf");
    assert_eq!(parse_page_count(&pdf), 2);

    let positions = extract_pdf_text_positions(&pdf);
    assert_eq!(
        positions.len(),
        2,
        "unexpected PDF text positions: {positions:?}"
    );
    for (position, page, text) in [(&positions[0], 1, "Hello"), (&positions[1], 2, "World")] {
        assert_eq!((position.page, position.text.as_str()), (page, text));
        assert!(
            (position.x_pt - 56.64).abs() <= 0.25,
            "{text} x differs from Word: {} pt",
            position.x_pt
        );
        assert!(
            (position.y_pt - 773.76).abs() <= 2.0,
            "{text} baseline differs from Word: {} pt",
            position.y_pt
        );
    }
}

#[test]
fn render_pdf_preserves_section_page_sizes_and_selectable_text() {
    let bytes = mixed_page_size_docx();
    let pdf = render_docx_to_pdf(&bytes, &RenderOptions::default()).expect("render pdf");
    let document = parse_document(&pdf);
    let pages = document.get_pages();

    assert_eq!(pages.len(), 2);
    let media_boxes = pages
        .values()
        .map(|page_id| {
            let page = document
                .get_object(*page_id)
                .expect("page object")
                .as_dict()
                .expect("page dictionary");
            let media_box = page
                .get(b"MediaBox")
                .expect("page MediaBox")
                .as_array()
                .expect("MediaBox array");
            (
                media_box[2].as_float().expect("page width") as f64,
                media_box[3].as_float().expect("page height") as f64,
            )
        })
        .collect::<Vec<_>>();
    for (actual, expected) in media_boxes.iter().zip([(792.0, 612.0), (595.3, 841.9)]) {
        assert!((actual.0 - expected.0).abs() < 0.01, "{actual:?}");
        assert!((actual.1 - expected.1).abs() < 0.01, "{actual:?}");
    }

    let text = extract_pdf_text_positions(&pdf)
        .into_iter()
        .map(|position| position.text)
        .collect::<String>();
    assert!(text.contains("Hello"));
    assert!(text.contains("World"));
}

#[test]
fn renders_pdf_from_contract_fixture() {
    let bytes = fs::read(fixture_path("contract.docx")).expect("read contract fixture");
    let pdf = render_docx_to_pdf(&bytes, &RenderOptions::default()).expect("render pdf");
    let text = String::from_utf8_lossy(&pdf);
    let document = parse_document(&pdf);

    assert!(text.starts_with("%PDF-"));
    assert!(parse_page_count(&pdf) >= 1);
    assert!(document.trailer.get(b"Root").is_ok());
    assert!(document.trailer.get(b"Info").is_ok());
}

#[test]
fn renders_pdf_from_yokohama_page6_fixture() {
    let bytes = fs::read(fixture_path("yokohama_page6.docx")).expect("read yokohama_page6 fixture");
    let pdf = render_docx_to_pdf(&bytes, &RenderOptions::default()).expect("render pdf");
    let text = String::from_utf8_lossy(&pdf);
    let document = parse_document(&pdf);

    assert!(text.starts_with("%PDF-"));
    assert!(parse_page_count(&pdf) >= 2);
    assert!(document.trailer.get(b"Root").is_ok());
    assert!(document.trailer.get(b"Info").is_ok());
}

#[test]
fn render_pdf_with_report_surfaces_prod_readiness_warnings() {
    let bytes = fs::read(fixture_path("yokohama_page6.docx")).expect("read yokohama_page6 fixture");
    let (pdf, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");

    assert!(String::from_utf8_lossy(&pdf).starts_with("%PDF-"));
    assert!(report.pages >= 2);
    assert!(report.rendered_lines > 0);
    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_FONT_NOT_EMBEDDED")),
        "default renderer should embed the bundled font: {:?}",
        report.warnings
    );
    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_NON_ASCII_TEXT_USES_VIEWER_FONT")),
        "embedded CJK text should not depend on viewer CID fonts: {:?}",
        report.warnings
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_REQUESTED_FONTS_NOT_EMBEDDED")),
        "requested DOCX fonts should be surfaced until per-run PDF font selection is implemented: {:?}",
        report.warnings
    );
}

#[test]
fn render_pdf_renders_math_as_text_fallback() {
    let bytes = fs::read(fixture_path("math.docx")).expect("read math fixture");
    let (pdf, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");
    let rendered_text = extract_pdf_text_positions(&pdf)
        .iter()
        .map(|position| position.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        report.rendered_math >= 1,
        "math fixture should render math text fallback: {:?}",
        report
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_MATH_TEXT_FALLBACK")),
        "math fallback should remain visible to production gates: {:?}",
        report.warnings
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_MATH_DRAWING: 1 ")),
        "unpreviewable math should remain visible as unsupported: {:?}",
        report.warnings
    );
    assert!(
        rendered_text.contains("a≤X≤b"),
        "math preview text should appear in PDF text stream: {rendered_text:?}"
    );
}

#[test]
fn render_pdf_places_math_fallback_on_its_document_page() {
    let original = fs::read(fixture_path("math.docx")).expect("read math fixture");
    let leading = vec!["Leading paragraph"; 60];
    let bytes = prepend_paragraphs_to_docx(&original, leading.iter().copied());
    let (pdf, _) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");
    let positions = extract_pdf_text_positions(&pdf);
    let math = positions
        .into_iter()
        .find(|position| position.text.contains("a≤X≤b"))
        .expect("math fallback text");

    assert!(
        math.page > 1,
        "math after 60 leading paragraphs should not be rendered on page 1"
    );
}

#[test]
fn render_pdf_draws_simple_footnote_text() {
    let bytes = fs::read(fixture_path("footnote.docx")).expect("read footnote fixture");
    let (pdf, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");
    let rendered_text = extract_pdf_text_positions(&pdf)
        .iter()
        .map(|position| position.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(report.rendered_notes, 1);
    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_NOTE_DRAWING")),
        "simple footnote should be drawn, warnings={:?}",
        report.warnings
    );
    assert!(
        rendered_text.contains("1. Hello World"),
        "footnote text should appear in PDF text stream: {rendered_text:?}"
    );
}

#[test]
fn footnote_is_drawn_on_its_reference_page() {
    let original = fs::read(fixture_path("footnote.docx")).expect("read footnote fixture");
    let leading = (0..60).map(|_| "Leading paragraph").collect::<Vec<_>>();
    let bytes = prepend_paragraphs_to_docx(&original, leading.iter().copied());
    let layout = block_on(layout_docx_lines_from_bytes(&bytes, &TestLayoutContext))
        .expect("layout modified footnote fixture");
    let reference_page = layout
        .get_lines()
        .find(|line| line.paragraph_index == leading.len())
        .map(|line| line.page)
        .expect("footnote reference paragraph line");
    assert!(
        reference_page > 1,
        "fixture must move the reference off page 1"
    );

    let pdf = render_docx_to_pdf(&bytes, &RenderOptions::default()).expect("render pdf");
    let note = extract_pdf_text_positions(&pdf)
        .into_iter()
        .find(|position| position.text.contains("1. Hello World"))
        .expect("rendered footnote text");
    assert_eq!(note.page, reference_page);
}

#[test]
fn render_pdf_draws_simple_endnote_text() {
    let bytes = fs::read(fixture_path("endnote.docx")).expect("read endnote fixture");
    let (pdf, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");
    let rendered_text = extract_pdf_text_positions(&pdf)
        .iter()
        .map(|position| position.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(report.rendered_notes, 1);
    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_NOTE_DRAWING")),
        "simple endnote should be drawn, warnings={:?}",
        report.warnings
    );
    assert!(
        rendered_text.contains("1. World"),
        "endnote text should appear in PDF text stream: {rendered_text:?}"
    );
}

#[test]
fn render_pdf_draws_simple_table_grid() {
    let bytes = fs::read(fixture_path("table2x2.docx")).expect("read table fixture");
    let (pdf, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");
    let stroke_count = first_page_operations(&pdf)
        .iter()
        .filter(|operation| operation.operator == "S")
        .count();
    let rendered_text = extract_pdf_text_positions(&pdf)
        .iter()
        .map(|position| position.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(report.rendered_tables, 1);
    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_TABLE_DRAWING")),
        "simple top-level table should be drawn, warnings={:?}",
        report.warnings
    );
    assert!(
        stroke_count >= 6,
        "2x2 table should draw at least 3 vertical and 3 horizontal strokes, got {stroke_count}"
    );
    assert!(
        rendered_text.contains("Hello") && rendered_text.contains("World"),
        "table cell text should remain in PDF text stream: {rendered_text:?}"
    );
}

#[test]
fn render_pdf_draws_table_grid_on_the_table_page() {
    let original = fs::read(fixture_path("table2x2.docx")).expect("read table fixture");
    let leading = vec!["Leading paragraph"; 60];
    let bytes = prepend_paragraphs_to_docx(&original, leading.iter().copied());
    let layout = block_on(layout_docx_lines_from_bytes(&bytes, &TestLayoutContext))
        .expect("layout table fixture");
    let table_page = layout
        .get_lines()
        .find(|line| line.text.contains("Hello"))
        .expect("table cell line")
        .page;
    assert!(table_page > 1, "table fixture should flow beyond page 1");

    let (pdf, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");
    let stroke_count = page_operations(&pdf, table_page as u32)
        .iter()
        .filter(|operation| operation.operator == "S")
        .count();

    assert_eq!(report.rendered_tables, 1);
    assert!(
        stroke_count >= 6,
        "table page {table_page} should draw the 2x2 grid, got {stroke_count} strokes"
    );
}

#[test]
fn render_pdf_preserves_numbering_markers_in_layout_lines() {
    let bytes = fs::read(fixture_path("numbering-del.docx")).expect("read numbering fixture");
    let (pdf, _) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");
    let rendered_text = extract_pdf_text_positions(&pdf)
        .iter()
        .map(|position| position.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        rendered_text.contains("1.") && rendered_text.contains("2."),
        "accepted list markers should remain in PDF text: {rendered_text:?}"
    );
}

#[test]
fn render_pdf_embeds_text_header_footer_lines() {
    let bytes = fs::read(fixture_path("header.docx")).expect("read header fixture");
    let (pdf, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");
    let positions = extract_pdf_text_positions(&pdf);
    let rendered_text = positions
        .iter()
        .map(|position| position.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        report.rendered_header_footer_lines >= 1,
        "expected header/footer lines in report: {:?}",
        report
    );
    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_HEADER_FOOTER_DRAWING")),
        "text header/footer should be rendered, warnings={:?}",
        report.warnings
    );
    assert!(
        rendered_text.contains("Hello"),
        "header text should appear in PDF text stream: {rendered_text:?}"
    );
}

#[test]
fn render_pdf_ignores_empty_header_footer_references_for_readiness() {
    let bytes = fs::read(fixture_path("hello.docx")).expect("read hello fixture");
    let (_, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");

    assert_eq!(report.rendered_header_footer_lines, 0);
    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_HEADER_FOOTER_DRAWING")),
        "empty header/footer parts should not fail readiness, warnings={:?}",
        report.warnings
    );
}

#[test]
fn render_pdf_warns_for_unsupported_line_numbering() {
    let bytes = fs::read(fixture_path("line-numbering.docx")).expect("read line numbering fixture");
    let (_, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");

    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_LINE_NUMBERING: 1 ")),
        "line numbering should remain visible to production gates: {:?}",
        report.warnings
    );

    let options = RenderOptions {
        fail_on_warnings: true,
        ..RenderOptions::default()
    };
    let err = render_docx_to_pdf_with_report(&bytes, &options)
        .expect_err("strict render should fail on unsupported line numbering");
    assert!(
        matches!(err, jade_docx_pdf_renderer::RenderError::ReadinessWarnings(ref warnings) if warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_LINE_NUMBERING"))),
        "strict error should include line numbering warning: {err:?}"
    );
}

#[test]
fn render_pdf_warns_for_unsupported_comments() {
    let bytes = fs::read(fixture_path("comment.docx")).expect("read comment fixture");
    let (_, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");

    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_COMMENT_RENDERING: 2 ")),
        "comments should remain visible to production gates: {:?}",
        report.warnings
    );

    let options = RenderOptions {
        fail_on_warnings: true,
        ..RenderOptions::default()
    };
    let err = render_docx_to_pdf_with_report(&bytes, &options)
        .expect_err("strict render should fail on unsupported comments");
    assert!(
        matches!(err, jade_docx_pdf_renderer::RenderError::ReadinessWarnings(ref warnings) if warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_COMMENT_RENDERING"))),
        "strict error should include comment warning: {err:?}"
    );
}

#[test]
fn render_pdf_warns_for_unsupported_revision_markup() {
    let bytes = fs::read(fixture_path("review.docx")).expect("read review fixture");
    let (_, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");

    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_REVISION_MARKUP: 2 ")),
        "tracked revisions should remain visible to production gates: {:?}",
        report.warnings
    );

    let options = RenderOptions {
        fail_on_warnings: true,
        ..RenderOptions::default()
    };
    let err = render_docx_to_pdf_with_report(&bytes, &options)
        .expect_err("strict render should fail on unsupported tracked revisions");
    assert!(
        matches!(err, jade_docx_pdf_renderer::RenderError::ReadinessWarnings(ref warnings) if warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_REVISION_MARKUP"))),
        "strict error should include revision warning: {err:?}"
    );
}

#[test]
fn render_pdf_draws_supported_direct_run_styling() {
    let bytes = fs::read(fixture_path("run-styles.docx")).expect("read run-styles fixture");
    let (pdf, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");
    let positions = extract_pdf_text_positions(&pdf);
    let hello = positions
        .iter()
        .find(|position| position.text == "Hello")
        .expect("styled Hello text should be present");

    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_RUN_STYLING")),
        "supported direct run styling should not trip production gates: {:?}",
        report.warnings
    );
    assert!(
        (hello.fill_color.0 - 1.0).abs() <= 0.001
            && (hello.fill_color.1 - (0x2f as f64 / 255.0)).abs() <= 0.001
            && (hello.fill_color.2 - (0x92 as f64 / 255.0)).abs() <= 0.001,
        "supported run color should be emitted as PDF text fill color: {:?}",
        hello.fill_color
    );
    assert!(
        count_pdf_page_operator(&pdf, "S") >= 2,
        "underline and strikethrough should emit at least two stroked PDF lines"
    );

    let options = RenderOptions {
        fail_on_warnings: true,
        ..RenderOptions::default()
    };
    render_docx_to_pdf_with_report(&bytes, &options)
        .expect("strict render should accept supported direct run styling");
}

#[test]
fn render_pdf_draws_run_highlight_background() {
    let bytes = run_styles_docx_with_highlight();
    let (pdf, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");

    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_RUN_STYLING")),
        "supported run highlight should not trip production gates: {:?}",
        report.warnings
    );
    assert_eq!(
        count_pdf_page_operator(&pdf, "f"),
        1,
        "run highlight should be emitted as one filled PDF rectangle"
    );

    let options = RenderOptions {
        fail_on_warnings: true,
        ..RenderOptions::default()
    };
    render_docx_to_pdf_with_report(&bytes, &options)
        .expect("strict render should accept supported run highlight");
}

#[test]
fn render_pdf_draws_run_character_spacing() {
    let bytes = run_styles_docx_with_char_spacing();
    let (pdf, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");

    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_RUN_STYLING")),
        "supported run character spacing should not trip production gates: {:?}",
        report.warnings
    );
    assert!(
        pdf_page_operator_numbers(&pdf, "Tc")
            .iter()
            .any(|value| (*value - 2.0).abs() <= 0.001),
        "w:spacing value 40 should be emitted as 2pt PDF character spacing"
    );

    let options = RenderOptions {
        fail_on_warnings: true,
        ..RenderOptions::default()
    };
    render_docx_to_pdf_with_report(&bytes, &options)
        .expect("strict render should accept supported run character spacing");
}

#[test]
fn render_pdf_draws_run_text_scale() {
    let base = fs::read(fixture_path("run-styles.docx")).expect("read run-styles fixture");
    let (base_pdf, _) =
        render_docx_to_pdf_with_report(&base, &RenderOptions::default()).expect("render base pdf");
    let base_hello_scale = extract_pdf_text_positions(&base_pdf)
        .into_iter()
        .find(|position| position.text == "Hello")
        .expect("base Hello text should be present")
        .scale_percent;

    let bytes = run_styles_docx_with_text_scale(50);
    let (pdf, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");
    let scaled_hello_scale = extract_pdf_text_positions(&pdf)
        .into_iter()
        .find(|position| position.text == "Hello")
        .expect("scaled Hello text should be present")
        .scale_percent;

    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_RUN_STYLING")),
        "supported run text scale should not trip production gates: {:?}",
        report.warnings
    );
    assert!(
        scaled_hello_scale < base_hello_scale * 0.75,
        "w:w=50 should emit a materially narrower PDF Tz scale: base={base_hello_scale}, scaled={scaled_hello_scale}"
    );

    let options = RenderOptions {
        fail_on_warnings: true,
        ..RenderOptions::default()
    };
    render_docx_to_pdf_with_report(&bytes, &options)
        .expect("strict render should accept supported run text scale");
}

#[test]
fn render_pdf_draws_run_text_position() {
    let base = fs::read(fixture_path("run-styles.docx")).expect("read run-styles fixture");
    let (base_pdf, _) =
        render_docx_to_pdf_with_report(&base, &RenderOptions::default()).expect("render base pdf");
    let base_hello = extract_pdf_text_positions(&base_pdf)
        .into_iter()
        .find(|position| position.text == "Hello")
        .expect("base Hello text should be present");

    let bytes = run_styles_docx_with_run_property(r#"<w:position w:val="12"/>"#);
    let (pdf, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");
    let raised_hello = extract_pdf_text_positions(&pdf)
        .into_iter()
        .find(|position| position.text == "Hello")
        .expect("raised Hello text should be present");

    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_RUN_STYLING")),
        "supported run text position should not trip production gates: {:?}",
        report.warnings
    );
    assert!(
        (raised_hello.y_pt - base_hello.y_pt - 6.0).abs() <= 0.001,
        "w:position value 12 should raise the PDF text baseline by 6pt: base={}, raised={}",
        base_hello.y_pt,
        raised_hello.y_pt
    );
    assert!(
        (raised_hello.font_size_pt - base_hello.font_size_pt).abs() <= 0.001,
        "w:position should shift the baseline without changing font size"
    );

    let options = RenderOptions {
        fail_on_warnings: true,
        ..RenderOptions::default()
    };
    render_docx_to_pdf_with_report(&bytes, &options)
        .expect("strict render should accept supported run text position");
}

#[test]
fn render_pdf_draws_run_superscript_baseline() {
    let base = fs::read(fixture_path("run-styles.docx")).expect("read run-styles fixture");
    let (base_pdf, _) =
        render_docx_to_pdf_with_report(&base, &RenderOptions::default()).expect("render base pdf");
    let base_hello = extract_pdf_text_positions(&base_pdf)
        .into_iter()
        .find(|position| position.text == "Hello")
        .expect("base Hello text should be present");

    let bytes = run_styles_docx_with_run_property(r#"<w:vertAlign w:val="superscript"/>"#);
    let (pdf, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");
    let superscript_hello = extract_pdf_text_positions(&pdf)
        .into_iter()
        .find(|position| position.text == "Hello")
        .expect("superscript Hello text should be present");

    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_RUN_STYLING")),
        "supported run superscript should not trip production gates: {:?}",
        report.warnings
    );
    assert!(
        superscript_hello.y_pt > base_hello.y_pt + 3.0,
        "superscript should raise the PDF text baseline: base={}, superscript={}",
        base_hello.y_pt,
        superscript_hello.y_pt
    );
    assert!(
        superscript_hello.font_size_pt < base_hello.font_size_pt * 0.75,
        "superscript should reduce PDF font size: base={}, superscript={}",
        base_hello.font_size_pt,
        superscript_hello.font_size_pt
    );

    let options = RenderOptions {
        fail_on_warnings: true,
        ..RenderOptions::default()
    };
    render_docx_to_pdf_with_report(&bytes, &options)
        .expect("strict render should accept supported run superscript");
}

#[test]
fn render_pdf_emits_external_hyperlink_annotations() {
    let bytes = fs::read(fixture_path("hyperlink.docx")).expect("read hyperlink fixture");
    let (pdf, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");
    let rendered_text = extract_pdf_text_positions(&pdf)
        .iter()
        .map(|position| position.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        rendered_text.contains("example.com"),
        "hyperlink text should remain visible in PDF text stream: {rendered_text:?}"
    );
    assert_eq!(report.rendered_hyperlinks, 1);
    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_HYPERLINK_ANNOTATION")),
        "resolved external hyperlinks should not trigger unsupported warnings: {:?}",
        report.warnings
    );
    assert!(
        pdf_contains_uri_annotation(&pdf, "https://example.com/"),
        "external hyperlink annotation should point at fixture URL"
    );

    let options = RenderOptions {
        fail_on_warnings: true,
        ..RenderOptions::default()
    };
    render_docx_to_pdf_with_report(&bytes, &options)
        .expect("strict render should accept resolved external hyperlink annotations");
}

#[test]
fn render_pdf_warns_for_unsupported_field_instructions() {
    let bytes = fs::read(fixture_path("toc.docx")).expect("read toc fixture");
    let (_, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");

    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_FIELD_RENDERING: ")),
        "unsupported fields should remain visible to production gates: {:?}",
        report.warnings
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_HYPERLINK_ANNOTATION")),
        "TOC links should also remain visible to production gates: {:?}",
        report.warnings
    );

    let options = RenderOptions {
        fail_on_warnings: true,
        ..RenderOptions::default()
    };
    let err = render_docx_to_pdf_with_report(&bytes, &options)
        .expect_err("strict render should fail on unsupported field instructions");
    assert!(
        matches!(err, jade_docx_pdf_renderer::RenderError::ReadinessWarnings(ref warnings) if warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_FIELD_RENDERING"))),
        "strict error should include field warning: {err:?}"
    );
}

#[test]
fn render_pdf_draws_simple_text_box() {
    let bytes = fs::read(fixture_path("textbox.docx")).expect("read textbox fixture");
    let (pdf, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");
    let operations = first_page_operations(&pdf);
    let rectangle_count = operations
        .iter()
        .filter(|operation| operation.operator == "re")
        .count();
    let rendered_text = extract_pdf_text_positions(&pdf)
        .iter()
        .map(|position| position.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(report.rendered_text_boxes, 1);
    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_TEXT_BOX_DRAWING")),
        "simple text box should be drawn, warnings={:?}",
        report.warnings
    );
    assert!(
        rectangle_count >= 1,
        "text box should emit at least one rectangle primitive"
    );
    assert!(
        rendered_text.contains("Hello"),
        "text box text should appear in PDF text stream: {rendered_text:?}"
    );
}

#[test]
fn render_pdf_draws_text_box_dash_pattern() {
    let bytes = fs::read(fixture_path("textbox-dashed.docx")).expect("read dashed textbox fixture");
    let (pdf, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");
    let operations = first_page_operations(&pdf);

    assert_eq!(report.rendered_text_boxes, 1);
    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_TEXT_BOX_DRAWING")),
        "dashed text box should be drawn, warnings={:?}",
        report.warnings
    );
    assert!(
        operations.iter().any(|operation| operation.operator == "d"),
        "dashed text box should emit a PDF dash pattern"
    );
}

#[test]
fn render_pdf_embeds_inline_png_images() {
    let bytes = fs::read(fixture_path("image-inline.docx")).expect("read image fixture");
    let (pdf, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");
    let document = parse_document(&pdf);
    let xobjects = first_page_xobject_resources(&document);

    assert_eq!(report.rendered_images, 1);
    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_IMAGE_DRAWING")),
        "inline PNG image should be rendered, warnings={:?}",
        report.warnings
    );
    assert_eq!(xobjects.len(), 1, "expected one image XObject");
    let image_ref = xobjects
        .iter()
        .next()
        .expect("image xobject")
        .1
        .as_reference()
        .expect("image reference");
    let image = document
        .get_object(image_ref)
        .expect("image object")
        .as_stream()
        .expect("image stream");
    assert_eq!(
        image
            .dict
            .get(b"Subtype")
            .expect("image subtype")
            .as_name()
            .unwrap(),
        b"Image"
    );
    assert_eq!(
        image
            .dict
            .get(b"ColorSpace")
            .expect("image colorspace")
            .as_name()
            .unwrap(),
        b"DeviceRGB"
    );
    assert!(
        image.dict.get(b"SMask").is_ok(),
        "RGBA PNG should carry alpha mask"
    );
}

#[test]
fn render_pdf_embeds_floating_png_images() {
    let bytes = fs::read(fixture_path("image-floating.docx")).expect("read floating image fixture");
    let (pdf, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");
    let document = parse_document(&pdf);
    let xobjects = first_page_xobject_resources(&document);

    assert_eq!(report.rendered_images, 1);
    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_IMAGE_DRAWING")),
        "floating PNG image should be rendered, warnings={:?}",
        report.warnings
    );
    assert_eq!(xobjects.len(), 1, "expected one floating image XObject");
    let positions = extract_pdf_text_positions(&pdf);
    assert!(
        !positions.is_empty(),
        "floating image PDF should still preserve surrounding text"
    );
}

#[test]
fn floating_image_uses_its_paragraph_layout_anchor() {
    let original = fs::read(fixture_path("image-floating.docx")).expect("read fixture");
    let bytes = prepend_paragraph_to_docx(&original, "Leading paragraph");
    let layout = block_on(layout_docx_lines_from_bytes(&bytes, &TestLayoutContext))
        .expect("layout modified fixture");
    let anchor_line = layout
        .get_lines()
        .find(|line| line.paragraph_index == 1)
        .expect("image paragraph layout line");

    let mut archive = ZipArchive::new(Cursor::new(&bytes)).expect("open modified docx");
    let mut document_xml = String::new();
    archive
        .by_name("word/document.xml")
        .expect("document xml")
        .read_to_string(&mut document_xml)
        .expect("read document xml");
    let parsed = jade_docx_reader::DocumentParser::new()
        .parse_document_xml(&document_xml)
        .expect("parse document");
    let image = parsed
        .body
        .as_ref()
        .and_then(|body| body.elements.get(1))
        .and_then(|element| element.as_paragraph())
        .and_then(|paragraph| paragraph.runs.iter().find_map(|run| run.image.as_ref()))
        .expect("floating image");
    let expected_x = anchor_line.x
        + image
            .layout
            .position_h
            .as_ref()
            .and_then(|position| position.offset_px)
            .unwrap_or(0.0);
    let expected_y = anchor_line.y
        + image
            .layout
            .position_v
            .as_ref()
            .and_then(|position| position.offset_px)
            .unwrap_or(0.0);
    let image_height = image.height_px.expect("image height");

    let pdf = render_docx_to_pdf(&bytes, &RenderOptions::default()).expect("render pdf");
    let document = parse_document(&pdf);
    let page_id = *document.get_pages().values().next().expect("first page");
    let content = Content::decode(
        &document
            .get_page_content(page_id)
            .expect("page content stream"),
    )
    .expect("decode page content");
    let image_matrix = content
        .operations
        .windows(2)
        .find(|ops| ops[0].operator == "cm" && ops[1].operator == "Do")
        .map(|ops| &ops[0])
        .expect("image placement matrix");
    let actual_x = image_matrix.operands[4].as_float().expect("cm e") as f64 * CSS_PX_PER_PT;
    let image_bottom = image_matrix.operands[5].as_float().expect("cm f") as f64;
    let actual_y =
        layout.page_height() - (image_bottom + image_height / CSS_PX_PER_PT) * CSS_PX_PER_PT;

    assert!(
        (actual_x - expected_x).abs() <= 0.05,
        "x={actual_x}, expected={expected_x}"
    );
    assert!(
        (actual_y - expected_y).abs() <= 0.05,
        "y={actual_y}, expected={expected_y}"
    );
}

#[test]
fn render_pdf_embeds_inline_jpeg_images() {
    let bytes = inline_image_docx_with_jpeg_media();
    let (pdf, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");
    let document = parse_document(&pdf);
    let xobjects = first_page_xobject_resources(&document);

    assert_eq!(report.rendered_images, 1);
    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_IMAGE_DRAWING")),
        "inline JPEG image should be rendered, warnings={:?}",
        report.warnings
    );
    assert_eq!(xobjects.len(), 1, "expected one JPEG image XObject");
    let image_ref = xobjects
        .iter()
        .next()
        .expect("image xobject")
        .1
        .as_reference()
        .expect("image reference");
    let image = document
        .get_object(image_ref)
        .expect("image object")
        .as_stream()
        .expect("image stream");
    assert_eq!(
        image
            .dict
            .get(b"Filter")
            .expect("image filter")
            .as_name()
            .unwrap(),
        b"DCTDecode"
    );
    assert!(
        image.dict.get(b"SMask").is_err(),
        "JPEG should not carry a PNG alpha mask"
    );
}

#[test]
fn inline_image_uses_its_paragraph_layout_anchor() {
    let bytes =
        prepend_paragraph_to_docx(&inline_image_docx_with_jpeg_media(), "Leading paragraph");
    let layout = block_on(layout_docx_lines_from_bytes(&bytes, &TestLayoutContext))
        .expect("layout modified fixture");
    let anchor_line = layout
        .get_lines()
        .find(|line| line.paragraph_index == 1)
        .expect("image paragraph layout line");
    let pdf = render_docx_to_pdf(&bytes, &RenderOptions::default()).expect("render pdf");
    let document = parse_document(&pdf);
    let page_id = *document.get_pages().values().next().expect("first page");
    let content = Content::decode(
        &document
            .get_page_content(page_id)
            .expect("page content stream"),
    )
    .expect("decode page content");
    let image_matrix = content
        .operations
        .windows(2)
        .find(|ops| ops[0].operator == "cm" && ops[1].operator == "Do")
        .map(|ops| &ops[0])
        .expect("image placement matrix");
    let actual_x = image_matrix.operands[4].as_float().expect("cm e") as f64 * CSS_PX_PER_PT;
    let image_height = image_matrix.operands[3].as_float().expect("cm d") as f64;
    let image_bottom = image_matrix.operands[5].as_float().expect("cm f") as f64;
    let actual_y = layout.page_height() - (image_bottom + image_height) * CSS_PX_PER_PT;

    assert!((actual_x - anchor_line.x).abs() <= 0.05);
    assert!((actual_y - anchor_line.y).abs() <= 0.05);
}

#[test]
fn pdf_text_positions_match_jade_docx_layout_for_hello_fixture() {
    let bytes = fs::read(fixture_path("hello.docx")).expect("read hello fixture");
    let layout = block_on(layout_docx_lines_from_bytes(&bytes, &TestLayoutContext))
        .expect("layout hello fixture");
    let pdf = render_docx_to_pdf(&bytes, &RenderOptions::default()).expect("render pdf");
    let pdf_positions = extract_pdf_text_positions(&pdf);
    let expected = layout
        .get_lines()
        .filter(|line| !line.text.trim().is_empty())
        .take(8)
        .collect::<Vec<_>>();

    assert!(
        !expected.is_empty(),
        "fixture should expose layout text lines"
    );
    assert!(
        pdf_positions.len() >= expected.len(),
        "pdf should contain at least the checked layout lines"
    );

    for (expected_line, actual) in expected.iter().zip(pdf_positions.iter()) {
        let expected_x_pt = expected_line.x / CSS_PX_PER_PT;
        let expected_baseline_y = expected_line.y + expected_line.height * 0.8;
        let expected_y_pt =
            layout.page_height() / CSS_PX_PER_PT - expected_baseline_y / CSS_PX_PER_PT;
        assert_eq!(actual.page, expected_line.page);
        assert_eq!(actual.text, normalize_pdf_text(expected_line.text.trim()));
        let expected_width_pt = expected_line.width / CSS_PX_PER_PT;
        let scaled_width_pt = estimated_pdf_text_width_pt(&actual.text, actual.font_size_pt)
            * actual.scale_percent
            / 100.0;
        assert!(
            (actual.x_pt - expected_x_pt).abs() <= 0.02,
            "x mismatch for {:?}: pdf={} layout={}",
            expected_line.text,
            actual.x_pt,
            expected_x_pt
        );
        assert!(
            (actual.y_pt - expected_y_pt).abs() <= 0.02,
            "y mismatch for {:?}: pdf={} layout={}",
            expected_line.text,
            actual.y_pt,
            expected_y_pt
        );
        assert!(
            (scaled_width_pt - expected_width_pt).abs() <= 0.02,
            "width mismatch for {:?}: pdf={} layout={} scale={}",
            expected_line.text,
            scaled_width_pt,
            expected_width_pt,
            actual.scale_percent
        );
    }
}

#[test]
fn pdf_text_preserves_japanese_fixture_text() {
    let bytes = fs::read(fixture_path("yokohama_page6.docx")).expect("read yokohama_page6 fixture");
    let pdf = render_docx_to_pdf(&bytes, &RenderOptions::default()).expect("render pdf");
    let pdf_positions = extract_pdf_text_positions(&pdf);
    let first_japanese_line = pdf_positions
        .iter()
        .find(|line| !line.text.is_ascii())
        .expect("pdf should contain Japanese text from fixture");

    assert!(
        first_japanese_line.text.contains('第') || first_japanese_line.text.contains('市'),
        "expected real Japanese text in PDF, got {:?}",
        first_japanese_line.text
    );
    assert!(
        !first_japanese_line.text.contains('?'),
        "Japanese text should not be replaced with question marks"
    );
}

#[test]
fn pdf_font_resource_has_to_unicode_and_descriptor() {
    let bytes = fs::read(fixture_path("yokohama_page6.docx")).expect("read yokohama_page6 fixture");
    let pdf = render_docx_to_pdf(&bytes, &RenderOptions::default()).expect("render pdf");
    let document = parse_document(&pdf);
    let font = first_page_font_resource(&document);

    assert_eq!(
        font.get(b"Subtype")
            .expect("font subtype")
            .as_name()
            .unwrap(),
        b"Type0"
    );
    assert!(
        font.get(b"ToUnicode").is_ok(),
        "Type0 font should expose ToUnicode"
    );

    let descendant_ref = font
        .get(b"DescendantFonts")
        .expect("descendant fonts")
        .as_array()
        .expect("descendant array")[0]
        .as_reference()
        .expect("descendant reference");
    let descendant = document
        .get_object(descendant_ref)
        .expect("descendant font object")
        .as_dict()
        .expect("descendant font dictionary");
    assert_eq!(
        descendant
            .get(b"Subtype")
            .expect("descendant subtype")
            .as_name()
            .unwrap(),
        b"CIDFontType2"
    );
    assert!(
        descendant.get(b"FontDescriptor").is_ok(),
        "CID descendant font should include a FontDescriptor"
    );
    assert_eq!(
        descendant
            .get(b"DW")
            .expect("default CID width")
            .as_i64()
            .unwrap(),
        1000
    );
    assert!(
        descendant.get(b"CIDToGIDMap").is_ok(),
        "embedded CID font should map Unicode CIDs to TrueType glyph IDs"
    );
    assert!(
        descendant.get(b"W").is_ok(),
        "embedded CID font should include used-glyph widths"
    );

    let descriptor_ref = descendant
        .get(b"FontDescriptor")
        .expect("font descriptor")
        .as_reference()
        .expect("font descriptor reference");
    let descriptor = document
        .get_object(descriptor_ref)
        .expect("font descriptor object")
        .as_dict()
        .expect("font descriptor dictionary");
    assert!(
        descriptor.get(b"FontFile2").is_ok(),
        "default PDF font should embed bundled TTF bytes"
    );
}

fn first_page_font_dictionary(document: &Document) -> lopdf::Dictionary {
    let (_, page_id) = document
        .get_pages()
        .into_iter()
        .next()
        .expect("pdf should have a first page");
    let page = document
        .get_object(page_id)
        .expect("first page object")
        .as_dict()
        .expect("first page dictionary");
    page.get(b"Resources")
        .expect("page resources")
        .as_dict()
        .expect("resource dictionary")
        .get(b"Font")
        .expect("font resources")
        .as_dict()
        .expect("font resource dictionary")
        .clone()
}

fn resolved_base_font_name(document: &Document, fonts: &lopdf::Dictionary, key: &[u8]) -> String {
    let font_ref = fonts
        .get(key)
        .unwrap_or_else(|_| panic!("font resource {}", String::from_utf8_lossy(key)))
        .as_reference()
        .expect("font resource reference");
    let font = document
        .get_object(font_ref)
        .expect("font object")
        .as_dict()
        .expect("font dictionary");
    String::from_utf8_lossy(
        font.get(b"BaseFont")
            .expect("BaseFont")
            .as_name()
            .expect("name"),
    )
    .into_owned()
}

/// Walks the first page's content stream and returns, for every text-showing
/// operation, the font resource selected by the preceding `Tf` together with
/// the decoded text.
fn first_page_texts_by_font(pdf_bytes: &[u8]) -> Vec<(String, String)> {
    let mut current_font = String::new();
    let mut out = Vec::new();
    for operation in first_page_operations(pdf_bytes) {
        match operation.operator.as_str() {
            "Tf" => {
                current_font = operation.operands[0]
                    .as_name()
                    .map(|name| String::from_utf8_lossy(name).into_owned())
                    .expect("Tf font name");
            }
            "Tj" => {
                if let Ok(bytes) = operation.operands[0].as_str() {
                    out.push((current_font.clone(), decode_pdf_text_bytes(bytes)));
                }
            }
            _ => {}
        }
    }
    out
}

#[test]
fn render_pdf_paints_bold_runs_with_a_real_embedded_bold_face() {
    let bytes = run_styles_docx_with_run_property(r#"<w:b w:val="1"/>"#);
    let pdf = render_docx_to_pdf(&bytes, &RenderOptions::default()).expect("render pdf");
    let document = parse_document(&pdf);

    let fonts = first_page_font_dictionary(&document);
    assert!(
        fonts.get(b"F1").is_ok(),
        "regular font resource must remain"
    );
    assert!(
        fonts.get(b"F2").is_ok(),
        "a document with bold runs must add a bold font resource"
    );

    let bold_name = resolved_base_font_name(&document, &fonts, b"F2");
    let (tag, base) = bold_name
        .split_once('+')
        .expect("bold font should be subset with a tag prefix");
    assert_eq!(base, "NotoSansJP-Bold");
    assert_eq!(tag.len(), 6);
    assert!(tag.bytes().all(|b| b.is_ascii_uppercase()));

    let regular_name = resolved_base_font_name(&document, &fonts, b"F1");
    assert!(regular_name.ends_with("NotoSansJP-Regular"));

    let texts = first_page_texts_by_font(&pdf);
    assert!(
        texts
            .iter()
            .any(|(font, text)| font == "F2" && text.contains("Hello")),
        "the bold run must be drawn with the bold font: {texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|(font, text)| font == "F1" && text.contains("World")),
        "the regular run must stay on the regular font: {texts:?}"
    );
}

#[test]
fn render_pdf_without_bold_text_embeds_only_the_regular_font() {
    let bytes = fs::read(fixture_path("hello.docx")).expect("read hello fixture");
    let pdf = render_docx_to_pdf(&bytes, &RenderOptions::default()).expect("render pdf");
    let document = parse_document(&pdf);

    let fonts = first_page_font_dictionary(&document);
    assert!(fonts.get(b"F1").is_ok());
    assert!(
        fonts.get(b"F2").is_err(),
        "a document with no bold text must not pay for a bold font subset"
    );
}

#[test]
fn render_pdf_report_no_longer_flags_bold_as_unsupported_styling() {
    let bytes = run_styles_docx_with_run_property(r#"<w:b w:val="1"/>"#);
    let (_, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");
    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_RUN_STYLING")),
        "bold is painted with a real bold face now and must not warn: {:?}",
        report.warnings
    );
}

/// Like `first_page_texts_by_font`, but also captures the text matrix's
/// horizontal shear (the `c` element of `Tm`) in effect for each shown text —
/// how the renderer slants synthetic italic.
fn first_page_texts_with_skew(pdf_bytes: &[u8]) -> Vec<(String, f64, String)> {
    let mut current_font = String::new();
    let mut current_skew = 0.0_f64;
    let mut out = Vec::new();
    for operation in first_page_operations(pdf_bytes) {
        match operation.operator.as_str() {
            "Tf" => {
                current_font = operation.operands[0]
                    .as_name()
                    .map(|name| String::from_utf8_lossy(name).into_owned())
                    .expect("Tf font name");
            }
            "Tm" => {
                current_skew = f64::from(operation.operands[2].as_float().expect("Tm c"));
            }
            "Tj" => {
                if let Ok(bytes) = operation.operands[0].as_str() {
                    out.push((
                        current_font.clone(),
                        current_skew,
                        decode_pdf_text_bytes(bytes),
                    ));
                }
            }
            _ => {}
        }
    }
    out
}

const SYNTHETIC_OBLIQUE_SHEAR: f64 = 0.25;

#[test]
fn render_pdf_slants_italic_runs_with_a_synthetic_oblique_shear() {
    let bytes = run_styles_docx_with_run_property(r#"<w:i w:val="1"/>"#);
    let pdf = render_docx_to_pdf(&bytes, &RenderOptions::default()).expect("render pdf");

    let runs = first_page_texts_with_skew(&pdf);
    let italic_run = runs
        .iter()
        .find(|(_, _, text)| text.contains("Hello"))
        .expect("italic run drawn");
    assert!(
        (italic_run.1 - SYNTHETIC_OBLIQUE_SHEAR).abs() < 1e-6,
        "the italic run must be slanted by Chromium's synthetic oblique shear: {runs:?}"
    );
    let upright_run = runs
        .iter()
        .find(|(_, _, text)| text.contains("World"))
        .expect("upright run drawn");
    assert!(
        upright_run.1.abs() < 1e-9,
        "the upright run must stay unslanted: {runs:?}"
    );
}

#[test]
fn render_pdf_combines_bold_face_and_italic_shear() {
    let bytes = run_styles_docx_with_run_property(r#"<w:b w:val="1"/><w:i w:val="1"/>"#);
    let pdf = render_docx_to_pdf(&bytes, &RenderOptions::default()).expect("render pdf");

    let runs = first_page_texts_with_skew(&pdf);
    let bold_italic_run = runs
        .iter()
        .find(|(_, _, text)| text.contains("Hello"))
        .expect("bold-italic run drawn");
    assert_eq!(
        bold_italic_run.0, "F2",
        "bold italic keeps the bold face: {runs:?}"
    );
    assert!(
        (bold_italic_run.1 - SYNTHETIC_OBLIQUE_SHEAR).abs() < 1e-6,
        "bold italic is slanted too: {runs:?}"
    );
}

#[test]
fn render_pdf_report_no_longer_flags_italic_as_unsupported_styling() {
    let bytes = run_styles_docx_with_run_property(r#"<w:i w:val="1"/>"#);
    let (_, report) =
        render_docx_to_pdf_with_report(&bytes, &RenderOptions::default()).expect("render pdf");
    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.starts_with("PDF_UNSUPPORTED_RUN_STYLING")),
        "italic is painted as a synthetic oblique now and must not warn: {:?}",
        report.warnings
    );
}
