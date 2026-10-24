//! The page header and footer of a sheet.
//!
//! `<headerFooter>` has a fixed place in the worksheet part — after
//! `pageSetup`, before `rowBreaks` — and Excel refuses a sheet whose
//! children are out of order, so a new element is inserted before the
//! first child that follows it rather than appended.

use quick_xml::events::Event;
use quick_xml::name::QName;
use quick_xml::Reader;

use jade_xlsx_reader::HeaderFooter;

use crate::patch::{apply, escape, local, Splice};
use crate::Result;

/// Worksheet children that come after `headerFooter`, in schema order.
const FOLLOWING: &[&[u8]] = &[
    b"rowBreaks",
    b"colBreaks",
    b"customProperties",
    b"cellWatches",
    b"ignoredErrors",
    b"smartTags",
    b"drawing",
    b"legacyDrawing",
    b"legacyDrawingHF",
    b"drawingHF",
    b"picture",
    b"oleObjects",
    b"controls",
    b"webPublishItems",
    b"tableParts",
    b"extLst",
];

/// The `<headerFooter>` element for `header_footer`, or nothing when it
/// says nothing.
pub fn header_footer_xml(header_footer: &HeaderFooter, prefix: &str) -> String {
    let sections: [(&str, &Option<String>); 6] = [
        ("oddHeader", &header_footer.odd_header),
        ("oddFooter", &header_footer.odd_footer),
        ("evenHeader", &header_footer.even_header),
        ("evenFooter", &header_footer.even_footer),
        ("firstHeader", &header_footer.first_header),
        ("firstFooter", &header_footer.first_footer),
    ];
    let mut attributes = String::new();
    if header_footer.different_odd_even {
        attributes.push_str(" differentOddEven=\"1\"");
    }
    if header_footer.different_first {
        attributes.push_str(" differentFirst=\"1\"");
    }
    if !header_footer.scale_with_doc {
        attributes.push_str(" scaleWithDoc=\"0\"");
    }
    if !header_footer.align_with_margins {
        attributes.push_str(" alignWithMargins=\"0\"");
    }
    let mut children = String::new();
    for (name, text) in sections {
        if let Some(text) = text {
            children.push_str(&format!(
                "<{prefix}{name}>{}</{prefix}{name}>",
                escape(text)
            ));
        }
    }
    if children.is_empty() && attributes.is_empty() {
        return String::new();
    }
    format!("<{prefix}headerFooter{attributes}>{children}</{prefix}headerFooter>")
}

/// The sheet part with its header and footer replaced by `header_footer`
/// — removed, when it says nothing.
pub fn set_header_footer(xml: &str, header_footer: &HeaderFooter) -> Result<String> {
    set_child(
        xml,
        b"headerFooter",
        |prefix| header_footer_xml(header_footer, prefix),
        FOLLOWING,
    )
}

/// The worksheet part with its direct child `name` replaced by what
/// `text` renders for the root's namespace prefix — inserted before the
/// first of `following` (the children the schema puts after it) when the
/// sheet has none, and removed when `text` renders nothing.
pub(crate) fn set_child(
    xml: &str,
    name: &[u8],
    text: impl FnOnce(&str) -> String,
    following: &[&[u8]],
) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut depth = 0usize;
    let mut prefix = String::new();
    let mut existing: Option<(usize, usize)> = None;
    let mut insert_before: Option<usize> = None;
    let mut root_end: Option<usize> = None;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let qualified = element.name().as_ref().to_vec();
                if depth == 0 {
                    prefix = crate::patch::namespace_prefix(&qualified);
                    if empty {
                        let text = text(&prefix);
                        if text.is_empty() {
                            return Ok(xml.to_string());
                        }
                        let head = &xml[start..reader.buffer_position() as usize];
                        let head = head.trim_end_matches("/>");
                        return Ok(apply(
                            xml,
                            vec![Splice {
                                start,
                                end: reader.buffer_position() as usize,
                                text: format!(
                                    "{head}>{text}</{}>",
                                    String::from_utf8_lossy(&qualified)
                                ),
                            }],
                        ));
                    }
                }
                if depth == 1 {
                    let local_name = local(&qualified);
                    if local_name == name {
                        if !empty {
                            reader.read_to_end_into(QName(&qualified), &mut Vec::new())?;
                        }
                        existing = Some((start, reader.buffer_position() as usize));
                        continue;
                    }
                    if insert_before.is_none() && following.contains(&local_name) {
                        insert_before = Some(start);
                    }
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    root_end = Some(start);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    let text = text(&prefix);
    let splice = match existing {
        Some((start, end)) => Splice { start, end, text },
        None => {
            if text.is_empty() {
                return Ok(xml.to_string());
            }
            let at = insert_before.or(root_end).unwrap_or(xml.len());
            Splice {
                start: at,
                end: at,
                text,
            }
        }
    };
    Ok(apply(xml, vec![splice]))
}
