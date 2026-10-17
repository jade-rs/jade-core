//! Reading a workbook from a shell script or an agent.
//!
//! Every command writes JSON to stdout, except `text`, which writes the sheet
//! as TSV, and a write with `--output -`, which writes XLSX bytes. Errors go to
//! stderr and set a non-zero status, so a caller can tell a failure from an
//! empty result.
//!
//! ```text
//! jade-xlsx-headless-cli summary book.xlsx
//! jade-xlsx-headless-cli cell book.xlsx Sheet1 B9
//! jade-xlsx-headless-cli range book.xlsx Sheet1 A1:D10
//! jade-xlsx-headless-cli text book.xlsx Sheet1
//! jade-xlsx-headless-cli find book.xlsx 横浜 [--case-sensitive]
//! jade-xlsx-headless-cli set-batch book.xlsx edits.json --output out.xlsx
//! cat book.xlsx | jade-xlsx-headless-cli cell - Sheet1 B9
//! ```

use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use serde::Deserialize;
use serde_json::Value;
use std::io::{Read, Write};
use std::path::PathBuf;

const USAGE: &str = "\
usage:
  jade-xlsx-headless-cli summary <book.xlsx>
  jade-xlsx-headless-cli cell    <book.xlsx> <sheet> <A1>
  jade-xlsx-headless-cli range   <book.xlsx> <sheet> <A1:D10>
  jade-xlsx-headless-cli text    <book.xlsx> <sheet>
  jade-xlsx-headless-cli find    <book.xlsx> <needle> [--case-sensitive]
  jade-xlsx-headless-cli set     <book.xlsx> <sheet> <A1> <value> --output <out.xlsx> [--text|--formula]
  jade-xlsx-headless-cli set-batch <book.xlsx> <edits.json> --output <out.xlsx>

<sheet> is a sheet name, or its index when no sheet has that name.
<value> is read the way a spreadsheet reads what you type — a number if it looks
like one, TRUE/FALSE for a boolean, empty to clear the cell. --text keeps it as
text whatever it looks like. --formula writes an explicit formula; its leading
= is optional. Put -- before a value that itself starts with --.
<edits.json> is an array of {\"sheet\",\"reference\",\"value\"} objects; use -
to read it from standard input. Optional as_text or as_formula booleans select
an explicit string or formula write. --output - writes the XLSX package to
standard output instead of printing a JSON result. Use - for <book.xlsx> to
read the workbook from standard input; set-batch cannot read both the workbook
and edits from standard input.";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchEdit {
    sheet: String,
    reference: String,
    value: Value,
    #[serde(default)]
    as_text: bool,
    #[serde(default)]
    as_formula: bool,
}

#[derive(Debug, PartialEq)]
enum Command {
    Summary {
        input: PathBuf,
    },
    Cell {
        input: PathBuf,
        sheet: String,
        reference: String,
    },
    Range {
        input: PathBuf,
        sheet: String,
        reference: String,
    },
    Text {
        input: PathBuf,
        sheet: String,
    },
    Find {
        input: PathBuf,
        needle: String,
        case_sensitive: bool,
    },
    Set {
        input: PathBuf,
        output: PathBuf,
        sheet: String,
        reference: String,
        value: String,
        as_text: bool,
        as_formula: bool,
    },
    SetBatch {
        input: PathBuf,
        edits: PathBuf,
        output: PathBuf,
    },
}

fn parse(arguments: &[String]) -> Result<Command, String> {
    let mut positional: Vec<String> = Vec::new();
    let mut case_sensitive = false;
    let mut as_text = false;
    let mut as_formula = false;
    let mut output: Option<PathBuf> = None;
    let mut options_done = false;
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--" if !options_done => options_done = true,
            "--case-sensitive" if !options_done => case_sensitive = true,
            "--text" if !options_done => as_text = true,
            "--formula" if !options_done => as_formula = true,
            "--output" if !options_done => {
                output = Some(PathBuf::from(arguments.next().ok_or_else(|| {
                    format!("expected a path after --output\n\n{USAGE}")
                })?))
            }
            flag if !options_done && flag.starts_with("--") => {
                return Err(format!("unknown flag {flag}\n\n{USAGE}"))
            }
            other => positional.push(other.to_string()),
        }
    }

    if as_text && as_formula {
        return Err(format!(
            "--text and --formula cannot be used together\n\n{USAGE}"
        ));
    }

    let mut positional = positional.into_iter();
    let command = positional.next().ok_or_else(|| USAGE.to_string())?;
    if output.is_some() && !matches!(command.as_str(), "set" | "set-batch") {
        return Err(format!(
            "--output is only valid for set and set-batch\n\n{USAGE}"
        ));
    }
    if (as_text || as_formula) && command != "set" {
        let flag = if as_formula { "--formula" } else { "--text" };
        return Err(format!("{flag} is only valid for set\n\n{USAGE}"));
    }
    if case_sensitive && command != "find" {
        return Err(format!(
            "--case-sensitive is only valid for find\n\n{USAGE}"
        ));
    }
    let mut expect = |what: &str| {
        positional
            .next()
            .ok_or_else(|| format!("expected {what}\n\n{USAGE}"))
    };

    let parsed = match command.as_str() {
        "summary" => Command::Summary {
            input: PathBuf::from(expect("a workbook")?),
        },
        "cell" => Command::Cell {
            input: PathBuf::from(expect("a workbook")?),
            sheet: expect("a sheet")?,
            reference: expect("a cell reference")?,
        },
        "range" => Command::Range {
            input: PathBuf::from(expect("a workbook")?),
            sheet: expect("a sheet")?,
            reference: expect("a range")?,
        },
        "text" => Command::Text {
            input: PathBuf::from(expect("a workbook")?),
            sheet: expect("a sheet")?,
        },
        "find" => Command::Find {
            input: PathBuf::from(expect("a workbook")?),
            needle: expect("something to look for")?,
            case_sensitive,
        },
        "set" => Command::Set {
            input: PathBuf::from(expect("a workbook")?),
            sheet: expect("a sheet")?,
            reference: expect("a cell reference")?,
            // An empty value clears the cell, so it is a value like any other
            // and cannot be reported as missing.
            value: positional.next().unwrap_or_default(),
            output: output.ok_or_else(|| format!("expected --output\n\n{USAGE}"))?,
            as_text,
            as_formula,
        },
        "set-batch" => Command::SetBatch {
            input: PathBuf::from(expect("a workbook")?),
            edits: PathBuf::from(expect("an edits JSON file")?),
            output: output.ok_or_else(|| format!("expected --output\n\n{USAGE}"))?,
        },
        "-h" | "--help" | "help" => return Err(USAGE.to_string()),
        other => return Err(format!("unknown command {other}\n\n{USAGE}")),
    };
    if let Some(extra) = positional.next() {
        return Err(format!("unexpected argument `{extra}`\n\n{USAGE}"));
    }
    Ok(parsed)
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let command = match parse(&arguments) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    if let Err(message) = run(command) {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

fn run(command: Command) -> Result<(), String> {
    if matches!(
        &command,
        Command::SetBatch { input, edits, .. }
            if input.to_str() == Some("-") && edits.to_str() == Some("-")
    ) {
        return Err("set-batch cannot read both the workbook and edits from standard input".into());
    }
    let input = match &command {
        Command::Summary { input }
        | Command::Cell { input, .. }
        | Command::Range { input, .. }
        | Command::Text { input, .. }
        | Command::Find { input, .. }
        | Command::Set { input, .. }
        | Command::SetBatch { input, .. } => input.clone(),
    };
    let bytes = if input.to_str() == Some("-") {
        let mut bytes = Vec::new();
        std::io::stdin()
            .read_to_end(&mut bytes)
            .map_err(|error| format!("standard input: {error}"))?;
        bytes
    } else {
        std::fs::read(&input).map_err(|error| format!("{}: {error}", input.display()))?
    };
    let mut workbook = HeadlessWorkbook::open(&bytes).map_err(|error| error.to_string())?;

    match command {
        Command::Summary { .. } => print_json(&workbook.summary()),
        Command::Cell {
            sheet, reference, ..
        } => print_json(
            &workbook
                .cell(&sheet, &reference)
                .map_err(|error| error.to_string())?,
        ),
        Command::Range {
            sheet, reference, ..
        } => print_json(
            &workbook
                .range(&sheet, &reference)
                .map_err(|error| error.to_string())?,
        ),
        Command::Text { sheet, .. } => {
            let text = workbook
                .sheet_text(&sheet)
                .map_err(|error| error.to_string())?;
            for row in text.rows {
                println!("{}", tsv_row(&row));
            }
            Ok(())
        }
        Command::Find {
            needle,
            case_sensitive,
            ..
        } => print_json(&workbook.find(&needle, case_sensitive)),
        Command::Set {
            output,
            sheet,
            reference,
            value,
            as_text,
            as_formula,
            ..
        } => {
            let input = if as_formula {
                CellInput::Formula(value)
            } else if as_text {
                CellInput::Text(value)
            } else {
                CellInput::parse(&value)
            };
            workbook
                .set(&sheet, &reference, input)
                .map_err(|error| error.to_string())?;
            let exported = workbook.export_bytes().map_err(|error| error.to_string())?;
            if write_workbook(&output, &exported)? {
                return Ok(());
            }
            // The cell as it now reads, so a caller can see what landed.
            print_json(
                &workbook
                    .cell(&sheet, &reference)
                    .map_err(|error| error.to_string())?,
            )
        }
        Command::SetBatch { edits, output, .. } => {
            let json = if edits.to_str() == Some("-") {
                let mut json = String::new();
                std::io::stdin()
                    .read_to_string(&mut json)
                    .map_err(|error| format!("standard input: {error}"))?;
                json
            } else {
                std::fs::read_to_string(&edits)
                    .map_err(|error| format!("{}: {error}", edits.display()))?
            };
            let edits: Vec<BatchEdit> = serde_json::from_str(&json)
                .map_err(|error| format!("{}: {error}", edits.display()))?;
            let values = edits
                .iter()
                .enumerate()
                .map(|(index, edit)| {
                    batch_cell_input(edit, index)
                        .map(|value| (edit.sheet.clone(), edit.reference.clone(), value))
                })
                .collect::<Result<Vec<_>, _>>()?;

            workbook
                .set_workbook_cells(&values)
                .map_err(|error| error.to_string())?;
            let exported = workbook.export_bytes().map_err(|error| error.to_string())?;
            if write_workbook(&output, &exported)? {
                return Ok(());
            }
            let cells = values
                .iter()
                .map(|(sheet, reference, _)| {
                    workbook
                        .cell(sheet, reference)
                        .map_err(|error| error.to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            print_json(&cells)
        }
    }
}

/// Writes an edited workbook, returning whether stdout now contains the XLSX
/// bytes and must not be followed by a JSON result.
fn write_workbook(output: &PathBuf, bytes: &[u8]) -> Result<bool, String> {
    if output.to_str() == Some("-") {
        std::io::stdout()
            .lock()
            .write_all(bytes)
            .map_err(|error| format!("standard output: {error}"))?;
        Ok(true)
    } else {
        std::fs::write(output, bytes).map_err(|error| format!("{}: {error}", output.display()))?;
        Ok(false)
    }
}

fn batch_cell_input(edit: &BatchEdit, index: usize) -> Result<CellInput, String> {
    let number = index + 1;
    if edit.as_text && edit.as_formula {
        return Err(format!(
            "edit {number}: as_text and as_formula cannot be used together"
        ));
    }
    if edit.as_formula {
        return match &edit.value {
            Value::String(value) => Ok(CellInput::Formula(value.clone())),
            _ => Err(format!("edit {number}: a formula value must be a string")),
        };
    }
    match &edit.value {
        Value::Number(value) => Ok(CellInput::Number(value.as_f64().unwrap_or_default())),
        Value::Bool(value) => Ok(CellInput::Bool(*value)),
        Value::Null => Ok(CellInput::Blank),
        Value::String(value) if edit.as_text => Ok(CellInput::Text(value.clone())),
        Value::String(value) => Ok(CellInput::parse(value)),
        _ => Err(format!(
            "edit {number}: value must be a number, string, boolean, or null"
        )),
    }
}

fn tsv_row(cells: &[String]) -> String {
    cells
        .iter()
        .map(|cell| {
            if cell.contains(['\t', '\n', '\r', '"']) {
                format!("\"{}\"", cell.replace('"', "\"\""))
            } else {
                cell.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("\t")
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<(), String> {
    let json = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    println!("{json}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse, tsv_row, Command};
    use std::path::PathBuf;

    fn arguments(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn every_command_parses() {
        assert_eq!(
            parse(&arguments("summary book.xlsx")).expect("summary"),
            Command::Summary {
                input: PathBuf::from("book.xlsx")
            }
        );
        assert_eq!(
            parse(&arguments("cell book.xlsx Sheet1 B9")).expect("cell"),
            Command::Cell {
                input: PathBuf::from("book.xlsx"),
                sheet: "Sheet1".into(),
                reference: "B9".into(),
            }
        );
        assert_eq!(
            parse(&arguments("range book.xlsx Sheet1 A1:D10")).expect("range"),
            Command::Range {
                input: PathBuf::from("book.xlsx"),
                sheet: "Sheet1".into(),
                reference: "A1:D10".into(),
            }
        );
        assert_eq!(
            parse(&arguments("text book.xlsx Sheet1")).expect("text"),
            Command::Text {
                input: PathBuf::from("book.xlsx"),
                sheet: "Sheet1".into(),
            }
        );
    }

    #[test]
    fn a_flag_can_come_before_or_after_the_positionals() {
        let expected = Command::Find {
            input: PathBuf::from("book.xlsx"),
            needle: "yokohama".into(),
            case_sensitive: true,
        };
        assert_eq!(
            parse(&arguments("find book.xlsx yokohama --case-sensitive")).expect("after"),
            expected
        );
        assert_eq!(
            parse(&arguments("find --case-sensitive book.xlsx yokohama")).expect("before"),
            expected
        );
    }

    #[test]
    fn set_needs_somewhere_to_write() {
        assert_eq!(
            parse(&arguments("set book.xlsx Sheet1 B4 12 --output out.xlsx")).expect("set"),
            Command::Set {
                input: PathBuf::from("book.xlsx"),
                output: PathBuf::from("out.xlsx"),
                sheet: "Sheet1".into(),
                reference: "B4".into(),
                value: "12".into(),
                as_text: false,
                as_formula: false,
            }
        );
        assert!(parse(&arguments("set book.xlsx Sheet1 B4 12"))
            .expect_err("no output")
            .contains("expected --output"));
        assert!(parse(&arguments("summary book.xlsx --output"))
            .expect_err("dangling")
            .contains("expected a path"));
    }

    #[test]
    fn set_formula_is_an_explicit_mode() {
        assert_eq!(
            parse(&arguments(
                "set book.xlsx Sheet1 A1 =SUM(B1:C1) --formula --output out.xlsx"
            ))
            .expect("formula"),
            Command::Set {
                input: PathBuf::from("book.xlsx"),
                output: PathBuf::from("out.xlsx"),
                sheet: "Sheet1".into(),
                reference: "A1".into(),
                value: "=SUM(B1:C1)".into(),
                as_text: false,
                as_formula: true,
            }
        );
        assert!(parse(&arguments(
            "set book.xlsx Sheet1 A1 =1+1 --text --formula --output out.xlsx"
        ))
        .expect_err("ambiguous modes")
        .contains("cannot be used together"));
    }

    #[test]
    fn set_batch_accepts_an_edits_file_and_output() {
        assert_eq!(
            parse(&arguments(
                "set-batch book.xlsx edits.json --output out.xlsx"
            ))
            .expect("batch"),
            Command::SetBatch {
                input: PathBuf::from("book.xlsx"),
                edits: PathBuf::from("edits.json"),
                output: PathBuf::from("out.xlsx"),
            }
        );
    }

    #[test]
    fn an_empty_value_clears_the_cell_rather_than_being_missing() {
        let Command::Set { value, .. } =
            parse(&arguments("set book.xlsx Sheet1 B4 --output out.xlsx")).expect("set")
        else {
            panic!("expected a set");
        };
        assert_eq!(value, "");
    }

    #[test]
    fn a_missing_argument_says_what_was_missing() {
        let error = parse(&arguments("cell book.xlsx")).expect_err("incomplete");
        assert!(error.contains("expected a sheet"), "{error}");
    }

    #[test]
    fn an_unknown_command_or_flag_is_refused() {
        assert!(parse(&arguments("frobnicate book.xlsx"))
            .expect_err("unknown")
            .contains("unknown command"));
        assert!(parse(&arguments("summary book.xlsx --wat"))
            .expect_err("unknown")
            .contains("unknown flag"));
        assert!(parse(&[]).is_err());
    }

    #[test]
    fn flags_that_do_not_apply_to_a_command_are_refused() {
        let output = parse(&arguments("summary book.xlsx --output ignored.xlsx"))
            .expect_err("summary output must not be ignored");
        assert!(output.contains("--output is only valid"), "{output}");

        let formula = parse(&arguments("cell book.xlsx Sheet1 A1 --formula"))
            .expect_err("cell formula mode must not be ignored");
        assert!(formula.contains("--formula is only valid"), "{formula}");

        let case = parse(&arguments("text book.xlsx Sheet1 --case-sensitive"))
            .expect_err("text case mode must not be ignored");
        assert!(case.contains("--case-sensitive is only valid"), "{case}");
    }

    #[test]
    fn extra_arguments_are_refused_instead_of_silently_ignored() {
        let error =
            parse(&arguments("cell book.xlsx Sheet1 A1 accidental")).expect_err("extra argument");
        assert!(
            error.contains("unexpected argument `accidental`"),
            "{error}"
        );

        let error = parse(&arguments(
            "set book.xlsx Sheet1 A1 12 accidental --output out.xlsx",
        ))
        .expect_err("extra argument");
        assert!(
            error.contains("unexpected argument `accidental`"),
            "{error}"
        );
    }

    #[test]
    fn end_of_options_allows_a_value_that_starts_with_dashes() {
        assert_eq!(
            parse(&arguments(
                "set --output out.xlsx book.xlsx Sheet1 A1 -- --pending"
            ))
            .expect("set dashed text"),
            Command::Set {
                input: PathBuf::from("book.xlsx"),
                output: PathBuf::from("out.xlsx"),
                sheet: "Sheet1".into(),
                reference: "A1".into(),
                value: "--pending".into(),
                as_text: false,
                as_formula: false,
            }
        );
    }

    #[test]
    fn text_output_quotes_cells_that_would_break_tsv_shape() {
        let row = vec![
            "plain".to_string(),
            "a\tb".to_string(),
            "line1\nline2".to_string(),
            "say \"yes\"".to_string(),
        ];
        assert_eq!(
            tsv_row(&row),
            "plain\t\"a\tb\"\t\"line1\nline2\"\t\"say \"\"yes\"\"\""
        );
    }
}
