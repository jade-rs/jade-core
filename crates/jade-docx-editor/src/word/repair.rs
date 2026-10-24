//! 開いて修復: a damaged `.docx` recovered as far as it can be, and what
//! was lost said — as Word's 「読み取れない内容が含まれています」 offers.
//!
//! Three kinds of damage are mended:
//!
//! - the ZIP: an archive whose directory is gone or cut short (a download
//!   that stopped) is read entry by entry from the local headers, each
//!   entry that still inflates kept;
//! - the XML: a part that is not well formed keeps every element up to the
//!   damage and as much after it as reads, its open elements closed;
//! - the package: missing content types and relationships are made again,
//!   and relationships to parts that are gone are dropped.

use std::fmt::Write as _;
use std::io::Read;

use jade_opc::OpcPackage;
use quick_xml::events::Event;
use quick_xml::Reader;

/// What the repair did, a line each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepairNote {
    /// The archive's directory was unreadable; it was read entry by entry.
    ArchiveRebuilt,
    /// An entry that would not inflate, by name.
    EntryLost(String),
    /// A part whose XML was damaged: what was kept of it.
    PartMended {
        part: String,
        kept_bytes: usize,
        total_bytes: usize,
    },
    /// A relationship to a part that is not there, by target.
    RelationshipDropped(String),
    /// The content types were made again.
    ContentTypesRebuilt,
    /// The package relationships were made again.
    PackageRelationshipsRebuilt,
}

/// A repaired document and what the repair did.
#[derive(Clone, Debug)]
pub struct Repaired {
    pub docx: Vec<u8>,
    pub notes: Vec<RepairNote>,
}

/// Whether `docx` needs repairing: its archive does not read, or one of
/// its XML parts is not well formed, or a part it needs is missing.
pub fn needs_repair(docx: &[u8]) -> bool {
    let Ok(package) = OpcPackage::open(docx) else {
        return true;
    };
    if package.part("[Content_Types].xml").is_none() || package.part("_rels/.rels").is_none() {
        return true;
    }
    package
        .part_names()
        .iter()
        .filter(|name| name.ends_with(".xml") || name.ends_with(".rels"))
        .any(|name| package.part(name).is_some_and(|xml| !well_formed(xml)))
}

/// Whether `xml` reads to its end with every element closed.
fn well_formed(xml: &[u8]) -> bool {
    let text = String::from_utf8_lossy(xml);
    let mut reader = Reader::from_str(&text);
    let mut depth = 0i64;
    loop {
        match reader.read_event() {
            Ok(Event::Start(_)) => depth += 1,
            Ok(Event::End(_)) => depth -= 1,
            Ok(Event::Eof) => return depth == 0,
            Err(_) => return false,
            _ => {}
        }
    }
}

/// Repairs `docx`. Fails only when nothing of a Word document is left.
pub fn repair(docx: &[u8]) -> Result<Repaired, String> {
    let mut notes = Vec::new();
    let parts: Vec<(String, Vec<u8>)> = match OpcPackage::open(docx) {
        Ok(package) => package
            .part_names()
            .iter()
            .filter_map(|name| Some((name.clone(), package.part(name)?.to_vec())))
            .collect(),
        Err(_) => {
            notes.push(RepairNote::ArchiveRebuilt);
            salvage_entries(docx, &mut notes)
        }
    };
    if !parts.iter().any(|(name, _)| name == "word/document.xml") {
        return Err("no document part could be recovered".to_string());
    }
    let mut package = OpcPackage::open(EMPTY_ZIP).map_err(|error| error.to_string())?;
    for (name, bytes) in &parts {
        let mended = if name.ends_with(".xml") || name.ends_with(".rels") {
            if well_formed(bytes) {
                bytes.clone()
            } else {
                let (mended, kept) = mend_xml(&String::from_utf8_lossy(bytes));
                notes.push(RepairNote::PartMended {
                    part: name.clone(),
                    kept_bytes: kept,
                    total_bytes: bytes.len(),
                });
                mended.into_bytes()
            }
        } else {
            bytes.clone()
        };
        package.replace(name, mended);
    }
    if package.part("[Content_Types].xml").is_none() {
        package.replace(
            "[Content_Types].xml",
            content_types(package.part_names()).into_bytes(),
        );
        notes.push(RepairNote::ContentTypesRebuilt);
    }
    if package.part("_rels/.rels").is_none() {
        package.replace("_rels/.rels", PACKAGE_RELS.as_bytes().to_vec());
        notes.push(RepairNote::PackageRelationshipsRebuilt);
    }
    // Relationships to parts that are not there go.
    let names: Vec<String> = package.part_names().to_vec();
    for rels_name in names.iter().filter(|name| name.ends_with(".rels")) {
        let Some(xml) = package.part_str(rels_name).ok().flatten() else {
            continue;
        };
        let source = rels_name
            .replace("_rels/", "")
            .trim_end_matches(".rels")
            .to_string();
        let mut kept = String::new();
        let mut changed = false;
        let mut rest = xml.as_str();
        while let Some(start) = rest.find("<Relationship ") {
            kept.push_str(&rest[..start]);
            let end = rest[start..]
                .find("/>")
                .map(|end| start + end + 2)
                .unwrap_or(rest.len());
            let element = &rest[start..end];
            let target = element
                .split("Target=\"")
                .nth(1)
                .and_then(|tail| tail.split('"').next())
                .unwrap_or("");
            let external = element.contains("TargetMode=\"External\"");
            let resolved = jade_opc::resolve_target(&source, target);
            if external || names.contains(&resolved) {
                kept.push_str(element);
            } else {
                notes.push(RepairNote::RelationshipDropped(resolved));
                changed = true;
            }
            rest = &rest[end..];
        }
        kept.push_str(rest);
        if changed {
            package.replace(rels_name, kept.into_bytes());
        }
    }
    let docx = package.write().map_err(|error| error.to_string())?;
    Ok(Repaired { docx, notes })
}

/// The entries of a ZIP read from its local headers, without its
/// directory: each one whose data inflates.
fn salvage_entries(bytes: &[u8], notes: &mut Vec<RepairNote>) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 30 <= bytes.len() {
        if bytes[at..at + 4] != [0x50, 0x4b, 0x03, 0x04] {
            at += 1;
            continue;
        }
        let u16_at =
            |offset: usize| u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
        let u32_at = |offset: usize| {
            u32::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ]) as usize
        };
        let flags = u16_at(at + 6);
        let method = u16_at(at + 8);
        let compressed_size = u32_at(at + 18);
        let name_length = u16_at(at + 26);
        let extra_length = u16_at(at + 28);
        let name_start = at + 30;
        let data_start = name_start + name_length + extra_length;
        if data_start > bytes.len() {
            break;
        }
        let name =
            String::from_utf8_lossy(&bytes[name_start..name_start + name_length]).into_owned();
        let known_size = flags & 0x08 == 0;
        let data_end = if known_size {
            (data_start + compressed_size).min(bytes.len())
        } else {
            bytes.len()
        };
        let data = &bytes[data_start..data_end];
        let (contents, consumed) = match method {
            // Stored with its size given: the size; without, up to the
            // next entry.
            0 if known_size => {
                let complete = data_start + compressed_size <= bytes.len();
                (complete.then(|| data.to_vec()), data.len())
            }
            0 => {
                let next = data
                    .windows(4)
                    .position(|window| window == [0x50, 0x4b, 0x03, 0x04])
                    .unwrap_or(data.len());
                (Some(data[..next].to_vec()), next)
            }
            8 => {
                let mut decoder = flate2::read::DeflateDecoder::new(data);
                let mut out = Vec::new();
                match decoder.read_to_end(&mut out) {
                    Ok(_) => {
                        let consumed = data.len() - decoder.into_inner().len();
                        (Some(out), consumed)
                    }
                    Err(_) => {
                        // A cut-off entry: what inflated before the cut.
                        (None, data.len())
                    }
                }
            }
            _ => (None, data.len()),
        };
        match contents {
            Some(contents) if !name.ends_with('/') => out.push((name, contents)),
            Some(_) => {}
            None => notes.push(RepairNote::EntryLost(name)),
        }
        at = data_start + consumed.max(1);
    }
    out
}

/// `xml` mended: every event up to the damage, then whatever reads after
/// it, end tags that close nothing dropped and the elements left open
/// closed. With how many bytes of the original made it through.
fn mend_xml(xml: &str) -> (String, usize) {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = false;
    let mut out = String::with_capacity(xml.len());
    let mut stack: Vec<String> = Vec::new();
    let mut kept = 0usize;
    let mut last = 0usize;
    loop {
        let event = reader.read_event();
        let position = reader.buffer_position() as usize;
        match event {
            Ok(Event::Eof) => break,
            Ok(Event::Start(start)) => {
                stack.push(String::from_utf8_lossy(start.name().as_ref()).into_owned());
                out.push_str(&xml[last..position]);
                kept += position - last;
            }
            Ok(Event::End(end)) => {
                let name = String::from_utf8_lossy(end.name().as_ref()).into_owned();
                if let Some(depth) = stack.iter().rposition(|open| *open == name) {
                    // Elements left open inside it close first.
                    while stack.len() > depth + 1 {
                        let open = stack.pop().expect("deeper than depth");
                        let _ = write!(out, "</{open}>");
                    }
                    stack.pop();
                    out.push_str(&xml[last..position]);
                    kept += position - last;
                }
            }
            Ok(_) => {
                out.push_str(&xml[last..position]);
                kept += position - last;
            }
            Err(_) => {
                // Skip to the next tag and go on reading from there.
                let next = xml[position.min(xml.len())..]
                    .find('<')
                    .map(|offset| position + offset)
                    .unwrap_or(xml.len());
                if next >= xml.len() {
                    break;
                }
                let tail = &xml[next..];
                let (rest, rest_kept) = mend_tail(tail, &mut stack);
                out.push_str(&rest);
                kept += rest_kept;
                break;
            }
        }
        last = position;
    }
    while let Some(open) = stack.pop() {
        let _ = write!(out, "</{open}>");
    }
    (out, kept)
}

/// The rest of a damaged part, read on from a tag, with `stack` the
/// elements open before it.
fn mend_tail(tail: &str, stack: &mut Vec<String>) -> (String, usize) {
    let mut reader = Reader::from_str(tail);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = false;
    let mut out = String::new();
    let mut kept = 0;
    let mut last = 0usize;
    loop {
        let event = reader.read_event();
        let position = reader.buffer_position() as usize;
        match event {
            Ok(Event::Eof) | Err(_) => break,
            Ok(Event::Start(start)) => {
                stack.push(String::from_utf8_lossy(start.name().as_ref()).into_owned());
                out.push_str(&tail[last..position]);
                kept += position - last;
            }
            Ok(Event::End(end)) => {
                let name = String::from_utf8_lossy(end.name().as_ref()).into_owned();
                if let Some(depth) = stack.iter().rposition(|open| *open == name) {
                    while stack.len() > depth + 1 {
                        let open = stack.pop().expect("deeper than depth");
                        let _ = write!(out, "</{open}>");
                    }
                    stack.pop();
                    out.push_str(&tail[last..position]);
                    kept += position - last;
                }
            }
            Ok(Event::Decl(_)) => {}
            Ok(_) => {
                out.push_str(&tail[last..position]);
                kept += position - last;
            }
        }
        last = position;
    }
    (out, kept)
}

fn content_types(names: &[String]) -> String {
    let mut out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="jpeg" ContentType="image/jpeg"/><Default Extension="jpg" ContentType="image/jpeg"/><Default Extension="gif" ContentType="image/gif"/><Default Extension="emf" ContentType="image/x-emf"/><Default Extension="wmf" ContentType="image/x-wmf"/>"#,
    );
    let known = [
        (
            "word/document.xml",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
        ),
        (
            "word/styles.xml",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml",
        ),
        (
            "word/settings.xml",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml",
        ),
        (
            "word/numbering.xml",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml",
        ),
        (
            "word/fontTable.xml",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml",
        ),
        (
            "word/webSettings.xml",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.webSettings+xml",
        ),
        (
            "word/footnotes.xml",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml",
        ),
        (
            "word/endnotes.xml",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml",
        ),
        (
            "word/comments.xml",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml",
        ),
        (
            "word/theme/theme1.xml",
            "application/vnd.openxmlformats-officedocument.theme+xml",
        ),
        (
            "docProps/core.xml",
            "application/vnd.openxmlformats-package.core-properties+xml",
        ),
        (
            "docProps/app.xml",
            "application/vnd.openxmlformats-officedocument.extended-properties+xml",
        ),
    ];
    for name in names {
        if let Some((_, kind)) = known.iter().find(|(known, _)| known == name) {
            let _ = write!(
                out,
                r#"<Override PartName="/{name}" ContentType="{kind}"/>"#
            );
        } else if let Some(stem) = name.strip_prefix("word/header") {
            if stem.ends_with(".xml") {
                let _ = write!(
                    out,
                    r#"<Override PartName="/{name}" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/>"#
                );
            }
        } else if let Some(stem) = name.strip_prefix("word/footer") {
            if stem.ends_with(".xml") {
                let _ = write!(
                    out,
                    r#"<Override PartName="/{name}" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/>"#
                );
            }
        }
    }
    out.push_str("</Types>");
    out
}

const PACKAGE_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

/// A ZIP archive with nothing in it: just the end of its central directory.
const EMPTY_ZIP: &[u8] = &[
    0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_damaged_part_keeps_what_reads() {
        let (mended, kept) = mend_xml("<a><b>one</b><c>two</d><e>three");
        assert!(well_formed(mended.as_bytes()), "{mended}");
        assert!(mended.contains("one") && mended.contains("two") && mended.contains("three"));
        assert!(kept > 0);
        let (mended, _) = mend_xml("<a><b>x</b><<<garbage <c>after</c></a>");
        assert!(well_formed(mended.as_bytes()), "{mended}");
        assert!(mended.contains("after"));
    }
}
