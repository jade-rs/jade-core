//! Conservative import diagnostics, not a certificate of Word compatibility.
use crate::OpenXmlPackage;
use quick_xml::{escape::unescape, events::Event, name::ResolveResult, NsReader};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompatibilityIssue {
    pub part: String,
    pub feature: String,
    pub occurrences: usize,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PartDigest {
    pub part: String,
    pub sha256: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProtectedTextDigest {
    pub feature: String,
    pub sha256: String,
    pub occurrences: usize,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompatibilityReport {
    pub scanned_xml_parts: usize,
    pub main_document_paragraph_count: usize,
    pub main_document_revision_count: usize,
    pub parts: Vec<String>,
    pub xml_part_digests: Vec<PartDigest>,
    pub opaque_part_digests: Vec<PartDigest>,
    pub protected_text_digests: Vec<ProtectedTextDigest>,
    pub issues: Vec<CompatibilityIssue>,
}

impl OpenXmlPackage {
    /// Inventory known import limitations without modifying the package.
    pub fn compatibility_report(&self) -> CompatibilityReport {
        let mut report = CompatibilityReport {
            parts: self.files.keys().cloned().collect(),
            ..Default::default()
        };
        report.parts.sort();
        let mut counts = BTreeMap::new();
        for (part, bytes) in &self.files {
            if part == "word/document.xml" {
                report.protected_text_digests = protected_text_digests(bytes);
                let (paragraphs, revisions, deleted_paragraph_marks, inserted_paragraph_marks) =
                    main_document_structure_counts(bytes);
                report.main_document_paragraph_count = paragraphs;
                report.main_document_revision_count = revisions;
                if deleted_paragraph_marks > 0 {
                    counts.insert(
                        (part.clone(), "deletedParagraphMark".into()),
                        deleted_paragraph_marks,
                    );
                }
                if inserted_paragraph_marks > 0 {
                    counts.insert(
                        (part.clone(), "insertedParagraphMark".into()),
                        inserted_paragraph_marks,
                    );
                }
            }
            if part.ends_with(".xml") || part.ends_with(".rels") {
                report.xml_part_digests.push(PartDigest {
                    part: part.clone(),
                    sha256: format!("{:x}", Sha256::digest(bytes)),
                });
            }
            if !part.ends_with(".xml") {
                if !part.ends_with(".rels") {
                    report.opaque_part_digests.push(PartDigest {
                        part: part.clone(),
                        sha256: format!("{:x}", Sha256::digest(bytes)),
                    });
                }
                continue;
            }
            report.scanned_xml_parts += 1;
            let (encoding, bom) =
                encoding_rs::Encoding::for_bom(bytes).unwrap_or((encoding_rs::UTF_8, 0));
            let (xml, errors) = encoding.decode_without_bom_handling(&bytes[bom..]);
            let mut reader = NsReader::from_str(&xml);
            let mut depth = 0usize;
            let mut inline_drawing_depth = None;
            let mut roots = 0usize;
            let mut invalid = errors;
            loop {
                match reader.read_resolved_event() {
                    Ok((namespace, event @ (Event::Start(_) | Event::Empty(_)))) => {
                        let empty = matches!(event, Event::Empty(_));
                        let (Event::Start(element) | Event::Empty(element)) = event else {
                            unreachable!()
                        };
                        if depth == 0 {
                            roots += 1;
                        }
                        if matches!(namespace, ResolveResult::Bound(ref ns) if matches!(ns.as_ref(),
                            b"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
                                | b"http://purl.oclc.org/ooxml/drawingml/wordprocessingDrawing"))
                            && element.local_name().as_ref() == b"inline"
                            && !empty
                        {
                            inline_drawing_depth = Some(depth + 1);
                        }
                        if inline_drawing_depth.is_some()
                            && matches!(namespace, ResolveResult::Bound(ref ns) if matches!(ns.as_ref(),
                                b"http://schemas.openxmlformats.org/wordprocessingml/2006/main"
                                    | b"http://purl.oclc.org/ooxml/wordprocessingml/main"))
                            && element.local_name().as_ref() == b"txbxContent"
                        {
                            *counts
                                .entry((part.clone(), "inlineTextBox".to_string()))
                                .or_insert(0) += 1;
                        }
                        if !empty {
                            depth += 1;
                        }
                        if let ResolveResult::Bound(ns) = namespace {
                            if let Some(feature) =
                                known_limitation(ns.as_ref(), element.local_name().as_ref())
                            {
                                *counts
                                    .entry((part.clone(), feature.to_string()))
                                    .or_insert(0) += 1;
                            }
                        }
                    }
                    Ok((_, Event::End(_))) => {
                        if inline_drawing_depth == Some(depth) {
                            inline_drawing_depth = None;
                        }
                        depth = depth.saturating_sub(1);
                    }
                    Ok((_, Event::Eof)) => {
                        invalid |= depth != 0 || roots != 1;
                        break;
                    }
                    Err(_) => {
                        invalid = true;
                        break;
                    }
                    _ => {}
                }
            }
            if invalid {
                counts.insert((part.clone(), "unreadableXml".into()), 1);
            }
        }
        report.issues = counts
            .into_iter()
            .map(|((part, feature), occurrences)| CompatibilityIssue {
                part,
                feature,
                occurrences,
            })
            .collect();
        report
            .xml_part_digests
            .sort_by(|left, right| left.part.cmp(&right.part));
        report
            .opaque_part_digests
            .sort_by(|left, right| left.part.cmp(&right.part));
        report
    }
}

fn main_document_structure_counts(bytes: &[u8]) -> (usize, usize, usize, usize) {
    let (encoding, bom) = encoding_rs::Encoding::for_bom(bytes).unwrap_or((encoding_rs::UTF_8, 0));
    let (xml, _) = encoding.decode_without_bom_handling(&bytes[bom..]);
    let mut reader = NsReader::from_str(&xml);
    let (mut paragraphs, mut revisions, mut deleted_paragraph_marks, mut inserted_paragraph_marks) =
        (0, 0, 0, 0);
    // Only the direct p > pPr > rPr ancestry matters here.
    let mut ancestors: Vec<u8> = Vec::new();
    loop {
        match reader.read_resolved_event() {
            Ok((namespace, event @ (Event::Start(_) | Event::Empty(_)))) => {
                let is_start = matches!(event, Event::Start(_));
                let (Event::Start(element) | Event::Empty(element)) = event else {
                    unreachable!()
                };
                let is_word = matches!(namespace, ResolveResult::Bound(ns) if matches!(
                    ns.as_ref(),
                    b"http://schemas.openxmlformats.org/wordprocessingml/2006/main"
                        | b"http://purl.oclc.org/ooxml/wordprocessingml/main"
                ));
                let local_name = element.local_name();
                let name = local_name.as_ref();
                if is_word {
                    match name {
                        b"p" => paragraphs += 1,
                        b"ins" | b"del" | b"moveFrom" | b"moveTo" => revisions += 1,
                        _ => {}
                    }
                    if matches!(name, b"del" | b"ins")
                        && ancestors.len() >= 3
                        && ancestors[ancestors.len() - 1] == 3
                        && ancestors[ancestors.len() - 2] == 2
                        && ancestors[ancestors.len() - 3] == 1
                    {
                        if name == b"del" {
                            deleted_paragraph_marks += 1;
                        } else {
                            inserted_paragraph_marks += 1;
                        }
                    }
                }
                if is_start {
                    ancestors.push(match (is_word, name) {
                        (true, b"p") => 1,
                        (true, b"pPr") => 2,
                        (true, b"rPr") => 3,
                        _ => 0,
                    });
                }
            }
            Ok((_, Event::End(_))) => {
                ancestors.pop();
            }
            Ok((_, Event::Eof)) | Err(_) => break,
            _ => {}
        }
    }
    (
        paragraphs,
        revisions,
        deleted_paragraph_marks,
        inserted_paragraph_marks,
    )
}

fn protected_text_digests(bytes: &[u8]) -> Vec<ProtectedTextDigest> {
    let (encoding, bom) = encoding_rs::Encoding::for_bom(bytes).unwrap_or((encoding_rs::UTF_8, 0));
    let (xml, _) = encoding.decode_without_bom_handling(&bytes[bom..]);
    let mut reader = NsReader::from_str(&xml);
    let mut depth = 0usize;
    let mut active: Option<(usize, &'static str, String)> = None;
    let mut values: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    loop {
        match reader.read_resolved_event() {
            Ok((namespace, Event::Start(element))) => {
                depth += 1;
                if active.is_none() {
                    if let ResolveResult::Bound(ns) = namespace {
                        if matches!(
                            ns.as_ref(),
                            b"http://schemas.openxmlformats.org/wordprocessingml/2006/main"
                                | b"http://purl.oclc.org/ooxml/wordprocessingml/main"
                        ) {
                            let feature = match element.local_name().as_ref() {
                                b"delText" => Some("deletedText"),
                                b"instrText" => Some("fieldInstructionText"),
                                _ => None,
                            };
                            if let Some(feature) = feature {
                                active = Some((depth, feature, String::new()));
                            }
                        }
                    }
                }
            }
            Ok((ResolveResult::Bound(ns), Event::Empty(element))) => {
                if matches!(
                    ns.as_ref(),
                    b"http://schemas.openxmlformats.org/wordprocessingml/2006/main"
                        | b"http://purl.oclc.org/ooxml/wordprocessingml/main"
                ) {
                    let feature = match element.local_name().as_ref() {
                        b"delText" => Some("deletedText"),
                        b"instrText" => Some("fieldInstructionText"),
                        _ => None,
                    };
                    if let Some(feature) = feature {
                        values.entry(feature).or_default().push(String::new());
                    }
                }
            }
            Ok((_, Event::Text(text))) => {
                if let Some((_, _, value)) = &mut active {
                    if let Ok(decoded) = text.decode() {
                        if let Ok(unescaped) = unescape(&decoded) {
                            value.push_str(&unescaped);
                        }
                    }
                }
            }
            Ok((_, Event::GeneralRef(reference))) => {
                if let Some((_, _, value)) = &mut active {
                    if let Ok(name) = reference.decode() {
                        if let Ok(unescaped) = unescape(&format!("&{name};")) {
                            value.push_str(&unescaped);
                        }
                    }
                }
            }
            Ok((_, Event::CData(text))) => {
                if let Some((_, _, value)) = &mut active {
                    if let Ok(decoded) = text.decode() {
                        value.push_str(&decoded);
                    }
                }
            }
            Ok((_, Event::End(_))) => {
                if active.as_ref().is_some_and(|(start, _, _)| *start == depth) {
                    let (_, feature, value) = active.take().unwrap();
                    values.entry(feature).or_default().push(value);
                }
                depth = depth.saturating_sub(1);
            }
            Ok((_, Event::Eof)) | Err(_) => break,
            _ => {}
        }
    }
    values
        .into_iter()
        .map(|(feature, texts)| {
            let mut hasher = Sha256::new();
            for value in &texts {
                hasher.update((value.len() as u64).to_le_bytes());
                hasher.update(value.as_bytes());
            }
            ProtectedTextDigest {
                feature: feature.to_owned(),
                sha256: format!("{:x}", hasher.finalize()),
                occurrences: texts.len(),
            }
        })
        .collect()
}

fn known_limitation(namespace: &[u8], local: &[u8]) -> Option<&'static str> {
    match (namespace, local) {
        (
            b"http://schemas.openxmlformats.org/wordprocessingml/2006/main"
            | b"http://purl.oclc.org/ooxml/wordprocessingml/main",
            b"altChunk",
        ) => Some("altChunk"),
        (
            b"http://schemas.openxmlformats.org/wordprocessingml/2006/main"
            | b"http://purl.oclc.org/ooxml/wordprocessingml/main",
            b"embedRegular" | b"embedBold" | b"embedItalic" | b"embedBoldItalic",
        ) => Some("embeddedFont"),
        (
            b"http://schemas.openxmlformats.org/drawingml/2006/diagram"
            | b"http://purl.oclc.org/ooxml/drawingml/diagram",
            b"relIds",
        ) => Some("smartArt"),
        (b"urn:schemas-microsoft-com:office:office", b"OLEObject") => Some("oleObject"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn package(parts: &[(&str, &str)]) -> OpenXmlPackage {
        OpenXmlPackage {
            files: parts
                .iter()
                .map(|(path, xml)| (path.to_string(), xml.as_bytes().to_vec()))
                .collect::<HashMap<_, _>>(),
            relationships: vec![],
            original_size_bytes: None,
        }
    }

    #[test]
    fn reports_alt_chunks_by_namespace_and_part_with_stable_counts() {
        let xml = r#"<x:document xmlns:x="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><x:body><x:altChunk/><x:altChunk></x:altChunk></x:body></x:document>"#;
        let pkg = package(&[("word/header1.xml", xml), ("word/document.xml", xml)]);
        let original = pkg.files.clone();
        let report = pkg.compatibility_report();
        assert_eq!(
            report.issues,
            vec![
                CompatibilityIssue {
                    part: "word/document.xml".into(),
                    feature: "altChunk".into(),
                    occurrences: 2
                },
                CompatibilityIssue {
                    part: "word/header1.xml".into(),
                    feature: "altChunk".into(),
                    occurrences: 2
                },
            ]
        );
        assert_eq!(pkg.files, original);
    }

    #[test]
    fn inventories_every_package_part_in_stable_order() {
        let mut pkg = package(&[("word/document.xml", "<document/>")]);
        pkg.files
            .insert("word/media/image.png".into(), vec![1, 2, 3]);
        pkg.files
            .insert("customXml/item1.xml".into(), b"<data/>".to_vec());
        let report = pkg.compatibility_report();
        assert_eq!(
            report.parts,
            vec![
                "customXml/item1.xml",
                "word/document.xml",
                "word/media/image.png"
            ]
        );
        assert_eq!(report.scanned_xml_parts, 2);
        assert_eq!(report.xml_part_digests.len(), 2);
        assert_eq!(report.xml_part_digests[0].part, "customXml/item1.xml");
        assert_eq!(report.opaque_part_digests.len(), 1);
        assert_eq!(report.opaque_part_digests[0].part, "word/media/image.png");
        assert_eq!(
            report.opaque_part_digests[0].sha256,
            format!("{:x}", Sha256::digest([1, 2, 3]))
        );
    }

    #[test]
    fn counts_main_document_paragraphs_and_revisions_by_namespace() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:del w:id="1"><w:r><w:delText>old</w:delText></w:r></w:del></w:p><w:p/><w:ins w:id="2"><w:p/></w:ins></w:body></w:document>"#;
        let other = r#"<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:ins/></w:p></w:hdr>"#;
        let report = package(&[("word/document.xml", xml), ("word/header1.xml", other)])
            .compatibility_report();
        assert_eq!(report.main_document_paragraph_count, 3);
        assert_eq!(report.main_document_revision_count, 2);
    }

    #[test]
    fn detects_revised_paragraph_marks_without_mistaking_run_revisions() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:x="urn:other"><w:body><w:p><w:pPr><w:rPr><w:del w:id="1"/></w:rPr></w:pPr><w:r><w:t>Hello</w:t></w:r></w:p><w:p><w:del w:id="2"><w:r><w:delText>old</w:delText></w:r></w:del><w:ins w:id="4"><w:r><w:t>new</w:t></w:r></w:ins></w:p><w:p><w:pPr><x:rPr><w:del w:id="3"/></x:rPr></w:pPr></w:p><w:p><w:pPr><w:rPr><w:ins w:id="5"/></w:rPr></w:pPr></w:p></w:body></w:document>"#;
        let report = package(&[("word/document.xml", xml)]).compatibility_report();
        assert_eq!(report.main_document_paragraph_count, 4);
        assert_eq!(report.main_document_revision_count, 5);
        assert_eq!(
            report.issues,
            vec![
                CompatibilityIssue {
                    part: "word/document.xml".into(),
                    feature: "deletedParagraphMark".into(),
                    occurrences: 1,
                },
                CompatibilityIssue {
                    part: "word/document.xml".into(),
                    feature: "insertedParagraphMark".into(),
                    occurrences: 1,
                },
            ]
        );
    }

    #[test]
    fn warns_only_for_text_boxes_inside_inline_drawings() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><w:body><w:p><w:r><w:drawing><wp:inline><w:txbxContent><w:p/></w:txbxContent></wp:inline></w:drawing></w:r><w:r><w:drawing><wp:anchor><w:txbxContent><w:p/></w:txbxContent></wp:anchor></w:drawing></w:r><w:r><w:txbxContent><w:p/></w:txbxContent></w:r></w:p></w:body></w:document>"#;
        let report = package(&[("word/document.xml", xml)]).compatibility_report();
        assert_eq!(
            report.issues,
            vec![CompatibilityIssue {
                part: "word/document.xml".into(),
                feature: "inlineTextBox".into(),
                occurrences: 1,
            }]
        );
    }

    #[test]
    fn detects_changed_opaque_part_bytes_even_when_part_name_is_preserved() {
        let mut pkg = package(&[("word/document.xml", "<document/>")]);
        pkg.files
            .insert("word/media/image.png".into(), vec![1, 2, 3]);
        let before = pkg.compatibility_report();
        pkg.files
            .insert("word/media/image.png".into(), vec![1, 2, 4]);
        let after = pkg.compatibility_report();
        assert_eq!(before.parts, after.parts);
        assert_ne!(before.opaque_part_digests, after.opaque_part_digests);
    }

    #[test]
    fn detects_changed_xml_part_bytes_even_when_part_name_is_preserved() {
        let mut pkg = package(&[("word/document.xml", "<document/>")]);
        pkg.files
            .insert("word/styles.xml".into(), b"<styles/>".to_vec());
        let before = pkg.compatibility_report();
        pkg.files.insert(
            "word/styles.xml".into(),
            b"<styles changed=\"1\"/>".to_vec(),
        );
        let after = pkg.compatibility_report();
        assert_eq!(before.parts, after.parts);
        assert_ne!(before.xml_part_digests, after.xml_part_digests);
    }

    #[test]
    fn tracks_protected_document_text_without_flagging_body_edits() {
        let source = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Original</w:t><w:delText>Removed</w:delText><w:instrText> PAGE </w:instrText></w:r></w:p></w:body></w:document>"#;
        let edited = source.replace("Original", "Edited");
        let changed = edited.replace("Removed", "Lost");
        let source_report = package(&[("word/document.xml", source)]).compatibility_report();
        let edited_report = package(&[("word/document.xml", &edited)]).compatibility_report();
        let changed_report = package(&[("word/document.xml", &changed)]).compatibility_report();
        assert_eq!(
            source_report.protected_text_digests,
            edited_report.protected_text_digests
        );
        assert_ne!(
            source_report.protected_text_digests,
            changed_report.protected_text_digests
        );
        assert_eq!(source_report.protected_text_digests.len(), 2);
        assert!(source_report
            .protected_text_digests
            .iter()
            .all(|digest| digest.occurrences == 1));
    }

    #[test]
    fn protected_text_uses_namespace_and_decoded_text() {
        let source = r#"<a:document xmlns:a="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:b="urn:other"><a:instrText>A<![CDATA[&]]>B</a:instrText><b:instrText>Ignored</b:instrText></a:document>"#;
        let output = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:instrText>A&#38;B</w:instrText></w:document>"#;
        assert_eq!(
            package(&[("word/document.xml", source)])
                .compatibility_report()
                .protected_text_digests,
            package(&[("word/document.xml", output)])
                .compatibility_report()
                .protected_text_digests,
        );
    }

    #[test]
    fn ignores_plain_text_comments_and_unrelated_namespaces() {
        let pkg = package(&[(
            "word/document.xml",
            r#"<w:document xmlns:w="urn:other"><!-- <w:altChunk/> --><w:altChunk/><w:t>altChunk &lt;w:altChunk/&gt;</w:t></w:document>"#,
        )]);
        assert!(pkg.compatibility_report().issues.is_empty());
    }

    #[test]
    fn inventories_smartart_ole_and_embedded_fonts() {
        let pkg = package(&[
            (
                "word/document.xml",
                r#"<root xmlns:d="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:o="urn:schemas-microsoft-com:office:office"><d:relIds/><o:OLEObject ProgID="Equation.3"/></root>"#,
            ),
            (
                "word/fontTable.xml",
                r#"<w:fonts xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:font><w:embedRegular/><w:embedBold/></w:font></w:fonts>"#,
            ),
        ]);
        let report = pkg.compatibility_report();
        assert_eq!(
            report
                .issues
                .iter()
                .map(|i| (i.feature.as_str(), i.occurrences))
                .collect::<Vec<_>>(),
            vec![("oleObject", 1), ("smartArt", 1), ("embeddedFont", 2)]
        );
    }

    #[test]
    fn malformed_xml_cannot_produce_a_clean_report() {
        for xml in ["<document><body></document>", "<document>", "", "<a/><b/>"] {
            let pkg = package(&[("word/document.xml", xml)]);
            assert_eq!(
                pkg.compatibility_report().issues[0].feature,
                "unreadableXml"
            );
        }
    }

    #[test]
    fn handles_default_strict_namespace_and_utf16_parts() {
        let xml = r#"<document xmlns="http://purl.oclc.org/ooxml/wordprocessingml/main"><altChunk /></document>"#;
        let mut pkg = package(&[("relocated/main.xml", xml)]);
        let mut utf16 = vec![0xff, 0xfe];
        utf16.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
        pkg.files.insert("relocated/utf16.xml".into(), utf16);
        pkg.files.insert("word/media/image.png".into(), vec![0xff]);
        let report = pkg.compatibility_report();
        assert_eq!(report.scanned_xml_parts, 2);
        assert_eq!(report.issues.len(), 2);
        assert!(report
            .issues
            .iter()
            .all(|issue| issue.feature == "altChunk" && issue.occurrences == 1));
    }
}
