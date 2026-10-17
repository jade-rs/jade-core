//! Builds `.xlsx` bytes in memory so the reader is exercised against a real
//! archive rather than a hand-fed XML string.
//!
//! The parts are written the way Excel writes them — namespaced root elements,
//! `r:id` sheet references, a `customWidth` column, a formula cell carrying its
//! cached value — so a test that passes here is not passing on a shape only
//! this file has.

use std::io::Write;
use zip::write::SimpleFileOptions;

pub const WORKBOOK_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
  <Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>
</Relationships>"#;

pub const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

pub const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <bookViews><workbookView activeTab="0"/></bookViews>
  <sheets>
    <sheet name="売上" sheetId="1" r:id="rId1"/>
    <sheet name="Sheet2" sheetId="2" state="veryHidden" r:id="rId2"/>
  </sheets>
  <definedNames>
    <definedName name="SalesData">売上!$A$1:$C$8</definedName>
    <definedName name="LocalTax" localSheetId="0">売上!$B$2</definedName>
    <definedName name="_xlnm.Print_Area" localSheetId="0" hidden="1">売上!$A$1:$C$8</definedName>
    <definedName name="Ampersand">&quot;A &amp; B&quot;</definedName>
  </definedNames>
  <calcPr calcId="191029" fullCalcOnLoad="1"/>
</workbook>"#;

/// `si` 2 carries an `rPh` phonetic guide, which must not join the value.
pub const SHARED_STRINGS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="5" uniqueCount="5">
  <si><t>Item</t></si>
  <si><r><rPr><b/></rPr><t>Total </t></r><r><t>(JPY)</t></r></si>
  <si><t>横浜</t><rPh sb="0" eb="2"><t>ヨコハマ</t></rPh><phoneticPr fontId="1"/></si>
  <si><t xml:space="preserve">  padded  </t></si>
  <si><t>a &amp; b &lt; c &gt; d</t></si>
</sst>"#;

pub const STYLES: &str = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <numFmts count="1"><numFmt numFmtId="164" formatCode="#,##0.00&quot;円&quot;"/></numFmts>
  <fonts count="2">
    <font><sz val="11"/><color theme="1"/><name val="Calibri"/></font>
    <font><b/><sz val="14"/><color rgb="FFCC0000"/><name val="Meiryo"/></font>
  </fonts>
  <fills count="3">
    <fill><patternFill patternType="none"/></fill>
    <fill><patternFill patternType="gray125"/></fill>
    <fill><patternFill patternType="solid"><fgColor rgb="FFFFFF00"/><bgColor indexed="64"/></patternFill></fill>
  </fills>
  <borders count="2">
    <border><left/><right/><top/><bottom/><diagonal/></border>
    <border>
      <left style="thin"><color rgb="FF000000"/></left>
      <right style="thin"><color rgb="FF000000"/></right>
      <top style="medium"><color rgb="FF000000"/></top>
      <bottom style="thin"><color rgb="FF000000"/></bottom>
      <diagonal/>
    </border>
  </borders>
  <cellXfs count="3">
    <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
    <xf numFmtId="0" fontId="1" fillId="2" borderId="1" xfId="0" applyFont="1" applyFill="1" applyBorder="1" applyAlignment="1">
      <alignment horizontal="center" vertical="center" wrapText="1" shrinkToFit="1"/>
    </xf>
    <xf numFmtId="164" fontId="0" fillId="0" borderId="1" xfId="0" applyNumberFormat="1"/>
  </cellXfs>
</styleSheet>"##;

pub const SHEET1: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheetViews><sheetView tabSelected="1" workbookViewId="0"><pane xSplit="1.0" ySplit="2" topLeftCell="B3" activePane="bottomRight" state="frozenSplit"/></sheetView></sheetViews>
  <sheetFormatPr defaultColWidth="8.83203125" defaultRowHeight="18.75"/>
  <cols>
    <col min="1" max="1" width="20.5" customWidth="1"/>
    <col min="2" max="2" width="12" customWidth="1" style="2"/>
    <col min="4" max="4" width="8.83203125" hidden="1"/>
  </cols>
  <sheetData>
    <row r="1" spans="1:3" ht="30" customHeight="1">
      <c r="A1" s="1" t="s"><v>0</v></c>
      <c r="B1" s="1" t="s"><v>1</v></c>
      <c r="C1" s="1" t="inlineStr"><is><t>Note</t></is></c>
    </row>
    <row r="2" spans="1:3">
      <c r="A2" t="s"><v>2</v></c>
      <c r="B2" s="2"><v>1234.5</v></c>
      <c r="C2" t="b"><v>1</v></c>
    </row>
    <row r="3" spans="1:3">
      <c r="A3" t="s"><v>3</v></c>
      <c r="B3" s="2"><f>SUM(B2:B2)</f><v>1234.5</v></c>
      <c r="C3" t="e"><v>#DIV/0!</v></c>
    </row>
    <row r="4" spans="1:3">
      <c r="A4" t="s"><v>4</v></c>
      <c r="B4" t="inlineStr"><is><t>x &amp; y</t></is></c>
    </row>
    <row r="5" spans="1:3" s="2" customFormat="1">
      <c r="C5"><v>1234.5</v></c>
    </row>
    <row r="6" spans="1:3" s="2">
      <c r="C6"><v>1234.5</v></c>
    </row>
    <row r="7" spans="1:3">
      <c r="A7"><v>1</v></c>
      <c r="B7"><v>2</v></c>
      <c r="C7"><f t="shared" ref="C7:C8" si="0">SUM(A7:B7)+$A7+A$1+$A$1+&quot;A7&quot;+LOG10(A7)+'Sheet 1'!A7+Sheet2!A7</f><v>7</v></c>
    </row>
    <row r="8" spans="1:3">
      <c r="A8"><v>3</v></c>
      <c r="B8"><v>4</v></c>
      <c r="C8"><f t="shared" si="0"/><v>11</v></c>
    </row>
  </sheetData>
  <mergeCells count="1"><mergeCell ref="A4:C4"/></mergeCells>
  <dataValidations count="3">
    <dataValidation type="whole" operator="between" allowBlank="1" showInputMessage="1" showErrorMessage="1" promptTitle="Quantity" prompt="Enter 0–10000" errorTitle="Invalid" error="Out of range" sqref="B2:B8">
      <formula1>0</formula1><formula2>10000</formula2>
    </dataValidation>
    <dataValidation type="list" allowBlank="0" sqref="A2 A4:A5"><formula1>&quot;横浜,東京&quot;</formula1></dataValidation>
    <dataValidation type="none" allowBlank="1" sqref="C2"/>
  </dataValidations>
  <hyperlinks>
    <hyperlink ref="A2" r:id="rId1" tooltip="Open example"/>
    <hyperlink ref="B2:C2" location="'Sheet2'!A1" display="Jump"/>
  </hyperlinks>
</worksheet>"#;

pub const SHEET1_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/a?x=1&amp;y=2" TargetMode="External"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="../comments1.xml"/>
</Relationships>"#;

pub const COMMENTS1: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <authors><author>Alice &amp; Bob</author><author>経理</author></authors>
  <commentList>
    <comment ref="A2" authorId="0"><text><t>External link note</t></text></comment>
    <comment ref="B2" authorId="1"><text><r><t>売上 </t></r><r><rPr><b/></rPr><t>確認</t></r></text></comment>
  </commentList>
</comments>"#;

pub const SHEET2: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetViews><sheetView showGridLines="0" workbookViewId="0"><pane xSplit="2000" ySplit="3000" topLeftCell="D10" state="split"/></sheetView></sheetViews>
  <sheetData/>
</worksheet>"#;

/// Zips the parts above into `.xlsx` bytes.
pub fn sample_workbook() -> Vec<u8> {
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        let options = SimpleFileOptions::default();
        for (name, contents) in [
            ("_rels/.rels", ROOT_RELS),
            ("xl/workbook.xml", WORKBOOK),
            ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS),
            ("xl/sharedStrings.xml", SHARED_STRINGS),
            ("xl/styles.xml", STYLES),
            ("xl/worksheets/sheet1.xml", SHEET1),
            ("xl/worksheets/_rels/sheet1.xml.rels", SHEET1_RELS),
            ("xl/comments1.xml", COMMENTS1),
            ("xl/worksheets/sheet2.xml", SHEET2),
        ] {
            writer.start_file(name, options).expect("start part");
            writer.write_all(contents.as_bytes()).expect("write part");
        }
        writer.finish().expect("finish archive");
    }
    buffer
}
