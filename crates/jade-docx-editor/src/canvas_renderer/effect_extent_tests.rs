//! `wp:effectExtent`: the space DrawingML reserves beyond a picture's
//! `wp:extent` "to compensate for any drawing effects applied" (ECMA-376
//! Part 1 §20.4.2.6) — shadows, outlines, glow and the bounding box of a
//! rotation. An inline picture reserves its extent grown by it and draws
//! inset by its left and top edges; text wraps around a floating picture's
//! enlarged box.

use super::recording_host::recording_renderer;
use super::section_tests::build_docx_with_body;
use super::*;
use crate::word_document::WordDocument;
use jade_painter::recording::Op;

/// 9525 EMU per CSS pixel.
const EMU_PER_PX: f64 = 9525.0;
const PAGE: &str = r#"<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr>"#;
const NAMESPACES: &str =
    r#"xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing""#;

fn emu(px: f64) -> i64 {
    (px * EMU_PER_PX).round() as i64
}

/// A `width` by `height` px picture graphic rotated by `rotation_deg`.
fn graphic(width: f64, height: f64, rotation_deg: f64) -> String {
    let (cx, cy) = (emu(width), emu(height));
    let rot = (rotation_deg * 60_000.0).round() as i64;
    format!(
        r#"<wp:docPr id="1" name="Picture 1"/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:nvPicPr><pic:cNvPr id="1" name="Picture 1"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rId10" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm rot="{rot}"><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic>"#
    )
}

/// `wp:effectExtent` of `(left, top, right, bottom)` px.
fn effect([l, t, r, b]: [f64; 4]) -> String {
    format!(
        r#"<wp:effectExtent l="{}" t="{}" r="{}" b="{}"/>"#,
        emu(l),
        emu(t),
        emu(r),
        emu(b)
    )
}

/// An inline `width` by `height` px picture with `effects`.
fn inline(width: f64, height: f64, rotation_deg: f64, effects: [f64; 4]) -> String {
    format!(
        r#"<w:r><w:drawing><wp:inline {NAMESPACES} distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{}" cy="{}"/>{}{}</wp:inline></w:drawing></w:r>"#,
        emu(width),
        emu(height),
        effect(effects),
        graphic(width, height, rotation_deg),
    )
}

/// A square-wrapped `width` by `height` px picture at the column's top left.
fn floating(width: f64, height: f64, rotation_deg: f64, effects: [f64; 4]) -> String {
    format!(
        r#"<w:r><w:drawing><wp:anchor {NAMESPACES} distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="1" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="{}" cy="{}"/>{}<wp:wrapSquare wrapText="bothSides"/>{}</wp:anchor></w:drawing></w:r>"#,
        emu(width),
        emu(height),
        effect(effects),
        graphic(width, height, rotation_deg),
    )
}

/// Renders `body_xml` with every picture given image data.
fn render(body_xml: &str) -> CanvasRenderer<jade_painter::recording::RecordingPainter> {
    let data = build_docx_with_body(&format!("{body_xml}{PAGE}"));
    let word_doc =
        futures::executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
    let mut document = (**word_doc.document.as_ref().expect("document")).clone();
    for element in &mut document.body.as_mut().expect("body").elements {
        if let BodyElement::Paragraph(paragraph) = element {
            for run in &mut paragraph.runs {
                if let Some(image) = run.image.as_mut() {
                    image.data_uri = Some("data:image/png;base64,eA==".into());
                }
            }
        }
    }
    let mut renderer = recording_renderer();
    renderer
        .render(&document, &Options::default())
        .expect("render");
    renderer
}

/// `(x, y)` of the first glyph drawn as `text`.
fn glyph(ops: &[Op], text: &str) -> (f64, f64) {
    ops.iter()
        .find_map(|op| match op {
            Op::FillText(value, x, y) if value == text => Some((*x, *y)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no {text} in {ops:?}"))
}

/// Where the glyphs around an inline picture land, and the picture's
/// painted rectangle.
struct InlineLayout {
    before: (f64, f64),
    after: (f64, f64),
    next_line: (f64, f64),
    picture: (f64, f64, f64, f64),
}

/// "A", the picture, "B", then "C" in the next paragraph.
fn inline_layout(picture: &str) -> InlineLayout {
    let renderer = render(&format!(
        r#"<w:p><w:r><w:t>A</w:t></w:r>{picture}<w:r><w:t>B</w:t></w:r></w:p><w:p><w:r><w:t>C</w:t></w:r></w:p>"#
    ));
    let ops = renderer.host.painter().inner().ops();
    let regions = renderer.image_debug_regions();
    assert_eq!(regions.len(), 1, "{regions:?}");
    let region = &regions[0];
    InlineLayout {
        before: glyph(&ops, "A"),
        after: glyph(&ops, "B"),
        next_line: glyph(&ops, "C"),
        picture: (region.x, region.y, region.width, region.height),
    }
}

fn assert_near(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < 0.05,
        "{what}: expected {expected}, got {actual}"
    );
}

/// An outer shadow reserves `l + r` more in the line and `t + b` more above
/// it; the picture is drawn at its extent, inset by `l` and `t`.
#[test]
fn an_inline_picture_reserves_its_effect_extent() {
    let (l, t, r, b) = (3.0, 2.0, 7.0, 9.0);
    let plain = inline_layout(&inline(40.0, 30.0, 0.0, [0.0; 4]));
    let shadowed = inline_layout(&inline(40.0, 30.0, 0.0, [l, t, r, b]));

    assert_near(shadowed.before.0, plain.before.0, "text before keeps its x");
    assert_near(
        shadowed.after.0 - plain.after.0,
        l + r,
        "the next glyph moves by l + r",
    );
    assert_near(
        (shadowed.next_line.1 - shadowed.before.1) - (plain.next_line.1 - plain.before.1),
        0.0,
        "the line's descent below the baseline is unchanged",
    );
    assert_near(
        shadowed.next_line.1 - plain.next_line.1,
        t + b,
        "the line grows by t + b",
    );
    let (x, y, width, height) = shadowed.picture;
    let (plain_x, plain_y, ..) = plain.picture;
    assert_near(x - plain_x, l, "the picture is inset by l");
    // The picture's box sits on the baseline: its extent ends b above it.
    assert_near(
        (shadowed.before.1 - y) - (plain.before.1 - plain_y),
        b,
        "the extent ends b above the baseline",
    );
    assert_near(width, 40.0, "the picture keeps its width");
    assert_near(height, 30.0, "the picture keeps its height");
}

/// A picture rotated 30° as Word writes it: `wp:effectExtent` holds the
/// rotated bounding box, so the line reserves that box and the picture's
/// centre stays in its middle.
#[test]
fn an_inline_picture_reserves_the_rotated_box_word_writes() {
    let (width, height) = (40.0_f64, 20.0_f64);
    let radians = 30.0_f64.to_radians();
    let rotated_width = width * radians.cos() + height * radians.sin();
    let rotated_height = width * radians.sin() + height * radians.cos();
    let (grow_x, grow_y) = (
        (rotated_width - width) / 2.0,
        (rotated_height - height) / 2.0,
    );
    let plain = inline_layout(&inline(width, height, 30.0, [0.0; 4]));
    let rotated = inline_layout(&inline(
        width,
        height,
        30.0,
        [grow_x, grow_y, grow_x, grow_y],
    ));
    assert_near(
        rotated.after.0 - plain.after.0,
        2.0 * grow_x,
        "the line reserves the rotated width",
    );
    assert_near(
        rotated.next_line.1 - plain.next_line.1,
        2.0 * grow_y,
        "the line reserves the rotated height",
    );
    assert_near(
        rotated.picture.0 - plain.picture.0,
        grow_x,
        "the extent is centred in the rotated box",
    );
}

/// Word for Mac lays a quarter-turned picture out in its rotated box. When
/// the file's `wp:effectExtent` already holds that box, it is not added
/// again; when it does not, the picture still gets it.
#[test]
fn a_mac_quarter_turn_counts_the_rotated_box_once() {
    use jade_docx_layout::line_metrics::{set_word_layout_platform, WordLayoutPlatform};

    set_word_layout_platform(WordLayoutPlatform::Mac);
    let (width, height) = (40.0, 20.0);
    let grow = (width - height) / 2.0;
    let without = inline_layout(&inline(width, height, 90.0, [0.0; 4]));
    let with = inline_layout(&inline(width, height, 90.0, [-grow, grow, -grow, grow]));
    let shadowed = inline_layout(&inline(
        width,
        height,
        90.0,
        [-grow + 1.0, grow + 2.0, -grow + 3.0, grow + 4.0],
    ));
    set_word_layout_platform(WordLayoutPlatform::Windows);

    assert_near(
        without.after.0 - without.before.0,
        with.after.0 - with.before.0,
        "the box is the same width either way",
    );
    assert_near(
        without.next_line.1,
        with.next_line.1,
        "and the same height either way",
    );
    assert_eq!(without.picture, with.picture);
    assert_near(
        shadowed.after.0 - with.after.0,
        4.0,
        "a shadow on top of the rotation still adds its l + r",
    );
}

/// Square-wrapped text clears a floating picture's effects as it clears its
/// extent.
#[test]
fn text_wraps_around_a_floating_pictures_effect_extent() {
    let text_x = |effects: [f64; 4]| {
        let renderer = render(&format!(
            r#"<w:p>{}<w:r><w:t>Host</w:t></w:r></w:p>"#,
            floating(40.0, 30.0, 0.0, effects)
        ));
        glyph(&renderer.host.painter().inner().ops(), "H").0
    };
    let plain = text_x([0.0; 4]);
    assert_near(text_x([2.0, 0.0, 8.0, 0.0]) - plain, 8.0, "text clears r");
    // Rotated 90°: Word writes the rotated box, which must not be added to
    // the rotated bounds the renderer already wraps around.
    let rotated = |effects: [f64; 4]| {
        let renderer = render(&format!(
            r#"<w:p>{}<w:r><w:t>Host</w:t></w:r></w:p>"#,
            floating(40.0, 20.0, 90.0, effects)
        ));
        glyph(&renderer.host.painter().inner().ops(), "H").0
    };
    assert_near(
        rotated([-10.0, 10.0, -10.0, 10.0]),
        rotated([0.0; 4]),
        "the rotated box is counted once",
    );
}

/// The committed probes (`tests/word-oracle/effect-extent-probes`), laid
/// out with and without their effect extents: the line that holds the
/// picture is `t + b` taller, and the text after it `l + r` further right.
#[test]
fn the_effect_extent_probes_reserve_their_effects() {
    for (name, data) in [
        (
            "shadow",
            &include_bytes!(
                "../../../../tests/word-oracle/effect-extent-probes/inline-shadow-effect-extent.docx"
            )[..],
        ),
        (
            "rotation",
            &include_bytes!(
                "../../../../tests/word-oracle/effect-extent-probes/inline-rotation-30-effect-extent.docx"
            )[..],
        ),
    ] {
        let word_doc = futures::executor::block_on(WordDocument::load(data, &Options::default()))
            .expect("load probe");
        let document = (**word_doc.document.as_ref().expect("document")).clone();
        let mut without = document.clone();
        let mut extent = None;
        for element in &mut without.body.as_mut().expect("body").elements {
            if let BodyElement::Paragraph(paragraph) = element {
                for run in &mut paragraph.runs {
                    if let Some(image) = run.image.as_mut() {
                        extent = image.layout.options_mut().effect_extent.take();
                    }
                }
            }
        }
        let extent = extent.unwrap_or_else(|| panic!("{name}: no effect extent"));
        let ops = |document: &Document| {
            let mut renderer = recording_renderer();
            renderer.render(document, &Options::default()).expect("render");
            renderer.host.painter().inner().ops()
        };
        let (with_ops, without_ops) = (ops(&document), ops(&without));
        // "g" first appears after the picture, "A" first in the next line.
        let (right, after) = (glyph(&with_ops, "g"), glyph(&with_ops, "A"));
        let (plain_right, plain_after) = (glyph(&without_ops, "g"), glyph(&without_ops, "A"));
        assert_near(
            right.0 - plain_right.0,
            extent.left + extent.right,
            &format!("{name}: text after the picture"),
        );
        assert_near(
            after.1 - plain_after.1,
            extent.top + extent.bottom,
            &format!("{name}: the next line"),
        );
    }
}
