//! Outlined PDF glyphs must retain the live painter's stroke coverage.
use jade_painter::Painter;
use jade_skia_painter::SkiaPainter;

fn assert_outline_density(name: &str, line_width: f64, transformed: bool, translucent: bool) {
    let raster = SkiaPainter::new(400, 150).unwrap();
    let recording = SkiaPainter::new_recording(400.0, 150.0).unwrap();
    let font = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fonts/Noto_Sans_JP/static/NotoSansJP-Regular.ttf"),
    )
    .unwrap();
    for painter in [&raster, &recording] {
        painter
            .register_font_bytes("Noto Sans JP", 400, &font)
            .unwrap();
        painter.set_font("20pt 'Noto Sans JP'");
        painter.set_stroke_style_str("#000000");
        painter.set_line_width(line_width);
        if transformed {
            painter.translate(30.2, 12.4).unwrap();
            painter.scale(1.4, 1.2).unwrap();
            painter.rotate(0.12).unwrap();
        }
        if translucent {
            painter.set_stroke_style_str("#2070c0");
            painter.set_global_alpha(0.6);
            painter.set_shadow_color("#803020");
            painter.set_shadow_blur(2.0);
            painter.set_shadow_offset_x(3.0);
            painter.set_shadow_offset_y(2.0);
        }
        painter.stroke_text("Outlined text", 10.6, 40.43).unwrap();
    }
    let directory =
        std::env::temp_dir().join(format!("jade-pdf-outline-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let pdf_path = directory.join("outline.pdf");
    std::fs::write(&pdf_path, SkiaPainter::encode_pdf(&[recording]).unwrap()).unwrap();
    std::fs::write(directory.join("editor.png"), raster.encode_png().unwrap()).unwrap();
    let status = std::process::Command::new(std::env::var_os("JADE_PDFTOPPM").unwrap())
        .args(["-png", "-singlefile", "-r", "96"])
        .arg(&pdf_path)
        .arg(directory.join("outline"))
        .status()
        .unwrap();
    assert!(status.success());
    let actual = SkiaPainter::new(400, 150).unwrap();
    assert!(actual.draw_encoded_image(
        &std::fs::read(directory.join("outline.png")).unwrap(),
        0.0,
        0.0,
        400.0,
        150.0
    ));
    let mass = |painter: &SkiaPainter| -> u64 {
        (0..150)
            .flat_map(|y| (0..400).map(move |x| (x, y)))
            .map(|(x, y)| {
                let (red, green, blue, _) = painter.pixel(x, y).unwrap();
                u64::from(255 - red) + u64::from(255 - green) + u64::from(255 - blue)
            })
            .sum()
    };
    let expected = mass(&raster);
    let observed = mass(&actual);
    assert!(expected > 0);
    assert!(
        expected.abs_diff(observed) * 100 <= expected * 15,
        "outline weight changed: live={expected}, PDF={observed}; artifacts: {}",
        directory.display()
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "requires JADE_PDFTOPPM (Poppler); CI runs this explicitly"]
fn outlined_text_keeps_live_stroke_density() {
    assert_outline_density("plain", 1.0, false, false);
}

#[test]
#[ignore = "requires JADE_PDFTOPPM (Poppler); CI runs this explicitly"]
fn transformed_translucent_outlines_keep_color_shadow_and_density() {
    assert_outline_density("transformed", 0.5, true, true);
    assert_outline_density("thick", 2.0, true, true);
}
