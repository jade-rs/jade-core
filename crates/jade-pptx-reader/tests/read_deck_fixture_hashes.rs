//! Pins `read_deck`'s output for every fixture deck. The values in
//! `read_deck_fixture_hashes.txt` were recorded with the code before
//! `read_deck` stopped opening the package twice.
//!
//! Run with `JADE_PRINT_DECK_HASHES=1 cargo test -p jade-pptx-reader
//! --test read_deck_fixture_hashes -- --nocapture` to print fresh values.

use std::path::{Path, PathBuf};

use jade_pptx_reader::read_deck;

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("pptx"))
        {
            out.push(path);
        }
    }
}

/// The Debug output holds HashMaps, so line order is random: hash the sorted
/// lines (FNV-1a 64) together with the line count and total byte length.
fn digest(text: &str) -> String {
    let mut lines: Vec<&str> = text.lines().collect();
    lines.sort_unstable();
    let mut hash: u64 = 0xcbf29ce484222325;
    let mut bytes = 0usize;
    for line in &lines {
        for b in line.bytes().chain(std::iter::once(b'\n')) {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        bytes += line.len() + 1;
    }
    format!("{hash:016x} lines={} bytes={bytes}", lines.len())
}

fn current() -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures");
    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();
    files
        .iter()
        .map(|path| {
            let rel = path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = std::fs::read(path).unwrap();
            let value = match read_deck(&bytes) {
                Ok(deck) => digest(&format!("{deck:#?}")),
                Err(_) => "ERR".to_string(),
            };
            format!("{rel}\t{value}")
        })
        .collect()
}

#[test]
fn read_deck_output_is_unchanged_for_every_fixture() {
    let now = current();
    if std::env::var_os("JADE_PRINT_DECK_HASHES").is_some() {
        for line in &now {
            println!("{line}");
        }
        return;
    }
    let recorded = include_str!("read_deck_fixture_hashes.txt");
    let recorded: Vec<&str> = recorded.lines().filter(|l| !l.is_empty()).collect();
    assert!(!recorded.is_empty());
    assert_eq!(now.len(), recorded.len(), "fixture set changed");
    for (got, want) in now.iter().zip(&recorded) {
        assert_eq!(got, want);
    }
}
