//! 確認用の CLI。
//!
//! ```bash
//! docx-diff old.docx new.docx   # 差分 JSON を標準出力へ (差分があれば終了コード 1)
//! docx-diff --ir doc.docx       # 抽出した段落 / 表を標準出力へ
//! ```

use std::path::PathBuf;

use jade_docx_diff::{DiffOptions, Limits, diff_files, load_document};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    if let ["--ir", path] = args.as_slice() {
        let document = load_document(&PathBuf::from(path), &Limits::default())?;
        println!("{}", serde_json::to_string_pretty(&document)?);
        return Ok(());
    }

    let [old, new] = args.as_slice() else {
        eprintln!("usage: docx-diff <old.docx> <new.docx>");
        eprintln!("       docx-diff --ir <doc.docx>");
        std::process::exit(2);
    };

    let diff = diff_files(
        &PathBuf::from(old),
        &PathBuf::from(new),
        &Limits::default(),
        &DiffOptions::default(),
    )?;
    println!("{}", serde_json::to_string_pretty(&diff)?);
    if diff.has_changes() {
        std::process::exit(1);
    }
    Ok(())
}
