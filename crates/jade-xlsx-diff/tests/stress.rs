use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use xlsx_diff::{CellIr, CellValueKind, ChangeKind, SheetIr, WorkbookIr, diff_workbooks};

#[derive(Debug, Deserialize)]
struct StressCases {
    schema_version: u32,
    duplicate_rows: DuplicateRows,
    wide_sparse: WideSparse,
    multi_sheet: MultiSheet,
}

#[derive(Debug, Deserialize, Serialize)]
struct DuplicateRows {
    row_count: usize,
    insert_at_row: usize,
}

#[derive(Debug, Deserialize, Serialize)]
struct WideSparse {
    old_column_count: usize,
    insert_at_column: usize,
}

#[derive(Debug, Deserialize, Serialize)]
struct MultiSheet {
    sheet_count: usize,
    target_sheet: usize,
    insert_at_row: usize,
}

#[test]
fn duplicate_heavy_512_row_insertion_is_precise_and_deterministic() -> Result<()> {
    let config = stress_cases()?.duplicate_rows;
    let mut old_rows = vec![row(&["id", "value"])];
    old_rows.extend((0..config.row_count).map(|_| row(&["duplicate", "same"])));
    let mut new_rows = old_rows.clone();
    new_rows.insert(config.insert_at_row - 1, row(&["P3-UNIQUE", "inserted"]));
    let old = workbook(vec![sheet("duplicates", old_rows)]);
    let new = workbook(vec![sheet("duplicates", new_rows)]);

    let render = || {
        let diff = diff_workbooks(
            Path::new("duplicates-old.xlsx"),
            Path::new("duplicates-new.xlsx"),
            &old,
            &new,
        )?;
        Ok::<_, anyhow::Error>((serde_json::to_string(&diff)?, diff))
    };
    let (first_json, first) = render()?;
    let (second_json, _) = render()?;

    assert_eq!(first_json, second_json);
    assert_eq!(first.summary.rows_added, 1);
    assert_eq!(first.summary.rows_removed, 0);
    assert_eq!(first.summary.rows_moved, 0);
    assert_eq!(first.summary.cells_changed, 0);
    assert_eq!(
        first.sheets[0].row_changes[0].new_index,
        Some(config.insert_at_row)
    );
    assert!(matches!(
        first.sheets[0].row_changes[0].kind,
        ChangeKind::Added
    ));
    assert!(first.sheets[0].alignment.row_ambiguous_signature_groups > 0);
    // gpui が serde_json の preserve_order を有効にしてもキー順が変わらないよう整列する。
    insta::with_settings!({ sort_maps => true }, {
        insta::assert_json_snapshot!(
            "p3_duplicate_heavy_result",
            serde_json::json!({
                "input": config,
                "summary": first.summary,
                "sheet": first.sheets[0],
            })
        );
    });
    Ok(())
}

#[test]
fn excel_max_width_sparse_column_insertion_stays_bounded_and_precise() -> Result<()> {
    let config = stress_cases()?.wide_sparse;
    let mut old_rows = vec![vec![CellIr::default(); config.old_column_count]; 2];
    old_rows[0][0] = string_cell("left-header");
    old_rows[1][0] = string_cell("left-value");
    old_rows[0][config.old_column_count - 1] = string_cell("right-header");
    old_rows[1][config.old_column_count - 1] = string_cell("right-value");
    let mut new_rows = old_rows.clone();
    for (row_idx, row) in new_rows.iter_mut().enumerate() {
        row.insert(
            config.insert_at_column - 1,
            string_cell(if row_idx == 0 {
                "inserted-header"
            } else {
                "inserted-value"
            }),
        );
    }
    let old = workbook(vec![sheet("wide-sparse", old_rows)]);
    let new = workbook(vec![sheet("wide-sparse", new_rows)]);

    let diff = diff_workbooks(
        Path::new("wide-old.xlsx"),
        Path::new("wide-new.xlsx"),
        &old,
        &new,
    )?;

    assert_eq!(diff.summary.columns_added, 1);
    assert_eq!(diff.summary.columns_removed, 0);
    assert_eq!(diff.summary.columns_moved, 0);
    assert_eq!(diff.summary.rows_added, 0);
    assert_eq!(diff.summary.cells_changed, 0);
    assert_eq!(
        diff.sheets[0].column_changes[0].new_index,
        Some(config.insert_at_column)
    );
    insta::with_settings!({ sort_maps => true }, {
        insta::assert_json_snapshot!(
            "p3_wide_sparse_result",
            serde_json::json!({
                "input": config,
                "summary": diff.summary,
                "sheet": diff.sheets[0],
            })
        );
    });
    Ok(())
}

#[test]
fn multi_sheet_workbook_isolates_one_structural_change() -> Result<()> {
    let config = stress_cases()?.multi_sheet;
    let old_sheets = (0..config.sheet_count)
        .map(|idx| {
            sheet(
                &format!("Sheet-{idx:03}"),
                vec![row(&["id", "value"]), row(&[&format!("ID-{idx}"), "old"])],
            )
        })
        .collect::<Vec<_>>();
    let mut new_sheets = old_sheets.clone();
    new_sheets[config.target_sheet - 1]
        .rows
        .insert(config.insert_at_row - 1, row(&["P3-NEW", "inserted"]));
    let old = workbook(old_sheets);
    let new = workbook(new_sheets);

    let diff = diff_workbooks(
        Path::new("multi-old.xlsx"),
        Path::new("multi-new.xlsx"),
        &old,
        &new,
    )?;

    assert_eq!(diff.summary.rows_added, 1);
    assert_eq!(diff.summary.rows_removed, 0);
    assert_eq!(diff.summary.cells_changed, 0);
    let changed = diff
        .sheets
        .iter()
        .filter(|sheet| !sheet.row_changes.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(changed.len(), 1);
    insta::with_settings!({ sort_maps => true }, {
        insta::assert_json_snapshot!(
            "p3_multi_sheet_result",
            serde_json::json!({
                "input": config,
                "summary": diff.summary,
                "changed_sheets": changed,
            })
        );
    });
    Ok(())
}

#[test]
fn tall_wide_sheet_uses_headers_to_isolate_a_row_insertion_and_cell_edits() -> Result<()> {
    const ROWS: usize = 2_000;
    const COLUMNS: usize = 50;
    let header = (0..COLUMNS)
        .map(|column| string_cell(&format!("column-{column:02}")))
        .collect::<Vec<_>>();
    let mut old_rows = Vec::with_capacity(ROWS);
    old_rows.push(header);
    old_rows.extend((0..ROWS - 1).map(|row| {
        (0..COLUMNS)
            .map(|column| string_cell(&format!("R{row:04}-C{column:02}")))
            .collect()
    }));
    let mut new_rows = old_rows.clone();
    new_rows.insert(
        ROWS / 2,
        (0..COLUMNS)
            .map(|column| string_cell(&format!("inserted-{column:02}")))
            .collect(),
    );
    for (row, column) in [(1, COLUMNS - 1), (ROWS / 2 - 1, COLUMNS / 2), (ROWS, 1)] {
        new_rows[row][column].value = format!("changed-{row}-{column}");
    }

    let diff = diff_workbooks(
        Path::new("tall-wide-old.xlsx"),
        Path::new("tall-wide-new.xlsx"),
        &workbook(vec![sheet("data", old_rows)]),
        &workbook(vec![sheet("data", new_rows)]),
    )?;

    assert_eq!(diff.summary.rows_added, 1);
    assert_eq!(diff.summary.rows_removed, 0);
    assert_eq!(diff.summary.columns_added, 0);
    assert_eq!(diff.summary.columns_removed, 0);
    assert_eq!(diff.summary.cells_changed, 3);
    assert_eq!(diff.sheets[0].row_changes[0].new_index, Some(ROWS / 2 + 1));
    Ok(())
}

fn stress_cases() -> Result<StressCases> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/p3/stress-cases.json");
    let content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let cases: StressCases = serde_json::from_str(&content).context("invalid P3 stress fixture")?;
    anyhow::ensure!(
        cases.schema_version == 1,
        "unsupported P3 stress fixture schema"
    );
    Ok(cases)
}

fn workbook(sheets: Vec<SheetIr>) -> WorkbookIr {
    WorkbookIr {
        sheets,
        default_style_signature: String::new(),
        explicit_cell_styles: Default::default(),
    }
}

fn sheet(name: &str, rows: Vec<Vec<CellIr>>) -> SheetIr {
    SheetIr {
        name: name.to_string(),
        start_row: 1,
        start_col: 1,
        rows,
        merged_ranges: Vec::new(),
    }
}

fn row(values: &[&str]) -> Vec<CellIr> {
    values.iter().map(|value| string_cell(value)).collect()
}

fn string_cell(value: &str) -> CellIr {
    CellIr {
        value: value.to_string(),
        value_kind: CellValueKind::String,
        formula: None,
        style_signature: String::new(),
    }
}
