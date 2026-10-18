//! 構造を見て比較する PowerPoint (.pptx) の差分ライブラリ。
//!
//! スライドと図形を先に対応付けてから中身を比べるので、スライドを 1 枚
//! 差し込んでも以降が全部差分になることはない。粒度は
//! スライド → 図形 → 段落 (と表のセル) の 3 段。
//!
//! 描画にも同じ中間表現 ([`PresentationIr`]) を使う。差分だけでは元の
//! スライドを再現できないので、UI 側は IR を読んで描き、差分を重ねる。
//!
//! ```no_run
//! use pptx_diff::{diff_presentations, load_presentation, DiffOptions, Limits};
//!
//! let old_bytes = std::fs::read("old.pptx")?;
//! let new_bytes = std::fs::read("new.pptx")?;
//! let old = load_presentation(&old_bytes, &Limits::default())?;
//! let new = load_presentation(&new_bytes, &Limits::default())?;
//! let diff = diff_presentations("old.pptx", "new.pptx", &old, &new, &DiffOptions::default());
//! println!("{} slides changed", diff.summary.slides_modified);
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! ## 比較の対象外
//!
//! 配色・フォントテーマ・アニメーション・画面切り替え・グラフの系列データ・
//! SmartArt の内部構造・埋め込みオブジェクトは比較しない。図形は種別
//! (テキスト / 表 / 画像 / その他) と位置・テキストで対応付ける。

mod diff;
mod ir;
mod parse;
mod xml;

pub use diff::{
    diff_presentations, ChangeKind, DiffOptions, DiffSummary, ParagraphChange, PresentationDiff,
    Segment, SegmentKind, ShapeDiff, SlideDiff, SlideSize, Status, TableCellChange, TableDiff,
    TableRowChange, DIFF_SCHEMA_VERSION,
};
pub use ir::{
    FormatSpanIr, Frame, ImageIr, ParagraphIr, Placeholder, PresentationIr, ShapeIr, ShapeKind,
    SlideIr, TableIr,
};
pub use parse::{load_presentation, Limits};

use std::path::Path;

/// ファイルパスから読み込む。
pub fn load_presentation_file(path: &Path, limits: &Limits) -> anyhow::Result<PresentationIr> {
    let bytes = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", path.display()))?;
    load_presentation(&bytes, limits)
}

/// 2 ファイルを読み込んで比較する。
pub fn diff_files(
    old_path: &Path,
    new_path: &Path,
    limits: &Limits,
    options: &DiffOptions,
) -> anyhow::Result<PresentationDiff> {
    let old = load_presentation_file(old_path, limits)?;
    let new = load_presentation_file(new_path, limits)?;
    Ok(diff_presentations(
        &old_path.display().to_string(),
        &new_path.display().to_string(),
        &old,
        &new,
        options,
    ))
}
