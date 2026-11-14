//! Capture the native renderer's unaligned glyph origins in PDF points.
//! Usage: word_coordinates document.docx output.json [mac|windows] [vector.pdf] [--unclipped-tables] [--password-file file] [--fonts-directory directory]
use jade_docx_layout::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
use jade_docx_native_render::{
    system_fonts, FontFace, NativeDocument, PdfExportOptions, PdfRenderingMode,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn main() {
    let mut args: Vec<_> = std::env::args().collect();
    let password_file = args
        .iter()
        .position(|arg| arg == "--password-file")
        .map(|index| {
            assert!(index + 1 < args.len(), "--password-file requires a path");
            let file = args.remove(index + 1);
            args.remove(index);
            file
        });
    let fonts_directory = args
        .iter()
        .position(|arg| arg == "--fonts-directory")
        .map(|index| {
            assert!(index + 1 < args.len(), "--fonts-directory requires a path");
            let directory = PathBuf::from(args.remove(index + 1));
            args.remove(index);
            assert!(directory.is_dir(), "font directory does not exist");
            directory
        });
    let unclipped_tables = args.iter().any(|arg| arg == "--unclipped-tables");
    args.retain(|arg| arg != "--unclipped-tables");
    assert!(args.len() >= 3, "document.docx output.json [mac|windows]");
    set_word_layout_platform(if args.get(3).map(String::as_str) == Some("mac") {
        WordLayoutPlatform::Mac
    } else {
        WordLayoutPlatform::Windows
    });
    let input = std::fs::read(&args[1]).unwrap();
    let input_sha256 = format!("{:x}", Sha256::digest(&input));
    let encrypted = jade_opc::encryption::is_encrypted_package(&input);
    let bytes = if let Some(file) = &password_file {
        assert!(
            encrypted,
            "--password-file requires an encrypted Office package"
        );
        let password = std::fs::read_to_string(file).expect("cannot read password file");
        jade_opc::encryption::decrypt_package(&input, password.trim_end_matches(['\r', '\n']))
            .expect("cannot decrypt Office package")
    } else {
        assert!(
            !encrypted,
            "encrypted Office package requires --password-file"
        );
        input
    };
    let render_input_sha256 = format!("{:x}", Sha256::digest(&bytes));
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
    let faces = if let Some(directory) = fonts_directory {
        // A replay can use only the fonts embedded in its original Word PDF,
        // independently of which Office fonts the CI host has installed.
        let library = system_fonts::FontLibrary::in_directories(&[directory]);
        let families = system_fonts::word_families(&bytes);
        let (reference_faces, _) = library.faces_for(families.iter().map(String::as_str));
        assert!(
            !reference_faces.is_empty(),
            "no document fonts in reference directory"
        );
        fallback.into_iter().chain(reference_faces).collect()
    } else {
        system_fonts::word_document_faces(&bytes, &fallback)
    };
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
    let result = json!({"input_sha256": input_sha256, "render_input_sha256": render_input_sha256, "password_provided": password_file.is_some(), "page_count": doc.page_count(), "table_glyph_projection": if unclipped_tables {"unclipped"} else {"visible"}, "glyphs": glyphs, "fonts": faces.iter().map(|f| json!({"family": f.family, "path": f.path, "index": f.index, "weight": f.weight, "italic": f.italic})).collect::<Vec<_>>()});
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
