//! Builds small `.xlsx` packages in memory, for cases the fixtures lack.
#![allow(dead_code)]

use std::io::Write;
use zip::write::SimpleFileOptions;

pub const MAIN: &str = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main""#;

/// One worksheet: its XML inside `<worksheet>`, and its relationships.
pub struct SheetPart {
    pub name: String,
    pub xml: String,
    pub rels: Vec<(String, String, String)>,
}

impl SheetPart {
    pub fn new(name: &str, xml: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            xml: xml.into(),
            rels: Vec::new(),
        }
    }

    pub fn rel(mut self, id: &str, kind: &str, target: &str) -> Self {
        self.rels.push((
            id.into(),
            format!("http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}"),
            target.into(),
        ));
        self
    }
}

pub fn styles(dxfs: &str, cell_xfs: &str, fonts: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><styleSheet {MAIN}>
<fonts count="1"><font><sz val="11"/><name val="Calibri"/></font>{fonts}</fonts>
<fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills>
<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
<cellXfs><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>{cell_xfs}</cellXfs>
<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>
<dxfs>{dxfs}</dxfs></styleSheet>"#
    )
}

/// A workbook of `sheets`, with `styles` and any further parts.
pub fn workbook(sheets: Vec<SheetPart>, styles: &str, parts: &[(&str, &str)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
        let options = SimpleFileOptions::default();
        let mut put = |name: &str, data: &str| {
            zip.start_file(name, options).unwrap();
            zip.write_all(data.as_bytes()).unwrap();
        };
        put(
            "[Content_Types].xml",
            r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/></Types>"#,
        );
        put(
            "_rels/.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        );
        let mut workbook_rels = String::new();
        let mut entries = String::new();
        for (index, sheet) in sheets.iter().enumerate() {
            let number = index + 1;
            workbook_rels.push_str(&format!(r#"<Relationship Id="rId{number}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{number}.xml"/>"#));
            entries.push_str(&format!(
                r#"<sheet name="{}" sheetId="{number}" r:id="rId{number}"/>"#,
                sheet.name
            ));
            put(
                &format!("xl/worksheets/sheet{number}.xml"),
                &format!(
                    r#"<?xml version="1.0"?><worksheet {MAIN}>{}</worksheet>"#,
                    sheet.xml
                ),
            );
            if !sheet.rels.is_empty() {
                let rels: String = sheet
                    .rels
                    .iter()
                    .map(|(id, kind, target)| {
                        format!(r#"<Relationship Id="{id}" Type="{kind}" Target="{target}"/>"#)
                    })
                    .collect();
                put(
                    &format!("xl/worksheets/_rels/sheet{number}.xml.rels"),
                    &format!(
                        r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>"#
                    ),
                );
            }
        }
        workbook_rels.push_str(r#"<Relationship Id="rIdS" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#);
        if parts
            .iter()
            .any(|(name, _)| *name == "xl/sharedStrings.xml")
        {
            workbook_rels.push_str(r#"<Relationship Id="rIdT" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>"#);
        }
        put(
            "xl/_rels/workbook.xml.rels",
            &format!(
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{workbook_rels}</Relationships>"#
            ),
        );
        put(
            "xl/workbook.xml",
            &format!(
                r#"<?xml version="1.0"?><workbook {MAIN}><sheets>{entries}</sheets></workbook>"#
            ),
        );
        put("xl/styles.xml", styles);
        for (name, data) in parts {
            put(name, data);
        }
        zip.finish().unwrap();
    }
    bytes
}

/// `<c>` for a number.
pub fn number(reference: &str, value: f64) -> String {
    format!(r#"<c r="{reference}"><v>{value}</v></c>"#)
}

/// Rows of numbers from `first` down column `column`.
pub fn column_of_numbers(column: &str, first_row: u32, values: &[f64]) -> String {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let row = first_row + index as u32;
            format!(
                r#"<row r="{row}">{}</row>"#,
                number(&format!("{column}{row}"), *value)
            )
        })
        .collect()
}
