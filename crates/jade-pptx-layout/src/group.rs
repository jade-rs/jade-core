use crate::{emu_to_px_f, ObjectTransform, PlacedShape, Rect};
use jade_pptx_reader::GroupTransform;
use serde::{Deserialize, Serialize};

/// A group's child coordinate space mapped into its parent's pixel space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedGroupTransform {
    pub frame: Rect,
    pub child_frame: Rect,
    pub rotation_rad: f64,
    pub flip_h: bool,
    pub flip_v: bool,
}
impl From<&GroupTransform> for PlacedGroupTransform {
    fn from(value: &GroupTransform) -> Self {
        let rect = |off: (i64, i64), ext: (i64, i64)| Rect {
            x: emu_to_px_f(off.0 as f64),
            y: emu_to_px_f(off.1 as f64),
            width: emu_to_px_f(ext.0 as f64),
            height: emu_to_px_f(ext.1 as f64),
        };
        Self {
            frame: rect(value.outer.offset_emu, value.outer.extent_emu),
            child_frame: rect(value.child_offset_emu, value.child_extent_emu),
            rotation_rad: crate::rotation_rad(&value.outer),
            flip_h: value.outer.flip_h,
            flip_v: value.outer.flip_v,
        }
    }
}
impl PlacedGroupTransform {
    pub fn scale(&self) -> (f64, f64) {
        let axis = |outer: f64, child: f64| if child == 0.0 { 1.0 } else { outer / child };
        (
            axis(self.frame.width, self.child_frame.width),
            axis(self.frame.height, self.child_frame.height),
        )
    }
    pub fn apply(&self, point: (f64, f64)) -> (f64, f64) {
        let (sx, sy) = self.scale();
        let c = self.child_frame;
        let r = self.frame;
        let x = (point.0 - c.x - c.width / 2.0) * sx * if self.flip_h { -1.0 } else { 1.0 };
        let y = (point.1 - c.y - c.height / 2.0) * sy * if self.flip_v { -1.0 } else { 1.0 };
        let (sin, cos) = self.rotation_rad.sin_cos();
        (
            r.x + r.width / 2.0 + cos * x - sin * y,
            r.y + r.height / 2.0 + sin * x + cos * y,
        )
    }
    pub fn inverse(&self, point: (f64, f64)) -> Option<(f64, f64)> {
        let (sx, sy) = self.scale();
        if sx == 0.0 || sy == 0.0 {
            return None;
        }
        let r = self.frame;
        let c = self.child_frame;
        let (x, y) = (
            point.0 - r.x - r.width / 2.0,
            point.1 - r.y - r.height / 2.0,
        );
        let (sin, cos) = self.rotation_rad.sin_cos();
        let point = (
            (cos * x + sin * y) * if self.flip_h { -1.0 } else { 1.0 } / sx + c.x + c.width / 2.0,
            (-sin * x + cos * y) * if self.flip_v { -1.0 } else { 1.0 } / sy + c.y + c.height / 2.0,
        );
        [point.0, point.1]
            .iter()
            .all(|v| v.is_finite())
            .then_some(point)
    }
    pub fn object_transform(&self) -> ObjectTransform {
        ObjectTransform {
            rect: self.frame,
            rotation_degrees: self.rotation_rad.to_degrees(),
            flip_h: self.flip_h,
            flip_v: self.flip_v,
        }
    }
}

impl PlacedShape {
    pub fn paint_rect(&self) -> Rect {
        self.local_rect.unwrap_or(self.rect)
    }
    /// DrawingML uses cumulative axis scales/flips and rotation, with a mapped centre.
    pub fn paint_transform(&self) -> PlacedPaintTransform {
        let r = self.paint_rect();
        let mut center = (r.x + r.width / 2.0, r.y + r.height / 2.0);
        for group in self.group_transforms.iter().rev() {
            center = group.apply(center);
        }
        let mut scale = (
            if self.flip_h { -1.0 } else { 1.0 },
            if self.flip_v { -1.0 } else { 1.0 },
        );
        let mut rotation_rad = self.rotation_rad;
        for group in &self.group_transforms {
            let (x, y) = group.scale();
            scale.0 *= x * if group.flip_h { -1.0 } else { 1.0 };
            scale.1 *= y * if group.flip_v { -1.0 } else { 1.0 };
            rotation_rad += group.rotation_rad;
        }
        PlacedPaintTransform {
            center,
            scale,
            rotation_rad,
        }
    }
    pub fn paint_to_slide(&self, point: (f64, f64)) -> (f64, f64) {
        let t = self.paint_transform();
        let r = self.paint_rect();
        let x = (point.0 - r.x - r.width / 2.0) * t.scale.0;
        let y = (point.1 - r.y - r.height / 2.0) * t.scale.1;
        let (sin, cos) = t.rotation_rad.sin_cos();
        (
            t.center.0 + cos * x - sin * y,
            t.center.1 + sin * x + cos * y,
        )
    }
    pub fn slide_to_paint(&self, point: (f64, f64)) -> Option<(f64, f64)> {
        let t = self.paint_transform();
        let r = self.paint_rect();
        if t.scale.0 == 0.0 || t.scale.1 == 0.0 {
            return None;
        }
        let (x, y) = (point.0 - t.center.0, point.1 - t.center.1);
        let (sin, cos) = t.rotation_rad.sin_cos();
        let point = (
            (cos * x + sin * y) / t.scale.0 + r.x + r.width / 2.0,
            (-sin * x + cos * y) / t.scale.1 + r.y + r.height / 2.0,
        );
        [point.0, point.1]
            .iter()
            .all(|v| v.is_finite())
            .then_some(point)
    }
    pub(crate) fn enter_group(&mut self, group: PlacedGroupTransform, id: Option<u32>) {
        self.local_rect = Some(self.paint_rect());
        self.group_transforms.insert(0, group);
        if let Some(id) = id {
            self.group_ids.insert(0, id);
        }
        let r = self.paint_rect();
        let points = [
            (r.x, r.y),
            (r.x + r.width, r.y),
            (r.x + r.width, r.y + r.height),
            (r.x, r.y + r.height),
        ]
        .map(|point| self.paint_to_slide(point));
        let x = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
        let y = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        self.rect = Rect {
            x,
            y,
            width: points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max) - x,
            height: points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max) - y,
        };
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedPaintTransform {
    pub center: (f64, f64),
    pub scale: (f64, f64),
    pub rotation_rad: f64,
}
