//! Native timing probe for `WordDocument::load`. Pass a fixture path (default
//! `dist/fixtures/yokohama.docx`); prints per-iteration load time, min and median.
use jade_docx_editor::word_document::WordDocument;
use jade_docx_editor::Options;
use std::time::Instant;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "dist/fixtures/yokohama.docx".into());
    let iterations: usize = std::env::args()
        .nth(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(8);
    let data = std::fs::read(&path).expect("read fixture");
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        let document = futures::executor::block_on(WordDocument::load(&data, &Options::default()))
            .expect("load");
        let elapsed = started.elapsed().as_secs_f64() * 1e3;
        drop(document);
        samples.push(elapsed);
    }
    samples.sort_by(|a, b| a.total_cmp(b));
    let median = samples[samples.len() / 2];
    println!(
        "{path}: load min {:.1} ms, median {median:.1} ms over {iterations} runs: {:?}",
        samples[0],
        samples
            .iter()
            .map(|s| (s * 10.0).round() / 10.0)
            .collect::<Vec<_>>()
    );
}
