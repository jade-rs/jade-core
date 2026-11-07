//! A workbook rendered natively, without a browser.
//!
//! The counterpart of `jade-xlsx-editor`: same reader, same layout, same
//! renderer, a different `jade_painter::Painter` underneath. What this crate
//! adds is the native half — a Skia surface, font files from disk, and pixels
//! in the byte order a native shell wants.
//!
//! Like the browser side it paints a *viewport* rather than the whole sheet.
//! A sheet used down to row 5000 is 125,000 px tall, and while Skia would
//! allocate that, holding it as a texture per frame would not be sensible.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use jade_skia_painter::SkiaPainter;
use jade_xlsx_layout::{layout_workbook_sheet, LayoutOptions, Rect, SheetLayout};
use jade_xlsx_render::{render_grid, PainterMeasure, RenderOptions};

/// A font file to register under a family name, so a document that asks for
/// Calibri gets something rather than nothing.
#[derive(Debug, Clone, Copy)]
pub struct FaceSpec<'a> {
    pub family: &'a str,
    pub weight: u16,
    pub path: &'a Path,
}

/// Columns and rows drawn past the last one the sheet writes a cell in, so a
/// small sheet still reads as a grid rather than a floating block of cells.
const MIN_COLUMNS: u32 = 12;
const MIN_ROWS: u32 = 30;

/// The families a document is likely to name. Workbooks ask for Excel's fonts,
/// which a machine outside Office does not have; aliasing the bundled face onto
/// all of them beats rendering a blank sheet.
pub const ALIASED_FAMILIES: &[&str] = &[
    "Calibri",
    "Meiryo",
    "Yu Gothic",
    "游ゴシック",
    "MS PGothic",
    "ＭＳ Ｐゴシック",
    "Noto Sans JP",
];

/// The bundled Noto faces, as (regular, bold), relative to a repository root.
pub fn bundled_font_paths(repository_root: &Path) -> (PathBuf, PathBuf) {
    let directory = repository_root.join("fonts/Noto_Sans_JP/static");
    (
        directory.join("NotoSansJP-Regular.ttf"),
        directory.join("NotoSansJP-Bold.ttf"),
    )
}

/// [`ALIASED_FAMILIES`] at both weights, pointing at the two files.
pub fn aliased_faces<'a>(regular: &'a Path, bold: &'a Path) -> Vec<FaceSpec<'a>> {
    ALIASED_FAMILIES
        .iter()
        .flat_map(|family| {
            [
                FaceSpec {
                    family,
                    weight: 400,
                    path: regular,
                },
                FaceSpec {
                    family,
                    weight: 700,
                    path: bold,
                },
            ]
        })
        .collect()
}

struct RegisteredFace {
    family: String,
    weight: u16,
    bytes: Vec<u8>,
}

/// An open workbook, with one sheet laid out and a surface to paint it on.
pub struct NativeWorkbook {
    workbook: jade_xlsx_reader::Workbook,
    faces: Vec<RegisteredFace>,
    sheet_index: usize,
    layout: Rc<SheetLayout>,
    /// Reused between frames; replaced when the viewport changes size.
    surface: Option<(SkiaPainter, i32, i32)>,
}

impl NativeWorkbook {
    /// Reads `.xlsx` bytes and lays out the first sheet.
    pub fn open(bytes: &[u8], faces: &[FaceSpec<'_>]) -> Result<Self, String> {
        let workbook = jade_xlsx_reader::read_xlsx(bytes)
            .map_err(|error| format!("cannot read the workbook: {error}"))?;
        if workbook.sheets.is_empty() {
            return Err("the workbook has no sheets".to_string());
        }

        let mut registered = Vec::new();
        for face in faces {
            match std::fs::read(face.path) {
                Ok(bytes) => registered.push(RegisteredFace {
                    family: face.family.to_string(),
                    weight: face.weight,
                    bytes,
                }),
                Err(error) => {
                    return Err(format!("{}: {error}", PathBuf::from(face.path).display()))
                }
            }
        }

        let mut open = Self {
            workbook,
            faces: registered,
            sheet_index: 0,
            layout: Rc::new(SheetLayout {
                columns: Vec::new(),
                rows: Vec::new(),
                cells: Vec::new(),
                filter_buttons: Vec::new(),
                blank_conditional_rules: Vec::new(),
                blank_row_styles: Default::default(),
                blank_column_styles: Vec::new(),
                shapes: Vec::new(),
                notes: Vec::new(),
                tables: Vec::new(),
                pivots: Vec::new(),
                sparklines: Vec::new(),
                show_grid_lines: true,
                show_headers: true,
                grid_line_color: None,
                content_width: 0.0,
                max_digit_width_px: jade_xlsx_layout::units::DEFAULT_MAX_DIGIT_WIDTH_PX,
                content_height: 0.0,
                frozen_columns: 0,
                frozen_rows: 0,
                cell_padding_px: 0.0,
                outline: Default::default(),
            }),
            surface: None,
        };
        open.lay_out()?;
        Ok(open)
    }

    pub fn sheet_names(&self) -> Vec<&str> {
        self.workbook
            .sheets
            .iter()
            .map(|sheet| sheet.name.as_str())
            .collect()
    }

    pub fn chart_status(&self) -> Vec<jade_xlsx_layout::chart::ChartStatus> {
        jade_xlsx_layout::chart::chart_status(&self.workbook)
    }

    pub fn sheet_index(&self) -> usize {
        self.sheet_index
    }

    /// Switches sheets, laying the new one out.
    pub fn select_sheet(&mut self, index: usize) -> Result<(), String> {
        if index >= self.workbook.sheets.len() {
            return Err(format!("no sheet at index {index}"));
        }
        if index == self.sheet_index {
            return Ok(());
        }
        self.sheet_index = index;
        self.lay_out()
    }

    /// The laid-out sheet's size in px, for sizing a scroll area.
    pub fn sheet_size(&self) -> (f64, f64) {
        (self.layout.content_width, self.layout.content_height)
    }

    /// The size of the header bands, so a shell can inset its scroll area to
    /// the part of the sheet that is actually on show.
    pub fn header_metrics(&self) -> jade_xlsx_layout::HeaderMetrics {
        self.layout.header_metrics()
    }

    pub fn layout(&self) -> &SheetLayout {
        &self.layout
    }

    /// Paints the part of the sheet under `scroll` onto a `width`×`height`
    /// surface and returns its pixels as BGRA, premultiplied.
    ///
    /// That byte order is what a native shell's image type wants; reading the
    /// surface in it beats swapping channels per pixel afterwards.
    pub fn viewport_pixels(
        &mut self,
        scroll: (f64, f64),
        size: (i32, i32),
    ) -> Option<(Vec<u8>, i32, i32)> {
        let (width, height) = (size.0.max(1), size.1.max(1));
        let reusable = matches!(self.surface, Some((_, w, h)) if w == width && h == height);
        if !reusable {
            self.surface = Some((self.new_painter(width, height)?, width, height));
        }
        let (painter, _, _) = self.surface.as_ref()?;

        // The surface holds the header bands as well as the sheet, so the
        // part of the sheet on show is smaller than it by exactly the bands.
        let metrics = self.layout.header_metrics();
        let viewport = Rect {
            x: scroll.0.max(0.0),
            y: scroll.1.max(0.0),
            width: (f64::from(width) - metrics.row_width).max(0.0),
            height: (f64::from(height) - metrics.column_height).max(0.0),
        };
        render_grid(
            painter,
            &self.layout,
            viewport,
            None,
            &RenderOptions::default(),
        )
        .ok()?;
        painter.read_bgra_premultiplied()
    }

    /// Lays the selected sheet out, measuring through a surface with the same
    /// faces registered — wrapping has to happen against the font that will
    /// draw, and the real surface cannot be sized until the rows are known.
    fn lay_out(&mut self) -> Result<(), String> {
        let measuring = self
            .new_painter(1, 1)
            .ok_or_else(|| "cannot create a measuring surface".to_string())?;
        self.layout = Rc::new(
            layout_workbook_sheet(
                &self.workbook,
                self.sheet_index,
                &LayoutOptions {
                    min_column_count: MIN_COLUMNS,
                    min_row_count: MIN_ROWS,
                    date_system: self.workbook.date_system,
                    measure: Rc::new(PainterMeasure::new(measuring)),
                    ..LayoutOptions::default()
                }
                .with_normal_font_metrics(&self.workbook.styles),
            )
            .ok_or_else(|| "sheet does not exist".to_string())?,
        );
        // The old surface belongs to the old sheet's size; drop it so the next
        // frame paints on one sized for this one.
        self.surface = None;
        Ok(())
    }

    /// A fresh surface with the workbook's faces registered.
    ///
    /// Public because a fixed-size output (a PNG of the whole sheet) needs a
    /// surface this type does not keep, and it must carry the same fonts or
    /// the text measured during layout is not the text that draws.
    pub fn new_surface(&self, width: i32, height: i32) -> Option<SkiaPainter> {
        self.new_painter(width, height)
    }

    fn new_painter(&self, width: i32, height: i32) -> Option<SkiaPainter> {
        let painter = SkiaPainter::new(width, height)?;
        for face in &self.faces {
            painter.register_font_bytes(&face.family, face.weight, &face.bytes)?;
        }
        Some(painter)
    }
}

#[cfg(test)]
mod normal_font_tests {
    use super::*;
    #[test]
    fn native_axes_measure_the_normal_style_reference_instead_of_font_zero() {
        let bytes = include_bytes!("../../../public/fixtures/grid-operations-workbook.xlsx");
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let (regular, bold) = bundled_font_paths(&root);
        let mut faces = aliased_faces(&regular, &bold);
        faces.push(FaceSpec {
            family: "Arial",
            weight: 400,
            path: &regular,
        });
        let mut book = NativeWorkbook::open(bytes, &faces).unwrap();
        assert_eq!(book.layout.max_digit_width_px, 7.0);
        let font_index = book.workbook.styles.fonts.len() as u32;
        book.workbook.styles.fonts.push(jade_xlsx_reader::Font {
            name: Some("Arial".into()),
            size_points: Some(22.0),
            ..Default::default()
        });
        let style_index = book.workbook.styles.style_formats.len() as u32;
        book.workbook
            .styles
            .style_formats
            .push(jade_xlsx_reader::CellFormat {
                font_index: Some(font_index),
                ..Default::default()
            });
        book.workbook.styles.normal_style_index = Some(style_index);
        book.lay_out().unwrap();
        assert!(book.layout.max_digit_width_px > 12.0);
        assert!(book.layout.columns[0].size > 100.0);
        let (pixels, ..) = book.viewport_pixels((0.0, 0.0), (320, 200)).unwrap();
        assert!(pixels
            .chunks_exact(4)
            .any(|pixel| pixel[0] < 100 && pixel[1] < 100 && pixel[2] < 100));
    }
}
