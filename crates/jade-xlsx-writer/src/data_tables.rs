//! What-If data tables as Excel saves them: the first result cell holds
//! `<f t="dataTable" ref dt2D dtr r1 r2/>`, an empty formula naming the
//! range and the input cells, and every result cell its value.

use quick_xml::events::Event;
use quick_xml::Reader;

use jade_xlsx_reader::DataTable;

use crate::patch::{apply, attribute, local, Splice};
use crate::shift::{range_text, reference_text};
use crate::Result;

/// The `<f>` Excel writes for `table`.
pub fn data_table_formula(table: &DataTable, prefix: &str) -> String {
    let mut text = format!(
        "<{prefix}f t=\"dataTable\" ref=\"{}\" dt2D=\"{}\" dtr=\"{}\"",
        range_text(table.range),
        u8::from(table.two_way),
        u8::from(table.row || table.two_way),
    );
    for (name, cell) in [("r1", table.first), ("r2", table.second)] {
        if let Some(cell) = cell {
            text.push_str(&format!(" {name}=\"{}\"", reference_text(cell)));
        }
    }
    text.push_str("/>");
    text
}

/// `xml` with the first cell of each of `tables` holding its `TABLE`: an
/// `<f>` it has is replaced, else one goes in before its value. A cell not
/// in the part is left out; write the values first.
pub fn mark_data_tables(xml: &str, tables: &[DataTable]) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut splices = Vec::new();
    // The anchor being read and its formula, once written.
    let mut open: Option<(&DataTable, String, bool)> = None;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        let end = reader.buffer_position() as usize;
        match event {
            Event::Start(ref element) | Event::Empty(ref element)
                if local(element.name().as_ref()) == b"c" =>
            {
                let reference = attribute(element, b"r")?;
                let name = String::from_utf8_lossy(element.name().as_ref()).into_owned();
                let prefix = name
                    .split_once(':')
                    .map(|(prefix, _)| format!("{prefix}:"))
                    .unwrap_or_default();
                let found = tables
                    .iter()
                    .find(|table| reference.as_deref() == Some(&reference_text(table.range.start)));
                if let Some(table) = found {
                    let formula = data_table_formula(table, &prefix);
                    if matches!(event, Event::Empty(_)) {
                        // `<c r="B2"/>`: opened, given the formula, closed.
                        let head = String::from_utf8_lossy(&xml.as_bytes()[start..end])
                            .trim_end_matches("/>")
                            .trim_end()
                            .to_string();
                        splices.push(Splice {
                            start,
                            end,
                            text: format!("{head}>{formula}</{name}>"),
                        });
                    } else {
                        open = Some((table, formula, false));
                        // The formula goes in right after the head unless an
                        // `<f>` is there to replace.
                        splices.push(Splice {
                            start: end,
                            end,
                            text: String::new(),
                        });
                    }
                }
            }
            Event::Start(ref element) | Event::Empty(ref element)
                if local(element.name().as_ref()) == b"f" =>
            {
                if let Some((_, formula, replaced)) = open.as_mut() {
                    let empty = matches!(event, Event::Empty(_));
                    let stop = if empty {
                        end
                    } else {
                        // Through the formula's end tag.
                        let mut depth = 1;
                        let mut stop = end;
                        let mut inner = Vec::new();
                        while depth > 0 {
                            match reader.read_event_into(&mut inner)? {
                                Event::Start(_) => depth += 1,
                                Event::End(_) => depth -= 1,
                                Event::Eof => break,
                                _ => {}
                            }
                            stop = reader.buffer_position() as usize;
                        }
                        stop
                    };
                    splices.push(Splice {
                        start,
                        end: stop,
                        text: std::mem::take(formula),
                    });
                    *replaced = true;
                }
            }
            Event::End(ref element) if local(element.name().as_ref()) == b"c" => {
                if let Some((_, formula, replaced)) = open.take() {
                    if !replaced {
                        // No `<f>`: the empty splice after the head takes it.
                        if let Some(splice) = splices
                            .iter_mut()
                            .rev()
                            .find(|splice| splice.start == splice.end && splice.text.is_empty())
                        {
                            splice.text = formula;
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    splices.retain(|splice| !(splice.start == splice.end && splice.text.is_empty()));
    Ok(apply(xml, splices))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jade_xlsx_reader::{CellRange, CellRef};

    fn table(range: &str) -> DataTable {
        DataTable {
            range: CellRange::parse(range).unwrap(),
            two_way: false,
            row: false,
            first: Some(CellRef::parse("A1").unwrap()),
            second: None,
        }
    }

    #[test]
    fn the_first_result_cell_takes_the_table() {
        let xml = r#"<sheetData><row r="2"><c r="B2" s="1"><v>5</v></c><c r="C2"/></row><row r="3"><c r="B3"/></row></sheetData>"#;
        assert_eq!(
            mark_data_tables(xml, &[table("B2:B3")]).unwrap(),
            r#"<sheetData><row r="2"><c r="B2" s="1"><f t="dataTable" ref="B2:B3" dt2D="0" dtr="0" r1="A1"/><v>5</v></c><c r="C2"/></row><row r="3"><c r="B3"/></row></sheetData>"#
        );
        assert_eq!(
            mark_data_tables(xml, &[table("C2:C3")]).unwrap(),
            r#"<sheetData><row r="2"><c r="B2" s="1"><v>5</v></c><c r="C2"><f t="dataTable" ref="C2:C3" dt2D="0" dtr="0" r1="A1"/></c></row><row r="3"><c r="B3"/></row></sheetData>"#
        );
        let formula =
            r#"<sheetData><row r="2"><c r="B2"><f>A1*2</f><v>5</v></c></row></sheetData>"#;
        assert_eq!(
            mark_data_tables(formula, &[table("B2:B3")]).unwrap(),
            r#"<sheetData><row r="2"><c r="B2"><f t="dataTable" ref="B2:B3" dt2D="0" dtr="0" r1="A1"/><v>5</v></c></row></sheetData>"#
        );
    }

    #[test]
    fn a_two_way_table_names_both_input_cells() {
        let two_way = DataTable {
            two_way: true,
            row: true,
            second: Some(CellRef::parse("A2").unwrap()),
            ..table("C3:E5")
        };
        assert_eq!(
            data_table_formula(&two_way, ""),
            r#"<f t="dataTable" ref="C3:E5" dt2D="1" dtr="1" r1="A1" r2="A2"/>"#
        );
    }
}
