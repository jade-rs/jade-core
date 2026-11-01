//! Legacy worksheet comments (called "notes" by current Excel).
use crate::{model::CellComment, Result};
/// Read only owned comments/authors. Rich text uses the same namespace/MCE,
/// entity, CDATA and phonetic-value policy as ordinary shared strings.
pub fn parse_comments(xml: &str) -> Result<Vec<CellComment>> {
    let projected = crate::cell_content::comments(xml)?;
    let source = crate::object_protection::comments_source(&projected)?;
    Ok(source
        .comments
        .into_iter()
        .filter_map(|c| {
            c.at.map(|reference| CellComment {
                reference,
                author: c.author_id.and_then(|i| source.authors.get(i).cloned()),
                text: c.value,
            })
        })
        .collect())
}

/// One threaded comment (a "comment" in current Excel) or reply.
struct ThreadEntry {
    reference: crate::CellRef,
    id: String,
    parent: Option<String>,
    person: Option<String>,
    text: String,
}

/// The display names in a `persons` part, by person id.
pub fn parse_persons(xml: &str) -> Result<std::collections::HashMap<String, String>> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut people = std::collections::HashMap::new();
    loop {
        match reader.read_event()? {
            Event::Start(element) | Event::Empty(element)
                if element.local_name().as_ref() == b"person" =>
            {
                let mut id = None;
                let mut name = None;
                for attribute in element.attributes().flatten() {
                    let value = attribute.unescape_value()?.into_owned();
                    match attribute.key.local_name().as_ref() {
                        b"id" => id = Some(value),
                        b"displayName" => name = Some(value),
                        _ => {}
                    }
                }
                if let (Some(id), Some(name)) = (id, name) {
                    people.insert(id, name);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(people)
}

/// The threads of a `threadedComments` part put in place of the notes Excel
/// writes beside them for older versions ("[Threaded comment] Your version
/// of Excel allows you to read this threaded comment…"): each cell's
/// comment and replies, in order, one `Name: text` line each, the comment's
/// author its author.
pub fn apply_threads(
    comments: &mut Vec<CellComment>,
    xml: &str,
    people: &std::collections::HashMap<String, String>,
) -> Result<()> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut entries: Vec<ThreadEntry> = Vec::new();
    let mut current: Option<ThreadEntry> = None;
    let mut in_text = false;
    loop {
        match reader.read_event()? {
            Event::Start(element) if element.local_name().as_ref() == b"threadedComment" => {
                let mut entry = ThreadEntry {
                    reference: crate::CellRef::new(0, 0),
                    id: String::new(),
                    parent: None,
                    person: None,
                    text: String::new(),
                };
                let mut placed = false;
                for attribute in element.attributes().flatten() {
                    let value = attribute.unescape_value()?.into_owned();
                    match attribute.key.local_name().as_ref() {
                        b"ref" => {
                            if let Ok(at) = crate::CellRef::parse(&value) {
                                entry.reference = at;
                                placed = true;
                            }
                        }
                        b"id" => entry.id = value,
                        b"parentId" => entry.parent = Some(value),
                        b"personId" => entry.person = Some(value),
                        _ => {}
                    }
                }
                current = placed.then_some(entry);
            }
            Event::Start(element)
                if current.is_some() && element.local_name().as_ref() == b"text" =>
            {
                in_text = true;
            }
            Event::Text(text) if in_text => {
                if let Some(entry) = current.as_mut() {
                    entry.text.push_str(&text.decode()?);
                }
            }
            Event::GeneralRef(reference) if in_text => {
                if let Some(entry) = current.as_mut() {
                    if let Some(character) = reference.resolve_char_ref()? {
                        entry.text.push(character);
                    } else if let Some(value) =
                        quick_xml::escape::resolve_predefined_entity(&reference.decode()?)
                    {
                        entry.text.push_str(value);
                    }
                }
            }
            Event::End(element) if element.local_name().as_ref() == b"text" => in_text = false,
            Event::End(element) if element.local_name().as_ref() == b"threadedComment" => {
                entries.extend(current.take());
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let name = |entry: &ThreadEntry| {
        entry
            .person
            .as_ref()
            .and_then(|person| people.get(person))
            .cloned()
    };
    for root in entries.iter().filter(|entry| entry.parent.is_none()) {
        let replies = entries
            .iter()
            .filter(|entry| entry.parent.as_deref() == Some(root.id.as_str()));
        let text = std::iter::once(root)
            .chain(replies)
            .map(|entry| match name(entry) {
                Some(name) => format!("{name}: {}", entry.text),
                None => entry.text.clone(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        let thread = CellComment {
            reference: root.reference,
            author: name(root),
            text,
        };
        match comments
            .iter_mut()
            .find(|comment| comment.reference == root.reference)
        {
            Some(note) => *note = thread,
            None => comments.push(thread),
        }
    }
    Ok(())
}
