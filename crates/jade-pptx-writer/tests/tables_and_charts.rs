//! Tables, charts and SmartArt in real decks (Apache POI's test data) edit
//! as in PowerPoint: a table's cells take text, a table resizes with its
//! columns and rows scaled to fill the new frame, a chart resizes as its
//! frame does, and a SmartArt diagram keeps its size, since its saved drawing
//! would not follow.

use std::io::Read;
use std::path::PathBuf;

use jade_pptx_writer::{editable_shapes, table_cells, EditSession};

fn deck(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../public/fixtures/poi")
            .join(name),
    )
    .expect("deck")
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    let mut file = archive.by_name(name).expect("part");
    let mut text = String::new();
    file.read_to_string(&mut text).expect("utf-8");
    text
}

/// The numbers after `key="` in `xml`, in order.
fn numbers(xml: &str, key: &str) -> Vec<i64> {
    xml.split(&format!("{key}=\""))
        .skip(1)
        .filter_map(|rest| rest.split('"').next()?.parse().ok())
        .collect()
}

fn texts(cells: &[Vec<jade_pptx_writer::TableCell>]) -> Vec<Vec<String>> {
    cells
        .iter()
        .map(|row| row.iter().map(|cell| cell.text.clone()).collect())
        .collect()
}

/// The text drawing slide `index` paints.
fn drawn_text(bytes: &[u8], index: usize) -> String {
    let deck = jade_pptx_reader::read_deck(bytes).expect("the deck reads");
    let painter = jade_painter::recording::RecordingPainter::new();
    jade_pptx_render::render_slide(
        &painter,
        &jade_pptx_layout::layout_deck_slide(&deck, index, &jade_pptx_layout::HeuristicMeasure)
            .expect("the slide lays out"),
        &Default::default(),
    )
    .expect("the slide renders");
    painter
        .ops()
        .into_iter()
        .filter_map(|op| match op {
            jade_painter::recording::Op::FillText(text, _, _) => Some(text),
            _ => None,
        })
        .collect()
}

#[test]
fn a_table_cell_takes_text_and_the_rest_stay() {
    let bytes = deck("table-with-different-font-colors.pptx");
    let before = texts(&table_cells(&bytes, 0, 4).unwrap());
    assert_eq!(before[0], ["Text 1", "Text 2", "Text 3", "Text 4"]);
    assert_eq!(before[2][1], "Text 10");

    let mut session = EditSession::open(&bytes).unwrap();
    assert!(session
        .set_table_cell_text(0, 4, 2, 1, "売上\n前年比")
        .unwrap());
    let saved = session.bytes().to_vec();
    let after = texts(&table_cells(&saved, 0, 4).unwrap());
    let mut expected = before.clone();
    expected[2][1] = "売上\n前年比".into();
    assert_eq!(after, expected);
    // The cell kept its formatting: its runs still carry their colour.
    let slide = part(&saved, "ppt/slides/slide1.xml");
    let cell = &slide[slide.find("売上").unwrap() - 400..slide.find("前年比").unwrap()];
    assert!(cell.contains("<a:rPr"), "{cell}");
    EditSession::open(&saved).expect("the saved deck opens");
    // And the slide draws the new text where the old was.
    let drawn = drawn_text(&saved, 0);
    assert!(
        drawn.contains("売上") && drawn.contains("前年比"),
        "{drawn}"
    );
    assert!(!drawn.contains("Text 10"), "{drawn}");

    assert!(session.undo());
    assert_eq!(texts(&table_cells(session.bytes(), 0, 4).unwrap()), before);
}

#[test]
fn a_merged_cell_is_edited_at_its_first_cell_only() {
    let bytes = deck("table_test2.pptx");
    let cells = table_cells(&bytes, 0, 4).unwrap();
    assert_eq!(cells[0][0].text, "Header1");
    // Header2 spans two columns: the second is part of the merge.
    let merged: Vec<_> = cells[0].iter().map(|cell| cell.merged).collect();
    assert_eq!(merged[..3], [false, false, true]);
    let mut session = EditSession::open(&bytes).unwrap();
    assert!(session.set_table_cell_text(0, 4, 0, 2, "x").is_err());
    assert!(session.set_table_cell_text(0, 4, 0, 1, "見出し").unwrap());
    assert_eq!(
        table_cells(session.bytes(), 0, 4).unwrap()[0][1].text,
        "見出し"
    );
}

#[test]
fn a_table_resizes_with_its_columns_and_rows_filling_the_frame() {
    let bytes = deck("table_test.pptx");
    let shape = editable_shapes(&bytes, 0)
        .unwrap()
        .into_iter()
        .find(|shape| shape.id == 4)
        .unwrap();
    assert_eq!(shape.kind, "table");
    assert!(shape.can_resize);
    let mut transform = shape.transform.unwrap();
    let (cx, cy) = transform.extent_emu;
    transform.extent_emu = (cx * 3 / 2, cy / 2);

    let mut session = EditSession::open(&bytes).unwrap();
    assert!(session.set_shape_transform(0, 4, transform).unwrap());
    let slide = part(session.bytes(), "ppt/slides/slide1.xml");
    let columns = numbers(&slide, "<a:gridCol w");
    let rows = numbers(&slide, "<a:tr h");
    assert_eq!(columns.iter().sum::<i64>(), cx * 3 / 2);
    assert_eq!(rows.iter().sum::<i64>(), cy / 2);
    // In proportion: the widest column stays the widest.
    assert_eq!(columns.len(), 3);
    assert!(columns[1] > columns[2] && columns[2] > columns[0]);
    let moved = editable_shapes(session.bytes(), 0)
        .unwrap()
        .into_iter()
        .find(|shape| shape.id == 4)
        .unwrap();
    assert_eq!(moved.transform.unwrap().extent_emu, (cx * 3 / 2, cy / 2));
}

#[test]
fn a_chart_resizes_and_a_diagram_keeps_its_size() {
    let bytes = deck("chart-slide-bg.pptx");
    let chart = editable_shapes(&bytes, 0)
        .unwrap()
        .into_iter()
        .find(|shape| shape.kind == "chart")
        .unwrap();
    assert!(chart.can_resize);
    let mut transform = chart.transform.unwrap();
    transform.extent_emu.0 /= 2;
    let mut session = EditSession::open(&bytes).unwrap();
    assert!(session.set_shape_transform(0, chart.id, transform).unwrap());
    let resized = editable_shapes(session.bytes(), 0)
        .unwrap()
        .into_iter()
        .find(|shape| shape.id == chart.id)
        .unwrap();
    assert_eq!(resized.transform.unwrap().extent_emu, transform.extent_emu);

    let diagram = editable_shapes(&deck("SmartArt.pptx"), 0)
        .unwrap()
        .into_iter()
        .find(|shape| shape.kind == "diagram")
        .unwrap();
    assert!(diagram.can_move && !diagram.can_resize);
}
