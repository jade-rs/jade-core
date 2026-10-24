//! The handles round a selected picture or shape, as Word draws them —
//! the eight sizing handles, and in トリミング the crop handles — and what
//! dragging one does to the box (and, cropping, to the picture's crop):
//! shared by the native and the browser shells.

pub use crate::document::ImageCrop;

/// Where a handle sits on an object's box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl Handle {
    pub const ALL: [Handle; 8] = [
        Handle::TopLeft,
        Handle::Top,
        Handle::TopRight,
        Handle::Right,
        Handle::BottomRight,
        Handle::Bottom,
        Handle::BottomLeft,
        Handle::Left,
    ];

    /// Its place on a box, as fractions of the width and height.
    pub fn anchor(self) -> (f64, f64) {
        match self {
            Handle::TopLeft => (0.0, 0.0),
            Handle::Top => (0.5, 0.0),
            Handle::TopRight => (1.0, 0.0),
            Handle::Right => (1.0, 0.5),
            Handle::BottomRight => (1.0, 1.0),
            Handle::Bottom => (0.5, 1.0),
            Handle::BottomLeft => (0.0, 1.0),
            Handle::Left => (0.0, 0.5),
        }
    }

    pub fn is_corner(self) -> bool {
        matches!(
            self,
            Handle::TopLeft | Handle::TopRight | Handle::BottomRight | Handle::BottomLeft
        )
    }
}

/// A box sized from one of its handles by `(dx, dy)`; `keep_ratio` keeps
/// its proportions.
pub fn resize(
    (x, y, width, height): (f64, f64, f64, f64),
    handle: Handle,
    (dx, dy): (f64, f64),
    keep_ratio: bool,
) -> (f64, f64, f64, f64) {
    const MIN: f64 = 4.0;
    let (fx, fy) = handle.anchor();
    let (mut left, mut top, mut right, mut bottom) = (x, y, x + width, y + height);
    if fx == 0.0 {
        left = (left + dx).min(right - MIN);
    } else if fx == 1.0 {
        right = (right + dx).max(left + MIN);
    }
    if fy == 0.0 {
        top = (top + dy).min(bottom - MIN);
    } else if fy == 1.0 {
        bottom = (bottom + dy).max(top + MIN);
    }
    if keep_ratio && handle.is_corner() && width > 0.0 && height > 0.0 {
        let ratio = width / height;
        let (new_width, new_height) = (right - left, bottom - top);
        // The larger change wins; the other follows.
        let (new_width, new_height) = if (new_width / width) > (new_height / height) {
            (new_width, new_width / ratio)
        } else {
            (new_height * ratio, new_height)
        };
        if fx == 0.0 {
            left = right - new_width;
        } else {
            right = left + new_width;
        }
        if fy == 0.0 {
            top = bottom - new_height;
        } else {
            bottom = top + new_height;
        }
    }
    (left, top, right - left, bottom - top)
}

/// A crop handle dragged by `(dx, dy)`: the new crop of the picture and the
/// visible part's new box. The picture itself does not move or scale: the
/// handle cuts into it (or, outward, back out of what was cut).
pub fn crop_drag(
    (x, y, width, height): (f64, f64, f64, f64),
    crop: ImageCrop,
    handle: Handle,
    (dx, dy): (f64, f64),
) -> (ImageCrop, (f64, f64, f64, f64)) {
    let visible_x = (1.0 - crop.left - crop.right).max(0.001);
    let visible_y = (1.0 - crop.top - crop.bottom).max(0.001);
    // The whole picture's size on the page.
    let (full_width, full_height) = (width / visible_x, height / visible_y);
    let (fx, fy) = handle.anchor();
    let mut crop = crop;
    let (mut left, mut top, mut right, mut bottom) = (x, y, x + width, y + height);
    if fx == 0.0 {
        let cut = (crop.left + dx / full_width).clamp(-1.0, 1.0 - crop.right - 0.02);
        left = x - (crop.left - cut) * full_width;
        crop.left = cut;
    } else if fx == 1.0 {
        let cut = (crop.right - dx / full_width).clamp(-1.0, 1.0 - crop.left - 0.02);
        right = x + width + (crop.right - cut) * full_width;
        crop.right = cut;
    }
    if fy == 0.0 {
        let cut = (crop.top + dy / full_height).clamp(-1.0, 1.0 - crop.bottom - 0.02);
        top = y - (crop.top - cut) * full_height;
        crop.top = cut;
    } else if fy == 1.0 {
        let cut = (crop.bottom - dy / full_height).clamp(-1.0, 1.0 - crop.top - 0.02);
        bottom = y + height + (crop.bottom - cut) * full_height;
        crop.bottom = cut;
    }
    (crop, (left, top, right - left, bottom - top))
}
