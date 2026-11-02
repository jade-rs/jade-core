//! Where `<p:transition>` and `<p:timing>` go in a slide part, and the
//! sound parts a transition plays.
//!
//! `CT_Slide` orders its children `cSld`, `clrMapOvr`, `transition`,
//! `timing`, `extLst`; PowerPoint wraps a 2010+ transition in
//! `mc:AlternateContent` in the transition's place. Both are replaced as a
//! whole, everything else in the part left byte for byte.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ops::Range;

use jade_opc::{rels_part_name, resolve_target, OpcPackage};
use quick_xml::events::Event;
use quick_xml::Reader;

use super::{parse_document_nodes, slides};
use crate::{Result, WriteError};

struct TopChild {
    local: String,
    range: Range<usize>,
    /// An `mc:AlternateContent` whose branches hold a `p:transition`.
    transition: bool,
}

fn xml_error(error: impl std::fmt::Display) -> WriteError {
    WriteError::Xml(error.to_string())
}

fn local(name: &[u8]) -> String {
    String::from_utf8_lossy(jade_opc::local_name(name)).into_owned()
}

/// The root's children with their byte ranges, and where the root's close
/// tag starts.
fn top_children(xml: &str) -> Result<(Vec<TopChild>, usize)> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut depth = 0usize;
    let mut children: Vec<TopChild> = Vec::new();
    let mut root_end = xml.len();
    let mut buffer = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buffer).map_err(xml_error)?;
        let end = reader.buffer_position() as usize;
        match event {
            Event::Start(element) => {
                if depth == 1 {
                    children.push(TopChild {
                        local: local(element.name().as_ref()),
                        range: start..end,
                        transition: false,
                    });
                } else if depth >= 2 && local(element.name().as_ref()) == "transition" {
                    if let Some(last) = children.last_mut() {
                        last.transition |= last.local == "AlternateContent";
                    }
                }
                depth += 1;
            }
            Event::Empty(element) => {
                if depth == 1 {
                    children.push(TopChild {
                        local: local(element.name().as_ref()),
                        range: start..end,
                        transition: false,
                    });
                } else if depth >= 2 && local(element.name().as_ref()) == "transition" {
                    if let Some(last) = children.last_mut() {
                        last.transition |= last.local == "AlternateContent";
                    }
                }
            }
            Event::End(_) => {
                depth -= 1;
                if depth == 1 {
                    if let Some(last) = children.last_mut() {
                        last.range.end = end;
                    }
                } else if depth == 0 {
                    root_end = start;
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok((children, root_end))
}

/// Replaces (or inserts, or with `None` removes) the slide's
/// `<p:timing>`.
pub(crate) fn replace_timing(xml: &str, timing: Option<&str>) -> Result<String> {
    let (children, root_end) = top_children(xml)?;
    if let Some(existing) = children.iter().find(|c| c.local == "timing") {
        let mut out = String::with_capacity(xml.len());
        out.push_str(&xml[..existing.range.start]);
        out.push_str(timing.unwrap_or(""));
        out.push_str(&xml[existing.range.end..]);
        return Ok(out);
    }
    let Some(timing) = timing else {
        return Ok(xml.to_string());
    };
    // After the transition, else clrMapOvr, else cSld; before extLst.
    let at = children
        .iter()
        .rev()
        .find(|c| c.local == "transition" || c.transition)
        .or_else(|| children.iter().rev().find(|c| c.local == "clrMapOvr"))
        .or_else(|| children.iter().find(|c| c.local == "cSld"))
        .map(|c| c.range.end)
        .or_else(|| {
            children
                .iter()
                .find(|c| c.local == "extLst")
                .map(|c| c.range.start)
        })
        .unwrap_or(root_end);
    let mut out = String::with_capacity(xml.len() + timing.len());
    out.push_str(&xml[..at]);
    out.push_str(timing);
    out.push_str(&xml[at..]);
    Ok(out)
}

/// Replaces (or inserts, or with `None` removes) the slide's transition,
/// whether written plainly or inside `mc:AlternateContent`.
pub(crate) fn replace_transition(xml: &str, transition: Option<&str>) -> Result<String> {
    let (children, root_end) = top_children(xml)?;
    if let Some(existing) = children
        .iter()
        .find(|c| c.local == "transition" || c.transition)
    {
        let mut out = String::with_capacity(xml.len());
        out.push_str(&xml[..existing.range.start]);
        out.push_str(transition.unwrap_or(""));
        out.push_str(&xml[existing.range.end..]);
        return Ok(out);
    }
    let Some(transition) = transition else {
        return Ok(xml.to_string());
    };
    let at = children
        .iter()
        .rev()
        .find(|c| c.local == "clrMapOvr")
        .or_else(|| children.iter().find(|c| c.local == "cSld"))
        .map(|c| c.range.end)
        .or_else(|| {
            children
                .iter()
                .find(|c| c.local == "timing" || c.local == "extLst")
                .map(|c| c.range.start)
        })
        .unwrap_or(root_end);
    let mut out = String::with_capacity(xml.len() + transition.len());
    out.push_str(&xml[..at]);
    out.push_str(transition);
    out.push_str(&xml[at..]);
    Ok(out)
}

const AUDIO_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/audio";

/// A relationship id from the slide to `part` (an existing audio part),
/// adding the relationship when the slide has none.
pub(crate) fn audio_relationship(
    package: &mut OpcPackage,
    slide_part: &str,
    part: &str,
) -> Result<String> {
    let rel_part = rels_part_name(slide_part);
    let old = package.part_str(&rel_part)?.unwrap_or_else(|| {
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>"
            .into()
    });
    let nodes = parse_document_nodes(&old)?;
    let mut ids = HashSet::new();
    for node in &nodes {
        if node.is(4, "Relationship") {
            if let Some(id) = node.attr("Id") {
                ids.insert(id.to_string());
            }
            if node.attr("Type").is_some_and(|t| t.ends_with("/audio"))
                && node
                    .attr("Target")
                    .is_some_and(|target| resolve_target(slide_part, target) == part)
            {
                if let Some(id) = node.attr("Id") {
                    return Ok(id.to_string());
                }
            }
        }
    }
    let root = nodes
        .first()
        .filter(|node| node.is(4, "Relationships"))
        .ok_or_else(|| WriteError::Xml("missing relationships root".into()))?;
    let rid = (1..)
        .map(|n| format!("rIdJadeSound{n}"))
        .find(|rid| !ids.contains(rid))
        .expect("an unused id");
    let relation = format!(
        "<jadePkg:Relationship xmlns:jadePkg=\"http://schemas.openxmlformats.org/package/2006/relationships\" Id=\"{rid}\" Type=\"{AUDIO_REL}\" Target=\"/{part}\"/>"
    );
    package.replace(
        &rel_part,
        slides::append_child(&old, root, &relation).into_bytes(),
    );
    Ok(rid)
}

/// Adds a sound file to the package as `ppt/media/…`, returning its part.
pub(crate) fn install_sound(package: &mut OpcPackage, name: &str, bytes: &[u8]) -> Result<String> {
    let extension = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .filter(|ext| matches!(ext.as_str(), "wav" | "mp3" | "m4a" | "aif" | "aiff" | "wma"))
        .unwrap_or_else(|| "wav".to_string());
    let mime = match extension.as_str() {
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "aif" | "aiff" => "audio/x-aiff",
        "wma" => "audio/x-ms-wma",
        _ => "audio/wav",
    };
    let existing: HashSet<String> = package.part_names().iter().cloned().collect();
    let part = (1..)
        .map(|n| format!("ppt/media/jade-sound{n}.{extension}"))
        .find(|part| !existing.contains(part))
        .expect("an unused part name");
    package.replace(&part, bytes.to_vec());
    slides::content_types(
        package,
        &BTreeMap::new(),
        &BTreeSet::new(),
        Some((&part, mime)),
    )?;
    Ok(part)
}
