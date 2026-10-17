//! DrawingML colour scheme from `xl/theme/theme1.xml`.

use jade_opc::local_name;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use crate::error::Result;

#[derive(Default)]
pub(crate) struct ThemePalette {
    colors: [Option<[u8; 3]>; 12],
}

impl ThemePalette {
    pub(crate) fn color(&self, index: u32) -> Option<[u8; 3]> {
        self.colors.get(index as usize).copied().flatten()
    }
}

pub(crate) fn parse_theme(xml: &str) -> Result<ThemePalette> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut palette = ThemePalette::default();
    let mut current_slot = None;
    let mut in_scheme = false;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let qualified_name = element.name();
                let name = local_name(qualified_name.as_ref());
                if name == b"clrScheme" {
                    in_scheme = true;
                } else if in_scheme {
                    if let Some(index) = theme_index(name) {
                        current_slot = Some(index);
                    } else if matches!(name, b"srgbClr" | b"sysClr") {
                        let key: &[u8] = if name == b"sysClr" {
                            b"lastClr"
                        } else {
                            b"val"
                        };
                        if let (Some(index), Some(color)) =
                            (current_slot, color_attribute(element, key)?)
                        {
                            palette.colors[index] = Some(color);
                        }
                    }
                }
            }
            Event::End(ref element) => {
                let qualified_name = element.name();
                let name = local_name(qualified_name.as_ref());
                if name == b"clrScheme" {
                    in_scheme = false;
                    current_slot = None;
                } else if theme_index(name).is_some() {
                    current_slot = None;
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(palette)
}

fn color_attribute(element: &BytesStart<'_>, key: &[u8]) -> Result<Option<[u8; 3]>> {
    for attribute in element.attributes() {
        let attribute = attribute?;
        if local_name(attribute.key.as_ref()) == key {
            let value = attribute.unescape_value()?;
            if value.len() != 6 {
                return Ok(None);
            }
            let mut color = [0; 3];
            for (index, channel) in color.iter_mut().enumerate() {
                *channel = match u8::from_str_radix(&value[index * 2..index * 2 + 2], 16) {
                    Ok(value) => value,
                    Err(_) => return Ok(None),
                };
            }
            return Ok(Some(color));
        }
    }
    Ok(None)
}

// SpreadsheetML theme indexes use this order, which differs from clrScheme's
// common XML order (dark first, then light).
fn theme_index(name: &[u8]) -> Option<usize> {
    Some(match name {
        b"lt1" => 0,
        b"dk1" => 1,
        b"lt2" => 2,
        b"dk2" => 3,
        b"accent1" => 4,
        b"accent2" => 5,
        b"accent3" => 6,
        b"accent4" => 7,
        b"accent5" => 8,
        b"accent6" => 9,
        b"hlink" => 10,
        b"folHlink" => 11,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spreadsheet_theme_order_and_system_fallbacks() {
        let xml = r#"<a:theme xmlns:a="urn:test"><a:themeElements><a:clrScheme name="Custom">
          <a:dk1><a:sysClr val="windowText" lastClr="102030"/></a:dk1>
          <a:lt1><a:sysClr val="window" lastClr="FAFBFC"/></a:lt1>
          <a:accent1><a:srgbClr val="4472C4"/></a:accent1>
          <a:hlink><a:srgbClr val="0563C1"/></a:hlink>
        </a:clrScheme></a:themeElements></a:theme>"#;
        let theme = parse_theme(xml).expect("theme");

        assert_eq!(theme.color(0), Some([0xFA, 0xFB, 0xFC]));
        assert_eq!(theme.color(1), Some([0x10, 0x20, 0x30]));
        assert_eq!(theme.color(4), Some([0x44, 0x72, 0xC4]));
        assert_eq!(theme.color(10), Some([0x05, 0x63, 0xC1]));
    }
}
