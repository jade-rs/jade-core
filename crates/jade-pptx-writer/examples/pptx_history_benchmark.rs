//! Reproducible native writer-session history benchmark; no Office parity claim.
use jade_pptx_writer::EditSession;
use std::{error::Error, fs, path::PathBuf, time::Instant};

fn image(seed: u32) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut state = seed;
    let mut data = vec![0; 1024 * 1024 * 4];
    for pixel in data.chunks_exact_mut(4) {
        for byte in &mut pixel[..3] {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *byte = state as u8;
        }
        pixel[3] = 255;
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 1024, 1024);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&data)?;
    }
    Ok(bytes)
}
fn percentile(values: &[f64], percentile: f64) -> f64 {
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * percentile).ceil() as usize]
}
fn main() -> Result<(), Box<dyn Error>> {
    if cfg!(debug_assertions) {
        return Err("run with cargo run --release".into());
    }
    let output = PathBuf::from(std::env::args().nth(1).ok_or(
        "usage: pptx_history_benchmark <output-directory> [source.pptx] [--generate-only]",
    )?);
    fs::create_dir_all(&output)?;
    let packages = output.join("packages");
    fs::create_dir_all(&packages)?;
    let generate_only = std::env::args().any(|arg| arg == "--generate-only");
    let input = std::env::args()
        .nth(2)
        .filter(|arg| arg != "--generate-only");
    let provided = input.is_some();
    let bytes = if let Some(input) = input {
        fs::read(input)?
    } else {
        let mut source =
            EditSession::open(include_bytes!("../../../public/fixtures/sample-deck.pptx"))?;
        for (index, seed) in [17, 23].into_iter().enumerate() {
            source.insert_picture(
                0,
                jade_pptx_reader::Transform {
                    offset_emu: (952500 + (index as i64) * 3048000, 3809999),
                    extent_emu: (2438400, 1828800),
                    rotation_60000ths: 0,
                    flip_h: false,
                    flip_v: false,
                },
                &image(seed)?,
            )?;
        }
        while jade_pptx_reader::read_deck(source.bytes())?.slides.len() < 100 {
            source.duplicate_slide(0)?;
        }
        source.bytes().to_vec()
    };
    assert_eq!(jade_pptx_reader::read_deck(&bytes)?.slides.len(), 100);
    use sha2::{Digest, Sha256};
    let source_hash = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    fs::write(packages.join("source.pptx"), &bytes)?;
    if generate_only {
        println!(
            "Generated 100-slide fixture, {} bytes, SHA-256 {source_hash}",
            bytes.len()
        );
        return Ok(());
    }
    let mut edits = Vec::new();
    let mut undos = Vec::new();
    let mut redos = Vec::new();
    let mut results = Vec::new();
    for trial in 0..3 {
        let mut session = EditSession::open(&bytes)?;
        let mut unshared_bytes = 0;
        for edit in 0..100 {
            unshared_bytes += session.bytes().len();
            let start = Instant::now();
            session.set_shape_text(0, 2, &format!("trial {trial} edit {edit}"), false)?;
            edits.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        unshared_bytes += session.bytes().len();
        let retained = session.retained_archive_bytes();
        assert_eq!(session.history_depth(), (100, 0));
        assert!(retained < bytes.len() * 3 + 100 * 65536);
        let edited = session.bytes().to_vec();
        for _ in 0..100 {
            let start = Instant::now();
            assert!(session.undo());
            undos.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        assert_eq!(session.bytes(), bytes);
        assert!(!session.is_dirty());
        for _ in 0..100 {
            let start = Instant::now();
            assert!(session.redo());
            redos.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        assert_eq!(session.bytes(), edited);
        fs::write(
            packages.join(format!("edited-trial-{trial}.pptx")),
            session.bytes(),
        )?;
        results.push(format!("{{\"trial\":{trial},\"retainedArchiveBytes\":{retained},\"derivedUnsharedArchiveBytes\":{unshared_bytes}}}"));
    }
    let report = format!(
        r#"{{
  "scope":"native_writer_session",
  "provenance":"{}","sourceSha256":"{}",
  "profile":"release","os":"{}","arch":"{}",
  "slides":100,"distinctInsertedImages":{},"archiveBytes":{},"trials":3,
  "editOperations":300,"undoOperations":300,"redoOperations":300,
  "editMedianMs":{},"editP95Ms":{},"undoMedianMs":{},"undoP95Ms":{},"redoMedianMs":{},"redoP95Ms":{},
  "trialMemory":[{}],
  "editSamplesMs":{:?},"undoSamplesMs":{:?},"redoSamplesMs":{:?}
}}
"#,
        if provided {
            "provided benchmark corpus"
        } else {
            "synthetic sample-deck plus two deterministic 1024x1024 PNGs and duplicated slides"
        },
        source_hash,
        std::env::consts::OS,
        std::env::consts::ARCH,
        if provided { "null" } else { "2" },
        bytes.len(),
        percentile(&edits, 0.5),
        percentile(&edits, 0.95),
        percentile(&undos, 0.5),
        percentile(&undos, 0.95),
        percentile(&redos, 0.5),
        percentile(&redos, 0.95),
        results.join(","),
        edits,
        undos,
        redos
    );
    fs::write(output.join("report.json"), report)?;
    println!(
        "100 slides, {} archive bytes; 300 edits/Undo/Redo complete",
        bytes.len()
    );
    Ok(())
}
