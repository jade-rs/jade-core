//! Ink, video and audio: drawn ink is written as PowerPoint writes it
//! (`p14` content part with an InkML part and a fallback picture) and read
//! back; media is inserted embedded or linked with its timing, and the
//! 再生 tab's options, trims, fades, bookmarks and posters round-trip.

use jade_opc::OpcPackage;
use jade_pptx_reader::{
    media_duration_ms, parse_media_playback, read_deck, InkBrush, InkTool, InkTrace, MediaKind,
    MediaPlayback, MediaStart, ShapeItem, Transform,
};
use jade_pptx_writer::media::{InkEdit, MediaCuts, MediaInsert, MediaSource};
use jade_pptx_writer::EditSession;

const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");
const VIDEO: &[u8] = include_bytes!("../../../public/fixtures/poi/EmbeddedVideo.pptx");
const AUDIO: &[u8] = include_bytes!("../../../public/fixtures/poi/EmbeddedAudio.pptx");

fn slide_xml(bytes: &[u8], slide: usize) -> String {
    let deck = read_deck(bytes).unwrap();
    let part = deck.slides[slide].part_name.clone();
    OpcPackage::open(bytes)
        .unwrap()
        .required_part_str(&part)
        .unwrap()
}

fn stroke(color: &str, points: &[(f64, f64)]) -> InkTrace {
    InkTrace {
        points: points.to_vec(),
        brush: InkBrush {
            tool: InkTool::Pen,
            color: color.into(),
            width_px: 3.0,
            height_px: 3.0,
            ..InkBrush::default()
        },
    }
}

fn inks(bytes: &[u8], slide: usize) -> Vec<jade_pptx_reader::Ink> {
    read_deck(bytes).unwrap().slides[slide]
        .content
        .shapes
        .iter()
        .filter_map(|item| match item {
            ShapeItem::Ink(ink) => Some(ink.clone()),
            _ => None,
        })
        .collect()
}

/// A second of silent 8 kHz mono WAV.
fn wav() -> Vec<u8> {
    let data = vec![0u8; 8000];
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&8000u32.to_le_bytes());
    out.extend_from_slice(&8000u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&8u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

fn png() -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, 2, 2);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&[0x80; 16])
        .unwrap();
    out
}

fn bounds() -> Transform {
    Transform {
        offset_emu: (952_500, 952_500),
        extent_emu: (3_810_000, 2_143_125),
        rotation_60000ths: 0,
        flip_h: false,
        flip_v: false,
    }
}

#[test]
fn drawn_ink_is_written_as_a_content_part_and_read_back() {
    let mut session = EditSession::open(DECK).unwrap();
    let created = session
        .edit_ink(
            0,
            &[InkEdit::New(vec![
                stroke("#FF0000", &[(100.0, 100.0), (200.0, 150.0), (300.0, 120.0)]),
                stroke("#0000FF", &[(120.0, 200.0), (220.0, 260.0)]),
            ])],
        )
        .unwrap();
    assert_eq!(created.len(), 1);
    let bytes = session.bytes().to_vec();
    let xml = slide_xml(&bytes, 0);
    assert!(xml.contains("mc:AlternateContent"), "{xml}");
    assert!(xml.contains("Requires=\"p14\""));
    assert!(
        xml.contains("<p:contentPart p14:bwMode=\"auto\"") && xml.contains("p14:nvContentPartPr")
    );
    assert!(xml.contains("mc:Fallback"));

    let read = inks(&bytes, 0);
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].id, Some(created[0]));
    assert_eq!(read[0].traces.len(), 2);
    assert_eq!(read[0].traces[0].brush.color.to_uppercase(), "#FF0000");
    let package = OpcPackage::open(&bytes).unwrap();
    let ink_part = read[0].ink_part.clone().unwrap();
    assert!(package.part(&ink_part).is_some());
    let types = package.required_part_str("[Content_Types].xml").unwrap();
    assert!(types.contains("application/inkml+xml"));

    // Erasing a stroke rewrites the part; erasing all of them deletes it.
    session
        .edit_ink(
            0,
            &[InkEdit::Replace {
                id: created[0],
                traces: vec![stroke("#0000FF", &[(120.0, 200.0), (220.0, 260.0)])],
            }],
        )
        .unwrap();
    assert_eq!(inks(session.bytes(), 0)[0].traces.len(), 1);
    session.edit_ink(0, &[InkEdit::Delete(created[0])]).unwrap();
    assert!(inks(session.bytes(), 0).is_empty());
    assert!(!slide_xml(session.bytes(), 0).contains("contentPart"));

    // Every step undoes.
    assert!(session.undo());
    assert!(session.undo());
    assert_eq!(inks(session.bytes(), 0)[0].traces.len(), 2);
}

#[test]
fn ink_survives_an_unrelated_edit_and_the_round_trip() {
    let mut session = EditSession::open(DECK).unwrap();
    session
        .edit_ink(
            0,
            &[InkEdit::New(vec![stroke(
                "#00AA00",
                &[(10.0, 10.0), (60.0, 60.0)],
            )])],
        )
        .unwrap();
    let before = inks(session.bytes(), 0);
    let id = before[0].id.unwrap();
    let shapes = jade_pptx_writer::editable_shapes(session.bytes(), 0).unwrap();
    let other = shapes.iter().find(|s| s.id != id).unwrap().id;
    let mut moved = shapes
        .iter()
        .find(|s| s.id == other)
        .unwrap()
        .transform
        .unwrap();
    moved.offset_emu.0 += 9525;
    session.set_shape_transform(0, other, moved).unwrap();
    assert_eq!(inks(session.bytes(), 0), before);
}

#[test]
fn embedded_audio_is_inserted_with_powerpoints_parts_and_timing() {
    let mut session = EditSession::open(DECK).unwrap();
    let audio = wav();
    assert_eq!(media_duration_ms(&audio), Some(1000.0));
    let id = session
        .insert_media(
            0,
            &MediaInsert {
                kind: MediaKind::Audio,
                source: MediaSource::Embedded {
                    bytes: &audio,
                    extension: "wav",
                },
                poster: &png(),
                transform: bounds(),
                duration_ms: Some(1000.0),
            },
        )
        .unwrap();
    let bytes = session.bytes().to_vec();
    let deck = read_deck(&bytes).unwrap();
    let picture = deck.slides[0]
        .content
        .shapes
        .iter()
        .find_map(|item| match item {
            ShapeItem::Picture(p) if p.id == Some(id) => Some(p.clone()),
            _ => None,
        })
        .unwrap();
    let media = picture.media.unwrap();
    assert_eq!(media.kind, MediaKind::Audio);
    assert!(!media.is_linked());
    let part = media.media_part.unwrap();
    let package = OpcPackage::open(&bytes).unwrap();
    assert_eq!(package.part(&part).unwrap(), audio.as_slice());

    let xml = slide_xml(&bytes, 0);
    assert!(xml.contains("a:audioFile"));
    assert!(xml.contains("p14:media"));
    assert!(xml.contains("ppaction://media"));
    assert!(xml.contains("presetClass=\"mediacall\""));
    let playback = parse_media_playback(&xml);
    assert_eq!(playback.len(), 1);
    assert_eq!(playback[0].shape_id, id);
    assert_eq!(playback[0].start, MediaStart::InClickSequence);
    assert_eq!(playback[0].volume, 80_000);

    let rels = package
        .part_names()
        .iter()
        .find(|n| n.ends_with("slide1.xml.rels"))
        .cloned()
        .unwrap();
    let rels = package.required_part_str(&rels).unwrap();
    assert!(rels.contains("relationships/media"));
    assert!(rels.contains("relationships/audio"));
}

#[test]
fn linked_video_points_at_its_file() {
    let mut session = EditSession::open(DECK).unwrap();
    let id = session
        .insert_media(
            0,
            &MediaInsert {
                kind: MediaKind::Video,
                source: MediaSource::Linked {
                    target: "file:///Users/me/My%20Clip.mp4",
                },
                poster: &png(),
                transform: bounds(),
                duration_ms: None,
            },
        )
        .unwrap();
    let bytes = session.bytes().to_vec();
    let deck = read_deck(&bytes).unwrap();
    let media = deck.slides[0]
        .content
        .shapes
        .iter()
        .find_map(|item| match item {
            ShapeItem::Picture(p) if p.id == Some(id) => p.media.clone(),
            _ => None,
        })
        .unwrap();
    assert!(media.is_linked());
    assert_eq!(
        media.linked_target.as_deref(),
        Some("file:///Users/me/My%20Clip.mp4")
    );
    let package = OpcPackage::open(&bytes).unwrap();
    let rels = package
        .required_part_str("ppt/slides/_rels/slide1.xml.rels")
        .unwrap();
    assert!(rels.contains("TargetMode=\"External\""));
    assert!(slide_xml(&bytes, 0).contains("a:videoFile"));
}

#[test]
fn the_playback_tab_round_trips_each_start_mode_and_option() {
    let mut session = EditSession::open(DECK).unwrap();
    let id = session
        .insert_media(
            0,
            &MediaInsert {
                kind: MediaKind::Video,
                source: MediaSource::Embedded {
                    bytes: &wav(),
                    extension: "wav",
                },
                poster: &png(),
                transform: bounds(),
                duration_ms: Some(1000.0),
            },
        )
        .unwrap();
    for start in [
        MediaStart::Automatically,
        MediaStart::WhenClicked,
        MediaStart::InClickSequence,
    ] {
        let mut playback = MediaPlayback::new(id, MediaKind::Video);
        playback.start = start;
        playback.volume = 50_000;
        playback.loop_until_stopped = true;
        playback.rewind = true;
        playback.full_screen = true;
        playback.hide_when_not_playing = true;
        playback.slides = 999;
        session
            .set_media_playback(0, &playback, Some(1000.0))
            .unwrap();
        let xml = slide_xml(session.bytes(), 0);
        let read = parse_media_playback(&xml);
        assert_eq!(read, vec![playback.clone()], "{start:?}: {xml}");
        assert!(xml.contains("fullScrn=\"1\""));
        assert!(xml.contains("showWhenStopped=\"0\""));
        assert!(xml.contains("repeatCount=\"indefinite\""));
    }
    // The timing goes when the media does, and the deck still reads.
    session.delete_media(0, id).unwrap();
    let xml = slide_xml(session.bytes(), 0);
    assert!(parse_media_playback(&xml).is_empty());
    assert!(!xml.contains(&format!("spid=\"{id}\"")));
}

#[test]
fn trims_fades_bookmarks_and_posters_round_trip() {
    let mut session = EditSession::open(DECK).unwrap();
    let id = session
        .insert_media(
            0,
            &MediaInsert {
                kind: MediaKind::Audio,
                source: MediaSource::Embedded {
                    bytes: &wav(),
                    extension: "wav",
                },
                poster: &png(),
                transform: bounds(),
                duration_ms: Some(1000.0),
            },
        )
        .unwrap();
    let cuts = MediaCuts {
        trim_start_ms: Some(120.0),
        trim_end_ms: Some(250.5),
        fade_in_ms: Some(500.0),
        fade_out_ms: Some(750.0),
        bookmarks: vec![jade_pptx_reader::MediaBookmark {
            name: "Bookmark 1".into(),
            time_ms: 400.0,
        }],
    };
    session.set_media_cuts(0, id, &cuts).unwrap();
    let deck = read_deck(session.bytes()).unwrap();
    let media = deck.slides[0]
        .content
        .shapes
        .iter()
        .find_map(|item| match item {
            ShapeItem::Picture(p) if p.id == Some(id) => p.media.clone(),
            _ => None,
        })
        .unwrap();
    assert_eq!(media.trim_start_ms, Some(120.0));
    assert_eq!(media.trim_end_ms, Some(250.5));
    assert_eq!(media.fade_in_ms, Some(500.0));
    assert_eq!(media.fade_out_ms, Some(750.0));
    assert_eq!(media.bookmarks, cuts.bookmarks);

    let before = OpcPackage::open(session.bytes())
        .unwrap()
        .part_names()
        .len();
    session.set_media_poster(0, id, &png()).unwrap();
    assert!(read_deck(session.bytes()).is_ok());
    // A new poster replaces the old one rather than piling up parts.
    assert!(
        OpcPackage::open(session.bytes())
            .unwrap()
            .part_names()
            .len()
            <= before + 1
    );
}

#[test]
fn powerpoints_own_media_reads_and_takes_new_settings() {
    for (fixture, kind) in [(VIDEO, MediaKind::Video), (AUDIO, MediaKind::Audio)] {
        let deck = read_deck(fixture).unwrap();
        let (slide, id) = deck
            .slides
            .iter()
            .enumerate()
            .find_map(|(index, slide)| {
                slide.content.shapes.iter().find_map(|item| match item {
                    ShapeItem::Picture(p) if p.media.is_some() => Some((index, p.id?)),
                    _ => None,
                })
            })
            .unwrap();
        let mut session = EditSession::open(fixture).unwrap();
        let mut playback = MediaPlayback::new(id, kind);
        playback.start = MediaStart::Automatically;
        playback.loop_until_stopped = true;
        session.set_media_playback(slide, &playback, None).unwrap();
        let read = parse_media_playback(&slide_xml(session.bytes(), slide));
        assert_eq!(read, vec![playback], "{kind:?}");
        // The embedded file is still there and compresses in place.
        let replaced = vec![7u8; 64];
        session
            .replace_media(
                slide,
                id,
                kind,
                &MediaSource::Embedded {
                    bytes: &replaced,
                    extension: "mp4",
                },
            )
            .unwrap();
        let deck = read_deck(session.bytes()).unwrap();
        let part = deck.slides[slide]
            .content
            .shapes
            .iter()
            .find_map(|item| match item {
                ShapeItem::Picture(p) if p.id == Some(id) => p.media.clone()?.media_part,
                _ => None,
            })
            .unwrap();
        assert_eq!(
            OpcPackage::open(session.bytes())
                .unwrap()
                .part(&part)
                .unwrap(),
            replaced.as_slice()
        );
    }
}

/// A deck with drawn ink, embedded audio playing across slides and a
/// trimmed video in the click sequence: what is checked against PowerPoint.
/// `JADE_MEDIA_DECK=path` writes it out.
#[test]
fn a_deck_with_ink_and_media_reads_back_whole() {
    let mut session = EditSession::open(VIDEO).unwrap();
    session
        .edit_ink(
            0,
            &[InkEdit::New(vec![stroke(
                "#C00000",
                &[(40.0, 40.0), (140.0, 90.0), (240.0, 60.0)],
            )])],
        )
        .unwrap();
    let audio = session
        .insert_media(
            0,
            &MediaInsert {
                kind: MediaKind::Audio,
                source: MediaSource::Embedded {
                    bytes: &wav(),
                    extension: "wav",
                },
                poster: &png(),
                transform: Transform {
                    offset_emu: (190_500, 190_500),
                    extent_emu: (609_600, 609_600),
                    ..bounds()
                },
                duration_ms: Some(1000.0),
            },
        )
        .unwrap();
    let mut background = MediaPlayback::new(audio, MediaKind::Audio);
    background.start = MediaStart::Automatically;
    background.slides = 999;
    background.loop_until_stopped = true;
    background.hide_when_not_playing = true;
    session
        .set_media_playback(0, &background, Some(1000.0))
        .unwrap();
    let xml = slide_xml(session.bytes(), 0);
    let playback = parse_media_playback(&xml);
    assert!(playback.contains(&background), "{playback:?}");
    assert_eq!(inks(session.bytes(), 0).len(), 1);
    if let Ok(path) = std::env::var("JADE_MEDIA_DECK") {
        std::fs::write(path, session.bytes()).unwrap();
    }
}

#[test]
fn moving_and_resizing_ink_takes_its_strokes_along() {
    let mut session = EditSession::open(DECK).unwrap();
    let id = session
        .edit_ink(
            0,
            &[InkEdit::New(vec![stroke(
                "#000000",
                &[(100.0, 100.0), (200.0, 200.0)],
            )])],
        )
        .unwrap()[0];
    let ink = &inks(session.bytes(), 0)[0];
    let mut moved = ink.transform.unwrap();
    moved.offset_emu.0 += 952_500;
    moved.extent_emu.0 *= 2;
    session.set_shape_transform(0, id, moved).unwrap();
    let after = &inks(session.bytes(), 0)[0];
    assert_eq!(after.transform.unwrap().offset_emu, moved.offset_emu);
    assert_eq!(after.transform.unwrap().extent_emu, moved.extent_emu);
    // The strokes fill the wider box.
    let last = *after.traces[0].points.last().unwrap();
    let first = after.traces[0].points[0];
    assert!((last.0 - first.0) > 150.0, "{:?}", after.traces[0].points);
}
