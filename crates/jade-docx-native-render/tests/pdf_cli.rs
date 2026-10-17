mod common;
use std::path::PathBuf;
use std::process::Command;

fn output_path(name: &str) -> PathBuf {
    // A separate Cargo target directory must not implicitly create this test's
    // output directory (CI uses CARGO_TARGET_DIR outside the crate workspace).
    let directory =
        std::env::temp_dir().join(format!("jade-pdf-cli-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    directory.join("output.pdf")
}

#[test]
fn headless_pdf_command_uses_the_editor_renderer() {
    let binary = env!("CARGO_BIN_EXE_docx_to_pdf");
    let output = output_path("editor");
    let result = Command::new(binary)
        .arg(common::repo_path("public/fixtures/hello.docx"))
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let pdf = lopdf::Document::load(&output).unwrap();
    assert_eq!(pdf.get_pages().len(), 2);
    assert!(pdf
        .extract_text(&[1])
        .unwrap()
        .replace('\n', "")
        .contains("Hello"));
    std::fs::remove_dir_all(output.parent().unwrap()).unwrap();
}

#[test]
fn headless_pdf_command_rejects_an_unreadable_font_before_writing() {
    let binary = env!("CARGO_BIN_EXE_docx_to_pdf");
    let output = output_path("invalid-font");
    let _ = std::fs::remove_file(&output);
    let result = Command::new(binary)
        .arg(common::repo_path("public/fixtures/hello.docx"))
        .arg(&output)
        .args(["--font", "My font", "400", "/missing/jade-font.ttf"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("/missing/jade-font.ttf"));
    assert!(!output.exists());
    std::fs::remove_dir_all(output.parent().unwrap()).unwrap();
}
