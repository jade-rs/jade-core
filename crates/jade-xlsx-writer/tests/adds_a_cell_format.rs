//! Making a cell look different means adding to `styles.xml`, never editing
//! what other cells share.

use jade_opc::OpcPackage;
use jade_xlsx_reader::styles::parse_styles as parse_styles_xml;
use jade_xlsx_reader::{Color, HorizontalAlignment, VerticalAlignment};
use jade_xlsx_writer::{add_cell_format, FormatChange};
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

fn bold() -> FormatChange {
    FormatChange {
        bold: Some(true),
        ..FormatChange::default()
    }
}

#[test]
fn bold_adds_a_font_and_an_xf_and_touches_nothing_else() {
    let original = fixture_styles();
    let (xml, index) = add_cell_format(&original, None, &bold()).expect("add");

    // The fixture has 11 xfs and 4 fonts: the new ones are 11 and 4.
    assert_eq!(index, 11);
    let table = parse_styles_xml(&xml).expect("parse");
    assert_eq!(table.cell_formats.len(), 12);
    assert_eq!(table.fonts.len(), 5);
    let format = &table.cell_formats[11];
    assert_eq!(format.font_index, Some(4));
    let font = table.font(format).expect("font");
    assert!(font.bold);
    // The base font's size, colour and name came along.
    assert_eq!(font.size_points, Some(11.0));
    assert_eq!(font.name.as_deref(), Some("Calibri"));
    assert!(matches!(font.color, Some(Color::Theme { index: 1, .. })));

    // Counts follow, and the text outside the two lists is as it was.
    assert!(xml.contains(r#"<fonts count="5">"#), "{xml}");
    assert!(xml.contains(r#"<cellXfs count="12">"#), "{xml}");
    assert!(
        xml.contains(r#"<font><b/><sz val="11"/><color theme="1"/><name val="Calibri"/></font>"#)
    );
    assert!(xml.contains(
        r#"<xf numFmtId="0" fontId="4" fillId="0" borderId="0" xfId="0" applyFont="1"/>"#
    ));
    let before_fonts = &original[..original.find("<fonts").expect("fonts")];
    assert!(xml.starts_with(before_fonts));
    let after_xfs = &original[original.find("</cellXfs>").expect("xfs")..];
    assert!(xml.ends_with(after_xfs));
}

#[test]
fn an_identical_format_is_reused_rather_than_added_again() {
    let original = fixture_styles();
    let (once, index) = add_cell_format(&original, None, &bold()).expect("add");
    let (twice, again) = add_cell_format(&once, None, &bold()).expect("add");
    assert_eq!(again, index);
    assert_eq!(twice, once);

    // Bold on a cell whose font is already bold (style 1) reuses that font
    // — only the xf differs, and one exists already: style 1's twin with
    // applyFont, which the fixture does not have, so one xf is added.
    let (xml, index) = add_cell_format(&original, Some(1), &bold()).expect("add");
    let table = parse_styles_xml(&xml).expect("parse");
    assert_eq!(table.fonts.len(), 4);
    assert_eq!(table.cell_formats[index as usize].font_index, Some(1));
}

#[test]
fn unbold_keeps_the_rest_of_the_font() {
    let original = fixture_styles();
    // Style 1 is bold 16pt Meiryo in navy.
    let change = FormatChange {
        bold: Some(false),
        ..FormatChange::default()
    };
    let (xml, index) = add_cell_format(&original, Some(1), &change).expect("add");
    let table = parse_styles_xml(&xml).expect("parse");
    let font = table
        .font(&table.cell_formats[index as usize])
        .expect("font");
    assert!(!font.bold);
    assert_eq!(font.size_points, Some(16.0));
    assert_eq!(font.name.as_deref(), Some("Meiryo"));
    assert_eq!(font.color, Some(Color::Argb([0xFF, 0x1F, 0x38, 0x64])));
    // And the base xf's alignment survives.
    assert_eq!(
        table.cell_formats[index as usize].alignment.horizontal,
        HorizontalAlignment::Center
    );
}

#[test]
fn number_formats_are_built_in_existing_or_new() {
    let original = fixture_styles();
    let code = |code: &str| FormatChange {
        number_format: Some(code.to_string()),
        ..FormatChange::default()
    };

    // `0.00` is built-in id 2: nothing is added to numFmts.
    let (xml, index) = add_cell_format(&original, None, &code("0.00")).expect("add");
    let table = parse_styles_xml(&xml).expect("parse");
    assert_eq!(table.cell_formats[index as usize].number_format_id, Some(2));
    assert!(xml.contains(r#"<numFmts count="3">"#));

    // `yyyy/mm/dd` is already 165.
    let (xml, index) = add_cell_format(&original, None, &code("yyyy/mm/dd")).expect("add");
    let table = parse_styles_xml(&xml).expect("parse");
    assert_eq!(
        table.cell_formats[index as usize].number_format_id,
        Some(165)
    );
    assert!(xml.contains(r#"<numFmts count="3">"#));

    // A new code takes the next id after the highest custom one, escaped.
    let (xml, index) = add_cell_format(&original, Some(4), &code("\"€\"#,##0.0")).expect("add");
    let table = parse_styles_xml(&xml).expect("parse");
    assert_eq!(
        table.cell_formats[index as usize].number_format_id,
        Some(167)
    );
    assert_eq!(
        table.number_formats.get(&167).map(String::as_str),
        Some("\"€\"#,##0.0")
    );
    assert!(xml.contains(r#"<numFmts count="4">"#), "{xml}");
    assert!(
        xml.contains(r#"<numFmt numFmtId="167" formatCode="&quot;€&quot;#,##0.0"/>"#),
        "{xml}"
    );
    // Base 4's border came along.
    assert_eq!(table.cell_formats[index as usize].border_index, Some(1));

    // General clears it.
    let (xml, index) = add_cell_format(&original, Some(4), &code("General")).expect("add");
    let table = parse_styles_xml(&xml).expect("parse");
    assert_eq!(table.cell_formats[index as usize].number_format_id, Some(0));
}

#[test]
fn a_styles_part_without_num_fmts_gets_one() {
    let original = fixture_styles();
    let start = original.find("<numFmts").expect("numFmts");
    let end = original.find("</numFmts>").expect("end") + "</numFmts>".len();
    let stripped = format!("{}{}", &original[..start], &original[end..]);
    let change = FormatChange {
        number_format: Some("0.0%".to_string()),
        ..FormatChange::default()
    };
    let (xml, index) = add_cell_format(&stripped, None, &change).expect("add");
    let table = parse_styles_xml(&xml).expect("parse");
    assert_eq!(
        table.cell_formats[index as usize].number_format_id,
        Some(164)
    );
    assert!(
        xml.contains(
            r#"<numFmts count="1"><numFmt numFmtId="164" formatCode="0.0%"/></numFmts><fonts"#
        ),
        "{xml}"
    );
}

#[test]
fn a_fill_colour_adds_a_solid_fill_and_the_empty_string_removes_it() {
    let original = fixture_styles();
    let change = FormatChange {
        fill_color: Some("#ffc000".to_string()),
        ..FormatChange::default()
    };
    let (xml, index) = add_cell_format(&original, None, &change).expect("add");
    let table = parse_styles_xml(&xml).expect("parse");
    let format = &table.cell_formats[index as usize];
    assert_eq!(format.fill_index, Some(5));
    let fill = table.fill(format).expect("fill");
    assert_eq!(fill.pattern.as_deref(), Some("solid"));
    assert_eq!(fill.foreground, Some(Color::Argb([0xFF, 0xFF, 0xC0, 0x00])));
    assert!(xml.contains(r#"<fills count="6">"#));

    // Style 5 has fill 3; clearing it points at fill 0 (none).
    let clear = FormatChange {
        fill_color: Some(String::new()),
        ..FormatChange::default()
    };
    let (xml, index) = add_cell_format(&original, Some(5), &clear).expect("add");
    let table = parse_styles_xml(&xml).expect("parse");
    assert_eq!(table.cell_formats[index as usize].fill_index, Some(0));
    assert_eq!(table.fills.len(), 5);

    let bad = FormatChange {
        fill_color: Some("blue".to_string()),
        ..FormatChange::default()
    };
    assert!(add_cell_format(&original, None, &bad).is_err());
}

#[test]
fn alignment_merges_into_what_the_base_had() {
    let original = fixture_styles();
    // Style 7 is left / top / wrap. Centre it and drop the wrap.
    let change = FormatChange {
        horizontal: Some(HorizontalAlignment::Center),
        wrap_text: Some(false),
        ..FormatChange::default()
    };
    let (xml, index) = add_cell_format(&original, Some(7), &change).expect("add");
    let table = parse_styles_xml(&xml).expect("parse");
    let alignment = &table.cell_formats[index as usize].alignment;
    assert_eq!(alignment.horizontal, HorizontalAlignment::Center);
    assert_eq!(alignment.vertical, VerticalAlignment::Top);
    assert!(!alignment.wrap_text);
    assert!(
        xml.contains(r#"<alignment horizontal="center" vertical="top"/>"#),
        "{xml}"
    );

    // Wrap on the default style: an alignment child appears.
    let wrap = FormatChange {
        wrap_text: Some(true),
        ..FormatChange::default()
    };
    let (xml, index) = add_cell_format(&original, None, &wrap).expect("add");
    let table = parse_styles_xml(&xml).expect("parse");
    assert!(table.cell_formats[index as usize].alignment.wrap_text);
    assert!(
        xml.contains(r#"applyAlignment="1"><alignment wrapText="1"/></xf>"#),
        "{xml}"
    );
}

#[test]
fn font_colour_size_and_name_replace_the_base_ones() {
    let original = fixture_styles();
    let change = FormatChange {
        font_color: Some("C00000".to_string()),
        font_size: Some(14.0),
        font_name: Some("Yu Gothic".to_string()),
        italic: Some(true),
        ..FormatChange::default()
    };
    let (xml, index) = add_cell_format(&original, Some(3), &change).expect("add");
    let table = parse_styles_xml(&xml).expect("parse");
    let font = table
        .font(&table.cell_formats[index as usize])
        .expect("font");
    assert!(font.italic);
    assert_eq!(font.size_points, Some(14.0));
    assert_eq!(font.name.as_deref(), Some("Yu Gothic"));
    assert_eq!(font.color, Some(Color::Argb([0xFF, 0xC0, 0x00, 0x00])));
    assert!(
        xml.contains(
            r#"<font><i/><sz val="14"/><color rgb="FFC00000"/><name val="Yu Gothic"/></font>"#
        ),
        "{xml}"
    );
}

#[test]
fn a_missing_base_is_refused() {
    let original = fixture_styles();
    assert!(add_cell_format(&original, Some(99), &bold()).is_err());
}

// ── property: any sequence of changes reads back ────────────────────

fn change() -> impl Strategy<Value = FormatChange> {
    (
        prop::option::of(any::<bool>()),
        prop::option::of(any::<bool>()),
        prop::option::of(any::<bool>()),
        prop::option::of("[0-9A-F]{6}"),
        prop::option::of(prop_oneof![
            Just(String::new()),
            "[0-9A-F]{6}".prop_map(String::from)
        ]),
        prop::option::of(prop_oneof![
            Just("0.00".to_string()),
            Just("#,##0".to_string()),
            Just("yyyy/mm/dd".to_string()),
            Just("0.0%".to_string()),
            Just(String::new()),
        ]),
        prop::option::of(prop_oneof![
            Just(HorizontalAlignment::General),
            Just(HorizontalAlignment::Left),
            Just(HorizontalAlignment::Center),
            Just(HorizontalAlignment::Right),
        ]),
        prop::option::of(any::<bool>()),
    )
        .prop_map(
            |(
                bold,
                italic,
                underline,
                font_color,
                fill_color,
                number_format,
                horizontal,
                wrap_text,
            )| {
                FormatChange {
                    bold,
                    italic,
                    underline,
                    font_color,
                    fill_color,
                    number_format,
                    horizontal,
                    wrap_text,
                    ..FormatChange::default()
                }
            },
        )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn every_change_reads_back_relative_to_its_base(
        steps in prop::collection::vec((0u32..11, change()), 1..6),
    ) {
        let mut xml = fixture_styles();
        for (base, change) in steps {
            let before = parse_styles_xml(&xml).expect("parse before");
            let base_format = before.cell_formats[base as usize];
            let (next, index) = add_cell_format(&xml, Some(base), &change).expect("add");
            let after = parse_styles_xml(&next).expect("parse after");
            let format = &after.cell_formats[index as usize];

            // Counts agree with what is there.
            let fonts_count = format!("<fonts count=\"{}\">", after.fonts.len());
            let fills_count = format!("<fills count=\"{}\">", after.fills.len());
            let xfs_count = format!("<cellXfs count=\"{}\">", after.cell_formats.len());
            prop_assert!(next.contains(&fonts_count));
            prop_assert!(next.contains(&fills_count));
            prop_assert!(next.contains(&xfs_count));

            // Font flags: as asked, else as the base had them.
            let font = after.font(format).expect("font");
            let base_font = before.font(&base_format).expect("base font");
            prop_assert_eq!(font.bold, change.bold.unwrap_or(base_font.bold));
            prop_assert_eq!(font.italic, change.italic.unwrap_or(base_font.italic));
            prop_assert_eq!(font.underline, change.underline.unwrap_or(base_font.underline));
            prop_assert_eq!(font.name.clone(), base_font.name.clone());
            match &change.font_color {
                Some(rgb) => {
                    let bytes = u32::from_str_radix(rgb, 16).expect("hex");
                    prop_assert_eq!(
                        font.color,
                        Some(Color::Argb([0xFF, (bytes >> 16) as u8, (bytes >> 8) as u8, bytes as u8]))
                    );
                }
                None => prop_assert_eq!(font.color, base_font.color),
            }

            // Fill.
            match &change.fill_color {
                Some(rgb) if rgb.is_empty() => prop_assert_eq!(format.fill_index, Some(0)),
                Some(rgb) => {
                    let fill = after.fill(format).expect("fill");
                    let bytes = u32::from_str_radix(rgb, 16).expect("hex");
                    prop_assert_eq!(fill.pattern.as_deref(), Some("solid"));
                    prop_assert_eq!(
                        fill.foreground,
                        Some(Color::Argb([0xFF, (bytes >> 16) as u8, (bytes >> 8) as u8, bytes as u8]))
                    );
                }
                None => prop_assert_eq!(format.fill_index, base_format.fill_index),
            }

            // Number format.
            match &change.number_format {
                Some(code) if code.is_empty() => prop_assert_eq!(format.number_format_id, Some(0)),
                Some(code) => prop_assert_eq!(after.number_format_code(format), Some(code.as_str())),
                None => prop_assert_eq!(format.number_format_id, base_format.number_format_id),
            }

            // Alignment, and the border the base had.
            prop_assert_eq!(
                format.alignment.horizontal,
                change.horizontal.unwrap_or(base_format.alignment.horizontal)
            );
            prop_assert_eq!(
                format.alignment.wrap_text,
                change.wrap_text.unwrap_or(base_format.alignment.wrap_text)
            );
            prop_assert_eq!(format.border_index, base_format.border_index);

            // Adding the same again changes nothing.
            let (again, same) = add_cell_format(&next, Some(base), &change).expect("again");
            prop_assert_eq!(same, index);
            prop_assert_eq!(again, next.clone());

            // Everything the base pointed at is still where it was.
            prop_assert_eq!(&after.cell_formats[..before.cell_formats.len()], &before.cell_formats[..]);
            prop_assert_eq!(&after.fonts[..before.fonts.len()], &before.fonts[..]);
            prop_assert_eq!(&after.fills[..before.fills.len()], &before.fills[..]);

            xml = next;
        }
    }
}
