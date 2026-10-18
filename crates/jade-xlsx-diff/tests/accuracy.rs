use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Result;
use xlsx_diff::{
    CellChangeKind, CellIr, CellValueKind, ChangeKind, DiffOptions, FormulaValueComparison,
    SheetIr, SheetStatus, WorkbookDiff, WorkbookIr, diff_workbooks, diff_workbooks_with_options,
};

#[test]
fn synthetic_golden_corpus_has_perfect_structural_precision_and_recall() -> Result<()> {
    for case in golden_cases()? {
        let observed = structural_events(&case.diff);
        let true_positives = observed.intersection(&case.expected).count();
        let precision = true_positives as f64 / observed.len().max(1) as f64;
        let recall = true_positives as f64 / case.expected.len().max(1) as f64;

        assert_eq!(observed, case.expected, "golden case `{}`", case.name);
        assert_eq!(precision, 1.0, "precision for `{}`", case.name);
        assert_eq!(recall, 1.0, "recall for `{}`", case.name);
    }
    Ok(())
}

struct GoldenCase {
    name: &'static str,
    diff: WorkbookDiff,
    expected: BTreeSet<String>,
}

fn golden_cases() -> Result<Vec<GoldenCase>> {
    let old = workbook(vec![sheet(
        "Data",
        &[&["id", "value"], &["A", "10"], &["B", "20"]],
    )]);
    let new = workbook(vec![sheet(
        "Data",
        &[
            &["id", "note", "value"],
            &["A", "x", "10"],
            &["NEW", "x", "15"],
            &["B", "x", "20"],
        ],
    )]);
    let compound = GoldenCase {
        name: "compound row and column insertion",
        diff: diff_workbooks(Path::new("old.xlsx"), Path::new("new.xlsx"), &old, &new)?,
        expected: BTreeSet::from([
            "column:added:Data:2".to_string(),
            "row:added:Data:3".to_string(),
        ]),
    };

    let mut old_summary = sheet("Summary", &[&["id", "total"], &["A", "20"]]);
    old_summary.rows[1][1].formula = Some("=Data!B3".to_string());
    let old = workbook(vec![
        sheet("Data", &[&["id", "value"], &["A", "10"], &["B", "20"]]),
        old_summary,
    ]);
    let mut new_summary = sheet("Summary", &[&["id", "total"], &["A", "20"]]);
    new_summary.rows[1][1].formula = Some("='Data 2026'!B4".to_string());
    let new = workbook(vec![
        sheet(
            "Data 2026",
            &[&["id", "value"], &["A", "10"], &["NEW", "15"], &["B", "20"]],
        ),
        new_summary,
    ]);
    let cross_sheet_formula = GoldenCase {
        name: "renamed cross-sheet formula shift",
        diff: diff_workbooks(Path::new("old.xlsx"), Path::new("new.xlsx"), &old, &new)?,
        expected: BTreeSet::from([
            "row:added:Data 2026:3".to_string(),
            "sheet:renamed:Data->Data 2026".to_string(),
        ]),
    };

    let mut old_calc = sheet("Calc", &[&["id", "total"], &["A", "2"]]);
    old_calc.rows[1][1].formula = Some("=1+1".to_string());
    let mut new_calc = old_calc.clone();
    new_calc.rows[1][1].value = "stale".to_string();
    let options = DiffOptions {
        formula_values: FormulaValueComparison::CompareCached,
        ..DiffOptions::default()
    };
    let strict_cache = GoldenCase {
        name: "strict cached formula value",
        diff: diff_workbooks_with_options(
            Path::new("old.xlsx"),
            Path::new("new.xlsx"),
            &workbook(vec![old_calc]),
            &workbook(vec![new_calc]),
            &options,
        )?,
        expected: BTreeSet::from(["cell:value_changed:Calc:B2->B2".to_string()]),
    };

    Ok(vec![compound, cross_sheet_formula, strict_cache])
}

fn structural_events(diff: &WorkbookDiff) -> BTreeSet<String> {
    let mut events = BTreeSet::new();
    for sheet in &diff.sheets {
        let name = sheet
            .new_name
            .as_deref()
            .or(sheet.old_name.as_deref())
            .unwrap_or("<unknown>");
        if matches!(&sheet.status, SheetStatus::Renamed) {
            events.insert(format!(
                "sheet:renamed:{}->{}",
                sheet.old_name.as_deref().unwrap_or("<unknown>"),
                sheet.new_name.as_deref().unwrap_or("<unknown>")
            ));
        }
        for change in &sheet.row_changes {
            let (kind, index) = match &change.kind {
                ChangeKind::Added => ("added", change.new_index),
                ChangeKind::Removed => ("removed", change.old_index),
                ChangeKind::Moved => ("moved", change.new_index),
            };
            events.insert(format!("row:{kind}:{name}:{}", index.unwrap_or(0)));
        }
        for change in &sheet.column_changes {
            let (kind, index) = match &change.kind {
                ChangeKind::Added => ("added", change.new_index),
                ChangeKind::Removed => ("removed", change.old_index),
                ChangeKind::Moved => ("moved", change.new_index),
            };
            events.insert(format!("column:{kind}:{name}:{}", index.unwrap_or(0)));
        }
        for change in &sheet.cell_changes {
            let kind = match &change.kind {
                CellChangeKind::ValueChanged => "value_changed",
                CellChangeKind::FormulaChanged => "formula_changed",
                CellChangeKind::ValueAndFormulaChanged => "value_and_formula_changed",
                CellChangeKind::StyleChanged => "style_changed",
                CellChangeKind::ValueAdded => "value_added",
                CellChangeKind::ValueRemoved => "value_removed",
            };
            events.insert(format!(
                "cell:{kind}:{name}:{}->{}",
                change.old_address.as_deref().unwrap_or("-"),
                change.new_address.as_deref().unwrap_or("-")
            ));
        }
    }
    events
}

fn workbook(sheets: Vec<SheetIr>) -> WorkbookIr {
    WorkbookIr {
        sheets,
        default_style_signature: String::new(),
        explicit_cell_styles: Default::default(),
    }
}

fn sheet(name: &str, rows: &[&[&str]]) -> SheetIr {
    SheetIr {
        name: name.to_string(),
        start_row: 1,
        start_col: 1,
        rows: rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|value| CellIr {
                        value: (*value).to_string(),
                        value_kind: if value.is_empty() {
                            CellValueKind::Empty
                        } else {
                            CellValueKind::String
                        },
                        formula: None,
                        style_signature: String::new(),
                    })
                    .collect()
            })
            .collect(),
        merged_ranges: Vec::new(),
    }
}
