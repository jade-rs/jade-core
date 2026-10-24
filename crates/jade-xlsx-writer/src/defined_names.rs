//! Workbook and worksheet scoped names. Only the edited name is rewritten;
//! built-in names, extension attributes and unrelated package parts survive.

use jade_opc::OpcPackage;
use jade_xlsx_reader::xstring::{decode_xstring, encode_xstring};
use jade_xlsx_reader::{CellRef, DefinedName};
use quick_xml::{events::Event, name::QName, Reader};

use crate::patch::{apply, attribute, escape, local, namespace_prefix, Splice};
use crate::{Result, WriteError};

/// Excel names are case insensitive and cannot be cell addresses (in either
/// reference style), or the R/C selection shortcuts. `_xlnm.*` is reserved
/// for Excel's built-in names, not the Name Manager.
pub fn validate_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let first = chars.next();
    let valid_start = |c: char| c.is_alphabetic() || matches!(c, '_' | '\\');
    if name.encode_utf16().count() > 255
        || !first.is_some_and(valid_start)
        || !chars.all(|c| c.is_alphanumeric() || matches!(c, '_' | '.'))
        || name.to_ascii_lowercase().starts_with("_xlnm.")
        || matches!(name.to_ascii_uppercase().as_str(), "R" | "C")
        || CellRef::parse(name).is_ok()
        || is_r1c1(name)
    {
        return Err(WriteError::DefinedName(format!(
            "`{name}` is not a valid user-defined name"
        )));
    }
    Ok(())
}

fn is_r1c1(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    let Some(rest) = upper.strip_prefix('R') else {
        return false;
    };
    let Some((row, column)) = rest.split_once('C') else {
        return false;
    };
    // R1C, RC1 and RC are reference forms too.
    row.bytes().all(|c| c.is_ascii_digit()) && column.bytes().all(|c| c.is_ascii_digit())
}

/// Creates a name or replaces the definition in the same scope. The same
/// spelling may exist in another scope. A leading `=` is accepted and
/// removed from the stored formula, as in Excel's Name Manager.
pub fn set_defined_name(original: &[u8], definition: &DefinedName) -> Result<Vec<u8>> {
    validate_name(&definition.name)?;
    let workbook = jade_xlsx_reader::read_xlsx(original)?;
    if workbook.sheets.iter().flat_map(|s| &s.tables).any(|t| {
        t.name.to_lowercase() == definition.name.to_lowercase()
            || t.display_name.to_lowercase() == definition.name.to_lowercase()
    }) {
        return Err(WriteError::DefinedName(
            "the name is already used by a table".into(),
        ));
    }
    set_builtin_or_user_name(original, definition)
}

pub(crate) fn set_builtin_or_user_name(
    original: &[u8],
    definition: &DefinedName,
) -> Result<Vec<u8>> {
    let formula = definition
        .formula
        .strip_prefix('=')
        .unwrap_or(&definition.formula);
    if formula.trim().is_empty()
        || formula
            .chars()
            .any(|c| !crate::patch::is_xml_10_character(c))
    {
        return Err(WriteError::DefinedName(
            "a definition needs a nonempty XML formula".into(),
        ));
    }
    edit(
        original,
        &definition.name,
        definition.local_sheet_index,
        Some((formula, definition.hidden)),
    )
}

/// Removes only this scope's name. Dependent formulas are kept so a
/// recalculation reports #NAME?, and another scope's definition can take over.
pub fn delete_defined_name(original: &[u8], name: &str, scope: Option<usize>) -> Result<Vec<u8>> {
    edit(original, name, scope, None)
}

fn edit(
    original: &[u8],
    name: &str,
    scope: Option<usize>,
    value: Option<(&str, bool)>,
) -> Result<Vec<u8>> {
    let mut package = OpcPackage::open(original)?;
    let part = crate::workbook_part_name(&package)?;
    let xml = package.required_part_str(&part)?;
    let workbook = jade_xlsx_reader::workbook_part::parse_workbook(&xml)?;
    if scope.is_some_and(|index| index >= workbook.sheets.len()) {
        return Err(WriteError::DefinedName(
            "sheet scope is outside the workbook".into(),
        ));
    }
    if let Some((formula, hidden)) = value {
        if workbook.defined_names.iter().any(|n| {
            n.name.to_lowercase() == name.to_lowercase()
                && n.local_sheet_index == scope
                && n.formula == formula
                && n.hidden == hidden
        }) {
            return Ok(original.to_vec());
        }
    }
    let mut reader = Reader::from_str(&xml);
    let mut depth = 0usize;
    let mut in_names = false;
    let mut container = None;
    let mut root_prefix = String::new();
    let mut existing = None;
    let mut buf = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let qualified = element.name().as_ref().to_vec();
                if depth == 0 {
                    root_prefix = namespace_prefix(&qualified);
                }
                if depth == 1 && local(&qualified) == b"definedNames" {
                    container = Some((
                        start,
                        reader.buffer_position() as usize,
                        empty,
                        namespace_prefix(&qualified),
                    ));
                    in_names = !empty;
                } else if depth == 2 && in_names && local(&qualified) == b"definedName" {
                    let candidate =
                        decode_xstring(&attribute(element, b"name")?.unwrap_or_default());
                    let candidate_scope =
                        attribute(element, b"localSheetId")?.and_then(|s| s.parse::<usize>().ok());
                    if candidate.to_lowercase() == name.to_lowercase() && candidate_scope == scope {
                        let head = String::from_utf8_lossy(element.as_ref()).into_owned();
                        if !empty {
                            reader.read_to_end_into(QName(&qualified), &mut Vec::new())?;
                        }
                        existing =
                            Some((start, reader.buffer_position() as usize, head, qualified));
                        continue;
                    }
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(ref element) => {
                depth = depth.saturating_sub(1);
                if depth == 1 && local(element.name().as_ref()) == b"definedNames" {
                    in_names = false;
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    let patched = if let Some((start, end, head, qualified)) = existing {
        let text = if let Some((formula, hidden)) = value {
            // Keep all metadata on an existing definition, including comments,
            // function flags and extension attributes.
            let element = quick_xml::events::BytesStart::from_content(
                head.trim_end_matches('/'),
                qualified.len(),
            );
            let mut text = format!("<{}", String::from_utf8_lossy(&qualified));
            for attr in element.attributes() {
                let attr = attr?;
                if attr.key.as_ref() != b"hidden" {
                    text.push_str(&format!(
                        " {}=\"{}\"",
                        String::from_utf8_lossy(attr.key.as_ref()),
                        escape(&attr.unescape_value()?)
                    ));
                }
            }
            text.push_str(&format!(
                " hidden=\"{}\">{}</{}>",
                u8::from(hidden),
                escape(formula),
                String::from_utf8_lossy(&qualified)
            ));
            text
        } else {
            String::new()
        };
        apply(&xml, vec![Splice { start, end, text }])
    } else if let Some((formula, hidden)) = value {
        let prefix = container
            .as_ref()
            .map(|c| c.3.clone())
            .unwrap_or(root_prefix);
        let scope = scope
            .map(|index| format!(" localSheetId=\"{index}\""))
            .unwrap_or_default();
        let definition = format!(
            "<{prefix}definedName name=\"{}\"{scope} hidden=\"{}\">{}</{prefix}definedName>",
            escape(&encode_xstring(name)),
            u8::from(hidden),
            escape(formula)
        );
        if let Some((start, end, empty, _)) = container {
            if empty {
                let head = xml[start..end].trim_end_matches('>').trim_end_matches('/');
                apply(
                    &xml,
                    vec![Splice {
                        start,
                        end,
                        text: format!("{head}>{definition}</{prefix}definedNames>"),
                    }],
                )
            } else {
                apply(
                    &xml,
                    vec![Splice {
                        start: end,
                        end,
                        text: definition,
                    }],
                )
            }
        } else {
            crate::header_footer::set_child(
                &xml,
                b"definedNames",
                |p| format!("<{p}definedNames>{definition}</{p}definedNames>"),
                &[
                    b"calcPr",
                    b"oleSize",
                    b"customWorkbookViews",
                    b"pivotCaches",
                    b"smartTagPr",
                    b"smartTagTypes",
                    b"webPublishing",
                    b"fileRecoveryPr",
                    b"webPublishObjects",
                    b"extLst",
                ],
            )?
        }
    } else {
        return Ok(original.to_vec());
    };
    package.replace(
        &part,
        crate::request_full_recalculation(&patched).into_bytes(),
    );
    package.write().map_err(Into::into)
}
