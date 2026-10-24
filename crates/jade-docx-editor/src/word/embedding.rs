//! TrueType フォントをファイルに埋め込む (ECMA-376 Part 1 §17.8.1): the
//! faces the document's font table names, put into the package as
//! obfuscated fonts (`.odttf`: the first 32 bytes XORed with a key spelled
//! by a GUID) related from the font table, with `w:embedTrueTypeFonts` in
//! the settings — so the document draws in its own fonts where they are not
//! installed. Reading them back is `jade-docx-reader`'s (`embedded_font`),
//! which the layout already uses.
//!
//! Faces whose licence forbids embedding (`OS/2.fsType` restricted) are left
//! out, as Word leaves them out.

use jade_docx_reader::embedded_font::{
    deobfuscate_font, embedding_permission, EmbeddingPermission,
};
use jade_opc::OpcPackage;

const FONT_RELATIONSHIP: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/font";
const FONT_TABLE_RELS: &str = "word/_rels/fontTable.xml.rels";
const OBFUSCATED_FONT: &str = "application/vnd.openxmlformats-officedocument.obfuscatedFont";

/// The settings that come after `w:embedTrueTypeFonts`, so it goes before
/// the first of them.
const AFTER_EMBED: &[&str] = &[
    "<w:embedSystemFonts",
    "<w:saveSubsetFonts",
    "<w:saveFormsData",
    "<w:mirrorMargins",
    "<w:alignBordersAndEdges",
    "<w:bordersDoNotSurroundHeader",
    "<w:bordersDoNotSurroundFooter",
    "<w:gutterAtTop",
    "<w:hideSpellingErrors",
    "<w:hideGrammaticalErrors",
    "<w:activeWritingStyle",
    "<w:proofState",
    "<w:formsDesign",
    "<w:attachedTemplate",
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
    "<w:characterSpacingControl",
    "<w:compat",
    "<w:rsids",
];

/// A face to embed: which slot of its family it fills, and its file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Face {
    pub bold: bool,
    pub italic: bool,
    pub data: Vec<u8>,
}

/// What embedding did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Embedded {
    /// The families embedded.
    pub families: Vec<String>,
    /// The families left out because their licence forbids it.
    pub restricted: Vec<String>,
    /// The families nothing installed answers to.
    pub missing: Vec<String>,
}

/// Whether `docx` asks for its fonts to be embedded, and only the
/// characters used.
pub fn embedding_settings(docx: &[u8]) -> (bool, bool) {
    let Some(settings) = OpcPackage::open(docx)
        .ok()
        .and_then(|package| package.part_str("word/settings.xml").ok().flatten())
    else {
        return (false, false);
    };
    let on = |tag: &str| {
        settings.contains(&format!("<{tag}/>"))
            || settings.contains(&format!("<{tag} w:val=\"1\""))
            || settings.contains(&format!("<{tag} w:val=\"true\""))
    };
    (on("w:embedTrueTypeFonts"), on("w:saveSubsetFonts"))
}

/// The families the font table names.
pub fn font_table_families(docx: &[u8]) -> Vec<String> {
    let Some(table) = OpcPackage::open(docx)
        .ok()
        .and_then(|package| package.part_str("word/fontTable.xml").ok().flatten())
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut rest = table.as_str();
    while let Some(at) = rest.find("<w:font ") {
        let element = &rest[at..at + rest[at..].find('>').unwrap_or(0)];
        if let Some(name) = element
            .split("w:name=\"")
            .nth(1)
            .and_then(|tail| tail.split('"').next())
        {
            let name = name.replace("&amp;", "&");
            if !out.contains(&name) {
                out.push(name);
            }
        }
        rest = &rest[at + 1..];
    }
    out
}

/// The face at `index` of a font collection (`.ttc`) as a font file of its
/// own; a single font comes back as it is.
pub fn sfnt_face(data: &[u8], index: u32) -> Option<Vec<u8>> {
    if !data.starts_with(b"ttcf") {
        return Some(data.to_vec());
    }
    let read_u32 = |at: usize| {
        data.get(at..at + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    let read_u16 = |at: usize| {
        data.get(at..at + 2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
    };
    let count = read_u32(8)?;
    if index >= count {
        return None;
    }
    let offset = read_u32(12 + index as usize * 4)? as usize;
    let tables = read_u16(offset + 4)? as usize;
    let header_size = 12 + tables * 16;
    let mut out = data.get(offset..offset + header_size)?.to_vec();
    let mut body: Vec<u8> = Vec::new();
    for table in 0..tables {
        let record = offset + 12 + table * 16;
        let table_offset = read_u32(record + 8)? as usize;
        let length = read_u32(record + 12)? as usize;
        let new_offset = header_size + body.len();
        body.extend_from_slice(data.get(table_offset..table_offset + length)?);
        while !body.len().is_multiple_of(4) {
            body.push(0);
        }
        out[12 + table * 16 + 8..12 + table * 16 + 12]
            .copy_from_slice(&(new_offset as u32).to_be_bytes());
    }
    out.extend_from_slice(&body);
    Some(out)
}

/// A GUID for a font key, made from the font's bytes and a counter, so
/// each one differs (Word's are random; nothing depends on that).
fn font_key(data: &[u8], counter: usize) -> String {
    let mut state: u64 = 0xcbf2_9ce4_8422_2325 ^ counter as u64;
    for byte in data.iter().step_by((data.len() / 4096).max(1)) {
        state ^= *byte as u64;
        state = state.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let mut bytes = [0u8; 16];
    for chunk in bytes.chunks_mut(8) {
        state ^= state >> 33;
        state = state.wrapping_mul(0xff51_afd7_ed55_8ccd);
        state ^= state >> 33;
        chunk.copy_from_slice(&state.to_be_bytes());
    }
    let hex: String = bytes.iter().map(|byte| format!("{byte:02X}")).collect();
    format!(
        "{{{}-{}-{}-{}-{}}}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn insert_before_first(xml: &str, element: &str, after: &[&str], close: &str) -> String {
    let at = after
        .iter()
        .filter_map(|tag| xml.find(tag))
        .min()
        .or_else(|| xml.rfind(close))
        .unwrap_or(xml.len());
    format!("{}{element}{}", &xml[..at], &xml[at..])
}

/// `docx` with no fonts embedded: the parts, their relationships and the
/// settings gone.
pub fn remove_embedded_fonts(docx: &[u8]) -> Result<Vec<u8>, String> {
    let mut package = OpcPackage::open(docx).map_err(|error| error.to_string())?;
    let mut changed = false;
    if let Some(table) = package
        .part_str("word/fontTable.xml")
        .map_err(|e| e.to_string())?
    {
        let mut out = String::with_capacity(table.len());
        let mut rest = table.as_str();
        while let Some(at) = rest.find("<w:embed") {
            out.push_str(&rest[..at]);
            let end = rest[at..]
                .find("/>")
                .map(|end| at + end + 2)
                .unwrap_or(rest.len());
            rest = &rest[end..];
            changed = true;
        }
        out.push_str(rest);
        if changed {
            package.replace("word/fontTable.xml", out.into_bytes());
        }
    }
    let fonts: Vec<String> = package
        .part_names()
        .iter()
        .filter(|name| name.starts_with("word/fonts/"))
        .cloned()
        .collect();
    for name in fonts {
        package.remove(&name);
        changed = true;
    }
    if package.remove(FONT_TABLE_RELS) {
        changed = true;
    }
    if let Some(settings) = package
        .part_str("word/settings.xml")
        .map_err(|e| e.to_string())?
    {
        let stripped = settings
            .replace("<w:embedTrueTypeFonts/>", "")
            .replace("<w:saveSubsetFonts/>", "");
        if stripped != settings {
            package.replace("word/settings.xml", stripped.into_bytes());
            changed = true;
        }
    }
    if !changed {
        return Ok(docx.to_vec());
    }
    package.write().map_err(|error| error.to_string())
}

/// `docx` with the faces `find` gives for each family its font table
/// names embedded, and the settings asking for that (with
/// `w:saveSubsetFonts` when `subset`, which a Word that saves it again
/// honours).
pub fn embed_fonts(
    docx: &[u8],
    subset: bool,
    find: &dyn Fn(&str) -> Vec<Face>,
) -> Result<(Vec<u8>, Embedded), String> {
    let cleared = remove_embedded_fonts(docx)?;
    let mut package = OpcPackage::open(&cleared).map_err(|error| error.to_string())?;
    let mut report = Embedded::default();
    let Some(mut table) = package
        .part_str("word/fontTable.xml")
        .map_err(|e| e.to_string())?
    else {
        return Ok((cleared, report));
    };
    let mut relationships = String::new();
    let mut counter = 0usize;
    for family in font_table_families(&cleared) {
        let faces = find(&family);
        if faces.is_empty() {
            report.missing.push(family);
            continue;
        }
        let allowed: Vec<&Face> = faces
            .iter()
            .filter(|face| {
                embedding_permission(&face.data).is_none_or(EmbeddingPermission::allows_use)
            })
            .collect();
        if allowed.is_empty() {
            report.restricted.push(family);
            continue;
        }
        let mut elements = String::new();
        for (slot, bold, italic) in [
            ("embedRegular", false, false),
            ("embedBold", true, false),
            ("embedItalic", false, true),
            ("embedBoldItalic", true, true),
        ] {
            let Some(face) = allowed
                .iter()
                .find(|face| face.bold == bold && face.italic == italic)
            else {
                continue;
            };
            counter += 1;
            let key = font_key(&face.data, counter);
            let obfuscated =
                deobfuscate_font(&face.data, &key).map_err(|error| error.to_string())?;
            let name = format!("font{counter}.odttf");
            package.replace(&format!("word/fonts/{name}"), obfuscated);
            let id = format!("rIdJadeFont{counter}");
            relationships.push_str(&format!(
                r#"<Relationship Id="{id}" Type="{FONT_RELATIONSHIP}" Target="fonts/{name}"/>"#
            ));
            elements.push_str(&format!(r#"<w:{slot} r:id="{id}" w:fontKey="{key}"/>"#));
        }
        if elements.is_empty() {
            continue;
        }
        let escaped = family.replace('&', "&amp;").replace('"', "&quot;");
        let marker = format!("w:name=\"{escaped}\"");
        let Some(at) = table.find(&marker) else {
            continue;
        };
        let start = table[..at].rfind("<w:font ").unwrap_or(at);
        let open_end = start + table[start..].find('>').unwrap_or(0);
        if table[..open_end].ends_with('/') {
            // `<w:font w:name="…"/>`: opened up to take its embeds.
            let element = &table[start..open_end - 1];
            table = format!(
                "{}{element}>{elements}</w:font>{}",
                &table[..start],
                &table[open_end + 1..]
            );
        } else {
            let close = open_end + table[open_end..].find("</w:font>").unwrap_or(0);
            table = format!("{}{elements}{}", &table[..close], &table[close..]);
        }
        report.families.push(family);
    }
    if report.families.is_empty() {
        return Ok((cleared, report));
    }
    if !table.contains("xmlns:r=") {
        table = table.replacen(
            "<w:fonts ",
            r#"<w:fonts xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" "#,
            1,
        );
    }
    package.replace("word/fontTable.xml", table.into_bytes());
    // The document has to name its font table for anyone to find it.
    if let Some(rels) = package
        .part_str("word/_rels/document.xml.rels")
        .map_err(|e| e.to_string())?
    {
        if !rels.contains("/relationships/fontTable\"") {
            let added = rels.replace(
                "</Relationships>",
                r#"<Relationship Id="rIdJadeFontTable" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/fontTable" Target="fontTable.xml"/></Relationships>"#,
            );
            package.replace("word/_rels/document.xml.rels", added.into_bytes());
        }
    }
    package.replace(
        FONT_TABLE_RELS,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{relationships}</Relationships>"#
        )
        .into_bytes(),
    );
    if let Some(types) = package
        .part_str("[Content_Types].xml")
        .map_err(|e| e.to_string())?
    {
        if !types.contains("Extension=\"odttf\"") {
            let added = types.replacen(
                "<Default ",
                &format!(
                    r#"<Default Extension="odttf" ContentType="{OBFUSCATED_FONT}"/><Default "#
                ),
                1,
            );
            package.replace("[Content_Types].xml", added.into_bytes());
        }
    }
    if let Some(settings) = package
        .part_str("word/settings.xml")
        .map_err(|e| e.to_string())?
    {
        let mut element = String::from("<w:embedTrueTypeFonts/>");
        if subset {
            element.push_str("<w:saveSubsetFonts/>");
        }
        let settings = insert_before_first(&settings, &element, AFTER_EMBED, "</w:settings>");
        package.replace("word/settings.xml", settings.into_bytes());
    }
    let bytes = package.write().map_err(|error| error.to_string())?;
    Ok((bytes, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::word::new::{blank_document, Blank};

    fn bundled_font() -> Vec<u8> {
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fonts/Noto_Sans_JP/static/NotoSansJP-Regular.ttf"
        ))
        .unwrap()
    }

    /// A blank document whose font table names `family`.
    fn naming(family: &str) -> Vec<u8> {
        let blank = blank_document(Blank::English);
        let mut package = OpcPackage::open(&blank).unwrap();
        package.replace(
            "word/fontTable.xml",
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:fonts xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:font w:name="{family}"><w:charset w:val="80"/></w:font><w:font w:name="Missing Sans"/></w:fonts>"#
            )
            .into_bytes(),
        );
        let types = package.part_str("[Content_Types].xml").unwrap().unwrap();
        package.replace(
            "[Content_Types].xml",
            types
                .replace("</Types>", r#"<Override PartName="/word/fontTable.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml"/></Types>"#)
                .into_bytes(),
        );
        package.write().unwrap()
    }

    #[test]
    fn embedded_fonts_read_back_as_the_faces_they_were() {
        let font = bundled_font();
        let docx = naming("Noto Sans JP");
        let find = |family: &str| {
            if family == "Noto Sans JP" {
                vec![Face {
                    bold: false,
                    italic: false,
                    data: font.clone(),
                }]
            } else {
                Vec::new()
            }
        };
        let (embedded, report) = embed_fonts(&docx, true, &find).unwrap();
        assert_eq!(report.families, vec!["Noto Sans JP".to_string()]);
        assert_eq!(report.missing, vec!["Missing Sans".to_string()]);
        assert_eq!(embedding_settings(&embedded), (true, true));
        let read = jade_docx_reader::docx_embedded_fonts(&embedded).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].data, font, "deobfuscated, it is the font");
        // The part itself is obfuscated.
        let package = OpcPackage::open(&embedded).unwrap();
        let part = package.part("word/fonts/font1.odttf").unwrap();
        assert_ne!(&part[..32], &font[..32]);
        assert_eq!(&part[32..64], &font[32..64]);
        // Turned off, they go.
        let removed = remove_embedded_fonts(&embedded).unwrap();
        assert_eq!(embedding_settings(&removed), (false, false));
        assert!(jade_docx_reader::docx_embedded_fonts(&removed)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn a_collection_face_becomes_a_font_of_its_own() {
        let font = bundled_font();
        assert_eq!(sfnt_face(&font, 0).unwrap(), font);
    }
}
