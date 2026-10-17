//! 段落 → 表 → 行 → 文字 の 4 段で比べる。
//!
//! どの段でも「先に対応を付けてから中身を比べる」形は同じで、対応付けは
//! [`jade_diff_align`] に任せる。内容が一致するものを錨として固定し、隙間だけ
//! 総当たりするので、長い文書でも現実的な時間で終わる。

use jade_diff_align::{Pairing, Step, lcs_diff, pair_with_anchors, similarity};
use serde::Serialize;
use std::hash::Hash;

use crate::ir::{BlockIr, CellIr, DocumentIr, ParagraphIr, RowIr, TableIr, normalize};

/// 出力 JSON の契約バージョン。
pub const DIFF_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct DiffOptions {
    /// 段落同士を同じ段落とみなす最小スコア。
    pub paragraph_match_score: f64,
    /// 表の行同士を同じ行とみなす最小スコア。
    pub row_match_score: f64,
    /// 1 区間あたりに許す総当たりの組数。
    pub max_pairwise: usize,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            paragraph_match_score: 0.34,
            row_match_score: 0.34,
            max_pairwise: 40_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Added,
    Removed,
    Modified,
    Moved,
    Unchanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Removed,
    Changed,
    Moved,
    Unchanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentKind {
    Equal,
    Insert,
    Delete,
}

#[derive(Debug, Clone, Serialize)]
pub struct Segment {
    pub kind: SegmentKind,
    pub text: String,
}

/// 本文の並びに現れるものの差分 1 件。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "block", rename_all = "snake_case")]
pub enum BlockChange {
    Paragraph(ParagraphChange),
    Table(TableChange),
}

impl BlockChange {
    pub fn kind(&self) -> ChangeKind {
        match self {
            BlockChange::Paragraph(change) => change.kind,
            BlockChange::Table(change) => change.kind,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ParagraphChange {
    pub kind: ChangeKind,
    pub old_index: Option<usize>,
    pub new_index: Option<usize>,
    pub old_text: Option<String>,
    pub new_text: Option<String>,
    /// 番号を付けない本文。画面上の段落と突き合わせるのはこちら。
    ///
    /// `old_text` / `new_text` は箇条書きの番号を含む。番号は numbering.xml から
    /// 組み立てられるもので文書の中に run として存在しないため、描画されている
    /// 段落に装飾を重ねたい側が使えるのは番号なしのほうになる。
    pub old_body_text: Option<String>,
    pub new_body_text: Option<String>,
    /// `w14:paraId` が一致したことで対応が付いたか。
    ///
    /// 一致していれば Word 上で同じ段落だと言い切れる。文面が大きく変わって
    /// いても対応は確かなので、画面でもそう見せられる。
    pub matched_by_para_id: bool,
    /// 位置だけが動いたか。
    pub moved: bool,
    /// 変更履歴が残ったままの段落か (どちらかの側で)。
    pub has_tracked_changes: bool,
    /// `changed` のときだけ入る文字単位の内訳。
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TableChange {
    pub kind: ChangeKind,
    pub old_index: Option<usize>,
    pub new_index: Option<usize>,
    pub moved: bool,
    pub rows: Vec<RowChange>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RowChange {
    pub kind: ChangeKind,
    pub old_index: Option<usize>,
    pub new_index: Option<usize>,
    pub moved: bool,
    pub cells: Vec<CellChange>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CellChange {
    pub kind: ChangeKind,
    pub old_index: Option<usize>,
    pub new_index: Option<usize>,
    /// セルの中身。表の入れ子もここに入る。
    pub blocks: Vec<BlockChange>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DiffSummary {
    pub paragraphs_added: usize,
    pub paragraphs_removed: usize,
    pub paragraphs_changed: usize,
    pub paragraphs_moved: usize,
    pub tables_added: usize,
    pub tables_removed: usize,
    pub table_rows_added: usize,
    pub table_rows_removed: usize,
    pub table_rows_changed: usize,
}

/// 結果を読むときに知っておくべきこと。
#[derive(Debug, Clone, Default, Serialize)]
pub struct Diagnostics {
    pub old_paragraphs: usize,
    pub new_paragraphs: usize,
    /// `w14:paraId` を持つ段落の数。少ないと対応付けは本文頼りになる。
    pub old_paragraphs_with_para_id: usize,
    pub new_paragraphs_with_para_id: usize,
    /// 変更履歴が残ったままの段落の数。
    ///
    /// 比較は履歴を反映した後 (accept 後) の本文で行う。履歴が残っていると
    /// 画面に見えている文字と違うものを比べることになるので、断っておく。
    pub old_paragraphs_with_tracked_changes: usize,
    pub new_paragraphs_with_tracked_changes: usize,
}

impl Diagnostics {
    /// paraId がほとんど無い文書か。
    ///
    /// Word 以外が書き出した docx では paraId が振られないことがある。その場合
    /// 対応付けは本文の一致と類似度だけが頼りになる。
    pub fn para_ids_missing(&self) -> bool {
        let old_missing =
            self.old_paragraphs > 0 && self.old_paragraphs_with_para_id * 2 < self.old_paragraphs;
        let new_missing =
            self.new_paragraphs > 0 && self.new_paragraphs_with_para_id * 2 < self.new_paragraphs;
        old_missing || new_missing
    }

    /// 変更履歴が残っているか。
    pub fn has_tracked_changes(&self) -> bool {
        self.old_paragraphs_with_tracked_changes > 0 || self.new_paragraphs_with_tracked_changes > 0
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DocumentDiff {
    pub schema_version: u32,
    pub old_file: String,
    pub new_file: String,
    pub summary: DiffSummary,
    pub diagnostics: Diagnostics,
    pub blocks: Vec<BlockChange>,
}

impl DocumentDiff {
    pub fn has_changes(&self) -> bool {
        self.blocks
            .iter()
            .any(|block| block.kind() != ChangeKind::Unchanged)
    }
}

/// 2 つの docx を比較する。
pub fn diff_documents(
    old_file: &str,
    new_file: &str,
    old: &DocumentIr,
    new: &DocumentIr,
    options: &DiffOptions,
) -> DocumentDiff {
    let blocks = diff_blocks(&old.blocks, &new.blocks, options);

    let mut summary = DiffSummary::default();
    for block in &blocks {
        accumulate(&mut summary, block);
    }

    let mut diagnostics = Diagnostics::default();
    collect_diagnostics(&old.blocks, &mut diagnostics, true);
    collect_diagnostics(&new.blocks, &mut diagnostics, false);

    DocumentDiff {
        schema_version: DIFF_SCHEMA_VERSION,
        old_file: old_file.to_string(),
        new_file: new_file.to_string(),
        summary,
        diagnostics,
        blocks,
    }
}

/// 本文の並びを対応付けて比べる。表の中のセルからも呼ばれる。
fn diff_blocks(old: &[BlockIr], new: &[BlockIr], options: &DiffOptions) -> Vec<BlockChange> {
    let old_keys: Vec<_> = old.iter().map(anchor_key).collect();
    let new_keys: Vec<_> = new.iter().map(anchor_key).collect();

    let mut pairings = pair_with_anchors(
        &old_keys,
        &new_keys,
        options.paragraph_match_score,
        options.max_pairwise,
        |i, j| block_score(&old[i], &new[j]),
    );
    recover_moves(&mut pairings, &old_keys, &new_keys, old, new);

    let mut out: Vec<BlockChange> = Vec::new();
    for pairing in &pairings {
        match *pairing {
            Pairing::Matched {
                old: o,
                new: n,
                moved,
            } => out.push(diff_block(&old[o], &new[n], moved, options)),
            Pairing::Removed(o) => out.push(one_sided_block(&old[o], true)),
            Pairing::Added(n) => out.push(one_sided_block(&new[n], false)),
        }
    }

    // 新しいほうの並びで見せる。消えたものは旧の並びで直前だった位置の後ろへ。
    out.sort_by_key(|block| (new_order(block), old_order(block)));
    reposition_removed(&mut out);
    out
}

/// 順番が入れ替わっただけのものを拾い直す。
///
/// 錨は順序を保てるものしか選べないので、後ろへ動かした段落は錨から外れて
/// 「消えた + 生えた」に割れる。鍵が同じまま位置だけ変わっているなら、それは
/// 移動として報告したい。段落 1 つを章の末尾へ送るのはよくある直し方で、
/// 追加と削除として出すと差分の量が実際の倍に見える。
fn recover_moves<K: Hash + Eq>(
    pairings: &mut Vec<Pairing>,
    old_keys: &[K],
    new_keys: &[K],
    old: &[BlockIr],
    new: &[BlockIr],
) {
    use std::collections::{HashMap, VecDeque};

    let mut removed: HashMap<&K, VecDeque<usize>> = HashMap::new();
    for pairing in pairings.iter() {
        if let Pairing::Removed(index) = *pairing {
            if !is_moveable(&old[index]) {
                continue;
            }
            removed
                .entry(&old_keys[index])
                .or_default()
                .push_back(index);
        }
    }
    if removed.is_empty() {
        return;
    }

    let mut paired: HashMap<usize, usize> = HashMap::new();
    for pairing in pairings.iter() {
        let Pairing::Added(new_index) = *pairing else {
            continue;
        };
        if !is_moveable(&new[new_index]) {
            continue;
        }
        // 同じ鍵が複数残っているときは出現順に組ませる。
        if let Some(candidates) = removed.get_mut(&new_keys[new_index])
            && let Some(old_index) = candidates.pop_front()
        {
            paired.insert(old_index, new_index);
        }
    }
    if paired.is_empty() {
        return;
    }

    let moved_new: std::collections::HashSet<usize> = paired.values().copied().collect();
    pairings.retain(|pairing| !matches!(pairing, Pairing::Added(n) if moved_new.contains(n)));
    for pairing in pairings.iter_mut() {
        if let Pairing::Removed(old_index) = *pairing
            && let Some(new_index) = paired.get(&old_index)
        {
            *pairing = Pairing::Matched {
                old: old_index,
                new: *new_index,
                moved: true,
            };
        }
    }
}

/// 移動として拾い直してよいものか。
///
/// 空の段落は文書のあちこちにあり、どれとどれが同じものだったのかを決める
/// 手掛かりがない (Word はコピー & ペーストで paraId ごと複製するので、
/// 識別子も当てにならない)。読み手にとっても、空行が動いたという報告に
/// 価値はない。中身のあるものだけを移動として扱う。
fn is_moveable(block: &BlockIr) -> bool {
    let text = match block {
        BlockIr::Paragraph(paragraph) => &paragraph.text,
        BlockIr::Table(table) => &table.signature(),
    };
    is_moveable_text(text)
}

/// 移動として報告する値のある本文か。
fn is_moveable_text(text: &str) -> bool {
    normalize(text).chars().count() >= 2
}

/// 錨に使う鍵。
///
/// paraId があればそれを最優先にする。Word が振る識別子なので、同じ文書から
/// 派生した 2 つのファイルでは、直していない段落がこれで一発で揃う。無いとき
/// (Word 以外が書き出した docx など) は正規化した本文で代える。
#[derive(Debug, PartialEq, Eq, Hash)]
enum AnchorKey<'a> {
    ParagraphId(&'a str),
    ParagraphText(String),
    Table(String),
}

fn anchor_key(block: &BlockIr) -> AnchorKey<'_> {
    match block {
        BlockIr::Paragraph(paragraph) => match &paragraph.para_id {
            Some(para_id) => AnchorKey::ParagraphId(para_id),
            None => AnchorKey::ParagraphText(normalize(&paragraph.text)),
        },
        BlockIr::Table(table) => AnchorKey::Table(normalize(&table.signature())),
    }
}

/// 文字列の似ぐあい。0.0 〜 1.0。
///
/// 長い文には文字 bigram の Dice 係数がよく効くが、短い文では bigram の数が
/// 足りずに破綻する。「第3回」と「第4回」は共通の bigram が 1 つも無いので
/// 0.0 になってしまう。表のセルはこの長さのものばかりなので、短いときは
/// 文字単位の一致数 (LCS) で測り直す。
fn score_text(old: &str, new: &str) -> f64 {
    const SHORT: usize = 8;

    let dice = similarity(old, new);
    let old_chars: Vec<char> = old.chars().filter(|c| !c.is_whitespace()).collect();
    let new_chars: Vec<char> = new.chars().filter(|c| !c.is_whitespace()).collect();
    if old_chars.len() > SHORT && new_chars.len() > SHORT {
        return dice;
    }
    if old_chars.is_empty() && new_chars.is_empty() {
        return 1.0;
    }
    if old_chars.is_empty() || new_chars.is_empty() {
        return 0.0;
    }

    let common = lcs_diff(&old_chars, &new_chars)
        .iter()
        .filter(|step| matches!(step, Step::Equal(..)))
        .count();
    let ratio = 2.0 * common as f64 / (old_chars.len() + new_chars.len()) as f64;
    ratio.max(dice)
}

/// 錨で決まらなかった隙間を埋めるためのスコア。
fn block_score(old: &BlockIr, new: &BlockIr) -> f64 {
    match (old, new) {
        (BlockIr::Paragraph(a), BlockIr::Paragraph(b)) => {
            // paraId が一致するなら、文面がどれだけ変わっていても同じ段落。
            // 全面的に書き直された条文がここで拾える。
            if let (Some(old_id), Some(new_id)) = (&a.para_id, &b.para_id)
                && old_id == new_id
            {
                return 1.0;
            }
            score_text(&normalize(&a.text), &normalize(&b.text))
        }
        (BlockIr::Table(a), BlockIr::Table(b)) => {
            score_text(&normalize(&a.text()), &normalize(&b.text()))
        }
        // 段落と表は別物。
        _ => 0.0,
    }
}

fn diff_block(old: &BlockIr, new: &BlockIr, moved: bool, options: &DiffOptions) -> BlockChange {
    match (old, new) {
        (BlockIr::Paragraph(a), BlockIr::Paragraph(b)) => {
            BlockChange::Paragraph(diff_paragraph(a, b, moved))
        }
        (BlockIr::Table(a), BlockIr::Table(b)) => {
            BlockChange::Table(diff_table(a, b, moved, options))
        }
        // 対応付けが段落と表を組ませることはない (スコアが 0)。念のため
        // 片側ずつに割っておく。
        _ => one_sided_block(new, false),
    }
}

fn diff_paragraph(old: &ParagraphIr, new: &ParagraphIr, moved: bool) -> ParagraphChange {
    let matched_by_para_id = match (&old.para_id, &new.para_id) {
        (Some(old_id), Some(new_id)) => old_id == new_id,
        _ => false,
    };
    let text_changed = old.text != new.text;

    ParagraphChange {
        kind: if text_changed {
            ChangeKind::Changed
        } else if moved && is_moveable_text(&new.text) {
            // 空行が動いたという報告に読み手の得るものはない。対応付けの
            // 途中でも並び替えは起きるので、報告するところで落とす。
            ChangeKind::Moved
        } else {
            ChangeKind::Unchanged
        },
        old_index: Some(old.index),
        new_index: Some(new.index),
        old_text: Some(old.text.clone()),
        new_text: Some(new.text.clone()),
        old_body_text: Some(old.body_text.clone()),
        new_body_text: Some(new.body_text.clone()),
        matched_by_para_id,
        moved,
        has_tracked_changes: old.has_tracked_changes() || new.has_tracked_changes(),
        segments: if text_changed {
            segment_text(&old.text, &new.text)
        } else {
            Vec::new()
        },
    }
}

fn diff_table(old: &TableIr, new: &TableIr, moved: bool, options: &DiffOptions) -> TableChange {
    let old_keys: Vec<String> = old.rows.iter().map(row_key).collect();
    let new_keys: Vec<String> = new.rows.iter().map(row_key).collect();

    let pairings = pair_with_anchors(
        &old_keys,
        &new_keys,
        options.row_match_score,
        options.max_pairwise,
        |i, j| {
            score_text(
                &normalize(&old.rows[i].text()),
                &normalize(&new.rows[j].text()),
            )
        },
    );

    let mut rows: Vec<RowChange> = Vec::new();
    for pairing in &pairings {
        match *pairing {
            Pairing::Matched {
                old: o,
                new: n,
                moved,
            } => rows.push(diff_row(&old.rows[o], &new.rows[n], moved, options)),
            Pairing::Removed(o) => rows.push(one_sided_row(&old.rows[o], true)),
            Pairing::Added(n) => rows.push(one_sided_row(&new.rows[n], false)),
        }
    }
    rows.sort_by_key(|row| {
        (
            row.new_index.unwrap_or(usize::MAX),
            row.old_index.unwrap_or(0),
        )
    });

    let changed = rows.iter().any(|row| row.kind != ChangeKind::Unchanged);
    TableChange {
        kind: if changed {
            ChangeKind::Changed
        } else if moved {
            ChangeKind::Moved
        } else {
            ChangeKind::Unchanged
        },
        old_index: Some(old.index),
        new_index: Some(new.index),
        moved,
        rows,
    }
}

/// 行の錨。行全体の本文で見る。
///
/// jade の新旧対照表は 1 列目だけで行を見分けているが、1 列目が同じで 2 列目
/// だけ違う行が並ぶ表 (「第1条 | 内容」のような対照表そのもの) では取り違える。
/// 行全体を使い、それで決まらないぶんは類似度に任せる。
fn row_key(row: &RowIr) -> String {
    format!("r:{}", normalize(&row.text()))
}

fn diff_row(old: &RowIr, new: &RowIr, moved: bool, options: &DiffOptions) -> RowChange {
    // セルは位置で突き合わせる。列の並びは表の形そのものなので、内容で
    // 入れ替えると見た目と対応しなくなる。
    let width = old.cells.len().max(new.cells.len());
    let mut cells: Vec<CellChange> = Vec::new();
    for index in 0..width {
        let old_cell = old.cells.get(index);
        let new_cell = new.cells.get(index);
        cells.push(match (old_cell, new_cell) {
            (Some(a), Some(b)) => diff_cell(a, b, options),
            (Some(a), None) => one_sided_cell(a, true),
            (None, Some(b)) => one_sided_cell(b, false),
            (None, None) => unreachable!("width はどちらかの長さ"),
        });
    }

    let changed = cells.iter().any(|cell| cell.kind != ChangeKind::Unchanged);
    RowChange {
        kind: if changed {
            ChangeKind::Changed
        } else if moved {
            ChangeKind::Moved
        } else {
            ChangeKind::Unchanged
        },
        old_index: Some(old.index),
        new_index: Some(new.index),
        moved,
        cells,
    }
}

fn diff_cell(old: &CellIr, new: &CellIr, options: &DiffOptions) -> CellChange {
    let blocks = diff_blocks(&old.blocks, &new.blocks, options);
    let changed = blocks
        .iter()
        .any(|block| block.kind() != ChangeKind::Unchanged);
    CellChange {
        kind: if changed {
            ChangeKind::Changed
        } else {
            ChangeKind::Unchanged
        },
        old_index: Some(old.index),
        new_index: Some(new.index),
        blocks,
    }
}

fn one_sided_block(block: &BlockIr, removed: bool) -> BlockChange {
    let kind = if removed {
        ChangeKind::Removed
    } else {
        ChangeKind::Added
    };
    match block {
        BlockIr::Paragraph(paragraph) => BlockChange::Paragraph(ParagraphChange {
            kind,
            old_index: removed.then_some(paragraph.index),
            new_index: (!removed).then_some(paragraph.index),
            old_text: removed.then(|| paragraph.text.clone()),
            new_text: (!removed).then(|| paragraph.text.clone()),
            old_body_text: removed.then(|| paragraph.body_text.clone()),
            new_body_text: (!removed).then(|| paragraph.body_text.clone()),
            matched_by_para_id: false,
            moved: false,
            has_tracked_changes: paragraph.has_tracked_changes(),
            segments: Vec::new(),
        }),
        BlockIr::Table(table) => BlockChange::Table(TableChange {
            kind,
            old_index: removed.then_some(table.index),
            new_index: (!removed).then_some(table.index),
            moved: false,
            rows: table
                .rows
                .iter()
                .map(|row| one_sided_row(row, removed))
                .collect(),
        }),
    }
}

fn one_sided_row(row: &RowIr, removed: bool) -> RowChange {
    RowChange {
        kind: if removed {
            ChangeKind::Removed
        } else {
            ChangeKind::Added
        },
        old_index: removed.then_some(row.index),
        new_index: (!removed).then_some(row.index),
        moved: false,
        cells: row
            .cells
            .iter()
            .map(|cell| one_sided_cell(cell, removed))
            .collect(),
    }
}

fn one_sided_cell(cell: &CellIr, removed: bool) -> CellChange {
    CellChange {
        kind: if removed {
            ChangeKind::Removed
        } else {
            ChangeKind::Added
        },
        old_index: removed.then_some(cell.index),
        new_index: (!removed).then_some(cell.index),
        blocks: cell
            .blocks
            .iter()
            .map(|block| one_sided_block(block, removed))
            .collect(),
    }
}

fn new_order(block: &BlockChange) -> usize {
    match block {
        BlockChange::Paragraph(change) => change.new_index.unwrap_or(usize::MAX),
        BlockChange::Table(change) => change.new_index.unwrap_or(usize::MAX),
    }
}

fn old_order(block: &BlockChange) -> usize {
    match block {
        BlockChange::Paragraph(change) => change.old_index.unwrap_or(0),
        BlockChange::Table(change) => change.old_index.unwrap_or(0),
    }
}

/// 消えたものを、旧の並びで直前だった位置の後ろへ戻す。
fn reposition_removed(blocks: &mut Vec<BlockChange>) {
    let mut removed: Vec<BlockChange> = Vec::new();
    let mut kept: Vec<BlockChange> = Vec::new();
    for block in blocks.drain(..) {
        if new_order(&block) == usize::MAX {
            removed.push(block);
        } else {
            kept.push(block);
        }
    }
    removed.sort_by_key(old_order);

    for block in removed {
        let old_index = old_order(&block);
        let position = kept
            .iter()
            .position(|candidate| old_order(candidate) > old_index)
            .unwrap_or(kept.len());
        kept.insert(position, block);
    }
    *blocks = kept;
}

/// 文字単位の内訳。日本語には単語境界が無いので文字で見る。
pub fn segment_text(old: &str, new: &str) -> Vec<Segment> {
    let old_chars: Vec<char> = old.chars().collect();
    let new_chars: Vec<char> = new.chars().collect();
    let steps = lcs_diff(&old_chars, &new_chars);

    let mut out: Vec<Segment> = Vec::new();
    for step in steps {
        let (kind, ch) = match step {
            Step::Equal(o, _) => (SegmentKind::Equal, old_chars[o]),
            Step::Removed(o) => (SegmentKind::Delete, old_chars[o]),
            Step::Added(n) => (SegmentKind::Insert, new_chars[n]),
        };
        match out.last_mut() {
            Some(last) if last.kind == kind => last.text.push(ch),
            _ => out.push(Segment {
                kind,
                text: ch.to_string(),
            }),
        }
    }
    out
}

fn accumulate(summary: &mut DiffSummary, block: &BlockChange) {
    match block {
        BlockChange::Paragraph(change) => match change.kind {
            ChangeKind::Added => summary.paragraphs_added += 1,
            ChangeKind::Removed => summary.paragraphs_removed += 1,
            ChangeKind::Changed => summary.paragraphs_changed += 1,
            ChangeKind::Moved => summary.paragraphs_moved += 1,
            ChangeKind::Unchanged => {}
        },
        BlockChange::Table(change) => {
            match change.kind {
                ChangeKind::Added => {
                    // まるごと足された表の中身は数えない。「表 1 つ追加」で
                    // 済む話を行の数まで並べると、どれだけ直したのかが
                    // 読み取れなくなる。中身は明細に入っている。
                    summary.tables_added += 1;
                    return;
                }
                ChangeKind::Removed => {
                    summary.tables_removed += 1;
                    return;
                }
                _ => {}
            }
            for row in &change.rows {
                match row.kind {
                    ChangeKind::Added => summary.table_rows_added += 1,
                    ChangeKind::Removed => summary.table_rows_removed += 1,
                    // 入れ子の表が変わっただけの行は数えない。内側の行を
                    // すでに数えているので、外側まで足すと 1 箇所の直しが
                    // 2 行の変更に見える。
                    ChangeKind::Changed if changed_outside_nested_tables(row) => {
                        summary.table_rows_changed += 1
                    }
                    _ => {}
                }
                if row.kind == ChangeKind::Added || row.kind == ChangeKind::Removed {
                    continue;
                }
                for cell in &row.cells {
                    for inner in &cell.blocks {
                        accumulate(summary, inner);
                    }
                }
            }
        }
    }
}

/// その行そのものが変わったか (入れ子の表の中だけの変更を除く)。
fn changed_outside_nested_tables(row: &RowChange) -> bool {
    row.cells.iter().any(|cell| {
        cell.kind != ChangeKind::Unchanged
            && cell.blocks.iter().any(|block| match block {
                BlockChange::Paragraph(change) => change.kind != ChangeKind::Unchanged,
                // 表そのものが増減したなら、それはこの行の変更。
                BlockChange::Table(change) => {
                    matches!(change.kind, ChangeKind::Added | ChangeKind::Removed)
                }
            })
    })
}

fn collect_diagnostics(blocks: &[BlockIr], diagnostics: &mut Diagnostics, old_side: bool) {
    for block in blocks {
        match block {
            BlockIr::Paragraph(paragraph) => {
                if old_side {
                    diagnostics.old_paragraphs += 1;
                    if paragraph.para_id.is_some() {
                        diagnostics.old_paragraphs_with_para_id += 1;
                    }
                    if paragraph.has_tracked_changes() {
                        diagnostics.old_paragraphs_with_tracked_changes += 1;
                    }
                } else {
                    diagnostics.new_paragraphs += 1;
                    if paragraph.para_id.is_some() {
                        diagnostics.new_paragraphs_with_para_id += 1;
                    }
                    if paragraph.has_tracked_changes() {
                        diagnostics.new_paragraphs_with_tracked_changes += 1;
                    }
                }
            }
            BlockIr::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        collect_diagnostics(&cell.blocks, diagnostics, old_side);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paragraph(text: &str, para_id: Option<&str>) -> BlockIr {
        BlockIr::Paragraph(ParagraphIr {
            index: 1,
            para_id: para_id.map(str::to_owned),
            text: text.to_owned(),
            body_text: text.to_owned(),
            original_text: text.to_owned(),
            in_textbox: false,
            in_shape: false,
        })
    }

    #[test]
    fn paragraph_id_anchor_borrows_the_existing_id() {
        let block = paragraph("body", Some("paragraph-id"));
        let BlockIr::Paragraph(paragraph) = &block else {
            unreachable!();
        };

        let AnchorKey::ParagraphId(key) = anchor_key(&block) else {
            panic!("paraId should be used as the anchor");
        };

        assert!(std::ptr::eq(key, paragraph.para_id.as_deref().unwrap()));
    }

    #[test]
    fn paragraph_id_and_text_anchors_are_distinct_namespaces() {
        let by_id = paragraph("other", Some("same"));
        let by_text = paragraph("same", None);

        assert_ne!(anchor_key(&by_id), anchor_key(&by_text));
    }
}
