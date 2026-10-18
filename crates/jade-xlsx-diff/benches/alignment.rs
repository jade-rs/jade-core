use std::hint::black_box;
use std::path::Path;

use criterion::{Criterion, criterion_group, criterion_main};
use xlsx_diff::{CellIr, CellValueKind, SheetIr, WorkbookIr, diff_workbooks};

fn workbook(rows: usize, inserted_at: Option<usize>) -> WorkbookIr {
    let mut data = Vec::with_capacity(rows + 1);
    data.push(row(&["id", "name", "amount"]));
    for idx in 0..rows {
        if inserted_at == Some(idx) {
            data.push(row(&["inserted", "new item", "999"]));
        }
        data.push(row(&[
            &format!("ID-{idx:06}"),
            &format!("item-{idx}"),
            &idx.to_string(),
        ]));
    }
    WorkbookIr {
        sheets: vec![SheetIr {
            name: "data".to_string(),
            start_row: 1,
            start_col: 1,
            rows: data,
            merged_ranges: Vec::new(),
        }],
        default_style_signature: String::new(),
        explicit_cell_styles: Default::default(),
    }
}

fn row(values: &[&str]) -> Vec<CellIr> {
    values
        .iter()
        .map(|value| CellIr {
            value: (*value).to_string(),
            value_kind: CellValueKind::String,
            formula: None,
            style_signature: String::new(),
        })
        .collect()
}

fn alignment_benchmark(criterion: &mut Criterion) {
    let old = workbook(10_000, None);
    let new = workbook(10_000, Some(5_000));
    criterion.bench_function("align 10k rows with one insertion", |bencher| {
        bencher.iter(|| {
            black_box(
                diff_workbooks(
                    Path::new("old.xlsx"),
                    Path::new("new.xlsx"),
                    black_box(&old),
                    black_box(&new),
                )
                .expect("benchmark diff should succeed"),
            )
        })
    });

    let duplicate_old = duplicate_workbook(512, None);
    let duplicate_new = duplicate_workbook(512, Some(256));
    criterion.bench_function("align 512 duplicate rows with one insertion", |bencher| {
        bencher.iter(|| {
            black_box(
                diff_workbooks(
                    Path::new("old.xlsx"),
                    Path::new("new.xlsx"),
                    black_box(&duplicate_old),
                    black_box(&duplicate_new),
                )
                .expect("duplicate benchmark diff should succeed"),
            )
        })
    });

    let edited_old = workbook(256, None);
    let mut edited_new = edited_old.clone();
    for row in edited_new.sheets[0].rows.iter_mut().skip(1) {
        row[2].value.push('0');
    }
    criterion.bench_function("align 256 rows with edited values", |bencher| {
        bencher.iter(|| {
            black_box(
                diff_workbooks(
                    Path::new("old.xlsx"),
                    Path::new("new.xlsx"),
                    black_box(&edited_old),
                    black_box(&edited_new),
                )
                .expect("edited benchmark diff should succeed"),
            )
        })
    });

    let (wide_old, wide_new) = wide_sparse_workbooks();
    criterion.bench_function("align Excel-max-width sparse column insertion", |bencher| {
        bencher.iter(|| {
            black_box(
                diff_workbooks(
                    Path::new("old.xlsx"),
                    Path::new("new.xlsx"),
                    black_box(&wide_old),
                    black_box(&wide_new),
                )
                .expect("wide sparse benchmark diff should succeed"),
            )
        })
    });
}

fn duplicate_workbook(rows: usize, inserted_at: Option<usize>) -> WorkbookIr {
    let mut data = vec![row(&["id", "value"])];
    data.extend((0..rows).map(|_| row(&["duplicate", "same"])));
    if let Some(inserted_at) = inserted_at {
        data.insert(inserted_at + 1, row(&["inserted", "unique"]));
    }
    WorkbookIr {
        sheets: vec![SheetIr {
            name: "duplicates".to_string(),
            start_row: 1,
            start_col: 1,
            rows: data,
            merged_ranges: Vec::new(),
        }],
        default_style_signature: String::new(),
        explicit_cell_styles: Default::default(),
    }
}

fn wide_sparse_workbooks() -> (WorkbookIr, WorkbookIr) {
    let width = 16_383;
    let mut old_rows = vec![vec![CellIr::default(); width]; 2];
    old_rows[0][0] = string_cell("left-header");
    old_rows[1][0] = string_cell("left-value");
    old_rows[0][width - 1] = string_cell("right-header");
    old_rows[1][width - 1] = string_cell("right-value");
    let mut new_rows = old_rows.clone();
    new_rows[0].insert(8_191, string_cell("inserted-header"));
    new_rows[1].insert(8_191, string_cell("inserted-value"));
    let make_workbook = |rows| WorkbookIr {
        sheets: vec![SheetIr {
            name: "wide-sparse".to_string(),
            start_row: 1,
            start_col: 1,
            rows,
            merged_ranges: Vec::new(),
        }],
        default_style_signature: String::new(),
        explicit_cell_styles: Default::default(),
    };
    (make_workbook(old_rows), make_workbook(new_rows))
}

fn string_cell(value: &str) -> CellIr {
    CellIr {
        value: value.to_string(),
        value_kind: CellValueKind::String,
        formula: None,
        style_signature: String::new(),
    }
}

criterion_group!(benches, alignment_benchmark);
criterion_main!(benches);
