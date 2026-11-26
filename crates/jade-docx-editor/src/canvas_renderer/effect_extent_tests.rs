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

/// A rotated picture's effect extent is added as plain insets: the
/// rotation itself is laid out as before (not at all for 30° inline), and
/// nothing is assumed about what Word writes for it.
#[test]
fn a_rotated_inline_picture_adds_its_effect_extent_as_insets() {
    let plain = inline_layout(&inline(40.0, 20.0, 30.0, [0.0; 4]));
    let upright = inline_layout(&inline(40.0, 20.0, 0.0, [0.0; 4]));
    let insets = inline_layout(&inline(40.0, 20.0, 30.0, [2.0, 3.0, 4.0, 5.0]));
    assert_near(
        plain.after.0,
        upright.after.0,
        "a 30° rotation still reserves the upright box",
    );
    assert_near(insets.after.0 - plain.after.0, 6.0, "l + r");
    assert_near(insets.next_line.1 - plain.next_line.1, 8.0, "t + b");
    assert_near(insets.picture.0 - plain.picture.0, 2.0, "inset by l");
    assert_near(insets.picture.2, 40.0, "the picture keeps its width");
    assert_near(insets.picture.3, 20.0, "the picture keeps its height");
}

/// Word for Mac lays a quarter-turned picture out in its swapped box, as
/// before; effect insets are added to that box, and negative edges count as
/// 0.
#[test]
fn a_mac_quarter_turn_adds_effect_insets_to_its_swapped_box() {
    use jade_docx_layout::line_metrics::{set_word_layout_platform, WordLayoutPlatform};

    set_word_layout_platform(WordLayoutPlatform::Mac);
    let without = inline_layout(&inline(40.0, 20.0, 90.0, [0.0; 4]));
    let negative = inline_layout(&inline(40.0, 20.0, 90.0, [-10.0, 10.0, -10.0, 10.0]));
    let shadowed = inline_layout(&inline(40.0, 20.0, 90.0, [1.0, 2.0, 3.0, 4.0]));
    set_word_layout_platform(WordLayoutPlatform::Windows);

    assert_near(
        without.after.0 - without.before.0,
        negative.after.0 - negative.before.0 + 0.0,
        "negative l and r count as 0",
    );
    assert_near(
        negative.next_line.1 - without.next_line.1,
        20.0,
        "the positive t and b are added",
    );
    assert_near(shadowed.after.0 - without.after.0, 4.0, "l + r");
    assert_near(shadowed.next_line.1 - without.next_line.1, 6.0, "t + b");
    // The swapped box is 20 wide: the 40 by 20 picture draws centred in it,
    // then inset by l.
    assert_near(
        shadowed.picture.0 - without.picture.0,
        1.0,
        "the picture is inset by l",
    );
    assert_near(shadowed.picture.2, without.picture.2, "it keeps its size");
}

/// Square-wrapped text clears a floating picture's effects as it clears its
/// rotated bounds; a malformed negative edge cannot pull text over it.
#[test]
fn text_wraps_around_a_floating_pictures_effect_extent() {
    let text_x = |rotation_deg: f64, effects: [f64; 4]| {
        let renderer = render(&format!(
            r#"<w:p>{}<w:r><w:t>Host</w:t></w:r></w:p>"#,
            floating(40.0, 20.0, rotation_deg, effects)
        ));
        glyph(&renderer.host.painter().inner().ops(), "H").0
    };
    let plain = text_x(0.0, [0.0; 4]);
    assert_near(
        text_x(0.0, [2.0, 0.0, 8.0, 0.0]) - plain,
        8.0,
        "text clears r",
    );
    assert_near(text_x(0.0, [0.0, 0.0, -30.0, 0.0]), plain, "r below 0 is 0");
    assert_near(
        text_x(90.0, [0.0, 0.0, -5.0, 0.0]),
        text_x(90.0, [0.0; 4]),
        "an edge without effects stays at the rotated bounds",
    );
}

/// A 200 by 100 picture at 30° whose effect extent already holds its
/// rotated box (as Word likely writes it) wraps at the rotated bounds, not
/// beyond them: each edge reaches the farther of the two, not their sum.
#[test]
fn a_rotated_floating_picture_counts_its_rotation_once() {
    let (width, height) = (200.0_f64, 100.0_f64);
    let radians = 30.0_f64.to_radians();
    let grow_x = (width * radians.cos() + height * radians.sin() - width) / 2.0;
    let grow_y = (width * radians.sin() + height * radians.cos() - height) / 2.0;
    let text_x = |effects: [f64; 4]| {
        let renderer = render(&format!(
            r#"<w:p>{}<w:r><w:t>Host</w:t></w:r></w:p>"#,
            floating(width, height, 30.0, effects)
        ));
        glyph(&renderer.host.painter().inner().ops(), "H").0
    };
    let rotated = text_x([0.0; 4]);
    assert_near(
        text_x([grow_x, grow_y, grow_x, grow_y]),
        rotated,
        "the extent's rotation is not added to the rotated bounds",
    );
    assert_near(
        text_x([grow_x, grow_y, grow_x + 3.0, grow_y]) - rotated,
        3.0,
        "an effect beyond the rotated box still keeps text clear",
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

/// An inline chart (`c:chart` in a graphic frame) is laid out as a picture
/// is, and reserves its effect extent the same way.
#[test]
fn an_inline_chart_reserves_its_effect_extent() {
    let chart = |effects: [f64; 4]| {
        format!(
            r#"<w:r><w:drawing><wp:inline {NAMESPACES} distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx}" cy="{cy}"/>{effect}<wp:docPr id="1" name="Chart 1"/><wp:cNvGraphicFramePr/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:id="rId20"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#,
            cx = emu(40.0),
            cy = emu(30.0),
            effect = effect(effects),
        )
    };
    let plain = inline_layout(&chart([0.0; 4]));
    let framed = inline_layout(&chart([1.0, 2.0, 3.0, 4.0]));
    assert_near(framed.after.0 - plain.after.0, 4.0, "l + r");
    assert_near(framed.next_line.1 - plain.next_line.1, 6.0, "t + b");
}
