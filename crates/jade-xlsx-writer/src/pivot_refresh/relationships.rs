//! A pivot table part also owns its cache-definition relationship. Preserve
//! valid links and add a missing legacy link only from an unambiguous catalog.
use super::{bad, xml, Result};
use crate::{xml_edit, xml_edit_ns};
use jade_opc::OpcPackage;
use jade_xlsx_reader::Workbook;
use std::collections::BTreeSet;
fn relative(owner: &str, target: &str) -> String {
    let from = owner
        .rsplit_once('/')
        .map_or("", |(d, _)| d)
        .split('/')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    let to = target.split('/').collect::<Vec<_>>();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    format!(
        "{}{}",
        "../".repeat(from.len() - common),
        to[common..].join("/")
    )
}
pub(super) fn ensure_cache_links(
    package: &mut OpcPackage,
    book: &Workbook,
    id: u32,
    cache: &str,
) -> Result<()> {
    for table in book.pivots.tables.iter().filter(|t| t.cache_id == id) {
        let rels = jade_opc::rels_part_name(&table.part_name);
        let body=package.part_str(&rels)?.unwrap_or_else(||"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>".into());
        let mut used = BTreeSet::new();
        let mut count = 0;
        for (_, _, c) in xml_edit_ns::children(&body, b"Relationship")? {
            if let Some(id) = xml::attr(&c.text, "Id")? {
                if !used.insert(id) {
                    return Err(bad("duplicate pivot relationship identity"));
                }
            }
            if matches!(xml::attr(&c.text,"Type")?.as_deref(),Some("http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotCacheDefinition"|"http://purl.oclc.org/ooxml/officeDocument/relationships/pivotCacheDefinition")) {count+=1;}
        }
        if count == 1 {
            continue;
        }
        if count > 1 {
            return Err(bad("ambiguous pivot cache relationship"));
        }
        let id = (1u32..)
            .map(|n| format!("rIdPivotCache{n}"))
            .find(|id| !used.contains(id))
            .ok_or_else(|| bad("pivot relationship identity overflow"))?;
        let strict = xml::strict(&package.required_part_str(&table.part_name)?)?;
        let base = if strict {
            "http://purl.oclc.org/ooxml/officeDocument/relationships"
        } else {
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
        };
        let p = xml::prefix(&body)?;
        let node = xml_edit::root_attributes(
            &format!("<{p}Relationship/>"),
            &[
                ("Id", Some(id)),
                ("Type", Some(format!("{base}/pivotCacheDefinition"))),
                ("Target", Some(relative(&table.part_name, cache))),
            ],
        )?;
        package.replace(&rels, xml_edit::append_child(&body, &node)?.into_bytes());
    }
    Ok(())
}
