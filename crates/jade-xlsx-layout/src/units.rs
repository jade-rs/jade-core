//! The unit conversions the format needs.
//!
//! Column widths are the awkward one: xlsx stores them in "characters", a unit
//! defined against the width of a digit in the workbook's default font, so the
//! same stored width is a different number of pixels in a workbook whose
//! default font differs.

/// Width of a digit in pixels for Calibri 11 at 96 dpi — the default Excel
/// itself assumes, and the reference point for the stored width unit.
pub const DEFAULT_MAX_DIGIT_WIDTH_PX: f64 = 7.0;

/// Encoded default column width (64px at MDW=7); its UI count is 8.43.
pub const DEFAULT_COLUMN_WIDTH_CHARS: f32 = 9.140625;

/// Excel's default row height, in points.
pub const DEFAULT_ROW_HEIGHT_POINTS: f32 = 15.0;

/// Cell padding baked into a column's pixel width: two pixels of margin on
/// each side plus the grid line.
pub const COLUMN_PADDING_PX: f64 = 5.0;

fn digit_width(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        DEFAULT_MAX_DIGIT_WIDTH_PX
    }
}

/// SpreadsheetML width already includes padding, in 1/256 digit units.
/// ISO 29500: truncate(((256*width + truncate(128/MDW))/256)*MDW).
pub fn column_width_chars_to_px(width_chars: f32, max_digit_width_px: f64) -> f64 {
    if !width_chars.is_finite() || width_chars <= 0.0 {
        return 0.0;
    }
    let mdw = digit_width(max_digit_width_px);
    (((256.0 * f64::from(width_chars) + (128.0 / mdw).floor()) / 256.0) * mdw).floor()
}

/// Pixel-aligned sizing converted to SpreadsheetML's encoded width.
/// Padding is already part of `pixels`; it must not be removed here.
pub fn column_width_px_to_chars(pixels: f64, max_digit_width_px: f64) -> f32 {
    if !pixels.is_finite() {
        return 0.0;
    }
    let mdw = digit_width(max_digit_width_px);
    let pixels = pixels.max(0.0).round();
    let mut ticks = (pixels / mdw * 256.0).floor();
    if column_width_chars_to_px((ticks / 256.0) as f32, mdw) < pixels {
        ticks += 1.0;
    }
    (ticks / 256.0).clamp(0.0, 255.0) as f32
}

/// Excel's displayed character count is separate from the encoded file value.
pub fn displayed_column_width_to_stored(characters: f32, max_digit_width_px: f64) -> f32 {
    if !characters.is_finite() {
        return 0.0;
    }
    let mdw = digit_width(max_digit_width_px);
    let characters = f64::from(characters).max(0.0);
    let pixels = if characters < 1.0 {
        (characters * (mdw + COLUMN_PADDING_PX)).round()
    } else {
        (characters * mdw).round() + COLUMN_PADDING_PX
    };
    column_width_px_to_chars(pixels, mdw)
}

pub fn stored_column_width_to_displayed(width: f32, max_digit_width_px: f64) -> f32 {
    let mdw = digit_width(max_digit_width_px);
    let pixels = column_width_chars_to_px(width, mdw);
    let characters = if pixels < mdw + COLUMN_PADDING_PX {
        pixels / (mdw + COLUMN_PADDING_PX)
    } else {
        (pixels - COLUMN_PADDING_PX) / mdw
    };
    ((characters * 100.0 + 0.5).floor() / 100.0).max(0.0) as f32
}

/// Points to CSS pixels at 96 dpi, which is how row heights are stored.
pub fn points_to_px(points: f32) -> f64 {
    f64::from(points) * 96.0 / 72.0
}

/// Excel's default font size, the one its default row height is derived from.
pub const DEFAULT_FONT_SIZE_POINTS: f32 = 11.0;

/// Height of one line of text at `font_size_points`, in points.
///
/// Anchored on Excel's own defaults rather than a guessed ratio: a single line
/// of the default 11 pt font occupies exactly the default 15 pt row, so the
/// ratio is 15/11 and an unwrapped default row keeps the height the file says.
/// A real implementation would take ascent, descent and line gap from the font;
/// this is proportional to the size instead, so an unusual face wraps to the
/// right number of lines but may sit a little loose or tight.
pub fn line_height_points(font_size_points: f32) -> f32 {
    font_size_points * (DEFAULT_ROW_HEIGHT_POINTS / DEFAULT_FONT_SIZE_POINTS)
}
