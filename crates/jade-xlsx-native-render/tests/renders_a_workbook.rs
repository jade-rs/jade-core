//! The native half of the spreadsheet path: a workbook opened from bytes and
//! painted onto a Skia surface, with no browser anywhere.

use std::path::PathBuf;

use jade_xlsx_native_render::{aliased_faces, bundled_font_paths, NativeWorkbook};

fn repository_root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn open(fixture: &str) -> NativeWorkbook {
    let bytes =
        std::fs::read(repository_root().join("public/fixtures").join(fixture)).expect("fixture");
    let (regular, bold) = bundled_font_paths(&repository_root());
    NativeWorkbook::open(&bytes, &aliased_faces(&regular, &bold)).expect("open")
}

/// Pixels that are not the sheet background, which is what "something was
/// drawn here" means for a grid that is mostly white.
fn ink(pixels: &[u8]) -> usize {
    pixels
        .chunks_exact(4)
        .filter(|pixel| pixel[0] < 250 || pixel[1] < 250 || pixel[2] < 250)
        .count()
}

/// The same, counting only the sheet itself.
///
/// The header bands are drawn whatever the sheet holds, so a count over the
/// whole surface has a floor that hides the difference between a busy sheet and
/// an almost empty one.
fn body_ink(workbook: &NativeWorkbook, pixels: &[u8], width: i32) -> usize {
    let metrics = workbook.header_metrics();
    let first_row = metrics.column_height.ceil() as usize;
    let first_column = metrics.row_width.ceil() as usize;
    pixels
        .chunks_exact(width as usize * 4)
        .skip(first_row)
        .flat_map(|row| row.chunks_exact(4).skip(first_column))
        .filter(|pixel| pixel[0] < 250 || pixel[1] < 250 || pixel[2] < 250)
        .count()
}

#[test]
fn a_workbook_opens_and_lists_its_sheets() {
    let workbook = open("sample-workbook.xlsx");
    assert_eq!(workbook.sheet_names(), ["売上サマリ", "Notes"]);
    assert_eq!(workbook.sheet_index(), 0);

    let (width, height) = workbook.sheet_size();
    assert!(width > 0.0 && height > 0.0);
}

#[test]
fn a_viewport_comes_back_as_bgra_of_the_size_asked_for() {
    let mut workbook = open("sample-workbook.xlsx");
    let (pixels, width, height) = workbook
        .viewport_pixels((0.0, 0.0), (320, 200))
        .expect("pixels");

    assert_eq!((width, height), (320, 200));
    assert_eq!(pixels.len(), 320 * 200 * 4);
    assert!(ink(&pixels) > 0, "the top of the sheet should have drawn");
}

#[test]
fn scrolling_paints_a_different_part_of_the_sheet() {
    let mut workbook = open("large-workbook.xlsx");
    let (top, ..) = workbook
        .viewport_pixels((0.0, 0.0), (320, 200))
        .expect("top");
    let (far, ..) = workbook
        .viewport_pixels((0.0, 50_000.0), (320, 200))
        .expect("far");

    assert_ne!(top, far);
    assert!(ink(&far) > 0, "row 2000 should have drawn something");
}

#[test]
fn a_sheet_taller_than_any_surface_still_paints_a_window() {
    let mut workbook = open("large-workbook.xlsx");
    let (_, height) = workbook.sheet_size();
    // The whole sheet is far past what a browser canvas would allocate, which
    // is the reason the viewport exists on both shells rather than only one.
    assert!(height > 100_000.0);

    let (pixels, _, _) = workbook
        .viewport_pixels((0.0, height - 100.0), (320, 200))
        .expect("pixels");
    assert_eq!(pixels.len(), 320 * 200 * 4);
}

#[test]
fn switching_sheets_repaints_from_the_new_one() {
    let mut workbook = open("sample-workbook.xlsx");
    let (first, ..) = workbook
        .viewport_pixels((0.0, 0.0), (320, 200))
        .expect("first");
    let first_ink = body_ink(&workbook, &first, 320);

    workbook.select_sheet(1).expect("select");
    assert_eq!(workbook.sheet_index(), 1);

    let (second, ..) = workbook
        .viewport_pixels((0.0, 0.0), (320, 200))
        .expect("second");
    assert_ne!(first, second);
    // Sheet 2 has one cell and no grid lines, so it is nearly blank next to
    // the summary — a good sign the old sheet's pixels did not survive.
    assert!(body_ink(&workbook, &second, 320) < first_ink / 4);
}

#[test]
fn selecting_a_sheet_that_is_not_there_is_an_error() {
    let mut workbook = open("sample-workbook.xlsx");
    assert!(workbook.select_sheet(9).is_err());
    assert_eq!(workbook.sheet_index(), 0);
}

#[test]
fn embedded_png_svg_and_transforms_produce_the_expected_pixels() {
    let mut workbook = open("pictures-workbook.xlsx");
    let metrics = workbook.header_metrics();
    let (pixels, width, _) = workbook.viewport_pixels((0.0, 0.0), (650, 300)).unwrap();
    let rgb = |x: usize, y: usize| {
        let index = ((y + metrics.column_height as usize) * width as usize
            + x
            + metrics.row_width as usize)
            * 4;
        [pixels[index + 2], pixels[index + 1], pixels[index]]
    };
    assert_eq!(rgb(30, 30), [255, 0, 0]);
    assert_eq!(rgb(80, 30), [0, 255, 0]);
    assert_eq!(rgb(30, 65), [0, 0, 255]);
    assert_eq!(rgb(130, 30), [0, 255, 0]);
    assert_eq!(rgb(130, 65), [255, 255, 0]);
    assert_eq!(rgb(230, 30), [0, 255, 0]);
    assert_eq!(rgb(280, 30), [255, 0, 0]);
    assert_eq!(rgb(340, 20), [0, 0, 255]);
    assert_eq!(rgb(380, 20), [255, 0, 0]);
    assert_eq!(rgb(30, 130), [255, 255, 255]);
    assert_eq!(rgb(50, 130), [255, 0, 0]);
    assert_eq!(rgb(130, 130), [255, 0, 255]);
    assert_eq!(rgb(180, 130), [0, 255, 255]);
    let alpha = rgb(230, 130);
    assert_eq!(alpha[0], 255);
    assert!((126..=129).contains(&alpha[1]) && alpha[1] == alpha[2]);
    let second = workbook.viewport_pixels((0.0, 0.0), (650, 300)).unwrap().0;
    assert_eq!(pixels, second, "reusing decoded images preserves pixels");
    let (far, fw, _) = workbook
        .viewport_pixels((880.0, 880.0), (240, 180))
        .unwrap();
    let index =
        ((30 + metrics.column_height as usize) * fw as usize + 30 + metrics.row_width as usize) * 4;
    assert_eq!([far[index + 2], far[index + 1], far[index]], [255, 0, 0]);
}

#[test]
fn charts_render_series_pixels_and_doughnut_holes_through_the_native_painter() {
    let mut workbook = open("charts-workbook.xlsx");
    assert_eq!(
        workbook
            .chart_status()
            .iter()
            .filter(|chart| chart.rendered)
            .count(),
        // Every chart, the column-and-line combination among them.
        10
    );
    let metrics = workbook.header_metrics();
    let (pixels, _, _) = workbook.viewport_pixels((190.0, 0.0), (440, 310)).unwrap();
    let green = pixels
        .chunks_exact(4)
        .filter(|p| p[2] == 0 && p[1] == 170 && p[0] == 85)
        .count();
    assert!(
        green > 1000,
        "column series should occupy real chart pixels: {green}"
    );
    let (donut, width, _) = workbook
        .viewport_pixels((200.0, 640.0), (440, 310))
        .unwrap();
    let pixel = |x: usize, y: usize| {
        let i = ((y + metrics.column_height as usize) * width as usize
            + x
            + metrics.row_width as usize)
            * 4;
        [donut[i + 2], donut[i + 1], donut[i]]
    };
    assert_eq!(
        pixel(190, 160),
        [255, 255, 255],
        "doughnut center is a hole"
    );
    assert!(
        donut
            .chunks_exact(4)
            .filter(|p| p[2] == 0 && p[1] == 170 && p[0] == 85)
            .count()
            > 1000
    );
    assert!(width > 0);
}

#[test]
fn conditional_color_interpolation_and_data_bar_gradient_reach_native_pixels() {
    use jade_xlsx_reader::CellRef;
    let mut workbook = open("visual-conditional-workbook.xlsx");
    let metrics = workbook.header_metrics();
    let a = workbook
        .layout()
        .cell_rect(CellRef::parse("A3").unwrap())
        .unwrap();
    let c = workbook
        .layout()
        .cell_rect(CellRef::parse("C3").unwrap())
        .unwrap();
    let bar = workbook
        .layout()
        .cell(CellRef::parse("C3").unwrap())
        .unwrap()
        .data_bar
        .as_ref()
        .unwrap()
        .rect;
    let (pixels, width, _) = workbook.viewport_pixels((0.0, 0.0), (1000, 300)).unwrap();
    let sample = |x: f64, y: f64| {
        let offset = ((y + metrics.column_height) as usize * width as usize
            + (x + metrics.row_width) as usize)
            * 4;
        pixels[offset..offset + 4].to_vec()
    };
    assert_eq!(sample(a.x + 4.0, a.y + 4.0), [0, 128, 128, 255]);
    let left = sample(bar.x + 2.0, bar.y + 3.0);
    let right = sample(bar.right() - 3.0, bar.y + 3.0);
    assert!(left[2] < 140 && left[1] < 180 && left[0] > 190, "{left:?}");
    assert!(
        right[2] > 220 && right[1] > 220 && right[0] > 220,
        "{right:?}"
    );
    assert_eq!(
        sample(c.right() - 5.0, c.y + c.height / 2.0),
        [255; 4],
        "showValue=false paints no right-aligned number"
    );
}

#[test]
fn modern_solid_negative_bars_and_rtl_context_have_matching_native_pixels() {
    use jade_xlsx_reader::CellRef;
    let mut workbook = open("x14-bars-workbook.xlsx");
    let metrics = workbook.header_metrics();
    let negative = workbook
        .layout()
        .cell(CellRef::parse("A1").unwrap())
        .unwrap()
        .data_bar
        .as_ref()
        .unwrap()
        .rect;
    let positive = workbook
        .layout()
        .cell(CellRef::parse("A5").unwrap())
        .unwrap()
        .data_bar
        .as_ref()
        .unwrap()
        .rect;
    let (pixels, width, _) = workbook.viewport_pixels((0.0, 0.0), (1000, 300)).unwrap();
    let sample = |rect: jade_xlsx_layout::Rect| {
        let index = ((rect.y + metrics.column_height + 4.0) as usize * width as usize
            + (rect.x + metrics.row_width + 4.0) as usize)
            * 4;
        pixels[index..index + 4].to_vec()
    };
    assert_eq!(sample(negative), [0, 0, 255, 255]);
    assert_eq!(sample(positive), [198, 142, 99, 255]);
    workbook.select_sheet(1).unwrap();
    let rtl = workbook
        .layout()
        .cell(CellRef::parse("A1").unwrap())
        .unwrap()
        .data_bar
        .as_ref()
        .unwrap()
        .rect;
    assert!(rtl.x > negative.x);
    let (pixels, width, _) = workbook.viewport_pixels((0.0, 0.0), (1000, 300)).unwrap();
    let index = ((rtl.y + metrics.column_height + 4.0) as usize * width as usize
        + (rtl.x + metrics.row_width + 4.0) as usize)
        * 4;
    assert_eq!(&pixels[index..index + 4], &[0, 0, 255, 255]);
}

#[test]
fn saved_table_filter_buttons_reach_native_pixels_with_shared_rectangles() {
    let mut workbook = open("filtered-subtotal-workbook.xlsx");
    let metrics = workbook.header_metrics();
    let rect = workbook
        .layout()
        .filter_buttons
        .first()
        .expect("table button")
        .rect;
    let (pixels, width, _) = workbook.viewport_pixels((0.0, 0.0), (1000, 300)).unwrap();
    let at = |x: f64, y: f64| {
        let offset = ((y + metrics.column_height) as usize * width as usize
            + (x + metrics.row_width) as usize)
            * 4;
        pixels[offset..offset + 4].to_vec()
    };
    assert_eq!(at(rect.x + 2.0, rect.y + 2.0), [245, 245, 245, 255]);
    let glyph = at(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
    assert!(glyph[..3].iter().all(|color| *color < 150), "{glyph:?}");
}
