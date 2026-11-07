//! スライド ショーの設定, 目的別スライド ショー, rehearsed timings and the
//! grid and guides: each written, read back, and undone exactly.

use jade_opc::OpcPackage;
use jade_pptx_reader::{
    read_show, read_view_props, CustomShow, Guide, LastView, ShowRange, ShowSettings, ShowType,
    ViewProps,
};
use jade_pptx_writer::EditSession;

const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");

#[test]
fn show_settings_are_written_into_a_new_pres_props_part_and_read_back() {
    let mut session = EditSession::open(DECK).unwrap();
    let settings = ShowSettings {
        show_type: ShowType::Kiosk {
            restart_ms: 300_000,
        },
        loop_until_esc: true,
        without_narration: true,
        without_animation: true,
        use_timings: true,
        range: ShowRange::Slides { from: 2, to: 3 },
        pen_color: Some("FF0000".into()),
        laser_color: Some("00B050".into()),
    };
    assert!(session.set_show_settings(&settings).unwrap());
    let bytes = session.bytes().to_vec();
    assert_eq!(read_show(&bytes).unwrap().settings, settings);
    let package = OpcPackage::open(&bytes).unwrap();
    assert!(package
        .required_part_str("[Content_Types].xml")
        .unwrap()
        .contains("/ppt/presProps.xml"));
    assert!(package
        .required_part_str("ppt/_rels/presentation.xml.rels")
        .unwrap()
        .contains("presProps"));

    // Changing one setting keeps the rest, the laser colour's extension
    // included.
    let browsed = ShowSettings {
        show_type: ShowType::Individual { scrollbar: false },
        loop_until_esc: false,
        ..settings.clone()
    };
    assert!(session.set_show_settings(&browsed).unwrap());
    assert_eq!(read_show(session.bytes()).unwrap().settings, browsed);
    assert!(!session.set_show_settings(&browsed).unwrap());

    session.undo();
    session.undo();
    assert_eq!(session.bytes(), DECK);
    assert_eq!(read_show(DECK).unwrap().settings, ShowSettings::default());
}

#[test]
fn custom_shows_name_slides_by_relationship_and_survive_slide_deletion() {
    let mut session = EditSession::open(DECK).unwrap();
    let shows = vec![
        CustomShow {
            id: 0,
            name: "Short & sweet".into(),
            slides: vec![2, 0],
        },
        CustomShow {
            id: 1,
            name: "Middle".into(),
            slides: vec![1],
        },
    ];
    assert!(session.set_custom_shows(&shows).unwrap());
    assert_eq!(read_show(session.bytes()).unwrap().custom_shows, shows);
    assert!(session
        .set_show_settings(&ShowSettings {
            range: ShowRange::Custom(0),
            ..ShowSettings::default()
        })
        .unwrap());
    assert_eq!(
        read_show(session.bytes()).unwrap().settings.range,
        ShowRange::Custom(0)
    );

    // Deleting a slide takes it out of the shows that played it.
    assert!(session.delete_slide(1).unwrap());
    let read = read_show(session.bytes()).unwrap().custom_shows;
    assert_eq!(read[0].slides, vec![1, 0]);
    assert!(read[1].slides.is_empty());

    assert!(session.set_custom_shows(&[]).unwrap());
    assert!(!OpcPackage::open(session.bytes())
        .unwrap()
        .required_part_str("ppt/presentation.xml")
        .unwrap()
        .contains("custShowLst"));
}

#[test]
fn rehearsed_timings_set_and_clear_each_slides_advance_in_one_step() {
    let mut session = EditSession::open(DECK).unwrap();
    assert!(session
        .set_slide_advances(&[(0, Some(4200)), (2, Some(1500))])
        .unwrap());
    let advances = read_show(session.bytes()).unwrap().advances;
    assert_eq!(advances[0].after_ms, Some(4200));
    assert_eq!(advances[1].after_ms, None);
    assert_eq!(advances[2].after_ms, Some(1500));
    assert!(advances.iter().all(|advance| advance.on_click));
    jade_pptx_reader::read_deck(session.bytes()).unwrap();

    assert!(session.set_slide_advances(&[(0, None)]).unwrap());
    assert_eq!(
        read_show(session.bytes()).unwrap().advances[0].after_ms,
        None
    );
    session.undo();
    session.undo();
    assert_eq!(session.bytes(), DECK);
}

#[test]
fn grid_guides_and_last_view_round_trip_through_view_props() {
    let mut session = EditSession::open(DECK).unwrap();
    let props = ViewProps {
        last_view: LastView::SlideSorter,
        grid_spacing_emu: (180_000, 180_000),
        snap_to_grid: false,
        snap_to_objects: true,
        show_guides: true,
        guides: vec![
            Guide {
                horizontal: true,
                position_emu: 3_429_000,
            },
            Guide {
                horizontal: false,
                position_emu: 6_096_000,
            },
        ],
    };
    assert!(session.set_view_props(&props).unwrap());
    assert_eq!(read_view_props(session.bytes()).unwrap(), props);
    let fewer = ViewProps {
        guides: vec![props.guides[1]],
        last_view: LastView::Normal,
        ..props.clone()
    };
    assert!(session.set_view_props(&fewer).unwrap());
    assert_eq!(read_view_props(session.bytes()).unwrap(), fewer);
    session.undo();
    session.undo();
    assert_eq!(session.bytes(), DECK);
}

#[test]
fn print_layout_is_kept_in_print_properties() {
    let mut session = EditSession::open(DECK).unwrap();
    assert_eq!(jade_pptx_writer::print_what(DECK).unwrap(), "slides");
    assert!(session.set_print_what("handouts6").unwrap());
    assert_eq!(
        jade_pptx_writer::print_what(session.bytes()).unwrap(),
        "handouts6"
    );
    assert!(session.set_print_what("handouts3").unwrap());
    assert_eq!(
        jade_pptx_writer::print_what(session.bytes()).unwrap(),
        "handouts3"
    );
    assert!(session.set_print_what("bogus").is_err());
}

#[test]
fn a_notes_master_is_made_for_notes_pages_and_linked_to_every_notes_slide() {
    let mut session = EditSession::open(DECK).unwrap();
    assert!(session.set_slide_notes(1, "Speak slowly").unwrap());
    assert!(session.ensure_notes_master().unwrap());
    assert!(!session.ensure_notes_master().unwrap());
    let bytes = session.bytes().to_vec();
    let pages = jade_pptx_reader::read_notes_pages(&bytes).unwrap();
    let master = pages.notes_master.as_ref().expect("a notes master");
    assert!(
        jade_pptx_reader::notes_pages::placeholder_transform(&master.content, "sldImg").is_some()
    );
    assert!(master.content.text_styles.is_some(), "notesStyle is read");
    // The slide image keeps the 16:9 slide's proportions.
    let image = pages.slide_image_rect(1);
    let aspect = image.extent_emu.0 as f64 / image.extent_emu.1 as f64;
    assert!(
        (aspect - 12_191_695.0 / 6_858_000.0).abs() < 0.01,
        "{aspect}"
    );
    let package = OpcPackage::open(&bytes).unwrap();
    let presentation = package.required_part_str("ppt/presentation.xml").unwrap();
    assert!(presentation.contains("notesMasterIdLst"));
    // Before sldIdLst, as the schema orders them.
    assert!(presentation.find("notesMasterIdLst") < presentation.find("sldIdLst"));
    let notes = pages.notes[1].as_ref().expect("slide 2 has notes");
    let rels = package
        .required_part_str(&jade_opc::rels_part_name(&notes.part_name))
        .unwrap();
    assert!(rels.contains("notesMaster"));
    jade_pptx_reader::read_deck(&bytes).unwrap();
}

#[test]
fn notes_and_handout_header_footer_apply_to_both_masters() {
    let mut session = EditSession::open(DECK).unwrap();
    let settings = jade_pptx_writer::PageHeaderFooter {
        date: true,
        date_text: Some("2026年11月2日".into()),
        header: Some("社外秘 <draft>".into()),
        page_number: true,
        footer: Some("jade".into()),
    };
    assert!(session.set_page_header_footer(&settings).unwrap());
    let bytes = session.bytes().to_vec();
    assert_eq!(
        jade_pptx_writer::page_header_footer(&bytes).unwrap(),
        settings
    );
    let pages = jade_pptx_reader::read_notes_pages(&bytes).unwrap();
    let handout = pages.handout_master.expect("a handout master");
    assert!(handout.header_footer.header && handout.header_footer.footer);

    let fewer = jade_pptx_writer::PageHeaderFooter {
        date: false,
        date_text: None,
        header: None,
        page_number: false,
        footer: Some("jade".into()),
    };
    assert!(session.set_page_header_footer(&fewer).unwrap());
    let read = jade_pptx_writer::page_header_footer(session.bytes()).unwrap();
    assert_eq!(read, fewer);
    let flags = jade_pptx_reader::read_notes_pages(session.bytes())
        .unwrap()
        .handout_master
        .unwrap()
        .header_footer;
    assert!(!flags.header && flags.footer && !flags.date && !flags.slide_number);
}

#[test]
fn handout_orientation_turns_the_page_and_its_masters() {
    let mut session = EditSession::open(DECK).unwrap();
    session.ensure_handout_master().unwrap();
    assert!(!session.set_notes_orientation(true).unwrap());
    assert!(session.set_notes_orientation(false).unwrap());
    let pages = jade_pptx_reader::read_notes_pages(session.bytes()).unwrap();
    assert_eq!(pages.page_size.width_emu, 9_144_000);
    assert_eq!(pages.page_size.height_emu, 6_858_000);
    let master = pages.handout_master.unwrap();
    let footer =
        jade_pptx_reader::notes_pages::placeholder_transform(&master.content, "ftr").unwrap();
    assert!(footer.offset_emu.1 + footer.extent_emu.1 <= 6_858_000 + 1);
    assert!(session.set_notes_orientation(true).unwrap());
    assert_eq!(
        jade_pptx_reader::read_notes_pages(session.bytes())
            .unwrap()
            .page_size
            .height_emu,
        9_144_000
    );
}

#[test]
fn notes_text_is_formatted_in_place() {
    let mut session = EditSession::open(DECK).unwrap();
    session.set_slide_notes(0, "Bold this word").unwrap();
    let format = jade_pptx_writer::TextFormat {
        bold: Some(true),
        color: Some("#C00000".into()),
        ..Default::default()
    };
    assert!(session.format_notes_range(0, 5, 9, &format).unwrap());
    let pages = jade_pptx_reader::read_notes_pages(session.bytes()).unwrap();
    let notes = pages.notes[0].as_ref().unwrap();
    let body = notes
        .content
        .shapes
        .iter()
        .find_map(|item| match item {
            jade_pptx_reader::ShapeItem::Shape(shape)
                if shape
                    .placeholder
                    .as_ref()
                    .is_some_and(|ph| ph.kind == "body") =>
            {
                shape.text.as_ref()
            }
            _ => None,
        })
        .unwrap();
    let runs = &body.paragraphs[0].runs;
    let bold: Vec<_> = runs
        .iter()
        .filter(|run| run.bold)
        .map(|run| run.text.as_str())
        .collect();
    assert_eq!(bold, vec!["this"]);
    // The text itself is unchanged.
    assert_eq!(
        jade_pptx_writer::notes_text(session.bytes(), 0)
            .unwrap()
            .text,
        "Bold this word"
    );
}

#[test]
fn kept_ink_lands_on_the_slide_as_freeform_lines() {
    let paths = vec![
        jade_pptx_writer::InkPath {
            points_emu: vec![(100_000, 100_000), (400_000, 250_000), (700_000, 120_000)],
            color: "FF0000".into(),
            width_emu: 28_575,
            highlighter: false,
        },
        jade_pptx_writer::InkPath {
            points_emu: vec![(100_000, 900_000), (900_000, 900_000)],
            color: "FFFF00".into(),
            width_emu: 171_450,
            highlighter: true,
        },
    ];
    let mut session = EditSession::open(DECK).unwrap();
    let before = jade_pptx_reader::read_deck(DECK).unwrap().slides[0]
        .content
        .shapes
        .len();
    assert!(session
        .apply_batch(|bytes| jade_pptx_writer::insert_ink(bytes, 0, &paths))
        .unwrap());
    let deck = jade_pptx_reader::read_deck(session.bytes()).unwrap();
    let shapes = &deck.slides[0].content.shapes;
    assert_eq!(shapes.len(), before + 2);
    let ink: Vec<_> = shapes
        .iter()
        .filter_map(|item| match item {
            jade_pptx_reader::ShapeItem::Shape(shape) if shape.name.starts_with("Ink") => {
                Some(shape)
            }
            _ => None,
        })
        .collect();
    assert_eq!(ink.len(), 2);
    assert!(ink[0].custom_geometry.is_some());
    let transform = ink[0].transform.unwrap();
    assert_eq!(transform.offset_emu, (100_000, 100_000));
    assert_eq!(transform.extent_emu, (600_000, 150_000));
    // Undo takes the ink away again.
    session.undo();
    assert_eq!(session.bytes(), DECK);
}
