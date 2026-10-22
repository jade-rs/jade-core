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

/// 比較の設定 as script passes them; a field left out takes Word's default.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
struct CompareOptionsJs {
    author: Option<String>,
    formatting: Option<bool>,
    case_changes: Option<bool>,
    white_space: Option<bool>,
    comments: Option<bool>,
    by_word: Option<bool>,
    /// 組み込み: the revised copy's changes go down to its reviewer.
    combine: bool,
}

/// 比較 / 組み込み: `original` with the differences to `revised` as tracked
/// changes, dated `date` (ISO 8601). Returns `{docx, insertions,
/// deletions, formatting}`.
#[wasm_bindgen(js_name = compareDocuments)]
pub fn compare_documents(
    original: &[u8],
    revised: &[u8],
    options: JsValue,
    date: &str,
) -> Result<JsValue, JsValue> {
    let js: CompareOptionsJs = if options.is_undefined() || options.is_null() {
        CompareOptionsJs::default()
    } else {
        serde_wasm_bindgen::from_value(options)
            .map_err(|error| JsValue::from_str(&format!("invalid compare options: {error}")))?
    };
    let mut settings =
        crate::word::compare::CompareOptions::new(js.author.as_deref().unwrap_or("jade"));
    settings.formatting = js.formatting.unwrap_or(settings.formatting);
    settings.case_changes = js.case_changes.unwrap_or(settings.case_changes);
    settings.white_space = js.white_space.unwrap_or(settings.white_space);
    settings.comments = js.comments.unwrap_or(settings.comments);
    settings.by_word = js.by_word.unwrap_or(settings.by_word);
    let (docx, summary) = if js.combine {
        crate::word::compare::combine_documents(original, revised, &settings, date)
    } else {
        crate::word::compare::compare_documents(original, revised, &settings, date)
    }
    .map_err(|error| JsValue::from_str(&error))?;
    let result = js_sys::Object::new();
    let set =
        |key: &str, value: JsValue| js_sys::Reflect::set(&result, &JsValue::from_str(key), &value);
    set("docx", js_sys::Uint8Array::from(docx.as_slice()).into())?;
    set("insertions", JsValue::from(summary.insertions as u32))?;
    set("deletions", JsValue::from(summary.deletions as u32))?;
    set("formatting", JsValue::from(summary.formatting as u32))?;
    Ok(result.into())
}

/// テンプレートとアドイン: the attached template's path and whether
/// 文書のスタイルを自動的に更新する is on, as `{template, linkStyles}`.
#[wasm_bindgen(js_name = templateInfo)]
pub fn template_info(docx: &[u8]) -> JsValue {
    let result = js_sys::Object::new();
    let template = crate::word::templates::attached_template(docx)
        .map_or(JsValue::NULL, |path| JsValue::from_str(&path));
    let _ = js_sys::Reflect::set(&result, &JsValue::from_str("template"), &template);
    let _ = js_sys::Reflect::set(
        &result,
        &JsValue::from_str("linkStyles"),
        &JsValue::from_bool(crate::word::templates::links_styles(docx)),
    );
    result.into()
}

/// 添付: `docx` attached to the template named `path`; with the template's
/// bytes and `link_styles`, its styles are taken as Word updates them.
#[wasm_bindgen(js_name = attachTemplate)]
pub fn attach_template(
    docx: &[u8],
    path: &str,
    template: Option<Vec<u8>>,
    link_styles: bool,
) -> Result<Vec<u8>, JsValue> {
    crate::word::templates::attach_template(docx, path, template.as_deref(), link_styles)
        .map_err(|error| JsValue::from_str(&error))
}

/// The template taken off: the document is attached to Normal again.
#[wasm_bindgen(js_name = detachTemplate)]
pub fn detach_template(docx: &[u8]) -> Result<Vec<u8>, JsValue> {
    crate::word::templates::detach_template(docx).map_err(|error| JsValue::from_str(&error))
}

/// A new document from a template (`.dotx` / `.dotm`): its content,
/// styles and settings, typed as a document.
#[wasm_bindgen(js_name = documentFromTemplate)]
pub fn document_from_template(template: &[u8]) -> Result<Vec<u8>, JsValue> {
    crate::word::new::document_from_template(template).map_err(|error| JsValue::from_str(&error))
}

/// The document saved as a template (`.dotx`): Word テンプレート.
#[wasm_bindgen(js_name = templateFromDocument)]
pub fn template_from_document(docx: &[u8]) -> Result<Vec<u8>, JsValue> {
    crate::word::new::template_from_document(docx).map_err(|error| JsValue::from_str(&error))
}
