use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use xlsx_diff::{
    CellChange, CellChangeKind, CellIr, CellRange, CellValueKind, ChangeKind, ColumnChange,
    ReportLayout, RowChange, SheetIr, SheetStatus, WorkbookDiff, WorkbookIr, diff_workbooks,
    load_workbook, write_html_report_with_layout, write_xlsx_report_with_layout,
};

const FIXTURE_DIR: &str = "tests/fixtures/public";

#[derive(Debug, Deserialize)]
struct Manifest {
    schema_version: u32,
    retrieved_at: String,
    fixtures: Vec<Fixture>,
}

#[derive(Debug, Deserialize)]
struct Fixture {
    id: String,
    file: String,
    source_url: String,
    source_page: String,
    revision: String,
    license: String,
    license_url: String,
    license_file: Option<String>,
    attribution: String,
    sha256: String,
    bytes: u64,
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct MutationManifest {
    schema_version: u32,
    cases: Vec<MutationCase>,
}

#[derive(Debug, Deserialize)]
struct MutationCase {
    id: String,
    fixture: String,
    operations: Vec<Mutation>,
    expected_events: BTreeSet<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Mutation {
    InsertGeneratedRow {
        sheet: String,
        at: usize,
        prefix: String,
    },
    InsertGeneratedColumn {
        sheet: String,
        at: usize,
        prefix: String,
    },
    SetCellValue {
        sheet: String,
        row: usize,
        column: usize,
        value: String,
    },
    SetCellFormula {
        sheet: String,
        row: usize,
        column: usize,
        formula: String,
    },
    AddMerge {
        sheet: String,
        start_row: usize,
        start_column: usize,
        end_row: usize,
        end_column: usize,
    },
    RenameSheet {
        from: String,
        to: String,
    },
    AddGeneratedSheet {
        name: String,
        rows: usize,
        columns: usize,
        prefix: String,
    },
}

#[derive(Debug, Serialize)]
struct MutationSnapshot {
    id: String,
    expected_events: BTreeSet<String>,
    observed_events: BTreeSet<String>,
    precision: f64,
    recall: f64,
    diff: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct WorkbookProfile {
    id: String,
    bytes: u64,
    sheets: Vec<SheetProfile>,
    cells: usize,
    non_empty_cells: usize,
    formula_cells: usize,
    merged_ranges: usize,
    value_kinds: Vec<(String, usize)>,
}

#[derive(Debug, Serialize)]
struct SheetProfile {
    name: String,
    start_row: usize,
    start_col: usize,
    rows: usize,
    columns: usize,
    non_empty_cells: usize,
    formula_cells: usize,
    merged_ranges: usize,
}

#[test]
fn public_corpus_has_verified_provenance_and_stable_profiles() -> Result<()> {
    let manifest = read_manifest()?;
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.retrieved_at, "2026-07-20");
    assert!(!manifest.fixtures.is_empty());

    let mut profiles = Vec::new();
    for fixture in &manifest.fixtures {
        validate_metadata(fixture)?;
        let path = fixture_path(&fixture.file);
        let bytes = fs::read(&path)
            .with_context(|| format!("failed to read public fixture {}", path.display()))?;
        assert_eq!(
            bytes.len() as u64,
            fixture.bytes,
            "fixture `{}`",
            fixture.id
        );
        assert_eq!(
            hex_sha256(&bytes),
            fixture.sha256,
            "fixture `{}`",
            fixture.id
        );

        let workbook = load_workbook(&path)
            .with_context(|| format!("failed to load public fixture `{}`", fixture.id))?;
        profiles.push(profile(fixture, &workbook));
    }

    insta::assert_json_snapshot!("public_corpus_profiles", profiles);
    Ok(())
}

#[test]
fn public_corpus_mutations_have_exact_events_and_stable_diffs() -> Result<()> {
    let mutation_manifest = read_mutations()?;
    assert_eq!(mutation_manifest.schema_version, 1);
    assert!(!mutation_manifest.cases.is_empty());

    let mut snapshots = Vec::new();
    for case in mutation_manifest.cases {
        let path = fixture_path(&case.fixture);
        let old = load_workbook(&path)
            .with_context(|| format!("failed to load mutation input `{}`", case.fixture))?;
        let mut new = old.clone();
        for operation in &case.operations {
            apply_mutation(&mut new, operation)
                .with_context(|| format!("failed to apply mutation case `{}`", case.id))?;
        }

        let diff = diff_workbooks(
            Path::new(&format!("{}#baseline", case.fixture)),
            Path::new(&format!("{}#mutated", case.fixture)),
            &old,
            &new,
        )?;
        let observed_events = structural_events(&diff);
        let true_positives = observed_events.intersection(&case.expected_events).count();
        let precision = true_positives as f64 / observed_events.len().max(1) as f64;
        let recall = true_positives as f64 / case.expected_events.len().max(1) as f64;
        assert_eq!(
            observed_events, case.expected_events,
            "mutation case `{}`",
            case.id
        );
        assert_eq!(precision, 1.0, "precision for mutation case `{}`", case.id);
        assert_eq!(recall, 1.0, "recall for mutation case `{}`", case.id);

        snapshots.push(MutationSnapshot {
            id: case.id,
            expected_events: case.expected_events,
            observed_events,
            precision,
            recall,
            diff: serde_json::to_value(diff)?,
        });
    }

    insta::assert_json_snapshot!("public_corpus_mutation_diffs", snapshots);
    Ok(())
}

#[test]
fn public_compound_insertion_renders_side_by_side_reports() -> Result<()> {
    let mutation_manifest = read_mutations()?;
    let case = mutation_manifest
        .cases
        .into_iter()
        .find(|case| case.id == "ons-compound-row-column-insertion")
        .context("compound insertion fixture case is missing")?;
    let path = fixture_path(&case.fixture);
    let old = load_workbook(&path)?;
    let mut new = old.clone();
    for operation in &case.operations {
        apply_mutation(&mut new, operation)?;
    }
    let diff = diff_workbooks(
        Path::new(&format!("{}#baseline", case.fixture)),
        Path::new(&format!("{}#mutated", case.fixture)),
        &old,
        &new,
    )?;
    let dir = tempfile::tempdir()?;
    let html_path = dir.path().join("diff.html");
    let xlsx_path = dir.path().join("diff.xlsx");

    write_html_report_with_layout(&html_path, &diff, ReportLayout::SideBySide)?;
    write_xlsx_report_with_layout(&xlsx_path, &diff, ReportLayout::SideBySide)?;

    let html = fs::read_to_string(html_path)?;
    assert!(html.contains(">OLD<"));
    assert!(html.contains(">NEW<"));
    assert!(html.contains("P3-ONS-COLUMN-R1"));
    assert!(html.contains("P3-ONS-ROW-C1"));
    assert!(xlsx_path.is_file());
    assert!(fs::metadata(xlsx_path)?.len() > 0);
    Ok(())
}

fn read_manifest() -> Result<Manifest> {
    let path = fixture_path("manifest.json");
    let content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&content).context("invalid public fixture manifest")
}

fn read_mutations() -> Result<MutationManifest> {
    let path = fixture_path("mutations.json");
    let content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&content).context("invalid public fixture mutation manifest")
}

fn fixture_path(file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(FIXTURE_DIR)
        .join(file)
}

fn validate_metadata(fixture: &Fixture) -> Result<()> {
    anyhow::ensure!(!fixture.id.is_empty(), "fixture id is empty");
    anyhow::ensure!(fixture.file.ends_with(".xlsx"), "fixture file is not xlsx");
    anyhow::ensure!(
        fixture.source_url.starts_with("https://"),
        "source URL is not HTTPS"
    );
    anyhow::ensure!(
        fixture.source_page.starts_with("https://"),
        "source page is not HTTPS"
    );
    anyhow::ensure!(!fixture.revision.is_empty(), "fixture revision is empty");
    anyhow::ensure!(!fixture.license.is_empty(), "fixture license is empty");
    anyhow::ensure!(
        fixture.license_url.starts_with("https://"),
        "license URL is not HTTPS"
    );
    if let Some(license_file) = &fixture.license_file {
        let path = fixture_path(license_file);
        anyhow::ensure!(path.is_file(), "fixture license file is missing");
    }
    anyhow::ensure!(
        !fixture.attribution.is_empty(),
        "fixture attribution is empty"
    );
    anyhow::ensure!(
        fixture.sha256.len() == 64,
        "fixture sha256 is not 64 hex digits"
    );
    anyhow::ensure!(fixture.bytes > 0, "fixture is empty");
    anyhow::ensure!(!fixture.tags.is_empty(), "fixture tags are empty");
    Ok(())
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn profile(fixture: &Fixture, workbook: &WorkbookIr) -> WorkbookProfile {
    let sheets = workbook
        .sheets
        .iter()
        .map(|sheet| {
            let cells = sheet.rows.iter().flatten();
            let non_empty_cells = cells
                .clone()
                .filter(|cell| cell.value_kind != CellValueKind::Empty || cell.formula.is_some())
                .count();
            let formula_cells = cells.filter(|cell| cell.formula.is_some()).count();
            SheetProfile {
                name: sheet.name.clone(),
                start_row: sheet.start_row,
                start_col: sheet.start_col,
                rows: sheet.rows.len(),
                columns: sheet.rows.iter().map(Vec::len).max().unwrap_or(0),
                non_empty_cells,
                formula_cells,
                merged_ranges: sheet.merged_ranges.len(),
            }
        })
        .collect::<Vec<_>>();

    let mut value_kinds = std::collections::BTreeMap::new();
    for cell in workbook
        .sheets
        .iter()
        .flat_map(|sheet| sheet.rows.iter().flatten())
    {
        let kind = match cell.value_kind {
            CellValueKind::Empty => "empty",
            CellValueKind::String => "string",
            CellValueKind::Number => "number",
            CellValueKind::Boolean => "boolean",
            CellValueKind::Error => "error",
            CellValueKind::DateTime => "date_time",
            CellValueKind::Duration => "duration",
        };
        *value_kinds.entry(kind.to_string()).or_insert(0usize) += 1;
    }

    WorkbookProfile {
        id: fixture.id.clone(),
        bytes: fixture.bytes,
        cells: sheets.iter().map(|sheet| sheet.rows * sheet.columns).sum(),
        non_empty_cells: sheets.iter().map(|sheet| sheet.non_empty_cells).sum(),
        formula_cells: sheets.iter().map(|sheet| sheet.formula_cells).sum(),
        merged_ranges: sheets.iter().map(|sheet| sheet.merged_ranges).sum(),
        sheets,
        value_kinds: value_kinds.into_iter().collect(),
    }
}

fn apply_mutation(workbook: &mut WorkbookIr, mutation: &Mutation) -> Result<()> {
    match mutation {
        Mutation::InsertGeneratedRow { sheet, at, prefix } => {
            let sheet = find_sheet_mut(workbook, sheet)?;
            let local = absolute_row_to_insertion_index(sheet, *at)?;
            let width = sheet.rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
            let row = (0..width)
                .map(|column| string_cell(format!("{prefix}-C{}", column + sheet.start_col)))
                .collect();
            sheet.rows.insert(local, row);
        }
        Mutation::InsertGeneratedColumn { sheet, at, prefix } => {
            let sheet = find_sheet_mut(workbook, sheet)?;
            let local = absolute_column_to_insertion_index(sheet, *at)?;
            for (row_index, row) in sheet.rows.iter_mut().enumerate() {
                if row.len() < local {
                    row.resize(local, CellIr::default());
                }
                row.insert(
                    local,
                    string_cell(format!("{prefix}-R{}", row_index + sheet.start_row)),
                );
            }
        }
        Mutation::SetCellValue {
            sheet,
            row,
            column,
            value,
        } => {
            let sheet = find_sheet_mut(workbook, sheet)?;
            let local_row = absolute_row_to_existing_index(sheet, *row)?;
            let local_column = absolute_column_to_existing_index(sheet, *column)?;
            let target = sheet
                .rows
                .get_mut(local_row)
                .and_then(|row| row.get_mut(local_column))
                .context("set_cell_value target is outside the loaded worksheet")?;
            target.value = value.clone();
        }
        Mutation::SetCellFormula {
            sheet,
            row,
            column,
            formula,
        } => {
            let sheet = find_sheet_mut(workbook, sheet)?;
            let local_row = absolute_row_to_existing_index(sheet, *row)?;
            let local_column = absolute_column_to_existing_index(sheet, *column)?;
            let target = sheet
                .rows
                .get_mut(local_row)
                .and_then(|row| row.get_mut(local_column))
                .context("set_cell_formula target is outside the loaded worksheet")?;
            target.formula = Some(formula.clone());
        }
        Mutation::AddMerge {
            sheet,
            start_row,
            start_column,
            end_row,
            end_column,
        } => {
            let sheet = find_sheet_mut(workbook, sheet)?;
            sheet.merged_ranges.push(CellRange {
                start_row: *start_row,
                start_col: *start_column,
                end_row: *end_row,
                end_col: *end_column,
            });
        }
        Mutation::RenameSheet { from, to } => {
            find_sheet_mut(workbook, from)?.name = to.clone();
        }
        Mutation::AddGeneratedSheet {
            name,
            rows,
            columns,
            prefix,
        } => {
            anyhow::ensure!(
                *rows > 0 && *columns > 0,
                "generated sheet must not be empty"
            );
            anyhow::ensure!(
                workbook.sheets.iter().all(|sheet| sheet.name != *name),
                "generated sheet name already exists"
            );
            workbook.sheets.push(SheetIr {
                name: name.clone(),
                start_row: 1,
                start_col: 1,
                rows: (0..*rows)
                    .map(|row| {
                        (0..*columns)
                            .map(|column| {
                                string_cell(format!("{prefix}-R{}-C{}", row + 1, column + 1))
                            })
                            .collect()
                    })
                    .collect(),
                merged_ranges: Vec::new(),
            });
        }
    }
    Ok(())
}

fn find_sheet_mut<'a>(workbook: &'a mut WorkbookIr, name: &str) -> Result<&'a mut SheetIr> {
    workbook
        .sheets
        .iter_mut()
        .find(|sheet| sheet.name == name)
        .with_context(|| format!("worksheet `{name}` was not found"))
}

fn absolute_row_to_insertion_index(sheet: &SheetIr, row: usize) -> Result<usize> {
    anyhow::ensure!(row >= sheet.start_row, "row is before worksheet start");
    let local = row - sheet.start_row;
    anyhow::ensure!(
        local <= sheet.rows.len(),
        "row insertion is after worksheet end"
    );
    Ok(local)
}

fn absolute_column_to_insertion_index(sheet: &SheetIr, column: usize) -> Result<usize> {
    anyhow::ensure!(
        column >= sheet.start_col,
        "column is before worksheet start"
    );
    let local = column - sheet.start_col;
    let width = sheet.rows.iter().map(Vec::len).max().unwrap_or(0);
    anyhow::ensure!(local <= width, "column insertion is after worksheet end");
    Ok(local)
}

fn absolute_row_to_existing_index(sheet: &SheetIr, row: usize) -> Result<usize> {
    anyhow::ensure!(row >= sheet.start_row, "row is before worksheet start");
    Ok(row - sheet.start_row)
}

fn absolute_column_to_existing_index(sheet: &SheetIr, column: usize) -> Result<usize> {
    anyhow::ensure!(
        column >= sheet.start_col,
        "column is before worksheet start"
    );
    Ok(column - sheet.start_col)
}

fn string_cell(value: String) -> CellIr {
    CellIr {
        value,
        value_kind: CellValueKind::String,
        formula: None,
        style_signature: String::new(),
    }
}

fn structural_events(diff: &WorkbookDiff) -> BTreeSet<String> {
    let mut events = BTreeSet::new();
    for sheet in &diff.sheets {
        let name = sheet
            .new_name
            .as_deref()
            .or(sheet.old_name.as_deref())
            .unwrap_or("<unknown>");
        match sheet.status {
            SheetStatus::Added => {
                events.insert(format!("sheet:added:{name}"));
            }
            SheetStatus::Removed => {
                events.insert(format!("sheet:removed:{name}"));
            }
            SheetStatus::Renamed => {
                events.insert(format!(
                    "sheet:renamed:{}->{}",
                    sheet.old_name.as_deref().unwrap_or("<unknown>"),
                    sheet.new_name.as_deref().unwrap_or("<unknown>")
                ));
            }
            SheetStatus::Modified | SheetStatus::Reordered | SheetStatus::Unchanged => {}
        }
        append_area_events(
            &mut events,
            name,
            &sheet.row_changes,
            &sheet.column_changes,
            &sheet.cell_changes,
        );
        for region in &sheet.table_regions {
            append_area_events(
                &mut events,
                name,
                &region.row_changes,
                &region.column_changes,
                &region.cell_changes,
            );
        }
        for change in &sheet.merge_changes {
            let (kind, range) = match change.kind {
                ChangeKind::Added => ("added", change.new_range.as_ref()),
                ChangeKind::Removed => ("removed", change.old_range.as_ref()),
                ChangeKind::Moved => ("moved", change.new_range.as_ref()),
            };
            if let Some(range) = range {
                events.insert(format!(
                    "merge:{kind}:{name}:{}:{}",
                    cell_address(range.start_row, range.start_col),
                    cell_address(range.end_row, range.end_col)
                ));
            }
        }
    }
    events
}

fn append_area_events(
    events: &mut BTreeSet<String>,
    name: &str,
    row_changes: &[RowChange],
    column_changes: &[ColumnChange],
    cell_changes: &[CellChange],
) {
    for change in row_changes {
        let (kind, index) = match change.kind {
            ChangeKind::Added => ("added", change.new_index),
            ChangeKind::Removed => ("removed", change.old_index),
            ChangeKind::Moved => ("moved", change.new_index),
        };
        events.insert(format!("row:{kind}:{name}:{}", index.unwrap_or(0)));
    }
    for change in column_changes {
        let (kind, index) = match change.kind {
            ChangeKind::Added => ("added", change.new_index),
            ChangeKind::Removed => ("removed", change.old_index),
            ChangeKind::Moved => ("moved", change.new_index),
        };
        events.insert(format!("column:{kind}:{name}:{}", index.unwrap_or(0)));
    }
    for change in cell_changes {
        let kind = match change.kind {
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

fn cell_address(row: usize, column: usize) -> String {
    let mut column_number = column;
    let mut letters = String::new();
    while column_number > 0 {
        let remainder = (column_number - 1) % 26;
        letters.insert(0, (b'A' + remainder as u8) as char);
        column_number = (column_number - 1) / 26;
    }
    format!("{letters}{row}")
}
