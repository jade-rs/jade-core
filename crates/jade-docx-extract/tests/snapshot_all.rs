use std::fs;
use std::path::{Path, PathBuf};

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn collect_docx(dir: &Path, acc: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_docx(&path, acc);
            } else if path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("docx"))
                .unwrap_or(false)
            {
                acc.push(path);
            }
        }
    }
}

#[test]
fn snapshot_all_docx_fixtures() {
    let root = fixtures_root();
    let mut files = Vec::new();
    collect_docx(&root, &mut files);
    files.sort();

    for path in files {
        let rel = path.strip_prefix(&root).unwrap_or(&path);
        let snapshot_name = format!(
            "docx__{}",
            rel.to_string_lossy().replace('\\', "/").replace('/', "__")
        );

        let paragraphs = jade_docx_extract::extract_from_path(&path).unwrap_or_else(|err| {
            panic!("failed to extract {}: {err}", path.display());
        });

        insta::assert_yaml_snapshot!(snapshot_name, paragraphs);
    }
}
