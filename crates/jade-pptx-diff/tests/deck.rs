//! 実ファイル (python-pptx が書き出した pptx) に対する結合テスト。
//!
//! フィクスチャの中身は `tests/fixtures/README.md` を参照。生成に使ったのは
//! およそ次のスクリプト:
//!
//! ```python
//! from pptx import Presentation
//! p = Presentation()
//! s = p.slides.add_slide(p.slide_layouts[0])
//! s.shapes.title.text = "四半期報告"
//! s.placeholders[1].text = "2026 Q2"
//! ...
//! p.save("deck-old.pptx")
//! ```

use pptx_diff::{
    diff_presentations, load_presentation, ChangeKind, DiffOptions, Limits, ShapeKind, Status,
};

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

fn load(name: &str) -> pptx_diff::PresentationIr {
    load_presentation(&fixture(name), &Limits::default()).expect("failed to load presentation")
}

#[test]
fn drawingml_color_pipeline_matches_literal_goldens_and_distinguishes_base_colors() {
    let cases: serde_json::Value =
        serde_json::from_slice(&fixture("color-pipeline-goldens.json")).unwrap();
    let mut failures = Vec::new();
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let original = load(&format!("colors-pipeline-{name}-transformed.pptx"));
        for (variant, count) in [("precise", 0), ("base", 2)] {
            let direct = load(&format!("colors-pipeline-{name}-{variant}.pptx"));
            let diff =
                diff_presentations("old", variant, &original, &direct, &DiffOptions::default());
            if diff.summary.shape_appearances_changed != count || (count == 0 && diff.has_changes())
            {
                failures.push(format!("{name}/{variant}: {diff:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn reads_slides_shapes_and_geometry() {
    let deck = load("deck-old.pptx");
    assert_eq!(deck.slides.len(), 3);
    assert_eq!(deck.width, 9_144_000);
    assert_eq!(deck.height, 6_858_000);

    let first = &deck.slides[0];
    assert_eq!(first.title().as_deref(), Some("四半期報告"));

    // プレースホルダは spPr が空でも、レイアウトから位置を継承できていること。
    let title = first
        .shapes
        .iter()
        .find(|shape| {
            shape
                .placeholder
                .as_ref()
                .is_some_and(|ph| ph.kind == "ctrTitle" || ph.kind == "title")
        })
        .expect("title placeholder");
    let frame = title.frame.expect("inherited frame");
    assert!(frame.width > 0 && frame.height > 0, "frame: {frame:?}");

    // 表は行と列を読めていること。
    let table = deck.slides[2]
        .shapes
        .iter()
        .find(|shape| shape.kind == ShapeKind::Table)
        .and_then(|shape| shape.table.as_ref())
        .expect("table shape");
    assert_eq!(table.rows.len(), 2);
    assert_eq!(table.rows[0], vec!["区分", "金額"]);
    assert_eq!(table.rows[1], vec!["商品", "200"]);
    assert_eq!(table.column_widths.len(), 2);
}

#[test]
fn inserting_a_slide_does_not_shift_the_rest() {
    let old = load("deck-old.pptx");
    let new = load("deck-new.pptx");
    let diff = diff_presentations("old", "new", &old, &new, &DiffOptions::default());

    assert_eq!(diff.summary.slides_added, 1);
    assert_eq!(diff.summary.slides_removed, 0);

    // 旧 2 枚目 (売上) は新 3 枚目に、旧 3 枚目 (内訳) は新 4 枚目に対応する。
    let pairs: Vec<(Option<usize>, Option<usize>)> = diff
        .slides
        .iter()
        .map(|slide| (slide.old_index, slide.new_index))
        .collect();
    assert_eq!(
        pairs,
        vec![
            (Some(1), Some(1)),
            (None, Some(2)),
            (Some(2), Some(3)),
            (Some(3), Some(4)),
        ]
    );
    assert!(diff
        .slides
        .iter()
        .all(|slide| slide.status != Status::Moved));
}

#[test]
fn reports_paragraph_level_changes() {
    let old = load("deck-old.pptx");
    let new = load("deck-new.pptx");
    let diff = diff_presentations("old", "new", &old, &new, &DiffOptions::default());

    // 「東京 120」→「東京 150」は書き換え、「名古屋 40」は追加。
    let sales = diff
        .slides
        .iter()
        .find(|slide| slide.title.as_deref() == Some("売上"))
        .expect("sales slide");
    let changes: Vec<(&ChangeKind, Option<&str>, Option<&str>)> = sales
        .shapes
        .iter()
        .flat_map(|shape| shape.paragraphs.iter())
        .filter(|change| change.kind != ChangeKind::Unchanged)
        .map(|change| {
            (
                &change.kind,
                change.old_text.as_deref(),
                change.new_text.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        changes,
        vec![
            (&ChangeKind::Changed, Some("東京 120"), Some("東京 150")),
            (&ChangeKind::Added, None, Some("名古屋 40")),
        ]
    );

    // 書き換えは文字単位の内訳を持つ。
    let rewritten = sales
        .shapes
        .iter()
        .flat_map(|shape| shape.paragraphs.iter())
        .find(|change| change.kind == ChangeKind::Changed)
        .expect("rewritten paragraph");
    let inserted: String = rewritten
        .segments
        .iter()
        .filter(|segment| segment.kind == pptx_diff::SegmentKind::Insert)
        .map(|segment| segment.text.as_str())
        .collect();
    assert_eq!(inserted, "5");
}

#[test]
fn reports_table_cell_changes() {
    let old = load("deck-old.pptx");
    let new = load("deck-new.pptx");
    let diff = diff_presentations("old", "new", &old, &new, &DiffOptions::default());

    assert_eq!(diff.summary.table_cells_changed, 1);
    let cell = diff
        .slides
        .iter()
        .flat_map(|slide| slide.shapes.iter())
        .filter_map(|shape| shape.table.as_ref())
        .flat_map(|table| table.rows.iter())
        .flat_map(|row| row.cells.iter())
        .find(|cell| cell.kind == ChangeKind::Changed)
        .expect("changed cell");
    assert_eq!(cell.old_text.as_deref(), Some("200"));
    assert_eq!(cell.new_text.as_deref(), Some("260"));
}

#[test]
fn identical_decks_have_no_changes() {
    let deck = load("deck-old.pptx");
    let diff = diff_presentations("old", "copy", &deck, &deck, &DiffOptions::default());
    assert!(!diff.has_changes());
    assert_eq!(diff.summary.slides_modified, 0);
    assert_eq!(diff.summary.paragraphs_changed, 0);
}

#[test]
fn serializes_the_documented_contract() {
    let old = load("deck-old.pptx");
    let new = load("deck-new.pptx");
    let diff = diff_presentations("old.pptx", "new.pptx", &old, &new, &DiffOptions::default());
    let json = serde_json::to_value(&diff).unwrap();

    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["coordinate_system"], "one_based");
    assert_eq!(json["old_file"], "old.pptx");
    assert!(json["slides"].as_array().is_some_and(|s| !s.is_empty()));
    assert!(json["old_size"]["width"].as_i64().is_some());
}

#[test]
fn rejects_input_over_the_limit() {
    let limits = Limits {
        max_input_bytes: 16,
        ..Limits::default()
    };
    let error = load_presentation(&fixture("deck-old.pptx"), &limits).unwrap_err();
    assert!(
        error.to_string().contains("max_input_bytes"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_files_that_are_not_pptx() {
    let error = load_presentation(b"not a zip at all", &Limits::default()).unwrap_err();
    assert!(
        error.to_string().contains("ZIP"),
        "unexpected error: {error}"
    );
}

#[test]
fn a_shape_that_only_moved_shows_up_in_the_summary() {
    // 中身は同じで、テキストボックスを 1 つ右へずらしただけの資料。
    // 図形の status は「中身も枠も見て」Modified になるので、枠だけの
    // 変化を要約で拾えないと「1 枚変更」としか言えなくなる。
    let diff = diff_presentations(
        "shift-old.pptx",
        "shift-new.pptx",
        &load("shift-old.pptx"),
        &load("shift-new.pptx"),
        &DiffOptions::default(),
    );

    assert_eq!(diff.summary.slides_modified, 1);
    assert_eq!(diff.summary.shapes_moved, 1, "{:?}", diff.summary);
    assert_eq!(diff.summary.paragraphs_changed, 0);

    let moved: Vec<_> = diff.slides[0]
        .shapes
        .iter()
        .filter(|shape| shape.frame_changed)
        .collect();
    assert_eq!(moved.len(), 1, "動いたのは 1 つだけ");
}

#[test]
fn an_added_slide_is_not_counted_twice() {
    // 1 枚差し込んだだけの資料。「1 枚追加」で言い切れる話を
    // 「図形 n 個追加・段落 m 個追加」まで並べると、何枚直したのかが
    // 読み取れなくなる。中身は明細のほうに入っている。
    let diff = diff_presentations(
        "deck-old.pptx",
        "deck-new.pptx",
        &load("deck-old.pptx"),
        &load("deck-new.pptx"),
        &DiffOptions::default(),
    );

    let added: Vec<_> = diff
        .slides
        .iter()
        .filter(|slide| slide.status == Status::Added)
        .collect();
    assert!(!added.is_empty(), "差し込んだスライドがある");
    assert!(
        added.iter().any(|slide| !slide.shapes.is_empty()),
        "明細には図形が残っている"
    );

    let counted: usize = diff.summary.shapes_added;
    let inside: usize = added.iter().map(|slide| slide.shapes.len()).sum();
    assert!(
        counted < inside,
        "追加スライドの中身まで数えている: {counted} / {inside}"
    );
}

#[test]
fn drawingml_tint_and_shade_match_independent_linear_light_swatches() {
    // LibreOffice 26.8's font-free render gives RGB188 for both transformed
    // swatches, matching explicit BCBCBC, rather than explicit RGB128.
    let old = load("colors-transformed.pptx");
    let rendered = load("colors-linear.pptx");
    let diff = diff_presentations("old", "rendered", &old, &rendered, &DiffOptions::default());
    assert!(!diff.has_changes(), "{diff:?}");
}

#[test]
fn drawingml_tint_and_shade_do_not_alias_different_srgb_swatches() {
    let old = load("colors-transformed.pptx");
    let different = load("colors-wrong-srgb.pptx");
    let diff = diff_presentations(
        "old",
        "different",
        &old,
        &different,
        &DiffOptions::default(),
    );
    assert_eq!(diff.summary.shape_appearances_changed, 2, "{diff:?}");
    assert_eq!(
        diff.slides[0]
            .shapes
            .iter()
            .filter(|shape| shape.appearance_changed)
            .count(),
        2
    );
}

#[test]
fn consecutive_shades_preserve_precision_until_the_final_color() {
    let original = load("colors-chain-transformed.pptx");
    let precise = load("colors-chain-precise.pptx");
    let rounded = load("colors-chain-rounded.pptx");
    assert!(!diff_presentations(
        "old",
        "precise",
        &original,
        &precise,
        &DiffOptions::default()
    )
    .has_changes());
    assert_eq!(
        diff_presentations(
            "old",
            "rounded",
            &original,
            &rounded,
            &DiffOptions::default()
        )
        .summary
        .shape_appearances_changed,
        2
    );
}

#[test]
fn consecutive_tints_and_mixed_shade_tint_preserve_precision() {
    let original = load("colors-chain-tint-transformed.pptx");
    let precise = load("colors-chain-tint-precise.pptx");
    let rounded = load("colors-chain-tint-rounded.pptx");
    assert!(!diff_presentations(
        "old",
        "precise",
        &original,
        &precise,
        &DiffOptions::default()
    )
    .has_changes());
    assert_eq!(
        diff_presentations(
            "old",
            "rounded",
            &original,
            &rounded,
            &DiffOptions::default()
        )
        .summary
        .shape_appearances_changed,
        2
    );
}

#[test]
fn alpha_modulation_matches_explicit_opacity_and_distinguishes_opaque_shapes() {
    let original = load("colors-alpha-transformed.pptx");
    let explicit = load("colors-alpha-explicit.pptx");
    let opaque = load("colors-alpha-opaque.pptx");
    assert!(!diff_presentations(
        "old",
        "explicit",
        &original,
        &explicit,
        &DiffOptions::default()
    )
    .has_changes());
    assert_eq!(
        diff_presentations("old", "opaque", &original, &opaque, &DiffOptions::default())
            .summary
            .shape_appearances_changed,
        2
    );
}

#[test]
fn opacity_between_shades_keeps_color_precision_and_only_the_control_changes() {
    let original = load("colors-chain-alpha-transformed.pptx");
    let precise = load("colors-chain-alpha-precise.pptx");
    let diff = diff_presentations(
        "old",
        "precise",
        &original,
        &precise,
        &DiffOptions::default(),
    );
    assert_eq!(diff.summary.shape_appearances_changed, 1);
    assert!(!diff.slides[0].shapes[0].appearance_changed);
    assert!(!diff.slides[0].shapes[1].appearance_changed);
    assert!(diff.slides[0].shapes[3].appearance_changed);
}

#[test]
fn drawingml_unary_inv_matches_direct_color_and_distinguishes_the_base() {
    let original = load("colors-unary-inv-transformed.pptx");
    let precise = load("colors-unary-inv-precise.pptx");
    let base = load("colors-unary-inv-base.pptx");
    assert!(!diff_presentations(
        "old",
        "precise",
        &original,
        &precise,
        &DiffOptions::default()
    )
    .has_changes());
    assert_eq!(
        diff_presentations("old", "base", &original, &base, &DiffOptions::default())
            .summary
            .shape_appearances_changed,
        2
    );
}

#[test]
fn drawingml_unary_comp_matches_direct_color_and_distinguishes_the_base() {
    let original = load("colors-unary-comp-transformed.pptx");
    let precise = load("colors-unary-comp-precise.pptx");
    let base = load("colors-unary-comp-base.pptx");
    assert!(!diff_presentations(
        "old",
        "precise",
        &original,
        &precise,
        &DiffOptions::default()
    )
    .has_changes());
    assert_eq!(
        diff_presentations("old", "base", &original, &base, &DiffOptions::default())
            .summary
            .shape_appearances_changed,
        1
    );
}

#[test]
fn drawingml_unary_gray_matches_direct_color_and_distinguishes_the_base() {
    let original = load("colors-unary-gray-transformed.pptx");
    let precise = load("colors-unary-gray-precise.pptx");
    let base = load("colors-unary-gray-base.pptx");
    assert!(!diff_presentations(
        "old",
        "precise",
        &original,
        &precise,
        &DiffOptions::default()
    )
    .has_changes());
    assert_eq!(
        diff_presentations("old", "base", &original, &base, &DiffOptions::default())
            .summary
            .shape_appearances_changed,
        1
    );
}

#[test]
fn drawingml_unary_gamma_matches_direct_color_and_distinguishes_the_base() {
    let original = load("colors-unary-gamma-transformed.pptx");
    let precise = load("colors-unary-gamma-precise.pptx");
    let base = load("colors-unary-gamma-base.pptx");
    assert!(!diff_presentations(
        "old",
        "precise",
        &original,
        &precise,
        &DiffOptions::default()
    )
    .has_changes());
    assert_eq!(
        diff_presentations("old", "base", &original, &base, &DiffOptions::default())
            .summary
            .shape_appearances_changed,
        2
    );
}

#[test]
fn drawingml_unary_inverse_gamma_matches_direct_color_and_distinguishes_the_base() {
    let original = load("colors-unary-invGamma-transformed.pptx");
    let precise = load("colors-unary-invGamma-precise.pptx");
    let base = load("colors-unary-invGamma-base.pptx");
    assert!(!diff_presentations(
        "old",
        "precise",
        &original,
        &precise,
        &DiffOptions::default()
    )
    .has_changes());
    assert_eq!(
        diff_presentations("old", "base", &original, &base, &DiffOptions::default())
            .summary
            .shape_appearances_changed,
        2
    );
}

#[test]
fn ordered_gamma_complement_and_shade_keep_precision_and_opacity() {
    let original = load("colors-unary-ordered-transformed.pptx");
    let precise = load("colors-unary-ordered-precise.pptx");
    let diff = diff_presentations(
        "old",
        "precise",
        &original,
        &precise,
        &DiffOptions::default(),
    );
    assert_eq!(diff.summary.shape_appearances_changed, 1);
    assert!(!diff.slides[0].shapes[0].appearance_changed);
    assert!(!diff.slides[0].shapes[1].appearance_changed);
    assert!(diff.slides[0].shapes[3].appearance_changed);
}
