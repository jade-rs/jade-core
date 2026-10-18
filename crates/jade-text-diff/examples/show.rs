//! 内訳を目で見るための確認用。
//!
//! ```bash
//! cargo run -p structural-text-diff --example show
//! ```

use docx_diff::SegmentKind;
use text_diff::Segmenter;

fn main() -> anyhow::Result<()> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src-tauri/resources/dict/system.dic.zst"
    );
    let segmenter = match Segmenter::from_dictionary(path) {
        Ok(s) => s,
        Err(error) => {
            eprintln!("辞書を読めないので 1 文字ずつで出す: {error:#}");
            Segmenter::char_only()
        }
    };

    let cases = [
        (
            "第9条（協議事項） 本契約に定めのない事項については、甲乙誠意をもって協議のうえ定める。",
            "第9条（協議事項） 本契約に定めのない事項および本契約の解釈に疑義が生じた事項については、甲乙は誠実に協議し、円満な解決を図るものとする。",
        ),
        (
            "第3条（委託料） 委託料は月額 500,000 円（消費税別）とする。",
            "第3条（委託料） 委託料は月額 550,000 円（消費税別）とする。",
        ),
    ];

    for (old, new) in cases {
        println!("---");
        let mut line = String::new();
        for segment in segmenter.segments(old, new) {
            match segment.kind {
                SegmentKind::Equal => line.push_str(&segment.text),
                SegmentKind::Delete => line.push_str(&format!("[-{}-]", segment.text)),
                SegmentKind::Insert => line.push_str(&format!("{{+{}+}}", segment.text)),
            }
        }
        println!("{line}");
    }
    Ok(())
}
