use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use jade_opc::OpcPackage;
use jade_pptx_reader::{SourceRect, Transform};
use jade_pptx_writer::{editable_shapes, picture_crop, EditSession};
use std::io::Cursor;
const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");
fn inserted() -> (Vec<u8>, u32) {
    let mut image = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(RgbImage::from_pixel(40, 20, Rgb([200, 20, 30])))
        .write_to(&mut image, ImageFormat::Png)
        .unwrap();
    let mut session = EditSession::open(DECK).unwrap();
    let inserted = session
        .insert_picture(
            0,
            Transform {
                offset_emu: (952500, 952500),
                extent_emu: (3810000, 1905000),
                rotation_60000ths: 0,
                flip_h: false,
                flip_v: false,
            },
            image.get_ref(),
        )
        .unwrap();
    (session.bytes().to_vec(), inserted.id)
}
fn xml(bytes: &[u8]) -> String {
    OpcPackage::open(bytes)
        .unwrap()
        .required_part_str("ppt/slides/slide1.xml")
        .unwrap()
}
fn edited(bytes: &[u8], value: &str) -> Vec<u8> {
    let mut p = OpcPackage::open(bytes).unwrap();
    p.replace("ppt/slides/slide1.xml", value.as_bytes().to_vec());
    p.write().unwrap()
}

#[test]
fn cropping_preserves_image_scale_data_rotation_and_aspect_lock_in_one_undo() {
    let (bytes, id) = inserted();
    let mut s = EditSession::open(&bytes).unwrap();
    assert!(!s
        .set_picture_crop(0, id, SourceRect::default(), false)
        .unwrap());
    assert_eq!(s.bytes(), bytes);
    let crop = SourceRect {
        left: 25000,
        ..Default::default()
    };
    assert!(s.set_picture_crop(0, id, crop, false).unwrap());
    let info = editable_shapes(s.bytes(), 0)
        .unwrap()
        .into_iter()
        .find(|p| p.id == id)
        .unwrap();
    assert_eq!(info.transform.unwrap().offset_emu, (1905000, 952500));
    assert_eq!(info.transform.unwrap().extent_emu, (2857500, 1905000));
    assert!(info.lock_aspect_ratio);
    let old = OpcPackage::open(&bytes).unwrap();
    let new = OpcPackage::open(s.bytes()).unwrap();
    for part in old
        .part_names()
        .iter()
        .filter(|p| p.as_str() != "ppt/slides/slide1.xml")
    {
        assert_eq!(old.part(part), new.part(part), "{part}");
    }
    let cropped = s.bytes().to_vec();
    assert!(s.undo());
    assert_eq!(s.bytes(), bytes);
    assert!(!s.undo());
    assert!(s.redo());
    assert_eq!(s.bytes(), cropped);
    assert_eq!(picture_crop(s.bytes(), 0, id).unwrap().unwrap().crop, crop);
}

#[test]
fn fixed_frame_crop_negative_outsets_reset_and_unchanged_lexical_offsets_are_preserved() {
    let (bytes, id) = inserted();
    let value=xml(&bytes).replace("<jadeDraw:stretch>","<c:srcRect xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/main\" l=\"12.5%\" r=\"000\" xmlns:k=\"urn:test\" k:keep=\"&amp;\"/><jadeDraw:stretch>");
    let bytes = edited(&bytes, &value);
    let mut s = EditSession::open(&bytes).unwrap();
    assert!(!s
        .set_picture_crop(
            0,
            id,
            SourceRect {
                left: 12500,
                ..Default::default()
            },
            true
        )
        .unwrap());
    assert_eq!(s.bytes(), bytes);
    s.set_picture_crop(
        0,
        id,
        SourceRect {
            left: 12500,
            top: -10000,
            ..Default::default()
        },
        true,
    )
    .unwrap();
    assert!(xml(s.bytes()).contains("l=\"12.5%\" r=\"000\""));
    assert!(xml(s.bytes()).contains("k:keep=\"&amp;\""));
    assert_eq!(
        editable_shapes(s.bytes(), 0)
            .unwrap()
            .last()
            .unwrap()
            .transform,
        editable_shapes(&bytes, 0)
            .unwrap()
            .last()
            .unwrap()
            .transform
    );
    s.set_picture_crop(0, id, SourceRect::default(), true)
        .unwrap();
    assert_eq!(
        picture_crop(s.bytes(), 0, id).unwrap().unwrap().crop,
        SourceRect::default()
    );
}

#[test]
fn invalid_ranges_and_crop_locks_do_not_mutate_bytes_or_history() {
    let (bytes, id) = inserted();
    let mut s = EditSession::open(&bytes).unwrap();
    for crop in [
        SourceRect {
            left: 100000,
            ..Default::default()
        },
        SourceRect {
            left: 60000,
            right: 40000,
            ..Default::default()
        },
        SourceRect {
            top: 100000,
            bottom: -50000,
            ..Default::default()
        },
    ] {
        assert!(s.set_picture_crop(0, id, crop, true).is_err());
    }
    assert!(s
        .set_picture_crop(0, 2, SourceRect::default(), true)
        .is_err());
    assert_eq!(s.bytes(), bytes);
    assert!(!s.undo());
    for lock in ["noCrop=\"1\"", "noSelect=\"true\""] {
        let locked = edited(
            &bytes,
            &xml(&bytes).replace(
                "noChangeAspect=\"1\"",
                &format!("noChangeAspect=\"1\" {lock}"),
            ),
        );
        let mut s = EditSession::open(&locked).unwrap();
        assert!(s
            .set_picture_crop(
                0,
                id,
                SourceRect {
                    left: 10000,
                    ..Default::default()
                },
                true
            )
            .is_err());
        assert_eq!(s.bytes(), locked);
        assert!(!s.undo());
    }
    let locked = edited(
        &bytes,
        &xml(&bytes).replace(
            "noChangeAspect=\"1\"",
            "noChangeAspect=\"1\" noResize=\"1\"",
        ),
    );
    let mut s = EditSession::open(&locked).unwrap();
    assert!(s
        .set_picture_crop(
            0,
            id,
            SourceRect {
                left: 10000,
                ..Default::default()
            },
            false
        )
        .is_err());
    assert!(s
        .set_picture_crop(
            0,
            id,
            SourceRect {
                left: 10000,
                ..Default::default()
            },
            true
        )
        .unwrap());
}

#[test]
fn paired_alternate_content_updates_every_branch_instead_of_only_the_fallback() {
    let (bytes, id) = inserted();
    let original = xml(&bytes);
    let start = original.find("<jadeSp:pic ").unwrap();
    let end = start + original[start..].find("</jadeSp:pic>").unwrap() + "</jadeSp:pic>".len();
    let pic = &original[start..end];
    let wrapper=format!("<mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><mc:Choice xmlns:req=\"urn:test\" Requires=\"req\">{pic}</mc:Choice><mc:Fallback>{pic}</mc:Fallback></mc:AlternateContent>");
    let bytes = edited(
        &bytes,
        &format!("{}{}{}", &original[..start], wrapper, &original[end..]),
    );
    let mut s = EditSession::open(&bytes).unwrap();
    assert!(
        picture_crop(&bytes, 0, id)
            .unwrap()
            .unwrap()
            .can_preserve_scale
    );
    s.set_picture_crop(
        0,
        id,
        SourceRect {
            left: 25000,
            ..Default::default()
        },
        false,
    )
    .unwrap();
    assert_eq!(xml(s.bytes()).matches("cropDraw:srcRect").count(), 2);
    assert_eq!(xml(s.bytes()).matches("x=\"1905000\"").count(), 2);
}

#[test]
fn picture_filled_shapes_keep_their_outline_bounds_and_reject_tiling() {
    let (bytes, id) = inserted();
    let value = xml(&bytes)
        .replace("jadeSp:pic", "jadeSp:sp")
        .replace("jadeSp:nvPicPr", "jadeSp:nvSpPr")
        .replace("jadeSp:cNvPicPr", "jadeSp:cNvSpPr")
        .replace("jadeDraw:picLocks", "jadeDraw:spLocks");
    // Move the fill from the picture slot into the shape properties.
    let a = value.find("<jadeSp:blipFill>").unwrap();
    let b = a + value[a..].find("</jadeSp:blipFill>").unwrap() + "</jadeSp:blipFill>".len();
    let fill = value[a..b].replace("jadeSp:blipFill", "jadeDraw:blipFill");
    let value = format!("{}{}", &value[..a], &value[b..])
        .replace("</jadeSp:spPr>", &format!("{fill}</jadeSp:spPr>"));
    let bytes = edited(&bytes, &value);
    let mut s = EditSession::open(&bytes).unwrap();
    let before = editable_shapes(&bytes, 0)
        .unwrap()
        .last()
        .unwrap()
        .transform;
    assert!(!picture_crop(&bytes, 0, id).unwrap().unwrap().picture);
    s.set_picture_crop(
        0,
        id,
        SourceRect {
            left: 25000,
            ..Default::default()
        },
        false,
    )
    .unwrap();
    assert_eq!(
        editable_shapes(s.bytes(), 0)
            .unwrap()
            .last()
            .unwrap()
            .transform,
        before
    );
    let tiled = edited(
        &bytes,
        &value.replace(
            "<jadeDraw:stretch><jadeDraw:fillRect/></jadeDraw:stretch>",
            "<jadeDraw:tile/>",
        ),
    );
    assert!(!picture_crop(&tiled, 0, id).unwrap().unwrap().can_edit);
}
