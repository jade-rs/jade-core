use std::fs;
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
fn extract_all_public_fixtures() {
    let dir = fixtures_dir();
    let mut failures = Vec::new();

    for entry in fs::read_dir(&dir).expect("read fixtures dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("docx") {
            continue;
        }

        if let Err(err) = jade_docx_extract::extract_from_path(&path) {
            failures.push((path, err));
        }
    }

    if !failures.is_empty() {
        panic!(
            "Failed to extract {} fixtures:\n{}",
            failures.len(),
            failures
                .into_iter()
                .map(|(path, err)| format!("  {} -> {err}", path.display()))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
