use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use jade_opc::{parse_relationships, OpcPackage};
use jade_pptx_reader::Transform;
use jade_pptx_writer::{editable_shapes, picture_info, EditSession};
use sha2::{Digest, Sha256};
use std::io::Cursor;
const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");
fn pixels(format: ImageFormat, color: [u8; 3]) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(RgbImage::from_pixel(40, 20, Rgb(color)))
        .write_to(&mut out, format)
        .unwrap();
    out.into_inner()
}
fn bounds() -> Transform {
    Transform {
        offset_emu: (952500, 952500),
        extent_emu: (3810000, 1905000),
        rotation_60000ths: 0,
        flip_h: false,
        flip_v: false,
    }
}
fn xml(bytes: &[u8]) -> String {
    OpcPackage::open(bytes)
        .unwrap()
        .required_part_str("ppt/slides/slide1.xml")
        .unwrap()
}

#[test]
fn png_and_jpeg_insertions_preserve_pixels_and_existing_parts_with_exact_undo_redo() {
    for format in [ImageFormat::Png, ImageFormat::Jpeg] {
        let image = pixels(format, [200, 20, 30]);
        let mut session = EditSession::open(DECK).unwrap();
        let original = OpcPackage::open(DECK).unwrap();
        let inserted = session.insert_picture(0, bounds(), &image).unwrap();
        let saved = session.bytes().to_vec();
        let package = OpcPackage::open(&saved).unwrap();
        assert_eq!(package.part(&inserted.part).unwrap(), image);
        for part in original.part_names() {
            if ![
                "ppt/slides/slide1.xml",
                "ppt/slides/_rels/slide1.xml.rels",
                "[Content_Types].xml",
            ]
            .contains(&part.as_str())
            {
                assert_eq!(original.part(part), package.part(part), "{part}");
            }
        }
        let objects = editable_shapes(&saved, 0).unwrap();
        let picture = objects.last().unwrap();
        assert_eq!(picture.id, inserted.id);
        assert_eq!(picture.kind, "picture");
        assert_eq!(picture.transform, Some(bounds()));
        assert!(picture.lock_aspect_ratio);
        let rels = parse_relationships(
            &package
                .required_part_str("ppt/slides/_rels/slide1.xml.rels")
                .unwrap(),
        )
        .unwrap();
        assert!(rels
            .by_type_suffix("/image")
            .iter()
            .any(|rel| rel.target == format!("/{}", inserted.part) && !rel.external));
        assert!(session.undo());
        assert_eq!(session.bytes(), DECK);
        assert!(!session.undo());
        assert!(session.redo());
        assert_eq!(session.bytes(), saved);
        assert_eq!(
            OpcPackage::open(EditSession::open(session.bytes()).unwrap().bytes())
                .unwrap()
                .part(&inserted.part)
                .unwrap(),
            image
        );
    }
}

#[test]
fn invalid_images_and_bounds_do_not_change_history_or_the_document() {
    let image = pixels(ImageFormat::Png, [0, 0, 255]);
    let mut session = EditSession::open(DECK).unwrap();
    for bytes in [&b"not an image"[..], &image[..20], &b"GIF89a"[..]] {
        assert!(session.insert_picture(0, bounds(), bytes).is_err());
    }
    let mut broken = image.clone();
    let last = broken.len() - 5;
    broken[last] ^= 0xff;
    assert!(session.insert_picture(0, bounds(), &broken).is_err());
    assert!(session
        .insert_picture(
            0,
            Transform {
                extent_emu: (0, 20),
                ..bounds()
            },
            &image
        )
        .is_err());
    assert!(session.insert_picture(99, bounds(), &image).is_err());
    assert_eq!(session.bytes(), DECK);
    assert!(!session.undo());
}

#[test]
fn media_names_track_content_across_undo_branches_and_do_not_overwrite_or_activate_reserved_parts()
{
    let red = pixels(ImageFormat::Png, [255, 0, 0]);
    let blue = pixels(ImageFormat::Png, [0, 0, 255]);
    let reserved = format!("ppt/media/jade-picture-{:x}.png", Sha256::digest(&red));
    let mut package = OpcPackage::open(DECK).unwrap();
    let relationships = package.required_part_str("ppt/slides/_rels/slide1.xml.rels").unwrap().replace("</Relationships>", &format!("<Relationship Id=\"rIdJadePicture1\" Type=\"urn:test\" Target=\"/{reserved}\"/></Relationships>"));
    package.replace(
        "ppt/slides/_rels/slide1.xml.rels",
        relationships.into_bytes(),
    );
    let old_xml = package.required_part_str("ppt/slides/slide1.xml").unwrap().replace("</p:sld>", "<p:extLst><p:ext uri=\"test\"><x xmlns=\"urn:test\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" r:embed=\"rIdJadePicture2\"/></p:ext></p:extLst></p:sld>");
    package.replace("ppt/slides/slide1.xml", old_xml.into_bytes());
    let before = package.write().unwrap();
    let mut session = EditSession::open(&before).unwrap();
    let first = session.insert_picture(0, bounds(), &red).unwrap();
    assert_ne!(first.part, reserved);
    assert!(OpcPackage::open(session.bytes())
        .unwrap()
        .part(&reserved)
        .is_none());
    assert!(xml(session.bytes()).contains("jadeRel:embed=\"rIdJadePicture3\""));
    assert!(session.undo());
    let second = session.insert_picture(0, bounds(), &blue).unwrap();
    assert_ne!(first.part, second.part);
    assert!(session.undo());
    let again = session.insert_picture(0, bounds(), &red).unwrap();
    assert_eq!(first.part, again.part);
}

#[test]
fn strict_slide_namespaces_and_missing_relationship_parts_are_supported() {
    let mut package = OpcPackage::open(DECK).unwrap();
    let strict = xml(DECK)
        .replace(
            "http://schemas.openxmlformats.org/presentationml/2006/main",
            "http://purl.oclc.org/ooxml/presentationml/main",
        )
        .replace(
            "http://schemas.openxmlformats.org/drawingml/2006/main",
            "http://purl.oclc.org/ooxml/drawingml/main",
        );
    package.replace("ppt/slides/slide1.xml", strict.into_bytes());
    package.remove("ppt/slides/_rels/slide1.xml.rels");
    let mut session = EditSession::open(&package.write().unwrap()).unwrap();
    let inserted = session
        .insert_picture(0, bounds(), &pixels(ImageFormat::Png, [1, 2, 3]))
        .unwrap();
    let result = OpcPackage::open(session.bytes()).unwrap();
    assert!(xml(session.bytes())
        .contains("xmlns:jadeSp=\"http://purl.oclc.org/ooxml/presentationml/main\""));
    assert!(result
        .required_part_str("ppt/slides/_rels/slide1.xml.rels")
        .unwrap()
        .contains("http://purl.oclc.org/ooxml/officeDocument/relationships/image"));
    assert_eq!(
        result.part(&inserted.part).unwrap(),
        pixels(ImageFormat::Png, [1, 2, 3])
    );
}

#[test]
fn jpeg_exif_orientation_reports_display_dimensions_without_reencoding_bytes() {
    let jpeg = pixels(ImageFormat::Jpeg, [1, 2, 3]);
    let exif = b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let mut bytes = jpeg[..2].to_vec();
    bytes.extend([0xff, 0xe1]);
    bytes.extend(((exif.len() + 2) as u16).to_be_bytes());
    bytes.extend(exif);
    bytes.extend(&jpeg[2..]);
    let info = picture_info(&bytes).unwrap();
    assert_eq!(
        (info.width, info.height, info.mime_type),
        (20, 40, "image/jpeg")
    );
    let mut session = EditSession::open(DECK).unwrap();
    let inserted = session.insert_picture(0, bounds(), &bytes).unwrap();
    assert_eq!(
        OpcPackage::open(session.bytes())
            .unwrap()
            .part(&inserted.part)
            .unwrap(),
        bytes
    );
}
