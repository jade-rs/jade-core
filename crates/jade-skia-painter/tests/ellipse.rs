use jade_painter::Painter;
use jade_skia_painter::SkiaPainter;
use std::f64::consts::{FRAC_PI_2, PI};

const RED: (u8, u8, u8, u8) = (255, 0, 0, 255);
const WHITE: (u8, u8, u8, u8) = (255, 255, 255, 255);

fn painter() -> SkiaPainter {
    let painter = SkiaPainter::new(120, 120).unwrap();
    painter.set_fill_style_str("#ffffff");
    painter.fill_rect(0.0, 0.0, 120.0, 120.0);
    painter.set_fill_style_str("#ff0000");
    painter.begin_path();
    painter
}

#[test]
fn a_full_sweep_fills_the_whole_circle() {
    for anticlockwise in [false, true] {
        let painter = painter();
        let end = if anticlockwise { -2.0 * PI } else { 2.0 * PI };
        painter
            .ellipse_with_anticlockwise(60.0, 60.0, 40.0, 40.0, 0.0, 0.0, end, anticlockwise)
            .unwrap();
        painter.fill();
        for (x, y) in [(60, 60), (25, 60), (95, 60), (60, 25), (60, 95)] {
            assert_eq!(painter.pixel(x, y), Some(RED), "({x}, {y}) {anticlockwise}");
        }
        assert_eq!(painter.pixel(5, 5), Some(WHITE));
    }
}

#[test]
fn a_rotated_ellipse_is_turned_about_its_centre() {
    let painter = painter();
    painter
        .ellipse(60.0, 60.0, 50.0, 10.0, FRAC_PI_2, 0.0, 2.0 * PI)
        .unwrap();
    painter.fill();
    assert_eq!(painter.pixel(60, 60), Some(RED));
    assert_eq!(painter.pixel(60, 15), Some(RED));
    assert_eq!(painter.pixel(60, 105), Some(RED));
    assert_eq!(painter.pixel(15, 60), Some(WHITE));
    assert_eq!(painter.pixel(105, 60), Some(WHITE));
}

#[test]
fn a_rotated_arc_joins_the_current_point_with_a_line() {
    let painter = painter();
    painter.move_to(60.0, 60.0);
    // Rotated a quarter turn, the arc from 0 to PI runs from (60, 100) to
    // (60, 20) through (20, 60): with the line from the centre this fills
    // the left half.
    painter
        .ellipse(60.0, 60.0, 40.0, 40.0, FRAC_PI_2, 0.0, PI)
        .unwrap();
    painter.fill();
    assert_eq!(painter.pixel(35, 60), Some(RED));
    assert_eq!(painter.pixel(85, 60), Some(WHITE));
}

#[test]
fn a_partial_anticlockwise_arc_keeps_its_direction() {
    let painter = painter();
    painter
        .ellipse_with_anticlockwise(60.0, 60.0, 40.0, 40.0, 0.0, 0.0, PI, true)
        .unwrap();
    painter.fill();
    assert_eq!(painter.pixel(60, 35), Some(RED));
    assert_eq!(painter.pixel(60, 85), Some(WHITE));
}
