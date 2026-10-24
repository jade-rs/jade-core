//! Standard worksheet hyperlinks, resolved by namespace rather than prefix.
use crate::{error::Result, xstring::decode_xstring, CellRange, Hyperlink};
use jade_opc::Relationships;
use quick_xml::{events::Event, name::ResolveResult, NsReader};

const MAIN: &[u8] = b"http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const STRICT: &[u8] = b"http://purl.oclc.org/ooxml/spreadsheetml/main";
const REL: &[u8] = b"http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const STRICT_REL: &[u8] = b"http://purl.oclc.org/ooxml/officeDocument/relationships";

pub(crate) fn parse(xml: &str, rels: &Relationships) -> Result<Vec<Hyperlink>> {
    let mut reader = NsReader::from_str(xml);
    let mut depth = 0usize;
    let mut container = false;
    let mut root_namespace = None;
    let mut links = Vec::new();
    loop {
        let event = reader.read_event()?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                let (ns, local) = reader.resolver().resolve_element(e.name());
                let namespace = match ns {
                    ResolveResult::Unbound => Some(Vec::new()),
                    ResolveResult::Bound(ns) => Some(ns.as_ref().to_vec()),
                    _ => None,
                };
                if depth == 0 && local.as_ref() == b"worksheet" {
                    root_namespace = namespace
                        .clone()
                        .filter(|ns| matches!(ns.as_slice(), b"" | MAIN | STRICT));
                }
                let main = root_namespace.is_some() && namespace == root_namespace;
                if depth == 1 && main && local.as_ref() == b"hyperlinks" {
                    container = !empty;
                }
                if depth == 2 && container && main && local.as_ref() == b"hyperlink" {
                    let mut reference = None;
                    let mut id = None;
                    let mut location = None;
                    let mut display = None;
                    let mut tooltip = None;
                    for a in e.attributes() {
                        let a = a?;
                        let (ns, local) = reader.resolver().resolve_attribute(a.key);
                        let value = a.unescape_value()?.into_owned();
                        // Older producers sometimes omit the conventional r
                        // declaration. Retain that legacy tolerance, while a
                        // prefix explicitly bound to a foreign URI stays foreign.
                        let relationship = matches!(&ns,ResolveResult::Bound(n) if n.as_ref()==REL || n.as_ref()==STRICT_REL)
                            || (a.key.as_ref() == b"r:id"
                                && matches!(&ns, ResolveResult::Unknown(_)));
                        if relationship && local.as_ref() == b"id" {
                            id = Some(value);
                        } else if matches!(ns, ResolveResult::Unbound) {
                            match local.as_ref() {
                                b"ref" => reference = Some(value),
                                b"location" => location = Some(decode_xstring(&value)),
                                b"display" => display = Some(decode_xstring(&value)),
                                b"tooltip" => tooltip = Some(decode_xstring(&value)),
                                _ => {}
                            }
                        }
                    }
                    if let Some(reference) = reference {
                        let target = id
                            .and_then(|id| rels.relationship(&id))
                            .filter(|r| r.relationship_type.ends_with("/hyperlink"))
                            .map(|r| r.target.clone());
                        links.push(Hyperlink {
                            reference: CellRange::parse(&reference)?,
                            target,
                            location,
                            display,
                            tooltip,
                        });
                    }
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 1 {
                    container = false;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(links)
}
