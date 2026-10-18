//! Borders are one more entry an `xf` points at: setting an edge adds a
//! `<border>` (the base one with that edge replaced) and an `xf` pointing at
//! it, reusing identical entries, and never edits what other cells share.

use jade_opc::OpcPackage;
use jade_xlsx_reader::styles::parse_styles as parse_styles_xml;
use jade_xlsx_reader::{Border, BorderEdge, Color};
use jade_xlsx_writer::{add_cell_format, BorderLine, BorderStyle, FormatChange};
use proptest::prelude::*;
use std::path::PathBuf;

fn fixture_styles() -> String {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ));
    let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let package = OpcPackage::open(&bytes).expect("package");
    package
        .part_str("xl/styles.xml")
        .expect("read")
        .expect("styles part")
}

fn line(style: BorderStyle, color: Option<&str>) -> BorderLine {
    BorderLine {
        style,
        color: color.map(str::to_string),
    }
}

fn edge(style: &str, rgb: u32) -> BorderEdge {
    BorderEdge {
        style: style.to_string(),
        color: Some(Color::Argb([
            0xFF,
            (rgb >> 16) as u8,
            (rgb >> 8) as u8,
            rgb as u8,
        ])),
    }
}

fn box_of(line: BorderLine) -> FormatChange {
    FormatChange {
        border_left: Some(line.clone()),
        border_right: Some(line.clone()),
        border_top: Some(line.clone()),
        border_bottom: Some(line),
        ..FormatChange::default()
    }
}

#[test]
fn a_box_adds_a_border_and_an_xf_and_touches_nothing_else() {
    let original = fixture_styles();
    let change = box_of(line(BorderStyle::Medium, Some("#c00000")));
    let (xml, index) = add_cell_format(&original, None, &change).expect("add");

    // The fixture has 4 borders and 11 xfs.
    assert_eq!(index, 11);
    let table = parse_styles_xml(&xml).expect("parse");
    assert_eq!(table.borders.len(), 5);
    let format = &table.cell_formats[11];
    assert_eq!(format.border_index, Some(4));
    let medium = Some(edge("medium", 0xC00000));
    assert_eq!(
        table.borders[4],
        Border {
            left: medium.clone(),
            right: medium.clone(),
            top: medium.clone(),
            bottom: medium,
            ..Border::default()
        }
    );
    assert!(xml.contains(r#"<borders count="5">"#), "{xml}");
    assert!(xml.contains(
        r#"<xf numFmtId="0" fontId="0" fillId="0" borderId="4" xfId="0" applyBorder="1"/>"#
    ));
    // Before `<borders>` and after `</cellXfs>`, the text is as it was, and
    // so is every entry already there.
    let before_borders = &original[..original.find("<borders").expect("borders")];
    assert!(xml.starts_with(before_borders));
    let after_xfs = &original[original.find("</cellXfs>").expect("xfs")..];
    assert!(xml.ends_with(after_xfs));
    let before = parse_styles_xml(&original).expect("parse");
    assert_eq!(&table.borders[..4], &before.borders[..]);
    assert_eq!(&table.cell_formats[..11], &before.cell_formats[..]);
}

#[test]
fn one_edge_replaces_only_that_edge_of_the_base_border() {
    let original = fixture_styles();
    // Style 3 wears border 1: thin 8EA9DB all round.
    let change = FormatChange {
        border_top: Some(line(BorderStyle::Double, Some("1F3864"))),
        border_left: Some(BorderLine::none()),
        ..FormatChange::default()
    };
    let (xml, index) = add_cell_format(&original, Some(3), &change).expect("add");
    let table = parse_styles_xml(&xml).expect("parse");
    let format = &table.cell_formats[index as usize];
    let border = &table.borders[format.border_index.expect("border") as usize];
    assert_eq!(border.left, None);
    assert_eq!(border.right, Some(edge("thin", 0x8EA9DB)));
    assert_eq!(border.top, Some(edge("double", 0x1F3864)));
    assert_eq!(border.bottom, Some(edge("thin", 0x8EA9DB)));
    // The rest of style 3 came along.
    assert_eq!(format.font_index, Some(0));
    assert_eq!(format.number_format_id, Some(0));
}

#[test]
fn an_identical_border_is_reused_rather_than_added_again() {
    let original = fixture_styles();
    let change = box_of(line(BorderStyle::Thin, Some("8EA9DB")));
    // Border 1 is exactly that box.
    let (xml, index) = add_cell_format(&original, None, &change).expect("add");
    let table = parse_styles_xml(&xml).expect("parse");
    assert_eq!(table.borders.len(), 4);
    assert_eq!(table.cell_formats[index as usize].border_index, Some(1));

    // And style 3 already is that look: nothing is added at all.
    let (same, three) = add_cell_format(&original, Some(3), &change).expect("add");
    let table = parse_styles_xml(&same).expect("parse");
    assert_eq!(table.borders.len(), 4);
    assert_eq!(table.cell_formats[three as usize].border_index, Some(1));

    let thick = box_of(line(BorderStyle::Thick, None));
    let (once, first) = add_cell_format(&original, None, &thick).expect("add");
    let (twice, second) = add_cell_format(&once, None, &thick).expect("add");
    assert_eq!(first, second);
    assert_eq!(once, twice);
}

#[test]
fn diagonals_share_a_line_and_have_their_own_directions() {
    let original = fixture_styles();
    let change = FormatChange {
        border_diagonal: Some(line(BorderStyle::Dashed, Some("008000"))),
        diagonal_up: Some(true),
        ..FormatChange::default()
    };
    let (xml, index) = add_cell_format(&original, None, &change).expect("add");
    let table = parse_styles_xml(&xml).expect("parse");
    let border = &table.borders[table.cell_formats[index as usize]
        .border_index
        .expect("border") as usize];
    assert_eq!(border.diagonal, Some(edge("dashed", 0x008000)));
    assert!(border.diagonal_up);
    assert!(!border.diagonal_down);

    // Style 10 wears border 3, both diagonals: turning one direction off
    // keeps the line and the other direction.
    let down_only = FormatChange {
        diagonal_up: Some(false),
        ..FormatChange::default()
    };
    let (xml, index) = add_cell_format(&original, Some(10), &down_only).expect("add");
    let table = parse_styles_xml(&xml).expect("parse");
    let border = &table.borders[table.cell_formats[index as usize]
        .border_index
        .expect("border") as usize];
    assert!(!border.diagonal_up);
    assert!(border.diagonal_down);
    assert_eq!(border.diagonal, Some(edge("medium", 0x008000)));
}

#[test]
fn an_automatic_colour_writes_no_colour() {
    let original = fixture_styles();
    let change = FormatChange {
        border_bottom: Some(line(BorderStyle::Hair, None)),
        ..FormatChange::default()
    };
    let (xml, index) = add_cell_format(&original, None, &change).expect("add");
    let table = parse_styles_xml(&xml).expect("parse");
    let border = &table.borders[table.cell_formats[index as usize]
        .border_index
        .expect("border") as usize];
    assert_eq!(
        border.bottom,
        Some(BorderEdge {
            style: "hair".into(),
            color: None
        })
    );
}

#[test]
fn a_bad_border_colour_is_refused() {
    let original = fixture_styles();
    let change = FormatChange {
        border_left: Some(line(BorderStyle::Thin, Some("blue"))),
        ..FormatChange::default()
    };
    assert!(add_cell_format(&original, None, &change).is_err());
}

#[test]
fn a_styles_part_without_borders_gets_a_border_list() {
    let original = fixture_styles();
    let start = original.find("<borders").expect("borders");
    let end = original.find("</borders>").expect("borders") + "</borders>".len();
    let without = format!("{}{}", &original[..start], &original[end..]);
    let change = box_of(line(BorderStyle::Thin, None));
    let (xml, index) = add_cell_format(&without, None, &change).expect("add");
    let table = parse_styles_xml(&xml).expect("parse");
    // The default "no border" first, as every xf without one points at it,
    // then the new one.
    assert_eq!(table.borders.len(), 2);
    assert_eq!(table.borders[0], Border::default());
    assert_eq!(table.cell_formats[index as usize].border_index, Some(1));
    assert!(xml.contains(r#"<borders count="2">"#), "{xml}");
    // In schema order: after the fills, before cellStyleXfs.
    let borders = xml.find("<borders").expect("borders");
    assert!(xml.find("</fills>").expect("fills") < borders);
    assert!(borders < xml.find("<cellStyleXfs").expect("cellStyleXfs"));
}

// ── property: any sequence of border changes reads back ─────────────

fn arb_line() -> impl Strategy<Value = BorderLine> {
    (
        prop::sample::select(&BorderStyle::ALL[..]),
        prop::option::of("[0-9A-F]{6}"),
    )
        .prop_map(|(style, color)| BorderLine { style, color })
}

fn arb_change() -> impl Strategy<Value = FormatChange> {
    use prop::option::of;
    (
        (of(arb_line()), of(arb_line()), of(arb_line())),
        (of(arb_line()), of(arb_line())),
        (of(any::<bool>()), of(any::<bool>()), of(any::<bool>())),
    )
        .prop_map(
            |(
                (border_left, border_right, border_top),
                (border_bottom, border_diagonal),
                (diagonal_up, diagonal_down, bold),
            )| FormatChange {
                border_left,
                border_right,
                border_top,
                border_bottom,
                border_diagonal,
                diagonal_up,
                diagonal_down,
                bold,
                ..FormatChange::default()
            },
        )
}

/// What the reader makes of `line`.
fn expected(line: &BorderLine) -> Option<BorderEdge> {
    (!line.is_none()).then(|| BorderEdge {
        style: line.style.as_str().to_string(),
        color: line.color.as_deref().map(|rgb| {
            let bytes = u32::from_str_radix(rgb, 16).expect("hex");
            Color::Argb([0xFF, (bytes >> 16) as u8, (bytes >> 8) as u8, bytes as u8])
        }),
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn every_border_change_reads_back_relative_to_its_base(
        steps in prop::collection::vec((0u32..11, arb_change()), 1..6),
    ) {
        let mut xml = fixture_styles();
        for (base, change) in steps {
            let before = parse_styles_xml(&xml).expect("parse before");
            let base_format = before.cell_formats[base as usize];
            let base_border = before.borders[base_format.border_index.unwrap_or(0) as usize].clone();
            let (next, index) = add_cell_format(&xml, Some(base), &change).expect("add");
            let after = parse_styles_xml(&next).expect("parse after");
            let format = &after.cell_formats[index as usize];
            let border = &after.borders[format.border_index.unwrap_or(0) as usize];

            let borders_count = format!("<borders count=\"{}\">", after.borders.len());
            prop_assert!(next.contains(&borders_count));
            for (asked, got, had) in [
                (&change.border_left, &border.left, &base_border.left),
                (&change.border_right, &border.right, &base_border.right),
                (&change.border_top, &border.top, &base_border.top),
                (&change.border_bottom, &border.bottom, &base_border.bottom),
                (&change.border_diagonal, &border.diagonal, &base_border.diagonal),
            ] {
                match asked {
                    Some(line) => prop_assert_eq!(got, &expected(line)),
                    None => prop_assert_eq!(got, had),
                }
            }
            prop_assert_eq!(border.diagonal_up, change.diagonal_up.unwrap_or(base_border.diagonal_up));
            prop_assert_eq!(
                border.diagonal_down,
                change.diagonal_down.unwrap_or(base_border.diagonal_down)
            );
            // The rest of the base look stays.
            prop_assert_eq!(format.fill_index, base_format.fill_index);
            prop_assert_eq!(format.number_format_id, base_format.number_format_id);
            if change.bold.is_none() {
                prop_assert_eq!(format.font_index, base_format.font_index);
            }

            // Adding the same again changes nothing.
            let (again, same) = add_cell_format(&next, Some(base), &change).expect("again");
            prop_assert_eq!(same, index);
            prop_assert_eq!(again, next.clone());

            // Entries already there stay where they were.
            prop_assert_eq!(&after.borders[..before.borders.len()], &before.borders[..]);
            prop_assert_eq!(&after.cell_formats[..before.cell_formats.len()], &before.cell_formats[..]);

            xml = next;
        }
    }
}
