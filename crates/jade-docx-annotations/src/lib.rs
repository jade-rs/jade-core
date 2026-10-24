//! Package-wide annotation decisions for copies of a DOCX.
//!
//! The interactive review commands operate on the editable body model. This
//! module works on OPC parts, so secondary stories and orphaned comment parts
//! receive the same decision without changing the meaning of those commands.

use jade_docx_reader::{DocxXmlDocument, OpenXmlPackage, XmlElement, XmlNode};
use std::collections::{HashMap, HashSet};

const WORD_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const STRICT_WORD_NS: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";

/// Return a DOCX copy with package-wide review choices applied.
///
/// Both flags are positive: `true` keeps that annotation class. The source
/// bytes are never changed. Unknown XML in an unrelated part is copied intact;
/// an annotation we cannot remove causes an error instead of a clean-looking
/// result with review markup still present.
pub async fn transform_docx_annotations(
    data: &[u8],
    include_comments: bool,
    include_tracked_changes: bool,
) -> Result<Vec<u8>, String> {
    let package = OpenXmlPackage::load(data)
        .await
        .map_err(|error| format!("failed to read DOCX: {error}"))?;
    if include_comments && include_tracked_changes {
        return Ok(data.to_vec());
    }

    let mut overrides = HashMap::new();
    let mut removed = HashSet::new();
    // 承諾の前に本文側（本文・ヘッダー・フッター・脚注など）にあった参照（コメント・脚注・文末脚注）。承諾の後と比べて、
    // 参照ごと消えたコメント・脚注を外すのに使う。
    let tracks_lost_comments = include_comments && !include_tracked_changes;
    let mut references_before: HashMap<&'static str, HashSet<String>> = HashMap::new();
    if !include_comments {
        for (path, bytes) in &package.files {
            if !path.ends_with(".rels") {
                continue;
            }
            let Some(doc) = parse_package_xml(path, bytes)? else {
                continue;
            };
            for node in &doc.root().children {
                let XmlNode::Element(relationship) = node else {
                    continue;
                };
                if local_name(&relationship.name) != "Relationship"
                    || !is_comment_relationship(attribute(relationship, "Type").unwrap_or_default())
                    || attribute(relationship, "TargetMode") == Some("External")
                {
                    continue;
                }
                if let Some(target) = attribute(relationship, "Target") {
                    let target = resolve_relationship_target(path, target)?;
                    if package.files.contains_key(&target) {
                        removed.insert(target);
                    }
                }
            }
        }
    }
    for (path, bytes) in &package.files {
        if removed.contains(path)
            || !path.ends_with(".xml")
            || path == "[Content_Types].xml"
            || path.ends_with(".rels")
        {
            continue;
        }
        let xml = match std::str::from_utf8(bytes) {
            Ok(xml) => xml,
            Err(error) if path.starts_with("word/") => {
                return Err(format!("{path} is not UTF-8 XML: {error}"));
            }
            Err(_) => continue,
        };
        let mut doc = match DocxXmlDocument::parse(xml) {
            Ok(doc) => doc,
            Err(error) if path.starts_with("word/") => {
                return Err(format!("cannot inspect {path}: {error}"));
            }
            Err(_) => continue,
        };
        if !include_comments && is_comment_part(doc.root()) {
            removed.insert(path.clone());
            continue;
        }
        let mut changed = false;
        let mut namespace = HashMap::new();
        namespace.insert(String::new(), String::new());
        if !include_tracked_changes && !is_comment_part(doc.root()) {
            for reference in REFERENCE_NAMES {
                collect_references(
                    doc.root(),
                    &namespace,
                    reference,
                    references_before.entry(reference).or_default(),
                );
            }
        }
        let root_id = doc.root().id;
        let root = doc.element_mut(root_id).expect("root is indexed");
        transform_element(
            root,
            &namespace,
            include_comments,
            include_tracked_changes,
            &mut changed,
        )?;
        if has_annotation(root, &namespace, include_comments, include_tracked_changes) {
            return Err(format!("unsupported review markup remains in {path}"));
        }
        if changed {
            overrides.insert(
                path.clone(),
                doc.to_string()
                    .map_err(|error| format!("cannot write {path}: {error}"))?
                    .into_bytes(),
            );
        }
    }

    if !include_tracked_changes {
        // 承諾で参照ごと消えた脚注・文末脚注を外す（Word の「すべて承諾」も消す）。脚注の中のコメントの参照も一緒に
        // 消えるので、コメントはその後で比べる。
        let after = references_in(
            &package.files,
            &removed,
            &overrides,
            &["footnoteReference", "endnoteReference"],
        );
        for (reference, part, note) in [
            ("footnoteReference", "footnotes", "footnote"),
            ("endnoteReference", "endnotes", "endnote"),
        ] {
            let lost: HashSet<String> = references_before
                .get(reference)
                .map(|before| before.difference(&after[reference]).cloned().collect())
                .unwrap_or_default();
            if !lost.is_empty() {
                remove_lost_notes(&package.files, &removed, &mut overrides, part, note, &lost)?;
            }
        }
    }
    if tracks_lost_comments {
        let after = references_in(&package.files, &removed, &overrides, &["commentReference"]);
        let lost: HashSet<String> = references_before
            .get("commentReference")
            .map(|before| {
                before
                    .difference(&after["commentReference"])
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if !lost.is_empty() {
            remove_lost_comments(&package.files, &removed, &mut overrides, &lost)?;
        }
    }

    if !include_comments {
        let comment_parts: Vec<_> = removed.iter().cloned().collect();
        for path in comment_parts {
            let (folder, file) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
            let relationship_path = if folder.is_empty() {
                format!("_rels/{file}.rels")
            } else {
                format!("{folder}/_rels/{file}.rels")
            };
            if package.files.contains_key(&relationship_path) {
                removed.insert(relationship_path);
            }
        }
    }

    if !include_comments {
        for (path, bytes) in &package.files {
            if removed.contains(path) || !(path.ends_with(".rels") || path == "[Content_Types].xml")
            {
                continue;
            }
            let Some(mut doc) = parse_package_xml(path, bytes)? else {
                continue;
            };
            let root_id = doc.root().id;
            let root = doc.element_mut(root_id).expect("root is indexed");
            let before = root.children.len();
            root.children.retain(|node| {
                let XmlNode::Element(element) = node else {
                    return true;
                };
                let local = local_name(&element.name);
                if path.ends_with(".rels") && local == "Relationship" {
                    let relation_type = attribute(element, "Type").unwrap_or_default();
                    return !is_comment_relationship(relation_type);
                }
                if path == "[Content_Types].xml" && local == "Override" {
                    let part_name = attribute(element, "PartName")
                        .unwrap_or_default()
                        .trim_start_matches('/');
                    return !removed.contains(part_name);
                }
                true
            });
            if root.children.len() != before {
                overrides.insert(
                    path.clone(),
                    doc.to_string()
                        .map_err(|error| format!("cannot write {path}: {error}"))?
                        .into_bytes(),
                );
            }
        }
    }
    jade_docx_writer::write_docx_with_removed(&package.files, &overrides, &removed)
        .map_err(|error| format!("cannot write DOCX: {error}"))
}

/// 承諾の前後で比べる参照。
const REFERENCE_NAMES: [&str; 3] = ["commentReference", "footnoteReference", "endnoteReference"];

/// 要素の中の参照（`reference`、例: `w:commentReference`）の ID を集める。
fn collect_references(
    element: &XmlElement,
    inherited: &HashMap<String, String>,
    reference: &str,
    out: &mut HashSet<String>,
) {
    let namespace = namespace_for(element, inherited);
    if local_name(&element.name) == reference && is_word(element, &namespace) {
        if let Some(id) = attribute(element, "id") {
            out.insert(id.to_string());
        }
    }
    for child in &element.children {
        if let XmlNode::Element(child) = child {
            collect_references(child, &namespace, reference, out);
        }
    }
}

/// 今の（書き換えた後の）本文側の部品にある参照の ID。コメントの部品は見ない。
fn references_in(
    files: &HashMap<String, Vec<u8>>,
    removed: &HashSet<String>,
    overrides: &HashMap<String, Vec<u8>>,
    references: &[&'static str],
) -> HashMap<&'static str, HashSet<String>> {
    let mut found: HashMap<&'static str, HashSet<String>> =
        references.iter().map(|r| (*r, HashSet::new())).collect();
    for path in files.keys() {
        if removed.contains(path) || !path.ends_with(".xml") || path == "[Content_Types].xml" {
            continue;
        }
        let bytes = overrides.get(path).unwrap_or(&files[path]);
        let Some(doc) = std::str::from_utf8(bytes)
            .ok()
            .and_then(|xml| DocxXmlDocument::parse(xml).ok())
        else {
            continue;
        };
        if is_comment_part(doc.root()) {
            continue;
        }
        let mut namespace = HashMap::new();
        namespace.insert(String::new(), String::new());
        for reference in references {
            collect_references(
                doc.root(),
                &namespace,
                reference,
                found.get_mut(reference).expect("listed"),
            );
        }
    }
    found
}

/// 承諾で本文から参照ごと消えた脚注・文末脚注（`lost`、`w:id`）を、脚注の部品（`part` は footnotes / endnotes）から外す。
/// 区切りの脚注（`w:type` のあるもの）には触らない。
fn remove_lost_notes(
    files: &HashMap<String, Vec<u8>>,
    removed: &HashSet<String>,
    overrides: &mut HashMap<String, Vec<u8>>,
    part: &str,
    note: &str,
    lost: &HashSet<String>,
) -> Result<(), String> {
    let mut paths: Vec<_> = files
        .keys()
        .filter(|path| !removed.contains(*path) && path.ends_with(".xml"))
        .cloned()
        .collect();
    paths.sort();
    for path in paths {
        let bytes = overrides.get(&path).unwrap_or(&files[&path]);
        let Some(mut doc) = std::str::from_utf8(bytes)
            .ok()
            .and_then(|xml| DocxXmlDocument::parse(xml).ok())
        else {
            continue;
        };
        let mut namespace = HashMap::new();
        namespace.insert(String::new(), String::new());
        let root_ns = namespace_for(doc.root(), &namespace);
        if local_name(&doc.root().name) != part || !is_word(doc.root(), &root_ns) {
            continue;
        }
        let root_id = doc.root().id;
        let root = doc.element_mut(root_id).expect("root is indexed");
        let before = root.children.len();
        root.children.retain(|node| {
            !matches!(node, XmlNode::Element(element)
                if local_name(&element.name) == note
                    && attribute(element, "type").is_none()
                    && attribute(element, "id").is_some_and(|id| lost.contains(id)))
        });
        if root.children.len() != before {
            let xml = doc
                .to_string()
                .map_err(|error| format!("cannot write {path}: {error}"))?;
            overrides.insert(path, xml.into_bytes());
        }
    }
    Ok(())
}

/// `keep` が false の子孫要素を外す。外したら true。
fn retain_descendants(
    element: &mut XmlElement,
    keep: &mut impl FnMut(&XmlElement) -> bool,
) -> bool {
    let before = element.children.len();
    element
        .children
        .retain(|node| !matches!(node, XmlNode::Element(child) if !keep(child)));
    let mut changed = element.children.len() != before;
    for child in &mut element.children {
        if let XmlNode::Element(child) = child {
            changed |= retain_descendants(child, keep);
        }
    }
    changed
}

/// 承諾で本文から参照ごと消えたコメント（`lost`、`w:id`）を、コメントの部品と付随の部品、本文側に残った範囲の印から外す。
///
/// Word の「すべて承諾」は、削除した行・セル・文字の中にあったコメントも一緒に消す。残すと、本文のどこにも付いていない
/// コメントになる。付随の部品はコメントの段落の `w14:paraId`（commentsExtended・commentsIds）と、commentsIds の
/// `durableId`（commentsExtensible）でコメントを指す。
fn remove_lost_comments(
    files: &HashMap<String, Vec<u8>>,
    removed: &HashSet<String>,
    overrides: &mut HashMap<String, Vec<u8>>,
    lost: &HashSet<String>,
) -> Result<(), String> {
    let mut paths: Vec<_> = files
        .keys()
        .filter(|path| {
            !removed.contains(*path) && path.ends_with(".xml") && *path != "[Content_Types].xml"
        })
        .cloned()
        .collect();
    paths.sort();
    let current = |overrides: &HashMap<String, Vec<u8>>, path: &str| -> Option<DocxXmlDocument> {
        let bytes = overrides.get(path).or_else(|| files.get(path))?;
        DocxXmlDocument::parse(std::str::from_utf8(bytes).ok()?).ok()
    };
    let write = |overrides: &mut HashMap<String, Vec<u8>>,
                 path: &str,
                 doc: DocxXmlDocument|
     -> Result<(), String> {
        let xml = doc
            .to_string()
            .map_err(|error| format!("cannot write {path}: {error}"))?;
        overrides.insert(path.to_string(), xml.into_bytes());
        Ok(())
    };

    // 1. コメントの部品からコメントを外し、その段落の paraId を集める。本文側からは残った範囲の印を外す。
    let mut lost_para_ids = HashSet::new();
    for path in &paths {
        let Some(mut doc) = current(overrides, path) else {
            continue;
        };
        let root_id = doc.root().id;
        let comment_part = is_comment_part(doc.root());
        let root_name = local_name(&doc.root().name).to_string();
        let root = doc.element_mut(root_id).expect("root is indexed");
        let changed = if comment_part && root_name == "comments" {
            retain_descendants(root, &mut |element| {
                let lost_comment = local_name(&element.name) == "comment"
                    && attribute(element, "id").is_some_and(|id| lost.contains(id));
                if lost_comment {
                    collect_para_ids(element, &mut lost_para_ids);
                }
                !lost_comment
            })
        } else if !comment_part {
            retain_descendants(root, &mut |element| {
                !(matches!(
                    local_name(&element.name),
                    "commentRangeStart" | "commentRangeEnd"
                ) && attribute(element, "id").is_some_and(|id| lost.contains(id)))
            })
        } else {
            false
        };
        if changed {
            write(overrides, path, doc)?;
        }
    }

    // 2. commentsExtended・commentsIds から、外したコメントの段落を指す項目を外す（commentsIds の durableId を集める）。
    let mut lost_durable_ids = HashSet::new();
    for path in &paths {
        let Some(mut doc) = current(overrides, path) else {
            continue;
        };
        if !is_comment_part(doc.root()) {
            continue;
        }
        let root_name = local_name(&doc.root().name).to_string();
        let root_id = doc.root().id;
        let root = doc.element_mut(root_id).expect("root is indexed");
        let changed = match root_name.as_str() {
            "commentsEx" => retain_descendants(root, &mut |element| {
                !(local_name(&element.name) == "commentEx"
                    && attribute(element, "paraId")
                        .is_some_and(|id| lost_para_ids.contains(&id.to_ascii_uppercase())))
            }),
            "commentsIds" => retain_descendants(root, &mut |element| {
                let lost_id = local_name(&element.name) == "commentId"
                    && attribute(element, "paraId")
                        .is_some_and(|id| lost_para_ids.contains(&id.to_ascii_uppercase()));
                if lost_id {
                    if let Some(durable) = attribute(element, "durableId") {
                        lost_durable_ids.insert(durable.to_string());
                    }
                }
                !lost_id
            }),
            _ => false,
        };
        if changed {
            write(overrides, path, doc)?;
        }
    }

    // 3. commentsExtensible から、外したコメントの durableId を外す。
    for path in &paths {
        let Some(mut doc) = current(overrides, path) else {
            continue;
        };
        if !is_comment_part(doc.root()) || local_name(&doc.root().name) != "commentsExtensible" {
            continue;
        }
        let root_id = doc.root().id;
        let root = doc.element_mut(root_id).expect("root is indexed");
        let changed = retain_descendants(root, &mut |element| {
            !(local_name(&element.name) == "commentExtensible"
                && attribute(element, "durableId").is_some_and(|id| lost_durable_ids.contains(id)))
        });
        if changed {
            write(overrides, path, doc)?;
        }
    }
    Ok(())
}

/// コメントの段落の `w14:paraId`（大文字にそろえる）を集める。
fn collect_para_ids(element: &XmlElement, out: &mut HashSet<String>) {
    if local_name(&element.name) == "p" {
        if let Some(id) = attribute(element, "paraId") {
            out.insert(id.to_ascii_uppercase());
        }
    }
    for child in &element.children {
        if let XmlNode::Element(child) = child {
            collect_para_ids(child, out);
        }
    }
}

/// 関係（.rels）や [Content_Types].xml を読む。
///
/// 文書の部品（`word/` 以下、パッケージ直下の `_rels/.rels`、`[Content_Types].xml`）が読めなければ失敗にする。
/// それ以外の読めない部品は Word も使わないので飛ばす。macOS の Finder で圧縮し直した DOCX には、
/// `__MACOSX/word/_rels/._document.xml.rels` のような AppleDouble のバイナリが入っており、
/// これを理由に変換全体を失敗させていた。
fn parse_package_xml(path: &str, bytes: &[u8]) -> Result<Option<DocxXmlDocument>, String> {
    let required =
        path.starts_with("word/") || path == "_rels/.rels" || path == "[Content_Types].xml";
    let xml = match std::str::from_utf8(bytes) {
        Ok(xml) => xml,
        Err(error) if required => return Err(format!("{path} is not UTF-8 XML: {error}")),
        Err(_) => return Ok(None),
    };
    match DocxXmlDocument::parse(xml) {
        Ok(doc) => Ok(Some(doc)),
        Err(error) if required => Err(format!("cannot inspect {path}: {error}")),
        Err(_) => Ok(None),
    }
}

fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn attribute<'a>(element: &'a XmlElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| local_name(&attribute.name) == name)
        .map(|attribute| attribute.value.as_str())
}

fn is_comment_relationship(relation_type: &str) -> bool {
    relation_type
        .rsplit('/')
        .next()
        .is_some_and(|name| name.starts_with("comment"))
}

fn resolve_relationship_target(rels_path: &str, target: &str) -> Result<String, String> {
    let source = if let Some((folder, file)) = rels_path.rsplit_once("/_rels/") {
        format!("{folder}/{}", file.trim_end_matches(".rels"))
    } else if let Some(file) = rels_path.strip_prefix("_rels/") {
        file.trim_end_matches(".rels").to_string()
    } else {
        return Err(format!("invalid relationships path: {rels_path}"));
    };
    let joined = if target.starts_with('/') {
        target.trim_start_matches('/').to_string()
    } else {
        let folder = source.rsplit_once('/').map_or("", |(folder, _)| folder);
        format!("{folder}/{target}")
    };
    let mut components = Vec::new();
    for component in joined.split('/') {
        match component {
            "" | "." => {}
            ".." if !components.is_empty() => {
                components.pop();
            }
            ".." => return Err(format!("relationship escapes DOCX package: {target}")),
            component => components.push(component),
        }
    }
    Ok(components.join("/"))
}

fn namespace_for(
    element: &XmlElement,
    inherited: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut namespace = inherited.clone();
    for attribute in &element.attributes {
        if attribute.name == "xmlns" {
            namespace.insert(String::new(), attribute.value.clone());
        } else if let Some(prefix) = attribute.name.strip_prefix("xmlns:") {
            namespace.insert(prefix.to_string(), attribute.value.clone());
        }
    }
    namespace
}

fn is_word(element: &XmlElement, namespace: &HashMap<String, String>) -> bool {
    let prefix = element
        .name
        .split_once(':')
        .map_or("", |(prefix, _)| prefix);
    namespace
        .get(prefix)
        .is_some_and(|uri| uri == WORD_NS || uri == STRICT_WORD_NS)
}

fn is_office_word_annotation(element: &XmlElement, namespace: &HashMap<String, String>) -> bool {
    let prefix = element
        .name
        .split_once(':')
        .map_or("", |(prefix, _)| prefix);
    namespace.get(prefix).is_some_and(|uri| {
        uri == WORD_NS
            || uri == STRICT_WORD_NS
            || uri.starts_with("http://schemas.microsoft.com/office/word/")
    })
}

fn is_comment_name(local: &str) -> bool {
    matches!(
        local,
        "comment"
            | "commentRangeStart"
            | "commentRangeEnd"
            | "commentReference"
            | "commentEx"
            | "commentExtensible"
            | "commentId"
    )
}

fn is_revision_name(local: &str) -> bool {
    matches!(
        local,
        "ins"
            | "del"
            | "moveFrom"
            | "moveTo"
            | "moveFromRangeStart"
            | "moveFromRangeEnd"
            | "moveToRangeStart"
            | "moveToRangeEnd"
            | "pPrChange"
            | "rPrChange"
            | "sectPrChange"
            | "tblPrChange"
            | "tblGridChange"
            | "trPrChange"
            | "tcPrChange"
            | "tblPrExChange"
            | "numberingChange"
            | "cellIns"
            | "cellDel"
            | "cellMerge"
            | "customXmlInsRangeStart"
            | "customXmlInsRangeEnd"
            | "customXmlDelRangeStart"
            | "customXmlDelRangeEnd"
            | "customXmlMoveFromRangeStart"
            | "customXmlMoveFromRangeEnd"
            | "customXmlMoveToRangeStart"
            | "customXmlMoveToRangeEnd"
    )
}

fn is_comment_part(root: &XmlElement) -> bool {
    let name = local_name(&root.name);
    let namespace = namespace_for(root, &HashMap::new());
    is_office_word_annotation(root, &namespace)
        && matches!(
            name,
            "comments" | "commentsEx" | "commentsIds" | "commentsExtensible"
        )
}

fn row_or_cell_deleted(element: &XmlElement, namespace: &HashMap<String, String>) -> bool {
    let property_name = match local_name(&element.name) {
        "tr" => "trPr",
        "tc" => "tcPr",
        _ => return false,
    };
    for child in &element.children {
        let XmlNode::Element(properties) = child else {
            continue;
        };
        let properties_ns = namespace_for(properties, namespace);
        if !is_word(properties, &properties_ns) || local_name(&properties.name) != property_name {
            continue;
        }
        for marker in &properties.children {
            let XmlNode::Element(marker) = marker else {
                continue;
            };
            let marker_ns = namespace_for(marker, &properties_ns);
            if is_word(marker, &marker_ns) && matches!(local_name(&marker.name), "del" | "cellDel")
            {
                return true;
            }
        }
    }
    false
}

fn has_deleted_paragraph_mark(element: &XmlElement, namespace: &HashMap<String, String>) -> bool {
    if local_name(&element.name) != "p" || !is_word(element, namespace) {
        return false;
    }
    element.children.iter().any(|node| {
        let XmlNode::Element(properties) = node else {
            return false;
        };
        let properties_ns = namespace_for(properties, namespace);
        if local_name(&properties.name) != "pPr" || !is_word(properties, &properties_ns) {
            return false;
        }
        properties.children.iter().any(|node| {
            let XmlNode::Element(run_properties) = node else {
                return false;
            };
            let run_ns = namespace_for(run_properties, &properties_ns);
            local_name(&run_properties.name) == "rPr"
                && is_word(run_properties, &run_ns)
                && run_properties.children.iter().any(|node| {
                    let XmlNode::Element(marker) = node else {
                        return false;
                    };
                    let marker_ns = namespace_for(marker, &run_ns);
                    // 移動元の段落記号（moveFrom）も、承諾すると段落記号が無くなる。
                    matches!(local_name(&marker.name), "del" | "moveFrom")
                        && is_word(marker, &marker_ns)
                })
        })
    })
}

fn transform_element(
    element: &mut XmlElement,
    inherited: &HashMap<String, String>,
    include_comments: bool,
    include_revisions: bool,
    changed: &mut bool,
) -> Result<(), String> {
    let namespace = namespace_for(element, inherited);
    let mut transformed = Vec::with_capacity(element.children.len());
    // 段落記号が削除された段落。後ろの段落に連結するまでここで待たせる。
    let mut merging: Vec<XmlElement> = Vec::new();
    for child in element.children.drain(..) {
        let XmlNode::Element(mut child) = child else {
            transformed.push(child);
            continue;
        };
        let child_ns = namespace_for(&child, &namespace);
        let word = is_word(&child, &child_ns);
        let local = local_name(&child.name);
        if !include_comments
            && is_comment_name(local)
            && is_office_word_annotation(&child, &child_ns)
        {
            *changed = true;
            continue;
        }
        if !include_revisions && word {
            if local == "p" {
                let mark_deleted = has_deleted_paragraph_mark(&child, &child_ns);
                // 中の w:del / w:ins と、段落記号の w:del の印はここで処理される。
                transform_element(
                    &mut child,
                    &namespace,
                    include_comments,
                    include_revisions,
                    changed,
                )?;
                if mark_deleted {
                    *changed = true;
                    merging.push(child);
                } else {
                    merge_paragraphs(&mut child, merging.drain(..));
                    transformed.push(XmlNode::Element(child));
                }
                continue;
            }
            // 丸ごと消える要素は、段落の連結を妨げない。中の範囲の印のうち相方が外にあるものは、その位置へ寄せて残す。
            if row_or_cell_deleted(&child, &child_ns)
                || matches!(local, "del" | "moveFrom" | "cellDel")
            {
                *changed = true;
                keep_hoisted(
                    &child,
                    &child_ns,
                    include_comments,
                    &mut merging,
                    &mut transformed,
                );
                continue;
            }
            if let Some(last) = merging.last_mut() {
                if is_range_markup(local) {
                    last.children.push(XmlNode::Element(child));
                    continue;
                }
            }
            if matches!(local, "ins" | "moveTo") {
                transformed.extend(flush_unmergeable(&mut merging));
                *changed = true;
                transform_element(
                    &mut child,
                    &namespace,
                    include_comments,
                    include_revisions,
                    changed,
                )?;
                let declarations: Vec<_> = child
                    .attributes
                    .iter()
                    .filter(|attribute| {
                        attribute.name == "xmlns" || attribute.name.starts_with("xmlns:")
                    })
                    .cloned()
                    .collect();
                for node in &mut child.children {
                    let XmlNode::Element(promoted) = node else {
                        continue;
                    };
                    for declaration in &declarations {
                        if !promoted
                            .attributes
                            .iter()
                            .any(|attribute| attribute.name == declaration.name)
                        {
                            promoted.attributes.push(declaration.clone());
                        }
                    }
                }
                transformed.extend(child.children);
                continue;
            }
            if is_revision_name(local) {
                *changed = true;
                continue;
            }
        }
        transform_element(
            &mut child,
            &namespace,
            include_comments,
            include_revisions,
            changed,
        )?;
        if !include_revisions && word && lost_all_rows_or_cells(&child, &child_ns) {
            // 行をすべて削除した表・セルをすべて削除した行は、要素ごと消す（空の w:tbl / w:tr は不正）。寄せてあった
            // 範囲の印のうち相方が外にあるものは、さらに外へ寄せる。
            *changed = true;
            keep_hoisted(
                &child,
                &child_ns,
                include_comments,
                &mut merging,
                &mut transformed,
            );
            continue;
        }
        transformed.extend(flush_unmergeable(&mut merging));
        transformed.push(XmlNode::Element(child));
    }
    transformed.extend(flush_unmergeable(&mut merging));
    element.children = transformed;
    Ok(())
}

/// 消す要素 `removed` の中にある範囲の印のうち、相方（始まりと終わり）が外にあるもの（文書の順）。
///
/// Word は削除を承諾しても範囲の印を消さず、削除した位置へ寄せる。片方だけ消すと、ブックマーク（相互参照の参照先）や
/// コメントの範囲が壊れる。中で完結している印は、中身と一緒に消えてよい。
fn unbalanced_range_markup(
    removed: &XmlElement,
    namespace: &HashMap<String, String>,
) -> Vec<XmlElement> {
    fn pair(local: &str) -> Option<(&'static str, bool)> {
        match local {
            "bookmarkStart" => Some(("bookmark", true)),
            "bookmarkEnd" => Some(("bookmark", false)),
            "commentRangeStart" => Some(("comment", true)),
            "commentRangeEnd" => Some(("comment", false)),
            "permStart" => Some(("perm", true)),
            "permEnd" => Some(("perm", false)),
            _ => None,
        }
    }
    fn collect(
        element: &XmlElement,
        inherited: &HashMap<String, String>,
        out: &mut Vec<XmlElement>,
    ) {
        for child in &element.children {
            let XmlNode::Element(child) = child else {
                continue;
            };
            let namespace = namespace_for(child, inherited);
            if is_word(child, &namespace) && pair(local_name(&child.name)).is_some() {
                out.push(child.clone());
            }
            collect(child, &namespace, out);
        }
    }
    let mut markup = Vec::new();
    collect(removed, namespace, &mut markup);
    let key = |element: &XmlElement| {
        let (kind, start) = pair(local_name(&element.name)).expect("range markup");
        (
            kind,
            start,
            attribute(element, "id").unwrap_or_default().to_string(),
        )
    };
    let present: HashSet<_> = markup.iter().map(key).collect();
    markup
        .into_iter()
        .filter(|element| {
            let (kind, start, id) = key(element);
            !present.contains(&(kind, !start, id))
        })
        .collect()
}

/// 消す要素の中の、相方が外にある範囲の印を、消した位置に残す（段落の連結を待っていれば、その段落へ）。コメントを外す
/// 指定のときは、コメントの範囲の印は残さない。
fn keep_hoisted(
    removed: &XmlElement,
    namespace: &HashMap<String, String>,
    include_comments: bool,
    merging: &mut [XmlElement],
    transformed: &mut Vec<XmlNode>,
) {
    for markup in unbalanced_range_markup(removed, namespace) {
        if !include_comments && is_comment_name(local_name(&markup.name)) {
            continue;
        }
        match merging.last_mut() {
            Some(last) => last.children.push(XmlNode::Element(markup)),
            None => transformed.push(XmlNode::Element(markup)),
        }
    }
}

/// 段落記号が削除された段落の中身を、残る段落 `target` の先頭（`w:pPr` の後ろ）へ順に差し込む。
///
/// 連結後の段落の書式は、残る段落記号を持つ `target` の `w:pPr` に従う（ECMA-376 17.13.5.15）。
fn merge_paragraphs(target: &mut XmlElement, merged: impl Iterator<Item = XmlElement>) {
    let mut contents: Vec<XmlNode> = merged
        .flat_map(|paragraph| paragraph.children)
        .filter(
            |node| !matches!(node, XmlNode::Element(element) if local_name(&element.name) == "pPr"),
        )
        .collect();
    if contents.is_empty() {
        return;
    }
    // 整形された XML では w:pPr の前に空白のテキストノードがあるので、「最初の w:pPr 以外」ではなく
    // w:pPr の直後に差し込む（w:pPr は段落の先頭の要素でなければならない）。
    let insert_at = target
        .children
        .iter()
        .position(
            |node| matches!(node, XmlNode::Element(element) if local_name(&element.name) == "pPr"),
        )
        .map_or(0, |index| index + 1);
    contents.extend(target.children.drain(insert_at..));
    target.children.extend(contents);
}

/// 後ろに連結できる段落が無い（表の直前・セルや本文の末尾）ときの始末。
///
/// Word でもこの位置の段落記号は消えないので、最後の段落を残し、それより前の段落をそこへ連結する。
fn flush_unmergeable(merging: &mut Vec<XmlElement>) -> Option<XmlNode> {
    let mut last = merging.pop()?;
    merge_paragraphs(&mut last, merging.drain(..));
    Some(XmlNode::Element(last))
}

/// 変更履歴を承諾した結果、行を 1 つも持たなくなった表・セルを 1 つも持たなくなった行か。
///
/// content control（w:sdt）や w:customXml で包まれた行・セルも、行・セルがあるものとして数える。
fn lost_all_rows_or_cells(element: &XmlElement, namespace: &HashMap<String, String>) -> bool {
    let required = match local_name(&element.name) {
        "tbl" => "tr",
        "tr" => "tc",
        _ => return false,
    };
    !element.children.iter().any(|node| {
        let XmlNode::Element(child) = node else {
            return false;
        };
        let child_ns = namespace_for(child, namespace);
        is_word(child, &child_ns)
            && (local_name(&child.name) == required
                || matches!(local_name(&child.name), "sdt" | "customXml"))
    })
}

/// 段落の間にも中にも置ける範囲の印。段落の連結を妨げない。
fn is_range_markup(local: &str) -> bool {
    matches!(
        local,
        "bookmarkStart"
            | "bookmarkEnd"
            | "commentRangeStart"
            | "commentRangeEnd"
            | "permStart"
            | "permEnd"
            | "proofErr"
    )
}

fn has_annotation(
    element: &XmlElement,
    inherited: &HashMap<String, String>,
    include_comments: bool,
    include_revisions: bool,
) -> bool {
    let namespace = namespace_for(element, inherited);
    let local = local_name(&element.name);
    if (!include_comments
        && is_comment_name(local)
        && is_office_word_annotation(element, &namespace))
        || (!include_revisions && is_word(element, &namespace) && is_revision_name(local))
    {
        return true;
    }
    element.children.iter().any(|node| match node {
        XmlNode::Element(child) => {
            has_annotation(child, &namespace, include_comments, include_revisions)
        }
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::HashMap;
    use std::io::{Cursor, Read, Write};
    use zip::write::SimpleFileOptions;
    use zip::{ZipArchive, ZipWriter};

    fn package(parts: &[(&str, &str)]) -> Vec<u8> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        for (path, content) in parts {
            zip.start_file(path, SimpleFileOptions::default()).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn parts(data: &[u8]) -> HashMap<String, String> {
        let mut archive = ZipArchive::new(Cursor::new(data)).unwrap();
        let mut result = HashMap::new();
        for index in 0..archive.len() {
            let mut part = archive.by_index(index).unwrap();
            let mut value = String::new();
            part.read_to_string(&mut value).unwrap();
            result.insert(part.name().unwrap().into_owned(), value);
        }
        result
    }

    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    const R: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
    const CT: &str = "http://schemas.openxmlformats.org/package/2006/content-types";

    fn fixture(unrelated: &str) -> Vec<u8> {
        let document = format!(
            r#"<w:document xmlns:w="{W}"><w:body><w:p><w:ins w:id="1"><w:r><w:t>new</w:t></w:r></w:ins><w:del w:id="2"><w:r><w:delText>old</w:delText></w:r></w:del><w:commentRangeStart w:id="9"/><w:r><w:commentReference w:id="9"/></w:r><w:commentRangeEnd w:id="9"/></w:p><w:tbl><w:tr><w:trPr><w:del w:id="3"/></w:trPr><w:tc><w:p><w:r><w:t>deleted row</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#
        );
        let header = format!(
            r#"<w:hdr xmlns:w="{W}"><w:p><w:moveFrom w:id="4"><w:r><w:t>from</w:t></w:r></w:moveFrom><w:moveTo w:id="5"><w:r><w:t>to</w:t></w:r></w:moveTo><w:commentRangeStart w:id="9"/></w:p></w:hdr>"#
        );
        let footer = format!(
            r#"<w:ftr xmlns:w="{W}"><w:p><w:r><w:rPr><w:rPrChange w:id="6"/></w:rPr><w:t>footer</w:t></w:r></w:p></w:ftr>"#
        );
        let footnote = format!(
            r#"<w:footnotes xmlns:w="{W}"><w:footnote w:id="1"><w:p><w:del w:id="7"><w:r><w:delText>note old</w:delText></w:r></w:del><w:ins w:id="8"><w:r><w:t>note new</w:t></w:r></w:ins></w:p></w:footnote></w:footnotes>"#
        );
        let endnote = format!(
            r#"<w:endnotes xmlns:w="{W}"><w:endnote w:id="1"><w:p><w:commentReference w:id="9"/></w:p></w:endnote></w:endnotes>"#
        );
        let comments = format!(
            r#"<w:comments xmlns:w="{W}"><w:comment w:id="9"><w:p><w:r><w:t>anchored</w:t></w:r></w:p></w:comment><w:comment w:id="99"><w:p><w:ins w:id="10"><w:r><w:t>orphan too</w:t></w:r></w:ins></w:p></w:comment></w:comments>"#
        );
        let rels = format!(
            r#"<Relationships xmlns="{R}"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/></Relationships>"#
        );
        let comment_rels = format!(r#"<Relationships xmlns="{R}"/>"#);
        let ct = format!(
            r#"<Types xmlns="{CT}"><Override PartName="/word/comments.xml" ContentType="comments"/><Override PartName="/word/header1.xml" ContentType="header"/></Types>"#
        );
        let unrelated_xml = format!("<data><comment>{unrelated}</comment></data>");
        package(&[
            ("word/document.xml", &document),
            ("word/header1.xml", &header),
            ("word/footer1.xml", &footer),
            ("word/footnotes.xml", &footnote),
            ("word/endnotes.xml", &endnote),
            ("word/comments.xml", &comments),
            ("word/_rels/comments.xml.rels", &comment_rels),
            ("word/_rels/document.xml.rels", &rels),
            ("[Content_Types].xml", &ct),
            ("customXml/item1.xml", &unrelated_xml),
        ])
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn all_flag_combinations_are_idempotent_and_preserve_unrelated_parts(
            unrelated in "[a-z]{0,24}",
        ) {
            let source = fixture(&unrelated);
            for include_comments in [false, true] {
                for include_revisions in [false, true] {
                    let first = futures::executor::block_on(transform_docx_annotations(
                        &source, include_comments, include_revisions,
                    )).unwrap();
                    let second = futures::executor::block_on(transform_docx_annotations(
                        &first, include_comments, include_revisions,
                    )).unwrap();
                    if include_comments && include_revisions {
                        prop_assert_eq!(&first, &source);
                    }
                    let output = parts(&first);
                    prop_assert_eq!(parts(&second), output.clone());
                    prop_assert_eq!(output["customXml/item1.xml"].as_str(), format!("<data><comment>{unrelated}</comment></data>"));
                    prop_assert_eq!(output.contains_key("word/comments.xml"), include_comments);
                    prop_assert_eq!(output.contains_key("word/_rels/comments.xml.rels"), include_comments);
                    if include_comments {
                        prop_assert!(output["word/comments.xml"].contains(r#"<w:comment w:id="99">"#));
                    }
                    let rels = &output["word/_rels/document.xml.rels"];
                    prop_assert_eq!(rels.contains("relationships/comments"), include_comments);
                    let content_types = &output["[Content_Types].xml"];
                    prop_assert_eq!(content_types.contains("/word/comments.xml"), include_comments);
                    if include_comments && !include_revisions {
                        prop_assert!(output["word/comments.xml"].contains("orphan too"));
                        prop_assert!(!output["word/comments.xml"].contains("<w:ins"));
                    }
                    let body = &output["word/document.xml"];
                    prop_assert_eq!(body.contains("commentReference"), include_comments);
                    prop_assert_eq!(body.contains("<w:ins"), include_revisions);
                    if !include_revisions {
                        prop_assert!(body.contains("new"));
                        prop_assert!(!body.contains("old"));
                        prop_assert!(!body.contains("deleted row"));
                        prop_assert!(output["word/header1.xml"].contains("to"));
                        prop_assert!(!output["word/header1.xml"].contains("from"));
                        prop_assert!(output["word/footnotes.xml"].contains("note new"));
                        prop_assert!(!output["word/footnotes.xml"].contains("note old"));
                    }
                }
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]
        #[test]
        fn mixed_annotation_sequences_compose_and_keep_expected_text(
            fragments in proptest::collection::vec((0_u8..8, any::<bool>(), "[a-z]{1,8}"), 1..20),
            first_comments in any::<bool>(),
            first_revisions in any::<bool>(),
            second_comments in any::<bool>(),
            second_revisions in any::<bool>(),
        ) {
            let mut body = String::new();
            for (index, (kind, mark_deleted, text)) in fragments.iter().enumerate() {
                let value = format!("v{index}_{text}");
                let run = format!("<w:r><w:t>{value}</w:t></w:r>");
                let deleted_run = format!("<w:r><w:delText>{value}</w:delText></w:r>");
                let comment = format!("<w:commentReference w:id=\"{}\"/>", 1000 + index);
                let content = match kind {
                    0 => run,
                    1 => format!("<w:ins w:id=\"{index}\">{run}</w:ins>"),
                    2 => format!("<w:del w:id=\"{index}\">{deleted_run}</w:del>"),
                    3 => format!("<w:moveFrom w:id=\"{index}\">{deleted_run}</w:moveFrom>"),
                    4 => format!("<w:moveTo w:id=\"{index}\">{run}</w:moveTo>"),
                    5 => format!("{run}{comment}"),
                    6 => format!("<w:ins w:id=\"{index}\">{run}{comment}</w:ins>"),
                    _ => format!("<w:del w:id=\"{index}\">{deleted_run}{comment}</w:del>"),
                };
                let properties = if *mark_deleted {
                    format!("<w:pPr><w:rPr><w:del w:id=\"{}\"/></w:rPr></w:pPr>", 2000 + index)
                } else {
                    String::new()
                };
                body.push_str(&format!("<w:p>{properties}{content}</w:p>"));
            }
            let document = format!(r#"<w:document xmlns:w="{W}"><w:body>{body}</w:body></w:document>"#);
            let source = package(&[("word/document.xml", &document)]);
            let first = futures::executor::block_on(transform_docx_annotations(
                &source, first_comments, first_revisions,
            )).unwrap();
            let sequential = futures::executor::block_on(transform_docx_annotations(
                &first, second_comments, second_revisions,
            )).unwrap();
            let keep_comments = first_comments && second_comments;
            let keep_revisions = first_revisions && second_revisions;
            let direct = futures::executor::block_on(transform_docx_annotations(
                &source, keep_comments, keep_revisions,
            )).unwrap();
            prop_assert_eq!(parts(&sequential), parts(&direct));

            let output = parts(&direct);
            let xml = &output["word/document.xml"];
            // 段落記号が削除された段落は後ろの段落へ連結される（末尾の段落は連結先が無いので残る）。
            let merged = fragments[..fragments.len() - 1].iter().filter(|(_, deleted, _)| *deleted).count();
            let expected_paragraphs = if keep_revisions { fragments.len() } else { fragments.len() - merged };
            prop_assert_eq!(paragraph_texts(xml).len(), expected_paragraphs);
            let mut previous = 0;
            for (index, (kind, _, text)) in fragments.iter().enumerate() {
                let value = format!("v{index}_{text}");
                let text_remains = keep_revisions || !matches!(kind, 2 | 3 | 7);
                prop_assert_eq!(xml.contains(&value), text_remains, "fragment {}: kind={}", index, kind);
                if let Some(position) = xml.find(&value) {
                    prop_assert!(position >= previous, "fragment {} keeps document order", index);
                    previous = position;
                }
                if matches!(kind, 5..=7) {
                    let marker = format!("w:id=\"{}\"", 1000 + index);
                    let marker_remains = keep_comments && (keep_revisions || *kind != 7);
                    prop_assert_eq!(xml.contains(&marker), marker_remains, "comment {}: kind={}", index, kind);
                }
            }
        }
    }

    fn paragraph_texts(xml: &str) -> Vec<String> {
        let doc = DocxXmlDocument::parse(xml).unwrap();
        let mut result = Vec::new();
        fn collect(element: &XmlElement, result: &mut Vec<String>) {
            if local_name(&element.name) == "p" {
                let mut text = String::new();
                fn texts(element: &XmlElement, text: &mut String) {
                    for node in &element.children {
                        match node {
                            XmlNode::Element(child) => texts(child, text),
                            XmlNode::Text(value) => text.push_str(value),
                            _ => {}
                        }
                    }
                }
                texts(element, &mut text);
                result.push(text);
                return;
            }
            for node in &element.children {
                if let XmlNode::Element(child) = node {
                    collect(child, result);
                }
            }
        }
        collect(doc.root(), &mut result);
        result
    }

    /// Word で本文の段落記号を削除すると、その段落は次の段落に連結される（ECMA-376 17.13.5.15）。
    /// 変更履歴を統合したコピーでは、連結後の段落は残る段落記号（後ろの段落）の書式を持つ。
    /// 以前はここで失敗させており、案件チャットへの「変更提案統合」添付が失敗していた（jlsi/lawgue 案件チャット添付）。
    #[test]
    fn accepting_deleted_paragraph_marks_merges_into_following_paragraph() {
        let deleted_mark =
            r#"<w:pPr><w:jc w:val="center"/><w:rPr><w:del w:id="1" w:author="a"/></w:rPr></w:pPr>"#;
        let xml = format!(
            r#"<w:document xmlns:w="{W}"><w:body><w:p>{deleted_mark}<w:r><w:t>a</w:t></w:r><w:del w:id="2"><w:r><w:delText>x</w:delText></w:r></w:del></w:p><w:p>{deleted_mark}<w:del w:id="3"><w:r><w:delText>y</w:delText></w:r></w:del><w:ins w:id="4"><w:r><w:t>b</w:t></w:r></w:ins></w:p><w:p><w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="3"/></w:numPr></w:pPr><w:r><w:t>c</w:t></w:r></w:p><w:p><w:r><w:t>d</w:t></w:r></w:p></w:body></w:document>"#
        );
        let source = package(&[("word/document.xml", &xml)]);
        for include_comments in [false, true] {
            let result = futures::executor::block_on(transform_docx_annotations(
                &source,
                include_comments,
                false,
            ))
            .expect("deleted paragraph marks are accepted");
            let document = &parts(&result)["word/document.xml"];
            assert_eq!(paragraph_texts(document), vec!["abc", "d"]);
            assert!(!document.contains("<w:del"));
            assert!(!document.contains("<w:ins"));
            assert!(
                !document.contains("w:jc"),
                "merged paragraph keeps the surviving mark's properties"
            );
            assert!(document.contains(r#"<w:p><w:pPr><w:numPr>"#));
        }
        let kept =
            futures::executor::block_on(transform_docx_annotations(&source, true, true)).unwrap();
        assert_eq!(kept, source);
    }

    /// 表の直前・セルや本文の末尾など、後ろに連結できる段落が無い段落記号は Word でも消えないので、印だけ外して段落を残す。
    /// それより前の削除された段落記号は、残した段落へ連結する。
    #[test]
    fn deleted_paragraph_mark_without_following_paragraph_keeps_the_paragraph() {
        let deleted_mark = r#"<w:pPr><w:rPr><w:del w:id="1"/></w:rPr></w:pPr>"#;
        let xml = format!(
            r#"<w:document xmlns:w="{W}"><w:body><w:p>{deleted_mark}<w:r><w:t>before table</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p>{deleted_mark}<w:r><w:t>last in cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p>{deleted_mark}<w:r><w:t>la</w:t></w:r></w:p><w:p>{deleted_mark}<w:r><w:t>st</w:t></w:r></w:p><w:sectPr/></w:body></w:document>"#
        );
        let source = package(&[("word/document.xml", &xml)]);
        let result = futures::executor::block_on(transform_docx_annotations(&source, true, false))
            .expect("unmergeable paragraph marks are kept");
        let document = &parts(&result)["word/document.xml"];
        assert_eq!(
            paragraph_texts(document),
            vec!["before table", "last in cell", "last"]
        );
        assert!(!document.contains("<w:del"));
    }

    /// 行をすべて削除した表は表ごと消す（行の無い w:tbl は不正）。消えた表は段落の連結も妨げない。
    #[test]
    fn table_whose_rows_are_all_deleted_is_removed() {
        let xml = format!(
            r#"<w:document xmlns:w="{W}"><w:body><w:p><w:pPr><w:rPr><w:del w:id="1"/></w:rPr></w:pPr><w:r><w:t>a</w:t></w:r></w:p><w:tbl><w:tblGrid><w:gridCol/></w:tblGrid><w:tr><w:trPr><w:del w:id="2"/></w:trPr><w:tc><w:p><w:del w:id="3"><w:r><w:delText>x</w:delText></w:r></w:del></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>b</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:tcPr><w:cellDel w:id="4"/></w:tcPr><w:p/></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>kept</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>c</w:t></w:r></w:p></w:body></w:document>"#
        );
        let source = package(&[("word/document.xml", &xml)]);
        let result =
            futures::executor::block_on(transform_docx_annotations(&source, true, false)).unwrap();
        let document = &parts(&result)["word/document.xml"];
        assert_eq!(paragraph_texts(document), vec!["ab", "kept", "c"]);
        assert_eq!(document.matches("<w:tbl>").count(), 1, "{document}");
        assert_eq!(
            document.matches("<w:tr>").count(),
            1,
            "the row whose cells are all deleted is removed: {document}"
        );
    }

    /// 整形された XML（要素の間に空白のテキストノードがある）でも、連結後の w:pPr は段落の先頭に残る。
    #[test]
    fn merging_indented_paragraphs_keeps_properties_first() {
        let xml = format!(
            "<w:document xmlns:w=\"{W}\"><w:body>\n  <w:p>\n    <w:pPr><w:rPr><w:del w:id=\"1\"/></w:rPr></w:pPr>\n    <w:r><w:t>a</w:t></w:r>\n  </w:p>\n  <w:p>\n    <w:pPr><w:jc w:val=\"center\"/></w:pPr>\n    <w:r><w:t>b</w:t></w:r>\n  </w:p>\n</w:body></w:document>"
        );
        let source = package(&[("word/document.xml", &xml)]);
        let result =
            futures::executor::block_on(transform_docx_annotations(&source, true, false)).unwrap();
        let document = &parts(&result)["word/document.xml"];
        assert_eq!(
            paragraph_texts(document)
                .iter()
                .map(|text| text.trim().replace(char::is_whitespace, ""))
                .collect::<Vec<_>>(),
            vec!["ab"]
        );
        let paragraph = &document[document.find("<w:p>").unwrap()..];
        let first_element = paragraph["<w:p>".len()..].trim_start();
        assert!(first_element.starts_with("<w:pPr><w:jc"), "{document}");
    }

    /// macOS の Finder で圧縮し直した DOCX には `__MACOSX/` に AppleDouble のバイナリが入る。
    /// 文書の部品ではないので、読めなくても変換を失敗させない。
    #[test]
    fn unreadable_parts_outside_the_document_are_ignored() {
        let xml = format!(
            r#"<w:document xmlns:w="{W}"><w:body><w:p><w:commentReference w:id="1"/><w:ins w:id="2"><w:r><w:t>kept</w:t></w:r></w:ins></w:p></w:body></w:document>"#
        );
        let apple_double = "\u{0}\u{5}\u{16}\u{7}\u{0}\u{2}\u{0}\u{0}Mac OS X        \u{0}\u{2}\u{0}\u{0}\u{0}\u{9}\u{0}\u{0}\u{0}2\u{0}\u{0}\u{fffd}";
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        for (path, bytes) in [
            ("word/document.xml", xml.as_bytes().to_vec()),
            (
                "__MACOSX/word/_rels/._document.xml.rels",
                [apple_double.as_bytes(), &[0xff, 0xfe]].concat(),
            ),
            ("__MACOSX/._[Content_Types].xml", vec![0xff, 0x00, 0x01]),
        ] {
            zip.start_file(path, SimpleFileOptions::default()).unwrap();
            zip.write_all(&bytes).unwrap();
        }
        let source = zip.finish().unwrap().into_inner();
        for (comments, revisions) in [(false, false), (false, true), (true, false)] {
            let result = futures::executor::block_on(transform_docx_annotations(
                &source, comments, revisions,
            ))
            .expect("AppleDouble files do not break the transform");
            let mut archive = ZipArchive::new(Cursor::new(result)).unwrap();
            let mut document = String::new();
            archive
                .by_name("word/document.xml")
                .unwrap()
                .read_to_string(&mut document)
                .unwrap();
            assert!(document.contains("kept"));
        }
        // 文書の部品が読めないときは、これまでどおり失敗にする。
        let broken = package(&[
            ("word/document.xml", &xml),
            ("word/_rels/document.xml.rels", "<Relationships"),
        ]);
        assert!(
            futures::executor::block_on(transform_docx_annotations(&broken, false, true)).is_err()
        );
    }

    /// 段落の間に挟まる範囲の印（ブックマーク等）は連結を妨げない。
    #[test]
    fn range_markup_between_paragraphs_does_not_block_the_merge() {
        let xml = format!(
            r#"<w:document xmlns:w="{W}"><w:body><w:p><w:pPr><w:rPr><w:del w:id="1"/></w:rPr></w:pPr><w:r><w:t>a</w:t></w:r></w:p><w:bookmarkEnd w:id="5"/><w:p><w:r><w:t>b</w:t></w:r></w:p></w:body></w:document>"#
        );
        let source = package(&[("word/document.xml", &xml)]);
        let result =
            futures::executor::block_on(transform_docx_annotations(&source, true, false)).unwrap();
        let document = &parts(&result)["word/document.xml"];
        assert_eq!(paragraph_texts(document), vec!["ab"]);
        assert!(document.contains(r#"<w:bookmarkEnd w:id="5"/>"#));
    }

    #[test]
    fn promoted_inserted_content_keeps_its_namespace_declarations() {
        let xml = format!(
            r#"<w:document xmlns:w="{W}"><w:body><w:p><w:ins w:id="1" xmlns:x="{W}"><x:r><x:t>new</x:t></x:r></w:ins></w:p></w:body></w:document>"#
        );
        let source = package(&[("word/document.xml", &xml)]);
        let result = futures::executor::block_on(transform_docx_annotations(&source, true, false))
            .expect("inserted content can be promoted");
        let document = &parts(&result)["word/document.xml"];
        assert!(document.contains(&format!(r#"<x:r xmlns:x="{W}">"#)));
    }

    #[test]
    fn strict_wordprocessing_namespace_is_transformed() {
        let xml = format!(
            r#"<w:document xmlns:w="{STRICT_WORD_NS}"><w:body><w:p><w:ins w:id="1"><w:r><w:t>kept</w:t></w:r></w:ins><w:commentReference w:id="2"/></w:p></w:body></w:document>"#
        );
        let source = package(&[("word/document.xml", &xml)]);
        let result = futures::executor::block_on(transform_docx_annotations(&source, false, false))
            .expect("strict review markup is recognized");
        let document = &parts(&result)["word/document.xml"];
        assert!(document.contains("kept"));
        assert!(!document.contains("<w:ins"));
        assert!(!document.contains("commentReference"));
    }

    #[test]
    fn opaque_media_part_bytes_are_preserved() {
        let xml = format!(
            r#"<w:document xmlns:w="{W}"><w:body><w:p><w:commentReference w:id="1"/></w:p></w:body></w:document>"#
        );
        let media = [0_u8, 1, 2, 254, 255];
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("word/document.xml", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
        zip.start_file("word/media/image1.png", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&media).unwrap();
        let source = zip.finish().unwrap().into_inner();
        let result = futures::executor::block_on(transform_docx_annotations(&source, false, true))
            .expect("media does not affect comment removal");
        let mut archive = ZipArchive::new(Cursor::new(result)).unwrap();
        let mut output = Vec::new();
        archive
            .by_name("word/media/image1.png")
            .unwrap()
            .read_to_end(&mut output)
            .unwrap();
        assert_eq!(output, media);
    }

    /// 承諾で本文から参照（commentReference）ごと消えたコメントは、comments.xml とその付随の part からも外す。
    /// Word の「すべて承諾」は、削除した行・セル・文字の中にあったコメントも一緒に消す。残すと本文のどこにも付いて
    /// いないコメントになる（lawgue-backend-rs の原本ダウンロードの PBT で見つけた）。承諾の前から参照の無かった
    /// コメントは、知らない内容として触らない。
    #[test]
    fn accepting_removes_comments_whose_reference_was_deleted() {
        const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
        const W15: &str = "http://schemas.microsoft.com/office/word/2012/wordml";
        const W16CID: &str = "http://schemas.microsoft.com/office/word/2016/wordml/cid";
        const W16CEX: &str = "http://schemas.microsoft.com/office/word/2018/wordml/cex";
        let document = format!(
            r#"<w:document xmlns:w="{W}"><w:body><w:p><w:r><w:t>keep</w:t></w:r><w:commentRangeStart w:id="1"/><w:r><w:t>kept</w:t></w:r><w:commentRangeEnd w:id="1"/><w:r><w:commentReference w:id="1"/></w:r><w:commentRangeStart w:id="2"/><w:del w:id="20" w:author="a"><w:r><w:delText>gone</w:delText></w:r><w:r><w:commentReference w:id="2"/></w:r></w:del><w:commentRangeEnd w:id="2"/></w:p><w:tbl><w:tr><w:trPr><w:del w:id="21" w:author="a"/></w:trPr><w:tc><w:p><w:commentRangeStart w:id="3"/><w:r><w:t>row</w:t></w:r><w:commentRangeEnd w:id="3"/><w:r><w:commentReference w:id="3"/></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>end</w:t></w:r></w:p></w:body></w:document>"#
        );
        let comment = |id: u32, para: &str| {
            format!(
                r#"<w:comment w:id="{id}" w:author="a"><w:p w14:paraId="{para}"><w:r><w:t>c{id}</w:t></w:r></w:p></w:comment>"#
            )
        };
        let comments = format!(
            r#"<w:comments xmlns:w="{W}" xmlns:w14="{W14}">{}{}{}{}</w:comments>"#,
            comment(1, "00000A01"),
            comment(2, "00000A02"),
            comment(3, "00000A03"),
            // 承諾の前から本文に参照の無いコメント。
            comment(4, "00000A04"),
        );
        let extended = format!(
            r#"<w15:commentsEx xmlns:w15="{W15}"><w15:commentEx w15:paraId="00000A01" w15:done="0"/><w15:commentEx w15:paraId="00000A02" w15:done="0"/><w15:commentEx w15:paraId="00000A03" w15:done="0"/><w15:commentEx w15:paraId="00000A04" w15:done="0"/></w15:commentsEx>"#
        );
        let ids = format!(
            r#"<w16cid:commentsIds xmlns:w16cid="{W16CID}"><w16cid:commentId w16cid:paraId="00000A01" w16cid:durableId="11"/><w16cid:commentId w16cid:paraId="00000A02" w16cid:durableId="12"/><w16cid:commentId w16cid:paraId="00000A03" w16cid:durableId="13"/><w16cid:commentId w16cid:paraId="00000A04" w16cid:durableId="14"/></w16cid:commentsIds>"#
        );
        let extensible = format!(
            r#"<w16cex:commentsExtensible xmlns:w16cex="{W16CEX}"><w16cex:commentExtensible w16cex:durableId="11"/><w16cex:commentExtensible w16cex:durableId="12"/><w16cex:commentExtensible w16cex:durableId="13"/><w16cex:commentExtensible w16cex:durableId="14"/></w16cex:commentsExtensible>"#
        );
        let source = package(&[
            ("word/document.xml", &document),
            ("word/comments.xml", &comments),
            ("word/commentsExtended.xml", &extended),
            ("word/commentsIds.xml", &ids),
            ("word/commentsExtensible.xml", &extensible),
        ]);
        let output = parts(
            &futures::executor::block_on(transform_docx_annotations(&source, true, false)).unwrap(),
        );

        let ids_of = |xml: &str, element: &str, attr: &str| -> Vec<String> {
            let doc = DocxXmlDocument::parse(xml).unwrap();
            let mut found = Vec::new();
            fn walk(e: &XmlElement, element: &str, attr: &str, found: &mut Vec<String>) {
                if local_name(&e.name) == element {
                    if let Some(value) = attribute(e, attr) {
                        found.push(value.to_string());
                    }
                }
                for child in &e.children {
                    if let XmlNode::Element(child) = child {
                        walk(child, element, attr, found);
                    }
                }
            }
            walk(doc.root(), element, attr, &mut found);
            found
        };
        assert_eq!(
            ids_of(&output["word/comments.xml"], "comment", "id"),
            ["1", "4"]
        );
        assert_eq!(
            ids_of(&output["word/commentsExtended.xml"], "commentEx", "paraId"),
            ["00000A01", "00000A04"]
        );
        assert_eq!(
            ids_of(&output["word/commentsIds.xml"], "commentId", "durableId"),
            ["11", "14"]
        );
        assert_eq!(
            ids_of(
                &output["word/commentsExtensible.xml"],
                "commentExtensible",
                "durableId"
            ),
            ["11", "14"]
        );
        // 消したコメントの範囲の印も本文に残さない。残るコメントの印はそのまま。
        let document = &output["word/document.xml"];
        assert_eq!(ids_of(document, "commentRangeStart", "id"), ["1"]);
        assert_eq!(ids_of(document, "commentRangeEnd", "id"), ["1"]);
        assert_eq!(ids_of(document, "commentReference", "id"), ["1"]);
    }

    fn ids_of(xml: &str, element: &str) -> Vec<String> {
        let doc = DocxXmlDocument::parse(xml).unwrap();
        let mut found = Vec::new();
        fn walk(e: &XmlElement, element: &str, found: &mut Vec<String>) {
            if local_name(&e.name) == element {
                if let Some(value) = attribute(e, "id") {
                    found.push(value.to_string());
                }
            }
            for child in &e.children {
                if let XmlNode::Element(child) = child {
                    walk(child, element, found);
                }
            }
        }
        walk(doc.root(), element, &mut found);
        found
    }

    /// 削除の中に範囲の印（ブックマーク・コメントの範囲）の片方だけがあるとき、承諾しても印は消さずに削除した位置へ
    /// 寄せる（Word と同じ）。消すと、片方だけが残ってブックマーク（相互参照の参照先）やコメントの範囲が壊れていた
    /// （lawgue-backend-rs の変更提案の承諾の PBT で見つけた）。
    #[test]
    fn accepting_keeps_range_markup_whose_partner_is_outside_the_deletion() {
        let document = format!(
            r#"<w:document xmlns:w="{W}"><w:body><w:p><w:r><w:t>a</w:t></w:r><w:del w:id="10" w:author="a"><w:bookmarkStart w:id="1" w:name="_Ref1"/><w:commentRangeStart w:id="5"/><w:r><w:delText>gone</w:delText></w:r></w:del><w:r><w:t>b</w:t></w:r><w:bookmarkEnd w:id="1"/><w:commentRangeEnd w:id="5"/><w:r><w:commentReference w:id="5"/></w:r></w:p><w:p><w:bookmarkStart w:id="2" w:name="_Ref2"/><w:r><w:t>c</w:t></w:r><w:moveFrom w:id="11" w:author="a"><w:r><w:delText>moved</w:delText></w:r><w:bookmarkEnd w:id="2"/></w:moveFrom></w:p><w:tbl><w:tblGrid><w:gridCol/></w:tblGrid><w:tr><w:trPr><w:del w:id="12" w:author="a"/></w:trPr><w:tc><w:p><w:bookmarkStart w:id="3" w:name="_Ref3"/><w:r><w:t>row</w:t></w:r><w:bookmarkStart w:id="4" w:name="_Ref4"/><w:bookmarkEnd w:id="4"/></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>kept</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>d</w:t></w:r><w:bookmarkEnd w:id="3"/></w:p></w:body></w:document>"#
        );
        let comments = format!(
            r#"<w:comments xmlns:w="{W}"><w:comment w:id="5" w:author="a"><w:p><w:r><w:t>c5</w:t></w:r></w:p></w:comment></w:comments>"#
        );
        let source = package(&[
            ("word/document.xml", &document),
            ("word/comments.xml", &comments),
        ]);
        for include_comments in [true, false] {
            let output = parts(
                &futures::executor::block_on(transform_docx_annotations(
                    &source,
                    include_comments,
                    false,
                ))
                .unwrap(),
            );
            let document = &output["word/document.xml"];
            // 片方が削除の外にある印は残る。中にすべてある印（4）は、中身ごと消えてよい。
            for id in ["1", "2", "3"] {
                assert_eq!(
                    ids_of(document, "bookmarkStart")
                        .iter()
                        .filter(|i| *i == id)
                        .count(),
                    1,
                    "bookmarkStart {id}: {document}"
                );
                assert_eq!(
                    ids_of(document, "bookmarkEnd")
                        .iter()
                        .filter(|i| *i == id)
                        .count(),
                    1,
                    "bookmarkEnd {id}: {document}"
                );
            }
            let starts = ids_of(document, "bookmarkStart");
            let ends = ids_of(document, "bookmarkEnd");
            assert_eq!(
                starts.iter().filter(|i| *i == "4").count(),
                ends.iter().filter(|i| *i == "4").count()
            );
            if include_comments {
                assert_eq!(ids_of(document, "commentRangeStart"), ["5"], "{document}");
                assert_eq!(ids_of(document, "commentRangeEnd"), ["5"]);
                assert_eq!(ids_of(document, "commentReference"), ["5"]);
            }
            assert!(
                !document.contains("gone")
                    && !document.contains("moved")
                    && !document.contains(">row<")
            );
        }
    }

    /// 承諾で本文から参照ごと消えた脚注・文末脚注は、footnotes.xml・endnotes.xml からも外す（Word と同じ）。区切りの
    /// 脚注（separator など）と、承諾の前から参照の無い脚注には触らない。
    #[test]
    fn accepting_removes_notes_whose_reference_was_deleted() {
        let document = format!(
            r#"<w:document xmlns:w="{W}"><w:body><w:p><w:r><w:t>a</w:t></w:r><w:r><w:footnoteReference w:id="1"/></w:r><w:del w:id="10" w:author="a"><w:r><w:delText>x</w:delText></w:r><w:r><w:footnoteReference w:id="2"/></w:r><w:r><w:endnoteReference w:id="7"/></w:r></w:del></w:p><w:tbl><w:tblGrid><w:gridCol/></w:tblGrid><w:tr><w:trPr><w:del w:id="11" w:author="a"/></w:trPr><w:tc><w:p><w:r><w:footnoteReference w:id="3"/></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>kept</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:endnoteReference w:id="8"/></w:r></w:p></w:body></w:document>"#
        );
        let note = |kind: &str, id: i32, extra: &str| {
            format!(
                r#"<w:{kind} w:id="{id}"{extra}><w:p><w:r><w:t>{kind} {id}</w:t></w:r></w:p></w:{kind}>"#
            )
        };
        let footnotes = format!(
            r#"<w:footnotes xmlns:w="{W}">{}{}{}{}{}{}</w:footnotes>"#,
            note("footnote", -1, r#" w:type="separator""#),
            note("footnote", 0, r#" w:type="continuationSeparator""#),
            note("footnote", 1, ""),
            note("footnote", 2, ""),
            note("footnote", 3, ""),
            // 承諾の前から参照の無い脚注。
            note("footnote", 4, ""),
        );
        let endnotes = format!(
            r#"<w:endnotes xmlns:w="{W}">{}{}{}</w:endnotes>"#,
            note("endnote", -1, r#" w:type="separator""#),
            note("endnote", 7, ""),
            note("endnote", 8, ""),
        );
        let source = package(&[
            ("word/document.xml", &document),
            ("word/footnotes.xml", &footnotes),
            ("word/endnotes.xml", &endnotes),
        ]);
        let output = parts(
            &futures::executor::block_on(transform_docx_annotations(&source, true, false)).unwrap(),
        );
        assert_eq!(
            ids_of(&output["word/footnotes.xml"], "footnote"),
            ["-1", "0", "1", "4"]
        );
        assert_eq!(ids_of(&output["word/endnotes.xml"], "endnote"), ["-1", "8"]);
    }
}
