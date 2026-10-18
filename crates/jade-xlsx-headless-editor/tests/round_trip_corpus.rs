//! Saving must not lose anything: every workbook fixture, opened for editing
//! and saved, reopens with every cell's value, formula and style and every
//! part of its package; and a typed value survives the save while every
//! other cell stays as it was.
//!
//! This is the path the app takes (`open_for_editing`, `set`,
//! `save_bytes`), run over the whole corpus.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use jade_opc::OpcPackage;
use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::CellValue;

fn fixtures() -> Vec<(String, Vec<u8>)> {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures");
    let mut fixtures: Vec<_> = std::fs::read_dir(&directory)
        .expect("fixtures")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "xlsx")
        })
        .filter(|path| {
            !path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("._"))
        })
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, std::fs::read(&path).expect("read fixture"))
        })
        .collect();
    fixtures.sort();
    fixtures
}

type Snapshot = BTreeMap<(String, u32, u32), (CellValue, Option<String>, Option<u32>)>;

/// Every stored cell: its value, formula and style, by sheet and position.
fn snapshot(book: &HeadlessWorkbook) -> Snapshot {
    let mut cells = BTreeMap::new();
    for sheet in &book.workbook().sheets {
        for row in &sheet.rows {
            for cell in &row.cells {
                cells.insert(
                    (
                        sheet.name.clone(),
                        cell.reference.row,
                        cell.reference.column,
                    ),
                    (cell.value.clone(), cell.formula.clone(), cell.style_index),
                );
            }
        }
    }
    cells
}

fn part_names(bytes: &[u8]) -> BTreeSet<String> {
    OpcPackage::open(bytes)
        .expect("package")
        .part_names()
        .iter()
        .cloned()
        .collect()
}

fn part(bytes: &[u8], name: &str) -> Option<Vec<u8>> {
    OpcPackage::open(bytes).ok()?.part(name).map(<[u8]>::to_vec)
}

/// `B3` for zero-based row 2, column 1.
fn a1(row: u32, column: u32) -> String {
    let mut letters = String::new();
    let mut column = column + 1;
    while column > 0 {
        let digit = (column - 1) % 26;
        letters.insert(0, (b'A' + digit as u8) as char);
        column = (column - 1) / 26;
    }
    format!("{letters}{}", row + 1)
}

fn differences(
    before: &Snapshot,
    after: &Snapshot,
    except: Option<&(String, u32, u32)>,
) -> Vec<String> {
    let keys: BTreeSet<_> = before.keys().chain(after.keys()).collect();
    keys.into_iter()
        .filter(|key| Some(*key) != except)
        .filter(|key| before.get(*key) != after.get(*key))
        .map(|key| {
            format!(
                "{}!{}: {:?} became {:?}",
                key.0,
                a1(key.1, key.2),
                before.get(key),
                after.get(key)
            )
        })
        .collect()
}

/// Checks one fixture; every problem found, described.
fn check(name: &str, original: &[u8]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut book = match HeadlessWorkbook::open_for_editing(original) {
        Ok(book) => book,
        Err(error) => return vec![format!("{name}: does not open: {error}")],
    };
    let before = snapshot(&book);

    // ---- saved as it is ---------------------------------------------------
    let saved = match book.save_bytes() {
        Ok(saved) => saved,
        Err(error) => return vec![format!("{name}: does not save: {error}")],
    };
    let missing: Vec<_> = part_names(original)
        .difference(&part_names(&saved))
        .cloned()
        .collect();
    if !missing.is_empty() {
        problems.push(format!("{name}: saving drops {missing:?}"));
    }
    for untouched in part_names(original)
        .into_iter()
        .filter(|part_name| part_name.starts_with("xl/media/"))
    {
        if part(original, &untouched) != part(&saved, &untouched) {
            problems.push(format!("{name}: saving changes {untouched}"));
        }
    }
    match HeadlessWorkbook::open(&saved) {
        Ok(reopened) => {
            for difference in differences(&before, &snapshot(&reopened), None)
                .into_iter()
                .take(3)
            {
                problems.push(format!("{name}: saving changes {difference}"));
            }
        }
        Err(error) => problems.push(format!("{name}: saved copy does not open: {error}")),
    }

    // ---- a value typed below the data, then saved ---------------------------
    let mut edited = HeadlessWorkbook::open_for_editing(original).expect("opened once already");
    let Some(sheet) = edited
        .workbook()
        .sheets
        .first()
        .map(|sheet| sheet.name.clone())
    else {
        return problems;
    };
    // The first empty cell near the top left: below the data does not exist
    // on a sheet that reaches its last row.
    let occupied = snapshot(&edited);
    let Some(typed) = (0..64u32)
        .flat_map(|row| (0..16u32).map(move |column| (row, column)))
        .find(|(row, column)| !occupied.contains_key(&(sheet.clone(), *row, *column)))
    else {
        return problems;
    };
    if let Err(error) = edited.set(
        &sheet,
        &a1(typed.0, typed.1),
        CellInput::Text("typed".into()),
    ) {
        problems.push(format!("{name}: typing fails: {error}"));
        return problems;
    }
    let expected = snapshot(&edited);
    let saved = match edited.save_bytes() {
        Ok(saved) => saved,
        Err(error) => {
            problems.push(format!("{name}: edited copy does not save: {error}"));
            return problems;
        }
    };
    match HeadlessWorkbook::open(&saved) {
        Ok(reopened) => {
            let after = snapshot(&reopened);
            let key = (sheet.clone(), typed.0, typed.1);
            if !matches!(after.get(&key), Some((CellValue::Text(text), _, _)) if text == "typed") {
                problems.push(format!(
                    "{name}: the typed value is lost on save: {:?}",
                    after.get(&key)
                ));
            }
            for difference in differences(&expected, &after, Some(&key))
                .into_iter()
                .take(3)
            {
                problems.push(format!("{name}: saving an edit changes {difference}"));
            }
        }
        Err(error) => problems.push(format!("{name}: edited copy does not open: {error}")),
    }
    problems
}

#[test]
fn every_workbook_saves_without_losing_anything() {
    let fixtures = fixtures();
    assert!(fixtures.len() >= 20, "the corpus is missing");
    let problems: Vec<String> = fixtures
        .iter()
        .flat_map(|(name, bytes)| check(name, bytes))
        .collect();
    assert!(
        problems.is_empty(),
        "{} problems across {} workbooks:\n{}",
        problems.len(),
        fixtures.len(),
        problems.join("\n")
    );
}
