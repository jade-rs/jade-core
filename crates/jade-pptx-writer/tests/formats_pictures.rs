//! The 図の形式 tab on a picture: each command written the way PowerPoint
//! writes it, read back as the effect it names, undone in one step, and
//! the picture's own bytes never touched — except by 図の圧縮.

use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use jade_opc::OpcPackage;
use jade_pptx_reader::{read_deck, BlipEffect, Picture, ShapeItem, Transform};
use jade_pptx_writer::{
    picture_format, EditSession, PictureEdit, PictureEffect, Recolor, PICTURE_STYLES,
};
use std::io::Cursor;

const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(RgbImage::from_fn(width, height, |x, y| {
        Rgb([
            (x * 7 % 256) as u8,
            (y * 5 % 256) as u8,
            ((x ^ y) % 256) as u8,
        ])
    }))
    .write_to(&mut out, ImageFormat::Png)
    .unwrap();
    out.into_inner()
}

fn bounds() -> Transform {
    Transform {
        offset_emu: (952_500, 952_500),
        extent_emu: (3_810_000, 1_905_000),
        rotation_60000ths: 0,
        flip_h: false,
        flip_v: false,
    }
}

fn picture(bytes: &[u8], id: u32) -> Picture {
    read_deck(bytes).unwrap().slides[0]
        .content
        .shapes
        .iter()
        .find_map(|item| match item {
            ShapeItem::Picture(p) if p.id == Some(id) => Some(p.clone()),
            _ => None,
        })
        .expect("the picture")
}

fn slide(bytes: &[u8]) -> String {
    OpcPackage::open(bytes)
        .unwrap()
        .required_part_str("ppt/slides/slide1.xml")
        .unwrap()
}

fn session_with_picture() -> (EditSession, u32) {
    let mut session = EditSession::open(DECK).unwrap();
    let id = session
        .insert_picture(0, bounds(), &png(80, 40))
        .unwrap()
        .id;
    (session, id)
}

#[test]
fn corrections_colour_and_artistic_effects_read_back_in_powerpoints_order() {
    let (mut session, id) = session_with_picture();
    let before = session.bytes().to_vec();
    for edit in [
        PictureEdit::Corrections {
            brightness: 0.2,
            contrast: -0.2,
        },
        PictureEdit::Sharpness(0.25),
        PictureEdit::Saturation(2.0),
        PictureEdit::Temperature(8800.0),
        PictureEdit::Artistic(Some("artisticPencilSketch".into())),
        PictureEdit::RemoveBackground(Some([0.1, 0.05, 0.9, 0.95])),
        PictureEdit::Recolor(Recolor::Grayscale),
        PictureEdit::Transparency(0.4),
        PictureEdit::TransparentColor(Some("#FFFFFF".into())),
    ] {
        assert!(session.format_picture(0, id, &edit).unwrap(), "{edit:?}");
    }
    let read = picture(session.bytes(), id);
    assert_eq!(read.alpha, Some(0.6));
    let effects = &read.blip_effects;
    assert!(effects.contains(&BlipEffect::Grayscale));
    assert!(effects.contains(&BlipEffect::Luminance {
        brightness: 0.2,
        contrast: -0.2
    }));
    assert!(effects
        .iter()
        .any(|e| matches!(e, BlipEffect::ColorChange { .. })));
    let image: Vec<&BlipEffect> = effects
        .iter()
        .filter(|e| {
            !matches!(
                e,
                BlipEffect::Grayscale
                    | BlipEffect::ColorChange { .. }
                    | BlipEffect::Luminance { .. }
            )
        })
        .collect();
    assert_eq!(
        image,
        vec![
            &BlipEffect::BackgroundRemoval([0.1, 0.05, 0.9, 0.95]),
            &BlipEffect::Artistic("artisticPencilSketch".into()),
            &BlipEffect::Sharpen(0.25),
            &BlipEffect::Temperature(8800.0),
            &BlipEffect::Saturation(2.0),
        ]
    );
    // Office bakes these into a copy the blip shows, the original kept in
    // the image layer.
    let original = picture(&before, id).image_part.unwrap();
    let baked = read.image_part.clone().unwrap();
    assert_ne!(baked, original);
    let xml = slide(session.bytes());
    assert!(
        xml.contains("<a14:imgLayer jadeRel:embed=\"rIdJadePicture1\">"),
        "{xml}"
    );
    let package = OpcPackage::open(session.bytes()).unwrap();
    assert_ne!(package.part(&baked), package.part(&original));
    // Changing one replaces it; the neutral value takes it away.
    session
        .format_picture(0, id, &PictureEdit::Artistic(Some("artisticBlur".into())))
        .unwrap();
    session
        .format_picture(0, id, &PictureEdit::Saturation(1.0))
        .unwrap();
    let effects = picture(session.bytes(), id).blip_effects;
    assert!(effects.contains(&BlipEffect::Artistic("artisticBlur".into())));
    assert!(!effects
        .iter()
        .any(|e| matches!(e, BlipEffect::Saturation(_))));
    // 図のリセット takes everything away and shows the picture as inserted;
    // the baked copies go.
    session.format_picture(0, id, &PictureEdit::Reset).unwrap();
    let read = picture(session.bytes(), id);
    assert!(read.blip_effects.is_empty() && read.alpha.is_none());
    assert_eq!(read.image_part.as_deref(), Some(original.as_str()));
    let package = OpcPackage::open(session.bytes()).unwrap();
    assert_eq!(
        package.part(&original),
        OpcPackage::open(&before).unwrap().part(&original)
    );
    assert_eq!(
        package
            .part_names()
            .iter()
            .filter(|name| name.starts_with("ppt/media/"))
            .count(),
        OpcPackage::open(&before)
            .unwrap()
            .part_names()
            .iter()
            .filter(|name| name.starts_with("ppt/media/"))
            .count()
    );
}

#[test]
fn recolour_presets_are_the_blip_transforms_powerpoint_writes() {
    let (mut session, id) = session_with_picture();
    let cases = [
        (Recolor::Sepia, "duotone"),
        (Recolor::Washout, "lum"),
        (Recolor::BlackWhite(50), "biLevel"),
        (Recolor::Dark("accent1".into()), "duotone"),
        (Recolor::Light("accent2".into()), "duotone"),
    ];
    for (recolor, element) in cases {
        session
            .format_picture(0, id, &PictureEdit::Recolor(recolor.clone()))
            .unwrap();
        let xml = slide(session.bytes());
        assert!(xml.contains(&format!(":{element}")), "{recolor:?}");
        // Only one recolour at a time.
        let count = ["grayscl", "duotone", "biLevel", ":lum "]
            .iter()
            .filter(|name| xml.contains(*name))
            .count();
        assert_eq!(count, 1, "{recolor:?}");
    }
    session
        .format_picture(0, id, &PictureEdit::Recolor(Recolor::None))
        .unwrap();
    assert!(picture(session.bytes(), id).blip_effects.is_empty());
}

#[test]
fn styles_borders_effects_shape_and_aspect_lock() {
    let (mut session, id) = session_with_picture();
    session
        .format_picture(0, id, &PictureEdit::Style(6))
        .unwrap();
    let read = picture(session.bytes(), id);
    assert_eq!(
        read.preset_geometry.as_deref(),
        Some(PICTURE_STYLES[6].geometry_name())
    );
    assert!(read.effects.soft_edge_emu.is_some());
    session
        .format_picture(
            0,
            id,
            &PictureEdit::Border {
                color: Some("#C00000".into()),
                width_pt: Some(3.0),
            },
        )
        .unwrap();
    session
        .format_picture(0, id, &PictureEdit::Effect(PictureEffect::Shadow(true)))
        .unwrap();
    session
        .format_picture(0, id, &PictureEdit::Effect(PictureEffect::Reflection(true)))
        .unwrap();
    session
        .format_picture(
            0,
            id,
            &PictureEdit::Effect(PictureEffect::Glow(Some(("#FFC000".into(), 8.0)))),
        )
        .unwrap();
    let read = picture(session.bytes(), id);
    assert!(read.outline.is_some());
    assert!(read.shadow.is_some());
    assert!(read.effects.reflection.is_some() && read.effects.glow.is_some());
    // The effect list keeps the schema's order.
    let xml = slide(session.bytes());
    let (glow, shadow, reflection, soft) = (
        xml.find(":glow ").unwrap(),
        xml.find(":outerShdw ").unwrap(),
        xml.find(":reflection ").unwrap(),
        xml.find(":softEdge ").unwrap(),
    );
    assert!(glow < shadow && shadow < reflection && reflection < soft);
    session
        .format_picture(0, id, &PictureEdit::CropToShape("hexagon".into()))
        .unwrap();
    assert_eq!(
        picture(session.bytes(), id).preset_geometry.as_deref(),
        Some("hexagon")
    );
    session
        .format_picture(0, id, &PictureEdit::LockAspect(false))
        .unwrap();
    assert!(!picture_format(session.bytes(), 0, id).unwrap().lock_aspect);
    session
        .format_picture(0, id, &PictureEdit::LockAspect(true))
        .unwrap();
    let info = picture_format(session.bytes(), 0, id).unwrap();
    assert!(info.lock_aspect);
    assert_eq!(info.pixels, Some((80, 40)));
    // 図とサイズのリセット: plain, uncropped, at its own size (96 dpi).
    session
        .set_picture_crop(
            0,
            id,
            jade_pptx_reader::SourceRect {
                left: 10_000,
                ..Default::default()
            },
            true,
        )
        .unwrap();
    session
        .format_picture(0, id, &PictureEdit::ResetWithSize)
        .unwrap();
    let read = picture(session.bytes(), id);
    assert_eq!(read.preset_geometry.as_deref(), Some("rect"));
    assert!(read.source_rect.is_none_or(|r| r == Default::default()));
    assert_eq!(read.transform.unwrap().extent_emu, (80 * 9525, 40 * 9525));
    assert!(read.outline.is_none() && read.shadow.is_none());
}

#[test]
fn each_command_is_one_undo() {
    let (mut session, id) = session_with_picture();
    let before = session.bytes().to_vec();
    session
        .format_picture(0, id, &PictureEdit::Recolor(Recolor::Grayscale))
        .unwrap();
    assert!(session.undo());
    assert_eq!(session.bytes(), before);
}

#[test]
fn compression_downsamples_to_the_shown_size_and_drops_cropped_parts() {
    let mut session = EditSession::open(DECK).unwrap();
    let big = png(2000, 1000);
    let small_frame = Transform {
        extent_emu: (914_400, 457_200),
        ..bounds()
    };
    let id = session.insert_picture(0, small_frame, &big).unwrap().id;
    session
        .set_picture_crop(
            0,
            id,
            jade_pptx_reader::SourceRect {
                left: 50_000,
                ..Default::default()
            },
            true,
        )
        .unwrap();
    let count = session.compress_pictures(Some((0, id)), 150, true).unwrap();
    assert_eq!(count, 1);
    let read = picture(session.bytes(), id);
    assert!(read.source_rect.is_none_or(|r| r == Default::default()));
    let package = OpcPackage::open(session.bytes()).unwrap();
    let bytes = package.part(read.image_part.as_deref().unwrap()).unwrap();
    let image = image::load_from_memory(bytes).unwrap();
    // One inch by half an inch at 150 ppi.
    assert!(
        image.width() <= 150 && image.height() <= 75,
        "{}x{}",
        image.width(),
        image.height()
    );
    assert!(bytes.len() < big.len());
    // The old part went with its last reference.
    assert!(!package
        .part_names()
        .iter()
        .any(|name| package.part(name) == Some(big.as_slice())));
}

#[test]
fn compression_keeps_svg_crop_and_fallback_together() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="512"><path d="M0 0h1024v512H0z" fill="#FF0000"/><path d="M0 0L1024 512H0z" fill="#0000FF"/></svg>"##;
    for all_pictures in [false, true] {
        for delete_cropped in [true, false] {
            let mut session = EditSession::open(DECK).unwrap();
            let placement = Transform {
                extent_emu: (914_400, 457_200),
                ..bounds()
            };
            let id = session.insert_svg_picture(0, placement, svg).unwrap();
            session
                .set_picture_crop(
                    0,
                    id,
                    jade_pptx_reader::SourceRect {
                        left: 25_000,
                        ..Default::default()
                    },
                    true,
                )
                .unwrap();
            let bitmap_id = all_pictures.then(|| {
                session
                    .insert_picture(0, placement, &png(1000, 500))
                    .unwrap()
                    .id
            });
            let before = session.bytes().to_vec();
            let original = picture(&before, id);
            let package = OpcPackage::open(&before).unwrap();
            let original_fallback = package
                .part(original.image_part.as_deref().unwrap())
                .unwrap();
            let original_svg = package.part(original.svg_part.as_deref().unwrap()).unwrap();
            let count = session
                .compress_pictures((!all_pictures).then_some((0, id)), 72, delete_cropped)
                .unwrap();
            let current = picture(session.bytes(), id);
            assert_eq!(
                current.source_rect, original.source_rect,
                "SVG crop must remain shared by both representations"
            );
            assert_eq!(current.image_part, original.image_part);
            assert_eq!(current.svg_part, original.svg_part);
            let package = OpcPackage::open(session.bytes()).unwrap();
            assert_eq!(
                package
                    .part(current.image_part.as_deref().unwrap())
                    .unwrap(),
                original_fallback
            );
            assert_eq!(
                package.part(current.svg_part.as_deref().unwrap()).unwrap(),
                original_svg
            );
            if let Some(bitmap_id) = bitmap_id {
                assert!(count >= 1, "the bitmap must still be compressed");
                assert_ne!(
                    picture(session.bytes(), bitmap_id).image_part,
                    picture(&before, bitmap_id).image_part
                );
                let after = session.bytes().to_vec();
                assert!(session.undo());
                assert_eq!(session.bytes(), before);
                assert!(session.redo());
                assert_eq!(session.bytes(), after);
            } else {
                assert_eq!(count, 0);
                assert_eq!(session.bytes(), before);
            }
        }
    }
}

#[test]
fn svg_pictures_carry_a_png_stand_in_and_take_a_graphic_fill() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><path d="M2 2h20v20H2z" fill="#000000"/><circle cx="12" cy="12" r="4" fill="none" stroke="#000"/></svg>"##;
    let mut session = EditSession::open(DECK).unwrap();
    let id = session.insert_svg_picture(0, bounds(), svg).unwrap();
    let read = picture(session.bytes(), id);
    let svg_part = read.svg_part.clone().expect("the SVG");
    assert!(svg_part.ends_with(".svg"));
    assert!(read.image_part.as_deref().unwrap().ends_with(".png"));
    let types = OpcPackage::open(session.bytes())
        .unwrap()
        .required_part_str("[Content_Types].xml")
        .unwrap();
    assert!(types.contains("image/svg+xml"));
    session
        .format_picture(0, id, &PictureEdit::GraphicFill("#4472C4".into()))
        .unwrap();
    let read = picture(session.bytes(), id);
    let package = OpcPackage::open(session.bytes()).unwrap();
    let filled = package
        .required_part_str(read.svg_part.as_deref().unwrap())
        .unwrap();
    assert!(filled.contains("fill=\"#4472C4\""));
    // An unfilled outline stays unfilled.
    assert!(filled.contains("fill=\"none\""));
}

#[test]
fn an_svg_converts_to_freeforms_in_a_group() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="12" viewBox="0 0 24 12"><path d="M0 0h12v12H0z" fill="#FF0000"/><circle cx="18" cy="6" r="4" fill="none" stroke="#0000FF" stroke-width="2"/></svg>"##;
    let mut session = EditSession::open(DECK).unwrap();
    let id = session.insert_svg_picture(0, bounds(), svg).unwrap();
    let svg_part = picture(session.bytes(), id).svg_part.unwrap();
    let group = session.convert_svg_to_shapes(0, id).unwrap();
    let deck = read_deck(session.bytes()).unwrap();
    let found = deck.slides[0]
        .content
        .shapes
        .iter()
        .find_map(|item| match item {
            ShapeItem::Group(g) if g.id == Some(group) => Some(g.clone()),
            _ => None,
        });
    let group_read = found.expect("the group");
    assert_eq!(group_read.children.len(), 2);
    assert!(!deck.slides[0]
        .content
        .shapes
        .iter()
        .any(|item| matches!(item, ShapeItem::Picture(p) if p.id == Some(id))));
    let xml = slide(session.bytes());
    assert!(xml.contains("<a:custGeom>"));
    assert!(xml.contains("FF0000"));
    assert!(xml.contains("0000FF"));
    // The SVG and its stand-in are gone with the picture.
    let package = OpcPackage::open(session.bytes()).unwrap();
    assert!(package.part(&svg_part).is_none());
    // The group sits where the picture was.
    assert!(xml.contains("<a:off x=\"952500\" y=\"952500\"/>"));
    assert!(session.undo());
    assert!(picture(session.bytes(), id).svg_part.is_some());
}

#[test]
fn svg_conversion_preserves_rotation_and_both_flips_in_one_history_step() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="12"><path d="M0 0h24L0 12z" fill="#FF0000"/></svg>"##;
    for (rotation, flip_h, flip_v) in [
        (37 * 60_000, false, false),
        (0, true, false),
        (0, false, true),
        (-23 * 60_000, true, true),
    ] {
        let placement = Transform {
            rotation_60000ths: rotation,
            flip_h,
            flip_v,
            ..bounds()
        };
        let mut session = EditSession::open(DECK).unwrap();
        let id = session.insert_svg_picture(0, placement, svg).unwrap();
        let before = session.bytes().to_vec();
        let group_id = session.convert_svg_to_shapes(0, id).unwrap();
        let converted = session.bytes().to_vec();
        let deck = read_deck(&converted).unwrap();
        let group = deck.slides[0]
            .content
            .shapes
            .iter()
            .find_map(|item| match item {
                ShapeItem::Group(group) if group.id == Some(group_id) => Some(group),
                _ => None,
            })
            .unwrap();
        let transform = group.transform.unwrap();
        assert_eq!(transform.outer, placement);
        assert_eq!(transform.child_offset_emu, placement.offset_emu);
        assert_eq!(transform.child_extent_emu, placement.extent_emu);
        assert_eq!(group.children.len(), 1);
        assert!(session.undo());
        assert_eq!(session.bytes(), before);
        assert!(session.redo());
        assert_eq!(session.bytes(), converted);
    }
}
