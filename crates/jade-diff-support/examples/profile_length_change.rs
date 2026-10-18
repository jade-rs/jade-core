//! cargo run --release -p structural-diff-support --example profile_length_change
//! Measures alignment only, excluding document loading and rendering.
use std::{hint::black_box, time::Instant};

fn main() {
    let old: Vec<u32> = (0..4_096).collect();
    let new: Vec<u32> = (10_000..10_032).collect();
    let started = Instant::now();
    for _ in 0..100 {
        black_box(diff_support::trimmed_lcs_diff(
            black_box(&old),
            black_box(&new),
        ));
    }
    println!(
        "4096 -> 32 elements, 100 comparisons: {:?}",
        started.elapsed()
    );
}
