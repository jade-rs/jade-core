//! Allocation/timing micro-benchmark for run-style property cloning and
//! merging — the per-render and parse-merge hot paths. Compiles identically on
//! the `String` baseline and the `Rc<str>` branch (field values use `.into()`,
//! which works for both `String` and `Rc<str>`), so the same source measures
//! both. The headline metric is allocations-per-operation, which is what the
//! `Rc<str>` change targets (a clone becomes a refcount bump, not a heap copy).
//!
//! Run: `cargo run --release --example bench_props -p jade-docx-reader`

use jade_docx_reader::styles::merge_run_properties_override;
use jade_docx_reader::{RunFonts, RunProperties};
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

struct Counting;

// SAFETY: delegates straight to the System allocator; only adds relaxed counters.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// A representative directly-formatted run: an explicit font (latin + east-asia
/// slots), a color, a font_family, and a character-style reference — the common
/// shape of a styled run in a real document.
fn representative() -> RunProperties {
    RunProperties {
        bold: Some(true),
        font_size: Some(10.5),
        style: Some("BodyText".into()),
        font_family: Some("Yu Mincho".into()),
        color: Some("#1A1A1A".into()),
        fonts: Some(RunFonts {
            ascii: Some("Yu Mincho".into()),
            h_ansi: Some("Yu Mincho".into()),
            east_asia: Some("游明朝".into()),
            ..RunFonts::default()
        }),
        ..RunProperties::default()
    }
}

fn snapshot() -> (usize, usize) {
    (
        ALLOCS.load(Ordering::Relaxed),
        BYTES.load(Ordering::Relaxed),
    )
}

fn main() {
    const N: usize = 2_000_000;
    let source = representative();

    // Warm up.
    for _ in 0..10_000 {
        black_box(source.clone());
    }

    // --- clone (per-render run clone) ---
    let (a0, b0) = snapshot();
    let t = Instant::now();
    for _ in 0..N {
        black_box(source.clone());
    }
    let dt = t.elapsed();
    let (a1, b1) = snapshot();
    let allocs = a1 - a0;
    let bytes = b1 - b0;
    println!("clone   x{N}: {:>8.2} ns/op | {:>6.3} allocs/op | {:>5.1} bytes/op (total {} allocs, {} bytes, {:?})",
        dt.as_nanos() as f64 / N as f64,
        allocs as f64 / N as f64,
        bytes as f64 / N as f64,
        allocs, bytes, dt);

    // --- merge into a fresh default (style-resolution step) ---
    let (a0, b0) = snapshot();
    let t = Instant::now();
    for _ in 0..N {
        let mut target = RunProperties::default();
        merge_run_properties_override(&mut target, &source);
        black_box(&target);
    }
    let dt = t.elapsed();
    let (a1, b1) = snapshot();
    let allocs = a1 - a0;
    let bytes = b1 - b0;
    println!("merge   x{N}: {:>8.2} ns/op | {:>6.3} allocs/op | {:>5.1} bytes/op (total {} allocs, {} bytes, {:?})",
        dt.as_nanos() as f64 / N as f64,
        allocs as f64 / N as f64,
        bytes as f64 / N as f64,
        allocs, bytes, dt);
}
