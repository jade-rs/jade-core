//! A copied sheet's own parts, copied with it: its drawing — the pictures
//! in it shared with the original, its charts copied and pointed at the
//! copy's cells, as Excel's Move or Copy does — its notes (comments and
//! their VML), its printer settings, its tables, each under a name and id
//! of its own, and its pivot tables, reading the same pivot cache as the
//! original's do, a PivotChart on it showing the copy's. Objects of other
//! kinds are refused rather than copied in part.

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
                "drawing" | "comments" | "vmlDrawing" | "printerSettings" | "table" | "pivotTable"
            )
        {
            return Err(WriteError::CopySheet(
                "a sheet with objects of this kind cannot be copied yet".into(),
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
    tables: &mut TableNames,
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
        if relationship.relationship_type.ends_with("/table") {
            rename_table(package, &copied, tables)?;
        }
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
        // A PivotChart names the pivot table it shows by its sheet and file:
        // the copy's shows the copy's pivot table.
        let xml = if xml.contains("pivotSource") {
            crate::pivot_chart::copied_chart_source(&xml, source, copy)?
        } else {
            xml
        };
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
            // Pictures stay shared, and a pivot table's cache: the copy
            // reads the same one, as Excel's copy does.
            let target = if relationship.relationship_type.ends_with("/image")
                || relationship
                    .relationship_type
                    .ends_with("/pivotCacheDefinition")
            {
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

/// The table names and ids taken in the workbook, so a copied table gets a
/// name and an id of its own; and the renames made, for the copy's
/// structured references.
pub(crate) struct TableNames {
    taken: Vec<String>,
    next_id: u32,
    pub(crate) renamed: Vec<(String, String)>,
}

impl TableNames {
    pub(crate) fn of(package: &OpcPackage, book: &jade_xlsx_reader::Workbook) -> Result<Self> {
        let mut taken: Vec<String> = book
            .defined_names
            .iter()
            .map(|name| name.name.to_lowercase())
            .collect();
        for table in book.sheets.iter().flat_map(|sheet| &sheet.tables) {
            taken.push(table.name.to_lowercase());
            taken.push(table.display_name.to_lowercase());
        }
        let mut next_id = 1;
        for part in package.part_names() {
            if !part.starts_with("xl/tables/") || !part.ends_with(".xml") {
                continue;
            }
            if let Some(xml) = package.part_str(part)? {
                if let Some(id) = table_root(&xml)?.and_then(|root| root.id) {
                    next_id = next_id.max(id + 1);
                }
            }
        }
        Ok(Self {
            taken,
            next_id,
            renamed: Vec::new(),
        })
    }

    /// Append a free suffix without exceeding Excel's UTF-16 name limit.
    fn fresh(&mut self, name: &str) -> Result<String> {
        crate::defined_names::validate_name(name)?;
        let with_suffix = |suffix: &str| {
            let budget = 255 - suffix.len();
            let mut units = 0;
            let stem: String = name
                .chars()
                .take_while(|c| {
                    units += c.len_utf16();
                    units <= budget
                })
                .collect();
            format!("{stem}{suffix}")
        };
        let fresh = (2u32..=u32::MAX)
            .map(|n| {
                let suffix = n.to_string();
                let candidate = with_suffix(&suffix);
                if crate::defined_names::validate_name(&candidate).is_ok() {
                    candidate
                } else {
                    // AB is a valid name, but AB2 is a cell reference.
                    with_suffix(&format!("_{suffix}"))
                }
            })
            .find(|candidate| {
                !self.taken.contains(&candidate.to_lowercase())
                    && crate::defined_names::validate_name(candidate).is_ok()
            })
            .ok_or_else(|| WriteError::CopySheet("no free table name".into()))?;
        self.taken.push(fresh.to_lowercase());
        Ok(fresh)
    }
}

/// A table part's `<table>` start tag: where it is and its `id` and `name`.
struct TableRoot {
    start: usize,
    end: usize,
    empty: bool,
    id: Option<u32>,
    name: Option<String>,
}

fn table_root(xml: &str) -> Result<Option<TableRoot>> {
    let mut reader = quick_xml::Reader::from_str(xml);
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event()? {
            quick_xml::events::Event::Start(ref element)
            | quick_xml::events::Event::Empty(ref element)
                if crate::patch::local(element.name().as_ref()) == b"table" =>
            {
                let empty = xml[start..reader.buffer_position() as usize].ends_with("/>");
                let id = crate::patch::attribute(element, b"id")?.and_then(|id| id.parse().ok());
                let name = crate::patch::attribute(element, b"name")?;
                return Ok(Some(TableRoot {
                    start,
                    end: reader.buffer_position() as usize,
                    empty,
                    id,
                    name,
                }));
            }
            quick_xml::events::Event::Eof => return Ok(None),
            _ => {}
        }
    }
}

/// The copied table part `part` given a fresh id and name, its own
/// formulas reading it under that name.
fn rename_table(package: &mut OpcPackage, part: &str, tables: &mut TableNames) -> Result<()> {
    let xml = package
        .part_str(part)?
        .ok_or_else(|| WriteError::MissingPart(part.to_string()))?;
    let Some(TableRoot {
        start,
        end,
        empty,
        name: Some(old),
        ..
    }) = table_root(&xml)?
    else {
        return Ok(());
    };
    let new = tables.fresh(&old)?;
    let id = tables.next_id;
    tables.next_id += 1;
    let mut reader = quick_xml::Reader::from_str(&xml[start..end]);
    let head = loop {
        match reader.read_event()? {
            quick_xml::events::Event::Start(element) | quick_xml::events::Event::Empty(element) => {
                break crate::shift::rewrite(
                    &element,
                    &[
                        ("id", Some(id.to_string())),
                        ("name", Some(new.clone())),
                        ("displayName", Some(new.clone())),
                    ],
                    empty,
                )?;
            }
            quick_xml::events::Event::Eof => return Ok(()),
            _ => {}
        }
    };
    let xml = format!("{}{head}{}", &xml[..start], &xml[end..]);
    let xml = crate::sheets::rewrite_formula_elements(&xml, &|formula: &str| {
        rename_table_references(formula, &old, &new)
    })?;
    package.replace(part, xml.into_bytes());
    tables.renamed.push((old, new));
    Ok(())
}

/// `formula` with its structured references to table `old` reading `new`.
pub(crate) fn rename_table_references(formula: &str, old: &str, new: &str) -> String {
    jade_xlsx_reader::rewrite_table_references(formula, |name, selector| {
        name.filter(|name| name.eq_ignore_ascii_case(old))
            .map(|_| format!("{new}{}", selector.unwrap_or_default()))
    })
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

#[cfg(test)]
mod tests {
    use super::TableNames;

    #[test]
    fn copied_table_names_fit_the_utf16_limit_without_collisions() {
        let originals = std::iter::once("Sales".to_string())
            .chain((1..=255).map(|length| "T".repeat(length)))
            .chain((0..=127).map(|length| format!("_{}", "𐐀".repeat(length))));
        for name in originals {
            crate::defined_names::validate_name(&name).expect("valid original name");
            let mut names = TableNames {
                taken: vec![name.to_lowercase()],
                next_id: 1,
                renamed: Vec::new(),
            };
            let mut allocated = vec![name.to_lowercase()];
            for suffix in 2..=12 {
                let copied = names.fresh(&name).expect("allocate a name");
                crate::defined_names::validate_name(&copied).expect("valid copied name");
                assert!(copied.ends_with(&suffix.to_string()));
                assert!(!allocated.contains(&copied.to_lowercase()));
                allocated.push(copied.to_lowercase());
                if name == "Sales" {
                    assert_eq!(copied, format!("Sales{suffix}"));
                }
            }
        }
    }

    #[test]
    fn copied_table_names_do_not_search_through_cell_references() {
        let mut names = TableNames {
            taken: vec!["ab".to_string(), "ab_2".to_string()],
            next_id: 1,
            renamed: Vec::new(),
        };
        assert_eq!(names.fresh("AB").unwrap(), "AB_3");
        assert_eq!(names.fresh("AB").unwrap(), "AB_4");
    }

    #[test]
    fn invalid_original_table_names_fail_without_reserving_names() {
        let mut names = TableNames {
            taken: Vec::new(),
            next_id: 1,
            renamed: Vec::new(),
        };
        for name in ["", "A B", "_xlnm.bad", "1Thing"] {
            assert!(names.fresh(name).is_err());
            assert!(names.taken.is_empty());
        }
    }
}
