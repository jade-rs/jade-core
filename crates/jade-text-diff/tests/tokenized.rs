//! 辞書を読める環境で、語で切れているかを見る。
//!
//! 辞書は実行時に読む外部ファイルなので、置いていなければ飛ばす
//! (`scripts/fetch-dict.sh` が用意する)。

use std::path::PathBuf;

use docx_diff::SegmentKind;
use text_diff::Segmenter;

fn dictionary() -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../src-tauri/resources/dict/system.dic.zst");
    path.exists().then_some(path)
}

fn segmenter() -> Option<Segmenter> {
    Segmenter::from_dictionary(dictionary()?).ok()
}

/// 内訳から、片側だけの文字列を組み直す。
fn side(segments: &[docx_diff::Segment], keep: SegmentKind) -> String {
    segments
        .iter()
        .filter(|s| s.kind == SegmentKind::Equal || s.kind == keep)
        .map(|s| s.text.as_str())
        .collect()
}

#[test]
fn 語のまとまりで置き換わる() {
    let Some(segmenter) = segmenter() else {
        eprintln!("辞書が無いので飛ばす");
        return;
    };
    assert!(segmenter.has_dictionary());

    let old = "甲乙は誠意をもって協議し";
    let new = "甲乙は誠実に協議し";
    let segments = segmenter.segments(old, new);

    // 1 文字ずつだと「誠」だけ残って「意をもって」「実に」が虫食いに出る。
    // 語で切れていれば、消えた側に「誠意」がまとまって出るはず。
    let deleted: String = segments
        .iter()
        .filter(|s| s.kind == SegmentKind::Delete)
        .map(|s| s.text.as_str())
        .collect();
    assert!(
        deleted.contains("誠意"),
        "「誠意」がまとまって消えていない: {segments:?}"
    );
}

#[test]
fn 繋ぎ直すと元の文字列に戻る() {
    let Some(segmenter) = segmenter() else {
        eprintln!("辞書が無いので飛ばす");
        return;
    };

    // 空白や記号、英数の混ざったもの。切って繋いで元に戻らないと、
    // 内訳のバイト位置が本文とずれて、装飾が別の場所に乗る。
    for (old, new) in [
        (
            "第3条（委託料） 委託料は月額 500,000 円（消費税別）とする。",
            "第3条（委託料） 委託料は月額 550,000 円（消費税別）とする。",
        ),
        ("ABC 123 あいう", "ABC 456 あいう"),
        ("　全角空白から始まる", "　全角空白から始まる文"),
        ("", "新しく入った文"),
        ("消えた文", ""),
    ] {
        let segments = segmenter.segments(old, new);
        assert_eq!(
            side(&segments, SegmentKind::Delete),
            old,
            "変更前が戻らない"
        );
        assert_eq!(
            side(&segments, SegmentKind::Insert),
            new,
            "変更後が戻らない"
        );
    }
}

#[test]
fn 変わっていない文は内訳が_1_つ() {
    let Some(segmenter) = segmenter() else {
        eprintln!("辞書が無いので飛ばす");
        return;
    };
    let text = "本契約の成立を証するため、本書 2 通を作成する。";
    let segments = segmenter.segments(text, text);
    assert_eq!(segments.len(), 1, "{segments:?}");
    assert_eq!(segments[0].kind, SegmentKind::Equal);
}

#[test]
fn 数字の_1_文字違いは数字だけに出る() {
    let Some(segmenter) = segmenter() else {
        eprintln!("辞書が無いので飛ばす");
        return;
    };
    let segments = segmenter.segments("月額 500,000 円", "月額 550,000 円");
    let inserted: String = segments
        .iter()
        .filter(|s| s.kind == SegmentKind::Insert)
        .map(|s| s.text.as_str())
        .collect();
    // 語で切ると数字はひとかたまりになる。「月額」や「円」まで巻き込まない。
    assert!(
        !inserted.contains('月'),
        "関係ない語まで動いた: {segments:?}"
    );
    assert!(
        !inserted.contains('円'),
        "関係ない語まで動いた: {segments:?}"
    );
}
