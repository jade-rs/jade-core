//! Dynamic arrays as Excel saves them.
//!
//! A formula that spills is written on its anchor as `<f t="array" ref>`,
//! the range it fills, with `cm` on the cell pointing at the workbook's
//! dynamic-array cell metadata (an `XLDAPR` block in `xl/metadata.xml`).
//! Without that link Excel reads the formula as an old fixed-size array.
//! The spilled values are plain cells beside the anchor.

use quick_xml::events::Event;
use quick_xml::Reader;

use jade_opc::OpcPackage;
use jade_xlsx_reader::{CellRange, CellRef};

use crate::notes::{add_relationship, ensure_override};
use crate::patch::{apply, attribute, local, Splice};
use crate::shift::{range_text, reference_text, rewrite};
use crate::{write_xlsx, CellEdit, Result, SheetEdits};

const SHEET_METADATA: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/sheetMetadata";
const METADATA_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheetMetadata+xml";

/// One array formula's anchor and the range it fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArrayRange {
    pub anchor: CellRef,
    pub range: CellRange,
}

/// What one worksheet part's array formulas spilled.
#[derive(Debug, Clone, PartialEq)]
pub struct SheetSpills {
    pub part_name: String,
    /// The cells beside the anchors: their values, or a blank for a cell a
    /// spill no longer reaches.
    pub values: Vec<CellEdit>,
    /// The anchors whose range to write.
    pub ranges: Vec<ArrayRange>,
}

/// Writes spilled values and marks each anchor as a dynamic array. With
/// `withdraw_recalculation` — the values are a complete recalculation's
/// own — the workbook stops asking for a full calculation on load, which
/// writing cells would otherwise request.
pub fn write_spills(
    original: &[u8],
    sheets: &[SheetSpills],
    withdraw_recalculation: bool,
) -> Result<Vec<u8>> {
    let edits: Vec<SheetEdits> = sheets
        .iter()
        .filter(|sheet| !sheet.values.is_empty())
        .map(|sheet| SheetEdits {
            part_name: sheet.part_name.clone(),
            cells: sheet.values.clone(),
        })
        .collect();
    let bytes = if edits.is_empty() {
        original.to_vec()
    } else {
        write_xlsx(original, &edits)?
    };
    let marks = sheets.iter().any(|sheet| !sheet.ranges.is_empty());
    if !marks && !withdraw_recalculation {
        return Ok(bytes);
    }
    let mut package = OpcPackage::open(&bytes)?;
    if withdraw_recalculation {
        let name = crate::workbook_part_name(&package)?;
        if let Some(xml) = package.part_str(&name)? {
            package.replace(&name, crate::withdraw_full_recalculation(&xml).into_bytes());
        }
    }
    if !marks {
        return Ok(package.write()?);
    }
    let cm = dynamic_array_metadata(&mut package)?;
    for sheet in sheets.iter().filter(|sheet| !sheet.ranges.is_empty()) {
        let Some(xml) = package.part_str(&sheet.part_name)? else {
            continue;
        };
        let marked = mark_array_formulas(&xml, &sheet.ranges, cm)?;
        package.replace(&sheet.part_name, marked.into_bytes());
    }
    Ok(package.write()?)
}

/// `xml` with each anchor's formula marked `t="array" ref`, and `cm` set on
/// the cell when given. An anchor without a formula, or with a shared one,
/// is left as it is.
pub fn mark_array_formulas(xml: &str, ranges: &[ArrayRange], cm: Option<u32>) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut splices = Vec::new();
    // The anchor being read: its range, and where its start tag is.
    let mut open: Option<(CellRange, Splice)> = None;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        let end = reader.buffer_position() as usize;
        match event {
            Event::Start(ref element) if local(element.name().as_ref()) == b"c" => {
                let reference = attribute(element, b"r")?;
                open = ranges
                    .iter()
                    .find(|array| reference.as_deref() == Some(&reference_text(array.anchor)))
                    .map(|array| {
                        let head = match cm {
                            Some(index) => {
                                rewrite(element, &[("cm", Some(index.to_string()))], false)
                            }
                            None => rewrite(element, &[], false),
                        };
                        head.map(|text| (array.range, Splice { start, end, text }))
                    })
                    .transpose()?;
            }
            Event::Start(ref element) | Event::Empty(ref element)
                if local(element.name().as_ref()) == b"f" =>
            {
                if let Some((range, head)) = open.take() {
                    let shared = attribute(element, b"t")?.as_deref() == Some("shared");
                    if !shared {
                        let empty = matches!(event, Event::Empty(_));
                        let text = rewrite(
                            element,
                            &[
                                ("t", Some("array".to_string())),
                                ("ref", Some(range_text(range))),
                            ],
                            empty,
                        )?;
                        splices.push(head);
                        splices.push(Splice { start, end, text });
                    }
                }
            }
            Event::End(ref element) if local(element.name().as_ref()) == b"c" => open = None,
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(apply(xml, splices))
}

/// The 1-based cell-metadata index that marks a dynamic array: the
/// workbook's own `XLDAPR` entry, or a metadata part added with one. A
/// metadata part without one gives `None`, and the formula is written as a
/// fixed-size array.
pub fn dynamic_array_metadata(package: &mut OpcPackage) -> Result<Option<u32>> {
    let workbook = crate::workbook_part_name(package)?;
    let rels_name = jade_opc::rels_part_name(&workbook);
    let rels = package.part_str(&rels_name)?;
    let existing = match &rels {
        Some(xml) => jade_opc::parse_relationships(xml)?
            .target_by_type_suffix("/sheetMetadata")
            .map(|target| jade_opc::resolve_target(&workbook, target)),
        None => None,
    };
    if let Some(part) = existing {
        return Ok(match package.part_str(&part)? {
            Some(xml) => dynamic_array_index(&xml)?,
            None => None,
        });
    }
    let part = jade_opc::resolve_target(&workbook, "metadata.xml");
    package.replace(&part, METADATA.as_bytes().to_vec());
    let (rels, _) = add_relationship(rels.as_deref(), SHEET_METADATA, "metadata.xml")?;
    package.replace(&rels_name, rels.into_bytes());
    ensure_override(package, &part, METADATA_CONTENT_TYPE)?;
    Ok(Some(1))
}

/// Which cell-metadata block of `xml` (1-based) points at the `XLDAPR`
/// metadata type.
fn dynamic_array_index(xml: &str) -> Result<Option<u32>> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut types = 0u32;
    let mut dynamic_type: Option<u32> = None;
    let mut in_cell_metadata = false;
    let mut blocks = 0u32;
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(ref element) | Event::Empty(ref element) => {
                match local(element.name().as_ref()) {
                    b"metadataType" => {
                        types += 1;
                        if attribute(element, b"name")?.as_deref() == Some("XLDAPR") {
                            dynamic_type = Some(types);
                        }
                    }
                    b"cellMetadata" => in_cell_metadata = true,
                    b"bk" if in_cell_metadata => blocks += 1,
                    b"rc" if in_cell_metadata => {
                        let kind = attribute(element, b"t")?.and_then(|t| t.parse::<u32>().ok());
                        if kind.is_some() && kind == dynamic_type {
                            return Ok(Some(blocks));
                        }
                    }
                    _ => {}
                }
            }
            Event::End(ref element) if local(element.name().as_ref()) == b"cellMetadata" => {
                in_cell_metadata = false;
            }
            Event::Eof => return Ok(None),
            _ => {}
        }
        buf.clear();
    }
}

/// The metadata part Excel writes for a workbook whose only cell metadata
/// is the dynamic-array flag.
const METADATA: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<metadata xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:xda="http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray"><metadataTypes count="1"><metadataType name="XLDAPR" minSupportedVersion="120000" copy="1" pasteAll="1" pasteValues="1" merge="1" splitFirst="1" rowColShift="1" clearFormats="1" clearComments="1" assign="1" coerce="1" cellMeta="1"/></metadataTypes><futureMetadata name="XLDAPR" count="1"><bk><extLst><ext uri="{bdbb8cdc-fa1e-496e-a857-3c3f30c029c3}"><xda:dynamicArrayProperties fDynamic="1" fCollapsed="0"/></ext></extLst></bk></futureMetadata><cellMetadata count="1"><bk><rc t="1" v="0"/></bk></cellMetadata></metadata>"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_anchor_gains_its_range_and_metadata_link() {
        let xml = r#"<sheetData><row r="1"><c r="A1" s="2"><f>SEQUENCE(3)</f><v>1</v></c><c r="B1"><f>A1</f></c></row></sheetData>"#;
        let marked = mark_array_formulas(
            xml,
            &[ArrayRange {
                anchor: CellRef::new(0, 0),
                range: CellRange::parse("A1:A3").unwrap(),
            }],
            Some(1),
        )
        .unwrap();
        assert_eq!(
            marked,
            r#"<sheetData><row r="1"><c r="A1" s="2" cm="1"><f t="array" ref="A1:A3">SEQUENCE(3)</f><v>1</v></c><c r="B1"><f>A1</f></c></row></sheetData>"#
        );
    }
}
