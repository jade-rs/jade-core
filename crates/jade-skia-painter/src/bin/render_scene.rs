//! Renders the shared comparison scene with Skia.
//!
//! `tests/chromium-scene.mjs` draws the identical operations through Chromium's
//! canvas. Both write a PNG so the two rasterisers can be compared directly.

use jade_painter::Painter;
use jade_skia_painter::SkiaPainter;

fn main() {
    let repo = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let painter = SkiaPainter::new(400, 120).expect("surface");
    for (weight, file) in [
        (400, "NotoSansJP-Regular.ttf"),
        (700, "NotoSansJP-Bold.ttf"),
    ] {
        let bytes =
            std::fs::read(format!("{repo}/fonts/Noto_Sans_JP/static/{file}")).expect("font");
        painter
            .register_font_bytes("Noto Sans JP", weight, &bytes)
            .expect("register");
    }

    painter.set_font("12.00pt 'Noto Sans JP'");
    painter.set_fill_style_str("#101820");
    painter
        .fill_text("横浜市市税条例 Hgあ", 20.0, 40.0)
        .unwrap();

    painter.set_fill_style_str("#c00000");
    painter.fill_rect(20.0, 60.0, 100.0, 10.0);

    painter.set_stroke_style_str("#0057b8");
    painter.set_line_width(2.0);
    painter.begin_path();
    painter.move_to(20.0, 90.0);
    painter.line_to(380.0, 90.0);
    painter.stroke();

    let out = std::env::args()
        .nth(1)
        .expect("usage: render_scene <out.png>");
    std::fs::write(&out, painter.encode_png().expect("encode")).expect("write");
    eprintln!("wrote {out}");
}
