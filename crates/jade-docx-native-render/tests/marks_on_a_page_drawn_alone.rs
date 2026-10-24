//! A page drawn on its own shows the formatting marks a full render draws.
//!
//! A lazy repaint draws only the pages it needs. A paragraph that starts on a
//! page the repaint skips and breaks onto the page it draws used to keep the
//! skipped page's "do not draw" for its lines: its text came back, the square
//! in its full-width space did not.

use futures::executor::block_on;
use jade_docx_editor::canvas_renderer::CanvasRenderer;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_editor::{FormattingMarkKinds, Options};
use jade_docx_native_render::SkiaRenderHost;
use jade_skia_painter::SkiaPainter;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

const PAGE_WIDTH_PX: f64 = 816.0;
const PAGE_HEIGHT_PX: f64 = 1056.0;

type Pages = Rc<RefCell<Vec<SkiaPainter>>>;

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn options() -> Options {
    let mut options = Options::default();
    options.show_formatting_marks = false;
    options.set_formatting_mark_kinds(FormattingMarkKinds {
        spaces: true,
        ..FormattingMarkKinds::default()
    });
    options
}

fn renderer() -> (CanvasRenderer<SkiaPainter>, Pages) {
    let mut host = SkiaRenderHost::new(PAGE_WIDTH_PX, PAGE_HEIGHT_PX).expect("raster surface");
    let path = repo_path("fonts/Noto_Sans_JP/static/NotoSansJP-Regular.ttf");
    for family in ["Noto Sans JP", "Times New Roman", "MS Mincho", "serif"] {
        host.register_font(family, 400, &path)
            .expect("register face");
    }
    let pages = host.pages();
    let renderer =
        CanvasRenderer::<SkiaPainter>::with_host(Box::new(host), false).expect("build renderer");
    (renderer, pages)
}

/// Single-line paragraphs with a full-width space each, enough for three
/// pages: whichever opens page 2 started its layout on page 1.
fn document() -> WordDocument {
    let bytes = std::fs::read(repo_path("public/fixtures/contract.docx")).expect("read fixture");
    let mut package = jade_opc::OpcPackage::open(&bytes).expect("open package");
    let xml = package
        .part_str("word/document.xml")
        .expect("read part")
        .expect("main document");
    let body_start = xml.find("<w:body>").expect("body") + "<w:body>".len();
    let sect = xml.rfind("<w:sectPr").expect("section properties");
    let paragraphs: String = (1..=120)
        .map(|n| format!(r#"<w:p><w:r><w:t xml:space="preserve">({n})　項目</w:t></w:r></w:p>"#))
        .collect();
    let edited = format!("{}{paragraphs}{}", &xml[..body_start], &xml[sect..]);
    package.replace("word/document.xml", edited.into_bytes());
    let bytes = package.write().expect("write package");
    block_on(WordDocument::load(&bytes, &options())).expect("parse docx")
}

fn render(word: &WordDocument, visible: Option<std::ops::RangeInclusive<usize>>) -> Vec<u8> {
    let (mut renderer, pages) = renderer();
    renderer.set_visible_page_range(visible);
    renderer
        .render_with_numbering_rc(
            word.document.clone().expect("main document"),
            &options(),
            word.numbering_part.as_ref(),
            word.header_lines.as_deref(),
            word.footer_lines.as_deref(),
            None,
        )
        .expect("render");
    assert!(
        renderer.page_count() >= 3,
        "the paragraphs fill three pages"
    );
    let pixels = pages.borrow()[1]
        .read_bgra_premultiplied()
        .expect("read pixels")
        .0;
    pixels
}

#[test]
fn page_two_drawn_alone_matches_a_full_render() {
    let word = document();
    let full = render(&word, None);
    let alone = render(&word, Some(2..=2));
    assert_eq!(full.len(), alone.len());
    let differing = full
        .chunks_exact(4)
        .zip(alone.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(
        differing, 0,
        "page 2 drawn alone differs from a full render in {differing} pixels"
    );
}
