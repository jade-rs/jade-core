//! Reading a deck from a shell script or an agent.
//!
//! Every command writes JSON to stdout, except `text`, which writes readable
//! plain text. Errors go to stderr and set a non-zero status.
//!
//! ```text
//! jade-pptx-headless-cli summary deck.pptx
//! jade-pptx-headless-cli slide   deck.pptx 2
//! jade-pptx-headless-cli text    deck.pptx
//! jade-pptx-headless-cli find    deck.pptx 横浜 [--case-sensitive]
//! jade-pptx-headless-cli layout  deck.pptx 2
//! jade-pptx-headless-cli check   deck.pptx [2]
//! jade-pptx-headless-cli media   deck.pptx ppt/media/image1.png --output image1.png
//! ```

use jade_pptx_headless_cli::{
    check_layout, deck_summary, deck_text, find_text, media, slide_layout, slide_view, Result,
};

const USAGE: &str = "\
usage:
  jade-pptx-headless-cli summary <deck.pptx>
  jade-pptx-headless-cli slide   <deck.pptx> <slide-number>
  jade-pptx-headless-cli text    <deck.pptx>
  jade-pptx-headless-cli find    <deck.pptx> <needle> [--case-sensitive]
  jade-pptx-headless-cli layout  <deck.pptx> <slide-number>
  jade-pptx-headless-cli check   <deck.pptx> [<slide-number>]
  jade-pptx-headless-cli media   <deck.pptx> <part-name> --output <file>

<slide-number> is 1-based, in presentation order.
`slide` lists a slide's shapes with their text, table cells, placements in px
and speaker notes. `layout` lists what would be drawn, master and layout shapes
included, with each shape's text lines and overflow. `check` reports text that
overflows its shape and shapes lying outside the slide; its exit status is 2
when it finds any. `media` writes a picture's bytes — the part name comes from
`slide`'s image_part.";

fn main() {
    match run() {
        Ok(status) => std::process::exit(status),
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut status = 0;
    let output = match args.first().map(String::as_str) {
        Some("summary") => {
            let bytes = read_input(&args, 1)?;
            serde_json::to_string_pretty(&deck_summary(&bytes)?)?
        }
        Some("slide") => {
            let bytes = read_input(&args, 1)?;
            let index = slide_number(&args, 2)?.ok_or(USAGE)?;
            serde_json::to_string_pretty(&slide_view(&bytes, index)?)?
        }
        Some("text") => {
            let bytes = read_input(&args, 1)?;
            deck_text(&bytes)?
        }
        Some("find") => {
            let bytes = read_input(&args, 1)?;
            let needle = args.get(2).ok_or(USAGE)?;
            let case_sensitive = args.iter().any(|arg| arg == "--case-sensitive");
            serde_json::to_string_pretty(&find_text(&bytes, needle, case_sensitive)?)?
        }
        Some("layout") => {
            let bytes = read_input(&args, 1)?;
            let index = slide_number(&args, 2)?.ok_or(USAGE)?;
            serde_json::to_string_pretty(&slide_layout(&bytes, index)?)?
        }
        Some("check") => {
            let bytes = read_input(&args, 1)?;
            let index = slide_number(&args, 2)?;
            let issues = check_layout(&bytes, index)?;
            if !issues.is_empty() {
                status = 2;
            }
            serde_json::to_string_pretty(&issues)?
        }
        Some("media") => {
            let bytes = read_input(&args, 1)?;
            let part = args.get(2).ok_or(USAGE)?;
            let output_path = args
                .iter()
                .position(|arg| arg == "--output")
                .and_then(|at| args.get(at + 1))
                .ok_or(USAGE)?;
            let (data, content_type) = media(&bytes, part)?;
            std::fs::write(output_path, &data)
                .map_err(|error| format!("cannot write {output_path}: {error}"))?;
            serde_json::to_string_pretty(&serde_json::json!({
                "part": part,
                "content_type": content_type,
                "bytes": data.len(),
                "output": output_path,
            }))?
        }
        _ => return Err(USAGE.into()),
    };
    println!("{output}");
    Ok(status)
}

fn slide_number(args: &[String], position: usize) -> Result<Option<usize>> {
    match args.get(position) {
        None => Ok(None),
        Some(raw) => raw
            .parse::<usize>()
            .ok()
            .filter(|index| *index >= 1)
            .map(Some)
            .ok_or_else(|| "slide number must be a positive integer".into()),
    }
}

fn read_input(args: &[String], position: usize) -> Result<Vec<u8>> {
    let path = args.get(position).ok_or(USAGE)?;
    Ok(std::fs::read(path).map_err(|error| format!("cannot read {path}: {error}"))?)
}
