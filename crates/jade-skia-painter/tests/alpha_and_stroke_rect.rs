//! Covers the two `Painter` operations Skia has to implement itself rather than
//! forward: canvas expresses both in terms of state the Skia API has no direct
//! equivalent for.

use jade_painter::Painter;
use jade_skia_painter::SkiaPainter;

fn white_canvas(width: i32, height: i32) -> SkiaPainter {
    let painter = SkiaPainter::new(width, height).expect("create raster surface");
    painter.set_fill_style_str("#ffffff");
    painter.fill_rect(0.0, 0.0, width as f64, height as f64);
    painter
}

/// Skia has no canvas-style global alpha, so it is folded into every paint's
/// colour. Half alpha over white must land halfway, not at either end.
#[test]
fn global_alpha_scales_what_is_painted() {
    let painter = white_canvas(20, 20);
    painter.set_fill_style_str("#000000");
    painter.set_global_alpha(0.5);
    painter.fill_rect(5.0, 5.0, 10.0, 10.0);

    let (r, g, b, _) = painter.pixel(10, 10).expect("read pixel");
    for channel in [r, g, b] {
        assert!(
            (100..=155).contains(&channel),
            "expected a mid grey from 50% black on white, got {r},{g},{b}"
        );
    }
}

/// Canvas keeps global alpha in the saved state. A backend that stored it
/// outside the state stack would leak a faded watermark into everything drawn
/// after it.
#[test]
fn global_alpha_is_restored_with_the_state() {
    let painter = white_canvas(20, 20);
    painter.save();
    painter.set_global_alpha(0.1);
    painter.restore();

    painter.set_fill_style_str("#000000");
    painter.fill_rect(5.0, 5.0, 10.0, 10.0);

    let (r, _, _, _) = painter.pixel(10, 10).expect("read pixel");
    assert!(
        r < 20,
        "expected full-strength black after restore, got {r}"
    );
}

/// `strokeRect` is not `rect` + `stroke`: it paints immediately and leaves the
/// path being built untouched. Implemented as the latter, the diagonal below
/// would be stroked too.
#[test]
fn stroke_rect_paints_immediately_and_leaves_the_path_alone() {
    let painter = white_canvas(40, 40);
    painter.set_stroke_style_str("#000000");
    painter.set_line_width(2.0);

    painter.begin_path();
    painter.move_to(2.0, 2.0);
    painter.line_to(2.0, 38.0);
    painter.stroke_rect(20.0, 10.0, 12.0, 12.0);
    // Deliberately no `stroke()`.

    let (rect_edge, _, _, _) = painter.pixel(20, 16).expect("read pixel");
    assert!(rect_edge < 128, "rectangle should already be painted");

    let (unstroked_line, _, _, _) = painter.pixel(2, 20).expect("read pixel");
    assert!(
        unstroked_line > 200,
        "the path was never stroked, so the line must be absent; got {unstroked_line}"
    );
}

#[test]
fn compositor_readback_preserves_channel_order_and_owns_its_pixels() {
    let painter = SkiaPainter::new_scaled(1.5, 1.0, 2.0).expect("create scaled raster");
    painter.set_fill_style_str("#ff4000");
    painter.set_global_alpha(0.5);
    painter.fill_rect(0.0, 0.0, 1.0, 1.0);

    let generation = painter.raster_generation();
    let (before, width, height) = painter.read_bgra_premultiplied().unwrap();
    assert_eq!((width, height), (3, 2));
    assert_eq!(
        before,
        [127, 159, 255, 255, 127, 159, 255, 255, 255, 255, 255, 255].repeat(2)
    );
    assert_eq!(painter.raster_generation(), generation);

    painter.set_global_alpha(1.0);
    painter.set_fill_style_str("#0000ff");
    painter.fill_rect(1.0, 0.0, 0.5, 1.0);
    let (after, _, _) = painter.read_bgra_premultiplied().unwrap();
    assert_eq!(&after[8..12], &[255, 0, 0, 255]);
    assert_eq!(&before[8..12], &[255, 255, 255, 255]);
}
