//! Editor PDF recordings preserve raster coverage across painter state and DPI.
use jade_painter::Painter;
use jade_skia_painter::SkiaPainter;

#[test]
fn editor_pdf_keeps_scaled_transformed_and_clipped_painter_pixels() {
    let font = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fonts/Noto_Sans_JP/static/NotoSansJP-Regular.ttf"),
    )
    .unwrap();
    for scale in [0.5_f64, 1.0, 1.25, 1.5, 2.0, 2.5, 4.0] {
        let (width, height) = ((240.4 * scale).ceil() as i32, (160.3 * scale).ceil() as i32);
        let live = SkiaPainter::new(width, height).unwrap();
        live.scale(scale, scale).unwrap();
        let recording = SkiaPainter::new_editor_recording(240.4, 160.3, scale).unwrap();
        for painter in [&live, &recording] {
            painter
                .register_font_bytes("Noto Sans JP", 400, &font)
                .unwrap();
            painter.set_font("14pt 'Noto Sans JP'");
            painter.set_fill_style_str("#2080c0");
            painter.fill_rect(7.2, 11.6, 75.3, 23.4);
            painter.save();
            painter.begin_path();
            painter.rect(5.4, 20.2, 200.8, 105.6);
            painter.clip();
            painter.translate(13.4, 18.7).unwrap();
            painter.rotate(0.12).unwrap();
            painter.set_global_alpha(0.65);
            painter.set_fill_style_str("#803020");
            painter.set_shadow_color("#102030");
            painter.set_shadow_blur(2.3);
            painter.set_shadow_offset_x(2.4);
            painter.set_shadow_offset_y(3.5);
            painter.fill_text("日本語 😀 é", 8.3, 54.6).unwrap();
            painter.set_line_width(0.7);
            painter.set_stroke_style_str("#203080");
            painter.stroke_text("Outline", 10.4, 85.3).unwrap();
            painter.stroke_rect(4.6, 40.2, 125.3, 63.6);
            painter.restore();
        }
        let bytes = SkiaPainter::encode_pdf(&[recording]).unwrap();
        let pdf = lopdf::Document::load_mem(&bytes).unwrap();
        let image = pdf
            .objects
            .values()
            .find_map(|object| {
                let stream = object.as_stream().ok()?;
                (stream.dict.get(b"Subtype").ok()?.as_name().ok()? == b"Image").then_some(stream)
            })
            .unwrap();
        assert_eq!(
            image.dict.get(b"Width").unwrap().as_i64().unwrap(),
            i64::from(width)
        );
        assert_eq!(
            image.dict.get(b"Height").unwrap().as_i64().unwrap(),
            i64::from(height)
        );
        let expected = (0..height)
            .flat_map(|y| {
                let live = &live;
                (0..width).flat_map(move |x| {
                    let (red, green, blue, alpha) = live.pixel(x, y).unwrap();
                    assert_eq!(alpha, 255);
                    [red, green, blue]
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            image.decompressed_content().unwrap(),
            expected,
            "PDF pixels at scale {scale}"
        );
    }
}
