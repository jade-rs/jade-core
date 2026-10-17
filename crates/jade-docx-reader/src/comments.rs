use crate::document::{BodyElement, DocumentComment, Paragraph};
use crate::parser::DocumentParser;
use quick_xml::events::Event;
use quick_xml::{Reader, Writer};
use std::collections::HashMap;
use std::io::BufRead;

const COMMENT_DOC_PREFIX: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <w:body>
"#;

const COMMENT_DOC_SUFFIX: &str = r#"
  </w:body>
</w:document>
"#;

/// Parsed representation of `word/comments.xml`.
#[derive(Debug, Default)]
pub struct CommentsPart {
    pub comments: HashMap<String, DocumentComment>,
    comment_para_ids: HashMap<String, Vec<String>>,
}

impl CommentsPart {
    pub fn from_xml(
        parser: &DocumentParser,
        xml: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(false);

        let mut buf = Vec::new();
        let mut comments = HashMap::new();
        let mut comment_para_ids = HashMap::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) if e.name().as_ref() == b"w:comment" => {
                    let mut id = String::new();
                    let mut author: Option<String> = None;
                    let mut initials: Option<String> = None;
                    let mut date: Option<String> = None;
                    let mut resolved: Option<bool> = None;

                    for attr in e.attributes().flatten() {
                        let key = attr.key.as_ref();
                        let value = String::from_utf8_lossy(&attr.value).to_string();
                        match key {
                            b"w:id" => id = value,
                            b"w:author" => author = normalize_attr(value),
                            b"w:initials" => initials = normalize_attr(value),
                            b"w:date" => date = normalize_attr(value),
                            b"w15:done" | b"w16cex:done" | b"done" => {
                                resolved = parse_done_attr(&value)
                            }
                            _ => {}
                        }
                    }

                    let inner_xml = Self::collect_inner_xml(&mut reader)?;
                    let text = Self::extract_plain_text(parser, &inner_xml)?;
                    let para_ids = Self::collect_comment_para_ids(&inner_xml)?;

                    if !id.is_empty() {
                        comment_para_ids.insert(id.clone(), para_ids);
                        comments.insert(
                            id.clone(),
                            DocumentComment {
                                id,
                                author,
                                initials,
                                date,
                                resolved,
                                thread_parent_id: None,
                                text,
                            },
                        );
                    }
                }
                Ok(Event::Eof) => break,
                Err(err) => return Err(err.into()),
                _ => {}
            }
            buf.clear();
        }

        Ok(Self {
            comments,
            comment_para_ids,
        })
    }

    pub fn apply_comments_extended_xml(
        &mut self,
        xml: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let entries = Self::parse_comments_extended_entries(xml)?;
        if entries.is_empty() {
            return Ok(());
        }

        let mut para_to_comment_id: HashMap<String, String> = HashMap::new();
        for (comment_id, para_ids) in &self.comment_para_ids {
            for para_id in para_ids {
                para_to_comment_id.insert(para_id.clone(), comment_id.clone());
            }
        }

        for entry in entries {
            let Some(comment_id) = para_to_comment_id.get(&entry.para_id) else {
                continue;
            };
            if let Some(done) = entry.done {
                if let Some(comment) = self.comments.get_mut(comment_id) {
                    comment.resolved = Some(done);
                }
            }
            if let Some(parent_para_id) = entry.parent_para_id {
                let Some(parent_comment_id) = para_to_comment_id.get(&parent_para_id) else {
                    continue;
                };
                if parent_comment_id == comment_id {
                    continue;
                }
                if let Some(comment) = self.comments.get_mut(comment_id) {
                    comment.thread_parent_id = Some(parent_comment_id.clone());
                }
            }
        }

        Ok(())
    }

    fn collect_inner_xml<R: BufRead>(
        reader: &mut Reader<R>,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let mut depth = 1usize;
        let mut writer = Writer::new(Vec::new());
        let mut buf = Vec::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    depth += 1;
                    writer.write_event(Event::Start(e.to_owned()))?;
                }
                Ok(Event::Empty(ref e)) => {
                    writer.write_event(Event::Empty(e.to_owned()))?;
                }
                Ok(Event::Text(ref e)) => {
                    writer.write_event(Event::Text(e.to_owned()))?;
                }
                Ok(Event::CData(ref e)) => {
                    writer.write_event(Event::CData(e.to_owned()))?;
                }
                Ok(Event::Comment(ref e)) => {
                    writer.write_event(Event::Comment(e.to_owned()))?;
                }
                Ok(Event::End(ref e)) => {
                    if depth == 1 {
                        break;
                    }
                    depth -= 1;
                    writer.write_event(Event::End(e.to_owned()))?;
                }
                Ok(Event::Eof) => break,
                Err(err) => return Err(err.into()),
                _ => {}
            }
            buf.clear();
        }

        let bytes = writer.into_inner();
        Ok(String::from_utf8(bytes)?)
    }

    fn collect_comment_para_ids(xml: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(false);

        let mut buf = Vec::new();
        let mut para_ids = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e))
                    if is_xml_name(e.name().as_ref(), b"p") =>
                {
                    for attr in e.attributes().flatten() {
                        if !is_xml_name(attr.key.as_ref(), b"paraId") {
                            continue;
                        }
                        let value = String::from_utf8_lossy(&attr.value).trim().to_string();
                        if value.is_empty() {
                            continue;
                        }
                        if !para_ids.iter().any(|existing| existing == &value) {
                            para_ids.push(value);
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Err(err) => return Err(err.into()),
                _ => {}
            }
            buf.clear();
        }
        Ok(para_ids)
    }

    fn parse_comments_extended_entries(
        xml: &str,
    ) -> Result<Vec<CommentExtendedEntry>, Box<dyn std::error::Error>> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(false);

        let mut buf = Vec::new();
        let mut entries = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e))
                    if is_xml_name(e.name().as_ref(), b"commentEx") =>
                {
                    let mut para_id = None;
                    let mut parent_para_id = None;
                    let mut done = None;
                    for attr in e.attributes().flatten() {
                        let key = attr.key.as_ref();
                        let value = String::from_utf8_lossy(&attr.value).to_string();
                        if is_xml_name(key, b"paraId") {
                            para_id = normalize_attr(value);
                        } else if is_xml_name(key, b"paraIdParent") {
                            parent_para_id = normalize_attr(value);
                        } else if is_xml_name(key, b"done") {
                            done = parse_done_attr(&value);
                        }
                    }
                    if let Some(para_id) = para_id {
                        entries.push(CommentExtendedEntry {
                            para_id,
                            parent_para_id,
                            done,
                        });
                    }
                }
                Ok(Event::Eof) => break,
                Err(err) => return Err(err.into()),
                _ => {}
            }
            buf.clear();
        }
        Ok(entries)
    }

    fn extract_plain_text(
        parser: &DocumentParser,
        inner_xml: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        if inner_xml.trim().is_empty() {
            return Ok(String::new());
        }

        let wrapped = format!("{COMMENT_DOC_PREFIX}{inner_xml}{COMMENT_DOC_SUFFIX}");
        let document = parser.parse_document_xml(&wrapped)?;

        let mut lines = Vec::new();
        if let Some(body) = document.body {
            for element in body.elements {
                Self::collect_text(&element, &mut lines);
            }
        }

        Ok(lines.join("\n"))
    }

    fn collect_text(element: &BodyElement, lines: &mut Vec<String>) {
        match element {
            BodyElement::Paragraph(paragraph) => {
                if let Some(text) = Self::paragraph_text(paragraph) {
                    lines.push(text);
                }
            }
            BodyElement::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        for block in &cell.blocks {
                            Self::collect_text(block, lines);
                        }
                    }
                }
            }
            BodyElement::Math(_) => {
                // コメントのテキスト化ではブロック数式を無視
            }
        }
    }

    fn paragraph_text(paragraph: &Paragraph) -> Option<String> {
        let mut text = String::new();

        for run in &paragraph.runs {
            if run.text.is_empty() {
                continue;
            }
            let trimmed = run.text.trim();
            if trimmed.is_empty() {
                continue;
            }

            let first_char = trimmed.chars().next();
            let needs_space = !text.is_empty()
                && !text.ends_with(char::is_whitespace)
                && !trimmed.starts_with(char::is_whitespace)
                && !matches!(
                    first_char,
                    Some(',')
                        | Some('.')
                        | Some('!')
                        | Some('?')
                        | Some(';')
                        | Some(':')
                        | Some(')')
                        | Some(']')
                );
            if needs_space {
                text.push(' ');
            }

            text.push_str(trimmed);
        }

        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }
}

#[derive(Debug, Clone)]
struct CommentExtendedEntry {
    para_id: String,
    parent_para_id: Option<String>,
    done: Option<bool>,
}

fn is_xml_name(full_name: &[u8], local_name: &[u8]) -> bool {
    full_name == local_name
        || full_name
            .split(|ch| *ch == b':')
            .next_back()
            .is_some_and(|name| name == local_name)
}

fn normalize_attr(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn parse_done_attr(value: &str) -> Option<bool> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        None
    } else {
        Some(matches!(normalized.as_str(), "1" | "true" | "on"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comment_entries() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:comments xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml">
  <w:comment w:id="1" w:author="Alice" w:initials="AL" w:date="2024-01-01T10:00:00Z" w15:done="1">
    <w:p>
      <w:r><w:t>Hello</w:t></w:r>
      <w:r><w:t> world</w:t></w:r>
    </w:p>
  </w:comment>
  <w:comment w:id="2">
    <w:p><w:r><w:t>Second</w:t></w:r></w:p>
    <w:tbl>
      <w:tr>
        <w:tc>
          <w:p><w:r><w:t>Cell text</w:t></w:r></w:p>
        </w:tc>
      </w:tr>
    </w:tbl>
  </w:comment>
</w:comments>
"#;

        let parser = DocumentParser::new();
        let part = CommentsPart::from_xml(&parser, xml).expect("parse comments");

        assert_eq!(part.comments.len(), 2);

        let first = part.comments.get("1").expect("comment 1");
        assert_eq!(first.author.as_deref(), Some("Alice"));
        assert_eq!(first.initials.as_deref(), Some("AL"));
        assert_eq!(first.date.as_deref(), Some("2024-01-01T10:00:00Z"));
        assert_eq!(first.resolved, Some(true));
        assert_eq!(first.thread_parent_id, None);
        assert_eq!(first.text, "Hello world");

        let second = part.comments.get("2").expect("comment 2");
        assert!(second.author.is_none());
        assert_eq!(second.resolved, None);
        assert_eq!(second.thread_parent_id, None);
        assert_eq!(second.text, "Second\nCell text");
    }

    #[test]
    fn applies_comments_extended_entries_to_comments() {
        let comments_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:comments xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml">
  <w:comment w:id="0">
    <w:p w14:paraId="AAAA1111"><w:r><w:t>Root</w:t></w:r></w:p>
  </w:comment>
  <w:comment w:id="1">
    <w:p w14:paraId="BBBB2222"><w:r><w:t>Reply</w:t></w:r></w:p>
    <w:p w14:paraId="CCCC3333"><w:r><w:t/></w:r></w:p>
  </w:comment>
</w:comments>"#;
        let comments_ex_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w15:commentsEx xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml">
  <w15:commentEx w15:paraId="AAAA1111" w15:done="0"/>
  <w15:commentEx w15:paraId="CCCC3333" w15:paraIdParent="AAAA1111" w15:done="1"/>
</w15:commentsEx>"#;

        let parser = DocumentParser::new();
        let mut part = CommentsPart::from_xml(&parser, comments_xml).expect("parse comments");
        part.apply_comments_extended_xml(comments_ex_xml)
            .expect("apply comments extended");

        let root = part.comments.get("0").expect("root comment");
        assert_eq!(root.resolved, Some(false));
        assert_eq!(root.thread_parent_id, None);

        let reply = part.comments.get("1").expect("reply comment");
        assert_eq!(reply.resolved, Some(true));
        assert_eq!(reply.thread_parent_id.as_deref(), Some("0"));
    }
}
