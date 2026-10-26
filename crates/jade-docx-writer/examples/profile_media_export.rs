//! Compare map rebuilding with source-member preservation, including CRC/byte checks.
//! cargo run --release -p jade-docx-writer --example profile_media_export -- input.docx
use jade_docx_writer::{write_docx, write_docx_preserving_archive};
use std::{
    collections::{HashMap, HashSet},
    io::{Cursor, Read},
    time::Instant,
};
use zip::ZipArchive;

fn inflated(bytes: &[u8]) -> HashMap<String, Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|index| {
            let mut part = archive.by_index(index).unwrap();
            let name = part.name().unwrap().into_owned();
            let mut data = Vec::new();
            part.read_to_end(&mut data).unwrap();
            (name, data)
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for path in std::env::args().skip(1) {
        let source = std::fs::read(&path)?;
        let files = inflated(&source);
        let mut edited = files["word/document.xml"].clone();
        edited.extend_from_slice(b"<!-- synthetic export benchmark edit -->");
        let overrides = HashMap::from([("word/document.xml".into(), edited)]);
        // The baseline also repairs invalid WordprocessingML namespaces.
        let expected = inflated(&write_docx(&files, &overrides)?);
        let mut samples = [Vec::new(), Vec::new()];
        let mut sizes = [0; 2];
        // Alternate order; discard two warmups per implementation. Correctness
        // validation is outside the measured interval, for both implementations.
        for iteration in 0..15 {
            for operation in [iteration % 2, (iteration + 1) % 2] {
                let started = Instant::now();
                let output = if operation == 0 {
                    write_docx(&files, &overrides)?
                } else {
                    write_docx_preserving_archive(&source, &overrides, &HashSet::new())?
                };
                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                if iteration >= 2 {
                    samples[operation].push(elapsed);
                }
                sizes[operation] = output.len();
                assert_eq!(
                    inflated(&output),
                    expected,
                    "every member and CRC must survive"
                );
            }
        }
        for operation in 0..2 {
            samples[operation].sort_by(f64::total_cmp);
            println!(
                "input={path:?} source_bytes={} mode={} samples={} output_bytes={} median_ms={:.3} p95_ms={:.3}",
                source.len(),
                ["recompress", "preserve"][operation],
                samples[operation].len(),
                sizes[operation],
                samples[operation][6],
                samples[operation][12],
            );
        }
    }
    Ok(())
}
