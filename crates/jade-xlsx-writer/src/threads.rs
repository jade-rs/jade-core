//! Replies to a threaded comment (a "comment" in current Excel): a
//! `threadedComment` naming the thread's first one as its parent, added
//! after the thread's last, its author found in — or added to — the
//! workbook's `persons` part.

use std::hash::{Hash, Hasher};

use jade_opc::OpcPackage;
use jade_xlsx_reader::CellRef;
use quick_xml::events::Event;
use quick_xml::Reader;

use crate::notes::{add_relationship, ensure_override, relative_target};
use crate::patch::{escape, local};
use crate::{Result, WriteError};

const THREAD_TYPE: &str =
    "http://schemas.microsoft.com/office/2017/10/relationships/threadedComment";
const PERSON_TYPE: &str = "http://schemas.microsoft.com/office/2017/10/relationships/person";
const PERSON_CONTENT: &str = "application/vnd.ms-excel.person+xml";
const THREADS_NS: &str = "http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments";

/// An id as Excel writes them, `{8-4-4-4-12}` in hex, made from `seed`.
fn guid(seed: &str) -> String {
    let mut parts = [0u64; 2];
    for (salt, part) in parts.iter_mut().enumerate() {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (seed, salt).hash(&mut hasher);
        *part = hasher.finish();
    }
    let hex = format!("{:016X}{:016X}", parts[0], parts[1]);
    format!(
        "{{{}-{}-{}-{}-{}}}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn attribute(element: &quick_xml::events::BytesStart, name: &[u8]) -> Option<String> {
    element
        .attributes()
        .flatten()
        .find(|attribute| attribute.key.local_name().as_ref() == name)
        .and_then(|attribute| {
            attribute
                .unescape_value()
                .ok()
                .map(|value| value.into_owned())
        })
}

/// The thread on `at` in a threadedComments part: its first comment's id
/// and where the thread's last element ends.
fn thread_at(xml: &str, at: CellRef) -> Result<Option<(String, usize)>> {
    let wanted = crate::shift::reference_text(at);
    let mut reader = Reader::from_str(xml);
    let mut root: Option<String> = None;
    let mut end = None;
    let mut depth = 0usize;
    let mut current: Option<(String, Option<String>)> = None;
    loop {
        let event = reader.read_event()?;
        match &event {
            Event::Start(element) | Event::Empty(element)
                if local(element.name().as_ref()) == b"threadedComment" =>
            {
                let reference = attribute(element, b"ref").unwrap_or_default();
                let id = attribute(element, b"id").unwrap_or_default();
                let parent = attribute(element, b"parentId");
                if reference.eq_ignore_ascii_case(&wanted) {
                    current = Some((id, parent));
                    if matches!(event, Event::Empty(_)) {
                        if let Some((id, parent)) = current.take() {
                            if parent.is_none() && root.is_none() {
                                root = Some(id);
                            }
                            end = Some(reader.buffer_position() as usize);
                        }
                    } else {
                        depth = 1;
                    }
                }
            }
            Event::Start(_) if current.is_some() => depth += 1,
            Event::End(element) if current.is_some() => {
                depth -= 1;
                if depth == 0 && local(element.name().as_ref()) == b"threadedComment" {
                    if let Some((id, parent)) = current.take() {
                        if parent.is_none() && root.is_none() {
                            root = Some(id);
                        }
                        end = Some(reader.buffer_position() as usize);
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(root.zip(end))
}

/// The id of the person named `author` in the workbook's persons part,
/// added when there is none (and the part, too, when the workbook has
/// none).
fn person(package: &mut OpcPackage, author: &str) -> Result<String> {
    let existing = package
        .part_names()
        .iter()
        .find(|name| name.starts_with("xl/persons/") && name.ends_with(".xml"))
        .cloned();
    let id = guid(&format!("person:{author}"));
    let entry = format!(
        "<person displayName=\"{}\" id=\"{id}\" userId=\"{}\" providerId=\"None\"/>",
        escape(author),
        escape(author)
    );
    match existing {
        Some(part) => {
            let xml = package
                .part_str(&part)?
                .ok_or_else(|| WriteError::MissingPart(part.clone()))?;
            let people = jade_xlsx_reader::comments::parse_persons(&xml)?;
            if let Some((id, _)) = people.iter().find(|(_, name)| name.as_str() == author) {
                return Ok(id.clone());
            }
            let close = xml
                .rfind("</")
                .filter(|at| xml[*at..].contains("personList"))
                .ok_or_else(|| WriteError::MissingPart(part.clone()))?;
            let patched = format!("{}{entry}{}", &xml[..close], &xml[close..]);
            package.replace(&part, patched.into_bytes());
        }
        None => {
            let part = "xl/persons/person.xml".to_string();
            let xml = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<personList xmlns=\"{THREADS_NS}\" xmlns:x=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">{entry}</personList>"
            );
            package.replace(&part, xml.into_bytes());
            ensure_override(package, &part, PERSON_CONTENT)?;
            let workbook = crate::workbook_part_name(package)?;
            let rels_name = jade_opc::rels_part_name(&workbook);
            let rels = package.part_str(&rels_name)?;
            let (rels, _) = add_relationship(
                rels.as_deref(),
                PERSON_TYPE,
                &relative_target(&workbook, &part),
            )?;
            package.replace(&rels_name, rels.into_bytes());
        }
    }
    Ok(id)
}

/// A reply by `author` to the threaded comment on `at`, dated `when`
/// (`YYYY-MM-DDTHH:MM:SS.00`). `false` when the cell has no thread.
pub fn reply_to_thread(
    package: &mut OpcPackage,
    sheet_part: &str,
    at: CellRef,
    author: &str,
    text: &str,
    when: &str,
) -> Result<bool> {
    let rels = package
        .part_str(&jade_opc::rels_part_name(sheet_part))?
        .map(|xml| jade_opc::parse_relationships(&xml))
        .transpose()?
        .unwrap_or_default();
    let Some(target) = rels
        .all()
        .into_iter()
        .find(|r| !r.external && r.relationship_type == THREAD_TYPE)
        .map(|r| r.target.clone())
    else {
        return Ok(false);
    };
    let part = jade_opc::resolve_target(sheet_part, &target);
    let xml = package
        .part_str(&part)?
        .ok_or_else(|| WriteError::MissingPart(part.clone()))?;
    let Some((root, end)) = thread_at(&xml, at)? else {
        return Ok(false);
    };
    let person = person(package, author)?;
    let id = guid(&format!("reply:{root}:{when}:{text}:{}", xml.len()));
    let reply = format!(
        "<threadedComment ref=\"{}\" dT=\"{when}\" personId=\"{person}\" id=\"{id}\" parentId=\"{root}\"><text>{}</text></threadedComment>",
        crate::shift::reference_text(at),
        escape(text)
    );
    let patched = format!("{}{reply}{}", &xml[..end], &xml[end..]);
    package.replace(&part, patched.into_bytes());
    Ok(true)
}
