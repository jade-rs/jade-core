//! `<a:buBlip>`: a picture as the bullet, drawn by the host in the bullet's
//! square, with the round bullet where the picture cannot be drawn.

use jade_painter::recording::{Op, RecordingPainter};
use jade_pptx_layout::{layout_slide, HeuristicMeasure, PlacedShape};
use jade_pptx_reader::{parse_slide_xml, Bullet, ShapeItem, SlideSize, ThemeColors};
use jade_pptx_render::{render_slide_with, RenderOptions};

const SLIDE: &str = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree>
<p:sp><p:nvSpPr><p:cNvPr id="2" name="Body"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
<p:spPr><a:xfrm><a:off x="914400" y="914400"/><a:ext cx="4572000" cy="914400"/></a:xfrm><a:prstGeom prst="rect"/></p:spPr>
<p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:pPr marL="342900" indent="-342900"><a:buBlip><a:blip r:embed="rId3"/></a:buBlip></a:pPr><a:r><a:rPr lang="en-US" sz="2400"/><a:t>Arrow</a:t></a:r></a:p></p:txBody></p:sp>
</p:spTree></p:cSld></p:sld>"#;

fn laid_out(image_part: Option<&str>) -> jade_pptx_layout::SlideLayout {
    let mut content = parse_slide_xml(SLIDE).unwrap();
    let ShapeItem::Shape(shape) = &mut content.shapes[0] else {
        panic!("a shape");
    };
    let paragraph = &mut shape.text.as_mut().unwrap().paragraphs[0];
    // What read_deck resolves through the slide's relationships.
    match &mut paragraph.bullet {
        Some(Bullet::Picture { rel_id, image_part: part }) => {
            assert_eq!(rel_id, "rId3");
            *part = image_part.map(str::to_string);
        }
        other => panic!("a picture bullet, not {other:?}"),
    }
    layout_slide(
        SlideSize {
            width_emu: 12_192_000,
            height_emu: 6_858_000,
        },
        &content,
        &ThemeColors::default(),
        &HeuristicMeasure,
    )
}

#[test]
fn the_host_draws_the_bullet_picture_in_a_square_on_the_baseline() {
    let layout = laid_out(Some("ppt/media/image8.gif"));
    let run = &layout.shapes[0].lines[0].runs[0];
    assert_eq!(run.bullet_image.as_deref(), Some("ppt/media/image8.gif"));
    let painter = RecordingPainter::new();
    let mut asked: Vec<PlacedShape> = Vec::new();
    render_slide_with(&painter, &layout, &RenderOptions::default(), &mut |shape| {
        asked.push(shape.clone());
        true
    })
    .unwrap();
    assert_eq!(asked.len(), 1);
    let bullet = &asked[0];
    assert_eq!(bullet.image_part.as_deref(), Some("ppt/media/image8.gif"));
    // Square, as wide as the run, resting on the baseline at the run's x.
    let shape = &layout.shapes[0];
    assert!((bullet.rect.width - bullet.rect.height).abs() < 1e-9);
    assert!((bullet.rect.width - run.width).abs() < 1e-9);
    assert!((bullet.rect.x - (shape.rect.x + run.x)).abs() < 1e-9);
    assert!((bullet.rect.y + bullet.rect.height - (shape.rect.y + run.baseline_y)).abs() < 1e-9);
    // The glyph stands down when the picture drew.
    assert!(!painter
        .ops()
        .iter()
        .any(|op| matches!(op, Op::FillText(text, ..) if text == "\u{2022}")));
}

#[test]
fn without_the_picture_the_round_bullet_draws() {
    let layout = laid_out(Some("ppt/media/missing.emf"));
    let painter = RecordingPainter::new();
    render_slide_with(&painter, &layout, &RenderOptions::default(), &mut |_| false).unwrap();
    assert!(painter
        .ops()
        .iter()
        .any(|op| matches!(op, Op::FillText(text, ..) if text == "\u{2022}")));
    // Unresolved: no picture to ask for.
    let layout = laid_out(None);
    assert_eq!(layout.shapes[0].lines[0].runs[0].bullet_image, None);
}
