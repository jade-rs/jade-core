//! 実物の docx で、語で切り直した内訳を見る。
//!
//! ```bash
//! cargo run -p structural-text-diff --example docx -- 旧.docx 新.docx
//! ```

use std::path::PathBuf;

use docx_diff::{diff_files, DiffOptions, Limits, SegmentKind};
use text_diff::{resegment_docx, Segmenter};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [old, new] = args.as_slice() else {
        eprintln!("usage: docx <old.docx> <new.docx>");
        std::process::exit(2);
    };

    let mut diff = diff_files(
        &PathBuf::from(old),
        &PathBuf::from(new),
        &Limits::default(),
        &DiffOptions::default(),
    )?;

    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src-tauri/resources/dict/system.dic.zst"
    );
    let segmenter = Segmenter::from_dictionary(path).unwrap_or_else(|_| Segmenter::char_only());
    println!(
        "辞書: {}",
        if segmenter.has_dictionary() {
            "あり"
        } else {
            "無し (1 文字ずつ)"
        }
    );
    resegment_docx(&mut diff, &segmenter);

    fn show(blocks: &[docx_diff::BlockChange], indent: &str) {
        for block in blocks {
            match block {
                docx_diff::BlockChange::Paragraph(change) => {
                    if change.segments.is_empty() {
                        continue;
                    }
                    let mut line = String::new();
                    for segment in &change.segments {
                        match segment.kind {
                            SegmentKind::Equal => line.push_str(&segment.text),
                            SegmentKind::Delete => line.push_str(&format!("[-{}-]", segment.text)),
                            SegmentKind::Insert => {
                                line.push_str(&format!("{{+{}+}}", segment.text))
                            }
                        }
                    }
                    println!("{indent}{line}");
                }
                docx_diff::BlockChange::Table(change) => {
                    for row in &change.rows {
                        for cell in &row.cells {
                            show(&cell.blocks, &format!("{indent}  表 "));
                        }
                    }
                }
            }
        }
    }
    show(&diff.blocks, "");
    Ok(())
}
