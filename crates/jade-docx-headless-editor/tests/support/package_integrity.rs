//! Independent OPC graph checks shared by the property and fuzz harnesses.
use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};

const REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const CT: &str = "http://schemas.openxmlformats.org/package/2006/content-types";

fn resolve_target(source: &str, target: &str) -> String {
    let mut path: Vec<_> = if target.starts_with('/') {
        Vec::new()
    } else {
        source
            .rsplit_once('/')
            .map_or(Vec::new(), |(dir, _)| dir.split('/').collect())
    };
    for segment in target.split('#').next().unwrap().split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                assert!(path.pop().is_some(), "relationship escapes package root");
            }
            _ => path.push(segment),
        }
    }
    path.join("/")
}

pub fn assert_valid_package(bytes: &[u8]) {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("valid ZIP");
    let mut files = HashMap::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).unwrap();
        if file.is_dir() {
            continue;
        }
        let mut data = Vec::new();
        file.read_to_end(&mut data)
            .expect("entry decompression and CRC");
        assert!(
            files
                .insert(
                    file.name().expect("valid ZIP entry name").into_owned(),
                    data
                )
                .is_none(),
            "duplicate ZIP entry"
        );
    }
    let xml: HashMap<_, _> = files
        .iter()
        .filter(|(name, _)| name.ends_with(".xml") || name.ends_with(".rels"))
        .map(|(name, data)| {
            let text = std::str::from_utf8(data).expect("fixture XML is UTF-8");
            let parsed = roxmltree::Document::parse(text)
                .unwrap_or_else(|err| panic!("invalid XML in {name}: {err}"));
            (name.as_str(), parsed)
        })
        .collect();
    let types = xml
        .get("[Content_Types].xml")
        .expect("content type manifest");
    let mut defaults = HashMap::new();
    let mut overrides = HashMap::new();
    for node in types
        .root_element()
        .children()
        .filter(|node| node.is_element())
    {
        if node.has_tag_name((CT, "Default")) {
            let extension = node.attribute("Extension").unwrap();
            assert!(
                defaults
                    .insert(extension, node.attribute("ContentType").unwrap())
                    .is_none(),
                "duplicate default content type"
            );
        } else if node.has_tag_name((CT, "Override")) {
            let name = node
                .attribute("PartName")
                .unwrap()
                .strip_prefix('/')
                .unwrap();
            assert!(
                files.contains_key(name),
                "content type points to missing part {name}"
            );
            assert!(
                overrides
                    .insert(name, node.attribute("ContentType").unwrap())
                    .is_none(),
                "duplicate override content type"
            );
        }
    }
    for name in files
        .keys()
        .filter(|name| name.as_str() != "[Content_Types].xml")
    {
        assert!(
            overrides.contains_key(name.as_str())
                || name
                    .rsplit_once('.')
                    .is_some_and(|(_, extension)| defaults.contains_key(extension)),
            "missing content type for {name}"
        );
    }
    let mut relationships = HashMap::new();
    for (name, part) in &xml {
        let source = if *name == "_rels/.rels" {
            String::new()
        } else if let Some((dir, file)) = name.rsplit_once("/_rels/") {
            format!("{dir}/{}", file.strip_suffix(".rels").unwrap())
        } else {
            continue;
        };
        if !source.is_empty() {
            assert!(
                files.contains_key(&source),
                "relationships for missing source {source}"
            );
        }
        let mut ids = HashSet::new();
        for node in part
            .descendants()
            .filter(|node| node.has_tag_name((REL, "Relationship")))
        {
            let id = node.attribute("Id").unwrap();
            assert!(ids.insert(id), "duplicate relationship id {id} in {name}");
            if node.attribute("TargetMode") != Some("External") {
                let target = resolve_target(&source, node.attribute("Target").unwrap());
                assert!(
                    files.contains_key(&target),
                    "missing relationship target {target} from {name}"
                );
            }
        }
        relationships.insert(source, ids);
    }
    assert!(relationships.contains_key(""), "package root relationships");
    for (name, part) in &xml {
        for attribute in part.descendants().flat_map(|node| node.attributes()) {
            if attribute.namespace() == Some(R)
                && matches!(attribute.name(), "id" | "embed" | "link")
            {
                assert!(
                    relationships
                        .get(*name)
                        .is_some_and(|ids| ids.contains(attribute.value())),
                    "unresolved relationship {} in {name}",
                    attribute.value()
                );
            }
        }
    }
}
