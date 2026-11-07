//! Resolves dynamic-array cell metadata without changing the source part.
use crate::Result;
use quick_xml::{events::Event, name::ResolveResult, NsReader};

const DYNAMIC: &str = "http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray";
const EXTENSION: &str = "{BDBB8CDC-FA1E-496E-A857-3C3F30C029C3}";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

#[derive(Default)]
struct Node {
    namespace: String,
    name: String,
    attributes: Vec<(String, String)>,
    children: Vec<usize>,
    supported_choice: bool,
}
impl Node {
    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
    fn index(&self, name: &str) -> Option<usize> {
        xml_space(self.attribute(name)?).parse().ok()
    }
}

fn xml_space(value: &str) -> &str {
    value.trim_matches([' ', '\t', '\r', '\n'])
}

fn children<'a>(
    nodes: &'a [Node],
    parent: usize,
    namespace: &'a str,
    name: &'a str,
) -> impl Iterator<Item = usize> + 'a {
    compatible_children(nodes, parent)
        .into_iter()
        .filter(move |&i| nodes[i].namespace == namespace && nodes[i].name == name)
}

fn compatible_children(nodes: &[Node], parent: usize) -> Vec<usize> {
    let mut pending: Vec<_> = nodes[parent].children.iter().rev().copied().collect();
    let mut result = Vec::new();
    while let Some(index) = pending.pop() {
        let node = &nodes[index];
        if node.namespace != MC || node.name != "AlternateContent" {
            result.push(index);
            continue;
        }
        let selected = node
            .children
            .iter()
            .copied()
            .find(|&i| {
                nodes[i].namespace == MC && nodes[i].name == "Choice" && nodes[i].supported_choice
            })
            .or_else(|| {
                unique(
                    node.children
                        .iter()
                        .copied()
                        .filter(|&i| nodes[i].namespace == MC && nodes[i].name == "Fallback"),
                )
            });
        if let Some(selected) = selected {
            pending.extend(nodes[selected].children.iter().rev().copied());
        }
    }
    result
}
fn unique(mut indexes: impl Iterator<Item = usize>) -> Option<usize> {
    let first = indexes.next()?;
    indexes.next().is_none().then_some(first)
}

/// Finds the first cell-metadata block proven to describe a dynamic array.
/// Office's cell-metadata and type indexes are one-based; the future-metadata
/// record index is zero-based. Unsupported or ambiguous links are not evidence
/// of a dynamic array. XML errors are reported; unrelated metadata is ignored.
/// This resolves the selected compatibility representation; it does not certify
/// that the same index has the same meaning in other stored branches.
pub fn first_dynamic_array_cell_metadata(xml: &str) -> Result<Option<u32>> {
    Ok(cell_dynamic_array_flags(xml)?
        .iter()
        .position(|flag| *flag == Some(true))
        .and_then(|index| u32::try_from(index + 1).ok()))
}

/// Explicit dynamic/fixed flags indexed by Office's one-based `cm` minus one.
/// An absent, unsupported, invalid or ambiguous record yields `None`.
/// Compatibility content uses the first supported Choice, otherwise Fallback.
pub fn cell_dynamic_array_flags(xml: &str) -> Result<Vec<Option<bool>>> {
    let mut reader = NsReader::from_str(xml);
    let mut nodes = vec![Node::default()];
    let mut stack = vec![0];
    loop {
        let event = reader.read_event()?;
        let opened = matches!(&event, Event::Start(_));
        match event {
            Event::Start(element) | Event::Empty(element) => {
                let namespace = match reader.resolver().resolve_element(element.name()).0 {
                    ResolveResult::Bound(namespace) => {
                        String::from_utf8_lossy(namespace.as_ref()).into_owned()
                    }
                    ResolveResult::Unbound => String::new(),
                    ResolveResult::Unknown(_) => "!unresolved".into(),
                };
                let mut attributes = Vec::new();
                for attribute in element.attributes() {
                    let attribute = attribute?;
                    if !attribute.key.as_ref().contains(&b':') {
                        attributes.push((
                            String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
                            attribute
                                .decode_and_unescape_value(reader.decoder())?
                                .into_owned(),
                        ));
                    }
                }
                let index = nodes.len();
                let supported_choice = namespace == MC
                    && element.local_name().as_ref() == b"Choice"
                    && attributes.iter().find(|(key, _)| key == "Requires").is_some_and(|(_, required)| {
                        let prefixes: Vec<_> = required.split_ascii_whitespace().collect();
                        !prefixes.is_empty() && prefixes.iter().all(|prefix| {
                            let qualified = format!("{prefix}:x");
                            matches!(reader.resolver().resolve_element(quick_xml::name::QName(qualified.as_bytes())).0,
                                ResolveResult::Bound(ns) if matches!(ns.as_ref(),
                                    b"http://schemas.openxmlformats.org/spreadsheetml/2006/main" |
                                    b"http://purl.oclc.org/ooxml/spreadsheetml/main") || ns.as_ref() == DYNAMIC.as_bytes())
                        })
                    });
                nodes.push(Node {
                    namespace,
                    name: String::from_utf8_lossy(element.local_name().as_ref()).into_owned(),
                    attributes,
                    children: Vec::new(),
                    supported_choice,
                });
                nodes[*stack.last().unwrap()].children.push(index);
                if opened {
                    stack.push(index);
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if stack.len() != 1 {
        return Ok(Vec::new());
    }
    let Some(root) = unique(compatible_children(&nodes, 0).into_iter()) else {
        return Ok(Vec::new());
    };
    let namespace = nodes[root].namespace.as_str();
    if nodes[root].name != "metadata"
        || !matches!(
            namespace,
            "" | "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                | "http://purl.oclc.org/ooxml/spreadsheetml/main"
        )
    {
        return Ok(Vec::new());
    }
    let Some(types) = unique(children(&nodes, root, namespace, "metadataTypes")) else {
        return Ok(Vec::new());
    };
    let types: Vec<_> = children(&nodes, types, namespace, "metadataType").collect();
    let dynamic_types: Vec<_> = types
        .iter()
        .enumerate()
        .filter(|(_, i)| nodes[**i].attribute("name") == Some("XLDAPR"))
        .map(|(i, _)| i + 1)
        .collect();
    if dynamic_types.len() != 1 {
        return Ok(Vec::new());
    }
    let Some(future) = unique(
        children(&nodes, root, namespace, "futureMetadata")
            .filter(|&i| nodes[i].attribute("name") == Some("XLDAPR")),
    ) else {
        return Ok(Vec::new());
    };
    let records: Vec<_> = children(&nodes, future, namespace, "bk")
        .map(|record| {
            let list = unique(children(&nodes, record, namespace, "extLst"))?;
            let extension = unique(children(&nodes, list, namespace, "ext").filter(|&i| {
                nodes[i]
                    .attribute("uri")
                    .is_some_and(|uri| uri.eq_ignore_ascii_case(EXTENSION))
            }))?;
            let properties = unique(children(
                &nodes,
                extension,
                DYNAMIC,
                "dynamicArrayProperties",
            ))?;
            match nodes[properties].attribute("fDynamic").map(xml_space) {
                Some("1" | "true") => Some(true),
                Some("0" | "false") => Some(false),
                _ => None,
            }
        })
        .collect();
    let Some(cells) = unique(children(&nodes, root, namespace, "cellMetadata")) else {
        return Ok(Vec::new());
    };
    Ok(children(&nodes, cells, namespace, "bk")
        .map(|block| {
            let link = unique(
                children(&nodes, block, namespace, "rc")
                    .filter(|&i| nodes[i].index("t") == Some(dynamic_types[0])),
            )?;
            let record = nodes[link].index("v")?;
            records.get(record).copied().flatten()
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<metadata xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:d="http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray" xmlns:foreign="urn:foreign"><metadataTypes><metadataType name="OTHER"/><metadataType name="XLDAPR"/></metadataTypes><futureMetadata name="XLDAPR"><bk><extLst><ext uri="{bdbb8cdc-fa1e-496e-a857-3c3f30c029c3}"><d:dynamicArrayProperties fDynamic="false"/></ext></extLst></bk><bk><extLst><ext uri="{bdbb8cdc-fa1e-496e-a857-3c3f30c029c3}"><d:dynamicArrayProperties fDynamic="true"/></ext></extLst></bk></futureMetadata><cellMetadata><bk><rc t="2" v="0"/></bk><bk><rc t="2" v="1"/></bk></cellMetadata></metadata>"#;

    #[test]
    fn metadata_choices_resolve_required_namespaces_and_keep_first_supported_order() {
        let property = "<d:dynamicArrayProperties fDynamic=\"true\"/>";
        for (requires, expected) in [
            ("known", Some(false)),
            ("future", Some(true)),
            ("missing", Some(true)),
            ("known future", Some(true)),
            ("", Some(true)),
        ] {
            let branch = format!("<mc:AlternateContent xmlns:mc=\"{MC}\"><mc:Choice xmlns:known=\"{DYNAMIC}\" xmlns:future=\"urn:future\" Requires=\"{requires}\"><d:dynamicArrayProperties fDynamic=\"false\"/></mc:Choice><mc:Fallback>{property}</mc:Fallback></mc:AlternateContent>");
            for xml in [
                XML.to_string(),
                XML.replace(
                    "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
                    "http://purl.oclc.org/ooxml/spreadsheetml/main",
                ),
            ] {
                let xml = xml.replace(property, &branch);
                assert_eq!(
                    cell_dynamic_array_flags(&xml).unwrap(),
                    [Some(false), expected]
                );
            }
        }
        let first = format!("<mc:AlternateContent xmlns:mc=\"{MC}\"><mc:Choice Requires=\"d\"><d:dynamicArrayProperties fDynamic=\"false\"/></mc:Choice><mc:Choice Requires=\"d\">{property}</mc:Choice><mc:Fallback>{property}</mc:Fallback></mc:AlternateContent>");
        assert_eq!(
            cell_dynamic_array_flags(&XML.replace(property, &first)).unwrap(),
            [Some(false), Some(false)]
        );
        let empty = format!("<mc:AlternateContent xmlns:mc=\"{MC}\"><mc:Choice Requires=\"d\"/><mc:Fallback>{property}</mc:Fallback></mc:AlternateContent>");
        assert_eq!(
            cell_dynamic_array_flags(&XML.replace(property, &empty)).unwrap(),
            [Some(false), None]
        );
        let mut nested = property.to_string();
        for _ in 0..256 {
            nested = format!("<mc:AlternateContent xmlns:mc=\"{MC}\"><mc:Choice Requires=\"d\">{nested}</mc:Choice><mc:Fallback/></mc:AlternateContent>");
        }
        assert_eq!(
            cell_dynamic_array_flags(&XML.replace(property, &nested)).unwrap(),
            [Some(false), Some(true)]
        );
        let foreign = first.replace(MC, "urn:foreign");
        assert_eq!(
            cell_dynamic_array_flags(&XML.replace(property, &foreign)).unwrap(),
            [Some(false), None]
        );
    }

    #[test]
    fn reads_selected_metadata_compatibility_branches() {
        let declared = XML.replacen(
            "<metadata ",
            "<metadata xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" ",
            1,
        );
        for (start, end) in [
            ("<metadataTypes>", "</metadataTypes>"),
            ("<metadataType name=\"XLDAPR\"/>", ""),
            ("<futureMetadata name=\"XLDAPR\">", "</futureMetadata>"),
            ("<cellMetadata>", "</cellMetadata>"),
            ("<rc t=\"2\" v=\"1\"/>", ""),
            ("<d:dynamicArrayProperties fDynamic=\"true\"/>", ""),
        ] {
            let from = declared.find(start).unwrap();
            let to = if end.is_empty() {
                from + start.len()
            } else {
                from + declared[from..].find(end).unwrap() + end.len()
            };
            let content = &declared[from..to];
            for requires in ["d", "future", "missing", "d future"] {
                let branch = format!("<mc:AlternateContent xmlns:future=\"urn:future\"><mc:Choice Requires=\"{requires}\">{content}</mc:Choice><mc:Fallback>{content}</mc:Fallback></mc:AlternateContent>");
                let xml = format!("{}{}{}", &declared[..from], branch, &declared[to..]);
                assert_eq!(
                    cell_dynamic_array_flags(&xml).unwrap(),
                    [Some(false), Some(true)],
                    "{start}, {requires}"
                );
            }
        }
    }

    #[test]
    fn resolves_type_and_record_indexes_with_namespace_aliases() {
        assert_eq!(
            cell_dynamic_array_flags(XML).unwrap(),
            [Some(false), Some(true)]
        );
        assert_eq!(
            cell_dynamic_array_flags(&XML.replace("fDynamic=\"false\"", "")).unwrap(),
            [None, Some(true)]
        );
        assert_eq!(
            cell_dynamic_array_flags(&XML.replace("fDynamic=\"false\"", "fDynamic=\"invalid\""))
                .unwrap(),
            [None, Some(true)]
        );
        for xml in [
            XML.to_string(),
            XML.replace(
                "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
                "http://purl.oclc.org/ooxml/spreadsheetml/main",
            ),
            XML.replace(
                " xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"",
                "",
            ),
            XML.replace("fDynamic=\"true\"", "fDynamic=\"1\""),
            XML.replace("fDynamic=\"true\"", "fDynamic=\" &#x9;true &#xA;\"")
                .replace("t=\"2\"", "t=\" 2 \"")
                .replace("v=\"1\"", "v=\" 1 \""),
        ] {
            assert_eq!(first_dynamic_array_cell_metadata(&xml).unwrap(), Some(2));
        }
    }

    #[test]
    fn missing_invalid_or_ambiguous_links_do_not_certify_a_dynamic_array() {
        for xml in [
            XML.replace("fDynamic=\"true\"", "fDynamic=\"0\""),
            XML.replace("fDynamic=\"true\"", "fDynamic=\"TRUE\""),
            XML.replace("fDynamic=\"true\"", ""),
            XML.replace("fDynamic=\"true\"", "foreign:fDynamic=\"true\""),
            XML.replace("v=\"1\"", "v=\"2\""),
            XML.replace("v=\"1\"", ""),
            XML.replace("t=\"2\"", "t=\"0\""),
            XML.replace("<rc t=\"2\" v=\"1\"/>", "<rc t=\"2\" v=\"1\"/><rc t=\"2\" v=\"0\"/>"),
            XML.replace("<metadataType name=\"XLDAPR\"/>", "<metadataType name=\"XLDAPR\"/><metadataType name=\"XLDAPR\"/>"),
            XML.replace("</futureMetadata>", "</futureMetadata><futureMetadata name=\"XLDAPR\"/>"),
            XML.replace("<d:dynamicArrayProperties fDynamic=\"true\"/>", "<d:dynamicArrayProperties fDynamic=\"true\"/><d:dynamicArrayProperties fDynamic=\"false\"/>"),
        ] {
            assert_eq!(first_dynamic_array_cell_metadata(&xml).unwrap(), None, "{xml}");
        }
    }

    #[test]
    fn foreign_names_and_wrong_hierarchy_cannot_supply_the_flag() {
        for xml in [
            XML.replace(
                "<d:dynamicArrayProperties",
                "<foreign:dynamicArrayProperties",
            ),
            XML.replace("<rc ", "<foreign:rc "),
            XML.replace("<metadataType ", "<foreign:metadataType "),
            XML.replace("<cellMetadata>", "<foreign:wrapper><cellMetadata>")
                .replace("</cellMetadata>", "</cellMetadata></foreign:wrapper>"),
            XML.replace(
                "<d:dynamicArrayProperties fDynamic=\"true\"/>",
                "<foreign:wrapper><d:dynamicArrayProperties fDynamic=\"true\"/></foreign:wrapper>",
            ),
            XML.replace(
                "{bdbb8cdc-fa1e-496e-a857-3c3f30c029c3}",
                "{00000000-0000-0000-0000-000000000000}",
            ),
        ] {
            assert_eq!(
                first_dynamic_array_cell_metadata(&xml).unwrap(),
                None,
                "{xml}"
            );
        }
        let with_decoys = XML.replace("<metadataTypes>", "<foreign:metadataType name=\"XLDAPR\"/><metadataTypes><foreign:metadataType name=\"XLDAPR\"/>");
        assert_eq!(
            first_dynamic_array_cell_metadata(&with_decoys).unwrap(),
            Some(2)
        );
    }

    #[test]
    fn invalid_xml_is_not_treated_as_valid_metadata() {
        assert!(first_dynamic_array_cell_metadata("<metadata></wrong>").is_err());
        assert_eq!(
            first_dynamic_array_cell_metadata(&XML.replace("</metadata>", "")).unwrap(),
            None
        );
        assert_eq!(
            first_dynamic_array_cell_metadata("<metadata/><metadata/>").unwrap(),
            None
        );
    }
}
