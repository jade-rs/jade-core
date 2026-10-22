//! Word's other file formats in the browser: opening `.doc`, `.rtf`,
//! `.odt`, `.html` and text files as `.docx` for the editor, and saving the
//! editor's `.docx` as one of them (ファイルの種類の変更).

use wasm_bindgen::prelude::*;

/// The `.docx` for a file in another format: `doc`, `rtf`, `odt`, `html`
/// or `txt` (any of Word's encodings, Shift_JIS included).
#[wasm_bindgen(js_name = convertToDocx)]
pub fn convert_to_docx(bytes: &[u8], format: &str) -> Result<Vec<u8>, JsValue> {
    let flow = match format {
        "doc" | "dot" => jade_doc_reader::read_doc(bytes).map_err(|error| error.to_string()),
        "rtf" => jade_docx_convert::rtf::read_rtf(bytes),
        "odt" => jade_docx_convert::odt::read_odt(bytes),
        "html" | "htm" => Ok(jade_docx_convert::html::read_html(bytes, &|_| None)),
        "txt" | "text" => Ok(jade_docx_convert::text::read_text(bytes, None)),
        _ => Err(format!("cannot open .{format} files")),
    }
    .map_err(|error| JsValue::from_str(&error))?;
    jade_docx_convert::write_docx(&flow).map_err(|error| JsValue::from_str(&error))
}

/// The editor's `.docx` saved as `doc`, `dot`, `rtf`, `odt`, `html` or
/// `txt` (UTF-8, CRLF line endings, as Word's テキストのみ writes them).
#[wasm_bindgen(js_name = convertFromDocx)]
pub fn convert_from_docx(docx: &[u8], format: &str) -> Result<Vec<u8>, JsValue> {
    let flow = jade_docx_convert::read_docx(docx).map_err(|error| JsValue::from_str(&error))?;
    let bytes = match format {
        "doc" => jade_doc_writer::write_doc(&flow),
        "dot" => jade_doc_writer::write_dot(&flow),
        "rtf" => Ok(jade_docx_convert::rtf::write_rtf(&flow).into_bytes()),
        "odt" => jade_docx_convert::odt::write_odt(&flow),
        "html" | "htm" => Ok(jade_docx_convert::html::write_html(&flow).into_bytes()),
        "txt" | "text" => Ok(jade_docx_convert::text::write_text(
            &flow,
            jade_docx_convert::text::TextOptions {
                line_ending: jade_docx_convert::text::LineEnding::CrLf,
                include_hidden: false,
            },
        )
        .into_bytes()),
        _ => Err(format!("cannot save .{format} files")),
    };
    bytes.map_err(|error| JsValue::from_str(&error))
}
