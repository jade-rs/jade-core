//! テンプレートとアドイン: the template a document is attached to
//! (`w:attachedTemplate`, a relationship of the settings part to the
//! template's file), attaching another — and with 文書のスタイルを自動的に
//! 更新する (`w:linkStyles`) taking its styles — detaching it back to
//! Normal, and the Normal template itself: the styles and page every new
//! document starts from.

use jade_opc::OpcPackage;

const ATTACHED_TEMPLATE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/attachedTemplate";
const SETTINGS_RELS: &str = "word/_rels/settings.xml.rels";
const RELATIONSHIP_ID: &str = "rIdJadeTemplate";

/// The settings elements that come after `w:attachedTemplate` (ECMA-376
/// Part 1 §17.15.1.78's sequence), so it goes in before the first of them.
const AFTER_ATTACHED_TEMPLATE: &[&str] = &[
    "<w:linkStyles",
    "<w:stylePaneFormatFilter",
    "<w:stylePaneSortMethod",
    "<w:documentType",
    "<w:mailMerge",
    "<w:revisionView",
    "<w:trackRevisions",
    "<w:doNotTrackMoves",
    "<w:doNotTrackFormatting",
    "<w:documentProtection",
    "<w:autoFormatOverride",
    "<w:styleLockTheme",
    "<w:styleLockQFSet",
    "<w:defaultTabStop",
    "<w:autoHyphenation",
    "<w:consecutiveHyphenLimit",
    "<w:hyphenationZone",
    "<w:doNotHyphenateCaps",
    "<w:showEnvelope",
    "<w:summaryLength",
    "<w:clickAndTypeStyle",
    "<w:defaultTableStyle",
    "<w:evenAndOddHeaders",
    "<w:bookFoldRevPrinting",
    "<w:bookFoldPrinting",
    "<w:bookFoldPrintingSheets",
    "<w:drawingGridHorizontalSpacing",
    "<w:drawingGridVerticalSpacing",
    "<w:displayHorizontalDrawingGridEvery",
    "<w:displayVerticalDrawingGridEvery",
    "<w:doNotUseMarginsForDrawingGridOrigin",
    "<w:drawingGridHorizontalOrigin",
    "<w:drawingGridVerticalOrigin",
    "<w:doNotShadeFormData",
    "<w:noPunctuationKerning",
    "<w:characterSpacingControl",
    "<w:printTwoOnOne",
    "<w:strictFirstAndLastChars",
    "<w:noLineBreaksAfter",
    "<w:noLineBreaksBefore",
    "<w:savePreviewPicture",
    "<w:doNotValidateAgainstSchema",
    "<w:saveInvalidXml",
    "<w:ignoreMixedContent",
    "<w:alwaysShowPlaceholderText",
    "<w:doNotDemarcateInvalidXml",
    "<w:saveXmlDataOnly",
    "<w:useXSLTWhenSaving",
    "<w:saveThroughXslt",
    "<w:showXMLTags",
    "<w:alwaysMergeEmptyNamespace",
    "<w:updateFields",
    "<w:hdrShapeDefaults",
    "<w:footnotePr",
    "<w:endnotePr",
    "<w:compat",
    "<w:docVars",
    "<w:rsids",
    "<m:mathPr",
    "<w:attachedSchema",
    "<w:themeFontLang",
    "<w:clrSchemeMapping",
    "<w:doNotIncludeSubdocsInStats",
    "<w:doNotAutoCompressPictures",
    "<w:forceUpgrade",
    "<w:captions",
    "<w:readModeInkLockDown",
    "<w:smartTagType",
    "<sl:schemaLibrary",
    "<w:shapeDefaults",
    "<w:doNotEmbedSmartTags",
    "<w:decimalSymbol",
    "<w:listSeparator",
];

fn settings(package: &OpcPackage) -> Result<String, String> {
    package
        .part_str("word/settings.xml")
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the document has no settings".to_string())
}

/// Removes the first element named `tag` from `xml`.
fn remove_element(xml: &str, tag: &str) -> String {
    let open = format!("<{tag}");
    let Some(start) = xml.find(&open) else {
        return xml.to_string();
    };
    let after = &xml[start + open.len()..];
    if !after.starts_with(['/', '>', ' ']) {
        return xml.to_string();
    }
    let Some(end) = after.find('>') else {
        return xml.to_string();
    };
    let tail = if after[..end].ends_with('/') {
        &after[end + 1..]
    } else {
        let close = format!("</{tag}>");
        match after.find(&close) {
            Some(close_at) => &after[close_at + close.len()..],
            None => &after[end + 1..],
        }
    };
    format!("{}{}", &xml[..start], tail)
}

/// Puts `element` into the settings where the schema wants it.
fn insert_setting(settings: &str, element: &str) -> String {
    let at = AFTER_ATTACHED_TEMPLATE
        .iter()
        .filter_map(|tag| settings.find(tag))
        .min()
        .or_else(|| settings.rfind("</w:settings>"))
        .unwrap_or(settings.len());
    format!("{}{element}{}", &settings[..at], &settings[at..])
}

/// The template `docx` is attached to: the path or URL its settings name.
pub fn attached_template(docx: &[u8]) -> Option<String> {
    let package = OpcPackage::open(docx).ok()?;
    let settings = settings(&package).ok()?;
    let at = settings.find("<w:attachedTemplate")?;
    let element = &settings[at..at + settings[at..].find('>')?];
    let id = element.split("r:id=\"").nth(1)?.split('"').next()?;
    let rels = package.part_str(SETTINGS_RELS).ok()??;
    let relationships = jade_opc::parse_relationships(&rels).ok()?;
    let target = relationships.target(id)?.to_string();
    Some(file_url_path(&target))
}

/// A `file:///…` target as a path, percent escapes undone.
fn file_url_path(target: &str) -> String {
    let Some(path) = target.strip_prefix("file://") else {
        return target.to_string();
    };
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' && at + 2 < bytes.len() {
            if let Ok(value) = u8::from_str_radix(&path[at + 1..at + 3], 16) {
                out.push(value);
                at += 3;
                continue;
            }
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A path as a `file:///` URL, as Word writes the relationship.
fn file_url(path: &str) -> String {
    let mut out = String::from("file://");
    for byte in path.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'.' | b'-' | b'_' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Whether 文書のスタイルを自動的に更新する is on.
pub fn links_styles(docx: &[u8]) -> bool {
    OpcPackage::open(docx)
        .ok()
        .and_then(|package| settings(&package).ok())
        .is_some_and(|settings| {
            settings.contains("<w:linkStyles/>")
                || settings.contains("<w:linkStyles w:val=\"1\"")
                || settings.contains("<w:linkStyles w:val=\"true\"")
        })
}

/// 添付: `docx` attached to the template at `path`. With `link_styles`
/// (文書のスタイルを自動的に更新する) and the template's bytes given, the
/// template's styles replace the document's of the same name and its other
/// styles are added, as Word updates them.
pub fn attach_template(
    docx: &[u8],
    path: &str,
    template: Option<&[u8]>,
    link_styles: bool,
) -> Result<Vec<u8>, String> {
    let detached = detach_template(docx)?;
    let mut package = OpcPackage::open(&detached).map_err(|error| error.to_string())?;
    let mut settings = settings(&package)?;
    settings = insert_setting(
        &settings,
        &format!(r#"<w:attachedTemplate r:id="{RELATIONSHIP_ID}"/>"#),
    );
    if link_styles {
        settings = insert_setting(&settings, "<w:linkStyles/>");
    }
    if !settings.contains("xmlns:r=") {
        settings = settings.replacen(
            "<w:settings ",
            r#"<w:settings xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" "#,
            1,
        );
    }
    package.replace("word/settings.xml", settings.into_bytes());
    let relationship = format!(
        r#"<Relationship Id="{RELATIONSHIP_ID}" Type="{ATTACHED_TEMPLATE}" Target="{}" TargetMode="External"/>"#,
        escape(&file_url(path))
    );
    let rels = match package
        .part_str(SETTINGS_RELS)
        .map_err(|error| error.to_string())?
    {
        Some(rels) => rels.replace(
            "</Relationships>",
            &format!("{relationship}</Relationships>"),
        ),
        None => format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{relationship}</Relationships>"#
        ),
    };
    package.replace(SETTINGS_RELS, rels.into_bytes());
    let mut bytes = package.write().map_err(|error| error.to_string())?;
    if link_styles {
        if let Some(template) = template {
            bytes = update_styles(&bytes, template)?;
        }
    }
    Ok(bytes)
}

/// 切り離す: `docx` with no template of its own — attached to Normal again.
pub fn detach_template(docx: &[u8]) -> Result<Vec<u8>, String> {
    let mut package = OpcPackage::open(docx).map_err(|error| error.to_string())?;
    let Ok(settings) = settings(&package) else {
        return Ok(docx.to_vec());
    };
    let id = settings.find("<w:attachedTemplate").and_then(|at| {
        let element = &settings[at..at + settings[at..].find('>')?];
        Some(
            element
                .split("r:id=\"")
                .nth(1)?
                .split('"')
                .next()?
                .to_string(),
        )
    });
    let stripped = remove_element(
        &remove_element(&settings, "w:attachedTemplate"),
        "w:linkStyles",
    );
    if stripped == settings {
        return Ok(docx.to_vec());
    }
    package.replace("word/settings.xml", stripped.into_bytes());
    if let (Some(id), Some(rels)) = (id, package.part_str(SETTINGS_RELS).ok().flatten()) {
        let marker = format!(r#"Id="{id}""#);
        if let Some(at) = rels.find(&marker) {
            let start = rels[..at].rfind('<').unwrap_or(at);
            let end = at + rels[at..].find("/>").map(|end| end + 2).unwrap_or(0);
            let removed = format!("{}{}", &rels[..start], &rels[end..]);
            package.replace(SETTINGS_RELS, removed.into_bytes());
        }
    }
    package.write().map_err(|error| error.to_string())
}

/// The `<w:style …>…</w:style>` elements of a styles part, by style id.
fn styles_by_id(styles: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(found) = styles[at..].find("<w:style ") {
        let start = at + found;
        let Some(open_end) = styles[start..].find('>').map(|end| start + end) else {
            break;
        };
        let end = if styles[..open_end].ends_with('/') {
            open_end + 1
        } else {
            match styles[open_end..].find("</w:style>") {
                Some(close) => open_end + close + "</w:style>".len(),
                None => break,
            }
        };
        let element = &styles[start..end];
        let id = element
            .split("w:styleId=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .unwrap_or("")
            .to_string();
        out.push((id, element.to_string()));
        at = end;
    }
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

/// `docx` with `template`'s styles: the template's replace those of the
/// same id, the template's defaults replace the document's, and the
/// document keeps the styles the template does not have.
pub fn update_styles(docx: &[u8], template: &[u8]) -> Result<Vec<u8>, String> {
    let mut package = OpcPackage::open(docx).map_err(|error| error.to_string())?;
    let template_package =
        OpcPackage::open(template).map_err(|error| format!("cannot read the template: {error}"))?;
    let Some(template_styles) = template_package
        .part_str("word/styles.xml")
        .map_err(|error| error.to_string())?
    else {
        return Ok(docx.to_vec());
    };
    let own = package
        .part_str("word/styles.xml")
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
    let template_ids: Vec<String> = styles_by_id(&template_styles)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    let kept: String = styles_by_id(&own)
        .into_iter()
        .filter(|(id, _)| !template_ids.contains(id))
        .map(|(_, element)| element)
        .collect();
    let merged = match template_styles.rfind("</w:styles>") {
        Some(end) => format!(
            "{}{kept}{}",
            &template_styles[..end],
            &template_styles[end..]
        ),
        None => template_styles.clone(),
    };
    package.replace("word/styles.xml", merged.into_bytes());
    package.write().map_err(|error| error.to_string())
}

/// The Normal template made from `docx`: its styles, settings, theme and
/// page, with an empty body — what 既定に設定 keeps for new documents.
pub fn normal_template_from(docx: &[u8]) -> Result<Vec<u8>, String> {
    let mut package = OpcPackage::open(docx).map_err(|error| error.to_string())?;
    let document = package
        .part_str("word/document.xml")
        .map_err(|error| error.to_string())?
        .ok_or("the document has no body")?;
    let body_start = document
        .find("<w:body>")
        .ok_or("the document has no body")?
        + "<w:body>".len();
    let body_end = document
        .rfind("</w:body>")
        .ok_or("the document has no body")?;
    let body = &document[body_start..body_end];
    // The last section's properties are the page.
    let section = body
        .rfind("<w:sectPr")
        .map(|start| &body[start..])
        .unwrap_or("");
    let emptied = format!(
        "{}<w:p/>{section}{}",
        &document[..body_start],
        &document[body_end..]
    );
    package.replace("word/document.xml", emptied.into_bytes());
    let bytes = package.write().map_err(|error| error.to_string())?;
    let bytes = detach_template(&bytes)?;
    super::formats::template_from_document(&bytes, false).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::word::new::{blank_document, Blank};

    fn styles(docx: &[u8]) -> String {
        OpcPackage::open(docx)
            .unwrap()
            .part_str("word/styles.xml")
            .unwrap()
            .unwrap()
    }

    #[test]
    fn attaching_names_the_template_and_detaching_forgets_it() {
        let blank = blank_document(Blank::English);
        assert_eq!(attached_template(&blank), None);
        let attached =
            attach_template(&blank, "/Users/me/テンプレート/Letter.dotx", None, false).unwrap();
        assert_eq!(
            attached_template(&attached).as_deref(),
            Some("/Users/me/テンプレート/Letter.dotx")
        );
        assert!(!links_styles(&attached));
        let settings = OpcPackage::open(&attached)
            .unwrap()
            .part_str("word/settings.xml")
            .unwrap()
            .unwrap();
        // In the schema's order: before the tab stop default.
        assert!(
            settings.find("<w:attachedTemplate").unwrap()
                < settings.find("<w:defaultTabStop").unwrap()
        );
        let detached = detach_template(&attached).unwrap();
        assert_eq!(attached_template(&detached), None);
        let rels = OpcPackage::open(&detached)
            .unwrap()
            .part_str(SETTINGS_RELS)
            .unwrap()
            .unwrap();
        assert!(!rels.contains("attachedTemplate"));
    }

    #[test]
    fn linked_styles_come_from_the_template() {
        let document = blank_document(Blank::English);
        // A template whose Heading 1 is red and which has a style of its own.
        let mut template = OpcPackage::open(&blank_document(Blank::Japanese)).unwrap();
        let template_styles = template.part_str("word/styles.xml").unwrap().unwrap();
        let template_styles = template_styles
            .replacen(
                r#"<w:style w:type="paragraph" w:styleId="Heading1">"#,
                r#"<w:style w:type="paragraph" w:styleId="Heading1"><w:rPr><w:color w:val="FF0000"/></w:rPr>"#,
                1,
            )
            .replace(
                "</w:styles>",
                r#"<w:style w:type="paragraph" w:styleId="LetterHead"><w:name w:val="Letter Head"/></w:style></w:styles>"#,
            );
        template.replace("word/styles.xml", template_styles.into_bytes());
        let template = template.write().unwrap();
        // The document has a style the template does not.
        let mut own = OpcPackage::open(&document).unwrap();
        let own_styles = own.part_str("word/styles.xml").unwrap().unwrap().replace(
            "</w:styles>",
            r#"<w:style w:type="character" w:styleId="Mine"><w:name w:val="Mine"/></w:style></w:styles>"#,
        );
        own.replace("word/styles.xml", own_styles.into_bytes());
        let document = own.write().unwrap();
        let attached = attach_template(&document, "/t/Letter.dotx", Some(&template), true).unwrap();
        assert!(links_styles(&attached));
        let merged = styles(&attached);
        assert!(merged.contains("LetterHead"));
        assert!(
            merged.contains(r#"w:styleId="Mine""#),
            "the document's own style stays"
        );
        assert!(merged.contains(r#"<w:color w:val="FF0000"/>"#));
        assert_eq!(merged.matches(r#"w:styleId="Heading1""#).count(), 1);
    }

    #[test]
    fn the_normal_template_is_the_document_without_its_text() {
        let blank = blank_document(Blank::Japanese);
        let mut package = OpcPackage::open(&blank).unwrap();
        let xml = package.part_str("word/document.xml").unwrap().unwrap();
        package.replace(
            "word/document.xml",
            xml.replace(
                "<w:body><w:p/>",
                "<w:body><w:p><w:r><w:t>text</w:t></w:r></w:p>",
            )
            .into_bytes(),
        );
        let docx = package.write().unwrap();
        let normal = normal_template_from(&docx).unwrap();
        assert_eq!(
            super::super::formats::WordFormat::sniff(&normal),
            Some(super::super::formats::WordFormat::Dotx)
        );
        let xml = OpcPackage::open(&normal)
            .unwrap()
            .part_str("word/document.xml")
            .unwrap()
            .unwrap();
        assert!(!xml.contains("text"));
        assert!(xml.contains(r#"w:w="11906""#), "the page stays");
    }
}
