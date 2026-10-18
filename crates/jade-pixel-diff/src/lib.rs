//! Region-level pixel diff of two renders of the same page.
//!
//! Glyph-position checks only see where characters land. Baselines, grid
//! placement, borders, ruby, emphasis marks and shapes need a picture
//! comparison, but a raw per-pixel diff drowns in anti-aliasing noise. This
//! crate composites both images on white, measures each pixel's CIE76 ΔE in
//! CIELAB, counts the pixels above a threshold in fixed-size cells, and merges
//! the cells with enough of them into rectangular regions. The number of
//! regions is the figure a calibration document tracks before and after a
//! layout fix.
//!
//! The expected image is typically Word's PDF rasterised at the same DPI as
//! jade's render (for example `pdftoppm -r 96 -png`); this crate only compares
//! the two PNGs. Word output stays out of the repository, as elsewhere.

use serde::Serialize;

/// An 8-bit RGBA image, row-major, four bytes per pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl RgbaImage {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, DiffError> {
        if pixels.len() != width as usize * height as usize * 4 {
            return Err(DiffError::Format(format!(
                "{width}x{height} RGBA needs {} bytes, got {}",
                width as usize * height as usize * 4,
                pixels.len()
            )));
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// A `width` × `height` image of one colour.
    pub fn filled(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        let pixels = rgba
            .iter()
            .copied()
            .cycle()
            .take(width as usize * height as usize * 4)
            .collect();
        Self {
            width,
            height,
            pixels,
        }
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let index = (y as usize * self.width as usize + x as usize) * 4;
        [
            self.pixels[index],
            self.pixels[index + 1],
            self.pixels[index + 2],
            self.pixels[index + 3],
        ]
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        let index = (y as usize * self.width as usize + x as usize) * 4;
        self.pixels[index..index + 4].copy_from_slice(&rgba);
    }

    /// Decodes a PNG of any colour type and bit depth into 8-bit RGBA.
    pub fn decode_png(bytes: &[u8]) -> Result<Self, DiffError> {
        let mut decoder = png::Decoder::new(bytes);
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder
            .read_info()
            .map_err(|error| DiffError::Format(error.to_string()))?;
        let mut buffer = vec![0; reader.output_buffer_size()];
        let info = reader
            .next_frame(&mut buffer)
            .map_err(|error| DiffError::Format(error.to_string()))?;
        let samples = &buffer[..info.buffer_size()];
        let pixels = match info.color_type {
            png::ColorType::Rgba => samples.to_vec(),
            png::ColorType::Rgb => samples
                .chunks_exact(3)
                .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 255])
                .collect(),
            png::ColorType::GrayscaleAlpha => samples
                .chunks_exact(2)
                .flat_map(|ga| [ga[0], ga[0], ga[0], ga[1]])
                .collect(),
            png::ColorType::Grayscale => samples.iter().flat_map(|&g| [g, g, g, 255]).collect(),
            png::ColorType::Indexed => {
                return Err(DiffError::Format(
                    "indexed PNG was not expanded".to_string(),
                ))
            }
        };
        Self::new(info.width, info.height, pixels)
    }

    pub fn encode_png(&self) -> Result<Vec<u8>, DiffError> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, self.width, self.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder
                .write_header()
                .map_err(|error| DiffError::Format(error.to_string()))?;
            writer
                .write_image_data(&self.pixels)
                .map_err(|error| DiffError::Format(error.to_string()))?;
        }
        Ok(bytes)
    }
}

#[derive(Debug)]
pub enum DiffError {
    Format(String),
}

impl std::fmt::Display for DiffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Format(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for DiffError {}

/// Thresholds of a comparison.
///
/// The defaults are provisional: they tolerate grey-scale text
/// anti-aliasing between two rasterisers but have not yet been tuned against
/// Word PDFs (jlsi/jade#475). Pass explicit values when a gate depends on them.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct DiffOptions {
    /// A pixel differs when its CIE76 ΔE exceeds this. ΔE ≈ 2.3 is a just
    /// noticeable difference; anti-aliased glyph edges reach far beyond it.
    pub delta_e_threshold: f32,
    /// Side of the square cells differing pixels are counted in, in pixels.
    pub cell_size: u32,
    /// A cell is part of a region when at least this many of its pixels differ.
    pub min_cell_pixels: u32,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            delta_e_threshold: 10.0,
            cell_size: 8,
            min_cell_pixels: 6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

/// A rectangle of differing cells, in pixels of the compared area.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiffRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Pixels above the threshold inside the region.
    pub diff_pixels: u64,
    /// Mean ΔE of those pixels.
    pub mean_delta_e: f32,
    pub max_delta_e: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiffReport {
    pub expected: Size,
    pub actual: Size,
    /// The area compared: the larger of the two images. Pixels only one image
    /// covers count as differing with the maximum ΔE of 100.
    pub compared: Size,
    pub options: DiffOptions,
    /// Pixels above the threshold anywhere, including outside regions.
    pub diff_pixels: u64,
    /// `diff_pixels` over the compared area.
    pub diff_ratio: f64,
    /// Mean ΔE of the pixels above the threshold (0 when there are none).
    pub mean_delta_e: f32,
    /// Largest ΔE of any pixel.
    pub max_delta_e: f32,
    pub regions: Vec<DiffRegion>,
}

impl DiffReport {
    pub fn is_identical(&self) -> bool {
        self.regions.is_empty()
    }
}

/// ΔE of a pixel only one of the two images covers.
const OUT_OF_BOUNDS_DELTA_E: f32 = 100.0;

/// Compares `expected` with `actual` and returns the per-pixel ΔE map along
/// with the report, so a caller can draw it ([`render_diff`]).
pub fn diff_images_with_map(
    expected: &RgbaImage,
    actual: &RgbaImage,
    options: DiffOptions,
) -> (DiffReport, Vec<f32>) {
    let width = expected.width.max(actual.width);
    let height = expected.height.max(actual.height);
    let cell_size = options.cell_size.max(1);
    let lab = LabConverter::new();

    let mut delta_map = vec![0.0f32; width as usize * height as usize];
    let mut diff_pixels = 0u64;
    let mut delta_sum = 0.0f64;
    let mut max_delta_e = 0.0f32;
    for y in 0..height {
        for x in 0..width {
            let inside_expected = x < expected.width && y < expected.height;
            let inside_actual = x < actual.width && y < actual.height;
            let delta = if inside_expected && inside_actual {
                delta_e76(
                    lab.convert(expected.pixel(x, y)),
                    lab.convert(actual.pixel(x, y)),
                )
            } else {
                OUT_OF_BOUNDS_DELTA_E
            };
            delta_map[y as usize * width as usize + x as usize] = delta;
            max_delta_e = max_delta_e.max(delta);
            if delta > options.delta_e_threshold {
                diff_pixels += 1;
                delta_sum += f64::from(delta);
            }
        }
    }

    let columns = width.div_ceil(cell_size);
    let rows = height.div_ceil(cell_size);
    let mut cell_counts = vec![0u32; columns as usize * rows as usize];
    for y in 0..height {
        for x in 0..width {
            if delta_map[y as usize * width as usize + x as usize] > options.delta_e_threshold {
                cell_counts[(y / cell_size * columns + x / cell_size) as usize] += 1;
            }
        }
    }
    let flagged: Vec<bool> = cell_counts
        .iter()
        .map(|&count| count > 0 && count >= options.min_cell_pixels)
        .collect();

    let regions = connected_cell_regions(&flagged, columns, rows)
        .into_iter()
        .map(|(column0, row0, column1, row1)| {
            let x = column0 * cell_size;
            let y = row0 * cell_size;
            let right = ((column1 + 1) * cell_size).min(width);
            let bottom = ((row1 + 1) * cell_size).min(height);
            let mut region_pixels = 0u64;
            let mut region_sum = 0.0f64;
            let mut region_max = 0.0f32;
            for py in y..bottom {
                for px in x..right {
                    let delta = delta_map[py as usize * width as usize + px as usize];
                    region_max = region_max.max(delta);
                    if delta > options.delta_e_threshold {
                        region_pixels += 1;
                        region_sum += f64::from(delta);
                    }
                }
            }
            DiffRegion {
                x,
                y,
                width: right - x,
                height: bottom - y,
                diff_pixels: region_pixels,
                mean_delta_e: mean(region_sum, region_pixels),
                max_delta_e: region_max,
            }
        })
        .collect();

    let area = u64::from(width) * u64::from(height);
    let report = DiffReport {
        expected: Size {
            width: expected.width,
            height: expected.height,
        },
        actual: Size {
            width: actual.width,
            height: actual.height,
        },
        compared: Size { width, height },
        options,
        diff_pixels,
        diff_ratio: if area == 0 {
            0.0
        } else {
            diff_pixels as f64 / area as f64
        },
        mean_delta_e: mean(delta_sum, diff_pixels),
        max_delta_e,
        regions,
    };
    (report, delta_map)
}

/// Compares `expected` with `actual`.
pub fn diff_images(expected: &RgbaImage, actual: &RgbaImage, options: DiffOptions) -> DiffReport {
    diff_images_with_map(expected, actual, options).0
}

/// A picture of a comparison: `actual` washed out to light grey, differing
/// pixels in red, and each region outlined in blue.
pub fn render_diff(
    actual: &RgbaImage,
    report: &DiffReport,
    delta_map: &[f32],
    options: DiffOptions,
) -> RgbaImage {
    let Size { width, height } = report.compared;
    let mut image = RgbaImage::filled(width, height, [255, 255, 255, 255]);
    for y in 0..height {
        for x in 0..width {
            let delta = delta_map[y as usize * width as usize + x as usize];
            let pixel = if delta > options.delta_e_threshold {
                [220, 0, 0, 255]
            } else if x < actual.width && y < actual.height {
                let [r, g, b] = composite_on_white(actual.pixel(x, y));
                let luma = (0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b))
                    .round() as u8;
                let faded = 255 - (255 - luma) / 3;
                [faded, faded, faded, 255]
            } else {
                continue;
            };
            image.set_pixel(x, y, pixel);
        }
    }
    for region in &report.regions {
        let right = region.x + region.width - 1;
        let bottom = region.y + region.height - 1;
        for x in region.x..=right {
            image.set_pixel(x, region.y, [0, 90, 255, 255]);
            image.set_pixel(x, bottom, [0, 90, 255, 255]);
        }
        for y in region.y..=bottom {
            image.set_pixel(region.x, y, [0, 90, 255, 255]);
            image.set_pixel(right, y, [0, 90, 255, 255]);
        }
    }
    image
}

fn mean(sum: f64, count: u64) -> f32 {
    if count == 0 {
        0.0
    } else {
        (sum / count as f64) as f32
    }
}

/// Bounding boxes `(column0, row0, column1, row1)` of the 8-connected groups of
/// flagged cells, in reading order of their first cell.
fn connected_cell_regions(flagged: &[bool], columns: u32, rows: u32) -> Vec<(u32, u32, u32, u32)> {
    let mut visited = vec![false; flagged.len()];
    let mut regions = Vec::new();
    let mut stack = Vec::new();
    for start in 0..flagged.len() {
        if !flagged[start] || visited[start] {
            continue;
        }
        visited[start] = true;
        stack.push(start);
        let (mut column0, mut row0) = (u32::MAX, u32::MAX);
        let (mut column1, mut row1) = (0, 0);
        while let Some(index) = stack.pop() {
            let column = index as u32 % columns;
            let row = index as u32 / columns;
            column0 = column0.min(column);
            row0 = row0.min(row);
            column1 = column1.max(column);
            row1 = row1.max(row);
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let (nx, ny) = (i64::from(column) + dx, i64::from(row) + dy);
                    if nx < 0 || ny < 0 || nx >= i64::from(columns) || ny >= i64::from(rows) {
                        continue;
                    }
                    let neighbour = (ny * i64::from(columns) + nx) as usize;
                    if flagged[neighbour] && !visited[neighbour] {
                        visited[neighbour] = true;
                        stack.push(neighbour);
                    }
                }
            }
        }
        regions.push((column0, row0, column1, row1));
    }
    regions
}

fn composite_on_white([r, g, b, a]: [u8; 4]) -> [u8; 3] {
    if a == 255 {
        return [r, g, b];
    }
    let alpha = u32::from(a);
    let blend =
        |channel: u8| ((u32::from(channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
    [blend(r), blend(g), blend(b)]
}

fn delta_e76(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dl = a[0] - b[0];
    let da = a[1] - b[1];
    let db = a[2] - b[2];
    (dl * dl + da * da + db * db).sqrt()
}

/// sRGB (D65) → CIELAB, with the sRGB transfer curve tabulated.
struct LabConverter {
    linear: [f32; 256],
}

impl LabConverter {
    fn new() -> Self {
        let mut linear = [0.0f32; 256];
        for (value, slot) in linear.iter_mut().enumerate() {
            let c = value as f32 / 255.0;
            *slot = if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            };
        }
        Self { linear }
    }

    fn convert(&self, rgba: [u8; 4]) -> [f32; 3] {
        let [r, g, b] = composite_on_white(rgba);
        let (r, g, b) = (
            self.linear[r as usize],
            self.linear[g as usize],
            self.linear[b as usize],
        );
        // D65 reference white.
        let x = (0.412_456_4 * r + 0.357_576_1 * g + 0.180_437_5 * b) / 0.950_47;
        let y = 0.212_672_9 * r + 0.715_152_2 * g + 0.072_175 * b;
        let z = (0.019_333_9 * r + 0.119_192 * g + 0.950_304_1 * b) / 1.088_83;
        let f = |t: f32| {
            if t > 216.0 / 24389.0 {
                t.cbrt()
            } else {
                (24389.0 / 27.0 * t + 16.0) / 116.0
            }
        };
        let (fx, fy, fz) = (f(x), f(y), f(z));
        [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const BLACK: [u8; 4] = [0, 0, 0, 255];

    fn fill_rect(image: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, rgba: [u8; 4]) {
        for py in y..y + h {
            for px in x..x + w {
                image.set_pixel(px, py, rgba);
            }
        }
    }

    #[test]
    fn lab_conversion_matches_reference_values() {
        let lab = LabConverter::new();
        let white = lab.convert(WHITE);
        assert!((white[0] - 100.0).abs() < 0.01 && white[1].abs() < 0.01 && white[2].abs() < 0.01);
        let black = lab.convert(BLACK);
        assert!(black[0].abs() < 0.01);
        // sRGB red is L*a*b* (53.24, 80.09, 67.20).
        let red = lab.convert([255, 0, 0, 255]);
        assert!((red[0] - 53.24).abs() < 0.05, "{red:?}");
        assert!((red[1] - 80.09).abs() < 0.1, "{red:?}");
        assert!((red[2] - 67.20).abs() < 0.1, "{red:?}");
        // A transparent pixel is the white page underneath it.
        assert_eq!(lab.convert([0, 0, 0, 0]), white);
        assert!((delta_e76(white, black) - 100.0).abs() < 0.01);
    }

    #[test]
    fn identical_images_have_no_regions() {
        let mut image = RgbaImage::filled(40, 30, WHITE);
        fill_rect(&mut image, 5, 5, 10, 3, BLACK);
        let report = diff_images(&image, &image.clone(), DiffOptions::default());
        assert!(report.is_identical());
        assert_eq!(report.diff_pixels, 0);
        assert_eq!(report.max_delta_e, 0.0);
    }

    #[test]
    fn faint_and_sparse_differences_stay_below_the_region_thresholds() {
        let expected = RgbaImage::filled(32, 32, WHITE);
        let mut actual = expected.clone();
        // A light grey wash is below the ΔE threshold everywhere.
        fill_rect(&mut actual, 0, 0, 32, 16, [250, 250, 250, 255]);
        // Anti-aliasing-like specks: a few strong pixels, too few per cell.
        for (x, y) in [(20, 20), (21, 28), (30, 18)] {
            actual.set_pixel(x, y, BLACK);
        }
        let report = diff_images(&expected, &actual, DiffOptions::default());
        assert!(report.is_identical(), "{report:?}");
        assert_eq!(report.diff_pixels, 3);
        assert!((report.max_delta_e - 100.0).abs() < 0.01);
    }

    #[test]
    fn a_moved_line_becomes_one_region_covering_both_positions() {
        let mut expected = RgbaImage::filled(64, 48, WHITE);
        let mut actual = expected.clone();
        fill_rect(&mut expected, 8, 10, 40, 2, BLACK);
        fill_rect(&mut actual, 8, 13, 40, 2, BLACK);
        // An unrelated box far away is its own region.
        fill_rect(&mut actual, 56, 40, 6, 6, [0, 0, 255, 255]);

        let report = diff_images(&expected, &actual, DiffOptions::default());
        assert_eq!(report.regions.len(), 2, "{:?}", report.regions);
        let line = &report.regions[0];
        assert_eq!((line.x, line.y, line.width, line.height), (8, 8, 40, 8));
        assert_eq!(line.diff_pixels, 160);
        assert!((line.mean_delta_e - 100.0).abs() < 0.01);
        let square = &report.regions[1];
        assert_eq!(
            (square.x, square.y, square.width, square.height),
            (56, 40, 8, 8)
        );
        assert_eq!(report.diff_pixels, 196);
        assert!((report.diff_ratio - 196.0 / (64.0 * 48.0)).abs() < 1e-9);
    }

    #[test]
    fn size_mismatch_counts_the_uncovered_area_as_different() {
        let expected = RgbaImage::filled(16, 16, WHITE);
        let actual = RgbaImage::filled(16, 24, WHITE);
        let report = diff_images(&expected, &actual, DiffOptions::default());
        assert_eq!(
            report.compared,
            Size {
                width: 16,
                height: 24
            }
        );
        assert_eq!(report.diff_pixels, 16 * 8);
        assert_eq!(report.regions.len(), 1);
        let region = &report.regions[0];
        assert_eq!(
            (region.x, region.y, region.width, region.height),
            (0, 16, 16, 8)
        );
    }

    #[test]
    fn png_round_trip_and_diff_picture() {
        let mut expected = RgbaImage::filled(16, 16, WHITE);
        let actual = expected.clone();
        fill_rect(&mut expected, 0, 0, 8, 8, BLACK);
        let decoded = RgbaImage::decode_png(&expected.encode_png().unwrap()).unwrap();
        assert_eq!(decoded, expected);

        let options = DiffOptions::default();
        let (report, map) = diff_images_with_map(&decoded, &actual, options);
        let picture = render_diff(&actual, &report, &map, options);
        assert_eq!(picture.pixel(0, 0), [0, 90, 255, 255], "region outline");
        assert_eq!(picture.pixel(3, 3), [220, 0, 0, 255], "differing pixel");
        assert_eq!(picture.pixel(12, 12), WHITE, "unchanged white page");
    }
}
