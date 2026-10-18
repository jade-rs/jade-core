use std::process::{Command, Stdio};

use rust_xlsxwriter::Workbook;
use tempfile::tempdir;

#[test]
fn check_mode_uses_diff_exit_status_after_writing_report() {
    let dir = tempdir().unwrap();
    let old = dir.path().join("old.xlsx");
    let new = dir.path().join("new.xlsx");
    let report = dir.path().join("diff.json");
    write_value(&old, "old");
    write_value(&new, "new");

    let status = Command::new(env!("CARGO_BIN_EXE_xlsx-diff"))
        .arg(&old)
        .arg(&new)
        .arg("--json")
        .arg(&report)
        .arg("--check")
        .status()
        .unwrap();

    assert_eq!(status.code(), Some(1));
    assert!(report.is_file());
}

#[test]
fn check_mode_returns_success_for_equal_workbooks() {
    let dir = tempdir().unwrap();
    let old = dir.path().join("old.xlsx");
    let new = dir.path().join("new.xlsx");
    write_value(&old, "same");
    write_value(&new, "same");

    let status = Command::new(env!("CARGO_BIN_EXE_xlsx-diff"))
        .arg(&old)
        .arg(&new)
        .arg("--check")
        .stdout(Stdio::null())
        .status()
        .unwrap();

    assert!(status.success());
}

#[test]
fn side_by_side_mode_writes_html_and_xlsx_reports() {
    let dir = tempdir().unwrap();
    let old = dir.path().join("old.xlsx");
    let new = dir.path().join("new.xlsx");
    let html = dir.path().join("diff.html");
    let xlsx = dir.path().join("diff.xlsx");
    write_value(&old, "old");
    write_value(&new, "new");

    let status = Command::new(env!("CARGO_BIN_EXE_xlsx-diff"))
        .arg(&old)
        .arg(&new)
        .arg("--html")
        .arg(&html)
        .arg("--xlsx")
        .arg(&xlsx)
        .arg("--report-layout")
        .arg("side-by-side")
        .status()
        .unwrap();

    assert!(status.success());
    assert!(xlsx.is_file());
    assert!(
        std::fs::read_to_string(html)
            .unwrap()
            .contains("class=\"side-by-side\"")
    );
}

fn write_value(path: &std::path::Path, value: &str) {
    let mut workbook = Workbook::new();
    workbook.add_worksheet().write_string(0, 0, value).unwrap();
    workbook.save(path).unwrap();
}
