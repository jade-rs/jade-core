//! Native Word IME edit, caret and pixel-read costs (no GPUI/GPU presentation).
//! cargo run --release --example ime_profile -- [file.docx] [samples] [paragraph] [scale] [all|changed]
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
    let scale: f64 = args.next().map(|n| n.parse().unwrap()).unwrap_or(1.0);
    let changed_only = match args.next().as_deref() {
        None | Some("all") => false,
        Some("changed") => true,
        Some(_) => return Err("expected pixel read mode 'all' or 'changed'".into()),
    };
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
    doc.set_raster_scale(scale)?;
    let caret = doc.place_caret(paragraph, 0).ok_or("no caret")?;
    let page = doc.page_and_offset(caret.document_y).0;
    let mut lengths = 0;
    let mut times = Vec::new();
    let mut generations = [None; 2];
    let mut reads = 0;
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
        for (index, previous) in (page..doc.page_count()).zip(&mut generations) {
            let generation = if changed_only {
                doc.page_raster_generation(index)
            } else {
                None
            };
            if generation.is_none() || generation != *previous {
                std::hint::black_box(doc.page_pixels(index));
                *previous = generation;
                reads += 1;
            }
        }
        if i >= 10 {
            let total = start.elapsed().as_secs_f64() * 1000.0;
            times.push([edited, placed - edited, total - placed, total]);
        }
        lengths = text.len();
    }
    let (eligible, taken, fallbacks) = doc.renderer().band_redraw_stats();
    eprintln!("band eligible={eligible} taken={taken} fallbacks={fallbacks:?}");
    eprintln!("page reads={reads} (including warmup)");
    println!("phase,samples,p50_ms,p95_ms");
    for (column, phase) in [
        "edit_and_raster",
        "caret",
        "pixel_reads",
        "with_pixel_reads",
    ]
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
