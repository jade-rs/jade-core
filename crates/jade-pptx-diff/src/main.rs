//! 確認用の CLI。
//!
//! ```bash
//! pptx-diff old.pptx new.pptx   # 差分 JSON を標準出力へ (差分があれば終了コード 1)
//! pptx-diff --ir deck.pptx      # 描画用の中間表現を標準出力へ
//! ```

use std::path::PathBuf;

use pptx_diff::{diff_files, load_presentation_file, DiffOptions, Limits};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    if let ["--ir", path] = args.as_slice() {
        let deck = load_presentation_file(&PathBuf::from(path), &Limits::default())?;
        println!("{}", serde_json::to_string_pretty(&deck)?);
        return Ok(());
    }

    let [old, new] = args.as_slice() else {
        eprintln!("usage: pptx-diff <old.pptx> <new.pptx>");
        eprintln!("       pptx-diff --ir <deck.pptx>");
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
