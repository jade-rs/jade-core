//! `cargo run -p jade-docx-convert --example convert -- in.docx out.rtf`:
//! converts between the formats by their extensions, through the flow
//! model, as the app opens and saves them.

use jade_docx_convert::{flow::FlowDocument, html, odt, rtf, text};

fn main() -> Result<(), String> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let [input, output] = arguments.as_slice() else {
        return Err("usage: convert <input> <output>".into());
    };
    let bytes = std::fs::read(input).map_err(|error| format!("{input}: {error}"))?;
    let extension = |path: &str| path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let document: FlowDocument = match extension(input).as_str() {
        "docx" | "docm" | "dotx" | "dotm" => jade_docx_convert::read_docx(&bytes)?,
        "rtf" => rtf::read_rtf(&bytes)?,
        "odt" => odt::read_odt(&bytes)?,
        "html" | "htm" => html::read_html(&bytes, &|_| None),
        "txt" => text::read_text(&bytes, None),
        other => return Err(format!("cannot read .{other}")),
    };
    let written = match extension(output).as_str() {
        "docx" => jade_docx_convert::write_docx(&document)?,
        "rtf" => rtf::write_rtf(&document).into_bytes(),
        "odt" => odt::write_odt(&document)?,
        "html" | "htm" => html::write_html(&document).into_bytes(),
        "txt" => text::write_text(&document, text::TextOptions::default()).into_bytes(),
        other => return Err(format!("cannot write .{other}")),
    };
    std::fs::write(output, written).map_err(|error| format!("{output}: {error}"))
}
