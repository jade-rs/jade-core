//! Pins the reader's output for every .doc fixture: a hash of the `Debug`
//! of the flow model, recorded before the per-character loop was tuned.
//! `PRINT_DOC_HASHES=1 cargo test -p jade-doc-reader --test output_hash -- --nocapture`
//! prints the current values.

use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

const EXPECTED: &[(&str, u64)] = &[
    ("char-effects", 0x4dbac79eb3cba12a),
    ("contract", 0xab40c4de29b45716),
    ("header-footer-sections", 0xa7ca90d5217fae9f),
    ("hyperlink", 0xfc64db73743bb9b1),
    ("image-inline", 0x9cb44f8254468d03),
    ("numbering-restart", 0x0844b3d215f7b590),
    ("table3x2merged", 0x5a04c1e76677cd7f),
    ("yokohama_table", 0x1b5f0423f86bf565),
];

fn hash_of(name: &str) -> u64 {
    let bytes = std::fs::read(format!(
        "{}/tests/fixtures/{name}.doc",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let first = format!("{:?}", jade_doc_reader::read_doc(&bytes).unwrap());
    let second = format!("{:?}", jade_doc_reader::read_doc(&bytes).unwrap());
    assert_eq!(first, second, "{name}: output differs between two runs");
    let mut hasher = DefaultHasher::new();
    hasher.write(first.as_bytes());
    hasher.finish()
}

#[test]
fn fixture_output_is_unchanged() {
    let dir = format!("{}/tests/fixtures", env!("CARGO_MANIFEST_DIR"));
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| {
            let name = entry.unwrap().file_name().into_string().unwrap();
            name.strip_suffix(".doc").map(str::to_string)
        })
        .collect();
    names.sort();
    assert!(!names.is_empty());
    let hashes: Vec<(String, u64)> = names
        .into_iter()
        .map(|n| (hash_of(&n), n))
        .map(|(h, n)| (n, h))
        .collect();
    if std::env::var("PRINT_DOC_HASHES").is_ok() {
        for (name, hash) in &hashes {
            println!("    (\"{name}\", {hash:#018x}),");
        }
        return;
    }
    assert_eq!(hashes.len(), EXPECTED.len(), "fixture set changed");
    for ((name, hash), (expected_name, expected)) in hashes.iter().zip(EXPECTED) {
        assert_eq!(name, expected_name);
        assert_eq!(hash, expected, "{name}: reader output changed");
    }
}
