use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/sample-workbook.xlsx")
}

fn temporary(label: &str, extension: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "jade-xlsx-cli-{label}-{}-{nonce}.{extension}",
        std::process::id()
    ))
}

#[test]
fn batch_edits_several_sheets_and_reports_the_written_cells() {
    let edits = temporary("edits", "json");
    let output = temporary("written", "xlsx");
    std::fs::write(
        &edits,
        r#"[
          {"sheet":"売上サマリ","reference":"A4","value":"007","as_text":true},
          {"sheet":"Notes","reference":"B2","value":"=1+2","as_formula":true}
        ]"#,
    )
    .expect("write edits");

    let result = Command::new(env!("CARGO_BIN_EXE_jade-xlsx-headless-cli"))
        .args(["set-batch"])
        .arg(fixture())
        .arg(&edits)
        .args(["--output"])
        .arg(&output)
        .output()
        .expect("run set-batch");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );

    let cells: serde_json::Value = serde_json::from_slice(&result.stdout).expect("batch JSON");
    assert_eq!(cells[0]["display"], "007");
    assert_eq!(cells[1]["formula"], "1+2");

    let saved = Command::new(env!("CARGO_BIN_EXE_jade-xlsx-headless-cli"))
        .args(["cell"])
        .arg(&output)
        .args(["Notes", "B2"])
        .output()
        .expect("read saved cell");
    assert!(saved.status.success());
    let cell: serde_json::Value = serde_json::from_slice(&saved.stdout).expect("cell JSON");
    assert_eq!(cell["formula"], "1+2");

    std::fs::remove_file(edits).expect("remove edits");
    std::fs::remove_file(output).expect("remove output");
}

#[test]
fn a_bad_batch_does_not_write_a_partial_output() {
    let edits = temporary("bad-edits", "json");
    let output = temporary("partial", "xlsx");
    std::fs::write(
        &edits,
        r#"[
          {"sheet":"売上サマリ","reference":"A4","value":"would change"},
          {"sheet":"missing","reference":"A1","value":12}
        ]"#,
    )
    .expect("write edits");

    let result = Command::new(env!("CARGO_BIN_EXE_jade-xlsx-headless-cli"))
        .args(["set-batch"])
        .arg(fixture())
        .arg(&edits)
        .args(["--output"])
        .arg(&output)
        .output()
        .expect("run bad set-batch");
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("missing"));
    assert!(!output.exists(), "a failed atomic edit created an output");

    std::fs::remove_file(edits).expect("remove edits");
}

#[test]
fn batch_edits_can_stream_from_standard_input() {
    let output = temporary("stdin-written", "xlsx");
    let mut child = Command::new(env!("CARGO_BIN_EXE_jade-xlsx-headless-cli"))
        .args(["set-batch"])
        .arg(fixture())
        .arg("-")
        .args(["--output"])
        .arg(&output)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start set-batch");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(br#"[{"sheet":"Notes","reference":"C3","value":"streamed","as_text":true}]"#)
        .expect("stream edits");
    let result = child.wait_with_output().expect("finish set-batch");

    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let cells: serde_json::Value = serde_json::from_slice(&result.stdout).expect("batch JSON");
    assert_eq!(cells[0]["reference"], "C3");
    assert_eq!(cells[0]["display"], "streamed");
    assert!(output.exists());

    std::fs::remove_file(output).expect("remove output");
}

#[test]
fn batch_can_stream_the_written_workbook_to_standard_output() {
    let workdir = temporary("stdout-workdir", "dir");
    std::fs::create_dir(&workdir).expect("create workdir");
    let mut child = Command::new(env!("CARGO_BIN_EXE_jade-xlsx-headless-cli"))
        .args(["set-batch"])
        .arg(fixture())
        .arg("-")
        .args(["--output", "-"])
        .current_dir(&workdir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start streaming set-batch");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(br#"[{"sheet":"Notes","reference":"D4","value":42}]"#)
        .expect("stream edits");
    let result = child.wait_with_output().expect("finish set-batch");

    let accidental_dash = workdir.join("-");
    if accidental_dash.exists() {
        std::fs::remove_file(accidental_dash).expect("remove accidental output");
    }
    std::fs::remove_dir(workdir).expect("remove workdir");

    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        result.stdout.starts_with(b"PK"),
        "stdout was not an XLSX package: {:?}",
        &result.stdout[..result.stdout.len().min(24)]
    );

    let captured = temporary("stdout-captured", "xlsx");
    std::fs::write(&captured, &result.stdout).expect("capture stdout workbook");
    let read = Command::new(env!("CARGO_BIN_EXE_jade-xlsx-headless-cli"))
        .args(["cell"])
        .arg(&captured)
        .args(["Notes", "D4"])
        .output()
        .expect("read streamed workbook");
    assert!(
        read.status.success(),
        "{}",
        String::from_utf8_lossy(&read.stderr)
    );
    let cell: serde_json::Value = serde_json::from_slice(&read.stdout).expect("cell JSON");
    assert_eq!(cell["number"].as_f64(), Some(42.0));
    std::fs::remove_file(captured).expect("remove captured workbook");
}

#[test]
fn workbook_can_be_read_from_standard_input() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_jade-xlsx-headless-cli"))
        .args(["cell", "-", "Notes", "A1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start stdin workbook read");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(&std::fs::read(fixture()).expect("fixture bytes"))
        .expect("stream workbook");
    let result = child
        .wait_with_output()
        .expect("finish stdin workbook read");

    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let cell: serde_json::Value = serde_json::from_slice(&result.stdout).expect("cell JSON");
    assert_eq!(cell["display"], "備考");
}

#[test]
fn find_reports_formula_source_matches() {
    let result = Command::new(env!("CARGO_BIN_EXE_jade-xlsx-headless-cli"))
        .args(["find"])
        .arg(fixture())
        .arg("SUM(B4:C4)")
        .output()
        .expect("find formula");

    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let hits: serde_json::Value = serde_json::from_slice(&result.stdout).expect("find JSON");
    assert_eq!(hits.as_array().map(Vec::len), Some(1));
    assert_eq!(hits[0]["reference"], "D4");
    assert_eq!(hits[0]["formula"], "SUM(B4:C4)");
}

#[test]
fn single_edit_can_transform_a_workbook_stream() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_jade-xlsx-headless-cli"))
        .args(["set", "-", "Notes", "B2", "42", "--output", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start streamed transform");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(&std::fs::read(fixture()).expect("fixture bytes"))
        .expect("stream workbook");
    let result = child.wait_with_output().expect("finish streamed transform");

    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout.starts_with(b"PK"));

    let captured = temporary("stdin-transform", "xlsx");
    std::fs::write(&captured, result.stdout).expect("capture transformed workbook");
    let read = Command::new(env!("CARGO_BIN_EXE_jade-xlsx-headless-cli"))
        .args(["cell"])
        .arg(&captured)
        .args(["Notes", "B2"])
        .output()
        .expect("read transformed workbook");
    assert!(read.status.success());
    let cell: serde_json::Value = serde_json::from_slice(&read.stdout).expect("cell JSON");
    assert_eq!(cell["number"].as_f64(), Some(42.0));
    std::fs::remove_file(captured).expect("remove captured workbook");
}

#[test]
fn batch_rejects_two_standard_input_sources() {
    let result = Command::new(env!("CARGO_BIN_EXE_jade-xlsx-headless-cli"))
        .args(["set-batch", "-", "-", "--output", "-"])
        .output()
        .expect("run ambiguous stdin command");

    assert!(!result.status.success());
    assert_eq!(
        String::from_utf8_lossy(&result.stderr).trim(),
        "set-batch cannot read both the workbook and edits from standard input"
    );
    assert!(result.stdout.is_empty());
}

#[test]
fn read_commands_reject_write_only_flags() {
    let output = temporary("ignored-read-output", "xlsx");
    let result = Command::new(env!("CARGO_BIN_EXE_jade-xlsx-headless-cli"))
        .args(["summary"])
        .arg(fixture())
        .args(["--output"])
        .arg(&output)
        .output()
        .expect("run summary with invalid output flag");

    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("--output is only valid"));
    assert!(!output.exists());
}
