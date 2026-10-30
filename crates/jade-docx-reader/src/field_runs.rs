//! The semantic model assigns one field to a run. WordprocessingML permits
//! field boundaries anywhere among a run's children. Split those boundaries
//! before both consumers parse the XML, keeping their run identities aligned.
use std::{borrow::Cow, ops::Range};

use quick_xml::{events::Event, Reader};

struct Run {
    depth: usize,
    opening: Range<usize>,
    properties: Option<Range<usize>>,
    child: Option<(usize, bool, bool)>,
    children: Vec<(Range<usize>, bool)>,
}

/// Split mixed field-marker runs in namespace-normalized WordprocessingML.
/// All existing bytes remain in order; new runs copy the source attributes
/// and run properties. Already separated runs remain borrowed and unchanged.
pub fn normalize(xml: &str) -> quick_xml::Result<Cow<'_, str>> {
    if !xml.contains("w:fldChar") {
        return Ok(Cow::Borrowed(xml));
    }
    let mut reader = Reader::from_str(xml);
    let mut depth = 0;
    let mut runs: Vec<Run> = Vec::new();
    let mut insertions = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        let end = reader.buffer_position() as usize;
        match event {
            Event::Start(element) => {
                if let Some(run) = runs.last_mut().filter(|run| depth == run.depth) {
                    run.child = Some((
                        start,
                        element.name().as_ref() == b"w:fldChar",
                        element.name().as_ref() == b"w:rPr",
                    ));
                }
                depth += 1;
                if element.name().as_ref() == b"w:r" {
                    runs.push(Run {
                        depth,
                        opening: start..end,
                        properties: None,
                        child: None,
                        children: Vec::new(),
                    });
                }
            }
            Event::Empty(element) => {
                if let Some(run) = runs.last_mut().filter(|run| depth == run.depth) {
                    if element.name().as_ref() == b"w:rPr" {
                        run.properties = Some(start..end);
                    } else {
                        run.children
                            .push((start..end, element.name().as_ref() == b"w:fldChar"));
                    }
                }
            }
            Event::End(element) => {
                if element.name().as_ref() == b"w:r" {
                    if let Some(run) = runs.pop() {
                        let mut opening = String::from("</w:r>");
                        opening.push_str(&xml[run.opening]);
                        if let Some(properties) = run.properties {
                            opening.push_str(&xml[properties]);
                        }
                        for pair in run.children.windows(2) {
                            if pair[0].1 || pair[1].1 {
                                insertions.push((pair[1].0.start, opening.clone()));
                            }
                        }
                    }
                }
                depth -= 1;
                if let Some(run) = runs.last_mut().filter(|run| depth == run.depth) {
                    if let Some((start, marker, properties)) = run.child.take() {
                        if properties {
                            run.properties = Some(start..end);
                        } else {
                            run.children.push((start..end, marker));
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if insertions.is_empty() {
        return Ok(Cow::Borrowed(xml));
    }
    insertions.sort_by_key(|(offset, _)| *offset);
    let mut result = String::with_capacity(
        xml.len() + insertions.iter().map(|(_, text)| text.len()).sum::<usize>(),
    );
    let mut previous = 0;
    for (offset, text) in insertions {
        result.push_str(&xml[previous..offset]);
        result.push_str(&text);
        previous = offset;
    }
    result.push_str(&xml[previous..]);
    Ok(Cow::Owned(result))
}

#[cfg(test)]
mod tests {
    use crate::{docx_xml::attach_document_xml_ids, BodyElement, DocumentParser};

    #[test]
    fn mixed_field_runs_keep_text_properties_and_raw_bindings() {
        // Exercise expanded and empty markers with surrounding text.
        let xml = concat!(
            "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body><w:p>",
            "<w:r w:rsidR=\"00112233\"><w:rPr><w:b/></w:rPr><w:t>A</w:t>",
            "<w:fldChar w:fldCharType=\"begin\"></w:fldChar><w:instrText> NUMPAGES </w:instrText>",
            "<w:fldChar w:fldCharType=\"separate\"/><w:t>old</w:t><w:fldChar w:fldCharType=\"end\"/><w:t>Z</w:t></w:r>",
            "</w:p></w:body></w:document>"
        );
        let parser = DocumentParser::new();
        let (mut document, raw) = parser.parse_document_xml_with_raw_tree(xml).unwrap();
        let raw = raw.unwrap();
        attach_document_xml_ids(&mut document, &raw).unwrap();
        let BodyElement::Paragraph(paragraph) = &document.body.unwrap().elements[0] else {
            panic!("paragraph");
        };
        assert_eq!(
            paragraph
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>(),
            "AZ"
        );
        assert_eq!(
            paragraph
                .runs
                .iter()
                .filter_map(|run| run.field.as_ref())
                .count(),
            1
        );
        assert_eq!(
            paragraph
                .runs
                .iter()
                .find_map(|run| run.field.as_ref())
                .unwrap()
                .cached_text
                .as_deref(),
            Some("old")
        );
        for run in &paragraph.runs {
            let element = raw.element(run.node_id.expect("mapped run")).unwrap();
            assert!(element
                .attributes
                .iter()
                .any(|a| a.name == "w:rsidR" && a.value == "00112233"));
            assert!(run
                .properties
                .as_ref()
                .is_some_and(|properties| properties.bold == Some(true)));
        }
        let saved = raw.to_string().unwrap();
        assert!(matches!(
            super::normalize(&saved).unwrap(),
            std::borrow::Cow::Borrowed(_)
        ));
        let ordinary = xml
            .replace("<w:fldChar", "<w:other")
            .replace("</w:fldChar", "</w:other");
        assert!(matches!(
            super::normalize(&ordinary).unwrap(),
            std::borrow::Cow::Borrowed(_)
        ));
    }
}
