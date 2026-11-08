//! Source ownership for worksheet extension and conditional-format roots.
//! Extension URIs, expanded payload names and logical worksheet paths agree.
//! Unprocessed root payloads and ignored worksheet parents remain opaque.
use crate::{data_validations, process_content, Result};
use quick_xml::{events::Event, name::ResolveResult, NsReader};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Validations,
    ConditionalFormats,
    Sparklines,
    ProtectedRanges,
}
impl Kind {
    pub fn from_uri(uri: &str) -> Option<Self> {
        let uri = uri.trim_matches([' ', '\t', '\r', '\n']);
        [
            ("{CCE6A557-97BC-4B89-ADB6-D9C93CAAB3DF}", Self::Validations),
            (
                "{78C0D931-6437-407D-A8EE-F0AAD7539E65}",
                Self::ConditionalFormats,
            ),
            ("{05C60535-1F16-4FD2-B633-F4F36F0B64E0}", Self::Sparklines),
            (
                "{FC87AEE6-9EDD-4A0A-B7FB-166176984837}",
                Self::ProtectedRanges,
            ),
        ]
        .into_iter()
        .find_map(|(expected, kind)| uri.eq_ignore_ascii_case(expected).then_some(kind))
    }
    fn name(self) -> &'static [u8] {
        match self {
            Self::Validations => b"dataValidations",
            Self::ConditionalFormats => b"conditionalFormattings",
            Self::Sparklines => b"sparklineGroups",
            Self::ProtectedRanges => b"protectedRanges",
        }
    }
}

#[derive(Default)]
pub struct Sources {
    payloads: Vec<(usize, Kind)>,
    conditionals: Vec<usize>,
    opaque: Vec<usize>,
}
impl Sources {
    /// Main-namespace containers under compatible logical worksheet parents.
    pub fn owns_conditional_at(&self, offset: usize) -> bool {
        self.conditionals.binary_search(&offset).is_ok()
    }
    pub fn owns_payload_at(&self, offset: usize, kind: Kind) -> bool {
        self.payloads
            .binary_search_by_key(&offset, |(at, _)| *at)
            .is_ok_and(|index| self.payloads[index].1 == kind)
    }
    pub fn opaque_at(&self, offset: usize) -> bool {
        self.opaque.binary_search(&offset).is_ok()
    }
}
#[derive(Clone, Copy)]
enum Role {
    Worksheet,
    List,
    Extension(Kind),
}

/// Locate supported payload roots and opaque extension subtrees. Compatibility
/// wrappers do not change logical parentage; unsupported choices remain opaque.
pub fn sources(xml: &str) -> Result<Sources> {
    let mut result = Sources::default();
    if !xml.contains("ext")
        && !xml.contains("conditionalFormatting")
        && !xml.contains("AlternateContent")
        && !xml.contains("Ignorable")
        && !xml.contains("ProcessContent")
    {
        return Ok(result);
    }
    // Formula walkers also use these guards for chart, table and workbook
    // parts. Their root namespaces are not worksheet extensions: treating an
    // entire chart as foreign would suppress ordinary series-reference edits.
    let processed = if worksheet_root(xml)? {
        process_content::elements_with_opacity(xml)?
    } else {
        process_content::processed_elements(xml)?
    };
    result.opaque.extend(processed.opaque_starts());
    let mut reader = NsReader::from_str(xml);
    let mut main: Option<Vec<u8>> = None;
    let mut roles = Vec::new();
    // Whether this physical element supplied a logical role, and MC parentage.
    let mut frames: Vec<(bool, bool)> = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                let ns = match reader.resolver().resolve_element(e.name()).0 {
                    ResolveResult::Bound(ns) => ns.as_ref().to_vec(),
                    ResolveResult::Unbound => Vec::new(),
                    _ => b"?unknown".to_vec(),
                };
                let name = e.local_name();
                let name = name.as_ref();
                let mc = ns == data_validations::MC_NAMESPACE;
                let alternate = mc && name == b"AlternateContent";
                let branch = mc
                    && matches!(name, b"Choice" | b"Fallback")
                    && frames.last().is_some_and(|frame| frame.1);
                if branch && name == b"Choice" && !data_validations::choice_supported(&reader, e)? {
                    result.opaque.push(start);
                    if !empty {
                        reader.read_to_end(e.name())?;
                    }
                    continue;
                }
                if processed.processes_content_at(start) || alternate || branch {
                    if !empty {
                        frames.push((false, alternate));
                    }
                    continue;
                }
                let own = main.as_ref() == Some(&ns);
                let role = match roles.as_slice() {
                    [] if frames.is_empty()
                        && name == b"worksheet"
                        && matches!(
                            ns.as_slice(),
                            b"" | b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                                | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
                        ) =>
                    {
                        main = Some(ns.clone());
                        Some(Role::Worksheet)
                    }
                    [Role::Worksheet] if own && name == b"extLst" => Some(Role::List),
                    [Role::Worksheet] if own && name == b"conditionalFormatting" => {
                        result.conditionals.push(start);
                        None
                    }
                    [Role::Worksheet] if !own => {
                        result.opaque.push(start);
                        None
                    }
                    [Role::Worksheet, Role::List] if own && name == b"ext" => {
                        let mut kind = None;
                        for attr in e.attributes() {
                            let attr = attr?;
                            if attr.key.as_ref() == b"uri" {
                                kind = Kind::from_uri(
                                    &attr.decode_and_unescape_value(reader.decoder())?,
                                );
                            }
                        }
                        if kind.is_none() {
                            result.opaque.push(start);
                        }
                        kind.map(Role::Extension)
                    }
                    [Role::Worksheet, Role::List, Role::Extension(kind)] => {
                        if ns == data_validations::X14_NAMESPACE && name == kind.name() {
                            result.payloads.push((start, *kind));
                        } else {
                            result.opaque.push(start);
                        }
                        None
                    }
                    _ => None,
                };
                if let Some(role) = role {
                    if !empty {
                        roles.push(role);
                        frames.push((true, false));
                    }
                } else if !empty {
                    reader.read_to_end(e.name())?;
                }
            }
            Event::End(_) => {
                if frames.pop().is_some_and(|frame| frame.0) {
                    roles.pop();
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    result.opaque.sort_unstable();
    result.opaque.dedup();
    Ok(result)
}

fn worksheet_root(xml: &str) -> Result<bool> {
    let mut reader = NsReader::from_str(xml);
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                let namespace = match reader.resolver().resolve_element(e.name()).0 {
                    ResolveResult::Unbound => Vec::new(),
                    ResolveResult::Bound(n) => n.as_ref().to_vec(),
                    _ => return Ok(false),
                };
                return Ok(e.local_name().as_ref() == b"worksheet"
                    && matches!(
                        namespace.as_slice(),
                        b"" | b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                            | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
                    ));
            }
            Event::Eof => return Ok(false),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nonworksheet_parts_are_not_opaque_worksheet_payloads() {
        for namespace in [
            "http://schemas.openxmlformats.org/drawingml/2006/chart",
            "http://purl.oclc.org/ooxml/drawingml/chart",
        ] {
            let xml = format!(
                r#"<?xml version="1.0"?><!--part--><c:chartSpace xmlns:c="{namespace}"><c:chart><c:ser><c:f>Report!A1:A2</c:f></c:ser></c:chart><c:extLst/></c:chartSpace>"#
            );
            let ownership = sources(&xml).unwrap();
            assert!(!ownership.opaque_at(xml.find("<c:chartSpace").unwrap()));
            assert!(!ownership.opaque_at(xml.find("<c:f>").unwrap()));
            assert!(ownership.opaque.is_empty());
        }
        let xml = r#"<worksheet xmlns="urn:foreign"><extLst/></worksheet>"#;
        assert!(sources(xml).unwrap().opaque.is_empty());
    }
    #[test]
    fn compatibility_and_ignored_parents_are_opaque_without_extension_features() {
        let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:m="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:u="urn:future"><m:AlternateContent><m:Choice Requires="u"><mergeCells><mergeCell ref="A1:B1"/></mergeCells></m:Choice><m:Fallback/></m:AlternateContent></worksheet>"#;
        assert!(sources(xml)
            .unwrap()
            .opaque_at(xml.find("<m:Choice").unwrap()));
        let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:m="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:u="urn:future" m:Ignorable="u" m:ProcessContent="u:pass"><u:payload><mergeCells><mergeCell ref="A1:B1"/></mergeCells></u:payload><u:pass><mergeCells><mergeCell ref="C1:D1"/></mergeCells></u:pass></worksheet>"#;
        let ownership = sources(xml).unwrap();
        assert!(ownership.opaque_at(xml.find("<u:payload").unwrap()));
        assert!(!ownership.opaque_at(xml.find("<u:pass").unwrap()));
        let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:m="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:u="urn:future"><extLst><ext uri="{05C60535-1F16-4FD2-B633-F4F36F0B64E0}"><x:sparklineGroups><m:AlternateContent><m:Choice Requires="u"><x:sparklineGroup/></m:Choice><m:Fallback/></m:AlternateContent><x:sparklineGroup/></x:sparklineGroups></ext></extLst></worksheet>"#;
        let ownership = sources(xml).unwrap();
        assert!(
            ownership.owns_payload_at(xml.find("<x:sparklineGroups>").unwrap(), Kind::Sparklines)
        );
        assert!(ownership.opaque_at(xml.find("<m:Choice").unwrap()));
    }
    #[test]
    fn conditional_roots_require_compatible_logical_worksheet_parents() {
        let xml = r#"<s:worksheet xmlns:s="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:u="urn:future" mc:Ignorable="u" mc:ProcessContent="u:wrap"><u:wrap><s:conditionalFormatting id="processed"/></u:wrap><u:opaque><s:conditionalFormatting id="hidden"/></u:opaque><mc:AlternateContent><mc:Choice Requires="u"><s:conditionalFormatting id="unknown"/></mc:Choice><mc:Choice Requires="s"><s:conditionalFormatting id="choice"/></mc:Choice><mc:Fallback><s:conditionalFormatting id="fallback"/></mc:Fallback></mc:AlternateContent></s:worksheet>"#;
        let sources = sources(xml).unwrap();
        for id in ["processed", "choice", "fallback"] {
            assert!(sources.owns_conditional_at(
                xml.find(&format!("<s:conditionalFormatting id=\"{id}\""))
                    .unwrap()
            ));
        }
        for id in ["hidden", "unknown"] {
            assert!(!sources.owns_conditional_at(
                xml.find(&format!("<s:conditionalFormatting id=\"{id}\""))
                    .unwrap()
            ));
        }
        assert_eq!(sources.conditionals.len(), 3);
        assert!(sources.opaque_at(xml.find("<u:opaque>").unwrap()));
        assert!(sources.opaque_at(xml.find("<mc:Choice Requires=\"u\"").unwrap()));
    }
    #[test]
    fn uri_expanded_name_and_logical_parent_jointly_own_a_payload() {
        let xml = r#"<s:worksheet xmlns:s="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:u="urn:future"><s:extLst><s:ext uri="unknown"><x:dataValidations/></s:ext><s:ext uri="{cce6a557-97bc-4b89-adb6-d9c93caab3df}"><u:wrap mc:Ignorable="u" mc:ProcessContent="u:wrap"><mc:AlternateContent><mc:Choice Requires="u"><x:dataValidations id="skip"/></mc:Choice><mc:Choice Requires="s"><x:dataValidations id="choice"/></mc:Choice><mc:Fallback><x:dataValidations id="fallback"/></mc:Fallback></mc:AlternateContent></u:wrap><s:dataValidations/><x:mergeCells/></s:ext></s:extLst><s:ext uri="{CCE6A557-97BC-4B89-ADB6-D9C93CAAB3DF}"><x:dataValidations id="misplaced"/></s:ext></s:worksheet>"#;
        let sources = sources(xml).unwrap();
        for id in ["choice", "fallback"] {
            assert!(sources.owns_payload_at(
                xml.find(&format!("<x:dataValidations id=\"{id}\""))
                    .unwrap(),
                Kind::Validations
            ));
        }
        assert_eq!(sources.payloads.len(), 2);
        for tag in [
            "<s:ext uri=\"unknown\"",
            "<mc:Choice Requires=\"u\"",
            "<s:dataValidations",
            "<x:mergeCells",
        ] {
            assert!(sources.opaque_at(xml.find(tag).unwrap()), "{tag}");
        }
    }
}
