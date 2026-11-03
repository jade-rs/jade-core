//! Word's 原稿用紙設定 drawing: the `Genko:` VML group of squares, lines or a
//! ruled frame written into the header, read back from it, taken away with
//! なし, and undone — and the text laid one character to a square, centred
//! in it, across the page and down it.

use std::io::{Read, Write};
use std::path::PathBuf;

use jade_docx_editor::canvas_renderer::CanvasRenderer;
use jade_docx_editor::commands::{CommandContext, CommandExecutor};
use jade_docx_editor::headless_api::apply_headless_commands_with_executor_to_document;
use jade_docx_editor::render_host::{CanvasRenderHost, ImageCropMapping};
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::{
    HeadlessCommand, HeadlessDocGrid, HeadlessEditRequest, HeadlessSectionLayoutPatch,
    HeadlessSectionScope,
};
use jade_docx_reader::document::{ManuscriptGrid, ManuscriptStyle};
use jade_painter::recording::{Op, RecordingPainter};
use jade_painter::{Painter, TextMetrics};
use wasm_bindgen::JsValue;

fn load_bytes(bytes: &[u8]) -> WordDocument {
    futures::executor::block_on(WordDocument::load(bytes, &Default::default())).expect("load")
}

/// Every header part of the saved package, joined.
fn headers(document: &WordDocument) -> String {
    let exported = document.export_docx().expect("export");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
    let mut all = String::new();
    for index in 1..10 {
        if let Ok(mut file) = archive.by_name(&format!("word/header{index}.xml")) {
            file.read_to_string(&mut all).expect("read");
        }
    }
    all
}

fn grid_command(style: Option<&str>, chars: u32, lines: u32) -> HeadlessEditRequest {
    HeadlessEditRequest {
        commands: vec![HeadlessCommand::SetManuscriptGrid {
            style: style.map(str::to_string),
            chars,
            lines,
            color: "#00b050".to_string(),
        }],
    }
}

fn undo(document: &mut WordDocument, executor: &mut CommandExecutor) {
    let mut selection = None;
    let mut context = CommandContext::new(document, &mut selection);
    executor.undo(&mut context).expect("undo");
}

#[test]
fn a_manuscript_grid_is_written_read_back_removed_and_undone() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/hello.docx");
    let mut document = load_bytes(&std::fs::read(path).expect("fixture"));
    assert!(document
        .document
        .as_deref()
        .unwrap()
        .manuscript_grid
        .is_none());
    let before = headers(&document);
    let mut executor = CommandExecutor::new();

    // マス目付き 20 × 20: the frame and 400 squares in the header.
    apply_headless_commands_with_executor_to_document(
        &mut document,
        &mut executor,
        None,
        &grid_command(Some("squares"), 20, 20),
    )
    .expect("apply");
    let xml = headers(&document);
    assert!(xml.contains(r#"id="Genko:A4:20:20:P:0::""#), "{xml}");
    assert_eq!(xml.matches("<v:rect ").count(), 401);
    assert!(xml.contains(r##"strokecolor="#00B050""##));
    assert_eq!(
        document.document.as_deref().unwrap().manuscript_grid,
        Some(ManuscriptGrid {
            style: ManuscriptStyle::Squares,
            chars: 20,
            lines: 20,
            color: "#00B050".to_string(),
        })
    );

    // Saved and opened again, the grid is read back from the drawing.
    let reopened = load_bytes(&document.export_docx().expect("export"));
    let read = reopened
        .document
        .as_deref()
        .unwrap()
        .manuscript_grid
        .clone();
    assert_eq!(
        read.as_ref().map(|grid| grid.style),
        Some(ManuscriptStyle::Squares)
    );
    assert_eq!(
        read.as_ref().map(|grid| (grid.chars, grid.lines)),
        Some((20, 20))
    );

    // 下線付き 20 × 10 replaces it: ten rules, no squares.
    apply_headless_commands_with_executor_to_document(
        &mut document,
        &mut executor,
        None,
        &grid_command(Some("lines"), 20, 10),
    )
    .expect("apply");
    let xml = headers(&document);
    assert_eq!(xml.matches("Genko:").count(), 1, "one grid, not two");
    assert_eq!(xml.matches("<v:line ").count(), 10);
    assert_eq!(xml.matches("<v:rect ").count(), 0);
    let reopened = load_bytes(&document.export_docx().expect("export"));
    assert_eq!(
        reopened
            .document
            .as_deref()
            .unwrap()
            .manuscript_grid
            .as_ref()
            .map(|grid| grid.style),
        Some(ManuscriptStyle::Lines)
    );

    // なし takes it away.
    apply_headless_commands_with_executor_to_document(
        &mut document,
        &mut executor,
        None,
        &grid_command(None, 20, 10),
    )
    .expect("apply");
    assert!(!headers(&document).contains("Genko:"));
    assert!(document
        .document
        .as_deref()
        .unwrap()
        .manuscript_grid
        .is_none());

    // A style it does not know is refused.
    assert!(apply_headless_commands_with_executor_to_document(
        &mut document,
        &mut executor,
        None,
        &grid_command(Some("dots"), 20, 10),
    )
    .is_err());

    undo(&mut document, &mut executor);
    assert!(headers(&document).contains("<v:line "));
    undo(&mut document, &mut executor);
    undo(&mut document, &mut executor);
    assert!(document
        .document
        .as_deref()
        .unwrap()
        .manuscript_grid
        .is_none());
    assert!(!headers(&document).contains("Genko:"));
    let _ = before;
}

/// 10.5pt full-width characters: 14px wide, with the ideographic em box
/// 0.88 em above the baseline and 0.12 em below it.
const EM_PX: f64 = 14.0;

/// A render host on the recording painter that measures every character as
/// one 10.5pt em, its font box the em box.
struct EmHost(RecordingPainter);

impl CanvasRenderHost for EmHost {
    type Painter = RecordingPainter;
    fn painter(&self) -> RecordingPainter {
        self.0.clone()
    }
    fn draw_image_data_uri(
        &self,
        _: &str,
        _: f64,
        _: f64,
        _: f64,
        _: f64,
        _: Option<ImageCropMapping>,
    ) -> bool {
        true
    }
    fn measure_text_width(&self, text: &str) -> Result<f64, JsValue> {
        Ok(self.0.measure_text_width(text).unwrap_or(0.0))
    }
    fn measure_text_metrics(&self, text: &str) -> Result<TextMetrics, JsValue> {
        let width = self.0.measure_text_width(text).unwrap_or(0.0);
        Ok(TextMetrics {
            width,
            font_bounding_box_ascent: 0.88 * EM_PX,
            font_bounding_box_descent: 0.12 * EM_PX,
            actual_bounding_box_ascent: 0.88 * EM_PX,
            actual_bounding_box_descent: 0.12 * EM_PX,
            actual_bounding_box_right: width,
        })
    }
    fn device_pixel_ratio(&self) -> f64 {
        1.0
    }
    fn update_device_pixel_ratio(&mut self) -> Result<bool, JsValue> {
        Ok(false)
    }
    fn reset_pages(&mut self) -> Result<(), JsValue> {
        Ok(())
    }
    fn configure_base_canvas(&mut self, _: f64, _: f64, _: bool, _: f64) -> Result<(), JsValue> {
        Ok(())
    }
    fn clear(&mut self, _: f64, _: bool, _: f64) -> Result<(), JsValue> {
        Ok(())
    }
    fn ensure_pageless_height(&mut self, _: f64, _: f64) -> Result<(), JsValue> {
        Ok(())
    }
    fn start_new_page(&mut self, _: f64, _: f64, _: f64) -> Result<(), JsValue> {
        Ok(())
    }
    fn device_to_css(&self, value: f64) -> f64 {
        value
    }
    fn canvas_css_size(&self) -> (f64, f64) {
        (794.0, 1123.0)
    }
    fn page_count(&self) -> usize {
        1
    }
    fn page_dimensions(&self, _: usize) -> Option<(f64, f64)> {
        Some((794.0, 1123.0))
    }
    fn take_paged_snapshots(&mut self) -> Result<Option<Vec<JsValue>>, JsValue> {
        Ok(None)
    }
    fn take_paged_snapshots_for_pages(
        &mut self,
        _: Option<&[usize]>,
    ) -> Result<Option<Vec<JsValue>>, JsValue> {
        Ok(None)
    }
    fn context(&self) -> &web_sys::CanvasRenderingContext2d {
        unreachable!("the recording host has no canvas")
    }
    fn context_mut(&mut self) -> &mut web_sys::CanvasRenderingContext2d {
        unreachable!("the recording host has no canvas")
    }
    fn create_page_bitmap(&self, _: usize) -> Result<js_sys::Promise, JsValue> {
        unreachable!("the recording host has no page bitmaps")
    }
}

/// A Japanese A4 document (10.5pt ＭＳ 明朝, Word's defaults) whose body is
/// `paragraphs`.
fn japanese_package(paragraphs: &[String]) -> Vec<u8> {
    const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
    let body: String = paragraphs
        .iter()
        .map(|text| format!("<w:p><w:r><w:t xml:space=\"preserve\">{text}</w:t></w:r></w:p>"))
        .collect();
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {NS}><w:body>{body}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1985" w:right="1701" w:bottom="1701" w:left="1701" w:header="851" w:footer="992" w:gutter="0"/><w:docGrid w:type="lines" w:linePitch="360"/></w:sectPr></w:body></w:document>"#
    );
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles {NS}><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Century" w:eastAsia="ＭＳ 明朝" w:hAnsi="Century"/><w:kern w:val="2"/><w:sz w:val="21"/><w:szCs w:val="22"/><w:lang w:val="en-US" w:eastAsia="ja-JP"/></w:rPr></w:rPrDefault><w:pPrDefault/></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="a"><w:name w:val="Normal"/><w:pPr><w:widowControl w:val="0"/><w:jc w:val="both"/></w:pPr></w:style></w:styles>"#
    );
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default();
        let mut file = |name: &str, bytes: &[u8]| {
            zip.start_file(name, options).unwrap();
            zip.write_all(bytes).unwrap();
        };
        file(
            "[Content_Types].xml",
            br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#,
        );
        file(
            "_rels/.rels",
            br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        );
        file("word/document.xml", document.as_bytes());
        file("word/styles.xml", styles.as_bytes());
        file(
            "word/_rels/document.xml.rels",
            br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#,
        );
        zip.finish().unwrap();
    }
    cursor.into_inner()
}

/// What 原稿用紙設定's OK runs for マス目付き `chars` × `lines` on A4 portrait:
/// Word's manuscript margins (72pt, 85pt) and a `snapToChars` grid whose
/// pitches fit the counts in the body box — rounded down, so that the last
/// square of a line and the last line still fit.
fn manuscript_commands(chars: u32, lines: u32, vertical: bool) -> HeadlessEditRequest {
    let (body_width, body_height) = (595.3_f32 - 170.0, 841.9_f32 - 144.0);
    let (along, across) = if vertical {
        (body_height, body_width)
    } else {
        (body_width, body_height)
    };
    let char_pitch = (along / chars as f32 * 100.0).floor() / 100.0;
    let line_pitch = (across / lines as f32 * 20.0).floor() / 20.0;
    HeadlessEditRequest {
        commands: vec![
            HeadlessCommand::SetSectionLayout {
                scope: HeadlessSectionScope::All,
                patch: Box::new(HeadlessSectionLayoutPatch {
                    width: Some(595.3),
                    height: Some(841.9),
                    landscape: Some(false),
                    margin_top: Some(72.0),
                    margin_bottom: Some(72.0),
                    margin_left: Some(85.0),
                    margin_right: Some(85.0),
                    doc_grid: Some(HeadlessDocGrid {
                        grid_type: "snapToChars".to_string(),
                        line_pitch: Some(line_pitch),
                        char_pitch: Some(char_pitch),
                    }),
                    text_direction: vertical.then(|| "tbRl".to_string()),
                    ..Default::default()
                }),
            },
            HeadlessCommand::SetManuscriptGrid {
                style: Some("squares".to_string()),
                chars,
                lines,
                color: "#FF0000".to_string(),
            },
        ],
    }
}

/// Fills every square of a `chars` × `lines` manuscript page — all but the
/// last line with one paragraph, the last with a paragraph indented by an
/// ideographic space — and checks that each character is painted in the
/// middle of the square drawn for it, within 1pt each way.
fn characters_sit_in_their_squares(platform: &str, vertical: bool, chars: u32, lines: u32) {
    jade_docx_editor::set_word_layout_platform(platform);
    const KANA: &str = "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめも";
    let kana: Vec<char> = KANA.chars().collect();
    let full = (chars * (lines - 1)) as usize;
    let first: String = (0..full).map(|index| kana[index % kana.len()]).collect();
    let last: String = std::iter::once('\u{3000}')
        .chain(kana.iter().copied().take(5))
        .collect();
    let mut document = load_bytes(&japanese_package(&[first, last]));
    let mut executor = CommandExecutor::new();
    apply_headless_commands_with_executor_to_document(
        &mut document,
        &mut executor,
        None,
        &manuscript_commands(chars, lines, vertical),
    )
    .expect("apply 原稿用紙設定");

    let painter = RecordingPainter::with_width_per_char(EM_PX);
    let mut renderer =
        CanvasRenderer::with_host(Box::new(EmHost(painter.clone())), false).expect("renderer");
    let mut options = jade_docx_editor::Options::default();
    options.show_formatting_marks = false;
    options.set_pageless(false);
    renderer
        .render(document.document.as_deref().expect("document"), &options)
        .expect("render");

    let ops = painter.ops();
    // The frame, then the squares line by line.
    let squares: Vec<(f64, f64, f64, f64)> = ops
        .iter()
        .filter_map(|op| match op {
            Op::StrokeRect(x, y, width, height) => Some((*x, *y, *width, *height)),
            _ => None,
        })
        .skip(1)
        .collect();
    assert_eq!(squares.len(), (chars * lines) as usize);
    let mut baseline = String::new();
    let mut glyphs = Vec::new();
    for op in &ops {
        match op {
            Op::SetTextBaseline(value) => baseline = value.clone(),
            Op::FillText(text, x, y) if text.chars().all(|ch| kana.contains(&ch)) => {
                assert_eq!(baseline, "alphabetic", "{text} is placed on its baseline");
                glyphs.push((text.clone(), *x, *y));
            }
            _ => {}
        }
    }
    assert_eq!(
        glyphs.len(),
        full + 5,
        "every character on the page, one to a square"
    );
    let tolerance = 96.0 / 72.0;
    for (index, (text, x, y)) in glyphs.iter().enumerate() {
        // The last paragraph starts a line of its own after its indent.
        let (line, column) = if index < full {
            (index / chars as usize, index % chars as usize)
        } else {
            (lines as usize - 1, index - full + 1)
        };
        let (square_x, square_y, width, height) = squares[line * chars as usize + column];
        let centre = (x + EM_PX * 0.5, y - EM_PX * 0.38);
        let square = (square_x + width * 0.5, square_y + height * 0.5);
        assert!(
            (centre.0 - square.0).abs() <= tolerance && (centre.1 - square.1).abs() <= tolerance,
            "{platform} {} {chars}×{lines}: {text} (line {line}, square {column}) is centred at {centre:?}, its square at {square:?}",
            if vertical { "縦書き" } else { "横書き" },
        );
    }
}

#[test]
fn horizontal_manuscript_characters_sit_in_their_squares() {
    for platform in ["mac", "windows"] {
        characters_sit_in_their_squares(platform, false, 20, 20);
        characters_sit_in_their_squares(platform, false, 20, 10);
    }
}

#[test]
fn vertical_manuscript_characters_sit_in_their_squares() {
    for platform in ["mac", "windows"] {
        characters_sit_in_their_squares(platform, true, 20, 20);
        characters_sit_in_their_squares(platform, true, 20, 10);
    }
}
