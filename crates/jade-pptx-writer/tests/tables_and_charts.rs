//! Tables, charts and SmartArt in real decks (Apache POI's test data) edit
//! as in PowerPoint: a table's cells take text, a table resizes with its
//! columns and rows scaled to fill the new frame, a chart resizes as its
//! frame does, and a SmartArt diagram keeps its size, since its saved drawing
//! would not follow.

use std::io::Read;
use std::path::PathBuf;

use jade_pptx_writer::{editable_shapes, table_cells, EditSession, TableChange};

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

/// The frame's extent, from the slide's `p:xfrm`.
fn frame_extent(bytes: &[u8], id: u32) -> (i64, i64) {
    editable_shapes(bytes, 0)
        .unwrap()
        .into_iter()
        .find(|shape| shape.id == id)
        .unwrap()
        .transform
        .unwrap()
        .extent_emu
}

#[test]
fn a_row_inserted_below_is_empty_formatted_like_its_neighbour_and_grows_the_table() {
    let bytes = deck("table_test.pptx");
    let before = texts(&table_cells(&bytes, 0, 4).unwrap());
    let (width, height) = frame_extent(&bytes, 4);
    let mut session = EditSession::open(&bytes).unwrap();
    // Below the header: formatted like the row it pushes down.
    assert!(session
        .change_table(0, 4, TableChange::InsertRow(1))
        .unwrap());
    let saved = session.bytes().to_vec();
    let after = texts(&table_cells(&saved, 0, 4).unwrap());
    assert_eq!(after.len(), before.len() + 1);
    assert_eq!(after[0], before[0]);
    assert_eq!(after[1], vec![String::new(); before[0].len()]);
    assert_eq!(after[2..], before[1..]);
    // The frame grows by the row's height, the columns stay.
    let slide = part(&saved, "ppt/slides/slide1.xml");
    let rows = numbers(&slide, "<a:tr h");
    assert_eq!(rows[1], rows[0]);
    assert_eq!(frame_extent(&saved, 4), (width, height + rows[1]));
    assert_eq!(numbers(&slide, "<a:gridCol w").iter().sum::<i64>(), width);
    // No row id is repeated: PowerPoint repairs a deck that does.
    let ids: Vec<&str> = slide.split("rowId val=\"").skip(1).collect();
    let unique: std::collections::BTreeSet<_> =
        ids.iter().map(|rest| rest.split('"').next()).collect();
    assert_eq!(unique.len(), ids.len());
    EditSession::open(&saved).expect("the saved deck opens");
    drawn_text(&saved, 0);

    assert!(session.undo());
    assert_eq!(texts(&table_cells(session.bytes(), 0, 4).unwrap()), before);
}

#[test]
fn a_row_deleted_shrinks_the_table_by_its_height() {
    let bytes = deck("table_test.pptx");
    let before = texts(&table_cells(&bytes, 0, 4).unwrap());
    let (width, height) = frame_extent(&bytes, 4);
    let mut session = EditSession::open(&bytes).unwrap();
    // Row 3 is the tall one.
    assert!(session
        .change_table(0, 4, TableChange::DeleteRow(3))
        .unwrap());
    let after = texts(&table_cells(session.bytes(), 0, 4).unwrap());
    let mut expected = before.clone();
    expected.remove(3);
    assert_eq!(after, expected);
    assert_eq!(
        frame_extent(session.bytes(), 4),
        (width, height - 1_048_174)
    );
}

#[test]
fn a_column_inserted_or_deleted_moves_the_tables_width_by_the_column() {
    let bytes = deck("table_test.pptx");
    let before = texts(&table_cells(&bytes, 0, 4).unwrap());
    let (width, height) = frame_extent(&bytes, 4);
    let mut session = EditSession::open(&bytes).unwrap();
    // Left of the last column: formatted, and as wide, as it.
    assert!(session
        .change_table(0, 4, TableChange::InsertColumn(2))
        .unwrap());
    let after = texts(&table_cells(session.bytes(), 0, 4).unwrap());
    for (row, cells) in after.iter().enumerate() {
        assert_eq!(cells.len(), 4);
        assert_eq!(cells[2], "");
        assert_eq!(cells[..2], before[row][..2]);
        assert_eq!(cells[3], before[row][2]);
    }
    let slide = part(session.bytes(), "ppt/slides/slide1.xml");
    assert_eq!(
        numbers(&slide, "<a:gridCol w"),
        [1_589_024, 3_829_642, 2_709_333, 2_709_333]
    );
    assert_eq!(
        frame_extent(session.bytes(), 4),
        (width + 2_709_333, height)
    );
    let ids: Vec<&str> = slide.split("colId val=\"").skip(1).collect();
    let unique: std::collections::BTreeSet<_> =
        ids.iter().map(|rest| rest.split('"').next()).collect();
    assert_eq!(unique.len(), ids.len());
    drawn_text(session.bytes(), 0);

    assert!(session
        .change_table(0, 4, TableChange::DeleteColumn(0))
        .unwrap());
    let after = texts(&table_cells(session.bytes(), 0, 4).unwrap());
    assert_eq!(after[0].len(), 3);
    assert_eq!(after[0][0], before[0][1]);
    assert_eq!(
        frame_extent(session.bytes(), 4),
        (width + 2_709_333 - 1_589_024, height)
    );
}

#[test]
fn a_change_that_would_cut_a_merge_is_refused() {
    let bytes = deck("table_test2.pptx");
    let mut session = EditSession::open(&bytes).unwrap();
    // Header2 spans columns 1 and 2.
    for change in [
        TableChange::DeleteColumn(1),
        TableChange::DeleteColumn(2),
        TableChange::InsertColumn(2),
    ] {
        assert!(session.change_table(0, 4, change).is_err(), "{change:?}");
    }
    assert!(!session.is_dirty());
    // Left of the merge, or a row under the header, is fine.
    assert!(session
        .change_table(0, 4, TableChange::InsertColumn(1))
        .unwrap());
    assert!(session
        .change_table(0, 4, TableChange::InsertRow(1))
        .unwrap());
    let cells = table_cells(session.bytes(), 0, 4).unwrap();
    assert_eq!(cells[0][2].text, "Header2");
    assert!(cells[0][3].merged);
    EditSession::open(session.bytes()).expect("the saved deck opens");
}

#[test]
fn the_last_row_and_column_stay() {
    let bytes = deck("table-with-different-font-colors.pptx");
    let mut session = EditSession::open(&bytes).unwrap();
    for _ in 0..3 {
        assert!(session
            .change_table(0, 4, TableChange::DeleteRow(0))
            .unwrap());
        assert!(session
            .change_table(0, 4, TableChange::DeleteColumn(0))
            .unwrap());
    }
    assert!(session
        .change_table(0, 4, TableChange::DeleteRow(0))
        .is_err());
    assert!(session
        .change_table(0, 4, TableChange::DeleteColumn(0))
        .is_err());
    assert_eq!(
        texts(&table_cells(session.bytes(), 0, 4).unwrap()),
        [["Text 16"]]
    );
    // Out of range is refused, not clamped.
    assert!(session
        .change_table(0, 4, TableChange::InsertRow(5))
        .is_err());
}

/// The `<a:tc ...>` tags of row `row`, in order.
fn cell_tags(slide: &str, row: usize) -> Vec<String> {
    let rows: Vec<&str> = slide.split("<a:tr ").skip(1).collect();
    rows[row]
        .split("<a:tc")
        .skip(1)
        .filter(|rest| rest.starts_with('>') || rest.starts_with(' '))
        .map(|rest| format!("<a:tc{}", &rest[..=rest.find('>').unwrap()]))
        .collect()
}

#[test]
fn cells_merge_into_the_first_with_their_text_as_paragraphs() {
    let bytes = deck("table-with-different-font-colors.pptx");
    let mut session = EditSession::open(&bytes).unwrap();
    // Rows 1–2, columns 1–2: Text 6, Text 7 / Text 10, Text 11.
    assert!(session
        .change_table(
            0,
            4,
            TableChange::MergeCells {
                row: 1,
                column: 1,
                rows: 2,
                columns: 2,
            },
        )
        .unwrap());
    let cells = table_cells(session.bytes(), 0, 4).unwrap();
    assert_eq!(cells[1][1].text, "Text 6\nText 7\nText 10\nText 11");
    assert!(!cells[1][1].merged);
    for (row, column) in [(1, 2), (2, 1), (2, 2)] {
        assert!(cells[row][column].merged, "({row}, {column})");
        assert_eq!(cells[row][column].text, "");
    }
    // As PowerPoint writes it.
    let slide = part(session.bytes(), "ppt/slides/slide1.xml");
    let first = cell_tags(&slide, 1);
    assert!(first[1].contains("gridSpan=\"2\"") && first[1].contains("rowSpan=\"2\""));
    assert!(first[2].contains("hMerge=\"1\""));
    let second = cell_tags(&slide, 2);
    assert!(second[1].contains("vMerge=\"1\""));
    assert!(second[2].contains("hMerge=\"1\"") && second[2].contains("vMerge=\"1\""));
    // The rest stay, the deck reopens and draws the joined text once.
    assert_eq!(cells[0][0].text, "Text 1");
    assert_eq!(cells[3][3].text, "Text 16");
    EditSession::open(session.bytes()).expect("the saved deck opens");
    let drawn = drawn_text(session.bytes(), 0);
    assert_eq!(drawn.matches("Text 11").count(), 1, "{drawn}");

    assert!(session.undo());
    assert_eq!(
        table_cells(session.bytes(), 0, 4).unwrap()[2][2].text,
        "Text 11"
    );
}

#[test]
fn a_merge_takes_in_whole_merges_but_never_cuts_one() {
    let bytes = deck("table_test2.pptx");
    let mut session = EditSession::open(&bytes).unwrap();
    let merge = |row, column, rows, columns| TableChange::MergeCells {
        row,
        column,
        rows,
        columns,
    };
    // Header2 spans columns 1–2 and Header3 columns 3–4: a range starting
    // or ending inside either is refused, as is a single cell.
    for change in [merge(0, 2, 1, 2), merge(0, 1, 1, 2 + 1), merge(0, 0, 1, 1)] {
        assert!(session.change_table(0, 4, change).is_err(), "{change:?}");
    }
    assert!(!session.is_dirty());
    // Both whole: one cell across columns 1–4. (Header3 is vertical text,
    // a line break after each letter.)
    let before = table_cells(&bytes, 0, 4).unwrap();
    assert!(session.change_table(0, 4, merge(0, 1, 1, 4)).unwrap());
    let cells = table_cells(session.bytes(), 0, 4).unwrap();
    assert_eq!(
        cells[0][1].text,
        format!("{}\n{}", before[0][1].text, before[0][3].text)
    );
    assert!(cells[0][1].text.starts_with("Header2\nH\ne"));
    assert!((2..5).all(|column| cells[0][column].merged));
    assert!(!cells[0][5].merged);
    EditSession::open(session.bytes()).expect("the saved deck opens");
}

#[test]
fn a_merged_cell_splits_back_into_its_cells() {
    let bytes = deck("table_test2.pptx");
    let mut session = EditSession::open(&bytes).unwrap();
    let before = table_cells(&bytes, 0, 4).unwrap();
    assert_eq!((before[0][1].row_span, before[0][1].column_span), (1, 2));
    assert_eq!((before[2][3].row_span, before[2][3].column_span), (3, 1));
    assert_eq!((before[0][0].row_span, before[0][0].column_span), (1, 1));
    // A cell that is not merged has nothing to split.
    assert!(session
        .change_table(0, 4, TableChange::SplitCell { row: 0, column: 0 })
        .is_err());
    // Header2's merge, and the three-row merge below.
    assert!(session
        .change_table(0, 4, TableChange::SplitCell { row: 0, column: 1 })
        .unwrap());
    assert!(session
        .change_table(0, 4, TableChange::SplitCell { row: 2, column: 3 })
        .unwrap());
    let cells = table_cells(session.bytes(), 0, 4).unwrap();
    assert_eq!(cells[0][1].text, "Header2");
    assert!(!cells[0][2].merged);
    assert!((2..5).all(|row| !cells[row][3].merged));
    // The text stays in the first cell, as PowerPoint leaves it.
    assert!(cells[2][3].text.starts_with("eirmod"));
    assert_eq!(cells[3][3].text, "");
    let slide = part(session.bytes(), "ppt/slides/slide1.xml");
    assert!(!cell_tags(&slide, 0)[1].contains("gridSpan"));
    assert!(!cell_tags(&slide, 3)[3].contains("vMerge"));
    EditSession::open(session.bytes()).expect("the saved deck opens");
}
