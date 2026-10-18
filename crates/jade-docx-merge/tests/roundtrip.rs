//! 実物の docx で、合成した結果がまた読めるかを見る。
//!
//! 合成した docx をもう一度解析して、消えた条項の文字が残っていること、
//! 入った条項の文字も入っていること、並びが新の文書のままであることを
//! 確かめる。XML として壊れていれば、この解析がまず失敗する。

use std::path::PathBuf;

use docx_diff::{diff_files, DiffOptions, Limits};

fn fixture(name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var("HOME").ok()?)
        .join("Desktop/ellisii-docx-sample")
        .join(name);
    path.exists().then_some(path)
}

#[test]
fn 合成した_docx_をもう一度読める() {
    // 手元の確認用のサンプルがあるときだけ走る。CI では飛ばす。
    let (Some(old), Some(new)) = (fixture("契約書_旧.docx"), fixture("契約書_新.docx"))
    else {
        eprintln!("サンプルが無いので飛ばす");
        return;
    };

    let limits = Limits::default();
    let options = DiffOptions::default();
    let diff = diff_files(&old, &new, &limits, &options).expect("差分");

    let old_bytes = std::fs::read(&old).unwrap();
    let new_bytes = std::fs::read(&new).unwrap();
    let merged = docx_merge::merge(&old_bytes, &new_bytes, &diff).expect("合成");

    if std::env::var("DUMP").is_ok() {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&merged)).unwrap();
        let mut xml = String::new();
        use std::io::Read;
        archive
            .by_name("word/document.xml")
            .unwrap()
            .read_to_string(&mut xml)
            .unwrap();
        std::fs::write("/tmp/merged.xml", &xml).unwrap();
        std::fs::write("/tmp/merged.docx", &merged).unwrap();
        eprintln!("書いた /tmp/merged.xml ({} バイト)", xml.len());
    }

    // 合成した docx を解析し直す。ここで落ちるなら XML が壊れている。
    let parsed = docx_diff::load_document_bytes(&merged, &limits).expect("合成した docx の解析");
    let text: String = parsed
        .blocks
        .iter()
        .map(|block| format!("{block:?}"))
        .collect();

    // 新にしかない条項。
    assert!(text.contains("遅延損害金"), "入った条項が無い");
    // 旧にしかない条項。削除として残っているはず。
    assert!(text.contains("秘密保持"), "消えた条項が残っていない");
    // 表に足された行。
    assert!(text.contains("結合試験"), "足された行が無い");

    // 並びは新の文書のまま。差分のブロックの並びは文書順とは限らないので、
    // そちらを軸にすると表が先頭へ飛ぶ。
    let order: Vec<&str> = ["業務委託契約書", "別表", "結合試験"].into_iter().collect();
    let mut at = 0usize;
    for needle in order {
        let found = text[at..]
            .find(needle)
            .unwrap_or_else(|| panic!("{needle} が期待した並びに無い"));
        at += found;
    }
}
