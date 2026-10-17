use jade_docx_para_compare::build_comparison_docx_with_stats;
use std::env;
use std::path::PathBuf;

#[derive(Debug)]
struct CliArgs {
    new_path: PathBuf,
    old_path: PathBuf,
    output_path: PathBuf,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("jade-docx-para-compare failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = parse_args(env::args().skip(1))?;
    let new_docx = std::fs::read(&args.new_path).map_err(|error| {
        format!(
            "failed to read --new ({}): {error}",
            args.new_path.display()
        )
    })?;
    let old_docx = std::fs::read(&args.old_path).map_err(|error| {
        format!(
            "failed to read --old ({}): {error}",
            args.old_path.display()
        )
    })?;

    let (output, stats) = build_comparison_docx_with_stats(&new_docx, &old_docx)
        .map_err(|error| format!("failed to build comparison docx: {error}"))?;

    std::fs::write(&args.output_path, output).map_err(|error| {
        format!(
            "failed to write --output ({}): {error}",
            args.output_path.display()
        )
    })?;

    eprintln!(
        "comparison written: {} (matched={}, added={}, deleted={})",
        args.output_path.display(),
        stats.matched,
        stats.added,
        stats.deleted
    );

    Ok(())
}

fn parse_args<I>(mut args: I) -> Result<CliArgs, String>
where
    I: Iterator<Item = String>,
{
    let mut new_path = None;
    let mut old_path = None;
    let mut output_path = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--new" => new_path = args.next().map(PathBuf::from),
            "--old" => old_path = args.next().map(PathBuf::from),
            "--output" => output_path = args.next().map(PathBuf::from),
            "-h" | "--help" => return Err(usage()),
            unknown => return Err(format!("unknown argument: {unknown}\n{}", usage())),
        }
    }

    let new_path = new_path.ok_or_else(usage)?;
    let old_path = old_path.ok_or_else(usage)?;
    let output_path = output_path.ok_or_else(usage)?;

    Ok(CliArgs {
        new_path,
        old_path,
        output_path,
    })
}

fn usage() -> String {
    "Usage: jade-docx-para-compare --new <new.docx> --old <old.docx> --output <comparison.docx>"
        .to_string()
}
