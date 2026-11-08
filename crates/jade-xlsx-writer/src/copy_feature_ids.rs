//! A worksheet copy gets new identities for supported feature records.
use crate::{
    cell_shift::children_map,
    cell_shift_extensions::IdPool,
    patch::{apply, Splice},
    xml_edit, xml_edit_ns, Result,
};
use jade_opc::OpcPackage;
use jade_xlsx_reader::{
    data_validations::{all_supported_validation_containers, ValidationRecord},
    worksheet_extensions::{self, Kind},
};
use quick_xml::{events::Event, name::ResolveResult, NsReader};
use std::collections::BTreeMap;
const REVISION: &[u8] = b"http://schemas.microsoft.com/office/spreadsheetml/2014/revision";
const REVISION2: &[u8] = b"http://schemas.microsoft.com/office/spreadsheetml/2015/revision2";
fn refresh_root(
    xml: &str,
    ns: &[u8],
    package: &OpcPackage,
    ids: &mut Option<IdPool>,
) -> Result<String> {
    let mut reader = NsReader::from_str(xml);
    let mut moved = xml.to_owned();
    if let Event::Start(e) | Event::Empty(e) = reader.read_event()? {
        for attr in e.attributes() {
            let attr = attr?;
            let (namespace, local) = reader.resolver().resolve_attribute(attr.key);
            if local.as_ref() == b"uid"
                && matches!(namespace,ResolveResult::Bound(n) if n.as_ref()==ns)
            {
                let key = String::from_utf8_lossy(attr.key.as_ref());
                moved = xml_edit::root_attributes(
                    &moved,
                    &[(
                        &key,
                        Some(ids.get_or_insert_with(|| IdPool::new(package)).fresh()),
                    )],
                )?;
            }
        }
    }
    Ok(moved)
}
fn validation_edits(
    xml: &str,
    package: &OpcPackage,
    ids: &mut Option<IdPool>,
) -> Result<Vec<Splice>> {
    let mut groups: BTreeMap<usize, Vec<ValidationRecord>> = BTreeMap::new();
    for record in all_supported_validation_containers(xml)? {
        groups
            .entry(record.container.start)
            .or_default()
            .push(record);
    }
    let mut edits = Vec::new();
    for records in groups.into_values() {
        let span = records[0].container.clone();
        let context = xml_edit_ns::fragment(xml, span.clone())?;
        let entries = xml_edit_ns::children(&context.text, b"dataValidation")?;
        let mut children = Vec::new();
        for record in records {
            let Some((start, end, entry)) = entries.get(record.index) else {
                continue;
            };
            let moved = refresh_root(&entry.text, REVISION, package, ids)?;
            if moved != entry.text {
                children.push(Splice {
                    start: *start,
                    end: *end,
                    text: entry.finish(&moved)?,
                });
            }
        }
        if !children.is_empty() {
            edits.push(Splice {
                start: span.start,
                end: span.end,
                text: context.finish(&apply(&context.text, children))?,
            });
        }
    }
    Ok(edits)
}
pub(crate) fn rewrite(xml: &str, package: &OpcPackage) -> Result<String> {
    if !xml.contains(":uid") || (!xml.contains("sparklineGroup") && !xml.contains("dataValidation"))
    {
        return Ok(xml.into());
    }
    let mut ids = None;
    let mut edits = if xml.contains("dataValidation") {
        validation_edits(xml, package, &mut ids)?
    } else {
        Vec::new()
    };
    if !xml.contains("sparklineGroup") {
        return Ok(apply(xml, edits));
    }
    let sources = worksheet_extensions::sources(xml)?;
    let mut reader = NsReader::from_str(xml);
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event()? {
            Event::Start(e) if sources.owns_payload_at(start, Kind::Sparklines) => {
                reader.read_to_end(e.name())?;
                let end = reader.buffer_position() as usize;
                let context = xml_edit_ns::fragment(xml, start..end)?;
                let moved = children_map(&context.text, b"sparklineGroup", |group| {
                    refresh_root(group, REVISION2, package, &mut ids)
                })?;
                if moved != context.text {
                    edits.push(Splice {
                        start,
                        end,
                        text: context.finish(&moved)?,
                    });
                }
            }
            Event::Eof => return Ok(apply(xml, edits)),
            _ => {}
        }
    }
}
