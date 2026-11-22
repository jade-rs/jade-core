//! Header and footer stories in a long document: a page that does not draw
//! its story reuses the story's height instead of laying it out again, and
//! the document paints and hit-tests exactly as when every page lays it out.

use super::recording_host::RecordingHost;
use super::section_tests::build_docx_with_body;
use super::*;
use crate::word_document::WordDocument;
use jade_painter::recording::{Op, RecordingPainter};

const PAGES: usize = 200;

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
        text_controls: None,
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

fn paragraph_of(runs: Vec<Run>) -> BodyElement {
    BodyElement::from_paragraph(Paragraph {
        runs,
        properties: None,
        node_id: None,
        comment_ranges: Vec::new(),
        comment_markers: Vec::new(),
    })
}

fn paragraph(text: &str) -> BodyElement {
    paragraph_of(vec![run(text)])
}

/// A logo: an inline picture carried as a data URI, as loaded stories hold it.
fn picture() -> Run {
    let mut logo = run("");
    logo.image = Some(RunImage {
        data_uri: Some(format!(
            "data:image/png;base64,{}",
            "iVBORw0KGgo".repeat(256)
        )),
        width_px: Some(48.0),
        height_px: Some(24.0),
        ..RunImage::default()
    });
    logo
}

fn table(label: &str) -> BodyElement {
    let cell = |text: String| TableCell {
        blocks: vec![paragraph(&text)],
        ..TableCell::default()
    };
    BodyElement::from_table(Table {
        rows: (0..2)
            .map(|row| TableRow {
                cells: (0..2)
                    .map(|column| cell(format!("{label} r{row}c{column}")))
                    .collect(),
                ..TableRow::default()
            })
            .collect(),
        columns: vec![
            TableColumn { width: Some(120.0) },
            TableColumn { width: Some(120.0) },
        ],
        ..Table::default()
    })
}

/// A letterhead: a table and a paragraph holding the logo.
fn letterhead(label: &str) -> Body {
    Body {
        elements: vec![
            table(label),
            paragraph_of(vec![picture(), run(&format!(" {label} logo"))]),
        ],
    }
}

/// A taller letterhead, so the even header's height differs from the
/// default one's.
fn tall_letterhead(label: &str) -> Body {
    let mut body = letterhead(label);
    body.elements
        .push(paragraph(&format!("{label} address line")));
    body
}

fn text_story(text: &str) -> Body {
    Body {
        elements: vec![paragraph(text)],
    }
}

/// `PAGES` pages, one paragraph each, in two sections whose page boxes and
/// header distances differ.
fn long_document() -> Rc<Document> {
    let mut body = String::new();
    let page_break = r#"<w:r><w:br w:type="page"/></w:r>"#;
    let first_section = r#"<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/><w:titlePg/></w:sectPr>"#;
    let second_section = r#"<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1800" w:right="2160" w:bottom="1440" w:left="2160" w:header="900" w:footer="600" w:gutter="0"/><w:titlePg/></w:sectPr>"#;
    for index in 0..PAGES {
        let text = format!("<w:r><w:t>Body page {index}</w:t></w:r>");
        if index == PAGES / 2 - 1 {
            body.push_str(&format!("<w:p><w:pPr>{first_section}</w:pPr>{text}</w:p>"));
        } else if index == PAGES - 1 {
            body.push_str(&format!("<w:p>{text}</w:p>"));
        } else {
            body.push_str(&format!("<w:p>{text}{page_break}</w:p>"));
        }
    }
    body.push_str(second_section);
    let data = build_docx_with_body(&body);
    let word_doc =
        futures::executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
    Rc::clone(word_doc.document.as_ref().expect("document"))
}

/// Installs default, first and even headers and footers, document-wide and
/// for each of the two sections.
fn install_stories(renderer: &mut CanvasRenderer<RecordingPainter>) {
    renderer.set_header_footer_variants(None, None, None, None, true, true);
    renderer.set_header_footer_bodies(
        Some(letterhead("Default")),
        Some(text_story("Default footer")),
        Some(letterhead("First")),
        Some(text_story("First footer")),
        Some(letterhead("Even")),
        Some(text_story("Even footer")),
    );
    let section = |label: &str| SectionHeaderFooters {
        header_body: Some(Rc::new(letterhead(&format!("{label} default")))),
        footer_body: Some(Rc::new(text_story(&format!("{label} footer")))),
        header_body_first: Some(Rc::new(text_story(&format!("{label} first header")))),
        footer_body_first: Some(Rc::new(text_story(&format!("{label} first footer")))),
        header_body_even: Some(Rc::new(tall_letterhead(&format!("{label} even")))),
        footer_body_even: Some(Rc::new(text_story(&format!("{label} even footer")))),
        title_page: true,
        ..SectionHeaderFooters::default()
    };
    renderer.set_section_header_footers(vec![section("One"), section("Two")]);
}

/// What reaches the page: each drawing call with the painter state it is
/// drawn under. A measuring layout sets fonts and fills and measures text but
/// draws nothing; skipping one drops those state calls, never a pixel, so
/// they are folded into the state each drawing call sees.
fn painted(ops: &[Op]) -> Vec<(Op, String)> {
    #[derive(Clone, Default)]
    struct State {
        settings: BTreeMap<&'static str, Op>,
        transforms: Vec<Op>,
    }
    let mut state = State::default();
    let mut saved: Vec<State> = Vec::new();
    let mut out = Vec::new();
    for op in ops {
        let setting = match op {
            Op::SetFillStyle(_) => Some("fill"),
            Op::SetStrokeStyle(_) => Some("stroke"),
            Op::SetLineWidth(_) => Some("line_width"),
            Op::SetLineDash(_) => Some("line_dash"),
            Op::SetShadowColor(_) => Some("shadow_color"),
            Op::SetShadowBlur(_) => Some("shadow_blur"),
            Op::SetShadowOffsetX(_) => Some("shadow_x"),
            Op::SetShadowOffsetY(_) => Some("shadow_y"),
            Op::SetBlurFilter(_) => Some("blur"),
            Op::SetFont(_) => Some("font"),
            Op::SetTextBaseline(_) => Some("baseline"),
            Op::SetGlobalAlpha(_) => Some("alpha"),
            _ => None,
        };
        if let Some(kind) = setting {
            state.settings.insert(kind, op.clone());
            continue;
        }
        match op {
            Op::Save => saved.push(state.clone()),
            Op::Restore => state = saved.pop().unwrap_or_default(),
            Op::Translate(..) | Op::Scale(..) | Op::Rotate(..) => state.transforms.push(op.clone()),
            Op::MeasureText(_) => {}
            _ => out.push((
                op.clone(),
                format!("{:?} {:?}", state.settings, state.transforms),
            )),
        }
    }
    out
}

struct Outcome {
    ops: Vec<(Op, String)>,
    boundaries: Vec<u64>,
    layouts: usize,
    page_count: usize,
}

/// A renderer on the recording host (as `recording_renderer()` builds one),
/// with the painter it records into.
struct Recording {
    renderer: CanvasRenderer<RecordingPainter>,
    painter: RecordingPainter,
}

fn recording() -> Recording {
    let painter = RecordingPainter::new();
    let renderer = CanvasRenderer::with_host(
        Box::new(RecordingHost {
            painter: painter.clone(),
        }),
        false,
    )
    .expect("paged recording renderer");
    Recording { renderer, painter }
}

fn render(recording: &mut Recording, document: &Rc<Document>) -> Outcome {
    let Recording { renderer, painter } = recording;
    painter.clear();
    renderer.hf_story_layout_count = 0;
    renderer
        .render_with_numbering_rc(
            Rc::clone(document),
            &Options::default(),
            None,
            None,
            None,
            None,
        )
        .expect("render");
    let page_count = renderer
        .page_section_indices
        .len()
        .max(renderer.current_page_number);
    Outcome {
        ops: painted(&painter.ops()),
        boundaries: (0..PAGES)
            .map(|page| renderer.header_edit_boundary_for_page(page).to_bits())
            .collect(),
        layouts: renderer.hf_story_layout_count,
        page_count,
    }
}

fn renderer(memo: bool, visible: Option<std::ops::RangeInclusive<usize>>) -> Recording {
    let mut recording = recording();
    recording.renderer.hf_story_memo_disabled = !memo;
    recording.renderer.set_visible_page_range(visible);
    install_stories(&mut recording.renderer);
    recording
}

/// The window drawn below straddles the section change, so drawn pages show
/// a section's first header, its even header and the other section's.
const WINDOW: std::ops::RangeInclusive<usize> = 98..=103;

#[test]
fn undrawn_pages_reuse_their_story_height_and_paint_the_same() {
    let document = long_document();
    for visible in [None, Some(WINDOW)] {
        let mut memo = renderer(true, visible.clone());
        let mut layout = renderer(false, visible.clone());
        let with_memo = render(&mut memo, &document);
        let without = render(&mut layout, &document);
        assert!(
            with_memo.page_count >= PAGES,
            "{} pages",
            with_memo.page_count
        );
        if with_memo.ops != without.ops {
            let at = with_memo
                .ops
                .iter()
                .zip(&without.ops)
                .position(|(a, b)| a != b)
                .unwrap_or(with_memo.ops.len().min(without.ops.len()));
            panic!(
                "painted ops match with {visible:?}: {} vs {} ops, first difference at {at}: {:?} vs {:?}",
                with_memo.ops.len(),
                without.ops.len(),
                &with_memo.ops[at.saturating_sub(3)..(at + 4).min(with_memo.ops.len())],
                &without.ops[at.saturating_sub(3)..(at + 4).min(without.ops.len())],
            );
        }
        assert_eq!(
            with_memo.boundaries, without.boundaries,
            "header edit boundaries match with {visible:?}"
        );
        // Section one's and section two's first pages have their own
        // (first) header; even pages differ from odd ones.
        let distinct = with_memo
            .boundaries
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        assert!(
            distinct >= 3,
            "first/even/default headers differ: {distinct}"
        );

        if visible.is_some() {
            assert!(
                without.layouts >= PAGES,
                "every page lays its header out without the memo: {}",
                without.layouts
            );
            // Six drawn pages lay out a header and a footer each and measure
            // the footer; undrawn pages lay out each story once per section.
            assert!(
                with_memo.layouts <= 40,
                "story layouts stay bounded: {}",
                with_memo.layouts
            );
            // A repaint of the same window is bounded the same way.
            let repaint = render(&mut memo, &document);
            assert!(
                repaint.layouts <= 40,
                "repaint layouts: {}",
                repaint.layouts
            );
            assert!(repaint.ops == without.ops, "a repaint paints the same");
            assert_eq!(repaint.boundaries, without.boundaries);
        }
    }
}

#[test]
fn a_story_depending_on_its_page_is_laid_out_on_every_page() {
    let mut page_field = run("1");
    page_field.field = Some(RunField {
        kind: RunFieldKind::PageNumber,
        source_field_id: None,
        cached_text: Some("1".into()),
        resolved_value: None,
        word_mac_body_result: None,
    });
    let with_field = Body {
        elements: vec![paragraph_of(vec![run("Page "), page_field])],
    };
    assert!(!hf_story_is_page_independent(&with_field.elements));
    assert!(hf_story_is_page_independent(&letterhead("Logo").elements));

    let mut floating = picture();
    if let Some(image) = floating.image.as_mut() {
        image.layout.kind = RunImageKind::Floating;
    }
    assert!(!hf_story_is_page_independent(&[paragraph_of(vec![
        floating
    ])]));

    let document = long_document();
    let mut recording = recording();
    recording.renderer.set_visible_page_range(Some(1..=1));
    recording
        .renderer
        .set_header_footer_bodies(Some(with_field), None, None, None, None, None);
    let outcome = render(&mut recording, &document);
    assert!(
        outcome.layouts >= PAGES,
        "a PAGE field is laid out per page: {}",
        outcome.layouts
    );
}

/// Cold layout of the long document with one drawn page, with and without
/// the story-height memo. `cargo test -p jade-docx-editor --lib --release
/// header_footer_story_cost -- --ignored --nocapture`.
#[test]
#[ignore]
fn header_footer_story_cold_layout_timing() {
    let document = long_document();
    for memo in [false, true] {
        let mut samples = Vec::new();
        let mut layouts = 0;
        for _ in 0..7 {
            let mut renderer = renderer(memo, Some(1..=1));
            let started = std::time::Instant::now();
            layouts = render(&mut renderer, &document).layouts;
            samples.push(started.elapsed());
        }
        samples.sort();
        println!(
            "story memo {}: median cold layout {:?} over {} samples, {layouts} story layouts",
            if memo { "on" } else { "off" },
            samples[samples.len() / 2],
            samples.len()
        );
    }
}
