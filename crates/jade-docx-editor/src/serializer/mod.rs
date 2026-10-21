//! DOCX 書き戻し用のシリアライザ。
//!
//! 現状は最小限の段落/ランのみを対応しており、スタイルや画像などの高度な要素は
//! 今後のイテレーションで拡張する予定である。仕様の詳細は ECMA-376 4th edition
//! Part 1 を参照。

use std::fmt::Write;

use crate::document::{BodyElement, Document, Paragraph, Run, PAGE_BREAK_CHAR};

/// `word/document.xml` を生成するシリアライザ。
///
/// `DocumentParser` で得た `Document` を OpenXML の XML 文字列に変換する。
/// 今後は `DocxNode` ベースの差分適用を扱う予定だが、初期段階では
/// 段落・テキストランのみを書き戻す。
#[derive(Default)]
pub struct DocxSerializer;

impl DocxSerializer {
    pub fn new() -> Self {
        Self
    }

    /// `Document` から `word/document.xml` を生成する。
    ///
    /// # 制限
    /// - 段落とプレーンテキストランのみを出力。
    /// - スタイル属性、画像、テーブル、フィールド等は未対応。
    pub fn serialize_document(
        &self,
        document: &Document,
    ) -> Result<SerializedMainDocument, SerializeError> {
        let mut buffer = String::new();
        write!(&mut buffer, r#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
        write!(
            &mut buffer,
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">"#
        )?;
        write!(&mut buffer, "<w:body>")?;

        if let Some(body) = document.body.as_ref() {
            for element in &body.elements {
                self.serialize_body_element(element, &mut buffer)?;
            }
        }

        write!(&mut buffer, "</w:body></w:document>")?;

        Ok(SerializedMainDocument { xml: buffer })
    }

    fn serialize_body_element(
        &self,
        element: &BodyElement,
        buffer: &mut String,
    ) -> Result<(), SerializeError> {
        match element {
            BodyElement::Paragraph(paragraph) => self.serialize_paragraph(paragraph, buffer),
            BodyElement::Table(_) => Err(SerializeError::Unsupported("table element")),
            BodyElement::Math(_) => Err(SerializeError::Unsupported("math paragraph")),
        }
    }

    fn serialize_paragraph(
        &self,
        paragraph: &Paragraph,
        buffer: &mut String,
    ) -> Result<(), SerializeError> {
        write!(buffer, "<w:p>")?;

        for run in &paragraph.runs {
            self.serialize_run(run, buffer)?;
        }

        write!(buffer, "</w:p>")?;
        Ok(())
    }

    fn serialize_run(&self, run: &Run, buffer: &mut String) -> Result<(), SerializeError> {
        if run.image.is_some() || run.text_box.is_some() || run.field.is_some() {
            return Err(SerializeError::Unsupported("non-text run"));
        }

        if run.text.is_empty() {
            return Ok(());
        }

        write!(buffer, "<w:r>")?;
        let mut segment = String::new();

        let flush_text =
            |buffer: &mut String, segment: &mut String| -> Result<(), SerializeError> {
                if segment.is_empty() {
                    return Ok(());
                }
                write!(buffer, "<w:t>{}</w:t>", xml_escape(segment))?;
                segment.clear();
                Ok(())
            };

        for ch in run.text.chars() {
            match ch {
                '\n' => {
                    flush_text(buffer, &mut segment)?;
                    write!(buffer, r#"<w:br w:type="textWrapping"/>"#)?;
                }
                PAGE_BREAK_CHAR => {
                    flush_text(buffer, &mut segment)?;
                    write!(buffer, r#"<w:br w:type="page"/>"#)?;
                }
                crate::document::COLUMN_BREAK_CHAR => {
                    flush_text(buffer, &mut segment)?;
                    write!(buffer, r#"<w:br w:type="column"/>"#)?;
                }
                _ => segment.push(ch),
            }
        }

        flush_text(buffer, &mut segment)?;
        write!(buffer, "</w:r>")?;

        Ok(())
    }
}

/// `word/document.xml` を表す最小構造体。
pub struct SerializedMainDocument {
    pub xml: String,
}

/// シリアライズ時のエラー。
#[derive(Debug)]
pub enum SerializeError {
    Fmt(std::fmt::Error),
    Unsupported(&'static str),
}

impl From<std::fmt::Error> for SerializeError {
    fn from(value: std::fmt::Error) -> Self {
        Self::Fmt(value)
    }
}

impl std::fmt::Display for SerializeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SerializeError::Fmt(err) => write!(f, "formatting error: {err}"),
            SerializeError::Unsupported(feature) => {
                write!(f, "unsupported element encountered: {feature}")
            }
        }
    }
}

impl std::error::Error for SerializeError {}

fn xml_escape(text: &str) -> String {
    let mut escaped = String::new();
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Body, BodyElement, Document, Paragraph, Run, PAGE_BREAK_STR};

    fn sample_document(text: &str) -> Document {
        Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(Paragraph {
                    runs: vec![Run {
                        text: text.to_string(),
                        properties: None,
                        image: None,
                        text_box: None,
                        math: None,
                        field: None,
                        hyperlink: None,
                        node_id: None,
                        text_node_id: None,
                        revision_node_id: None,
                        comment_reference_ids: Vec::new(),
                        footnote_reference_ids: Vec::new(),
                        endnote_reference_ids: Vec::new(),
                        note_separator: None,
                        has_footnote_ref: false,
                        has_endnote_ref: false,
                        revision: None,
                    }],
                    properties: None,
                    node_id: None,
                    comment_ranges: Vec::new(),
                    comment_markers: Vec::new(),
                })],
            }),
            ..Document::default()
        }
    }

    #[test]
    fn serializes_plain_paragraph() {
        let document = sample_document("Hello");
        let serializer = DocxSerializer::new();
        let xml = serializer
            .serialize_document(&document)
            .expect("serialize")
            .xml;

        assert!(
            xml.contains("<w:p><w:r><w:t>Hello</w:t></w:r></w:p>"),
            "paragraph should be represented as a single run"
        );
    }

    #[test]
    fn escapes_xml_entities() {
        let document = sample_document("1 < 2 & 3 > 0");
        let serializer = DocxSerializer::new();
        let xml = serializer
            .serialize_document(&document)
            .expect("serialize")
            .xml;

        assert!(
            xml.contains("1 &lt; 2 &amp; 3 &gt; 0"),
            "special characters must be escaped"
        );
    }

    #[test]
    fn serializes_soft_break_as_br() {
        let document = sample_document("Hello\nWorld");
        let serializer = DocxSerializer::new();
        let xml = serializer
            .serialize_document(&document)
            .expect("serialize")
            .xml;

        assert!(
            xml.contains(r#"<w:br w:type="textWrapping"/>"#),
            "soft breaks should be serialized as w:br"
        );
    }

    #[test]
    fn serializes_page_break_as_page_br() {
        let document = sample_document(&format!("Hello{PAGE_BREAK_STR}World"));
        let serializer = DocxSerializer::new();
        let xml = serializer
            .serialize_document(&document)
            .expect("serialize")
            .xml;

        assert!(
            xml.contains(r#"<w:br w:type="page"/>"#),
            "page breaks should be serialized as w:br type=\"page\""
        );
    }
}
