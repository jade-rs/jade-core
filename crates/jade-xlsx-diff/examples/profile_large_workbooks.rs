use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use rust_xlsxwriter::Workbook;
use xlsx_diff::{ChangeKind, diff_workbooks, load_workbook_pair};

const DEFAULT_ROWS: usize = 50_000;
const DEFAULT_COLUMNS: usize = 50;
const INSERTED_ROW_ID: &str = "INSERTED-ROW";

fn main() -> Result<()> {
    let config = Config::from_env()?;
    fs::create_dir_all(&config.output_dir).with_context(|| {
        format!(
            "failed to create fixture directory {}",
            config.output_dir.display()
        )
    })?;
    let old_path = config.output_dir.join("large-old.xlsx");
    let new_path = config.output_dir.join("large-new.xlsx");

    if !config.compare_only {
        let started = Instant::now();
        write_fixture(&old_path, config.rows, config.columns, false)?;
        write_fixture(&new_path, config.rows, config.columns, true)?;
        eprintln!(
            "generated {}x{} fixtures in {:.3}s",
            config.columns,
            config.rows,
            started.elapsed().as_secs_f64()
        );
    }
    if config.generate_only {
        return Ok(());
    }

    let started = Instant::now();
    let (old, new) = load_workbook_pair(&old_path, &new_path)?;
    let load_elapsed = started.elapsed();
    let diff_started = Instant::now();
    let diff = diff_workbooks(&old_path, &new_path, &old, &new)?;
    let diff_elapsed = diff_started.elapsed();

    let expected_changes = changed_cells(config.rows, config.columns).len();
    anyhow::ensure!(diff.summary.rows_added == 1, "expected one added row");
    anyhow::ensure!(diff.summary.rows_removed == 0, "unexpected removed row");
    anyhow::ensure!(
        diff.summary.cells_changed == expected_changes,
        "expected {expected_changes} changed cells, got {}",
        diff.summary.cells_changed
    );
    let inserted_at = config.rows / 2 + 2;
    anyhow::ensure!(
        diff.sheets[0].row_changes.iter().any(|change| {
            matches!(change.kind, ChangeKind::Added) && change.new_index == Some(inserted_at)
        }),
        "inserted row was not reported at row {inserted_at}"
    );

    println!(
        "rows={} columns={} cells_per_workbook={} load_s={:.3} diff_s={:.3} total_s={:.3} rows_added={} cells_changed={}",
        config.rows,
        config.columns,
        config.rows * config.columns,
        load_elapsed.as_secs_f64(),
        diff_elapsed.as_secs_f64(),
        started.elapsed().as_secs_f64(),
        diff.summary.rows_added,
        diff.summary.cells_changed,
    );
    Ok(())
}

#[derive(Debug)]
struct Config {
    rows: usize,
    columns: usize,
    output_dir: PathBuf,
    generate_only: bool,
    compare_only: bool,
}

impl Config {
    fn from_env() -> Result<Self> {
        let mut rows = DEFAULT_ROWS;
        let mut columns = DEFAULT_COLUMNS;
        let mut output_dir = env::temp_dir().join("xlsx-diff-large-profile");
        let mut generate_only = false;
        let mut compare_only = false;
        let mut args = env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--rows" => rows = parse_usize("--rows", args.next())?,
                "--columns" => columns = parse_usize("--columns", args.next())?,
                "--output-dir" => {
                    output_dir = PathBuf::from(args.next().context("--output-dir requires a path")?)
                }
                "--generate-only" => generate_only = true,
                "--compare-only" => compare_only = true,
                "--help" | "-h" => {
                    println!(
                        "profile_large_workbooks [--rows N] [--columns N] [--output-dir PATH] [--generate-only|--compare-only]"
                    );
                    std::process::exit(0);
                }
                _ => bail!("unknown argument: {arg}"),
            }
        }
        anyhow::ensure!(rows >= 4, "--rows must be at least 4");
        anyhow::ensure!(columns >= 2, "--columns must be at least 2");
        anyhow::ensure!(
            !(generate_only && compare_only),
            "--generate-only and --compare-only are mutually exclusive"
        );
        Ok(Self {
            rows,
            columns,
            output_dir,
            generate_only,
            compare_only,
        })
    }
}

fn parse_usize(name: &str, value: Option<String>) -> Result<usize> {
    value
        .with_context(|| format!("{name} requires a value"))?
        .parse()
        .with_context(|| format!("{name} must be a positive integer"))
}

fn write_fixture(path: &Path, rows: usize, columns: usize, changed: bool) -> Result<()> {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();

    for column in 0..columns {
        worksheet.write_string(0, column as u16, format!("column-{column:02}"))?;
    }

    let inserted_at = rows / 2;
    let changes = changed_cells(rows, columns);
    let mut output_row = 1u32;
    for source_row in 0..rows - 1 {
        if changed && source_row == inserted_at {
            write_data_row(worksheet, output_row, columns, usize::MAX, &[])?;
            output_row += 1;
        }
        write_data_row(
            worksheet,
            output_row,
            columns,
            source_row,
            if changed { &changes } else { &[] },
        )?;
        output_row += 1;
    }
    workbook
        .save(path)
        .with_context(|| format!("failed to save {}", path.display()))?;
    Ok(())
}

fn changed_cells(rows: usize, columns: usize) -> Vec<(usize, usize)> {
    vec![(0, columns - 1), (rows / 2 - 1, columns / 2), (rows - 2, 1)]
}

fn write_data_row(
    worksheet: &mut rust_xlsxwriter::Worksheet,
    output_row: u32,
    columns: usize,
    source_row: usize,
    changes: &[(usize, usize)],
) -> Result<()> {
    if source_row == usize::MAX {
        worksheet.write_string(output_row, 0, INSERTED_ROW_ID)?;
        for column in 1..columns {
            worksheet.write_number(output_row, column as u16, -1.0)?;
        }
        return Ok(());
    }

    worksheet.write_string(output_row, 0, format!("ID-{source_row:06}"))?;
    for column in 1..columns {
        let mut value = (source_row * columns + column) as f64;
        if changes.contains(&(source_row, column)) {
            value = -value - 1.0;
        }
        worksheet.write_number(output_row, column as u16, value)?;
    }
    Ok(())
}
