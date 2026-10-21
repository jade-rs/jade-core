//! Asserts the painter operations a laid-out slide produces, against the
//! recording backend — the same arrangement the other renderers use, so the
//! browser canvas and Skia draw a deck identically.

use jade_painter::recording::{Op, RecordingPainter};
use jade_pptx_layout::{
    PlacedGeometry, PlacedLine, PlacedOutline, PlacedRun, PlacedShape, Rect, SlideLayout,
};
use jade_pptx_render::{render_slide, RenderOptions};

fn bare_shape(geometry: PlacedGeometry) -> PlacedShape {
    PlacedShape {
        text_region: None,
        local_rect: None,
        group_transforms: Vec::new(),
        group_ids: Vec::new(),
        name: "s".to_string(),
        rect: Rect {
            x: 96.0,
            y: 48.0,
            width: 192.0,
            height: 96.0,
        },
        rotation_rad: 0.0,
        flip_h: false,
        flip_v: false,
        geometry,
        fill: None,
        outline: None,
        image_part: None,
        source_rect: None,
        lines: Vec::new(),
        origin: jade_pptx_layout::PlacedOrigin::Slide,
        source_id: None,
        text_rotation_deg: 0.0,
        gradient: None,
        shadow: None,
        effects: Default::default(),
        image_alpha: None,
        pattern: None,
    }
}

fn layout_with(shapes: Vec<PlacedShape>) -> SlideLayout {
    SlideLayout {
        groups: Vec::new(),
        width_px: 960.0,
        height_px: 720.0,
        background: None,
        shapes,
        background_gradient: None,
    }
}

#[test]
fn the_background_is_painted_first() {
    let painter = RecordingPainter::new();
    render_slide(
        &painter,
        &layout_with(Vec::new()),
        &RenderOptions::default(),
    )
    .expect("render");
    let ops = painter.ops();
    assert_eq!(ops[0], Op::SetFillStyle("#FFFFFF".to_string()));
    assert_eq!(ops[1], Op::FillRect(0.0, 0.0, 960.0, 720.0));
}

#[test]
fn nested_group_maps_bracket_local_geometry_text_and_images_with_balanced_state() {
    let deck =
        jade_pptx_reader::read_deck(include_bytes!("../../../public/fixtures/grouped-deck.pptx"))
            .unwrap();
    let layout =
        jade_pptx_layout::layout_deck_slide(&deck, 0, &jade_pptx_layout::HeuristicMeasure).unwrap();
    let painter = RecordingPainter::new();
    render_slide(&painter, &layout, &RenderOptions::default()).unwrap();
    let ops = painter.ops();
    let start = ops
        .iter()
        .position(|op| *op == Op::Translate(380.0, 260.0))
        .unwrap();
    assert_eq!(ops[start - 1], Op::Save);
    assert!(matches!(ops[start+1],Op::Rotate(r) if (r-std::f64::consts::PI/6.0).abs()<1e-12));
    assert_eq!(ops[start + 2], Op::Scale(1.0, -1.0));
    assert_eq!(ops[start + 3], Op::Translate(-60.0, -40.0));
    assert_eq!(
        ops.iter().filter(|op| **op == Op::Save).count(),
        ops.iter().filter(|op| **op == Op::Restore).count()
    );
}

#[test]
fn a_filled_rect_shape_becomes_one_fill_rect() {
    let mut shape = bare_shape(PlacedGeometry::Rect);
    shape.fill = Some("#FF0000".to_string());
    let painter = RecordingPainter::new();
    render_slide(
        &painter,
        &layout_with(vec![shape]),
        &RenderOptions::default(),
    )
    .expect("render");
    let ops = painter.ops();
    let fill_index = ops
        .iter()
        .position(|op| *op == Op::SetFillStyle("#FF0000".to_string()))
        .expect("fill colour set");
    assert_eq!(ops[fill_index + 1], Op::FillRect(96.0, 48.0, 192.0, 96.0));
}

#[test]
fn an_outline_strokes_after_the_fill() {
    let mut shape = bare_shape(PlacedGeometry::Rect);
    shape.fill = Some("#FF0000".to_string());
    shape.outline = Some(PlacedOutline {
        color: "#4472C4".to_string(),
        width_px: 2.0,
        dash_px: Vec::new(),
        head_end: None,
        tail_end: None,
    });
    let painter = RecordingPainter::new();
    render_slide(
        &painter,
        &layout_with(vec![shape]),
        &RenderOptions::default(),
    )
    .expect("render");
    let ops = painter.ops();
    let stroke_style = ops
        .iter()
        .position(|op| *op == Op::SetStrokeStyle("#4472C4".to_string()))
        .expect("stroke colour set");
    assert!(ops.contains(&Op::SetLineWidth(2.0)));
    assert!(ops[stroke_style..].contains(&Op::StrokeRect(96.0, 48.0, 192.0, 96.0)));
    let fill_index = ops
        .iter()
        .position(|op| matches!(op, Op::FillRect(x, ..) if *x == 96.0))
        .expect("fill first");
    assert!(fill_index < stroke_style);
}

#[test]
fn an_ellipse_draws_as_an_ellipse_path() {
    let mut shape = bare_shape(PlacedGeometry::Ellipse);
    shape.fill = Some("#00FF00".to_string());
    let painter = RecordingPainter::new();
    render_slide(
        &painter,
        &layout_with(vec![shape]),
        &RenderOptions::default(),
    )
    .expect("render");
    let ops = painter.ops();
    assert!(ops.contains(&Op::BeginPath));
    // Centred in the rect, radii half the extents, a full turn.
    assert!(ops.iter().any(|op| matches!(
        op,
        Op::Ellipse(x, y, rx, ry, _, _, _)
            if *x == 192.0 && *y == 96.0 && *rx == 96.0 && *ry == 48.0
    )));
    assert!(ops.contains(&Op::Fill));
    assert!(!ops
        .iter()
        .any(|op| matches!(op, Op::FillRect(x, ..) if *x == 96.0)));
}

#[test]
fn a_round_rect_fills_a_path_not_a_rect() {
    let mut shape = bare_shape(PlacedGeometry::RoundRect { adjust: 0.16667 });
    shape.fill = Some("#0000FF".to_string());
    let painter = RecordingPainter::new();
    render_slide(
        &painter,
        &layout_with(vec![shape]),
        &RenderOptions::default(),
    )
    .expect("render");
    let ops = painter.ops();
    assert!(ops.contains(&Op::BeginPath));
    assert!(ops.contains(&Op::Fill));
    assert!(!ops
        .iter()
        .any(|op| matches!(op, Op::FillRect(x, ..) if *x == 96.0)));
}

#[test]
fn round_rect_corners_follow_four_circular_arcs_for_landscape_portrait_and_capsules() {
    use std::f64::consts::{FRAC_PI_2, PI};
    for (width, height, adjust) in [
        (320.0, 160.0, 0.16667),
        (160.0, 320.0, 0.25),
        (200.0, 200.0, 0.5),
    ] {
        let mut shape = bare_shape(PlacedGeometry::RoundRect { adjust });
        shape.rect.width = width;
        shape.rect.height = height;
        shape.fill = Some("#12AB34".into());
        let radius = width.min(height) * adjust;
        let painter = RecordingPainter::new();
        render_slide(
            &painter,
            &layout_with(vec![shape]),
            &RenderOptions::default(),
        )
        .unwrap();
        let arcs: Vec<_> = painter
            .ops()
            .into_iter()
            .filter(|op| matches!(op, Op::Ellipse(..)))
            .collect();
        assert_eq!(
            arcs,
            vec![
                Op::Ellipse(
                    96.0 + width - radius,
                    48.0 + radius,
                    radius,
                    radius,
                    0.0,
                    -FRAC_PI_2,
                    0.0
                ),
                Op::Ellipse(
                    96.0 + width - radius,
                    48.0 + height - radius,
                    radius,
                    radius,
                    0.0,
                    0.0,
                    FRAC_PI_2
                ),
                Op::Ellipse(
                    96.0 + radius,
                    48.0 + height - radius,
                    radius,
                    radius,
                    0.0,
                    FRAC_PI_2,
                    PI
                ),
                Op::Ellipse(
                    96.0 + radius,
                    48.0 + radius,
                    radius,
                    radius,
                    0.0,
                    PI,
                    3.0 * FRAC_PI_2
                ),
            ]
        );
    }
}

#[test]
fn zero_adjustment_round_rect_uses_a_closed_rectangle_without_degenerate_arcs() {
    let mut shape = bare_shape(PlacedGeometry::RoundRect { adjust: 0.0 });
    shape.fill = Some("#12AB34".into());
    let painter = RecordingPainter::new();
    render_slide(
        &painter,
        &layout_with(vec![shape]),
        &RenderOptions::default(),
    )
    .unwrap();
    assert!(painter.ops().contains(&Op::Rect(96.0, 48.0, 192.0, 96.0)));
    assert!(!painter.ops().iter().any(|op| matches!(op, Op::Ellipse(..))));
}

#[test]
fn text_runs_draw_at_their_shape_relative_positions() {
    let mut shape = bare_shape(PlacedGeometry::Rect);
    shape.lines = vec![PlacedLine {
        runs: vec![PlacedRun {
            text: "Hello".to_string(),
            x: 9.6,
            baseline_y: 30.0,
            size_pt: 32.0,
            bold: true,
            italic: true,
            underline: false,
            color: "#112233".to_string(),
            font: Some("Meiryo".to_string()),
            strike: false,
            east_asian_font: None,
            hyperlink: None,
            width: 0.0,
            source_offsets: Vec::new(),
            highlight: None,
            letter_spacing_px: 0.0,
            text_outline: None,
            text_shadow: None,
            text_glow: None,
            bullet_image: None,
            underline_style: None,
            double_strike: false,
        }],
        ..PlacedLine::default()
    }];
    let painter = RecordingPainter::new();
    render_slide(
        &painter,
        &layout_with(vec![shape]),
        &RenderOptions::default(),
    )
    .expect("render");
    let ops = painter.ops();
    assert!(ops.contains(&Op::SetFont("italic bold 32pt 'Meiryo'".to_string())));
    assert!(ops.contains(&Op::SetFillStyle("#112233".to_string())));
    // Shape-local run coordinates land inside the shape's rect.
    assert!(ops.contains(&Op::FillText("Hello".to_string(), 96.0 + 9.6, 48.0 + 30.0)));
}

#[test]
fn a_rotated_shape_is_drawn_inside_a_saved_transform() {
    let mut shape = bare_shape(PlacedGeometry::Rect);
    shape.fill = Some("#FF0000".to_string());
    shape.rotation_rad = std::f64::consts::FRAC_PI_2;
    shape.flip_h = true;
    let painter = RecordingPainter::new();
    render_slide(
        &painter,
        &layout_with(vec![shape]),
        &RenderOptions::default(),
    )
    .expect("render");
    let ops = painter.ops();

    let save = ops.iter().position(|op| *op == Op::Save).expect("save");
    let restore = ops
        .iter()
        .rposition(|op| *op == Op::Restore)
        .expect("restore");
    // Rotation happens about the shape's centre.
    assert!(ops[save..restore].contains(&Op::Translate(192.0, 96.0)));
    assert!(ops[save..restore]
        .iter()
        .any(|op| matches!(op, Op::Rotate(r) if (*r - std::f64::consts::FRAC_PI_2).abs() < 1e-9)));
    assert!(ops[save..restore].contains(&Op::Scale(-1.0, 1.0)));
    let fill = ops
        .iter()
        .position(|op| matches!(op, Op::FillRect(x, ..) if *x == 96.0))
        .expect("shape fill");
    assert!(save < fill && fill < restore);
}

#[test]
fn an_unrotated_shape_needs_no_transform_stack() {
    let mut shape = bare_shape(PlacedGeometry::Rect);
    shape.fill = Some("#FF0000".to_string());
    let painter = RecordingPainter::new();
    render_slide(
        &painter,
        &layout_with(vec![shape]),
        &RenderOptions::default(),
    )
    .expect("render");
    assert!(!painter.ops().contains(&Op::Rotate(0.0)));
}

#[test]
fn a_layout_background_wins_over_the_default() {
    let mut layout = layout_with(Vec::new());
    layout.background = Some("#123456".to_string());
    let painter = RecordingPainter::new();
    render_slide(&painter, &layout, &RenderOptions::default()).expect("render");
    let ops = painter.ops();
    assert_eq!(ops[0], Op::SetFillStyle("#123456".to_string()));
    assert_eq!(ops[1], Op::FillRect(0.0, 0.0, 960.0, 720.0));
    assert!(!ops.contains(&Op::SetFillStyle("#FFFFFF".to_string())));
}

#[test]
fn a_line_connector_strokes_corner_to_corner() {
    let mut line = bare_shape(PlacedGeometry::Line);
    line.outline = Some(PlacedOutline {
        color: "#FF0000".to_string(),
        width_px: 3.0,
        dash_px: Vec::new(),
        head_end: None,
        tail_end: None,
    });
    let painter = RecordingPainter::new();
    render_slide(
        &painter,
        &layout_with(vec![line]),
        &RenderOptions::default(),
    )
    .expect("render");
    let ops = painter.ops();
    assert!(ops.contains(&Op::SetStrokeStyle("#FF0000".to_string())));
    assert!(ops.contains(&Op::SetLineWidth(3.0)));
    let move_to = ops
        .iter()
        .position(|op| *op == Op::MoveTo(96.0, 48.0))
        .expect("line start");
    assert_eq!(ops[move_to + 1], Op::LineTo(96.0 + 192.0, 48.0 + 96.0));
    assert!(ops.contains(&Op::Stroke));
    assert!(!ops.contains(&Op::Fill));
}

#[test]
fn a_line_without_an_outline_still_shows() {
    let line = bare_shape(PlacedGeometry::Line);
    let painter = RecordingPainter::new();
    render_slide(
        &painter,
        &layout_with(vec![line]),
        &RenderOptions::default(),
    )
    .expect("render");
    // A connector with no ln child would otherwise be invisible; PowerPoint
    // draws it with the default line.
    assert!(painter.ops().contains(&Op::Stroke));
}

#[test]
fn a_polygon_fills_its_scaled_path() {
    let mut triangle = bare_shape(PlacedGeometry::Polygon(vec![
        (0.5, 0.0),
        (1.0, 1.0),
        (0.0, 1.0),
    ]));
    triangle.fill = Some("#00FF00".to_string());
    let painter = RecordingPainter::new();
    render_slide(
        &painter,
        &layout_with(vec![triangle]),
        &RenderOptions::default(),
    )
    .expect("render");
    let ops = painter.ops();
    // Normalized points scale into the 192x96 rect at (96, 48).
    let move_to = ops
        .iter()
        .position(|op| *op == Op::MoveTo(96.0 + 96.0, 48.0))
        .expect("apex");
    assert_eq!(ops[move_to + 1], Op::LineTo(96.0 + 192.0, 48.0 + 96.0));
    assert_eq!(ops[move_to + 2], Op::LineTo(96.0, 48.0 + 96.0));
    assert!(ops[move_to..].contains(&Op::ClosePath));
    assert!(ops.contains(&Op::Fill));
    assert!(!ops
        .iter()
        .any(|op| matches!(op, Op::FillRect(x, ..) if *x == 96.0)));
}

fn text_shape(run: PlacedRun) -> PlacedShape {
    let mut shape = bare_shape(PlacedGeometry::Rect);
    shape.lines = vec![PlacedLine {
        runs: vec![run],
        ..PlacedLine::default()
    }];
    shape
}

fn run(text: &str) -> PlacedRun {
    PlacedRun {
        text: text.to_string(),
        x: 9.6,
        baseline_y: 30.0,
        size_pt: 20.0,
        bold: false,
        italic: false,
        underline: false,
        strike: false,
        color: "#112233".to_string(),
        font: None,
        east_asian_font: None,
        hyperlink: None,
        width: 0.0,
        source_offsets: Vec::new(),
        highlight: None,
        letter_spacing_px: 0.0,
        text_outline: None,
        text_shadow: None,
        text_glow: None,
        bullet_image: None,
        underline_style: None,
        double_strike: false,
    }
}

#[test]
fn a_dashed_outline_sets_the_dash_before_stroking_and_clears_it_after() {
    let mut shape = bare_shape(PlacedGeometry::Rect);
    shape.outline = Some(PlacedOutline {
        color: "#FF0000".to_string(),
        width_px: 2.0,
        dash_px: vec![8.0, 6.0],
        head_end: None,
        tail_end: None,
    });
    let painter = RecordingPainter::new();
    render_slide(
        &painter,
        &layout_with(vec![shape]),
        &RenderOptions::default(),
    )
    .expect("render");
    let ops = painter.ops();
    let dash = ops
        .iter()
        .position(|op| *op == Op::SetLineDash(vec![8.0, 6.0]))
        .expect("dash set");
    let stroke = ops
        .iter()
        .position(|op| *op == Op::StrokeRect(96.0, 48.0, 192.0, 96.0))
        .expect("stroke");
    let cleared = ops
        .iter()
        .rposition(|op| *op == Op::SetLineDash(Vec::new()))
        .expect("dash cleared");
    assert!(dash < stroke && stroke < cleared, "{ops:?}");
}

#[test]
fn a_solid_outline_never_touches_the_dash() {
    let mut shape = bare_shape(PlacedGeometry::Rect);
    shape.outline = Some(PlacedOutline {
        color: "#FF0000".to_string(),
        width_px: 2.0,
        dash_px: Vec::new(),
        head_end: None,
        tail_end: None,
    });
    let painter = RecordingPainter::new();
    render_slide(
        &painter,
        &layout_with(vec![shape]),
        &RenderOptions::default(),
    )
    .expect("render");
    assert_eq!(
        painter.count(|op| matches!(op, Op::SetLineDash(_))),
        0,
        "{:?}",
        painter.ops()
    );
}

#[test]
fn an_underlined_run_draws_a_rule_under_its_baseline_in_its_colour() {
    let painter = RecordingPainter::with_width_per_char(10.0);
    render_slide(
        &painter,
        &layout_with(vec![text_shape(PlacedRun {
            underline: true,
            ..run("Hello")
        })]),
        &RenderOptions::default(),
    )
    .expect("render");
    let ops = painter.ops();
    let text = ops
        .iter()
        .position(|op| matches!(op, Op::FillText(t, _, _) if t == "Hello"))
        .expect("text drawn");
    // The rule: 50 px wide (5 chars × 10), just below the baseline, as thick
    // as PowerPoint's single underline (about a fifteenth of the size), and
    // in the run's colour.
    let rule = ops[text + 1..]
        .iter()
        .find_map(|op| match op {
            Op::FillRect(x, y, w, h) => Some((*x, *y, *w, *h)),
            _ => None,
        })
        .expect("underline rule");
    let size_px = 20.0 * 96.0 / 72.0;
    assert!((rule.0 - (96.0 + 9.6)).abs() < 1e-9);
    assert!((rule.2 - 50.0).abs() < 1e-9);
    assert!(
        rule.1 > 48.0 + 30.0 && rule.1 < 48.0 + 30.0 + size_px * 0.2,
        "{rule:?}"
    );
    assert!((rule.3 - size_px / 15.0).abs() < 1e-9);
    assert_eq!(
        ops[text - 2..text]
            .iter()
            .filter(|op| **op == Op::SetFillStyle("#112233".to_string()))
            .count(),
        1,
        "{ops:?}"
    );
}

#[test]
fn a_struck_run_draws_a_rule_through_the_x_height() {
    let painter = RecordingPainter::with_width_per_char(10.0);
    render_slide(
        &painter,
        &layout_with(vec![text_shape(PlacedRun {
            strike: true,
            ..run("Hello")
        })]),
        &RenderOptions::default(),
    )
    .expect("render");
    let ops = painter.ops();
    let text = ops
        .iter()
        .position(|op| matches!(op, Op::FillText(t, _, _) if t == "Hello"))
        .expect("text drawn");
    let rule = ops[text + 1..]
        .iter()
        .find_map(|op| match op {
            Op::FillRect(x, y, w, h) => Some((*x, *y, *w, *h)),
            _ => None,
        })
        .expect("strike rule");
    let size_px = 20.0 * 96.0 / 72.0;
    assert!(
        rule.1 < 48.0 + 30.0 - size_px * 0.2 && rule.1 > 48.0 + 30.0 - size_px * 0.5,
        "{rule:?}"
    );
}

#[test]
fn a_plain_run_draws_no_rule() {
    let painter = RecordingPainter::with_width_per_char(10.0);
    render_slide(
        &painter,
        &layout_with(vec![text_shape(run("Hello"))]),
        &RenderOptions::default(),
    )
    .expect("render");
    // Only the background fills a rect.
    assert_eq!(painter.count(|op| matches!(op, Op::FillRect(..))), 1);
}

#[test]
fn the_font_shorthand_lists_the_latin_face_then_the_east_asian_face() {
    let painter = RecordingPainter::new();
    render_slide(
        &painter,
        &layout_with(vec![text_shape(PlacedRun {
            font: Some("Yu Gothic".to_string()),
            east_asian_font: Some("Yu Gothic Medium".to_string()),
            ..run("対象")
        })]),
        &RenderOptions::default(),
    )
    .expect("render");
    assert!(
        painter.ops().contains(&Op::SetFont(
            "20pt 'Yu Gothic', 'Yu Gothic Medium'".to_string()
        )),
        "{:?}",
        painter.ops()
    );
}

#[test]
fn a_run_with_only_an_east_asian_face_uses_it_before_the_default() {
    let painter = RecordingPainter::new();
    render_slide(
        &painter,
        &layout_with(vec![text_shape(PlacedRun {
            east_asian_font: Some("Meiryo".to_string()),
            ..run("対象")
        })]),
        &RenderOptions::default(),
    )
    .expect("render");
    assert!(
        painter
            .ops()
            .contains(&Op::SetFont("20pt 'Meiryo', 'Calibri'".to_string())),
        "{:?}",
        painter.ops()
    );
}

#[test]
fn turned_text_draws_inside_a_rotated_frame_about_the_shape_centre() {
    let mut shape = text_shape(run("縦"));
    shape.text_rotation_deg = 90.0;
    let painter = RecordingPainter::new();
    render_slide(
        &painter,
        &layout_with(vec![shape]),
        &RenderOptions::default(),
    )
    .expect("render");
    let ops = painter.ops();
    let rotate = ops
        .iter()
        .position(|op| matches!(op, Op::Rotate(angle) if (angle - std::f64::consts::FRAC_PI_2).abs() < 1e-12))
        .expect("rotated a quarter turn");
    // Centre of the 192×96 shape at (96, 48), then back by half the turned
    // frame (96 × 192).
    assert_eq!(ops[rotate - 1], Op::Translate(96.0 + 96.0, 48.0 + 48.0));
    assert_eq!(ops[rotate + 1], Op::Translate(-48.0, -96.0));
    // The run draws at its frame coordinates, not offset by the shape.
    let text = ops
        .iter()
        .position(|op| matches!(op, Op::FillText(t, x, y) if t == "縦" && *x == 9.6 && *y == 30.0))
        .expect("text in frame coordinates");
    assert!(text > rotate);
    let restore = ops
        .iter()
        .rposition(|op| *op == Op::Restore)
        .expect("restored");
    assert!(restore > text);
}

fn gradient_shape() -> PlacedShape {
    let mut shape = bare_shape(PlacedGeometry::Rect);
    shape.fill = Some("#FF0000".to_string());
    shape.gradient = Some(jade_pptx_layout::PlacedGradient {
        angle_deg: 0.0,
        stops: vec![
            jade_pptx_layout::PlacedGradientStop {
                position: 0.0,
                color: "#FF0000".to_string(),
            },
            jade_pptx_layout::PlacedGradientStop {
                position: 1.0,
                color: "#0000FF".to_string(),
            },
        ],
        radial: None,
    });
    shape
}

#[test]
fn a_gradient_fills_the_clipped_shape_with_bands_from_the_first_stop_to_the_last() {
    let painter = RecordingPainter::new();
    render_slide(
        &painter,
        &layout_with(vec![gradient_shape()]),
        &RenderOptions::default(),
    )
    .expect("render");
    let ops = painter.ops();
    let clip = ops
        .iter()
        .position(|op| *op == Op::Clip)
        .expect("clipped to the shape");
    let bands: Vec<(usize, &Op)> = ops
        .iter()
        .enumerate()
        .skip(clip)
        .filter(|(_, op)| matches!(op, Op::FillRect(..)))
        .collect();
    assert!(bands.len() >= 16, "{} bands", bands.len());
    // The colour set before the first band is the first stop, before the last
    // band the last stop, and in between it moves from red toward blue.
    let colour_before = |index: usize| {
        ops[..index]
            .iter()
            .rev()
            .find_map(|op| match op {
                Op::SetFillStyle(colour) => Some(colour.clone()),
                _ => None,
            })
            .expect("colour")
    };
    assert_eq!(colour_before(bands[0].0), "#FF0000");
    assert_eq!(colour_before(bands[bands.len() - 1].0), "#0000FF");
    let middle = colour_before(bands[bands.len() / 2].0);
    assert_ne!(middle, "#FF0000");
    assert_ne!(middle, "#0000FF");
    // The flat fill is not painted as well: no FillRect of the shape's own
    // rect outside the clipped bands.
    assert!(!ops[..clip].contains(&Op::FillRect(96.0, 48.0, 192.0, 96.0)));
    // The clip is undone before the next shape.
    let restore = ops
        .iter()
        .rposition(|op| *op == Op::Restore)
        .expect("restore");
    assert!(restore > bands[bands.len() - 1].0);
}

#[test]
fn a_gradient_background_covers_the_slide() {
    let mut layout = layout_with(Vec::new());
    layout.background = Some("#FFFFFF".to_string());
    layout.background_gradient = Some(jade_pptx_layout::PlacedGradient {
        angle_deg: 90.0,
        stops: vec![
            jade_pptx_layout::PlacedGradientStop {
                position: 0.0,
                color: "#FFFFFF".to_string(),
            },
            jade_pptx_layout::PlacedGradientStop {
                position: 1.0,
                color: "#000000".to_string(),
            },
        ],
        radial: None,
    });
    let painter = RecordingPainter::new();
    render_slide(&painter, &layout, &RenderOptions::default()).expect("render");
    let ops = painter.ops();
    assert!(ops.contains(&Op::Rect(0.0, 0.0, 960.0, 720.0)), "{ops:?}");
    assert!(ops.contains(&Op::Clip));
    assert!(painter.count(|op| matches!(op, Op::FillRect(..))) >= 16);
}

#[test]
fn a_path_geometry_draws_each_subpath_closed() {
    let mut shape = bare_shape(PlacedGeometry::Path(vec![
        vec![(0.0, 0.0), (1.0, 0.0), (0.5, 1.0)],
        vec![(0.1, 0.1), (0.2, 0.1), (0.2, 0.2)],
    ]));
    shape.fill = Some("#00FF00".to_string());
    let painter = RecordingPainter::new();
    render_slide(
        &painter,
        &layout_with(vec![shape]),
        &RenderOptions::default(),
    )
    .expect("render");
    let ops = painter.ops();
    // Normalized points scale to the 192×96 rect at (96, 48).
    assert!(ops.contains(&Op::MoveTo(96.0, 48.0)));
    assert!(ops.contains(&Op::LineTo(96.0 + 192.0, 48.0)));
    assert!(ops.contains(&Op::LineTo(96.0 + 96.0, 48.0 + 96.0)));
    assert!(ops.contains(&Op::MoveTo(96.0 + 19.2, 48.0 + 9.6)));
    assert_eq!(painter.count(|op| *op == Op::ClosePath), 2);
    assert_eq!(painter.count(|op| *op == Op::Fill), 1);
}

fn picture_shape() -> PlacedShape {
    let mut shape = bare_shape(PlacedGeometry::Rect);
    shape.image_part = Some("ppt/media/image1.png".to_string());
    shape
}

#[test]
fn a_host_that_draws_the_picture_replaces_the_placeholder_fill() {
    let painter = RecordingPainter::new();
    let mut seen: Vec<String> = Vec::new();
    jade_pptx_render::render_slide_with(
        &painter,
        &layout_with(vec![picture_shape()]),
        &RenderOptions::default(),
        &mut |shape: &PlacedShape| {
            seen.push(shape.image_part.clone().expect("only pictures are offered"));
            true
        },
    )
    .expect("render");
    assert_eq!(seen, vec!["ppt/media/image1.png"]);
    // No placeholder rect at the picture's footprint: only the background.
    assert_eq!(painter.count(|op| matches!(op, Op::FillRect(..))), 1);
}

#[test]
fn a_host_that_declines_the_picture_gets_the_placeholder() {
    let painter = RecordingPainter::new();
    jade_pptx_render::render_slide_with(
        &painter,
        &layout_with(vec![picture_shape()]),
        &RenderOptions::default(),
        &mut |_shape: &PlacedShape| false,
    )
    .expect("render");
    assert!(painter
        .ops()
        .contains(&Op::FillRect(96.0, 48.0, 192.0, 96.0)));
    assert!(painter
        .ops()
        .contains(&Op::SetFillStyle("#E0E0E0".to_string())));
}

#[test]
fn a_rotated_picture_is_offered_inside_its_transform() {
    // The hook runs between the shape's save/rotate and its restore, so a
    // host drawing at the shape's rect lands rotated like everything else.
    let painter = RecordingPainter::new();
    let mut shape = picture_shape();
    shape.rotation_rad = 0.5;
    let mut ops_at_hook = 0usize;
    jade_pptx_render::render_slide_with(
        &painter,
        &layout_with(vec![shape]),
        &RenderOptions::default(),
        &mut |_shape: &PlacedShape| {
            ops_at_hook = painter.ops().len();
            true
        },
    )
    .expect("render");
    let ops = painter.ops();
    let rotate = ops
        .iter()
        .position(|op| matches!(op, Op::Rotate(_)))
        .expect("rotated");
    let restore = ops
        .iter()
        .rposition(|op| *op == Op::Restore)
        .expect("restored");
    assert!(
        rotate < ops_at_hook && ops_at_hook <= restore,
        "{rotate} < {ops_at_hook} <= {restore}"
    );
}

#[test]
fn shapes_without_a_picture_are_not_offered_to_the_host() {
    let painter = RecordingPainter::new();
    let mut offered = 0usize;
    let mut plain = bare_shape(PlacedGeometry::Rect);
    plain.fill = Some("#FF0000".to_string());
    jade_pptx_render::render_slide_with(
        &painter,
        &layout_with(vec![plain]),
        &RenderOptions::default(),
        &mut |_shape: &PlacedShape| {
            offered += 1;
            true
        },
    )
    .expect("render");
    assert_eq!(offered, 0);
}

#[test]
fn picture_fill_is_clipped_to_its_shape_before_the_host_draws() {
    let painter = RecordingPainter::new();
    let mut shape = picture_shape();
    shape.geometry = PlacedGeometry::Ellipse;
    let mut hook_ops = Vec::new();
    jade_pptx_render::render_slide_with(
        &painter,
        &layout_with(vec![shape]),
        &RenderOptions::default(),
        &mut |_| {
            hook_ops = painter.ops();
            true
        },
    )
    .unwrap();
    assert_eq!(hook_ops.last(), Some(&Op::Clip));
    assert!(hook_ops.iter().any(|op| matches!(op, Op::Ellipse(..))));
    let ops = painter.ops();
    assert_eq!(
        ops.iter().filter(|op| **op == Op::Save).count(),
        ops.iter().filter(|op| **op == Op::Restore).count()
    );
}

#[test]
fn a_decoded_picture_is_not_covered_by_the_theme_fallback_fill() {
    let painter = RecordingPainter::new();
    let mut shape = gradient_shape();
    shape.image_part = Some("ppt/media/image1.png".into());
    jade_pptx_render::render_slide_with(
        &painter,
        &layout_with(vec![shape]),
        &RenderOptions::default(),
        &mut |_| true,
    )
    .unwrap();
    assert_eq!(painter.count(|op| matches!(op, Op::FillRect(..))), 1);
    assert_eq!(painter.count(|op| matches!(op, Op::Fill)), 0);
}

#[test]
fn font_family_quotes_and_backslashes_are_escaped_for_painting_and_measurement() {
    let painter = RecordingPainter::new();
    render_slide(
        &painter,
        &layout_with(vec![text_shape(PlacedRun {
            font: Some("O'Brien \\ Face".into()),
            east_asian_font: Some("游ゴシック".into()),
            ..run("対象")
        })]),
        &RenderOptions::default(),
    )
    .unwrap();
    assert!(painter.ops().contains(&Op::SetFont(
        "20pt 'O\\'Brien \\\\ Face', '游ゴシック'".into()
    )));
}

#[test]
fn distributed_text_draws_native_glyph_positions_and_decorations_cover_expanded_advances() {
    let content=jade_pptx_reader::parse_slide_xml(r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:sp><p:spPr><a:xfrm><a:off x="952500" y="1905000"/><a:ext cx="952500" cy="952500"/></a:xfrm></p:spPr><p:txBody><a:bodyPr lIns="0" rIns="0" tIns="0" bIns="0"/><a:p><a:pPr algn="dist"/><a:r><a:rPr sz="1200" u="sng" strike="sngStrike"/><a:t>日本</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#).unwrap();
    let layout = jade_pptx_layout::layout_slide(
        jade_pptx_reader::SlideSize::default(),
        &content,
        &jade_pptx_reader::ThemeColors::default(),
        &jade_pptx_layout::HeuristicMeasure,
    );
    let shape = &layout.shapes[0];
    let runs = &shape.lines[0].runs;
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].width, 84.0);
    assert_eq!(runs[1].width, 16.0);
    let painter = RecordingPainter::with_width_per_char(3.0);
    render_slide(&painter, &layout, &RenderOptions::default()).unwrap();
    let ops = painter.ops();
    for run in runs {
        assert!(ops.contains(&Op::FillText(
            run.text.clone(),
            100.0 + run.x,
            200.0 + run.baseline_y
        )));
        assert!(ops.iter().any(|op|matches!(op,Op::FillRect(x,_,w,_) if (*x-(100.0+run.x)).abs()<1e-9&&(*w-run.width).abs()<1e-9)));
    }
    assert!(
        !ops.iter().any(|op| matches!(op, Op::MeasureText(_))),
        "renderer consumes layout advances rather than remeasuring justified text"
    );
}

#[test]
fn multi_master_deck_paints_each_slides_native_theme_and_override_font_and_colors() {
    let deck =
        jade_pptx_reader::read_deck(include_bytes!("../../../public/fixtures/themed-deck.pptx"))
            .unwrap();
    for (index, font, text_color, fill_color) in [
        (0, "monospace", "#14C814", "#F01414"),
        (1, "serif", "#1414F0", "#E6A010"),
        (2, "sans-serif", "#14C8C8", "#A020F0"),
        (3, "monospace", "#A020F0", "#F014C8"),
    ] {
        let layout =
            jade_pptx_layout::layout_deck_slide(&deck, index, &jade_pptx_layout::HeuristicMeasure)
                .unwrap();
        let painter = RecordingPainter::new();
        render_slide(&painter, &layout, &RenderOptions::default()).unwrap();
        let ops = painter.ops();
        assert!(ops.contains(&Op::SetFillStyle(text_color.into())));
        assert!(ops.contains(&Op::SetFillStyle(fill_color.into())));
        assert!(ops
            .iter()
            .any(|op| matches!(op,Op::SetFont(value) if value.contains(font))));
        assert!(ops
            .iter()
            .any(|op| matches!(op,Op::FillText(text,_,_) if text=="WWWW iii")));
    }
}
