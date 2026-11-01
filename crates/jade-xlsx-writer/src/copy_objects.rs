//! A copied sheet's own parts, copied with it: its drawing — the pictures
//! in it shared with the original, its charts copied and pointed at the
//! copy's cells, as Excel's Move or Copy does — its notes (comments and
//! their VML) and its printer settings. Tables and the like, whose names
//! and ids are the workbook's, are refused rather than copied in part.

use jade_opc::{OpcPackage, Relationship};
use jade_xlsx_reader::rename_sheet_in_formula;

use crate::notes::{ensure_override, free_part_name, relative_target};
use crate::patch::escape;
use crate::{Result, WriteError};

const RELATIONSHIPS_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

/// Whether a sheet relating to what `rels` lists can be copied: refused for
/// a table or other part the workbook numbers.
pub(crate) fn refuse_uncopyable(rels: &str) -> Result<()> {
    for relationship in jade_opc::parse_relationships(rels)?.all() {
        let kind = relationship
            .relationship_type
            .rsplit('/')
            .next()
            .unwrap_or_default();
        if !relationship.external
            && !matches!(
                kind,
                "drawing" | "comments" | "vmlDrawing" | "printerSettings"
            )
        {
            return Err(WriteError::CopySheet(
                "a sheet with tables, pivot tables or other objects of its own cannot be copied yet"
                    .into(),
            ));
        }
    }
    Ok(())
}

/// The copy's relationships part for the source sheet's `rels`: each part
/// the sheet relates to copied (`source_part` and `copy_part` name the two
/// sheets, `source` and `copy` their names), external targets kept.
pub(crate) fn copy_sheet_parts(
    package: &mut OpcPackage,
    source_part: &str,
    copy_part: &str,
    rels: &str,
    source: &str,
    copy: &str,
) -> Result<String> {
    refuse_uncopyable(rels)?;
    let mut entries = Vec::new();
    for relationship in jade_opc::parse_relationships(rels)?.all() {
        if relationship.external {
            entries.push(relationship.clone());
            continue;
        }
        let target = jade_opc::resolve_target(source_part, &relationship.target);
        let copied = duplicate(package, &target, source, copy, 0)?;
        entries.push(Relationship {
            target: relative_target(copy_part, &copied),
            ..relationship.clone()
        });
    }
    Ok(relationships_xml(&entries))
}

/// A copy of `part` under a new name like it, with a copy of everything it
/// relates to but its pictures, which stay shared; a chart's formulas that
/// read sheet `source` read sheet `copy`. Returns the copy's name.
fn duplicate(
    package: &mut OpcPackage,
    part: &str,
    source: &str,
    copy: &str,
    depth: usize,
) -> Result<String> {
    if depth > 4 {
        return Err(WriteError::CopySheet("its objects nest too deeply".into()));
    }
    let bytes = package
        .part(part)
        .ok_or_else(|| WriteError::MissingPart(part.to_string()))?
        .to_vec();
    let (stem, extension) = match part.rsplit_once('.') {
        Some((stem, extension)) => (stem, format!(".{extension}")),
        None => (part, String::new()),
    };
    let stem = stem.trim_end_matches(|c: char| c.is_ascii_digit());
    let name = free_part_name(package, stem, &extension);
    let content_type = override_type(package, part)?;
    let bytes = if content_type
        .as_deref()
        .is_some_and(|kind| kind.ends_with("drawingml.chart+xml"))
    {
        let xml = String::from_utf8(bytes).map_err(|_| WriteError::MissingPart(part.into()))?;
        crate::sheets::rewrite_formula_elements(&xml, &|formula: &str| {
            rename_sheet_in_formula(formula, source, copy)
        })?
        .into_bytes()
    } else {
        bytes
    };
    package.replace(&name, bytes);
    if let Some(content_type) = content_type {
        ensure_override(package, &name, &content_type)?;
    }
    if let Some(rels) = package.part_str(&jade_opc::rels_part_name(part))? {
        let mut entries = Vec::new();
        for relationship in jade_opc::parse_relationships(&rels)?.all() {
            if relationship.external {
                entries.push(relationship.clone());
                continue;
            }
            let target = jade_opc::resolve_target(part, &relationship.target);
            let target = if relationship.relationship_type.ends_with("/image") {
                target
            } else {
                duplicate(package, &target, source, copy, depth + 1)?
            };
            entries.push(Relationship {
                target: relative_target(&name, &target),
                ..relationship.clone()
            });
        }
        package.replace(
            &jade_opc::rels_part_name(&name),
            relationships_xml(&entries).into_bytes(),
        );
    }
    Ok(name)
}

/// The content type `[Content_Types].xml` gives `part` by name, if any.
fn override_type(package: &OpcPackage, part: &str) -> Result<Option<String>> {
    let Some(types) = package.part_str("[Content_Types].xml")? else {
        return Ok(None);
    };
    let wanted = format!("/{part}").to_ascii_lowercase();
    for element in types.split('<').skip(1) {
        if !element.starts_with("Override") {
            continue;
        }
        let value = |key: &str| -> Option<String> {
            let at = element.find(&format!("{key}=\""))? + key.len() + 2;
            let end = element[at..].find('"')?;
            Some(element[at..at + end].to_string())
        };
        if value("PartName").is_some_and(|name| name.to_ascii_lowercase() == wanted) {
            return Ok(value("ContentType"));
        }
    }
    Ok(None)
}

fn relationships_xml(entries: &[Relationship]) -> String {
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"{RELATIONSHIPS_NS}\">"
    );
    for entry in entries {
        xml.push_str(&format!(
            "<Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\"{}/>",
            escape(&entry.id),
            escape(&entry.relationship_type),
            escape(&entry.target),
            if entry.external {
                " TargetMode=\"External\""
            } else {
                ""
            }
        ));
    }
    xml.push_str("</Relationships>");
    xml
}
