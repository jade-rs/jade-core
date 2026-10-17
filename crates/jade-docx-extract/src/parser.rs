use quick_xml::Reader;
use quick_xml::escape::unescape;
use quick_xml::events::{BytesStart, Event};

/// 表の中での位置 (0 始まり)。
///
/// 入れ子の表があるので 1 段ぶんを表す。`table` は「その階層で何番目の表か」で、
/// 本文直下なら本文中の通し番号、セルの中ならそのセルの中での通し番号。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct TableCoord {
    pub table: usize,
    pub row: usize,
    pub cell: usize,
}

/// 開いている表 1 つぶんの数え上げ。
#[derive(Debug)]
struct TableFrame {
    index: usize,
    row: usize,
    cell: usize,
    /// このセルの中で開いた表の数。入れ子の表に通し番号を振るのに使う。
    nested: usize,
}

impl TableFrame {
    fn coord(&self) -> TableCoord {
        TableCoord {
            table: self.index,
            row: self.row,
            cell: self.cell,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParagraphData {
    pub segments: Vec<TextSegment>,
    pub numbering: Option<ParagraphNumbering>,
    pub numbering_text: String,
    pub accepted_numbering_text: String,
    pub in_table: bool,
    /// 表の中での位置。外側の表から順に積む。表の外なら空。
    pub table_path: Vec<TableCoord>,
    pub in_textbox: bool,
    pub in_shape: bool,
    pub para_id: Option<String>,
    pub style_id: Option<String>,
}

impl ParagraphData {
    pub fn new(
        in_table: bool,
        table_path: Vec<TableCoord>,
        in_textbox: bool,
        in_shape: bool,
    ) -> Self {
        Self {
            segments: Vec::new(),
            numbering: None,
            numbering_text: String::new(),
            accepted_numbering_text: String::new(),
            in_table,
            table_path,
            in_textbox,
            in_shape,
            para_id: None,
            style_id: None,
        }
    }

    pub fn add_text(&mut self, text: &str, kind: TextSegmentKind) {
        if text.is_empty() {
            return;
        }

        if let Some(last) = self.segments.last_mut()
            && last.kind == kind
        {
            last.content.push_str(text);
            return;
        }

        self.segments.push(TextSegment {
            kind,
            content: text.to_string(),
        });
    }

    pub fn add_owned_text(&mut self, text: String, kind: TextSegmentKind) {
        if text.is_empty() {
            return;
        }

        if let Some(last) = self.segments.last_mut()
            && last.kind == kind
        {
            last.content.push_str(&text);
            return;
        }

        self.segments.push(TextSegment {
            kind,
            content: text,
        });
    }

    pub fn ensure_numbering(&mut self) -> &mut ParagraphNumbering {
        if self.numbering.is_none() {
            self.numbering = Some(ParagraphNumbering {
                num_id: String::new(),
                level: 0,
            });
        }
        self.numbering.as_mut().unwrap()
    }

    pub fn set_numbering_id(&mut self, num_id: String) {
        let numbering = self.ensure_numbering();
        numbering.num_id = num_id;
    }

    pub fn set_numbering_level(&mut self, level: u32) {
        let numbering = self.ensure_numbering();
        numbering.level = level;
    }

    pub fn is_removed_after_accept(&self) -> bool {
        if self.segments.is_empty() {
            return false;
        }

        !self
            .segments
            .iter()
            .any(|segment| segment.kind != TextSegmentKind::Deleted && !segment.content.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParagraphNumbering {
    pub num_id: String,
    pub level: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSegment {
    pub kind: TextSegmentKind,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextSegmentKind {
    Normal,
    Inserted,
    Deleted,
}

#[derive(Clone, Copy)]
enum TextElement {
    Text,
    DelText,
    InstrText,
}

#[derive(Clone, Copy)]
enum ShapeContext {
    VShape,
    WordShape,
}

#[derive(Clone, Copy)]
enum TextBoxContext {
    WordTextBox,
    VmlTextBox,
}

#[derive(Clone, Copy)]
enum ChangeContext {
    Normal,
    Inserted,
    Deleted,
}

pub fn parse_document(xml: &str) -> Result<Vec<ParagraphData>, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut buf = Vec::new();
    let mut paragraphs = Vec::new();
    let mut current_paragraph: Option<ParagraphData> = None;

    let mut change_stack: Vec<ChangeContext> = Vec::new();
    let mut table_stack: Vec<TableFrame> = Vec::new();
    // 本文直下に現れた表の数。入れ子は各フレームが自分で数える。
    let mut body_tables = 0usize;
    let mut textbox_stack: Vec<TextBoxContext> = Vec::new();
    let mut shape_stack: Vec<ShapeContext> = Vec::new();

    let mut current_text_element: Option<TextElement> = None;
    let mut preserve_space = false;

    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| format!("document.xml の解析に失敗しました: {e}"))?
        {
            Event::Start(ref e) => match e.name().as_ref() {
                b"w:tbl" => {
                    let index = match table_stack.last_mut() {
                        Some(parent) => {
                            let index = parent.nested;
                            parent.nested += 1;
                            index
                        }
                        None => {
                            let index = body_tables;
                            body_tables += 1;
                            index
                        }
                    };
                    table_stack.push(TableFrame {
                        index,
                        row: 0,
                        cell: 0,
                        nested: 0,
                    });
                }
                b"w:tr" => {
                    if let Some(frame) = table_stack.last_mut() {
                        frame.cell = 0;
                    }
                }
                b"w:tc" => {
                    // セルに入るたび、その中で開く表の数え直し。
                    if let Some(frame) = table_stack.last_mut() {
                        frame.nested = 0;
                    }
                }
                b"w:txbxContent" => textbox_stack.push(TextBoxContext::WordTextBox),
                b"v:textbox" => textbox_stack.push(TextBoxContext::VmlTextBox),
                b"w:shape" => shape_stack.push(ShapeContext::WordShape),
                b"v:shape" => shape_stack.push(ShapeContext::VShape),
                b"wps:sp" | b"wps:cxnSp" | b"wps:grpSp" | b"wpg:sp" => {
                    shape_stack.push(ShapeContext::WordShape)
                }
                b"w:p" => {
                    let mut para = ParagraphData::new(
                        !table_stack.is_empty(),
                        table_stack.iter().map(TableFrame::coord).collect(),
                        !textbox_stack.is_empty(),
                        !shape_stack.is_empty(),
                    );
                    let para_id = attr_value(e, b"w:paraId")
                        .or_else(|| attr_value(e, b"w14:paraId"))
                        .or_else(|| attr_value(e, b"w15:paraId"));
                    para.para_id = para_id;
                    current_paragraph = Some(para);
                }
                b"w:pStyle" => {
                    if let Some(para) = current_paragraph.as_mut()
                        && let Some(val) = attr_value(e, b"w:val")
                    {
                        para.style_id = Some(val);
                    }
                }
                b"w:numId" => {
                    if let Some(para) = current_paragraph.as_mut()
                        && let Some(val) = attr_value(e, b"w:val")
                    {
                        para.set_numbering_id(val);
                    }
                }
                b"w:ilvl" => {
                    if let Some(para) = current_paragraph.as_mut()
                        && let Some(level) = attr_value(e, b"w:val")
                        && let Ok(parsed) = level.parse::<u32>()
                    {
                        para.set_numbering_level(parsed);
                    }
                }
                b"w:numPr" => {
                    if let Some(para) = current_paragraph.as_mut() {
                        para.ensure_numbering();
                    }
                }
                b"w:ins" => change_stack.push(ChangeContext::Inserted),
                b"w:del" | b"w:moveFrom" => change_stack.push(ChangeContext::Deleted),
                b"w:t" => {
                    current_text_element = Some(TextElement::Text);
                    preserve_space = has_space_preserve(e);
                }
                b"w:delText" => {
                    current_text_element = Some(TextElement::DelText);
                    preserve_space = has_space_preserve(e);
                }
                b"w:instrText" => {
                    current_text_element = Some(TextElement::InstrText);
                    preserve_space = has_space_preserve(e);
                }
                b"w:tab" => {
                    if let Some(para) = current_paragraph.as_mut() {
                        let kind = current_change(&change_stack);
                        para.add_text("\t", kind);
                    }
                }
                b"w:br" => {
                    if let Some(para) = current_paragraph.as_mut() {
                        let kind = current_change(&change_stack);
                        para.add_text("\n", kind);
                    }
                }
                b"w:sym" => {
                    if let Some(para) = current_paragraph.as_mut()
                        && let Some(ch) = attr_value(e, b"w:char")
                    {
                        let kind = current_change(&change_stack);
                        para.add_text(&ch, kind);
                    }
                }
                _ => {}
            },
            Event::Empty(ref e) => {
                match e.name().as_ref() {
                    // 中身の無い <w:tbl/> は始まりと終わりが同時に来たのと同じで、
                    // 段落を 1 つも含まない。以前は深さを足すだけで戻していなかった
                    // ため、これ以降の段落がすべて表の中と判定されていた。
                    b"w:tbl" => {}
                    b"w:tab" => {
                        if let Some(para) = current_paragraph.as_mut() {
                            let kind = current_change(&change_stack);
                            para.add_text("\t", kind);
                        }
                    }
                    b"w:br" => {
                        if let Some(para) = current_paragraph.as_mut() {
                            let kind = current_change(&change_stack);
                            para.add_text("\n", kind);
                        }
                    }
                    b"w:pStyle" => {
                        if let Some(para) = current_paragraph.as_mut()
                            && let Some(val) = attr_value(e, b"w:val")
                        {
                            para.style_id = Some(val);
                        }
                    }
                    b"w:numId" => {
                        if let Some(para) = current_paragraph.as_mut()
                            && let Some(val) = attr_value(e, b"w:val")
                        {
                            para.set_numbering_id(val);
                        }
                    }
                    b"w:ilvl" => {
                        if let Some(para) = current_paragraph.as_mut()
                            && let Some(level) = attr_value(e, b"w:val")
                            && let Ok(parsed) = level.parse::<u32>()
                        {
                            para.set_numbering_level(parsed);
                        }
                    }
                    b"w:sym" => {
                        if let Some(para) = current_paragraph.as_mut()
                            && let Some(ch) = attr_value(e, b"w:char")
                        {
                            let kind = current_change(&change_stack);
                            para.add_text(&ch, kind);
                        }
                    }
                    _ => {}
                }

                handle_end_event(
                    e.name().as_ref(),
                    &mut current_paragraph,
                    &mut paragraphs,
                    &mut table_stack,
                    &mut textbox_stack,
                    &mut shape_stack,
                    &mut change_stack,
                    &mut current_text_element,
                );
            }
            Event::End(ref e) => {
                handle_end_event(
                    e.name().as_ref(),
                    &mut current_paragraph,
                    &mut paragraphs,
                    &mut table_stack,
                    &mut textbox_stack,
                    &mut shape_stack,
                    &mut change_stack,
                    &mut current_text_element,
                );
            }
            Event::Text(e) => {
                handle_text_event(
                    &reader,
                    e,
                    preserve_space,
                    current_text_element,
                    &mut current_paragraph,
                    &change_stack,
                )?;
            }
            Event::CData(e) => {
                handle_cdata_event(
                    &reader,
                    e,
                    preserve_space,
                    current_text_element,
                    &mut current_paragraph,
                    &change_stack,
                )?;
            }
            Event::Eof => break,
            _ => {}
        }

        buf.clear();
    }

    Ok(paragraphs)
}

#[allow(clippy::too_many_arguments)]
fn handle_end_event(
    name: &[u8],
    current_paragraph: &mut Option<ParagraphData>,
    paragraphs: &mut Vec<ParagraphData>,
    table_stack: &mut Vec<TableFrame>,
    textbox_stack: &mut Vec<TextBoxContext>,
    shape_stack: &mut Vec<ShapeContext>,
    change_stack: &mut Vec<ChangeContext>,
    current_text_element: &mut Option<TextElement>,
) {
    match name {
        b"w:p" => {
            if let Some(mut para) = current_paragraph.take() {
                if let Some(numbering) = para.numbering.as_ref()
                    && numbering.num_id.is_empty()
                {
                    para.numbering = None;
                }
                paragraphs.push(para);
            }
        }
        b"w:tbl" => {
            table_stack.pop();
        }
        b"w:tr" => {
            if let Some(frame) = table_stack.last_mut() {
                frame.row += 1;
            }
        }
        b"w:tc" => {
            if let Some(frame) = table_stack.last_mut() {
                frame.cell += 1;
            }
        }
        b"w:txbxContent" | b"v:textbox" => {
            textbox_stack.pop();
        }
        b"w:shape" | b"v:shape" | b"wps:sp" | b"wps:cxnSp" | b"wps:grpSp" | b"wpg:sp" => {
            shape_stack.pop();
        }
        b"w:ins" | b"w:del" | b"w:moveFrom" => {
            change_stack.pop();
        }
        b"w:t" | b"w:delText" | b"w:instrText" => {
            *current_text_element = None;
        }
        _ => {}
    }
}

fn handle_text_event(
    reader: &Reader<&[u8]>,
    event: quick_xml::events::BytesText<'_>,
    preserve_space: bool,
    current_text_element: Option<TextElement>,
    current_paragraph: &mut Option<ParagraphData>,
    change_stack: &[ChangeContext],
) -> Result<(), String> {
    if current_text_element.is_none() {
        return Ok(());
    }

    if let Some(para) = current_paragraph.as_mut() {
        let mut text = decode_text(reader, event)?;
        if !preserve_space {
            text.retain(|ch| ch != '\r');
        }
        if !text.is_empty() {
            let kind = current_change(change_stack);
            para.add_owned_text(text, kind);
        }
    }

    Ok(())
}

fn handle_cdata_event(
    reader: &Reader<&[u8]>,
    event: quick_xml::events::BytesCData<'_>,
    preserve_space: bool,
    current_text_element: Option<TextElement>,
    current_paragraph: &mut Option<ParagraphData>,
    change_stack: &[ChangeContext],
) -> Result<(), String> {
    if current_text_element.is_none() {
        return Ok(());
    }

    if let Some(para) = current_paragraph.as_mut() {
        let mut text = decode_cdata(reader, event)?;
        if !preserve_space {
            text.retain(|ch| ch != '\r');
        }
        if !text.is_empty() {
            let kind = current_change(change_stack);
            para.add_owned_text(text, kind);
        }
    }

    Ok(())
}

fn current_change(stack: &[ChangeContext]) -> TextSegmentKind {
    match stack.last().copied().unwrap_or(ChangeContext::Normal) {
        ChangeContext::Normal => TextSegmentKind::Normal,
        ChangeContext::Inserted => TextSegmentKind::Inserted,
        ChangeContext::Deleted => TextSegmentKind::Deleted,
    }
}

fn attr_value(e: &BytesStart, key: &[u8]) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.as_ref() == key {
            return Some(String::from_utf8_lossy(&attr.value).to_string());
        }
    }
    None
}

fn has_space_preserve(e: &BytesStart) -> bool {
    e.attributes()
        .flatten()
        .any(|attr| attr.key.as_ref() == b"xml:space" && attr.value.as_ref() == b"preserve")
}

fn decode_text(
    _reader: &Reader<&[u8]>,
    event: quick_xml::events::BytesText<'_>,
) -> Result<String, String> {
    let decoded = event
        .decode()
        .map_err(|e| format!("テキストのデコードに失敗しました: {e}"))?;
    unescape(decoded.as_ref())
        .map(|cow| cow.into_owned())
        .map_err(|e| format!("テキストのデコードに失敗しました: {e}"))
}

fn decode_cdata(
    reader: &Reader<&[u8]>,
    event: quick_xml::events::BytesCData<'_>,
) -> Result<String, String> {
    reader
        .decoder()
        .decode(event.as_ref())
        .map(|cow| cow.into_owned())
        .map_err(|e| format!("テキストのデコードに失敗しました: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_text_reuses_the_input_buffer_for_a_new_segment() {
        let mut paragraph = ParagraphData::new(false, Vec::new(), false, false);
        let text = String::from("hello");
        let buffer = text.as_ptr();

        paragraph.add_owned_text(text, TextSegmentKind::Normal);

        assert_eq!(paragraph.segments[0].content, "hello");
        assert_eq!(paragraph.segments[0].content.as_ptr(), buffer);
    }

    #[test]
    fn owned_text_appends_to_an_adjacent_segment_of_the_same_kind() {
        let mut paragraph = ParagraphData::new(false, Vec::new(), false, false);
        paragraph.add_text("hello", TextSegmentKind::Normal);

        paragraph.add_owned_text(String::from(" world"), TextSegmentKind::Normal);

        assert_eq!(
            paragraph.segments,
            vec![TextSegment {
                kind: TextSegmentKind::Normal,
                content: String::from("hello world"),
            }]
        );
    }

    #[test]
    fn text_removes_carriage_returns_without_space_preserve() {
        let paragraphs = parse_document(
            "<w:document xmlns:w=\"w\"><w:body><w:p><w:r><w:t><![CDATA[a\rb]]></w:t></w:r></w:p></w:body></w:document>",
        )
        .unwrap();

        assert_eq!(paragraphs[0].segments[0].content, "ab");
    }

    #[test]
    fn text_keeps_carriage_returns_with_space_preserve() {
        let paragraphs = parse_document(
            "<w:document xmlns:w=\"w\"><w:body><w:p><w:r><w:t xml:space=\"preserve\"><![CDATA[a\rb]]></w:t></w:r></w:p></w:body></w:document>",
        )
        .unwrap();

        assert_eq!(paragraphs[0].segments[0].content, "a\rb");
    }
}
