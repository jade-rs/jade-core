//! Capture the native renderer's unaligned glyph origins in PDF points.
//! Usage: word_coordinates document.docx output.json [mac|windows] [vector.pdf] [--unclipped-tables]
use jade_docx_layout::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
use jade_docx_native_render::{
    system_fonts, FontFace, NativeDocument, PdfExportOptions, PdfRenderingMode,
};
use serde_json::json;
use std::path::PathBuf;

fn main() {
    let mut args: Vec<_> = std::env::args().collect();
    let unclipped_tables = args.iter().any(|arg| arg == "--unclipped-tables");
    args.retain(|arg| arg != "--unclipped-tables");
    assert!(args.len() >= 3, "document.docx output.json [mac|windows]");
    set_word_layout_platform(if args.get(3).map(String::as_str) == Some("mac") {
        WordLayoutPlatform::Mac
    } else {
        WordLayoutPlatform::Windows
    });
    let bytes = std::fs::read(&args[1]).unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fallback: Vec<_> = [(400, "Regular"), (700, "Bold")]
        .into_iter()
        .map(|(weight, style)| FontFace {
            family: "Noto Sans JP".into(),
            weight,
            italic: false,
            index: 0,
            path: root.join(format!("fonts/Noto_Sans_JP/static/NotoSansJP-{style}.ttf")),
        })
        .collect();
    let faces = system_fonts::word_document_faces(&bytes, &fallback);
    let mut doc = NativeDocument::open_with_font_faces(&bytes, 816.0, 1056.0, &faces).unwrap();
    let snapshot = if unclipped_tables {
        doc.capture_full_layout_debug_unclipped_tables().unwrap()
    } else {
        doc.capture_full_layout_debug().unwrap()
    };
    assert!(snapshot.complete, "incomplete glyph capture");
    let mut lines = snapshot.body_lines;
    lines.extend(snapshot.auxiliary_lines.into_iter().map(|entry| entry.line));
    let mut glyphs = Vec::new();
    for line in lines {
        for fragment in line.fragments {
            for glyph in fragment.glyphs {
                if glyph.text.is_empty() || glyph.text.chars().all(char::is_control) {
                    continue;
                }
                glyphs.push(json!([
                    line.page + 1,
                    glyph.text,
                    (fragment.x + glyph.x - 2.0 * line.report_x_offset) * 0.75,
                    fragment.baseline * 0.75
                ]));
            }
        }
    }
    let result = json!({"page_count": doc.page_count(), "table_glyph_projection": if unclipped_tables {"unclipped"} else {"visible"}, "glyphs": glyphs, "fonts": faces.iter().map(|f| json!({"family": f.family, "path": f.path, "index": f.index, "weight": f.weight, "italic": f.italic})).collect::<Vec<_>>()});
    std::fs::write(&args[2], serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    if let Some(pdf_path) = args.get(4) {
        let pdf = doc
            .export_pdf_with_options(PdfExportOptions {
                rendering: PdfRenderingMode::Vector,
                ..Default::default()
            })
            .unwrap();
        std::fs::write(pdf_path, pdf).unwrap();
    }
    if let Some((pixels, width, height)) = doc.page_pixels(0) {
        image::save_buffer(
            PathBuf::from(&args[2]).with_extension("png"),
            &pixels,
            width as u32,
            height as u32,
            image::ColorType::Rgba8,
        )
        .unwrap();
    }
}
