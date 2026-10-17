//! `jade-pixel-diff EXPECTED.png ACTUAL.png [options]`
//!
//! Prints a JSON report of the regions where the two renders differ (see the
//! `jade_pixel_diff` crate docs). Exit status follows diff(1): 0 when no region
//! differs, 1 when one does, 2 on a usage or I/O error.

use jade_pixel_diff::{diff_images_with_map, render_diff, DiffOptions, RgbaImage};
use std::process::ExitCode;

const USAGE: &str = "usage: jade-pixel-diff EXPECTED.png ACTUAL.png [options]

Compares two renders of the same page and prints the differing regions as JSON.

options:
  --threshold <ΔE>       CIE76 ΔE above which a pixel differs (default 10)
  --cell <px>            cell size the differing pixels are counted in (default 8)
  --min-cell-pixels <n>  differing pixels that make a cell part of a region (default 6)
  --diff-png <path>      also write a picture of the differences
  --compact              print the JSON on one line
";

struct Args {
    expected: String,
    actual: String,
    options: DiffOptions,
    diff_png: Option<String>,
    compact: bool,
}

fn parse_args(raw: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut paths = Vec::new();
    let mut options = DiffOptions::default();
    let mut diff_png = None;
    let mut compact = false;
    let mut raw = raw.into_iter();
    while let Some(arg) = raw.next() {
        let mut value = |name: &str| raw.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "-h" | "--help" => return Err(String::new()),
            "--threshold" => {
                options.delta_e_threshold = parse_number(&value("--threshold")?, "--threshold")?
            }
            "--cell" => options.cell_size = parse_number(&value("--cell")?, "--cell")?,
            "--min-cell-pixels" => {
                options.min_cell_pixels =
                    parse_number(&value("--min-cell-pixels")?, "--min-cell-pixels")?
            }
            "--diff-png" => diff_png = Some(value("--diff-png")?),
            "--compact" => compact = true,
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            path => paths.push(path.to_string()),
        }
    }
    if options.cell_size == 0 {
        return Err("--cell must be at least 1".to_string());
    }
    let [expected, actual]: [String; 2] = paths
        .try_into()
        .map_err(|_| "expected exactly two PNG paths".to_string())?;
    Ok(Args {
        expected,
        actual,
        options,
        diff_png,
        compact,
    })
}

fn parse_number<T: std::str::FromStr>(text: &str, name: &str) -> Result<T, String> {
    text.parse()
        .map_err(|_| format!("{name}: `{text}` is not a valid number"))
}

fn read_png(path: &str) -> Result<RgbaImage, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{path}: {error}"))?;
    RgbaImage::decode_png(&bytes).map_err(|error| format!("{path}: {error}"))
}

fn run(args: Args) -> Result<bool, String> {
    let expected = read_png(&args.expected)?;
    let actual = read_png(&args.actual)?;
    let (report, delta_map) = diff_images_with_map(&expected, &actual, args.options);
    if let Some(path) = &args.diff_png {
        let picture = render_diff(&actual, &report, &delta_map, args.options)
            .encode_png()
            .map_err(|error| error.to_string())?;
        std::fs::write(path, picture).map_err(|error| format!("{path}: {error}"))?;
    }
    let json = if args.compact {
        serde_json::to_string(&report)
    } else {
        serde_json::to_string_pretty(&report)
    }
    .map_err(|error| error.to_string())?;
    println!("{json}");
    Ok(report.is_identical())
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            if !message.is_empty() {
                eprintln!("jade-pixel-diff: {message}\n");
            }
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(message) => {
            eprintln!("jade-pixel-diff: {message}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Args, String> {
        parse_args(list.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn parses_paths_and_options() {
        let parsed = args(&[
            "word.png",
            "jade.png",
            "--threshold",
            "4.5",
            "--cell",
            "16",
            "--min-cell-pixels",
            "3",
            "--diff-png",
            "out.png",
            "--compact",
        ])
        .unwrap();
        assert_eq!(parsed.expected, "word.png");
        assert_eq!(parsed.actual, "jade.png");
        assert_eq!(parsed.options.delta_e_threshold, 4.5);
        assert_eq!(parsed.options.cell_size, 16);
        assert_eq!(parsed.options.min_cell_pixels, 3);
        assert_eq!(parsed.diff_png.as_deref(), Some("out.png"));
        assert!(parsed.compact);
    }

    #[test]
    fn rejects_bad_usage() {
        assert!(args(&["only.png"]).is_err());
        assert!(args(&["a.png", "b.png", "c.png"]).is_err());
        assert!(args(&["a.png", "b.png", "--cell", "0"]).is_err());
        assert!(args(&["a.png", "b.png", "--threshold"]).is_err());
        assert!(args(&["a.png", "b.png", "--threshold", "x"]).is_err());
        assert!(args(&["a.png", "b.png", "--bogus"]).is_err());
        assert_eq!(args(&["--help"]).err().as_deref(), Some(""));
    }
}
