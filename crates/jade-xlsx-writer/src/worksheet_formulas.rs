//! Formula sources owned by a worksheet feature. Namespace and parent paths
//! matter: an extension's familiar-looking text is not a formula source.
use std::collections::BTreeMap;

use jade_xlsx_reader::{
    data_validations::{validation_records, ValidationRecord, X14_NAMESPACE, XM_NAMESPACE},
    CellRange, CellRef,
};
use quick_xml::{events::Event, name::ResolveResult, NsReader};

use crate::{
    cell_shift::{children_map, head_attr, leaf_text},
    patch::{apply, Splice},
    validation_sources::map_source_level,
    xml_edit, xml_edit_ns, Result,
};

type Rewrite<'a> = dyn Fn(&str, Option<CellRef>) -> String + 'a;

fn namespace(reader: &NsReader<&[u8]>, name: quick_xml::name::QName<'_>) -> Vec<u8> {
    match reader.resolver().resolve_element(name).0 {
        ResolveResult::Bound(n) => n.as_ref().to_vec(),
        ResolveResult::Unbound => Vec::new(),
        ResolveResult::Unknown(_) => b"?unknown".to_vec(),
    }
}
fn first_range(text: &str) -> Option<CellRef> {
    text.split_ascii_whitespace()
        .next()
        .and_then(|s| CellRange::parse(s).ok())
        .map(|r| r.start)
}
fn coverage(xml: &str) -> Result<Option<CellRef>> {
    for (_, _, child) in xml_edit_ns::children_ns(xml, b"sqref", XM_NAMESPACE)? {
        if let Some(at) = first_range(&leaf_text(&child.text)?) {
            return Ok(Some(at));
        }
    }
    Ok(None)
}
fn map_ns(
    xml: &str,
    name: &[u8],
    ns: &[u8],
    mut map: impl FnMut(&str) -> Result<String>,
) -> Result<String> {
    let mut edits = Vec::new();
    for (start, end, child) in xml_edit_ns::children_ns(xml, name, ns)? {
        let moved = map(&child.text)?;
        if moved != child.text {
            edits.push(Splice {
                start,
                end,
                text: child.finish(&moved)?,
            });
        }
    }
    Ok(apply(xml, edits))
}

fn validations(xml: &str, rewrite: &Rewrite<'_>) -> Result<BTreeMap<usize, String>> {
    let mut groups: BTreeMap<usize, Vec<ValidationRecord>> = BTreeMap::new();
    for record in validation_records(xml)? {
        groups
            .entry(record.container.start)
            .or_default()
            .push(record);
    }
    let mut result = BTreeMap::new();
    for (start, records) in groups {
        let list = xml_edit_ns::fragment(xml, records[0].container.clone())?;
        let entries = xml_edit_ns::children(&list.text, b"dataValidation")?;
        let mut edits = Vec::new();
        for record in records {
            let Some((start, end, entry)) = entries.get(record.index) else {
                continue;
            };
            let at = record.rule.ranges.first().map(|r| r.start);
            let moved =
                crate::validation_sources::map_formulas(&entry.text, record.extended, &|f| {
                    rewrite(f, at)
                })?;
            if moved != entry.text {
                edits.push(Splice {
                    start: *start,
                    end: *end,
                    text: entry.finish(&moved)?,
                });
            }
        }
        if !edits.is_empty() {
            result.insert(start, list.finish(&apply(&list.text, edits))?);
        }
    }
    Ok(result)
}

fn conditional(xml: &str, main: &[u8], extended: bool, rewrite: &Rewrite<'_>) -> Result<String> {
    let at = if extended {
        coverage(xml)?
    } else {
        head_attr(xml, b"sqref")?.and_then(|s| first_range(&s))
    };
    children_map(xml, b"cfRule", |rule| {
        let source_ns = if extended { XM_NAMESPACE } else { main };
        let leaf = if extended {
            b"f".as_slice()
        } else {
            b"formula"
        };
        let mut moved = map_source_level(rule, source_ns, &[leaf], false, &|f| rewrite(f, at))?;
        for group in [b"dataBar".as_slice(), b"colorScale", b"iconSet"] {
            moved = children_map(&moved, group, |group| {
                children_map(group, b"cfvo", |bound| {
                    if head_attr(bound, b"type")?.as_deref() != Some("formula") {
                        return Ok(bound.into());
                    }
                    if extended {
                        map_source_level(bound, XM_NAMESPACE, &[b"f"], false, &|f| rewrite(f, at))
                    } else if let Some(value) = head_attr(bound, b"val")? {
                        let next = rewrite(&value, at);
                        if next == value {
                            Ok(bound.into())
                        } else {
                            xml_edit::root_attributes(bound, &[("val", Some(next))])
                        }
                    } else {
                        Ok(bound.into())
                    }
                })
            })?;
        }
        Ok(moved)
    })
}

fn sparklines(xml: &str, rewrite: &Rewrite<'_>) -> Result<String> {
    children_map(xml, b"sparklineGroup", |group| {
        let group = map_source_level(group, XM_NAMESPACE, &[b"f"], false, &|f| rewrite(f, None))?;
        children_map(&group, b"sparklines", |list| {
            children_map(list, b"sparkline", |spark| {
                let at = coverage(spark)?;
                map_source_level(spark, XM_NAMESPACE, &[b"f"], false, &|f| rewrite(f, at))
            })
        })
    })
}

/// Rewrites supported worksheet sources exactly once. General MCE around
/// worksheet feature containers is outside this visitor; source alternatives
/// inside a recognized cell/rule are handled by the shared leaf mapper.
pub(crate) fn rewrite(xml: &str, map: &Rewrite<'_>) -> Result<String> {
    let mut validations = validations(xml, map)?;
    let mut reader = NsReader::from_str(xml);
    let mut main = None;
    let mut stack: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut edits = Vec::new();
    let mut next_row = 0u32;
    let mut row = (0u32, 0u32);
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                let ns = namespace(&reader, e.name());
                let name = e.local_name().as_ref().to_vec();
                if stack.is_empty()
                    && name == b"worksheet"
                    && matches!(
                        ns.as_slice(),
                        b"" | b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                            | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
                    )
                {
                    main = Some(ns.clone());
                }
                let owned_path = |names: &[&[u8]]| {
                    stack.len() == names.len()
                        && stack.iter().zip(names).all(|((ns, name), expected)| {
                            Some(ns) == main.as_ref() && name.as_slice() == *expected
                        })
                };
                let own = Some(&ns) == main.as_ref();
                if own && name == b"row" && owned_path(&[b"worksheet", b"sheetData"]) {
                    row = (
                        head_attr(&xml[start..reader.buffer_position() as usize], b"r")?
                            .and_then(|s| s.parse::<u32>().ok())
                            .and_then(|i| i.checked_sub(1))
                            .unwrap_or(next_row),
                        0,
                    );
                    next_row = row.0.saturating_add(1);
                }
                let cell = own && name == b"c" && owned_path(&[b"worksheet", b"sheetData", b"row"]);
                let conditional =
                    own && name == b"conditionalFormatting" && owned_path(&[b"worksheet"]);
                let extension = ns == X14_NAMESPACE
                    && matches!(
                        name.as_slice(),
                        b"conditionalFormattings" | b"sparklineGroups"
                    )
                    && owned_path(&[b"worksheet", b"extLst", b"ext"]);
                let validation = matches!(name.as_slice(), b"dataValidations" | b"dataValidation");
                if cell || conditional || extension || validation {
                    let at = if cell {
                        let at = head_attr(&xml[start..reader.buffer_position() as usize], b"r")?
                            .and_then(|s| CellRef::parse(&s).ok())
                            .unwrap_or_else(|| CellRef::new(row.1, row.0));
                        row.1 = at.column.saturating_add(1);
                        Some(at)
                    } else {
                        None
                    };
                    // Snapshot this element's namespace scope before leaving
                    // it. Rescanning the worksheet for each cell is quadratic.
                    let bindings = if validation {
                        Vec::new()
                    } else {
                        xml_edit_ns::bindings(&reader)
                    };
                    if !empty {
                        reader.read_to_end(e.name())?;
                    }
                    let end = reader.buffer_position() as usize;
                    if validation {
                        if let Some(text) = validations.remove(&start) {
                            edits.push(Splice { start, end, text });
                        }
                    } else {
                        let context = xml_edit_ns::ContextXml::new(&xml[start..end], &bindings)?;
                        let moved = if cell {
                            map_source_level(
                                &context.text,
                                main.as_deref().unwrap(),
                                &[b"f"],
                                false,
                                &|f| map(f, at),
                            )?
                        } else if conditional {
                            self::conditional(&context.text, main.as_deref().unwrap(), false, map)?
                        } else if name == b"conditionalFormattings" {
                            map_ns(
                                &context.text,
                                b"conditionalFormatting",
                                X14_NAMESPACE,
                                |rule| self::conditional(rule, main.as_deref().unwrap(), true, map),
                            )?
                        } else {
                            sparklines(&context.text, map)?
                        };
                        if moved != context.text {
                            edits.push(Splice {
                                start,
                                end,
                                text: context.finish(&moved)?,
                            });
                        }
                    }
                    continue;
                }
                if !empty {
                    stack.push((ns, name));
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let moved = apply(xml, edits);
    if moved.chars().any(|c| !crate::patch::is_xml_10_character(c)) {
        return Err(crate::WriteError::Table(
            "a rewritten worksheet formula contains an invalid XML character".into(),
        ));
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;
    const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

    #[test]
    fn cell_source_alternatives_are_scoped_to_the_owned_cell_and_supported_branches() {
        let unknown = r#"<mc:Choice Requires="u"><s:f>SUM(Sales[Qty])</s:f></mc:Choice>"#;
        let foreign = r#"<u:wrapper><s:f>SUM(Sales[Qty])</s:f></u:wrapper>"#;
        let source = format!(
            r#"<s:worksheet xmlns:s="{MAIN}" xmlns:mc="{MC}" xmlns:u="urn:foreign"><s:sheetData><s:row r="3"><s:c r="I3"><mc:AlternateContent><mc:Choice Requires="s"><s:f>SUM(Sales[Qty])</s:f></mc:Choice>{unknown}<mc:Fallback><s:f>SUM(Sales[Qty])</s:f></mc:Fallback></mc:AlternateContent>{foreign}</s:c></s:row></s:sheetData>{foreign}</s:worksheet>"#
        );
        let out = rewrite(&source, &|formula, at| {
            assert_eq!(at, Some(CellRef::new(8, 2)));
            formula.replace("Sales[", "Revenue[")
        })
        .unwrap();
        assert_eq!(out.matches("<s:f>SUM(Revenue[Qty])</s:f>").count(), 2);
        assert!(out.contains(unknown));
        assert_eq!(out.matches(foreign).count(), 2);
    }

    #[test]
    fn nontext_formula_leaves_and_unaffected_cdata_keep_their_original_xml() {
        let source = format!(
            r#"<s:worksheet xmlns:s="{MAIN}" xmlns:u="urn:foreign"><s:sheetData><s:row r='3'><s:c r='I3'><s:f>Sales[Qty]<u:extra>keep</u:extra></s:f></s:c><s:c r='J3'><s:f><![CDATA[Other[Qty]]]></s:f></s:c></s:row></s:sheetData></s:worksheet>"#
        );
        let out = rewrite(&source, &|formula, _| formula.replace("Sales[", "Revenue[")).unwrap();
        assert_eq!(out, source);
        let commented = source.replace(
            "Sales[Qty]<u:extra>keep</u:extra>",
            "Sales<!-- keep -->[Qty]",
        );
        let out = rewrite(&commented, &|formula, _| {
            formula.replace("Sales[", "Revenue[")
        })
        .unwrap();
        assert!(out.contains("<s:f>Revenue[Qty]<!-- keep --></s:f>"));
    }

    #[test]
    fn invalid_replacement_text_is_rejected_before_returning_package_xml() {
        let source = format!(
            r#"<s:worksheet xmlns:s="{MAIN}"><s:sheetData><s:row><s:c><s:f>1+1</s:f></s:c></s:row></s:sheetData></s:worksheet>"#
        );
        assert!(rewrite(&source, &|_, _| "bad\u{1}text".into()).is_err());
    }
}
