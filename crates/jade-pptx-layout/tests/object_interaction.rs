use jade_pptx_layout::{
    creation_rect, crop_transform, drag_transform, layout_slide, DragKind, HeuristicMeasure,
    ObjectTransform, PlacedGeometry, PlacedShape, Rect,
};
use jade_pptx_reader::SourceRect;

fn bitmap_point(value: ObjectTransform, crop: SourceRect, u: f64, v: f64) -> (f64, f64) {
    let w = 1.0 - f64::from(crop.left + crop.right) / 100000.0;
    let h = 1.0 - f64::from(crop.top + crop.bottom) / 100000.0;
    let x = ((u - f64::from(crop.left) / 100000.0) / w - 0.5)
        * value.rect.width
        * if value.flip_h { -1.0 } else { 1.0 };
    let y = ((v - f64::from(crop.top) / 100000.0) / h - 0.5)
        * value.rect.height
        * if value.flip_v { -1.0 } else { 1.0 };
    let (sin, cos) = value.rotation_degrees.to_radians().sin_cos();
    (
        value.rect.x + value.rect.width / 2.0 + cos * x - sin * y,
        value.rect.y + value.rect.height / 2.0 + sin * x + cos * y,
    )
}

#[test]
fn cropping_preserves_bitmap_points_under_rotation_flips_and_outsets() {
    for (rotation, flip_h, flip_v) in [
        (0.0, false, false),
        (30.0, true, false),
        (-45.0, false, true),
        (90.0, true, true),
    ] {
        let value = ObjectTransform {
            rotation_degrees: rotation,
            flip_h,
            flip_v,
            ..before(0.0)
        };
        for (old, new) in [
            (
                SourceRect::default(),
                SourceRect {
                    left: 25000,
                    top: 15000,
                    right: 10000,
                    bottom: 5000,
                },
            ),
            (
                SourceRect {
                    left: -25000,
                    top: -10000,
                    right: -15000,
                    bottom: 0,
                },
                SourceRect::default(),
            ),
        ] {
            let cropped = crop_transform(value, old, new).unwrap();
            let a = bitmap_point(value, old, 0.6, 0.4);
            let b = bitmap_point(cropped, new, 0.6, 0.4);
            assert!((a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9);
            assert_eq!(crop_transform(cropped, new, old).unwrap().rect, value.rect);
        }
    }
}

#[test]
fn cropping_rejects_empty_frames_invalid_windows_and_nonfinite_geometry() {
    assert!(crop_transform(
        before(0.0),
        SourceRect {
            left: 100000,
            ..Default::default()
        },
        SourceRect::default()
    )
    .is_none());
    assert!(crop_transform(
        before(0.0),
        SourceRect::default(),
        SourceRect {
            left: 100000,
            ..Default::default()
        }
    )
    .is_none());
    assert!(crop_transform(
        ObjectTransform {
            rotation_degrees: f64::NAN,
            ..before(0.0)
        },
        SourceRect::default(),
        SourceRect::default()
    )
    .is_none());
}

#[test]
fn creation_bounds_normalize_all_drag_directions_without_changing_slide_coordinates() {
    for (start, end) in [
        ((10.0, 20.0), (110.0, 70.0)),
        ((110.0, 70.0), (10.0, 20.0)),
        ((10.0, 70.0), (110.0, 20.0)),
        ((110.0, 20.0), (10.0, 70.0)),
    ] {
        assert_eq!(
            creation_rect(start, end, false),
            Some(Rect {
                x: 10.0,
                y: 20.0,
                width: 100.0,
                height: 50.0
            })
        );
    }
    assert_eq!(
        creation_rect((-10.0, -20.0), (-30.0, -50.0), false),
        Some(Rect {
            x: -30.0,
            y: -50.0,
            width: 20.0,
            height: 30.0
        })
    );
    assert_eq!(
        creation_rect((5.0, 6.0), (5.0, 6.0), false),
        Some(Rect {
            x: 5.0,
            y: 6.0,
            width: 0.0,
            height: 0.0
        })
    );
}

#[test]
fn constrained_creation_keeps_the_start_corner_and_drag_quadrant() {
    for (end, x, y) in [
        ((30.0, 10.0), 0.0, 0.0),
        ((-30.0, 10.0), -30.0, 0.0),
        ((30.0, -10.0), 0.0, -30.0),
        ((-30.0, -10.0), -30.0, -30.0),
    ] {
        assert_eq!(
            creation_rect((0.0, 0.0), end, true),
            Some(Rect {
                x,
                y,
                width: 30.0,
                height: 30.0
            })
        );
    }
}

#[test]
fn creation_rejects_nonfinite_coordinates_and_overflow() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(creation_rect((value, 0.0), (1.0, 1.0), false).is_none());
        assert!(creation_rect((0.0, 0.0), (1.0, value), true).is_none());
    }
    assert!(creation_rect((-f64::MAX, 0.0), (f64::MAX, 1.0), false).is_none());
}
use jade_pptx_reader::{parse_slide_xml, SlideSize, ThemeColors};

fn shape() -> PlacedShape {
    let xml = r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:sp><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="952500" cy="190500"/></a:xfrm><a:prstGeom prst="rect"/></p:spPr></p:sp></p:spTree></p:cSld></p:sld>"#;
    layout_slide(
        SlideSize::default(),
        &parse_slide_xml(xml).unwrap(),
        &ThemeColors::default(),
        &HeuristicMeasure,
    )
    .shapes
    .remove(0)
}

#[test]
fn hit_testing_inverts_rotation_flips_and_actual_shape_geometry() {
    let mut s = shape();
    s.rotation_rad = std::f64::consts::FRAC_PI_2;
    assert!(s.contains_point(50.0, 50.0, 0.0));
    assert!(!s.contains_point(10.0, 10.0, 0.0));
    s.rotation_rad = 0.0;
    s.rect.height = 100.0;
    s.geometry = PlacedGeometry::Ellipse;
    assert!(s.contains_point(50.0, 50.0, 0.0));
    assert!(!s.contains_point(0.0, 0.0, 0.0));
    s.geometry = PlacedGeometry::Polygon(vec![(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)]);
    s.flip_h = true;
    assert!(s.contains_point(90.0, 90.0, 0.0));
    assert!(!s.contains_point(10.0, 90.0, 0.0));
    s.flip_h = false;
    s.geometry = PlacedGeometry::RoundRect { adjust: 0.25 };
    assert!(!s.contains_point(0.0, 0.0, 0.0));
    assert!(s.contains_point(25.0, 0.0, 0.0));
}

#[test]
fn connector_hit_testing_handles_zero_dimensions_and_screen_tolerance() {
    let mut s = shape();
    s.geometry = PlacedGeometry::Line;
    s.rect.height = 0.0;
    assert!(s.contains_point(50.0, 2.0, 3.0));
    assert!(!s.contains_point(50.0, 8.0, 3.0));
    s.rect.width = 0.0;
    assert!(s.contains_point(0.0, 0.0, 0.0));
    assert!(!s.contains_point(f64::NAN, 0.0, 0.0));
}

#[test]
fn freeform_holes_follow_nonzero_winding_like_the_renderer() {
    let mut s = shape();
    s.rect.height = 100.0;
    s.geometry = PlacedGeometry::Path(vec![
        vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
        vec![(0.25, 0.25), (0.25, 0.75), (0.75, 0.75), (0.75, 0.25)],
    ]);
    assert!(s.contains_point(10.0, 10.0, 0.0));
    assert!(!s.contains_point(50.0, 50.0, 0.0));
}

fn before(angle: f64) -> ObjectTransform {
    ObjectTransform {
        rect: Rect {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 50.0,
        },
        rotation_degrees: angle,
        flip_h: true,
        flip_v: false,
    }
}
fn top_left(t: ObjectTransform) -> (f64, f64) {
    let r = t.rect;
    let (sin, cos) = t.rotation_degrees.to_radians().sin_cos();
    (
        r.x + r.width / 2.0 - cos * r.width / 2.0 + sin * r.height / 2.0,
        r.y + r.height / 2.0 - sin * r.width / 2.0 - cos * r.height / 2.0,
    )
}

#[test]
fn rotated_resize_fixes_the_opposite_corner_and_shift_retains_aspect_ratio() {
    for angle in [0.0, 45.0, 90.0, 180.0, 270.0] {
        for constrained in [false, true] {
            let original = before(angle);
            let resized = drag_transform(
                original,
                DragKind::Resize,
                (0.0, 0.0),
                (20.0, 30.0),
                constrained,
            )
            .unwrap();
            let a = top_left(original);
            let b = top_left(resized);
            assert!((a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9);
            if constrained {
                assert!((resized.rect.width / resized.rect.height - 2.0).abs() < 1e-9);
            }
            assert_eq!(resized.flip_h, original.flip_h);
        }
    }
    let clamped = drag_transform(
        before(0.0),
        DragKind::Resize,
        (0.0, 0.0),
        (-200.0, -200.0),
        false,
    )
    .unwrap();
    assert_eq!((clamped.rect.width, clamped.rect.height), (1.0, 1.0));
}

#[test]
fn move_and_rotation_support_axis_and_fifteen_degree_constraints() {
    let moved =
        drag_transform(before(45.0), DragKind::Move, (0.0, 0.0), (30.0, 5.0), true).unwrap();
    assert_eq!((moved.rect.x, moved.rect.y), (40.0, 20.0));
    let rotated = drag_transform(
        before(0.0),
        DragKind::Rotate,
        (60.0, -5.0),
        (110.0, 45.0),
        false,
    )
    .unwrap();
    assert_eq!(rotated.rotation_degrees, 90.0);
    let snapped = drag_transform(
        before(0.0),
        DragKind::Rotate,
        (60.0, -5.0),
        (100.0, 10.0),
        true,
    )
    .unwrap();
    assert_eq!(snapped.rotation_degrees % 15.0, 0.0);
    assert!(drag_transform(
        before(0.0),
        DragKind::Move,
        (f64::NAN, 0.0),
        (0.0, 0.0),
        false
    )
    .is_none());
}
#[test]
fn grouping_bounds_cover_negative_frames_and_zero_height_lines_without_transforming_them() {
    use jade_pptx_layout::grouping_transform;
    let object = |x, y, w, h| ObjectTransform {
        rect: jade_pptx_layout::Rect {
            x,
            y,
            width: w,
            height: h,
        },
        rotation_degrees: 90.0,
        flip_h: true,
        flip_v: false,
    };
    let t = grouping_transform(&[
        object(-10.0, 20.0, 30.0, 0.0),
        object(40.0, 20.0, 20.0, 0.0),
    ])
    .unwrap();
    assert_eq!(t.offset_emu, (-95250, 190500));
    assert_eq!(t.extent_emu, (666750, 1));
    assert_eq!(t.rotation_60000ths, 0);
    assert!(!t.flip_h && !t.flip_v);
    assert!(grouping_transform(&[object(0.0, 0.0, 1.0, 1.0)]).is_none());
    assert!(
        grouping_transform(&[object(f64::NAN, 0.0, 1.0, 1.0), object(0.0, 0.0, 1.0, 1.0)])
            .is_none()
    );
}
