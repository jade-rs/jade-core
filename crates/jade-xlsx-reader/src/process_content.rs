//! Scoped expanded names for ignored elements whose content remains visible.
//! Namespace support is a consumer policy; callers must not unwrap understood
//! elements merely because a document also declares their namespace ignorable.
use crate::Result;
use quick_xml::{
    events::BytesStart,
    name::{QName, ResolveResult},
    NsReader,
};

const MC: &[u8] = b"http://schemas.openxmlformats.org/markup-compatibility/2006";

/// Source positions of transparent foreign wrappers, including inside extensions.
#[derive(Default)]
pub struct ProcessedElements {
    starts: Vec<usize>,
    ends: Vec<usize>,
    opaque: Vec<usize>,
}
impl ProcessedElements {
    pub fn processes_content_at(&self, offset: usize) -> bool {
        self.starts.binary_search(&offset).is_ok()
    }
    pub fn processes_content_end_at(&self, offset: usize) -> bool {
        self.ends.binary_search(&offset).is_ok()
    }
    pub(crate) fn opaque_starts(&self) -> impl Iterator<Item = usize> + '_ {
        self.opaque.iter().copied()
    }
}

/// Visit every understood alternative without treating ignored opaque parents
/// as transparent. Unlike the grid projection this includes worksheet metadata.
pub fn processed_elements(xml: &str) -> Result<ProcessedElements> {
    elements(xml, false)
}

/// Ownership guards also need opaque positions inside understood payload roots.
pub(crate) fn elements_with_opacity(xml: &str) -> Result<ProcessedElements> {
    elements(xml, true)
}

fn elements(xml: &str, collect_opaque: bool) -> Result<ProcessedElements> {
    use quick_xml::events::Event;
    let mut result = ProcessedElements::default();
    if !collect_opaque && !xml.contains("ProcessContent") {
        return Ok(result);
    }
    let mut reader = NsReader::from_str(xml);
    let mut scope = Scope::for_xml(xml);
    let mut frames: Vec<(bool, bool)> = Vec::new();
    loop {
        let offset = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                scope.push(&reader, e)?;
                let (ns, local) = reader.resolver().resolve_element(e.name());
                let namespace = match ns {
                    ResolveResult::Bound(ns) => ns.as_ref().to_vec(),
                    _ => Vec::new(),
                };
                let name = local.as_ref();
                let mc = namespace == MC;
                let processed = !namespace.is_empty()
                    && !mc
                    && !crate::data_validations::supported_namespace(&namespace)
                    && scope.processes(&namespace, name);
                let unsupported_choice = mc
                    && name == b"Choice"
                    && frames.last().is_some_and(|frame| frame.0)
                    && !crate::data_validations::choice_supported(&reader, e)?;
                let opaque = !namespace.is_empty()
                    && !mc
                    && !crate::data_validations::supported_namespace(&namespace)
                    && !processed;
                if opaque || unsupported_choice {
                    if collect_opaque {
                        result.opaque.push(offset);
                    }
                    if matches!(event, Event::Start(_)) {
                        reader.read_to_end(e.name())?;
                    }
                    scope.pop();
                    continue;
                }
                if processed {
                    result.starts.push(offset);
                }
                if matches!(event, Event::Empty(_)) {
                    scope.pop();
                } else {
                    frames.push((mc && name == b"AlternateContent", processed));
                }
            }
            Event::End(_) => {
                if frames.pop().is_some_and(|frame| frame.1) {
                    result.ends.push(offset);
                }
                scope.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(result)
}

pub struct Scope {
    enabled: bool,
    ignorable: Vec<String>,
    content: Vec<(String, String)>,
    frames: Vec<(usize, usize)>,
}
impl Default for Scope {
    fn default() -> Self {
        Self {
            enabled: true,
            ignorable: Vec::new(),
            content: Vec::new(),
            frames: Vec::new(),
        }
    }
}
impl Scope {
    /// For a complete XML part. An absence check for the attribute's name
    /// avoids per-element attribute scans and scope bookkeeping on ordinary parts.
    pub fn for_xml(xml: &str) -> Self {
        Self {
            enabled: xml.contains("ProcessContent"),
            ..Self::default()
        }
    }
    /// Enter before interpreting a Start/Empty event. Pop immediately for Empty,
    /// after End, or after skipping a whole subtree with read_to_end.
    pub fn push(&mut self, reader: &NsReader<&[u8]>, element: &BytesStart<'_>) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let before = (self.ignorable.len(), self.content.len());
        let mut ignore = Vec::new();
        let mut process = Vec::new();
        if element.attributes_raw().contains(&b':') {
            for attribute in element.attributes() {
                let attribute = attribute?;
                if !attribute.key.as_ref().contains(&b':') {
                    continue;
                }
                let (namespace, local) = reader.resolver().resolve_attribute(attribute.key);
                if !matches!(namespace, ResolveResult::Bound(ns) if ns.as_ref() == MC) {
                    continue;
                }
                if !matches!(local.as_ref(), b"Ignorable" | b"ProcessContent") {
                    continue;
                }
                let value = attribute.decode_and_unescape_value(reader.decoder())?;
                for token in value
                    .split([' ', '\t', '\r', '\n'])
                    .filter(|s| !s.is_empty())
                {
                    if local.as_ref() == b"Ignorable" {
                        if !token.contains(':') {
                            if let Some(namespace) = resolve(reader, token) {
                                ignore.push(namespace);
                            }
                        }
                    } else if let Some((prefix, name)) = token.split_once(':') {
                        if !prefix.is_empty() && !name.is_empty() && !name.contains(':') {
                            if let Some(namespace) = resolve(reader, prefix) {
                                process.push((namespace, name.to_string()));
                            }
                        }
                    }
                }
            }
        }
        // Resolve at the declaration, not after a descendant rebinds a prefix.
        // Unbound list entries do not nominate any expanded element name.
        self.ignorable.extend(ignore);
        self.content.extend(
            process
                .into_iter()
                .filter(|(namespace, _)| self.ignorable.contains(namespace)),
        );
        self.frames.push(before);
        Ok(())
    }
    pub fn pop(&mut self) {
        if let Some((ignore, content)) = self.frames.pop() {
            self.ignorable.truncate(ignore);
            self.content.truncate(content);
        }
    }
    /// Active ignorable namespace URIs, resolved at their declarations.
    pub fn ignorable_namespaces(&self) -> impl Iterator<Item = &str> {
        self.ignorable.iter().map(String::as_str)
    }
    /// Active nominated expanded names, including inherited declarations.
    pub fn process_content_names(&self) -> impl Iterator<Item = (&str, &str)> {
        self.content
            .iter()
            .map(|(ns, name)| (ns.as_str(), name.as_str()))
    }
    pub fn processes(&self, namespace: &[u8], local: &[u8]) -> bool {
        !self.content.is_empty()
            && self.ignorable.iter().any(|ns| ns.as_bytes() == namespace)
            && self.content.iter().any(|(ns, name)| {
                ns.as_bytes() == namespace && (name == "*" || name.as_bytes() == local)
            })
    }
}
fn resolve(reader: &NsReader<&[u8]>, prefix: &str) -> Option<String> {
    let qualified = format!("{prefix}:_");
    match reader
        .resolver()
        .resolve_element(QName(qualified.as_bytes()))
        .0
    {
        ResolveResult::Bound(ns) => Some(String::from_utf8_lossy(ns.as_ref()).into_owned()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quick_xml::events::Event;

    #[test]
    fn metadata_wrappers_keep_offsets_without_leaking_from_opaque_branches() {
        let xml = r#"<s:worksheet xmlns:s="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:u="urn:future"><s:extLst><s:ext uri="test"><u:wrap mc:Ignorable="u" mc:ProcessContent="u:*"><u:empty/><u:nested></u:nested></u:wrap><u:opaque><u:hidden mc:Ignorable="u" mc:ProcessContent="u:*"/></u:opaque><mc:AlternateContent><mc:Choice Requires="u"><u:skipped mc:Ignorable="u" mc:ProcessContent="u:*"/></mc:Choice><mc:Choice Requires="s"><u:known mc:Ignorable="u" mc:ProcessContent="u:*"/></mc:Choice><mc:Fallback><u:fallback mc:Ignorable="u" mc:ProcessContent="u:*"/></mc:Fallback></mc:AlternateContent><u:outside/></s:ext></s:extLst></s:worksheet>"#;
        let positions = processed_elements(xml).unwrap();
        for name in ["wrap", "empty", "nested", "known", "fallback"] {
            assert!(positions.processes_content_at(xml.find(&format!("<u:{name}")).unwrap()));
        }
        for name in ["opaque", "hidden", "skipped", "outside"] {
            assert!(!positions.processes_content_at(xml.find(&format!("<u:{name}")).unwrap()));
        }
        for name in ["wrap", "nested"] {
            assert!(positions.processes_content_end_at(xml.find(&format!("</u:{name}>")).unwrap()));
        }
        assert_eq!(positions.starts.len(), 5);
        assert_eq!(positions.ends.len(), 2);
    }

    #[test]
    fn expanded_names_inherit_additively_and_leave_with_their_scope() {
        let xml = r#"<root xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:u="urn:one" mc:Ignorable="u" mc:ProcessContent="u:wrap"><u:wrap/><child xmlns:u="urn:two" xmlns:v="urn:one" mc:Ignorable="u" mc:ProcessContent="u:*"><u:any/><v:wrap/></child><u:any/><u:wrap/></root>"#;
        let mut reader = NsReader::from_str(xml);
        let mut scope = Scope::for_xml(xml);
        let mut flags = Vec::new();
        loop {
            let event = reader.read_event().unwrap();
            match event {
                Event::Start(ref e) | Event::Empty(ref e) => {
                    scope.push(&reader, e).unwrap();
                    let ns = reader.resolver().resolve_element(e.name()).0;
                    if let ResolveResult::Bound(ns) = ns {
                        if ns.as_ref() != MC {
                            flags.push(scope.processes(ns.as_ref(), e.local_name().as_ref()));
                        }
                    }
                    if matches!(event, Event::Empty(_)) {
                        scope.pop();
                    }
                }
                Event::End(_) => scope.pop(),
                Event::Eof => break,
                _ => {}
            }
        }
        assert_eq!(flags, [true, true, true, false, true]);
    }

    #[test]
    fn only_bound_mc_attributes_and_ignorable_namespaces_nominate_content() {
        for attributes in [
            "fake:Ignorable=\"u\" fake:ProcessContent=\"u:wrap\"",
            "mc:ProcessContent=\"u:wrap\"",
            "mc:Ignorable=\"u\" mc:ProcessContent=\"missing:wrap u:other\"",
        ] {
            let xml = format!("<root xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" xmlns:fake=\"urn:lookalike\" xmlns:u=\"urn:future\" {attributes}/>");
            let mut reader = NsReader::from_str(&xml);
            let Event::Empty(element) = reader.read_event().unwrap() else {
                panic!()
            };
            let mut scope = Scope::for_xml(&xml);
            scope.push(&reader, &element).unwrap();
            assert!(!scope.processes(b"urn:future", b"wrap"));
            scope.pop();
            assert!(!scope.processes(b"urn:future", b"other"));
        }
    }
}
