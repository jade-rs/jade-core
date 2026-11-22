//! Table rows are moved out of the prepared table while it is drawn, not
//! deep-copied. These tests pin the painted output, the resize metadata and
//! the recorded cell layouts against the copying path, and check that rows
//! which fit on their page are never copied.
use super::recording_host::RecordingHost;
use super::*;
use jade_docx_reader::document::{Body, Table, TableCell, TableRow};
use jade_painter::recording::{Op, RecordingPainter};

fn run(text: &str) -> Run {
    Run {
        text: text.to_string(),
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

fn paragraph(text: &str) -> BodyElement {
    BodyElement::from_paragraph(Paragraph {
        runs: vec![run(text)],
        properties: None,
        node_id: None,
        comment_ranges: Vec::new(),
        comment_markers: Vec::new(),
    })
}

/// `rows` x 4 cells. With `merged`, the first column merges every three
/// rows. With `tall_row`, one body row holds enough paragraphs to split
/// over pages.
fn table(rows: usize, header: bool, merged: bool, tall_row: Option<usize>) -> Table {
    let rows = (0..rows)
        .map(|row| TableRow {
            is_header: header && row == 0,
            cells: (0..4)
                .map(|column| {
                    let paragraphs = if tall_row == Some(row) && column == 1 {
                        120
                    } else {
                        1
                    };
                    TableCell {
                        blocks: (0..paragraphs)
                            .map(|line| {
                                if row == 0 && column == 0 {
                                    paragraph("HEADERMARK")
                                } else {
                                    paragraph(&format!(
                                        "Row {row} column {column} line {line} text"
                                    ))
                                }
                            })
                            .collect(),
                        vertical_merge: (merged && column == 0 && row > 0)
                            .then(|| if row % 3 == 1 { "restart" } else { "continue" }.to_string()),
                        ..Default::default()
                    }
                })
                .collect(),
            ..Default::default()
        })
        .collect();
    Table {
        rows,
        ..Default::default()
    }
}

fn document(table: Table) -> Document {
    Document {
        body: Some(Body {
            elements: vec![
                paragraph("Before the table"),
                BodyElement::Table(Box::new(table)),
                paragraph("After the table"),
            ],
        }),
        ..Default::default()
    }
}

struct Rendered {
    ops: Vec<Op>,
    metadata: String,
    cells: String,
    pages: usize,
}

fn render(document: &Document) -> Rendered {
    let painter = RecordingPainter::new();
    let mut renderer = CanvasRenderer::with_host(
        Box::new(RecordingHost {
            painter: painter.clone(),
        }),
        false,
    )
    .expect("paged recording renderer");
    renderer
        .render(document, &Options::default())
        .expect("render");
    Rendered {
        ops: painter.ops(),
        metadata: format!("{:?}", renderer.table_resize_metadata()),
        cells: format!("{:?}", renderer.cell_layouts_debug()),
        pages: renderer.resolved_total_page_count(),
    }
}

fn timed_renders(document: &Document) -> std::time::Duration {
    let mut renderer = super::recording_host::recording_renderer();
    let options = Options::default();
    renderer.render(document, &options).expect("warm up");
    let start = std::time::Instant::now();
    for _ in 0..20 {
        renderer.render(document, &options).expect("render");
    }
    start.elapsed()
}

/// 20 renders of a 300x4 table through the copying path (the old
/// behaviour) and the moving one, best of five interleaved rounds:
/// `cargo test -p jade-docx-editor --lib --release table_row_clone_timing -- --ignored --show-output`
#[test]
#[ignore]
fn table_row_clone_timing() {
    for merged in [false, true] {
        let document = document(table(300, true, merged, None));
        let mut best = [std::time::Duration::MAX; 2];
        for _ in 0..5 {
            for (slot, copy) in [(0, true), (1, false)] {
                table::prepared_row_moves::set_force_copies(copy);
                best[slot] = best[slot].min(timed_renders(&document));
            }
        }
        table::prepared_row_moves::set_force_copies(false);
        let [copied, moved] = best;
        eprintln!(
            "TIMING 300x4 merged={merged}: copying {copied:?}, moving {moved:?} for 20 renders ({:.1}% faster)",
            (1.0 - moved.as_secs_f64() / copied.as_secs_f64()) * 100.0
        );
    }
}

/// Draws `document` through the copying path, then the moving one, and
/// returns both with the deep row copies the moving one made.
fn render_both(document: &Document) -> (Rendered, Rendered, usize) {
    table::prepared_row_moves::set_force_copies(true);
    let copied = render(document);
    table::prepared_row_moves::set_force_copies(false);
    table::prepared_row_moves::take_copies();
    let moved = render(document);
    (copied, moved, table::prepared_row_moves::take_copies())
}

fn assert_same(label: &str, document: &Document) -> Rendered {
    let (copied, moved, copies) = render_both(document);
    assert!(!copied.ops.is_empty(), "{label}: painted");
    assert_eq!(copied.ops.len(), moved.ops.len(), "{label}: op count");
    if let Some(index) = (0..copied.ops.len()).find(|&i| copied.ops[i] != moved.ops[i]) {
        panic!(
            "{label}: op {index} differs: {:?} vs {:?}",
            copied.ops[index], moved.ops[index]
        );
    }
    assert_eq!(copied.metadata, moved.metadata, "{label}: resize metadata");
    assert_eq!(copied.cells, moved.cells, "{label}: cell layouts");
    assert_eq!(copied.pages, moved.pages, "{label}: pages");
    assert_eq!(copies, 0, "{label}: deep row copies");
    moved
}

/// The recording host paints one glyph per call: join them to find words.
fn header_paints(rendered: &Rendered) -> usize {
    rendered
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::FillText(text, _, _) => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>()
        .matches("HEADERMARK")
        .count()
}

#[test]
fn large_table_draws_the_same_without_row_copies() {
    for merged in [false, true] {
        for header in [false, true] {
            let label = format!("300x4 merged={merged} header={header}");
            let rendered = assert_same(&label, &document(table(300, header, merged, None)));
            assert!(rendered.pages > 3, "{label}: {} pages", rendered.pages);
        }
    }
}

#[test]
fn split_row_with_repeated_header_draws_the_same_without_row_copies() {
    for merged in [false, true] {
        let label = format!("split merged={merged}");
        let rendered = assert_same(&label, &document(table(40, true, merged, Some(20))));
        assert!(rendered.pages >= 3, "{label}: {} pages", rendered.pages);
        assert_eq!(
            header_paints(&rendered),
            rendered.pages,
            "{label}: the header row repeats on every page"
        );
    }
}
