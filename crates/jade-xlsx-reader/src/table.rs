//! `xl/tables/tableN.xml` — an Excel table: the cells it covers and the
//! names of its columns.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use crate::error::Result;
use crate::model::{CellRange, Table};
use jade_opc::local_name;

/// Parses one table part. `None` when the part has no usable `ref`, which
/// is not a table Excel would open either.
pub fn parse_table(part_name: &str, xml: &str) -> Result<Option<Table>> {
    let (filter, _) = crate::auto_filter::parse(xml, b"table")?;
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut table: Option<Table> = None;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(ref element) | Event::Empty(ref element) => {
                match local_name(element.name().as_ref()) {
                    b"table" => {
                        let Some(reference) = attribute_value(element, b"ref")? else {
                            return Ok(None);
                        };
                        let name = attribute_value(element, b"name")?.unwrap_or_default();
                        let display_name = attribute_value(element, b"displayName")?
                            .unwrap_or_else(|| name.clone());
                        table = Some(Table {
                            part_name: part_name.to_string(),
                            name,
                            display_name,
                            reference: CellRange::parse(&reference)?,
                            header_row_count: attribute_value(element, b"headerRowCount")?
                                .and_then(|value| value.parse().ok())
                                .unwrap_or(1),
                            totals_row_count: attribute_value(element, b"totalsRowCount")?
                                .and_then(|value| value.parse().ok())
                                .unwrap_or(0),
                            columns: Vec::new(),
                            style_name: None,
                            auto_filter: false,
                            filter: None,
                        });
                    }
                    b"tableColumn" => {
                        if let Some(table) = table.as_mut() {
                            table
                                .columns
                                .push(attribute_value(element, b"name")?.unwrap_or_default());
                        }
                    }
                    b"tableStyleInfo" => {
                        if let Some(table) = table.as_mut() {
                            table.style_name = attribute_value(element, b"name")?;
                        }
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    if let Some(table) = &mut table {
        table.auto_filter = filter.is_some();
        table.filter = filter;
    }
    Ok(table)
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
