use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use std::collections::{HashMap, HashSet};

pub struct Styles {
    paragraph_styles: HashMap<String, ParagraphStyle>,
}

struct ParagraphStyle {
    based_on: Option<String>,
    num_id: Option<String>,
    level: Option<u32>,
}

impl Styles {
    pub fn from_xml(xml: &str) -> Result<Self, String> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut buf = Vec::new();
        let mut paragraph_styles = HashMap::new();

        let mut current_style: Option<PendingStyle> = None;
        let mut in_paragraph_props = false;
        let mut in_num_props = false;

        loop {
            match reader
                .read_event_into(&mut buf)
                .map_err(|e| format!("styles.xml の解析に失敗しました: {e}"))?
            {
                Event::Start(ref e) => match e.name().as_ref() {
                    b"w:style" => {
                        if let Some(style) = start_style(e)
                            && style.is_paragraph
                        {
                            current_style = Some(PendingStyle::new(style.id));
                        }
                    }
                    b"w:basedOn" => {
                        if let Some(style) = current_style.as_mut() {
                            style.based_on = attr_value(e, b"w:val");
                        }
                    }
                    b"w:pPr" => {
                        if current_style.is_some() {
                            in_paragraph_props = true;
                        }
                    }
                    b"w:numPr" => {
                        if in_paragraph_props {
                            in_num_props = true;
                        }
                    }
                    b"w:numId" => {
                        if in_num_props && let Some(style) = current_style.as_mut() {
                            style.num_id = attr_value(e, b"w:val");
                        }
                    }
                    b"w:ilvl" => {
                        if in_num_props
                            && let Some(style) = current_style.as_mut()
                            && let Some(val) = attr_value(e, b"w:val")
                        {
                            style.level = val.parse().ok();
                        }
                    }
                    _ => {}
                },
                Event::Empty(ref e) => {
                    match e.name().as_ref() {
                        b"w:style" => {
                            if let Some(style) = start_style(e)
                                && style.is_paragraph
                            {
                                let style_id = style.id;
                                let pending = PendingStyle::new(style_id.clone());
                                paragraph_styles.insert(style_id, pending.into_paragraph_style());
                            }
                        }
                        b"w:basedOn" => {
                            if let Some(style) = current_style.as_mut() {
                                style.based_on = attr_value(e, b"w:val");
                            }
                        }
                        b"w:pPr" => {
                            in_paragraph_props = false;
                        }
                        b"w:numPr" => {
                            in_num_props = false;
                        }
                        b"w:numId" => {
                            if in_num_props && let Some(style) = current_style.as_mut() {
                                style.num_id = attr_value(e, b"w:val");
                            }
                        }
                        b"w:ilvl" => {
                            if in_num_props
                                && let Some(style) = current_style.as_mut()
                                && let Some(val) = attr_value(e, b"w:val")
                            {
                                style.level = val.parse().ok();
                            }
                        }
                        _ => {}
                    }

                    handle_end_event(
                        e.name().as_ref(),
                        &mut current_style,
                        &mut paragraph_styles,
                        &mut in_paragraph_props,
                        &mut in_num_props,
                    );
                }
                Event::End(ref e) => {
                    handle_end_event(
                        e.name().as_ref(),
                        &mut current_style,
                        &mut paragraph_styles,
                        &mut in_paragraph_props,
                        &mut in_num_props,
                    );
                }
                Event::Eof => break,
                _ => {}
            }

            buf.clear();
        }

        Ok(Styles { paragraph_styles })
    }

    pub fn resolve_numbering(&self, style_id: &str) -> Option<(String, u32)> {
        let mut visited = HashSet::new();
        let mut current = Some(style_id.to_string());

        let mut resolved_num_id: Option<String> = None;
        let mut resolved_level: Option<u32> = None;

        while let Some(id) = current {
            if !visited.insert(id.clone()) {
                break;
            }
            let Some(style) = self.paragraph_styles.get(&id) else {
                break;
            };

            if resolved_num_id.is_none()
                && let Some(num_id) = &style.num_id
            {
                resolved_num_id = Some(num_id.clone());
            }

            if resolved_level.is_none()
                && let Some(level) = style.level
            {
                resolved_level = Some(level);
            }

            if resolved_num_id.is_some() && resolved_level.is_some() {
                break;
            }

            current = style.based_on.clone();
        }

        resolved_num_id.map(|num_id| (num_id, resolved_level.unwrap_or(0)))
    }
}

struct PendingStyle {
    id: String,
    based_on: Option<String>,
    num_id: Option<String>,
    level: Option<u32>,
}

impl PendingStyle {
    fn new(id: String) -> Self {
        Self {
            id,
            based_on: None,
            num_id: None,
            level: None,
        }
    }

    fn into_paragraph_style(self) -> ParagraphStyle {
        ParagraphStyle {
            based_on: self.based_on,
            num_id: self.num_id,
            level: self.level,
        }
    }
}

struct StyleStart {
    id: String,
    is_paragraph: bool,
}

fn start_style(e: &BytesStart) -> Option<StyleStart> {
    let mut id: Option<String> = None;
    let mut style_type: Option<String> = None;

    for attr in e.attributes().flatten() {
        match attr.key.as_ref() {
            b"w:styleId" => id = Some(String::from_utf8_lossy(&attr.value).to_string()),
            b"w:type" => style_type = Some(String::from_utf8_lossy(&attr.value).to_string()),
            _ => {}
        }
    }

    let id = id?;
    let is_paragraph = style_type.as_deref() == Some("paragraph");
    Some(StyleStart { id, is_paragraph })
}

fn attr_value(e: &BytesStart, key: &[u8]) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.as_ref() == key {
            return Some(String::from_utf8_lossy(&attr.value).to_string());
        }
    }
    None
}

fn handle_end_event(
    name: &[u8],
    current_style: &mut Option<PendingStyle>,
    paragraph_styles: &mut HashMap<String, ParagraphStyle>,
    in_paragraph_props: &mut bool,
    in_num_props: &mut bool,
) {
    match name {
        b"w:style" => {
            if let Some(style) = current_style.take() {
                paragraph_styles.insert(style.id.clone(), style.into_paragraph_style());
            }
        }
        b"w:pPr" => {
            *in_paragraph_props = false;
        }
        b"w:numPr" => {
            *in_num_props = false;
        }
        _ => {}
    }
}
