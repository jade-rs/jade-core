//! Selection geometry and drag arithmetic in slide coordinates.
use crate::{PlacedGeometry, PlacedShape, Rect};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectTransform {
    pub rect: Rect,
    pub rotation_degrees: f64,
    pub flip_h: bool,
    pub flip_v: bool,
}

/// An identity child space enclosing the selected pre-rotation object frames.
pub fn grouping_transform(objects: &[ObjectTransform]) -> Option<jade_pptx_reader::Transform> {
    if objects.len() < 2
        || objects.iter().any(|o| {
            [o.rect.x, o.rect.y, o.rect.width, o.rect.height]
                .iter()
                .any(|v| !v.is_finite())
                || o.rect.width < 0.0
                || o.rect.height < 0.0
        })
    {
        return None;
    }
    let x = objects
        .iter()
        .map(|o| o.rect.x)
        .fold(f64::INFINITY, f64::min);
    let y = objects
        .iter()
        .map(|o| o.rect.y)
        .fold(f64::INFINITY, f64::min);
    let right = objects
        .iter()
        .map(|o| o.rect.x + o.rect.width)
        .fold(f64::NEG_INFINITY, f64::max);
    let bottom = objects
        .iter()
        .map(|o| o.rect.y + o.rect.height)
        .fold(f64::NEG_INFINITY, f64::max);
    let (x, y, right, bottom) = (
        (x * 9525.0).floor(),
        (y * 9525.0).floor(),
        (right * 9525.0).ceil(),
        (bottom * 9525.0).ceil(),
    );
    let (width, height) = ((right - x).max(1.0), (bottom - y).max(1.0));
    if [x, y, width, height]
        .iter()
        .any(|v| !v.is_finite() || *v < f64::from(i32::MIN) || *v > f64::from(i32::MAX))
    {
        return None;
    }
    Some(jade_pptx_reader::Transform {
        offset_emu: (x as i64, y as i64),
        extent_emu: (width as i64, height as i64),
        rotation_60000ths: 0,
        flip_h: false,
        flip_v: false,
    })
}

impl From<&PlacedShape> for ObjectTransform {
    fn from(shape: &PlacedShape) -> Self {
        Self {
            rect: shape.rect,
            rotation_degrees: shape.rotation_rad.to_degrees(),
            flip_h: shape.flip_h,
            flip_v: shape.flip_v,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DragKind {
    Move,
    Resize,
    Rotate,
}

/// Bounds for drag creation, in the same slide coordinate system as objects.
pub fn creation_rect(start: (f64, f64), current: (f64, f64), constrained: bool) -> Option<Rect> {
    if [start.0, start.1, current.0, current.1]
        .iter()
        .any(|v| !v.is_finite())
    {
        return None;
    }
    let (mut dx, mut dy) = (current.0 - start.0, current.1 - start.1);
    if constrained {
        let side = dx.abs().max(dy.abs());
        dx = if dx < 0.0 { -side } else { side };
        dy = if dy < 0.0 { -side } else { side };
    }
    let rect = Rect {
        x: start.0 + dx.min(0.0),
        y: start.1 + dy.min(0.0),
        width: dx.abs(),
        height: dy.abs(),
    };
    [rect.x, rect.y, rect.width, rect.height]
        .iter()
        .all(|v| v.is_finite())
        .then_some(rect)
}

/// Keep the underlying bitmap's scale and position when changing its crop frame.
pub fn crop_transform(
    before: ObjectTransform,
    old: jade_pptx_reader::SourceRect,
    new: jade_pptx_reader::SourceRect,
) -> Option<ObjectTransform> {
    let axis = |left: i32, right: i32| {
        (100_000i64 - i64::from(left) - i64::from(right)) as f64 / 100_000.0
    };
    let (old_w, old_h) = (axis(old.left, old.right), axis(old.top, old.bottom));
    let (new_w, new_h) = (axis(new.left, new.right), axis(new.top, new.bottom));
    if old_w <= 0.0
        || old_h <= 0.0
        || new_w <= 0.0
        || new_h <= 0.0
        || before.rect.width <= 0.0
        || before.rect.height <= 0.0
    {
        return None;
    }
    let (image_w, image_h) = (before.rect.width / old_w, before.rect.height / old_h);
    let dx = (i64::from(new.left) - i64::from(old.left) - i64::from(new.right)
        + i64::from(old.right)) as f64
        / 200_000.0
        * image_w
        * if before.flip_h { -1.0 } else { 1.0 };
    let dy = (i64::from(new.top) - i64::from(old.top) - i64::from(new.bottom)
        + i64::from(old.bottom)) as f64
        / 200_000.0
        * image_h
        * if before.flip_v { -1.0 } else { 1.0 };
    let (sin, cos) = before.rotation_degrees.to_radians().sin_cos();
    let (width, height) = (image_w * new_w, image_h * new_h);
    let rect = Rect {
        x: before.rect.x + before.rect.width / 2.0 + cos * dx - sin * dy - width / 2.0,
        y: before.rect.y + before.rect.height / 2.0 + sin * dx + cos * dy - height / 2.0,
        width,
        height,
    };
    [rect.x, rect.y, width, height, before.rotation_degrees]
        .iter()
        .all(|v| v.is_finite())
        .then_some(ObjectTransform { rect, ..before })
}

/// Returns one preview, always relative to the pointer-down transform. The
/// opposite corner stays fixed when resizing a rotated frame.
pub fn drag_transform(
    before: ObjectTransform,
    kind: DragKind,
    start: (f64, f64),
    current: (f64, f64),
    constrained: bool,
) -> Option<ObjectTransform> {
    let r = before.rect;
    if [
        r.x,
        r.y,
        r.width,
        r.height,
        before.rotation_degrees,
        start.0,
        start.1,
        current.0,
        current.1,
    ]
    .iter()
    .any(|v| !v.is_finite())
        || r.width < 0.0
        || r.height < 0.0
    {
        return None;
    }
    let mut after = before;
    let (mut dx, mut dy) = (current.0 - start.0, current.1 - start.1);
    let angle = before.rotation_degrees.to_radians();
    let (sin, cos) = angle.sin_cos();
    match kind {
        DragKind::Move => {
            if constrained {
                if dx.abs() > dy.abs() {
                    dy = 0.0;
                } else {
                    dx = 0.0;
                }
            }
            after.rect.x += dx;
            after.rect.y += dy;
        }
        DragKind::Resize => {
            let local_dx = cos * dx + sin * dy;
            let local_dy = -sin * dx + cos * dy;
            let mut width = (r.width + local_dx).max(1.0);
            let mut height = (r.height + local_dy).max(1.0);
            if constrained && r.width > 0.0 && r.height > 0.0 {
                let factor = if (width / r.width - 1.0).abs() > (height / r.height - 1.0).abs() {
                    width / r.width
                } else {
                    height / r.height
                };
                let factor = factor.max(1.0 / r.width).max(1.0 / r.height);
                width = r.width * factor;
                height = r.height * factor;
            }
            let half_dx = (width - r.width) / 2.0;
            let half_dy = (height - r.height) / 2.0;
            after.rect = Rect {
                x: r.x + r.width / 2.0 + cos * half_dx - sin * half_dy - width / 2.0,
                y: r.y + r.height / 2.0 + sin * half_dx + cos * half_dy - height / 2.0,
                width,
                height,
            };
        }
        DragKind::Rotate => {
            let center = (r.x + r.width / 2.0, r.y + r.height / 2.0);
            let start_angle = (start.1 - center.1).atan2(start.0 - center.0);
            let next_angle = (current.1 - center.1).atan2(current.0 - center.0);
            let rotation = before.rotation_degrees + (next_angle - start_angle).to_degrees();
            after.rotation_degrees = if constrained {
                (rotation / 15.0).round() * 15.0
            } else {
                rotation
            }
            .rem_euclid(360.0);
        }
    }
    Some(after)
}

fn segment_distance(x: f64, y: f64, a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let denominator = dx * dx + dy * dy;
    let t = if denominator > 0.0 {
        ((x - a.0) * dx + (y - a.1) * dy) / denominator
    } else {
        0.0
    }
    .clamp(0.0, 1.0);
    (x - a.0 - t * dx).hypot(y - a.1 - t * dy)
}

fn polygon_winding(
    points: &[(f64, f64)],
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    tolerance: f64,
) -> (i32, bool) {
    let mut winding = 0;
    let mut edge = false;
    if points.len() < 2 {
        return (0, false);
    }
    for i in 0..points.len() {
        let a = (points[i].0 * width, points[i].1 * height);
        let b = (
            points[(i + 1) % points.len()].0 * width,
            points[(i + 1) % points.len()].1 * height,
        );
        edge |= segment_distance(x, y, a, b) <= tolerance;
        let cross = (b.0 - a.0) * (y - a.1) - (x - a.0) * (b.1 - a.1);
        if a.1 <= y && b.1 > y && cross > 0.0 {
            winding += 1;
        }
        if a.1 > y && b.1 <= y && cross < 0.0 {
            winding -= 1;
        }
    }
    (winding, edge)
}

impl PlacedShape {
    /// Hit testing uses the same centre rotation and flips as the renderer.
    /// Tolerance is in slide pixels, typically a few screen pixels / zoom.
    pub fn contains_point(&self, x: f64, y: f64, tolerance: f64) -> bool {
        let Some((x, y)) = self.slide_to_paint((x, y)) else {
            return false;
        };
        let scale = self.paint_transform().scale;
        let tolerance = tolerance / scale.0.abs().min(scale.1.abs());
        let r = self.paint_rect();
        if [
            x,
            y,
            tolerance,
            r.x,
            r.y,
            r.width,
            r.height,
            self.rotation_rad,
        ]
        .iter()
        .any(|v| !v.is_finite())
            || r.width < 0.0
            || r.height < 0.0
        {
            return false;
        }
        let (x, y) = (x - r.x, y - r.y);
        let tolerance = tolerance.max(0.0);
        match &self.geometry {
            PlacedGeometry::Line => {
                segment_distance(x, y, (0.0, 0.0), (r.width, r.height)) <= tolerance.max(0.5)
            }
            PlacedGeometry::Polygon(points) => {
                let (winding, edge) = polygon_winding(points, x, y, r.width, r.height, tolerance);
                winding != 0 || edge
            }
            PlacedGeometry::Path(paths) => {
                let (winding, edge) = paths
                    .iter()
                    .map(|points| polygon_winding(points, x, y, r.width, r.height, tolerance))
                    .fold((0, false), |a, b| (a.0 + b.0, a.1 || b.1));
                winding != 0 || edge
            }
            PlacedGeometry::Ellipse => {
                let (rx, ry) = (r.width / 2.0 + tolerance, r.height / 2.0 + tolerance);
                rx > 0.0
                    && ry > 0.0
                    && ((x - r.width / 2.0) / rx).powi(2) + ((y - r.height / 2.0) / ry).powi(2)
                        <= 1.0
            }
            PlacedGeometry::RoundRect { adjust } => {
                if x < -tolerance
                    || x > r.width + tolerance
                    || y < -tolerance
                    || y > r.height + tolerance
                {
                    return false;
                }
                let radius = (r.width.min(r.height) * adjust.clamp(0.0, 0.5)).max(0.0);
                let cx = x.clamp(radius, (r.width - radius).max(radius));
                let cy = y.clamp(radius, (r.height - radius).max(radius));
                (x - cx).hypot(y - cy) <= radius + tolerance
            }
            PlacedGeometry::Rect => {
                x >= -tolerance
                    && x <= r.width + tolerance
                    && y >= -tolerance
                    && y <= r.height + tolerance
            }
        }
    }
}
