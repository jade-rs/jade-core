//! 構造を見て docx を比べる。
//!
//! 段落 → 表 → 行 → 文字 の順に対応付けてから中身を比べる。先に対応を付ける
//! ので、途中に 1 段落挟まっても以降が全部差分になることはない。
//!
//! 対応付けの優先順位:
//!
//! 1. `w14:paraId` — Word が振る段落の識別子。同じ文書から派生した 2 つの
//!    ファイルなら、直していない段落はここが一致する。いちばん確かな手掛かり。
//! 2. 正規化したテキストの一致 — paraId が無い / 振り直された場合の錨。
//! 3. 文字 bigram の Dice 係数 — 上の 2 つで固まらなかった隙間だけ総当たり。
//!
//! ```no_run
//! use std::path::Path;
//! let diff = jade_docx_diff::diff_files(
//!     Path::new("old.docx"),
//!     Path::new("new.docx"),
//!     &jade_docx_diff::Limits::default(),
//!     &jade_docx_diff::DiffOptions::default(),
//! )?;
//! println!("{}", serde_json::to_string_pretty(&diff)?);
//! # Ok::<(), anyhow::Error>(())
//! ```

mod diff;
mod ir;
mod parse;

pub use diff::{
    BlockChange, ChangeKind, DIFF_SCHEMA_VERSION, DiffOptions, DiffSummary, DocumentDiff,
    RowChange, Segment, SegmentKind, Status, diff_documents,
};
pub use ir::{BlockIr, CellIr, DocumentIr, ParagraphIr, RowIr, TableIr, normalize};
pub use parse::{Limits, load_document, load_document_bytes};

use std::path::Path;

/// 2 つの docx を読んで比べる。
pub fn diff_files(
    old: &Path,
    new: &Path,
    limits: &Limits,
    options: &DiffOptions,
) -> anyhow::Result<DocumentDiff> {
    let old_ir = load_document(old, limits)?;
    let new_ir = load_document(new, limits)?;
    Ok(diff_documents(
        &old.display().to_string(),
        &new.display().to_string(),
        &old_ir,
        &new_ir,
        options,
    ))
}
