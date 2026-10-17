use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use jade_opc::OpcPackage;
use jade_pptx_reader::{SourceRect, Transform};
use jade_pptx_writer::{editable_shapes, picture_replacement, EditSession};
use std::io::Cursor;
const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");
fn image(format: ImageFormat, color: [u8; 3]) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(RgbImage::from_pixel(40, 20, Rgb(color)))
        .write_to(&mut out, format)
        .unwrap();
    out.into_inner()
}
fn inserted() -> (Vec<u8>, u32) {
    let mut s = EditSession::open(DECK).unwrap();
    let result = s
        .insert_picture(
            0,
            Transform {
                offset_emu: (952500, 952500),
                extent_emu: (3810000, 1905000),
                rotation_60000ths: 1800000,
                flip_h: true,
                flip_v: false,
            },
            &image(ImageFormat::Png, [230, 20, 20]),
        )
        .unwrap();
    s.set_picture_crop(
        0,
        result.id,
        SourceRect {
            left: 25000,
            top: -10000,
            ..Default::default()
        },
        true,
    )
    .unwrap();
    (s.bytes().to_vec(), result.id)
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
fn token(bytes: &[u8], id: u32) -> String {
    picture_replacement(bytes, 0, id)
        .unwrap()
        .unwrap()
        .fingerprint
}

#[test]
fn replacing_one_shared_picture_preserves_crop_transform_source_media_and_undo() {
    let (bytes, id) = inserted();
    let mut s = EditSession::open(&bytes).unwrap();
    let duplicate = s.duplicate_shape(0, id).unwrap();
    let before = s.bytes().to_vec();
    let old = OpcPackage::open(&before).unwrap();
    let replacement = image(ImageFormat::Jpeg, [20, 20, 230]);
    let result = s
        .replace_picture(0, id, &replacement, &token(&before, id))
        .unwrap();
    assert!(result.changed);
    let package = OpcPackage::open(s.bytes()).unwrap();
    assert_eq!(package.part(&result.part), Some(replacement.as_slice()));
    for part in old.part_names().iter().filter(|p| {
        ![
            "ppt/slides/slide1.xml",
            "ppt/slides/_rels/slide1.xml.rels",
            "[Content_Types].xml",
        ]
        .contains(&p.as_str())
    }) {
        assert_eq!(old.part(part), package.part(part), "{part}");
    }
    assert_eq!(
        editable_shapes(&before, 0)
            .unwrap()
            .iter()
            .find(|s| s.id == id)
            .unwrap()
            .transform,
        editable_shapes(s.bytes(), 0)
            .unwrap()
            .iter()
            .find(|s| s.id == id)
            .unwrap()
            .transform
    );
    assert_eq!(
        jade_pptx_writer::picture_crop(&before, 0, id)
            .unwrap()
            .unwrap()
            .crop,
        jade_pptx_writer::picture_crop(s.bytes(), 0, id)
            .unwrap()
            .unwrap()
            .crop
    );
    let old_deck = jade_pptx_reader::read_deck(&before).unwrap();
    let new_deck = jade_pptx_reader::read_deck(s.bytes()).unwrap();
    let old_picture = old_deck.slides[0]
        .content
        .shapes
        .iter()
        .filter_map(|s| match s {
            jade_pptx_reader::ShapeItem::Picture(p) => Some(p),
            _ => None,
        })
        .find(|s| s.id == Some(duplicate))
        .unwrap();
    let new_picture = new_deck.slides[0]
        .content
        .shapes
        .iter()
        .filter_map(|s| match s {
            jade_pptx_reader::ShapeItem::Picture(p) => Some(p),
            _ => None,
        })
        .find(|s| s.id == Some(duplicate))
        .unwrap();
    assert_eq!(old_picture.blip_rel_id, new_picture.blip_rel_id);
    let after = s.bytes().to_vec();
    assert!(s.undo());
    assert_eq!(s.bytes(), before);
    assert!(s.redo());
    assert_eq!(s.bytes(), after);
    assert!(
        !s.replace_picture(0, id, &replacement, &token(s.bytes(), id))
            .unwrap()
            .changed
    );
    assert_eq!(s.bytes(), after);
}
#[test]
fn replacement_rejects_stale_invalid_locked_external_or_variant_images_without_history() {
    let (bytes, id) = inserted();
    let replacement = image(ImageFormat::Png, [20, 230, 20]);
    let mut s = EditSession::open(&bytes).unwrap();
    assert!(s
        .replace_picture(0, id, b"broken", &token(&bytes, id))
        .is_err());
    assert!(s.replace_picture(0, id, &replacement, "stale").is_err());
    assert_eq!(s.bytes(), bytes);
    assert!(!s.undo());
    for value in [
        xml(&bytes).replace(
            "noChangeAspect=\"1\"",
            "noChangeAspect=\"1\" noSelect=\"1\"",
        ),
        xml(&bytes).replace(
            "<jadeDraw:blip ",
            "<jadeDraw:blip jadeRel:link=\"external\" ",
        ),
        xml(&bytes).replace(
            "/><cropDraw:srcRect",
            "><jadeDraw:extLst/></jadeDraw:blip><cropDraw:srcRect",
        ),
    ] {
        let locked = edited(&bytes, &value);
        let state = picture_replacement(&locked, 0, id).unwrap().unwrap();
        assert!(!state.can_replace, "{value}");
        let mut s = EditSession::open(&locked).unwrap();
        assert!(s
            .replace_picture(0, id, &replacement, &state.fingerprint)
            .is_err());
        assert_eq!(s.bytes(), locked);
        assert!(!s.undo());
    }
}
#[test]
fn replacement_updates_both_alternate_content_branches_and_preserves_locks() {
    let (bytes, id) = inserted();
    let original = xml(&bytes).replace(
        "noChangeAspect=\"1\"",
        "noChangeAspect=\"1\" noCrop=\"1\" noMove=\"1\" noResize=\"1\"",
    );
    let start = original.find("<jadeSp:pic ").unwrap();
    let end = start + original[start..].find("</jadeSp:pic>").unwrap() + "</jadeSp:pic>".len();
    let pic = &original[start..end];
    let wrapper=format!("<mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><mc:Choice xmlns:req=\"urn:test\" Requires=\"req\">{pic}</mc:Choice><mc:Fallback>{pic}</mc:Fallback></mc:AlternateContent>");
    let bytes = edited(
        &bytes,
        &format!("{}{}{}", &original[..start], wrapper, &original[end..]),
    );
    let mut s = EditSession::open(&bytes).unwrap();
    let result = s
        .replace_picture(
            0,
            id,
            &image(ImageFormat::Png, [20, 230, 20]),
            &token(&bytes, id),
        )
        .unwrap();
    assert!(result.changed);
    let value = xml(s.bytes());
    assert_eq!(
        value.matches("jadeRel:embed=\"rIdJadePicture2\"").count(),
        2
    );
    assert_eq!(
        value
            .matches("noCrop=\"1\" noMove=\"1\" noResize=\"1\"")
            .count(),
        2
    );
}
#[test]
fn image_filled_shapes_retain_their_geometry_and_fill_window() {
    let (bytes, id) = inserted();
    let original = xml(&bytes);
    let start = original.find("<jadeSp:pic ").unwrap();
    let end = start + original[start..].find("</jadeSp:pic>").unwrap() + "</jadeSp:pic>".len();
    let pic = &original[start..end];
    let fill_start = pic.find("<jadeSp:blipFill>").unwrap();
    let fill_end = pic.find("</jadeSp:blipFill>").unwrap() + "</jadeSp:blipFill>".len();
    let fill = pic[fill_start..fill_end].replace("jadeSp:blipFill", "jadeDraw:blipFill");
    let mut shape = pic
        .replace(&pic[fill_start..fill_end], "")
        .replace("jadeSp:pic", "jadeSp:sp")
        .replace("jadeSp:nvPicPr", "jadeSp:nvSpPr")
        .replace("jadeSp:cNvPicPr", "jadeSp:cNvSpPr")
        .replace("jadeDraw:picLocks", "jadeDraw:spLocks");
    shape = shape.replace("</jadeSp:spPr>", &format!("{fill}</jadeSp:spPr>"));
    let bytes = edited(
        &bytes,
        &format!("{}{}{}", &original[..start], shape, &original[end..]),
    );
    let mut s = EditSession::open(&bytes).unwrap();
    assert!(
        s.replace_picture(
            0,
            id,
            &image(ImageFormat::Png, [20, 230, 20]),
            &token(&bytes, id)
        )
        .unwrap()
        .changed
    );
    assert_eq!(
        jade_pptx_writer::picture_crop(&bytes, 0, id)
            .unwrap()
            .unwrap()
            .crop,
        jade_pptx_writer::picture_crop(s.bytes(), 0, id)
            .unwrap()
            .unwrap()
            .crop
    );
}
