//! IR 同士を突き合わせて差分にする。
//!
//! 粒度は スライド → 図形 → 段落 (と表のセル) の 3 段。どの段でも
//! 「対応付けてから中身を比べる」ので、スライドを 1 枚差し込んでも以降が
//! 全部差分になることはない。

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::ir::{Frame, ParagraphIr, PresentationIr, ShapeIr, ShapeKind, SlideIr, TableIr};
use diff_align::{pair_by_score, Pairing, Step};
use diff_support::{trimmed_lcs_diff, TextProfile};
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

/// 出力 JSON の契約バージョン。
pub const DIFF_SCHEMA_VERSION: u32 = 1;

/// 対応付けの閾値。既定値は手元のサンプルで調整したもの。
#[derive(Debug, Clone)]
pub struct DiffOptions {
    /// スライド同士を同じスライドとみなす最小スコア。
    pub slide_match_score: f64,
    /// 図形同士を同じ図形とみなす最小スコア。
    pub shape_match_score: f64,
    /// 複数候補がある変更区間で、削除 + 追加を「変更」に畳む最小スコア。
    /// 1段落対1段落だけの区間は曖昧さがないため、スコアに関係なく変更とする。
    pub text_change_score: f64,
    /// 位置の変化をこの EMU 以下なら無視する (既定 = 約 0.01 インチ)。
    pub position_tolerance: i64,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            slide_match_score: 0.34,
            shape_match_score: 0.30,
            text_change_score: 0.34,
            position_tolerance: 9_144,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Removed,
    Changed,
    Unchanged,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParagraphChange {
    pub kind: ChangeKind,
    /// 図形内での段落番号 (1 始まり)。
    pub old_index: Option<usize>,
    pub new_index: Option<usize>,
    pub old_text: Option<String>,
    pub new_text: Option<String>,
    /// `changed` のときだけ入る文字単位の内訳。
    pub segments: Vec<Segment>,
    /// 本文以外の段落表現（階層・太字・サイズ・揃え）が変わったか。
    pub format_changed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TableCellChange {
    pub kind: ChangeKind,
    /// 1 始まりの列番号。新しい側に列があればその番号、無ければ古い側の番号。
    pub column: usize,
    /// 古い側での 1 始まりの列番号。列が追加されたセルでは None。
    pub old_column: Option<usize>,
    /// 新しい側での 1 始まりの列番号。列が削除されたセルでは None。
    pub new_column: Option<usize>,
    pub old_text: Option<String>,
    pub new_text: Option<String>,
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TableRowChange {
    pub kind: ChangeKind,
    pub old_index: Option<usize>,
    pub new_index: Option<usize>,
    pub cells: Vec<TableCellChange>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TableDiff {
    pub rows: Vec<TableRowChange>,
    /// セル本文とは独立した列幅・行高の変更。
    pub layout_changed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShapeDiff {
    pub old_id: Option<String>,
    pub new_id: Option<String>,
    pub name: String,
    pub kind: ShapeKind,
    pub status: Status,
    pub placeholder: Option<String>,
    pub old_frame: Option<Frame>,
    pub new_frame: Option<Frame>,
    /// 位置・サイズが変わったか (許容量を超えたときだけ true)。
    pub frame_changed: bool,
    pub paragraphs: Vec<ParagraphChange>,
    pub table: Option<TableDiff>,
    /// 画像そのものが差し替わったか。
    pub image_changed: bool,
    /// 塗り・線・形状・回転など、図形自体の外観が変わったか。
    pub appearance_changed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SlideDiff {
    /// 1 始まりの表示順。
    pub old_index: Option<usize>,
    pub new_index: Option<usize>,
    pub status: Status,
    pub title: Option<String>,
    pub match_confidence: Option<f64>,
    pub shapes: Vec<ShapeDiff>,
    pub old_notes: Option<String>,
    pub new_notes: Option<String>,
    pub notes_changed: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DiffSummary {
    pub slides_added: usize,
    pub slides_removed: usize,
    pub slides_moved: usize,
    pub slides_modified: usize,
    pub shapes_added: usize,
    pub shapes_removed: usize,
    pub shapes_moved: usize,
    pub paragraphs_added: usize,
    pub paragraphs_removed: usize,
    pub paragraphs_changed: usize,
    pub table_cells_changed: usize,
    pub table_layouts_changed: usize,
    pub pictures_changed: usize,
    pub shape_appearances_changed: usize,
    pub notes_changed: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SlideSize {
    pub width: i64,
    pub height: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PresentationDiff {
    pub schema_version: u32,
    pub coordinate_system: &'static str,
    pub old_file: String,
    pub new_file: String,
    pub old_size: SlideSize,
    pub new_size: SlideSize,
    pub summary: DiffSummary,
    pub slides: Vec<SlideDiff>,
}

impl PresentationDiff {
    pub fn has_changes(&self) -> bool {
        self.old_size != self.new_size
            || self
                .slides
                .iter()
                .any(|slide| slide.status != Status::Unchanged)
    }
}

/// 2 つのプレゼンテーションを比較する。
pub fn diff_presentations(
    old_file: &str,
    new_file: &str,
    old: &PresentationIr,
    new: &PresentationIr,
    options: &DiffOptions,
) -> PresentationDiff {
    // 総当たりはスライド数の 2 乗回スコアを引くので、本文の正規化や bigram は
    // 先に 1 回だけ作っておく。
    let old_profiles: Vec<SlideProfile> = old.slides.iter().map(SlideProfile::new).collect();
    let new_profiles: Vec<SlideProfile> = new.slides.iter().map(SlideProfile::new).collect();
    let threshold = options.slide_match_score;
    let pairings = pair_by_score(old.slides.len(), new.slides.len(), threshold, |i, j| {
        slide_score_at_least(&old_profiles[i], &new_profiles[j], threshold)
    });

    let mut slides: Vec<SlideDiff> = Vec::new();
    for pairing in &pairings {
        match *pairing {
            Pairing::Matched {
                old: o,
                new: n,
                moved,
            } => {
                let confidence = slide_score(&old_profiles[o], &new_profiles[n]);
                slides.push(diff_slide(
                    &old.slides[o],
                    &new.slides[n],
                    moved,
                    confidence,
                    options,
                ));
            }
            Pairing::Removed(o) => slides.push(one_sided_slide(&old.slides[o], true)),
            Pairing::Added(n) => slides.push(one_sided_slide(&new.slides[n], false)),
        }
    }
    // 新しい方の並び順で見せる。削除されたスライドは直前の位置に挟む。
    slides.sort_by_key(|slide| {
        (
            slide.new_index.unwrap_or(usize::MAX),
            slide.old_index.unwrap_or(0),
        )
    });
    reposition_removed(&mut slides);

    let mut summary = DiffSummary::default();
    for slide in &slides {
        accumulate(&mut summary, slide);
    }

    PresentationDiff {
        schema_version: DIFF_SCHEMA_VERSION,
        coordinate_system: "one_based",
        old_file: old_file.to_string(),
        new_file: new_file.to_string(),
        old_size: SlideSize {
            width: old.width,
            height: old.height,
        },
        new_size: SlideSize {
            width: new.width,
            height: new.height,
        },
        summary,
        slides,
    }
}

/// 削除スライドを「旧の並びで直前だったスライド」の後ろへ移す。
///
/// そのままだと new_index が無いぶん全部末尾に寄ってしまい、どこが消えたのか
/// 読み取りにくい。
fn reposition_removed(slides: &mut Vec<SlideDiff>) {
    let mut removed: Vec<SlideDiff> = Vec::new();
    let mut kept: Vec<SlideDiff> = Vec::new();
    for slide in slides.drain(..) {
        if slide.new_index.is_none() {
            removed.push(slide);
        } else {
            kept.push(slide);
        }
    }
    removed.sort_by_key(|slide| slide.old_index.unwrap_or(0));

    // 削除側は旧順。挿入先は kept 内で前へ戻らないため、各候補を一度だけ調べる。
    // Vec::insert を繰り返すと、大量の削除で二乗回の要素移動が起きる。
    let mut buckets: Vec<Vec<SlideDiff>> = (0..=kept.len()).map(|_| Vec::new()).collect();
    let mut position = 0usize;
    for slide in removed {
        let old_index = slide.old_index.unwrap_or(0);
        while position < kept.len()
            && kept[position]
                .old_index
                .is_none_or(|index| index <= old_index)
        {
            position += 1;
        }
        buckets[position].push(slide);
    }

    let mut ordered = Vec::with_capacity(kept.len() + buckets.iter().map(Vec::len).sum::<usize>());
    for (position, slide) in kept.into_iter().enumerate() {
        ordered.append(&mut buckets[position]);
        ordered.push(slide);
    }
    ordered.append(buckets.last_mut().expect("末尾 bucket"));
    *slides = ordered;
}

/// スライド対応付け用の下ごしらえ。
struct SlideProfile {
    body: TextProfile,
    title: Option<TextProfile>,
    layout: Vec<String>,
}

impl SlideProfile {
    fn new(slide: &SlideIr) -> Self {
        Self {
            body: TextProfile::new(&comparison_text(&slide.text())),
            title: slide
                .title()
                .map(|title| TextProfile::new(&comparison_text(&title))),
            layout: slide_layout_signature(slide),
        }
    }
}

const SLIDE_BODY_WEIGHT: f64 = 0.55;
const SLIDE_TITLE_WEIGHT: f64 = 0.35;
const SLIDE_LAYOUT_WEIGHT: f64 = 0.10;

fn slide_score(old: &SlideProfile, new: &SlideProfile) -> f64 {
    let body = old.body.similarity(&new.body);
    let title = slide_title_score(old, new, body);
    slide_score_from_parts(old, new, body, title)
}

fn slide_title_score(old: &SlideProfile, new: &SlideProfile, body: f64) -> f64 {
    match (&old.title, &new.title) {
        (Some(a), Some(b)) => a.similarity(b),
        (None, None) => body,
        _ => 0.0,
    }
}

fn slide_score_from_parts(old: &SlideProfile, new: &SlideProfile, body: f64, title: f64) -> f64 {
    let layout = f64::from(old.layout == new.layout);
    SLIDE_BODY_WEIGHT * body + SLIDE_TITLE_WEIGHT * title + SLIDE_LAYOUT_WEIGHT * layout
}

/// `threshold` に届かないと長さだけで分かる組は本文を突き合わせずに 0 を返す。
/// 届きうる組は [`slide_score`] と同じ値を返す。
fn slide_score_at_least(old: &SlideProfile, new: &SlideProfile, threshold: f64) -> f64 {
    let body_bound = old.body.similarity_upper_bound(&new.body);
    let title = slide_title_score(old, new, body_bound);
    if slide_score_from_parts(old, new, body_bound, title) < threshold {
        return 0.0;
    }
    let body = old.body.similarity(&new.body);
    let title = if old.title.is_none() && new.title.is_none() {
        body
    } else {
        title
    };
    slide_score_from_parts(old, new, body, title)
}

fn slide_layout_signature(slide: &SlideIr) -> Vec<String> {
    let mut shapes: Vec<String> = slide
        .shapes
        .iter()
        .map(|shape| {
            let placeholder = shape
                .placeholder
                .as_ref()
                .map(|placeholder| format!("{}:{:?}", placeholder.kind, placeholder.index))
                .unwrap_or_default();
            let frame = shape
                .frame
                .map(|frame| format!("{}:{}:{}:{}", frame.x, frame.y, frame.width, frame.height))
                .unwrap_or_default();
            let table = shape
                .table
                .as_ref()
                .map(|table| format!("{:?}:{:?}", table.column_widths, table.row_heights))
                .unwrap_or_default();
            format!("{:?}|{placeholder}|{frame}|{}|{table}", shape.kind, shape.z)
        })
        .collect();
    shapes.sort();
    shapes
}

fn one_sided_slide(slide: &SlideIr, removed: bool) -> SlideDiff {
    let shapes = slide
        .shapes
        .iter()
        .map(|shape| one_sided_shape(shape, removed))
        .collect();
    SlideDiff {
        old_index: removed.then_some(slide.index),
        new_index: (!removed).then_some(slide.index),
        status: if removed {
            Status::Removed
        } else {
            Status::Added
        },
        title: slide.title(),
        match_confidence: None,
        shapes,
        old_notes: removed.then(|| slide.notes.clone()).flatten(),
        new_notes: (!removed).then(|| slide.notes.clone()).flatten(),
        notes_changed: slide.notes.is_some(),
    }
}

fn one_sided_shape(shape: &ShapeIr, removed: bool) -> ShapeDiff {
    let paragraphs = shape
        .paragraphs
        .iter()
        .enumerate()
        .map(|(index, paragraph)| ParagraphChange {
            kind: if removed {
                ChangeKind::Removed
            } else {
                ChangeKind::Added
            },
            old_index: removed.then_some(index + 1),
            new_index: (!removed).then_some(index + 1),
            old_text: removed.then(|| paragraph.text.clone()),
            new_text: (!removed).then(|| paragraph.text.clone()),
            segments: Vec::new(),
            format_changed: false,
        })
        .collect();

    ShapeDiff {
        old_id: removed.then(|| shape.id.clone()),
        new_id: (!removed).then(|| shape.id.clone()),
        name: shape.name.clone(),
        kind: shape.kind,
        status: if removed {
            Status::Removed
        } else {
            Status::Added
        },
        placeholder: shape.placeholder.as_ref().map(|ph| ph.kind.clone()),
        old_frame: removed.then_some(shape.frame).flatten(),
        new_frame: (!removed).then_some(shape.frame).flatten(),
        frame_changed: false,
        paragraphs,
        table: shape
            .table
            .as_ref()
            .map(|table| one_sided_table(table, removed)),
        image_changed: shape.image.is_some(),
        appearance_changed: false,
    }
}

fn one_sided_table(table: &TableIr, removed: bool) -> TableDiff {
    TableDiff {
        rows: table
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| TableRowChange {
                kind: if removed {
                    ChangeKind::Removed
                } else {
                    ChangeKind::Added
                },
                old_index: removed.then_some(index + 1),
                new_index: (!removed).then_some(index + 1),
                cells: one_sided_cells(row, removed),
            })
            .collect(),
        layout_changed: false,
    }
}

fn diff_slide(
    old: &SlideIr,
    new: &SlideIr,
    moved: bool,
    confidence: f64,
    options: &DiffOptions,
) -> SlideDiff {
    let old_profiles: Vec<Option<TextProfile>> = old.shapes.iter().map(shape_profile).collect();
    let new_profiles: Vec<Option<TextProfile>> = new.shapes.iter().map(shape_profile).collect();
    let pairings = pair_by_score(
        old.shapes.len(),
        new.shapes.len(),
        options.shape_match_score,
        |i, j| {
            shape_score(
                &old.shapes[i],
                &new.shapes[j],
                old_profiles[i].as_ref(),
                new_profiles[j].as_ref(),
            )
        },
    );

    // 新側の添字は対応付けで一意に決まる。ID は重複しうるため、
    // ID から描画順を引き直さずに直接配置する。
    let mut by_new: Vec<Option<ShapeDiff>> = (0..new.shapes.len()).map(|_| None).collect();
    let mut removed = Vec::new();
    for pairing in &pairings {
        match *pairing {
            Pairing::Matched {
                old: o,
                new: n,
                moved: shape_moved,
            } => {
                by_new[n] = Some(diff_shape(
                    &old.shapes[o],
                    &new.shapes[n],
                    shape_moved,
                    options,
                ));
            }
            Pairing::Removed(o) => removed.push(one_sided_shape(&old.shapes[o], true)),
            Pairing::Added(n) => by_new[n] = Some(one_sided_shape(&new.shapes[n], false)),
        }
    }
    let mut shapes: Vec<ShapeDiff> = by_new.into_iter().flatten().collect();
    shapes.extend(removed);

    let notes_changed = !optional_text_equivalent(old.notes.as_deref(), new.notes.as_deref());
    let changed = notes_changed || shapes.iter().any(|shape| shape.status != Status::Unchanged);

    SlideDiff {
        old_index: Some(old.index),
        new_index: Some(new.index),
        status: if changed {
            Status::Modified
        } else if moved {
            Status::Moved
        } else {
            Status::Unchanged
        },
        title: new.title().or_else(|| old.title()),
        match_confidence: Some(confidence),
        shapes,
        old_notes: old.notes.clone(),
        new_notes: new.notes.clone(),
        notes_changed,
    }
}

/// 本文で対応付ける図形の下ごしらえ。画像は中身のハッシュで比べるので作らない。
fn shape_profile(shape: &ShapeIr) -> Option<TextProfile> {
    (shape.kind != ShapeKind::Picture).then(|| TextProfile::new(&comparison_text(&shape.text())))
}

fn shape_score(
    old: &ShapeIr,
    new: &ShapeIr,
    old_profile: Option<&TextProfile>,
    new_profile: Option<&TextProfile>,
) -> f64 {
    if old.kind != new.kind {
        return 0.0;
    }

    // プレースホルダは種別/番号が同じなら強い手掛かり。ただし同じ種別・番号の
    // 枠が複数ある資料もあるため、ここで 1.0 を返さず本文と位置で曖昧さを解く。
    let same_placeholder = matches!(
        (&old.placeholder, &new.placeholder),
        (Some(old), Some(new)) if old == new
    );

    let mut score = match old.kind {
        ShapeKind::Picture => match (&old.image, &new.image) {
            (Some(a), Some(b)) if a.content_hash == b.content_hash => 1.0,
            _ => 0.4,
        },
        _ => match (old_profile, new_profile) {
            (Some(a), Some(b)) => a.similarity(b),
            _ => 0.0,
        },
    };

    if same_placeholder {
        // 本文の総取り替えでも同じ枠として対応できる最低点。
        score = score.max(0.75);
    }

    if old.id == new.id && !old.id.is_empty() {
        score = score.max(0.7);
    }
    if old.name == new.name && !old.name.is_empty() {
        score = score.max(0.55);
    }
    // 同じ位置に留まっているならわずかに加点する (同種の図形が並ぶ表紙など)。
    if let (Some(a), Some(b)) = (old.frame, new.frame) {
        if a == b {
            score = (score + 0.1).min(1.0);
        }
    }
    score
}

fn diff_shape(old: &ShapeIr, new: &ShapeIr, moved: bool, options: &DiffOptions) -> ShapeDiff {
    let paragraphs = diff_paragraphs(&old.paragraphs, &new.paragraphs, options);
    let table = match (&old.table, &new.table) {
        (Some(a), Some(b)) => Some(diff_table(a, b, options)),
        (Some(a), None) => Some(one_sided_table(a, true)),
        (None, Some(b)) => Some(one_sided_table(b, false)),
        (None, None) => None,
    };
    let image_changed = match (&old.image, &new.image) {
        (Some(a), Some(b)) => a.content_hash != b.content_hash,
        (None, None) => false,
        _ => true,
    };
    let appearance_changed = old.appearance_signature != new.appearance_signature;

    let frame_changed = match (old.frame, new.frame) {
        (Some(a), Some(b)) => {
            (a.x - b.x).abs() > options.position_tolerance
                || (a.y - b.y).abs() > options.position_tolerance
                || (a.width - b.width).abs() > options.position_tolerance
                || (a.height - b.height).abs() > options.position_tolerance
        }
        (None, None) => false,
        _ => true,
    };

    let text_changed = paragraphs
        .iter()
        .any(|change| change.kind != ChangeKind::Unchanged);
    let table_changed = table.as_ref().is_some_and(|table| {
        table.layout_changed
            || table.rows.iter().any(|row| {
                row.kind != ChangeKind::Unchanged
                    || row
                        .cells
                        .iter()
                        .any(|cell| cell.kind != ChangeKind::Unchanged)
            })
    });

    let status =
        if text_changed || table_changed || image_changed || appearance_changed || frame_changed {
            Status::Modified
        } else if moved {
            Status::Moved
        } else {
            Status::Unchanged
        };

    ShapeDiff {
        old_id: Some(old.id.clone()),
        new_id: Some(new.id.clone()),
        name: if new.name.is_empty() {
            old.name.clone()
        } else {
            new.name.clone()
        },
        kind: new.kind,
        status,
        placeholder: new
            .placeholder
            .as_ref()
            .or(old.placeholder.as_ref())
            .map(|ph| ph.kind.clone()),
        old_frame: old.frame,
        new_frame: new.frame,
        frame_changed,
        paragraphs,
        table,
        image_changed,
        appearance_changed,
    }
}

/// 段落列の差分。削除 + 追加が隣り合って似ていれば「変更」に畳む。
fn diff_paragraphs(
    old: &[ParagraphIr],
    new: &[ParagraphIr],
    options: &DiffOptions,
) -> Vec<ParagraphChange> {
    let old_text: Vec<&str> = old.iter().map(|p| p.text.as_str()).collect();
    let new_text: Vec<&str> = new.iter().map(|p| p.text.as_str()).collect();
    let old_keys: Vec<String> = old_text.iter().map(|text| comparison_text(text)).collect();
    let new_keys: Vec<String> = new_text.iter().map(|text| comparison_text(text)).collect();
    let steps = trimmed_lcs_diff(&old_keys, &new_keys);

    let mut out: Vec<ParagraphChange> = Vec::new();
    let mut index = 0usize;
    while index < steps.len() {
        match steps[index] {
            Step::Equal(o, n) => {
                let format_changed = paragraph_format_changed(&old[o], &new[n]);
                out.push(ParagraphChange {
                    kind: if format_changed {
                        ChangeKind::Changed
                    } else {
                        ChangeKind::Unchanged
                    },
                    old_index: Some(o + 1),
                    new_index: Some(n + 1),
                    old_text: Some(old_text[o].to_string()),
                    new_text: Some(new_text[n].to_string()),
                    segments: Vec::new(),
                    format_changed,
                });
                index += 1;
            }
            Step::Removed(_) | Step::Added(_) => {
                let end = hunk_end(&steps, index);
                let hunk = &steps[index..end];
                let (removed, added) = hunk_sides(hunk);
                let matched = if removed.len() == 1 && added.len() == 1 {
                    // 対応候補が1つずつなら、本文を総取り替えして類似度が0になっても
                    // 同じ位置の段落変更とみなせる。削除+追加へ崩すと変更箇所が読みにくい。
                    vec![(removed[0], added[0])]
                } else {
                    pair_hunk_by_similarity(
                        &removed,
                        &added,
                        &old_keys,
                        &new_keys,
                        options.text_change_score,
                    )
                };
                out.extend(emit_hunk(hunk, &matched, |change| match change {
                    HunkChange::Changed(o, n) => ParagraphChange {
                        kind: ChangeKind::Changed,
                        old_index: Some(o + 1),
                        new_index: Some(n + 1),
                        old_text: Some(old_text[o].to_string()),
                        new_text: Some(new_text[n].to_string()),
                        segments: segment_text(old_text[o], new_text[n]),
                        format_changed: paragraph_format_changed(&old[o], &new[n]),
                    },
                    HunkChange::Removed(o) => ParagraphChange {
                        kind: ChangeKind::Removed,
                        old_index: Some(o + 1),
                        new_index: None,
                        old_text: Some(old_text[o].to_string()),
                        new_text: None,
                        segments: Vec::new(),
                        format_changed: false,
                    },
                    HunkChange::Added(n) => ParagraphChange {
                        kind: ChangeKind::Added,
                        old_index: None,
                        new_index: Some(n + 1),
                        old_text: None,
                        new_text: Some(new_text[n].to_string()),
                        segments: Vec::new(),
                        format_changed: false,
                    },
                }));
                index = end;
            }
        }
    }
    out
}

fn paragraph_format_changed(old: &ParagraphIr, new: &ParagraphIr) -> bool {
    old.level != new.level
        || old.bold != new.bold
        || old.size != new.size
        || old.align != new.align
        || if old.text == new.text {
            old.format_spans != new.format_spans
        } else {
            old.format_spans
                .iter()
                .map(|span| &span.style)
                .ne(new.format_spans.iter().map(|span| &span.style))
        }
}

/// `start` から始まる変更区間 (次の完全一致の手前まで) の終端。
fn hunk_end(steps: &[Step], start: usize) -> usize {
    steps[start..]
        .iter()
        .position(|step| matches!(step, Step::Equal(_, _)))
        .map_or(steps.len(), |offset| start + offset)
}

/// 変更区間に含まれる旧側・新側の添字。
fn hunk_sides(hunk: &[Step]) -> (Vec<usize>, Vec<usize>) {
    let mut removed = Vec::new();
    let mut added = Vec::new();
    for step in hunk {
        match *step {
            Step::Removed(index) => removed.push(index),
            Step::Added(index) => added.push(index),
            Step::Equal(_, _) => unreachable!("変更区間に完全一致は含まれない"),
        }
    }
    (removed, added)
}

/// 変更区間を 1 件ずつ並べ直したもの。対応が付いた組は旧側の位置に出す。
enum HunkChange {
    Changed(usize, usize),
    Removed(usize),
    Added(usize),
}

fn emit_hunk<T>(
    hunk: &[Step],
    matched: &[(usize, usize)],
    mut emit: impl FnMut(HunkChange) -> T,
) -> Vec<T> {
    let by_old: HashMap<usize, usize> = matched.iter().copied().collect();
    let paired_new: HashSet<usize> = matched.iter().map(|(_, n)| *n).collect();
    let mut out = Vec::with_capacity(hunk.len());
    for step in hunk {
        match *step {
            Step::Removed(o) => out.push(emit(match by_old.get(&o) {
                Some(&n) => HunkChange::Changed(o, n),
                None => HunkChange::Removed(o),
            })),
            Step::Added(n) if !paired_new.contains(&n) => out.push(emit(HunkChange::Added(n))),
            Step::Added(_) => {}
            Step::Equal(_, _) => unreachable!("変更区間に完全一致は含まれない"),
        }
    }
    out
}

/// 変更区間の削除と追加を、比較用の本文の類似度で 1:1 に対応付ける。
fn pair_hunk_by_similarity(
    removed: &[usize],
    added: &[usize],
    old_keys: &[String],
    new_keys: &[String],
    threshold: f64,
) -> Vec<(usize, usize)> {
    pair_hunk_by_profile(
        removed,
        added,
        old_keys,
        new_keys,
        threshold,
        |key| TextProfile::new(key),
        |old, new| old.similarity_at_least(new, threshold),
    )
}

fn pair_hunk_by_profile<T, P>(
    removed: &[usize],
    added: &[usize],
    old_keys: &[T],
    new_keys: &[T],
    threshold: f64,
    profile: impl Fn(&T) -> P,
    score: impl Fn(&P, &P) -> f64,
) -> Vec<(usize, usize)> {
    if removed.is_empty() || added.is_empty() {
        return Vec::new();
    }
    let old: Vec<P> = removed
        .iter()
        .map(|&index| profile(&old_keys[index]))
        .collect();
    let new: Vec<P> = added
        .iter()
        .map(|&index| profile(&new_keys[index]))
        .collect();

    pair_by_score(old.len(), new.len(), threshold, |o, n| {
        score(&old[o], &new[n])
    })
    .into_iter()
    .filter_map(|pairing| match pairing {
        Pairing::Matched { old: o, new: n, .. } => Some((removed[o], added[n])),
        Pairing::Removed(_) | Pairing::Added(_) => None,
    })
    .collect()
}

/// 書記素クラスタ単位の内訳。結合文字や ZWJ 絵文字を途中で切らない。
pub fn segment_text(old: &str, new: &str) -> Vec<Segment> {
    let old_graphemes: Vec<&str> = old.graphemes(true).collect();
    let new_graphemes: Vec<&str> = new.graphemes(true).collect();
    let steps = trimmed_lcs_diff(&old_graphemes, &new_graphemes);

    let mut out: Vec<Segment> = Vec::new();
    for step in steps {
        let (kind, grapheme) = match step {
            Step::Equal(o, _) => (SegmentKind::Equal, old_graphemes[o]),
            Step::Removed(o) => (SegmentKind::Delete, old_graphemes[o]),
            Step::Added(n) => (SegmentKind::Insert, new_graphemes[n]),
        };
        match out.last_mut() {
            Some(last) if last.kind == kind => last.text.push_str(grapheme),
            _ => out.push(Segment {
                kind,
                text: grapheme.to_string(),
            }),
        }
    }
    out
}

/// 表の列の対応。`(旧の列, 新の列)` で、片側だけの列は追加/削除された列。
type ColumnPair = (Option<usize>, Option<usize>);

/// Whole-cell overlap remains useful when rows/columns are inserted into a
/// numeric table: "1\n2" and "1\n9\n2" share cells but no character bigrams.
struct CellSequenceProfile {
    text: TextProfile,
    cells: Vec<(String, usize)>,
    cell_count: usize,
}

impl CellSequenceProfile {
    fn new(cells: &[String]) -> Self {
        let text = TextProfile::new(&cells.join("\n"));
        // Sort borrowed values first so repeated cells allocate only once.
        let mut sorted: Vec<_> = cells.iter().filter(|cell| !cell.is_empty()).collect();
        sorted.sort_unstable();
        let cell_count = sorted.len();
        let mut cells: Vec<(String, usize)> = Vec::new();
        for cell in sorted {
            match cells.last_mut() {
                Some((value, count)) if value == cell => *count += 1,
                _ => cells.push((cell.clone(), 1)),
            }
        }
        Self {
            text,
            cells,
            cell_count,
        }
    }

    fn similarity_at_least(&self, other: &Self, threshold: f64) -> f64 {
        let (mut i, mut j, mut common) = (0, 0, 0);
        while i < self.cells.len() && j < other.cells.len() {
            match self.cells[i].0.cmp(&other.cells[j].0) {
                std::cmp::Ordering::Equal => {
                    common += self.cells[i].1.min(other.cells[j].1);
                    i += 1;
                    j += 1;
                }
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
            }
        }
        let total = self.cell_count + other.cell_count;
        let cell_score = if total == 0 {
            0.0
        } else {
            2.0 * common as f64 / total as f64
        };
        // Scores below the whole-cell score cannot change the final result.
        self.text
            .similarity_at_least(&other.text, threshold.max(cell_score))
            .max(cell_score)
    }
}

fn table_width(table: &TableIr) -> usize {
    table.rows.iter().map(Vec::len).max().unwrap_or(0)
}

/// 列を対応付ける。
///
/// 列数が同じなら位置どおりに対応させる (列の中身の書き換えは「変更」)。
/// 列数が違うときは列ごとの中身で対応付けるので、列を 1 本差し込んでも
/// 右側の列が全部ずれて「変更」になることはない。
fn align_columns(old: &TableIr, new: &TableIr, threshold: f64) -> Vec<ColumnPair> {
    let (old_width, new_width) = (table_width(old), table_width(new));
    if old_width == new_width {
        return (0..old_width)
            .map(|column| (Some(column), Some(column)))
            .collect();
    }

    // Preserve cell boundaries even when cell text contains newlines.
    let column_keys = |table: &TableIr, width: usize| -> Vec<Vec<String>> {
        (0..width)
            .map(|column| {
                table
                    .rows
                    .iter()
                    .map(|row| comparison_text(row.get(column).map_or("", String::as_str)))
                    .collect()
            })
            .collect()
    };
    let old_keys = column_keys(old, old_width);
    let new_keys = column_keys(new, new_width);
    let steps = trimmed_lcs_diff(&old_keys, &new_keys);

    let mut out = Vec::with_capacity(old_width.max(new_width));
    let mut index = 0usize;
    while index < steps.len() {
        match steps[index] {
            Step::Equal(o, n) => {
                out.push((Some(o), Some(n)));
                index += 1;
            }
            Step::Removed(_) | Step::Added(_) => {
                let end = hunk_end(&steps, index);
                let hunk = &steps[index..end];
                let (removed, added) = hunk_sides(hunk);
                let matched = pair_hunk_by_profile(
                    &removed,
                    &added,
                    &old_keys,
                    &new_keys,
                    threshold,
                    |cells| CellSequenceProfile::new(cells),
                    |old, new| old.similarity_at_least(new, threshold),
                );
                out.extend(emit_hunk(hunk, &matched, |change| match change {
                    HunkChange::Changed(o, n) => (Some(o), Some(n)),
                    HunkChange::Removed(o) => (Some(o), None),
                    HunkChange::Added(n) => (None, Some(n)),
                }));
                index = end;
            }
        }
    }
    out
}

/// 行の比較用キー。両側にある列だけを使うので、列の追加/削除だけでは
/// 行が変わったことにならない。
fn row_key(row: &[String], columns: &[ColumnPair], old_side: bool) -> Vec<String> {
    columns
        .iter()
        .filter_map(|&(old, new)| {
            let column = if old_side { old? } else { new? };
            // 片側にしか無い列は比べない。
            old.and(new)?;
            Some(comparison_text(row.get(column).map_or("", String::as_str)))
        })
        .collect()
}

fn diff_table(old: &TableIr, new: &TableIr, options: &DiffOptions) -> TableDiff {
    let columns = align_columns(old, new, options.text_change_score);
    let old_keys: Vec<Vec<String>> = old
        .rows
        .iter()
        .map(|row| row_key(row, &columns, true))
        .collect();
    let new_keys: Vec<Vec<String>> = new
        .rows
        .iter()
        .map(|row| row_key(row, &columns, false))
        .collect();
    let steps = trimmed_lcs_diff(&old_keys, &new_keys);

    let mut rows: Vec<TableRowChange> = Vec::new();
    let mut index = 0usize;
    while index < steps.len() {
        match steps[index] {
            Step::Equal(o, n) => {
                let cells = diff_row(&old.rows[o], &new.rows[n], &columns);
                // 共通の列が一致していても、追加/削除された列のセルがあれば変更。
                let kind = if cells.iter().all(|cell| cell.kind == ChangeKind::Unchanged) {
                    ChangeKind::Unchanged
                } else {
                    ChangeKind::Changed
                };
                rows.push(TableRowChange {
                    kind,
                    old_index: Some(o + 1),
                    new_index: Some(n + 1),
                    cells,
                });
                index += 1;
            }
            Step::Removed(_) | Step::Added(_) => {
                let end = hunk_end(&steps, index);
                let hunk = &steps[index..end];
                let (removed, added) = hunk_sides(hunk);
                // A single row replacement carries positional evidence even
                // when numeric values have no matching character bigrams.
                let matched = if removed.len() == 1 && added.len() == 1 {
                    vec![(removed[0], added[0])]
                } else {
                    pair_hunk_by_profile(
                        &removed,
                        &added,
                        &old_keys,
                        &new_keys,
                        options.text_change_score,
                        |cells| CellSequenceProfile::new(cells),
                        |old, new| old.similarity_at_least(new, options.text_change_score),
                    )
                };
                rows.extend(emit_hunk(hunk, &matched, |change| match change {
                    HunkChange::Changed(o, n) => TableRowChange {
                        kind: ChangeKind::Changed,
                        old_index: Some(o + 1),
                        new_index: Some(n + 1),
                        cells: diff_row(&old.rows[o], &new.rows[n], &columns),
                    },
                    HunkChange::Removed(o) => TableRowChange {
                        kind: ChangeKind::Removed,
                        old_index: Some(o + 1),
                        new_index: None,
                        cells: one_sided_cells(&old.rows[o], true),
                    },
                    HunkChange::Added(n) => TableRowChange {
                        kind: ChangeKind::Added,
                        old_index: None,
                        new_index: Some(n + 1),
                        cells: one_sided_cells(&new.rows[n], false),
                    },
                }));
                index = end;
            }
        }
    }
    let tolerance = options.position_tolerance.max(0) as u64;
    let layout_changed = dimensions_changed(&old.column_widths, &new.column_widths, tolerance)
        || dimensions_changed(&old.row_heights, &new.row_heights, tolerance);
    TableDiff {
        rows,
        layout_changed,
    }
}

fn dimensions_changed(old: &[i64], new: &[i64], tolerance: u64) -> bool {
    old.len() != new.len()
        || old
            .iter()
            .zip(new)
            .any(|(before, after)| before.abs_diff(*after) > tolerance)
}

fn one_sided_cells(row: &[String], removed: bool) -> Vec<TableCellChange> {
    row.iter()
        .enumerate()
        .map(|(column, text)| TableCellChange {
            kind: if removed {
                ChangeKind::Removed
            } else {
                ChangeKind::Added
            },
            column: column + 1,
            old_column: removed.then_some(column + 1),
            new_column: (!removed).then_some(column + 1),
            old_text: removed.then(|| text.clone()),
            new_text: (!removed).then(|| text.clone()),
            segments: Vec::new(),
        })
        .collect()
}

fn diff_row(old: &[String], new: &[String], columns: &[ColumnPair]) -> Vec<TableCellChange> {
    columns
        .iter()
        .filter_map(|&(old_column, new_column)| {
            let before = old_column.and_then(|column| old.get(column));
            let after = new_column.and_then(|column| new.get(column));
            let kind = match (before, after) {
                (Some(a), Some(b)) if text_equivalent(a, b) => ChangeKind::Unchanged,
                (Some(_), Some(_)) => ChangeKind::Changed,
                (Some(_), None) => ChangeKind::Removed,
                (None, Some(_)) => ChangeKind::Added,
                // この行には両側ともセルが無い (行ごとに列数が違う表)。
                (None, None) => return None,
            };
            let old_column = before.and(old_column).map(|column| column + 1);
            let new_column = after.and(new_column).map(|column| column + 1);
            Some(TableCellChange {
                kind,
                column: new_column.or(old_column).unwrap_or_default(),
                old_column,
                new_column,
                old_text: before.cloned(),
                new_text: after.cloned(),
                segments: if kind == ChangeKind::Changed {
                    segment_text(
                        before.map(String::as_str).unwrap_or_default(),
                        after.map(String::as_str).unwrap_or_default(),
                    )
                } else {
                    Vec::new()
                },
            })
        })
        .collect()
}

fn comparison_text(text: &str) -> String {
    text.chars()
        .filter(|character| *character != '\u{ad}')
        .nfc()
        .collect()
}

fn text_equivalent(old: &str, new: &str) -> bool {
    old == new || comparison_text(old) == comparison_text(new)
}

fn optional_text_equivalent(old: Option<&str>, new: Option<&str>) -> bool {
    match (old, new) {
        (Some(old), Some(new)) => text_equivalent(old, new),
        (None, None) => true,
        _ => false,
    }
}

fn accumulate(summary: &mut DiffSummary, slide: &SlideDiff) {
    match slide.status {
        Status::Added => summary.slides_added += 1,
        Status::Removed => summary.slides_removed += 1,
        Status::Moved => summary.slides_moved += 1,
        Status::Modified => summary.slides_modified += 1,
        Status::Unchanged => {}
    }
    if slide.notes_changed && slide.status == Status::Modified {
        summary.notes_changed += 1;
    }

    // まるごと足された / 消えたスライドの中身は数えない。「1 枚追加」で
    // 済む話を「1 枚追加・図形 2 個追加・段落 3 個追加」と並べると、何枚
    // 直したのかが読み取れなくなる。中身は各スライドの明細に入っている。
    if matches!(slide.status, Status::Added | Status::Removed) {
        return;
    }

    for shape in &slide.shapes {
        match shape.status {
            Status::Added => summary.shapes_added += 1,
            Status::Removed => summary.shapes_removed += 1,
            Status::Moved => summary.shapes_moved += 1,
            // 位置や大きさだけが変わった図形も「動いた」に数える。中身も
            // 変わっていると status は Modified になるので、ここで拾わないと
            // 枠の変化が要約のどこにも出てこない。
            _ if shape.frame_changed => summary.shapes_moved += 1,
            _ => {}
        }
        if shape.image_changed {
            summary.pictures_changed += 1;
        }
        if shape.appearance_changed {
            summary.shape_appearances_changed += 1;
        }
        for paragraph in &shape.paragraphs {
            match paragraph.kind {
                ChangeKind::Added => summary.paragraphs_added += 1,
                ChangeKind::Removed => summary.paragraphs_removed += 1,
                ChangeKind::Changed => summary.paragraphs_changed += 1,
                ChangeKind::Unchanged => {}
            }
        }
        if let Some(table) = &shape.table {
            if table.layout_changed {
                summary.table_layouts_changed += 1;
            }
            for row in &table.rows {
                for cell in &row.cells {
                    if cell.kind != ChangeKind::Unchanged {
                        summary.table_cells_changed += 1;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use std::collections::BTreeSet;

    use proptest::prelude::*;
    use unicode_segmentation::UnicodeSegmentation;

    use super::*;

    fn paragraph(text: String) -> ParagraphIr {
        ParagraphIr {
            text,
            level: 0,
            bold: false,
            size: None,
            align: None,
            format_spans: Vec::new(),
        }
    }

    fn table(rows: Vec<Vec<String>>) -> TableIr {
        TableIr {
            rows,
            column_widths: Vec::new(),
            row_heights: Vec::new(),
        }
    }

    fn presentation(paragraph: ParagraphIr) -> PresentationIr {
        PresentationIr {
            width: 10_000,
            height: 10_000,
            slides: vec![SlideIr {
                part: "ppt/slides/slide1.xml".to_string(),
                index: 1,
                shapes: vec![ShapeIr {
                    id: "1".to_string(),
                    name: "Text".to_string(),
                    kind: ShapeKind::Text,
                    placeholder: None,
                    frame: None,
                    z: 0,
                    appearance_signature: String::new(),
                    paragraphs: vec![paragraph],
                    table: None,
                    image: None,
                }],
                notes: None,
            }],
            media: Default::default(),
        }
    }

    #[test]
    fn スライド寸法だけの変更も差分として報告する() {
        let old = presentation(paragraph("同じ内容".to_string()));
        let mut new = old.clone();
        new.width += 1_000;
        let diff = diff_presentations("old.pptx", "new.pptx", &old, &new, &DiffOptions::default());
        assert!(diff.has_changes());
        assert_eq!(diff.summary.slides_modified, 0);
        assert_eq!(diff.slides[0].status, Status::Unchanged);
        assert_ne!(diff.old_size, diff.new_size);
    }

    #[test]
    fn 移動スライドが混ざっても削除位置を旧順に保つ() {
        let slide = |old_index: usize, new_index: Option<usize>| SlideDiff {
            old_index: Some(old_index),
            new_index,
            status: if new_index.is_some() {
                Status::Moved
            } else {
                Status::Removed
            },
            title: None,
            match_confidence: None,
            shapes: Vec::new(),
            old_notes: None,
            new_notes: None,
            notes_changed: false,
        };
        let mut slides = vec![
            slide(5, Some(1)),
            slide(1, Some(2)),
            slide(4, Some(3)),
            slide(2, None),
            slide(3, None),
        ];
        reposition_removed(&mut slides);
        assert_eq!(
            slides
                .iter()
                .map(|slide| slide.old_index.unwrap())
                .collect::<Vec<_>>(),
            vec![2, 3, 5, 1, 4]
        );
    }

    fn table_presentation(table: TableIr) -> PresentationIr {
        PresentationIr {
            width: 10_000,
            height: 10_000,
            slides: vec![SlideIr {
                part: "ppt/slides/slide1.xml".to_string(),
                index: 1,
                shapes: vec![ShapeIr {
                    id: "2".to_string(),
                    name: "Table".to_string(),
                    kind: ShapeKind::Table,
                    placeholder: None,
                    frame: None,
                    z: 0,
                    appearance_signature: String::new(),
                    paragraphs: Vec::new(),
                    table: Some(table),
                    image: None,
                }],
                notes: None,
            }],
            media: Default::default(),
        }
    }

    fn unicode_text() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop_oneof![
                3 => any::<char>().prop_map(|character| character.to_string()),
                2 => prop::sample::select(vec![
                    "a", "日", "\u{0301}", "\u{200d}", "👩", "💻", "🏽", "🇯", "🇵", "\r",
                    "\n", "　",
                ])
                .prop_map(str::to_owned),
            ],
            0..24,
        )
        .prop_map(|parts| parts.concat())
    }

    fn boundaries(text: &str) -> BTreeSet<usize> {
        std::iter::once(0)
            .chain(
                text.grapheme_indices(true)
                    .map(|(start, grapheme)| start + grapheme.len()),
            )
            .collect()
    }

    #[test]
    fn 連続する複数段落の書き換えをそれぞれ変更として対応付ける() {
        let old = [
            paragraph("売上は100万円です".to_string()),
            paragraph("利益は20万円です".to_string()),
        ];
        let new = [
            paragraph("売上は110万円です".to_string()),
            paragraph("利益は25万円です".to_string()),
        ];

        let changes = diff_paragraphs(&old, &new, &DiffOptions::default());
        assert_eq!(changes.len(), 2, "{changes:?}");
        assert!(
            changes
                .iter()
                .all(|change| change.kind == ChangeKind::Changed),
            "{changes:?}"
        );
    }

    #[test]
    fn 単一段落の総取り替えは削除追加ではなく変更にする() {
        let old = [paragraph("第一四半期".to_string())];
        let new = [paragraph("Completely replaced".to_string())];

        let changes = diff_paragraphs(&old, &new, &DiffOptions::default());
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(changes[0].kind, ChangeKind::Changed);
        assert_eq!(changes[0].old_index, Some(1));
        assert_eq!(changes[0].new_index, Some(1));
    }

    #[test]
    fn 本文が同じでも段落書式の変更を報告する() {
        let old = [paragraph("同じ本文".to_string())];
        let mut bold = old[0].clone();
        bold.bold = true;
        let mut level = old[0].clone();
        level.level = 1;
        let mut size = old[0].clone();
        size.size = Some(24.0);
        let mut align = old[0].clone();
        align.align = Some("ctr".to_string());

        for new in [bold, level, size, align] {
            let changes = diff_paragraphs(&old, &[new], &DiffOptions::default());
            assert_eq!(changes.len(), 1, "{changes:?}");
            assert_eq!(changes[0].kind, ChangeKind::Changed);
            assert!(changes[0].format_changed);
            assert!(changes[0].segments.is_empty());
        }
    }

    #[test]
    fn Unicode正規化形式とソフトハイフンだけの違いは変更にしない() {
        for (old, new) in [
            ("Caf\u{e9}", "Cafe\u{301}"),
            ("international", "inter\u{ad}national"),
        ] {
            let diff = diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(paragraph(old.to_string())),
                &presentation(paragraph(new.to_string())),
                &DiffOptions::default(),
            );
            assert!(!diff.has_changes(), "{old:?} / {new:?}: {diff:?}");
        }
    }

    fn placeholder_shape(id: &str, text: &str, x: i64, z: usize) -> ShapeIr {
        ShapeIr {
            id: id.to_string(),
            name: "Content Placeholder".to_string(),
            kind: ShapeKind::Text,
            placeholder: Some(crate::ir::Placeholder {
                kind: "body".to_string(),
                index: None,
            }),
            frame: Some(Frame {
                x,
                y: 0,
                width: 4_000,
                height: 2_000,
            }),
            z,
            appearance_signature: String::new(),
            paragraphs: vec![paragraph(text.to_string())],
            table: None,
            image: None,
        }
    }

    #[test]
    fn duplicate_placeholders_are_paired_by_content_instead_of_sequence() {
        let mut old = presentation(paragraph("unused".to_string()));
        old.slides[0].shapes = vec![
            placeholder_shape("1", "Alpha", 0, 0),
            placeholder_shape("2", "Beta", 500_000, 1),
        ];
        let mut new = presentation(paragraph("unused".to_string()));
        new.slides[0].shapes = vec![
            placeholder_shape("20", "Beta", 500_000, 0),
            placeholder_shape("10", "Alpha", 0, 1),
        ];

        let diff = diff_presentations("old.pptx", "new.pptx", &old, &new, &DiffOptions::default());

        assert_eq!(diff.summary.paragraphs_changed, 0, "{diff:?}");
        assert!(
            diff.slides[0]
                .shapes
                .iter()
                .all(|shape| !shape.frame_changed),
            "{diff:?}"
        );
    }

    #[test]
    fn duplicate_shape_ids_keep_new_drawing_order() {
        let mut alpha = placeholder_shape("1", "Alpha", 0, 0);
        alpha.name = "Alpha shape".to_string();
        let mut beta = placeholder_shape("1", "Beta", 500_000, 1);
        beta.name = "Beta shape".to_string();

        let mut old = presentation(paragraph("unused".to_string()));
        old.slides[0].shapes = vec![alpha.clone(), beta.clone()];
        let mut new = presentation(paragraph("unused".to_string()));
        new.slides[0].shapes = vec![beta, alpha];

        let diff = diff_presentations("old.pptx", "new.pptx", &old, &new, &DiffOptions::default());
        let names: Vec<&str> = diff.slides[0]
            .shapes
            .iter()
            .map(|shape| shape.name.as_str())
            .collect();
        assert_eq!(names, ["Beta shape", "Alpha shape"], "{diff:?}");
        assert_eq!(diff.summary.paragraphs_changed, 0, "{diff:?}");
    }

    #[test]
    fn duplicate_slide_text_is_disambiguated_by_layout() {
        fn slide(index: usize, x: i64) -> SlideIr {
            SlideIr {
                part: format!("ppt/slides/slide{index}.xml"),
                index,
                shapes: vec![ShapeIr {
                    id: "1".to_string(),
                    name: "Title".to_string(),
                    kind: ShapeKind::Text,
                    placeholder: Some(crate::ir::Placeholder {
                        kind: "title".to_string(),
                        index: None,
                    }),
                    frame: Some(Frame {
                        x,
                        y: 0,
                        width: 4_000,
                        height: 1_000,
                    }),
                    z: 0,
                    appearance_signature: String::new(),
                    paragraphs: vec![paragraph("Same title".to_string())],
                    table: None,
                    image: None,
                }],
                notes: None,
            }
        }

        let old = PresentationIr {
            width: 10_000,
            height: 10_000,
            slides: vec![slide(1, 0), slide(2, 500_000)],
            media: Default::default(),
        };
        let new = PresentationIr {
            width: 10_000,
            height: 10_000,
            slides: vec![slide(1, 500_000), slide(2, 0)],
            media: Default::default(),
        };

        let diff = diff_presentations("old.pptx", "new.pptx", &old, &new, &DiffOptions::default());

        assert_eq!(diff.summary.slides_modified, 0, "{diff:?}");
        // 2 枚の入れ替えは片方を錨にできるため、最小編集として移動は 1 枚。
        assert_eq!(diff.summary.slides_moved, 1, "{diff:?}");
        assert_eq!(diff.summary.shapes_moved, 0, "{diff:?}");
    }

    #[test]
    fn セル本文が同じでも表レイアウトの変更を報告する() {
        let old = table_presentation(TableIr {
            rows: vec![vec!["項目".to_string(), "金額".to_string()]],
            column_widths: vec![1_000_000, 2_000_000],
            row_heights: vec![300_000],
        });
        let new = table_presentation(TableIr {
            rows: vec![vec!["項目".to_string(), "金額".to_string()]],
            column_widths: vec![1_100_000, 1_900_000],
            row_heights: vec![400_000],
        });

        let diff = diff_presentations("old.pptx", "new.pptx", &old, &new, &DiffOptions::default());
        assert_eq!(diff.summary.slides_modified, 1, "{diff:?}");
        assert_eq!(diff.summary.table_layouts_changed, 1);
        assert_eq!(diff.summary.table_cells_changed, 0);
        assert!(diff.slides[0].shapes[0]
            .table
            .as_ref()
            .is_some_and(|table| table.layout_changed));
    }

    #[test]
    fn 表レイアウトの微小な丸め差は許容する() {
        assert!(!dimensions_changed(&[100_000], &[109_144], 9_144));
        assert!(dimensions_changed(&[100_000], &[109_145], 9_144));
        assert!(dimensions_changed(&[100_000], &[100_000, 0], 9_144));
    }

    fn rows(cells: &[&[&str]]) -> Vec<Vec<String>> {
        cells
            .iter()
            .map(|row| row.iter().map(|cell| cell.to_string()).collect())
            .collect()
    }

    #[test]
    fn 数値行の総取り替えをセル変更として報告する() {
        let old = table(rows(&[&["金額"], &["100"], &["合計"]]));
        let new = table(rows(&[&["金額"], &["999"], &["合計"]]));
        let diff = diff_table(&old, &new, &DiffOptions::default());
        assert_eq!(diff.rows.len(), 3, "{diff:?}");
        assert_eq!(diff.rows[1].kind, ChangeKind::Changed);
        assert_eq!(diff.rows[1].cells[0].kind, ChangeKind::Changed);
        assert_eq!(diff.rows[1].old_index, Some(2));
        assert_eq!(diff.rows[1].new_index, Some(2));
    }

    #[test]
    fn セル内のタブを列境界と混同せず挿入行を対応付ける() {
        let old = table(rows(&[&["a\tb", "c"], &["a", "b\tc"]]));
        let new = table(rows(&[&["a", "b\tc"], &["a\tb", "c"], &["a", "b\tc"]]));
        let diff = diff_table(&old, &new, &DiffOptions::default());
        assert_eq!(diff.rows.len(), 3, "{diff:?}");
        assert_eq!(diff.rows[0].kind, ChangeKind::Added, "{diff:?}");
        for row in &diff.rows[1..] {
            assert_eq!(row.kind, ChangeKind::Unchanged, "{diff:?}");
        }
    }

    #[test]
    fn セル内の改行を行境界と混同せず挿入列を対応付ける() {
        let old = table(rows(&[&["a\nb", "a"], &["c", "b\nc"]]));
        let new = table(rows(&[&["a", "a\nb", "a"], &["b\nc", "c", "b\nc"]]));
        let diff = diff_table(&old, &new, &DiffOptions::default());
        assert_eq!(diff.rows.len(), 2, "{diff:?}");
        for row in &diff.rows {
            assert_eq!(row.cells[0].kind, ChangeKind::Added, "{diff:?}");
            assert!(
                row.cells[1..]
                    .iter()
                    .all(|cell| cell.kind == ChangeKind::Unchanged),
                "{diff:?}"
            );
        }
    }

    #[test]
    fn 列の挿入では右側の列を変更扱いにしない() {
        let old = table(rows(&[
            &["区分", "金額", "前年比"],
            &["商品", "200", "105%"],
            &["サービス", "80", "98%"],
        ]));
        let new = table(rows(&[
            &["区分", "備考", "金額", "前年比"],
            &["商品", "主力", "200", "105%"],
            &["サービス", "", "80", "98%"],
        ]));

        let diff = diff_table(&old, &new, &DiffOptions::default());
        assert_eq!(diff.rows.len(), 3, "{diff:?}");
        for (index, row) in diff.rows.iter().enumerate() {
            assert_eq!(row.old_index, Some(index + 1), "{row:?}");
            assert_eq!(row.new_index, Some(index + 1), "{row:?}");
            let changed: Vec<_> = row
                .cells
                .iter()
                .filter(|cell| cell.kind != ChangeKind::Unchanged)
                .collect();
            assert_eq!(changed.len(), 1, "{row:?}");
            assert_eq!(changed[0].kind, ChangeKind::Added);
            assert_eq!(changed[0].old_column, None);
            assert_eq!(changed[0].new_column, Some(2));
            assert_eq!(changed[0].column, 2);
        }
        // 右へずれた列は、両側の列番号を持つ。
        let amount = &diff.rows[1].cells[2];
        assert_eq!(amount.kind, ChangeKind::Unchanged);
        assert_eq!(
            (amount.old_column, amount.new_column, amount.column),
            (Some(2), Some(3), 3)
        );
    }

    #[test]
    fn 列の削除とセルの書き換えを同時に報告する() {
        let old = table(rows(&[
            &["区分", "備考", "金額"],
            &["商品", "主力", "200"],
            &["サービス", "新規", "80"],
        ]));
        let new = table(rows(&[
            &["区分", "金額"],
            &["商品", "250"],
            &["サービス", "80"],
        ]));

        let diff = diff_table(&old, &new, &DiffOptions::default());
        assert_eq!(diff.rows.len(), 3, "{diff:?}");
        let removed: Vec<_> = diff
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .filter(|cell| cell.kind == ChangeKind::Removed)
            .collect();
        assert_eq!(removed.len(), 3, "{diff:?}");
        assert!(removed
            .iter()
            .all(|cell| cell.old_column == Some(2) && cell.new_column.is_none()));

        let product = &diff.rows[1];
        assert_eq!(product.kind, ChangeKind::Changed);
        let amount = product
            .cells
            .iter()
            .find(|cell| cell.kind == ChangeKind::Changed)
            .expect("金額セルの変更");
        assert_eq!((amount.old_column, amount.new_column), (Some(3), Some(2)));
        assert_eq!(amount.old_text.as_deref(), Some("200"));
        assert_eq!(amount.new_text.as_deref(), Some("250"));
    }

    #[test]
    fn 列数が同じなら列は位置どおりに対応させる() {
        let old = table(rows(&[&["項目", "金額"], &["売上高", "1200"]]));
        let new = table(rows(&[&["項目", "金額(円)"], &["売上高", "1300"]]));

        let diff = diff_table(&old, &new, &DiffOptions::default());
        let cells: Vec<_> = diff.rows.iter().flat_map(|row| &row.cells).collect();
        assert!(
            cells
                .iter()
                .all(|cell| cell.old_column == cell.new_column
                    && Some(cell.column) == cell.new_column)
        );
        assert_eq!(
            cells
                .iter()
                .filter(|cell| cell.kind == ChangeKind::Changed)
                .count(),
            2
        );
    }

    #[test]
    fn 長い段落の一部の変更は前後を削って内訳を出す() {
        let old: String = (0..3_000).map(|index| format!("文{index}。")).collect();
        let mut new = old.clone();
        let middle = new
            .char_indices()
            .nth(new.chars().count() / 2)
            .map_or(0, |(at, _)| at);
        new.insert_str(middle, "追記");

        let segments = segment_text(&old, &new);
        let inserted: Vec<_> = segments
            .iter()
            .filter(|segment| segment.kind != SegmentKind::Equal)
            .collect();
        assert_eq!(inserted.len(), 1, "{inserted:?}");
        assert_eq!(inserted[0].kind, SegmentKind::Insert);
        assert_eq!(inserted[0].text, "追記");
    }

    proptest! {
        #[test]
        fn 重複セルを圧縮して出現回数を保持する(
            repeats in 1usize..4096,
            other_repeats in 1usize..4096,
        ) {
            let old = vec!["1".to_string(); repeats];
            let new = vec!["1".to_string(); other_repeats];
            let old_profile = CellSequenceProfile::new(&old);
            let new_profile = CellSequenceProfile::new(&new);
            prop_assert_eq!(&old_profile.cells, &vec![("1".to_string(), repeats)]);
            prop_assert_eq!(old_profile.cell_count, repeats);
            let expected = (2.0 * repeats.min(other_repeats) as f64 / (repeats + other_repeats) as f64)
                .max(diff_align::similarity(&old.join("\n"), &new.join("\n")));
            prop_assert_eq!(old_profile.similarity_at_least(&new_profile, 0.0), expected);
        }

        #[test]
        fn 数値キーを持つ複数行の値変更をセル変更に保つ(height in 2usize..8) {
            let old = table((1..=height).map(|id| vec![id.to_string(), "000".to_string()]).collect());
            let new = table((1..=height).map(|id| vec![id.to_string(), "999".to_string()]).collect());
            let diff = diff_table(&old, &new, &DiffOptions::default());
            prop_assert_eq!(diff.rows.len(), height, "{:?}", diff);
            for (index, row) in diff.rows.iter().enumerate() {
                prop_assert_eq!(row.old_index, Some(index + 1));
                prop_assert_eq!(row.new_index, Some(index + 1));
                prop_assert_eq!(row.cells[0].kind, ChangeKind::Unchanged);
                prop_assert_eq!(row.cells[1].kind, ChangeKind::Changed);
            }
        }

        #[test]
        fn セル類似度の枝刈りは多重集合の参照スコアを保つ(
            old in prop::collection::vec(prop_oneof![unicode_text(), Just("1".to_string()), Just("2".to_string())], 0..64),
            new in prop::collection::vec(prop_oneof![unicode_text(), Just("1".to_string()), Just("2".to_string())], 0..64),
            threshold in 0.0f64..=1.0,
        ) {
            let old_profile = CellSequenceProfile::new(&old);
            let new_profile = CellSequenceProfile::new(&new);
            let mut remaining: Vec<_> = new.iter().filter(|cell| !cell.is_empty()).collect();
            let mut common = 0usize;
            for cell in old.iter().filter(|cell| !cell.is_empty()) {
                if let Some(index) = remaining.iter().position(|other| *other == cell) {
                    remaining.remove(index);
                    common += 1;
                }
            }
            let total = old.iter().filter(|cell| !cell.is_empty()).count() + new.iter().filter(|cell| !cell.is_empty()).count();
            let cells = if total == 0 { 0.0 } else { 2.0 * common as f64 / total as f64 };
            let expected = cells.max(diff_align::similarity(&old.join("\n"), &new.join("\n")));
            let actual = old_profile.similarity_at_least(&new_profile, threshold);
            if expected >= threshold { prop_assert_eq!(actual, expected); }
            else { prop_assert!(actual < threshold); }
            prop_assert_eq!(old_profile.similarity_at_least(&new_profile, 0.0), new_profile.similarity_at_least(&old_profile, 0.0));
        }

        #[test]
        fn 短い数値セルの行列挿入で既存セルを変更しない(
            width in 1usize..4,
            height in 1usize..4,
            row_at in 0usize..4,
            col_at in 0usize..4,
        ) {
            let row_at = row_at.min(height);
            let col_at = col_at.min(width);
            let old: Vec<Vec<String>> = (0..height).map(|r|
                (0..width).map(|c| (r * width + c + 1).to_string()).collect()
            ).collect();
            let mut new = old.clone();
            for (r, row) in new.iter_mut().enumerate() { row.insert(col_at, format!("88{r}")); }
            new.insert(row_at, (0..=width).map(|c| format!("99{c}")).collect());
            for (before, after, inserted) in [(&old, &new, true), (&new, &old, false)] {
                let diff = diff_table(&table(before.clone()), &table(after.clone()), &DiffOptions::default());
                prop_assert_eq!(diff.rows.len(), height + 1, "{:?}", diff);
                for row in diff.rows.iter().filter(|r| r.old_index.is_some() && r.new_index.is_some()) {
                    let changed: Vec<_> = row.cells.iter().filter(|c| c.kind != ChangeKind::Unchanged).collect();
                    prop_assert_eq!(changed.len(), 1, "{:?}", diff);
                    prop_assert_eq!(changed[0].kind, if inserted { ChangeKind::Added } else { ChangeKind::Removed });
                    prop_assert_eq!(if inserted { changed[0].new_column } else { changed[0].old_column }, Some(col_at + 1));
                }
                prop_assert_eq!(diff.rows.iter().filter(|r| r.kind == if inserted { ChangeKind::Added } else { ChangeKind::Removed }).count(), 1);
            }
        }

        #[test]
        fn 表の行列挿入と削除は既存セルを変更しない(
            width in 1usize..6,
            height in 1usize..7,
            row_at in 0usize..7,
            col_at in 0usize..6,
            text in prop::sample::select(vec!["", "\t", "\n", "\u{1f}", "e\u{301}", "👩‍💻"]),
        ) {
            let row_at = row_at.min(height);
            let col_at = col_at.min(width);
            let old: Vec<Vec<String>> = (0..height).map(|r|
                (0..width).map(|c| format!("行{r}列{c}:{text}")).collect()
            ).collect();
            let mut new = old.clone();
            for (r, row) in new.iter_mut().enumerate() {
                row.insert(col_at, format!("追加列:{r}"));
            }
            new.insert(row_at, (0..=width).map(|c| format!("追加行:{c}")).collect());
            for (before, after, inserted) in [(&old, &new, true), (&new, &old, false)] {
                let diff = diff_table(&table(before.clone()), &table(after.clone()), &DiffOptions::default());
                prop_assert_eq!(diff.rows.len(), height + 1);
                let changed: Vec<_> = diff.rows.iter().filter(|row|
                    row.kind == if inserted { ChangeKind::Added } else { ChangeKind::Removed }
                ).collect();
                prop_assert_eq!(changed.len(), 1, "{:?}", diff);
                prop_assert_eq!(if inserted { changed[0].new_index } else { changed[0].old_index }, Some(row_at + 1));
                for row in diff.rows.iter().filter(|row| row.old_index.is_some() && row.new_index.is_some()) {
                    let cells: Vec<_> = row.cells.iter().filter(|cell| cell.kind != ChangeKind::Unchanged).collect();
                    prop_assert_eq!(cells.len(), 1, "{:?}", row);
                    prop_assert_eq!(cells[0].kind, if inserted { ChangeKind::Added } else { ChangeKind::Removed });
                    prop_assert_eq!(if inserted { cells[0].new_column } else { cells[0].old_column }, Some(col_at + 1));
                }
            }
        }

        #[test]
        fn 任意のUnicode表の差分は両側のセルを一度ずつ覆う(
            old in prop::collection::vec(prop::collection::vec(unicode_text(), 0..5), 0..6),
            new in prop::collection::vec(prop::collection::vec(unicode_text(), 0..5), 0..6),
        ) {
            let diff = diff_table(&table(old.clone()), &table(new.clone()), &DiffOptions::default());
            for (input, old_side) in [(&old, true), (&new, false)] {
                let mut seen = BTreeSet::new();
                for row in &diff.rows {
                    let Some(r) = (if old_side { row.old_index } else { row.new_index }) else { continue };
                    prop_assert!(seen.insert(r));
                    let mut seen_cells = BTreeSet::new();
                    for cell in &row.cells {
                        let Some(c) = (if old_side { cell.old_column } else { cell.new_column }) else { continue };
                        prop_assert!(seen_cells.insert(c));
                        let text = if old_side { cell.old_text.as_ref() } else { cell.new_text.as_ref() };
                        prop_assert_eq!(text, input[r - 1].get(c - 1));
                    }
                    prop_assert_eq!(seen_cells, (1..=input[r-1].len()).collect::<BTreeSet<_>>());
                }
                prop_assert_eq!(seen, (1..=input.len()).collect::<BTreeSet<_>>());
            }
        }

        #[test]
        fn スライドスコアの枝刈りは対応付け結果を変えない(
            old_body in unicode_text(),
            new_body in unicode_text(),
            old_title in prop::option::of(unicode_text()),
            new_title in prop::option::of(unicode_text()),
            threshold in 0.0f64..1.0,
        ) {
            let old = SlideProfile {
                body: TextProfile::new(&old_body),
                title: old_title.as_deref().map(TextProfile::new),
                layout: vec!["text".to_string()],
            };
            let new = SlideProfile {
                body: TextProfile::new(&new_body),
                title: new_title.as_deref().map(TextProfile::new),
                layout: vec!["text".to_string()],
            };
            let expected = slide_score(&old, &new);
            let actual = slide_score_at_least(&old, &new, threshold);
            if expected >= threshold {
                prop_assert_eq!(actual, expected);
            } else {
                prop_assert!(actual < threshold);
            }
        }

        #[test]
        fn 列を差し込んでも既存の列のセルは変わらない(
            width in 1usize..5,
            height in 1usize..6,
            at in 0usize..5,
        ) {
            let at = at.min(width);
            let old: Vec<Vec<String>> = (0..height)
                .map(|row| (0..width).map(|column| format!("r{row}c{column}")).collect())
                .collect();
            let new: Vec<Vec<String>> = old
                .iter()
                .enumerate()
                .map(|(row, cells)| {
                    let mut cells = cells.clone();
                    cells.insert(at, format!("新しい列{row}"));
                    cells
                })
                .collect();

            let diff = diff_table(&table(old), &table(new), &DiffOptions::default());
            let cells: Vec<_> = diff.rows.iter().flat_map(|row| &row.cells).collect();
            prop_assert_eq!(
                cells.iter().filter(|cell| cell.kind == ChangeKind::Added).count(),
                height
            );
            prop_assert!(cells
                .iter()
                .all(|cell| matches!(cell.kind, ChangeKind::Added | ChangeKind::Unchanged)));
            prop_assert!(cells
                .iter()
                .filter(|cell| cell.kind == ChangeKind::Added)
                .all(|cell| cell.new_column == Some(at + 1)));
        }
    }

    proptest! {
        #[test]
        fn 表の列幅と行高の変更はセル本文が同じでも必ず報告される(
            columns in 1usize..8,
            row_count in 1usize..8,
            width in 100_000i64..2_000_000,
            height in 100_000i64..1_000_000,
            delta in 10_000i64..100_000,
        ) {
            let rows: Vec<Vec<String>> = (0..row_count)
                .map(|row| (0..columns).map(|column| format!("{row}:{column}")).collect())
                .collect();
            let old_table = TableIr {
                rows: rows.clone(),
                column_widths: vec![width; columns],
                row_heights: vec![height; row_count],
            };
            let mut new_table = old_table.clone();
            new_table.column_widths[0] += delta;
            new_table.row_heights[row_count - 1] += delta;

            let diff = diff_presentations(
                "old.pptx",
                "new.pptx",
                &table_presentation(old_table),
                &table_presentation(new_table),
                &DiffOptions::default(),
            );
            prop_assert_eq!(diff.summary.slides_modified, 1);
            prop_assert_eq!(diff.summary.table_layouts_changed, 1);
            prop_assert_eq!(diff.summary.table_cells_changed, 0);
            prop_assert!(diff.slides[0].shapes[0]
                .table
                .as_ref()
                .is_some_and(|table| table.layout_changed));
        }

        #[test]
        fn 段落書式の変更は本文が同じでも必ず報告される(
            text in ".{1,32}",
            level in 0u8..9,
            bold in any::<bool>(),
            size in 8u16..80,
        ) {
            let old = [ParagraphIr {
                text: text.clone(),
                level,
                bold,
                size: Some(size as f32),
                align: Some("l".to_string()),
                format_spans: Vec::new(),
            }];
            let new = [ParagraphIr {
                text,
                level: (level + 1) % 9,
                bold: !bold,
                size: Some(size as f32 + 0.5),
                align: Some("ctr".to_string()),
                format_spans: Vec::new(),
            }];

            let diff = diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(old[0].clone()),
                &presentation(new[0].clone()),
                &DiffOptions::default(),
            );
            prop_assert_eq!(diff.summary.slides_modified, 1);
            prop_assert_eq!(diff.summary.paragraphs_changed, 1);
            let change = &diff.slides[0].shapes[0].paragraphs[0];
            prop_assert_eq!(change.kind, ChangeKind::Changed);
            prop_assert!(change.format_changed);
            prop_assert_eq!(change.old_text.as_deref(), change.new_text.as_deref());
        }

        #[test]
        fn 同じ長さの連続段落変更は削除追加に崩れない(values in prop::collection::vec(0u32..10_000, 1..8)) {
            let old: Vec<_> = values
                .iter()
                .enumerate()
                .map(|(index, value)| paragraph(format!("項目{index}の予算は{value}万円です")))
                .collect();
            let new: Vec<_> = values
                .iter()
                .enumerate()
                .map(|(index, value)| paragraph(format!("項目{index}の予算は{}万円です", value + 10_000)))
                .collect();

            let changes = diff_paragraphs(&old, &new, &DiffOptions::default());
            prop_assert_eq!(changes.len(), values.len());
            prop_assert!(changes.iter().all(|change| change.kind == ChangeKind::Changed));
            for (index, change) in changes.iter().enumerate() {
                prop_assert_eq!(change.old_index, Some(index + 1));
                prop_assert_eq!(change.new_index, Some(index + 1));
            }

            let old_table = table(values
                .iter()
                .enumerate()
                .map(|(index, value)| vec![format!("項目{index}"), format!("{value}万円")])
                .collect());
            let new_table = table(values
                .iter()
                .enumerate()
                .map(|(index, value)| vec![format!("項目{index}"), format!("{}万円", value + 10_000)])
                .collect());
            let table_diff = diff_table(&old_table, &new_table, &DiffOptions::default());
            prop_assert_eq!(table_diff.rows.len(), values.len());
            prop_assert!(table_diff.rows.iter().all(|row| row.kind == ChangeKind::Changed));
            for (index, row) in table_diff.rows.iter().enumerate() {
                prop_assert_eq!(row.old_index, Some(index + 1));
                prop_assert_eq!(row.new_index, Some(index + 1));
            }
        }

        #[test]
        fn Unicode差分は両側を復元でき書記素境界を守る(old in unicode_text(), new in unicode_text()) {
            let segments = segment_text(&old, &new);
            let old_boundaries = boundaries(&old);
            let new_boundaries = boundaries(&new);
            let mut old_at = 0;
            let mut new_at = 0;

            for segment in &segments {
                prop_assert!(!segment.text.is_empty());
                match segment.kind {
                    SegmentKind::Equal => {
                        prop_assert!(old[old_at..].starts_with(&segment.text));
                        prop_assert!(new[new_at..].starts_with(&segment.text));
                        old_at += segment.text.len();
                        new_at += segment.text.len();
                        prop_assert!(old_boundaries.contains(&old_at));
                        prop_assert!(new_boundaries.contains(&new_at));
                    }
                    SegmentKind::Delete => {
                        prop_assert!(old[old_at..].starts_with(&segment.text));
                        old_at += segment.text.len();
                        prop_assert!(old_boundaries.contains(&old_at));
                    }
                    SegmentKind::Insert => {
                        prop_assert!(new[new_at..].starts_with(&segment.text));
                        new_at += segment.text.len();
                        prop_assert!(new_boundaries.contains(&new_at));
                    }
                }
            }
            prop_assert_eq!(old_at, old.len());
            prop_assert_eq!(new_at, new.len());
            for pair in segments.windows(2) {
                prop_assert_ne!(pair[0].kind, pair[1].kind);
            }
        }
    }
}
