//! A worksheet copy gets new identities for supported sparkline groups.
use crate::{
    cell_shift::children_map,
    cell_shift_extensions::IdPool,
    patch::{apply, Splice},
    xml_edit, xml_edit_ns, Result,
};
use jade_xlsx_reader::worksheet_extensions::{self, Kind};
use quick_xml::{events::Event, name::ResolveResult, NsReader};
const REVISION2: &[u8] = b"http://schemas.microsoft.com/office/spreadsheetml/2015/revision2";
pub(crate) fn rewrite(xml: &str, package: &jade_opc::OpcPackage) -> Result<String> {
    if !xml.contains("sparklineGroup") {
        return Ok(xml.into());
    }
    let mut ids = None;
    let sources = worksheet_extensions::sources(xml)?;
    let mut reader = NsReader::from_str(xml);
    let mut edits = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event()? {
            Event::Start(e) if sources.owns_payload_at(start, Kind::Sparklines) => {
                reader.read_to_end(e.name())?;
                let end = reader.buffer_position() as usize;
                let context = xml_edit_ns::fragment(xml, start..end)?;
                let moved = children_map(&context.text, b"sparklineGroup", |group| {
                    let mut reader = NsReader::from_str(group);
                    let mut moved = group.to_owned();
                    if let Event::Start(e) | Event::Empty(e) = reader.read_event()? {
                        for attr in e.attributes() {
                            let attr = attr?;
                            let (ns, local) = reader.resolver().resolve_attribute(attr.key);
                            if local.as_ref() == b"uid"
                                && matches!(ns,ResolveResult::Bound(n) if n.as_ref()==REVISION2)
                            {
                                let key = String::from_utf8_lossy(attr.key.as_ref());
                                moved = xml_edit::root_attributes(
                                    &moved,
                                    &[(
                                        &key,
                                        Some(
                                            ids.get_or_insert_with(|| IdPool::new(package)).fresh(),
                                        ),
                                    )],
                                )?;
                            }
                        }
                    }
                    Ok(moved)
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
