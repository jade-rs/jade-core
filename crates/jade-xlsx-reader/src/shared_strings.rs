//! `xl/sharedStrings.xml` — the string pool cells with `t="s"` index into.

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::error::Result;
use jade_opc::local_name;

/// Reads the pool in file order; a cell's `<v>` is an index into the result.
///
/// A `<si>` may be one `<t>` or a sequence of formatted `<r>` runs, and the
/// cell's text is all of them joined. `<rPh>` (phonetic guides Excel writes for
/// Japanese text) also contains `<t>` but is *not* part of the value, so it is
/// skipped — joining it in would duplicate the reading into the cell.
pub fn parse_shared_strings(xml: &str) -> Result<Vec<String>> {
    let content = crate::cell_content::shared_strings(xml)?;
    let mut reader = Reader::from_str(&content);
    reader.config_mut().trim_text(false);
    let mut strings = Vec::new();
    let mut current: Option<String> = None;
    let mut in_text = false;
    let mut phonetic_depth = 0usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(ref element) => match local_name(element.name().as_ref()) {
                b"si" => current = Some(String::new()),
                b"rPh" => phonetic_depth += 1,
                b"t" if phonetic_depth == 0 => in_text = true,
                _ => {}
            },
            Event::Text(ref text) if in_text => {
                if let Some(value) = current.as_mut() {
                    value.push_str(&text.decode()?);
                }
            }
            Event::CData(ref text) if in_text => {
                if let Some(value) = current.as_mut() {
                    value.push_str(&text.decode()?);
                }
            }
            Event::GeneralRef(ref reference) if in_text => {
                if let (Some(value), Some(decoded)) =
                    (current.as_mut(), decoded_reference(reference))
                {
                    value.push_str(&decoded);
                }
            }
            Event::End(ref element) => match local_name(element.name().as_ref()) {
                b"si" => {
                    if let Some(value) = current.take() {
                        strings.push(crate::xstring::decode_xstring(&value));
                    }
                }
                b"rPh" => phonetic_depth = phonetic_depth.saturating_sub(1),
                b"t" => in_text = false,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(strings)
}

/// The text an entity reference stands for.
///
/// `quick-xml` reports `&amp;` as its own event rather than as text, so a
/// parser that only handles `Event::Text` silently drops every `&`, `<` and
/// `>` a cell contains.
fn decoded_reference(reference: &quick_xml::events::BytesRef<'_>) -> Option<String> {
    let name = reference.decode().ok()?;
    quick_xml::escape::unescape(&format!("&{name};"))
        .ok()
        .map(|text| text.into_owned())
}
