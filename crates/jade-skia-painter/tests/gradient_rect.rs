use jade_painter::{ImageRect, Painter};
use jade_skia_painter::SkiaPainter;

#[test]
fn a_gradient_uses_global_alpha_and_preserves_the_fill_style() {
    let painter = SkiaPainter::new(120, 40).unwrap();
    painter.set_fill_style_str("#ffffff");
    painter.fill_rect(0.0, 0.0, 120.0, 40.0);
    painter.set_fill_style_str("rgba(0, 255, 0, 0.2)");
    painter.set_global_alpha(0.5);
    painter
        .fill_horizontal_gradient_rect(
            ImageRect {
                x: 10.0,
                y: 0.0,
                width: 100.0,
                height: 10.0,
            },
            [0, 0, 255, 255],
            [255; 4],
        )
        .unwrap();
    let (r, g, b, a) = painter.pixel(11, 5).unwrap();
    assert!((128..=134).contains(&r));
    assert_eq!(r, g);
    assert_eq!((b, a), (255, 255));
    let (r, g, b, a) = painter.pixel(60, 5).unwrap();
    assert!((190..=194).contains(&r));
    assert_eq!(r, g);
    assert_eq!((b, a), (255, 255));
    painter.fill_rect(10.0, 20.0, 100.0, 10.0);
    let (r, g, b, a) = painter.pixel(60, 25).unwrap();
    assert!((228..=231).contains(&r));
    assert_eq!((g, b, a), (255, r, 255));
}

#[test]
fn a_gradient_obeys_transform_and_clip_without_disturbing_the_current_path() {
    let painter = SkiaPainter::new(120, 40).unwrap();
    painter.set_fill_style_str("#ffffff");
    painter.fill_rect(0.0, 0.0, 120.0, 40.0);
    painter.save();
    painter.translate(10.0, 5.0).unwrap();
    painter.begin_path();
    painter.rect(0.0, 0.0, 50.0, 10.0);
    painter.clip();
    painter
        .fill_horizontal_gradient_rect(
            ImageRect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 10.0,
            },
            [255, 0, 0, 255],
            [0, 255, 0, 255],
        )
        .unwrap();
    painter.set_stroke_style_str("#000000");
    painter.set_line_width(2.0);
    painter.stroke();
    painter.restore();
    let (r, g, b, a) = painter.pixel(15, 9).unwrap();
    assert!(r > 230 && g < 25);
    assert_eq!((b, a), (0, 255));
    assert_eq!(painter.pixel(65, 9), Some((255, 255, 255, 255)));
    assert_eq!(
        painter.pixel(30, 5),
        Some((0, 0, 0, 255)),
        "stroke still uses the rectangle path"
    );
}
