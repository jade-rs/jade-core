//! Legacy worksheet comments (called "notes" by current Excel).

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use crate::error::Result;
use crate::model::{CellComment, CellRef};
use jade_opc::local_name;

/// Reads a comments part and flattens rich-text runs to the text a headless
/// caller can consume. Formatting stays in the untouched package for writes.
pub fn parse_comments(xml: &str) -> Result<Vec<CellComment>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut authors = Vec::new();
    let mut author = String::new();
    let mut in_author = false;
    let mut comment: Option<(CellRef, Option<usize>)> = None;
    let mut comment_text = String::new();
    let mut in_comment_text = false;
    let mut comments = Vec::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(ref element) => match local_name(element.name().as_ref()) {
                b"author" => {
                    author.clear();
                    in_author = true;
                }
                b"comment" => {
                    if let Some(reference) = attribute_value(element, b"ref")? {
                        comment = Some((
                            CellRef::parse(&reference)?,
                            attribute_value(element, b"authorId")?
                                .and_then(|value| value.parse().ok()),
                        ));
                        comment_text.clear();
                    }
                }
                b"t" if comment.is_some() => in_comment_text = true,
                _ => {}
            },
            Event::Text(ref text) => {
                let decoded = text.decode()?;
                if in_author {
                    author.push_str(&decoded);
                } else if in_comment_text {
                    comment_text.push_str(&decoded);
                }
            }
            Event::GeneralRef(ref reference) => {
                let Some(decoded) = decoded_reference(reference) else {
                    continue;
                };
                if in_author {
                    author.push_str(&decoded);
                } else if in_comment_text {
                    comment_text.push_str(&decoded);
                }
            }
            Event::End(ref element) => match local_name(element.name().as_ref()) {
                b"author" => {
                    authors.push(author.clone());
                    in_author = false;
                }
                b"t" => in_comment_text = false,
                b"comment" => {
                    if let Some((reference, author_id)) = comment.take() {
                        comments.push(CellComment {
                            reference,
                            author: author_id.and_then(|index| authors.get(index).cloned()),
                            text: crate::xstring::decode_xstring(&comment_text),
                        });
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(comments)
}

fn attribute_value(element: &BytesStart<'_>, key: &[u8]) -> Result<Option<String>> {
    for attribute in element.attributes() {
        let attribute = attribute?;
        if local_name(attribute.key.as_ref()) == key {
            return Ok(Some(attribute.unescape_value()?.into_owned()));
        }
    }
    Ok(None)
}

fn decoded_reference(reference: &quick_xml::events::BytesRef<'_>) -> Option<String> {
    let name = reference.decode().ok()?;
    quick_xml::escape::unescape(&format!("&{name};"))
        .ok()
        .map(|text| text.into_owned())
}
