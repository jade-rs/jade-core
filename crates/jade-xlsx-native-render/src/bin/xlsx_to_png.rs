//! Rasterises one sheet of an `.xlsx` to a PNG.
//!
//! The point is not the PNG: it is that the spreadsheet crates draw through
//! `jade_painter::Painter` and therefore produce pixels on the native backend
//! with no browser and no second renderer. The unit tests assert the draw
//! calls; this asserts they reach a rasteriser.
//!
//! ```text
//! cd crates/jade-xlsx-native-render
//! cargo run --bin xlsx_to_png -- ../../book.xlsx out.png [sheet-name]
//! ```
//!
//! Unlike the interactive shells this paints the whole sheet, which is exactly
//! what a fixed-size image wants and exactly what a window cannot do.

use std::path::PathBuf;

use jade_skia_painter::SkiaPainter;
use jade_xlsx_native_render::{aliased_faces, bundled_font_paths, NativeWorkbook};
use jade_xlsx_render::{render_sheet, RenderOptions};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        eprintln!("usage: xlsx_to_png <in.xlsx> <out.png> [sheet-name]");
        std::process::exit(2);
    };
    let wanted_sheet = args.next();

    let bytes = match std::fs::read(&input) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("cannot read {input}: {error}");
            std::process::exit(1);
        }
    };

    let (regular, bold) = bundled_font_paths(&PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../.."
    )));
    let faces = aliased_faces(&regular, &bold);
    let mut workbook = match NativeWorkbook::open(&bytes, &faces) {
        Ok(workbook) => workbook,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };

    if let Some(name) = &wanted_sheet {
        let index = workbook
            .sheet_names()
            .iter()
            .position(|sheet| sheet == name);
        match index {
            Some(index) => {
                if let Err(error) = workbook.select_sheet(index) {
                    eprintln!("{error}");
                    std::process::exit(1);
                }
            }
            None => {
                eprintln!("no sheet named {name} in {input}");
                std::process::exit(1);
            }
        }
    }

    let (sheet_width, sheet_height) = workbook.sheet_size();
    let width = sheet_width.ceil().max(1.0) as i32;
    let height = sheet_height.ceil().max(1.0) as i32;
    let Some(painter) = workbook.new_surface(width, height) else {
        eprintln!("cannot create a {width}x{height} surface");
        std::process::exit(1);
    };

    if let Err(error) = render_sheet(&painter, workbook.layout(), &RenderOptions::default()) {
        eprintln!("render failed: {error:?}");
        std::process::exit(1);
    }

    let Some(png) = SkiaPainter::encode_png(&painter) else {
        eprintln!("cannot encode the surface");
        std::process::exit(1);
    };
    if let Err(error) = std::fs::write(&output, png) {
        eprintln!("cannot write {output}: {error}");
        std::process::exit(1);
    }
    eprintln!(
        "wrote {output} ({width}x{height}, sheet \"{}\", {} cells)",
        workbook.sheet_names()[workbook.sheet_index()],
        workbook.layout().cells.len()
    );
}
