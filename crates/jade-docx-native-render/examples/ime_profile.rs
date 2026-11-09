//! Native Word IME edit, caret and pixel-read costs (no GPUI/GPU presentation).
//! cargo run --release --example ime_profile -- [file.docx] [samples] [paragraph]
use jade_docx_native_render::{FaceSpec, NativeDocument};
use std::{path::PathBuf, time::Instant};

fn main() -> Result<(), String> {
    let core = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut args = std::env::args().skip(1);
    let file = args
        .next()
        .map(PathBuf::from)
        .unwrap_or(core.join("contract.docx"));
    let count: usize = args.next().map(|n| n.parse().unwrap()).unwrap_or(100);
    let paragraph: usize = args.next().map(|n| n.parse().unwrap()).unwrap_or(3);
    assert!(count > 0);
    let regular = core.join("fonts/Noto_Sans_JP/static/NotoSansJP-Regular.ttf");
    let bold = core.join("fonts/Noto_Sans_JP/static/NotoSansJP-Bold.ttf");
    let mut faces = Vec::new();
    for family in [
        "Noto Sans JP",
        "Times New Roman",
        "Yu Mincho",
        "MS Mincho",
        "serif",
    ] {
        faces.push(FaceSpec {
            family,
            weight: 400,
            path: &regular,
        });
        faces.push(FaceSpec {
            family,
            weight: 700,
            path: &bold,
        });
    }
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    let mut doc = NativeDocument::open(&bytes, 816.0, 1056.0, &faces)?;
    let caret = doc.place_caret(paragraph, 0).ok_or("no caret")?;
    let page = doc.page_and_offset(caret.document_y).0;
    let mut lengths = 0;
    let mut times = Vec::new();
    eprintln!(
        "ready pid={} pages={} paragraph={paragraph} page={page}",
        std::process::id(),
        doc.page_count()
    );
    for i in 0..count + 10 {
        let text = ["に", "にほ", "にほん", "日本", "日本語", "にほんご", ""][i % 7];
        let start = Instant::now();
        doc.replace_range(paragraph, 0, lengths, text, page + 1)?;
        let edited = start.elapsed().as_secs_f64() * 1000.0;
        doc.place_caret(paragraph, text.len());
        let placed = start.elapsed().as_secs_f64() * 1000.0;
        // Same reads as DocumentView::refresh_caret_page_image.
        std::hint::black_box(doc.page_pixels(page));
        if page + 1 < doc.page_count() {
            std::hint::black_box(doc.page_pixels(page + 1));
        }
        if i >= 10 {
            times.push([
                edited,
                placed - edited,
                start.elapsed().as_secs_f64() * 1000.0,
            ]);
        }
        lengths = text.len();
    }
    let (eligible, taken, fallbacks) = doc.renderer().band_redraw_stats();
    eprintln!("band eligible={eligible} taken={taken} fallbacks={fallbacks:?}");
    println!("phase,samples,p50_ms,p95_ms");
    for (column, phase) in ["edit_and_raster", "caret", "with_pixel_reads"]
        .iter()
        .enumerate()
    {
        let mut values: Vec<_> = times.iter().map(|t| t[column]).collect();
        values.sort_by(f64::total_cmp);
        println!(
            "{phase},{count},{:.3},{:.3}",
            values[(count as f64 * 0.5).ceil() as usize - 1],
            values[(count as f64 * 0.95).ceil() as usize - 1]
        );
    }
    Ok(())
}
