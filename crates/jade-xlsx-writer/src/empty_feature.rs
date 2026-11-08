//! Keep disabled source metadata without leaving an invalid empty feature.
use crate::{
    patch::{apply, Splice},
    xml_edit_ns::{self, ContextXml},
    Result,
};
use jade_xlsx_reader::data_validations::{validation_choice_supported, MC_NAMESPACE};
use quick_xml::{events::Event, name::ResolveResult, NsReader, Reader};

// This stable capability identifies inert storage, not an understood worksheet
// feature. It must remain unsupported by the reader's MCE choice selection.
const PRESERVED_NAMESPACE: &str = "urn:jade-office:preserved-ooxml";

struct Frame {
    alternate: bool,
}

/// Preserve every potentially usable alternative. An unrecognized nonempty
/// choice is not proof of emptiness: another Excel version may understand it.
fn may_have_child(xml: &str, required: &[u8]) -> Result<bool> {
    let processed = jade_xlsx_reader::process_content::processed_elements(xml)?;
    let mut reader = NsReader::from_str(xml);
    let mut frames: Vec<Frame> = Vec::new();
    let mut root_ns = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event()? {
            event @ (Event::Start(_) | Event::Empty(_)) => {
                let empty = matches!(event, Event::Empty(_));
                let e = match event {
                    Event::Start(e) | Event::Empty(e) => e,
                    _ => unreachable!(),
                };
                let ns = match reader.resolver().resolve_element(e.name()).0 {
                    ResolveResult::Bound(n) => n.as_ref().to_vec(),
                    _ => Vec::new(),
                };
                if frames.is_empty() {
                    if empty {
                        return Ok(false);
                    }
                    root_ns = ns;
                    frames.push(Frame { alternate: false });
                    continue;
                }
                let name = e.local_name();
                let name = name.as_ref();
                if ns == root_ns && name == required {
                    return Ok(true);
                }
                let alternate = ns == MC_NAMESPACE && name == b"AlternateContent";
                let branch = ns == MC_NAMESPACE
                    && matches!(name, b"Choice" | b"Fallback")
                    && frames.last().is_some_and(|f| f.alternate);
                let transparent = if branch {
                    let mut retained = false;
                    if name == b"Choice" {
                        if let Some(attr) = e.try_get_attribute("Requires")? {
                            let value = attr.unescape_value()?;
                            let prefixes = value.split_ascii_whitespace().collect::<Vec<_>>();
                            if prefixes.len() == 1 {
                                let qname = format!("{}:x", prefixes[0]);
                                retained = matches!(reader.resolver().resolve_element(quick_xml::name::QName(qname.as_bytes())).0,
                                    ResolveResult::Bound(n) if n.as_ref() == PRESERVED_NAMESPACE.as_bytes());
                            }
                        }
                    }
                    let supported = name == b"Fallback" || {
                        let context = ContextXml::new(
                            &xml[start..reader.buffer_position() as usize],
                            &xml_edit_ns::bindings(&reader),
                        )?;
                        validation_choice_supported(&context.text)?
                    };
                    if !retained && !supported && !empty {
                        return Ok(true);
                    }
                    !retained && supported
                } else {
                    alternate || processed.processes_content_at(start)
                };
                if transparent {
                    if !empty {
                        frames.push(Frame { alternate });
                    }
                } else if !empty {
                    reader.read_to_end(e.name())?;
                }
            }
            Event::End(_) => {
                frames.pop();
                if frames.is_empty() {
                    return Ok(false);
                }
            }
            Event::Eof => return Ok(false),
            _ => {}
        }
    }
}

/// Finish every split root before placing inactive metadata in a non-selected
/// alternative. Uninterpreted alternatives are kept in place conservatively.
pub(crate) fn finish(context: &ContextXml, updated: &str, required: &[u8]) -> Result<String> {
    let mut inherited = NsReader::from_str(&context.text);
    let mut scope = jade_xlsx_reader::process_content::Scope::for_xml(&context.text);
    if let Event::Start(e) | Event::Empty(e) = inherited.read_event()? {
        scope.push(&inherited, &e)?;
    }
    let bindings = xml_edit_ns::bindings(&inherited);
    let mut reader = Reader::from_str(updated);
    let mut edits = Vec::new();
    let mut next = 0usize;
    let (mc, retained) = loop {
        let mc = format!("_jade_mc{next}");
        let retained = format!("_jade_retained{next}");
        next += 1;
        if !context.text.contains(&mc)
            && !context.text.contains(&retained)
            && !updated.contains(&mc)
            && !updated.contains(&retained)
        {
            break (mc, retained);
        }
    };
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event()? {
            event @ (Event::Start(_) | Event::Empty(_)) => {
                let empty = matches!(event, Event::Empty(_));
                let e = match event {
                    Event::Start(e) | Event::Empty(e) => e,
                    _ => unreachable!(),
                };
                if !empty {
                    reader.read_to_end(e.name())?;
                }
                let end = reader.buffer_position() as usize;
                let source = &updated[start..end];
                let scoped = ContextXml::scoped(source, &bindings, &scope)?;
                let text = if may_have_child(&scoped.text, required)? {
                    context.finish(source)?
                } else if xml_edit_ns::plain_empty(&scoped.text, &[b"count"])? {
                    String::new()
                } else {
                    let source = context.finish(source)?;
                    format!("<{mc}:AlternateContent xmlns:{mc}=\"{}\" xmlns:{retained}=\"{PRESERVED_NAMESPACE}\"><{mc}:Choice Requires=\"{retained}\">{source}</{mc}:Choice><{mc}:Fallback/></{mc}:AlternateContent>", String::from_utf8_lossy(MC_NAMESPACE))
                };
                edits.push(Splice { start, end, text });
            }
            Event::Eof => return Ok(apply(updated, edits)),
            _ => {}
        }
    }
}

pub(crate) fn required(name: &[u8]) -> Option<&'static [u8]> {
    Some(match name {
        b"dataValidations" => b"dataValidation",
        b"mergeCells" => b"mergeCell",
        b"hyperlinks" => b"hyperlink",
        b"conditionalFormattings" => b"conditionalFormatting",
        b"conditionalFormatting" => b"cfRule",
        b"sparklineGroups" => b"sparklineGroup",
        b"sparklineGroup" => b"sparklines",
        b"sparklines" => b"sparkline",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn required_children_preserve_compatible_and_uninterpreted_alternatives() {
        for (body, expected) in [
            ("<s:dataValidation/>", true),
            ("<u:pass><s:dataValidation/></u:pass>", true),
            ("<u:opaque><s:dataValidation/></u:opaque>", false),
            ("<u:pass xmlns:u=\"urn:shadow\"><s:dataValidation/></u:pass>", false),
            ("<u:pass xmlns:s=\"urn:shadow\"><s:dataValidation/></u:pass>", false),
            ("<m:AlternateContent><m:Choice Requires=\"u\"><s:dataValidation/></m:Choice><m:Fallback><u:pass><s:dataValidation/></u:pass></m:Fallback></m:AlternateContent>", true),
            ("<m:AlternateContent><m:Choice Requires=\"s\"/><m:Fallback><s:dataValidation/></m:Fallback></m:AlternateContent>", true),
            ("<m:AlternateContent><m:Choice Requires=\"u\"><u:futureRule/></m:Choice><m:Fallback/></m:AlternateContent>", true),
            ("<m:AlternateContent><m:Choice Requires=\"s\"/><m:Fallback/></m:AlternateContent>", false),
            ("<m:AlternateContent xmlns:retained=\"urn:jade-office:preserved-ooxml\"><m:Choice Requires=\"retained\"><s:dataValidation/></m:Choice><m:Fallback/></m:AlternateContent>", false),
            ("<m:AlternateContent><m:Choice Requires=\"s\"><u:pass><s:dataValidation/></u:pass></m:Choice><m:Fallback/></m:AlternateContent>", true),
        ] {
            let xml = format!(r#"<s:dataValidations xmlns:s="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:m="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:u="urn:future" m:Ignorable="u" m:ProcessContent="u:pass">{body}</s:dataValidations>"#);
            assert_eq!(may_have_child(&xml, b"dataValidation").unwrap(), expected, "{xml}");
        }
    }
    #[test]
    fn archiving_restores_context_and_preserves_each_split_roots_effective_children() {
        let xml = r#"<dataValidations xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:u="urn:future" count="1" u:note="_jade_mc0"><dataValidation sqref="B2"/></dataValidations>"#;
        let context = ContextXml::new(xml, &[]).unwrap();
        let empty = xml
            .replace("count=\"1\"", "count=\"0\"")
            .replace("<dataValidation sqref=\"B2\"/>", "<!--retained-->");
        let split = format!("{empty}<!--between-->{xml}");
        let saved = finish(&context, &split, b"dataValidation").unwrap();
        assert!(saved.contains(&empty), "{saved}");
        assert!(saved.ends_with(&format!("<!--between-->{xml}")), "{saved}");
        assert!(saved.contains("urn:jade-office:preserved-ooxml"), "{saved}");
        let sources = jade_xlsx_reader::worksheet_extensions::sources(&format!(
            "<worksheet>{saved}<extLst/></worksheet>"
        ))
        .unwrap();
        assert!(sources.opaque_at("<worksheet>".len() + saved.find("<_jade_mc1:Choice").unwrap()));
    }
}
