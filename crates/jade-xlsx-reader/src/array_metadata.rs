//! Resolves dynamic-array cell metadata without changing the source part.
use crate::Result;
use quick_xml::{events::Event, name::ResolveResult, NsReader};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
};

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

struct View<'a> {
    nodes: &'a [Node],
    choices: BTreeMap<usize, Option<usize>>,
    visited: RefCell<BTreeSet<usize>>,
}
impl<'a> View<'a> {
    fn new(nodes: &'a [Node], choices: BTreeMap<usize, Option<usize>>) -> Self {
        Self {
            nodes,
            choices,
            visited: RefCell::default(),
        }
    }
}

fn children<'a>(
    view: &'a View<'a>,
    parent: usize,
    namespace: &'a str,
    name: &'a str,
) -> impl Iterator<Item = usize> + 'a {
    compatible_children(view, parent)
        .into_iter()
        .filter(move |&i| view.nodes[i].namespace == namespace && view.nodes[i].name == name)
}

fn compatible_children(view: &View<'_>, parent: usize) -> Vec<usize> {
    let nodes = view.nodes;
    let mut pending: Vec<_> = nodes[parent].children.iter().rev().copied().collect();
    let mut result = Vec::new();
    while let Some(index) = pending.pop() {
        let node = &nodes[index];
        if node.namespace != MC || node.name != "AlternateContent" {
            result.push(index);
            continue;
        }
        view.visited.borrow_mut().insert(index);
        let selected = view.choices.get(&index).copied().unwrap_or_else(|| {
            node.children
                .iter()
                .copied()
                .find(|&i| {
                    nodes[i].namespace == MC
                        && nodes[i].name == "Choice"
                        && nodes[i].supported_choice
                })
                .or_else(|| {
                    unique(
                        node.children
                            .iter()
                            .copied()
                            .filter(|&i| nodes[i].namespace == MC && nodes[i].name == "Fallback"),
                    )
                })
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

#[derive(Eq, PartialEq, Ord, PartialOrd)]
enum Signature {
    Element(String, String, Vec<(String, String)>, Vec<usize>),
    Alternatives(Vec<Vec<usize>>),
}

fn branches(nodes: &[Node], parent: usize) -> impl Iterator<Item = usize> + '_ {
    nodes[parent].children.iter().copied().filter(|&i| {
        nodes[i].namespace == MC && matches!(nodes[i].name.as_str(), "Choice" | "Fallback")
    })
}

// Intern exact structural keys, not hashes: equivalent alternatives can be
// collapsed without ever trusting a hash collision for metadata identity.
fn signatures(nodes: &[Node]) -> Vec<Vec<usize>> {
    let mut result = vec![Vec::new(); nodes.len()];
    let mut interned = BTreeMap::new();
    for index in (0..nodes.len()).rev() {
        let node = &nodes[index];
        let content = |parent: usize| {
            nodes[parent]
                .children
                .iter()
                .flat_map(|&child| result[child].iter().copied())
                .collect::<Vec<_>>()
        };
        let key = if node.namespace == MC && node.name == "AlternateContent" {
            let mut alternatives: BTreeSet<_> = branches(nodes, index).map(content).collect();
            if !branches(nodes, index).any(|i| nodes[i].name == "Fallback") {
                alternatives.insert(Vec::new());
            }
            if alternatives.len() <= 1 {
                result[index] = alternatives.into_iter().next().unwrap_or_default();
                continue;
            }
            Signature::Alternatives(alternatives.into_iter().collect())
        } else {
            let mut attributes = node.attributes.clone();
            attributes.sort();
            Signature::Element(
                node.namespace.clone(),
                node.name.clone(),
                attributes,
                content(index),
            )
        };
        let next = interned.len();
        result[index] = vec![*interned.entry(key).or_insert(next)];
    }
    result
}

/// A pure dynamic-array cell record whose one-based index has the same dynamic
/// meaning in every stored Choice/Fallback layout, including opaque Choices.
/// Composite cell records are not reusable: that would attach unrelated
/// metadata to new cells. XML errors propagate. Ambiguity or more than 1024
/// distinct partial layouts yields None rather than an uncertified index.
pub fn common_dynamic_array_cell_metadata(xml: &str) -> Result<Option<u32>> {
    let nodes = parse_nodes(xml)?;
    let initial = View::new(&nodes, BTreeMap::new());
    let mut candidates: Vec<_> = flags(&initial, true)
        .iter()
        .enumerate()
        .filter_map(|(index, &flag)| (flag == Some(true)).then_some(index))
        .collect();
    if candidates.is_empty() {
        return Ok(None);
    }
    if initial.visited.borrow().is_empty() {
        return Ok(candidates
            .first()
            .and_then(|&index| u32::try_from(index + 1).ok()));
    }
    let signatures = signatures(&nodes);
    let mut pending = vec![BTreeMap::new()];
    for _ in 0..1024 {
        let Some(choices) = pending.pop() else {
            return Ok(candidates
                .first()
                .and_then(|&index| u32::try_from(index + 1).ok()));
        };
        let view = View::new(&nodes, choices);
        let current = flags(&view, true);
        candidates.retain(|&i| current.get(i) == Some(&Some(true)));
        if candidates.is_empty() {
            return Ok(None);
        }
        let next = view
            .visited
            .borrow()
            .iter()
            .copied()
            .find(|index| !view.choices.contains_key(index));
        if let Some(next) = next {
            let mut options = BTreeMap::new();
            for branch in branches(&nodes, next) {
                let key: Vec<_> = nodes[branch]
                    .children
                    .iter()
                    .flat_map(|&child| signatures[child].iter().copied())
                    .collect();
                options.entry(key).or_insert(Some(branch));
            }
            if !branches(&nodes, next).any(|i| nodes[i].name == "Fallback") {
                options.insert(Vec::new(), None);
            }
            for branch in options.into_values() {
                let mut choices = view.choices.clone();
                choices.insert(next, branch);
                pending.push(choices);
            }
        }
    }
    Ok(None)
}

/// Explicit dynamic/fixed flags indexed by Office's one-based `cm` minus one.
/// An absent, unsupported, invalid or ambiguous record yields `None`.
/// Compatibility content uses the first supported Choice, otherwise Fallback.
pub fn cell_dynamic_array_flags(xml: &str) -> Result<Vec<Option<bool>>> {
    let nodes = parse_nodes(xml)?;
    Ok(flags(&View::new(&nodes, BTreeMap::new()), false))
}

fn parse_nodes(xml: &str) -> Result<Vec<Node>> {
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
    Ok(nodes)
}

fn flags(view: &View<'_>, reusable: bool) -> Vec<Option<bool>> {
    let nodes = view.nodes;
    if nodes.is_empty() {
        return Vec::new();
    }
    let Some(root) = unique(compatible_children(view, 0).into_iter()) else {
        return Vec::new();
    };
    let namespace = nodes[root].namespace.as_str();
    if nodes[root].name != "metadata"
        || !matches!(
            namespace,
            "" | "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                | "http://purl.oclc.org/ooxml/spreadsheetml/main"
        )
    {
        return Vec::new();
    }
    let Some(types) = unique(children(view, root, namespace, "metadataTypes")) else {
        return Vec::new();
    };
    let types: Vec<_> = children(view, types, namespace, "metadataType").collect();
    let dynamic_types: Vec<_> = types
        .iter()
        .enumerate()
        .filter(|(_, i)| nodes[**i].attribute("name") == Some("XLDAPR"))
        .map(|(i, _)| i + 1)
        .collect();
    if dynamic_types.len() != 1 {
        return Vec::new();
    }
    let Some(future) = unique(
        children(view, root, namespace, "futureMetadata")
            .filter(|&i| nodes[i].attribute("name") == Some("XLDAPR")),
    ) else {
        return Vec::new();
    };
    let records: Vec<_> = children(view, future, namespace, "bk")
        .map(|record| {
            let list = unique(children(view, record, namespace, "extLst"))?;
            let extension = unique(children(view, list, namespace, "ext").filter(|&i| {
                nodes[i]
                    .attribute("uri")
                    .is_some_and(|uri| uri.eq_ignore_ascii_case(EXTENSION))
            }))?;
            let properties = unique(children(view, extension, DYNAMIC, "dynamicArrayProperties"))?;
            match nodes[properties].attribute("fDynamic").map(xml_space) {
                Some("1" | "true") => Some(true),
                Some("0" | "false") => Some(false),
                _ => None,
            }
        })
        .collect();
    let Some(cells) = unique(children(view, root, namespace, "cellMetadata")) else {
        return Vec::new();
    };
    children(view, cells, namespace, "bk")
        .map(|block| {
            if reusable && children(view, block, namespace, "rc").count() != 1 {
                return None;
            }
            let link = unique(
                children(view, block, namespace, "rc")
                    .filter(|&i| nodes[i].index("t") == Some(dynamic_types[0])),
            )?;
            let record = nodes[link].index("v")?;
            records.get(record).copied().flatten()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<metadata xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:d="http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray" xmlns:foreign="urn:foreign"><metadataTypes><metadataType name="OTHER"/><metadataType name="XLDAPR"/></metadataTypes><futureMetadata name="XLDAPR"><bk><extLst><ext uri="{bdbb8cdc-fa1e-496e-a857-3c3f30c029c3}"><d:dynamicArrayProperties fDynamic="false"/></ext></extLst></bk><bk><extLst><ext uri="{bdbb8cdc-fa1e-496e-a857-3c3f30c029c3}"><d:dynamicArrayProperties fDynamic="true"/></ext></extLst></bk></futureMetadata><cellMetadata><bk><rc t="2" v="0"/></bk><bk><rc t="2" v="1"/></bk></cellMetadata></metadata>"#;

    #[test]
    fn common_dynamic_indexes_are_certified_across_all_stored_layouts() {
        let alternate = |other: &str| {
            let head = XML.find('>').unwrap() + 1;
            let tail = XML.rfind("</metadata>").unwrap();
            let other_head = other.find('>').unwrap() + 1;
            let other_tail = other.rfind("</metadata>").unwrap();
            format!("{}<mc:AlternateContent xmlns:mc=\"{MC}\"><mc:Choice Requires=\"d\">{}</mc:Choice><mc:Choice xmlns:u=\"urn:future\" Requires=\"u\">{}</mc:Choice><mc:Fallback>{}</mc:Fallback></mc:AlternateContent></metadata>", &XML[..head], &XML[head..tail], &other[other_head..other_tail], &XML[head..tail])
        };
        assert_eq!(common_dynamic_array_cell_metadata(XML).unwrap(), Some(2));
        let reordered = XML
            .replace(
                "<metadataType name=\"OTHER\"/><metadataType name=\"XLDAPR\"/>",
                "<metadataType name=\"XLDAPR\"/><metadataType name=\"OTHER\"/>",
            )
            .replace("t=\"2\"", "t=\"1\"");
        assert_eq!(
            common_dynamic_array_cell_metadata(&alternate(&reordered)).unwrap(),
            Some(2)
        );
        for other in [
            XML.replace("fDynamic=\"true\"", "fDynamic=\"false\""),
            XML.replace(
                "<bk><rc t=\"2\" v=\"0\"/></bk><bk><rc t=\"2\" v=\"1\"/></bk>",
                "<bk><rc t=\"2\" v=\"1\"/></bk><bk><rc t=\"2\" v=\"0\"/></bk>",
            ),
            XML.replace("<bk><rc t=\"2\" v=\"1\"/></bk>", ""),
        ] {
            assert_eq!(
                first_dynamic_array_cell_metadata(&alternate(&other)).unwrap(),
                Some(2)
            );
            assert_eq!(
                common_dynamic_array_cell_metadata(&alternate(&other)).unwrap(),
                None
            );
        }
        let composite = XML.replace(
            "<rc t=\"2\" v=\"1\"/>",
            "<rc t=\"2\" v=\"1\"/><rc t=\"1\" v=\"0\"/>",
        );
        assert_eq!(
            cell_dynamic_array_flags(&composite).unwrap(),
            [Some(false), Some(true)]
        );
        assert_eq!(
            common_dynamic_array_cell_metadata(&composite).unwrap(),
            None
        );
        let property = "<d:dynamicArrayProperties fDynamic=\"true\"/>";
        let missing_fallback = format!("<mc:AlternateContent xmlns:mc=\"{MC}\"><mc:Choice Requires=\"d\">{property}</mc:Choice></mc:AlternateContent>");
        assert_eq!(
            common_dynamic_array_cell_metadata(&XML.replace(property, &missing_fallback)).unwrap(),
            None
        );
    }

    #[test]
    fn equivalent_nested_metadata_choices_do_not_expand_exponentially() {
        let property = "<d:dynamicArrayProperties fDynamic=\"true\"/>";
        let mut nested = property.to_string();
        for _ in 0..256 {
            nested = format!("<mc:AlternateContent xmlns:mc=\"{MC}\"><mc:Choice Requires=\"d\">{nested}</mc:Choice><mc:Fallback>{property}</mc:Fallback></mc:AlternateContent>");
        }
        assert_eq!(
            common_dynamic_array_cell_metadata(&XML.replace(property, &nested)).unwrap(),
            Some(2)
        );
    }

    #[test]
    fn certification_does_not_guess_after_its_layout_budget_is_exhausted() {
        let mut types = String::new();
        for index in 0..12 {
            types.push_str(&format!("<mc:AlternateContent xmlns:mc=\"{MC}\"><mc:Choice Requires=\"d\"><metadataType name=\"OTHER{index}A\"/></mc:Choice><mc:Fallback><metadataType name=\"OTHER{index}B\"/></mc:Fallback></mc:AlternateContent>"));
        }
        let xml = XML
            .replace("<metadataType name=\"OTHER\"/>", &types)
            .replace("t=\"2\"", "t=\"13\"");
        assert_eq!(first_dynamic_array_cell_metadata(&xml).unwrap(), Some(2));
        assert_eq!(common_dynamic_array_cell_metadata(&xml).unwrap(), None);
    }

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
