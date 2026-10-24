//! Envelopes and labels, as Word lays them out: an envelope is a page the
//! envelope's size with the return address in its top-left corner and the
//! delivery address placed well inside it (a Japanese 長形 envelope written
//! vertically, the name down its middle); a sheet of labels is a table with
//! a cell for each label, exact row heights and gap columns for the space
//! between labels, on a page whose margins put the cells over the labels.

use super::address::AddressBlockOptions;
use super::{result_runs_xml, MergeFieldSpec, MergeRule};
use crate::docx_xml::{DocxXmlDocument, XmlElement, XmlNode};
use crate::word_document::rewrite::{xml_escape, xml_fragment};

fn twips(mm: f32) -> i64 {
    (mm / 25.4 * 1440.0).round() as i64
}

/// An envelope's size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvelopeSize {
    pub name: &'static str,
    /// The page as the envelope is printed: width and height in mm.
    pub width_mm: f32,
    pub height_mm: f32,
    /// Written top to bottom, right to left: a Japanese 長形 or 角形.
    pub vertical: bool,
}

/// The sizes Word's 封筒オプション offers.
pub const ENVELOPE_SIZES: [EnvelopeSize; 13] = [
    EnvelopeSize {
        name: "長形 3 号",
        width_mm: 120.0,
        height_mm: 235.0,
        vertical: true,
    },
    EnvelopeSize {
        name: "長形 4 号",
        width_mm: 90.0,
        height_mm: 205.0,
        vertical: true,
    },
    EnvelopeSize {
        name: "角形 2 号",
        width_mm: 240.0,
        height_mm: 332.0,
        vertical: true,
    },
    EnvelopeSize {
        name: "洋形 2 号",
        width_mm: 162.0,
        height_mm: 114.0,
        vertical: false,
    },
    EnvelopeSize {
        name: "洋形 4 号",
        width_mm: 235.0,
        height_mm: 105.0,
        vertical: false,
    },
    EnvelopeSize {
        name: "洋長 3 号",
        width_mm: 235.0,
        height_mm: 120.0,
        vertical: false,
    },
    EnvelopeSize {
        name: "Com-10",
        width_mm: 241.3,
        height_mm: 104.8,
        vertical: false,
    },
    EnvelopeSize {
        name: "Monarch",
        width_mm: 190.5,
        height_mm: 98.4,
        vertical: false,
    },
    EnvelopeSize {
        name: "DL",
        width_mm: 220.0,
        height_mm: 110.0,
        vertical: false,
    },
    EnvelopeSize {
        name: "C4",
        width_mm: 324.0,
        height_mm: 229.0,
        vertical: false,
    },
    EnvelopeSize {
        name: "C5",
        width_mm: 229.0,
        height_mm: 162.0,
        vertical: false,
    },
    EnvelopeSize {
        name: "C6",
        width_mm: 162.0,
        height_mm: 114.0,
        vertical: false,
    },
    EnvelopeSize {
        name: "B5",
        width_mm: 250.0,
        height_mm: 176.0,
        vertical: false,
    },
];

/// What goes on an envelope and where.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvelopeSpec {
    pub size: EnvelopeSize,
    /// 宛先: lines of text, unless the merge's address block goes there.
    pub delivery: String,
    /// 差出人.
    pub return_address: String,
    /// 差出人を印刷しない.
    pub omit_return: bool,
    /// The address block field in place of `delivery`.
    pub merge: bool,
    /// The delivery address's distance from the left and top edges, in mm;
    /// `None` for Word's automatic place.
    pub delivery_left_mm: Option<f32>,
    pub delivery_top_mm: Option<f32>,
    pub japanese: bool,
}

impl EnvelopeSpec {
    /// Where the delivery address goes when nothing says: Word puts it a
    /// little left of the middle and below half way.
    pub fn delivery_position(&self) -> (f32, f32) {
        let size = self.size;
        let left = self.delivery_left_mm.unwrap_or(if size.vertical {
            size.width_mm * 0.12
        } else {
            size.width_mm * 0.4
        });
        let top = self.delivery_top_mm.unwrap_or(if size.vertical {
            size.height_mm * 0.12
        } else {
            size.height_mm * 0.45
        });
        (left, top)
    }
}

fn paragraph(text: &str, properties: &str, run_properties: &str) -> String {
    format!(
        "<w:p><w:pPr>{properties}<w:rPr>{run_properties}</w:rPr></w:pPr>{}</w:p>",
        result_runs_xml(text, &format!("<w:rPr>{run_properties}</w:rPr>"))
    )
}

fn address_block_field(japanese: bool, run_properties: &str) -> String {
    MergeFieldSpec::AddressBlock(AddressBlockOptions::new(japanese))
        .xml(&format!("<w:rPr>{run_properties}</w:rPr>"))
}

/// The body of an envelope: the addresses and the section the envelope's
/// size.
pub fn envelope_body(spec: &EnvelopeSpec) -> String {
    let size = spec.size;
    let margin = 10.0;
    let (left, top) = spec.delivery_position();
    let mut body = String::new();
    let tight = "<w:spacing w:before=\"0\" w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/>";
    if size.vertical {
        // 縦書き: the lines run down the page from the right. The address
        // first, the name larger down the middle, the sender at the left.
        let address = "<w:sz w:val=\"28\"/><w:szCs w:val=\"28\"/>";
        let name = "<w:sz w:val=\"44\"/><w:szCs w:val=\"44\"/>";
        if spec.merge {
            body.push_str(&format!(
                "<w:p><w:pPr>{tight}<w:ind w:left=\"{}\"/></w:pPr>{}</w:p>",
                twips(top - margin).max(0),
                address_block_field(spec.japanese, address)
            ));
        } else {
            let mut lines = spec
                .delivery
                .lines()
                .filter(|line| !line.trim().is_empty())
                .collect::<Vec<_>>();
            let name_line = lines.pop().unwrap_or_default();
            for line in lines {
                body.push_str(&paragraph(
                    line,
                    &format!("{tight}<w:ind w:left=\"{}\"/>", twips(top - margin).max(0)),
                    address,
                ));
            }
            body.push_str(&paragraph("", tight, address));
            body.push_str(&paragraph(
                name_line,
                &format!("{tight}<w:jc w:val=\"center\"/>"),
                name,
            ));
        }
        if !spec.omit_return && !spec.return_address.trim().is_empty() {
            body.push_str(&paragraph("", tight, address));
            for line in spec.return_address.lines() {
                body.push_str(&paragraph(
                    line,
                    &format!("{tight}<w:jc w:val=\"right\"/>"),
                    "<w:sz w:val=\"20\"/><w:szCs w:val=\"20\"/>",
                ));
            }
        }
        let _ = left;
        body.push_str(&format!(
            "<w:sectPr><w:pgSz w:w=\"{}\" w:h=\"{}\"/><w:pgMar w:top=\"{m}\" w:right=\"{m}\" w:bottom=\"{m}\" w:left=\"{m}\" w:header=\"0\" w:footer=\"0\" w:gutter=\"0\"/><w:textDirection w:val=\"tbRl\"/><w:docGrid w:linePitch=\"360\"/></w:sectPr>",
            twips(size.width_mm),
            twips(size.height_mm),
            m = twips(margin),
        ));
        return body;
    }
    let small = "<w:sz w:val=\"20\"/><w:szCs w:val=\"20\"/>";
    let large = "<w:sz w:val=\"24\"/><w:szCs w:val=\"24\"/>";
    let mut used_lines = 0.0;
    if !spec.omit_return && !spec.return_address.trim().is_empty() {
        for line in spec.return_address.lines() {
            body.push_str(&paragraph(line, tight, small));
            used_lines += 1.0;
        }
    }
    // The delivery address: down to its top, then in to its left.
    let line_mm = 4.2;
    let before = twips((top - margin - used_lines * line_mm).max(0.0));
    let indent = twips((left - margin).max(0.0));
    if spec.merge {
        body.push_str(&format!(
            "<w:p><w:pPr><w:spacing w:before=\"{before}\" w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/><w:ind w:left=\"{indent}\"/></w:pPr>{}</w:p>",
            address_block_field(spec.japanese, large)
        ));
    } else {
        for (index, line) in spec.delivery.lines().enumerate() {
            let spacing = if index == 0 {
                format!("<w:spacing w:before=\"{before}\" w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/>")
            } else {
                tight.to_string()
            };
            body.push_str(&paragraph(
                line,
                &format!("{spacing}<w:ind w:left=\"{indent}\"/>"),
                large,
            ));
        }
    }
    body.push_str(&format!(
        "<w:sectPr><w:pgSz w:w=\"{}\" w:h=\"{}\" w:orient=\"landscape\"/><w:pgMar w:top=\"{m}\" w:right=\"{m}\" w:bottom=\"{m}\" w:left=\"{m}\" w:header=\"0\" w:footer=\"0\" w:gutter=\"0\"/><w:docGrid w:linePitch=\"360\"/></w:sectPr>",
        twips(size.width_mm),
        twips(size.height_mm),
        m = twips(margin),
    ));
    body
}

/// A label sheet product: the page, the labels and where they are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabelProduct {
    pub vendor: &'static str,
    pub number: &'static str,
    pub description: &'static str,
    pub page_width_mm: f32,
    pub page_height_mm: f32,
    pub columns: usize,
    pub rows: usize,
    pub label_width_mm: f32,
    pub label_height_mm: f32,
    /// 上余白 and 横余白.
    pub top_mm: f32,
    pub side_mm: f32,
    /// The distance from one label to the next across and down (横方向 /
    /// 縦方向のラベル間隔).
    pub pitch_across_mm: f32,
    pub pitch_down_mm: f32,
}

const INCH: f32 = 25.4;

/// Label products the ラベル オプション offers: common Japanese A4 sheets
/// and Avery's.
pub const LABEL_PRODUCTS: [LabelProduct; 14] = [
    LabelProduct {
        vendor: "A-ONE",
        number: "72212",
        description: "A4 12面",
        page_width_mm: 210.0,
        page_height_mm: 297.0,
        columns: 2,
        rows: 6,
        label_width_mm: 86.4,
        label_height_mm: 42.3,
        top_mm: 21.6,
        side_mm: 18.6,
        pitch_across_mm: 86.4,
        pitch_down_mm: 42.3,
    },
    LabelProduct {
        vendor: "A-ONE",
        number: "72210",
        description: "A4 10面",
        page_width_mm: 210.0,
        page_height_mm: 297.0,
        columns: 2,
        rows: 5,
        label_width_mm: 86.4,
        label_height_mm: 50.8,
        top_mm: 21.5,
        side_mm: 18.6,
        pitch_across_mm: 86.4,
        pitch_down_mm: 50.8,
    },
    LabelProduct {
        vendor: "A-ONE",
        number: "72224",
        description: "A4 24面",
        page_width_mm: 210.0,
        page_height_mm: 297.0,
        columns: 3,
        rows: 8,
        label_width_mm: 66.0,
        label_height_mm: 33.9,
        top_mm: 12.9,
        side_mm: 6.0,
        pitch_across_mm: 66.0,
        pitch_down_mm: 33.9,
    },
    LabelProduct {
        vendor: "A-ONE",
        number: "72244",
        description: "A4 44面",
        page_width_mm: 210.0,
        page_height_mm: 297.0,
        columns: 4,
        rows: 11,
        label_width_mm: 48.3,
        label_height_mm: 25.4,
        top_mm: 8.8,
        side_mm: 8.4,
        pitch_across_mm: 48.3,
        pitch_down_mm: 25.4,
    },
    LabelProduct {
        vendor: "A-ONE",
        number: "72221",
        description: "A4 21面",
        page_width_mm: 210.0,
        page_height_mm: 297.0,
        columns: 3,
        rows: 7,
        label_width_mm: 70.0,
        label_height_mm: 42.3,
        top_mm: 0.4,
        side_mm: 0.0,
        pitch_across_mm: 70.0,
        pitch_down_mm: 42.3,
    },
    LabelProduct {
        vendor: "Avery",
        number: "L7160",
        description: "Address, A4 21",
        page_width_mm: 210.0,
        page_height_mm: 297.0,
        columns: 3,
        rows: 7,
        label_width_mm: 63.5,
        label_height_mm: 38.1,
        top_mm: 15.15,
        side_mm: 7.25,
        pitch_across_mm: 66.04,
        pitch_down_mm: 38.1,
    },
    LabelProduct {
        vendor: "Avery",
        number: "L7163",
        description: "Address, A4 14",
        page_width_mm: 210.0,
        page_height_mm: 297.0,
        columns: 2,
        rows: 7,
        label_width_mm: 99.1,
        label_height_mm: 38.1,
        top_mm: 15.15,
        side_mm: 4.65,
        pitch_across_mm: 101.6,
        pitch_down_mm: 38.1,
    },
    LabelProduct {
        vendor: "Avery",
        number: "L7159",
        description: "Address, A4 24",
        page_width_mm: 210.0,
        page_height_mm: 297.0,
        columns: 3,
        rows: 8,
        label_width_mm: 63.5,
        label_height_mm: 33.9,
        top_mm: 12.9,
        side_mm: 6.45,
        pitch_across_mm: 66.04,
        pitch_down_mm: 33.9,
    },
    LabelProduct {
        vendor: "Avery",
        number: "L7651",
        description: "Mini, A4 65",
        page_width_mm: 210.0,
        page_height_mm: 297.0,
        columns: 5,
        rows: 13,
        label_width_mm: 38.1,
        label_height_mm: 21.2,
        top_mm: 10.7,
        side_mm: 4.75,
        pitch_across_mm: 40.64,
        pitch_down_mm: 21.2,
    },
    LabelProduct {
        vendor: "Avery",
        number: "5160",
        description: "Address, Letter 30",
        page_width_mm: 8.5 * INCH,
        page_height_mm: 11.0 * INCH,
        columns: 3,
        rows: 10,
        label_width_mm: 2.625 * INCH,
        label_height_mm: 1.0 * INCH,
        top_mm: 0.5 * INCH,
        side_mm: 0.1875 * INCH,
        pitch_across_mm: 2.75 * INCH,
        pitch_down_mm: 1.0 * INCH,
    },
    LabelProduct {
        vendor: "Avery",
        number: "5161",
        description: "Address, Letter 20",
        page_width_mm: 8.5 * INCH,
        page_height_mm: 11.0 * INCH,
        columns: 2,
        rows: 10,
        label_width_mm: 4.0 * INCH,
        label_height_mm: 1.0 * INCH,
        top_mm: 0.5 * INCH,
        side_mm: 0.15625 * INCH,
        pitch_across_mm: 4.1875 * INCH,
        pitch_down_mm: 1.0 * INCH,
    },
    LabelProduct {
        vendor: "Avery",
        number: "5162",
        description: "Address, Letter 14",
        page_width_mm: 8.5 * INCH,
        page_height_mm: 11.0 * INCH,
        columns: 2,
        rows: 7,
        label_width_mm: 4.0 * INCH,
        label_height_mm: 1.33 * INCH,
        top_mm: 0.83 * INCH,
        side_mm: 0.15625 * INCH,
        pitch_across_mm: 4.1875 * INCH,
        pitch_down_mm: 1.33 * INCH,
    },
    LabelProduct {
        vendor: "Avery",
        number: "5163",
        description: "Shipping, Letter 10",
        page_width_mm: 8.5 * INCH,
        page_height_mm: 11.0 * INCH,
        columns: 2,
        rows: 5,
        label_width_mm: 4.0 * INCH,
        label_height_mm: 2.0 * INCH,
        top_mm: 0.5 * INCH,
        side_mm: 0.15625 * INCH,
        pitch_across_mm: 4.1875 * INCH,
        pitch_down_mm: 2.0 * INCH,
    },
    LabelProduct {
        vendor: "Avery",
        number: "5164",
        description: "Shipping, Letter 6",
        page_width_mm: 8.5 * INCH,
        page_height_mm: 11.0 * INCH,
        columns: 2,
        rows: 3,
        label_width_mm: 4.0 * INCH,
        label_height_mm: 3.33 * INCH,
        top_mm: 0.5 * INCH,
        side_mm: 0.15625 * INCH,
        pitch_across_mm: 4.1875 * INCH,
        pitch_down_mm: 3.33 * INCH,
    },
];

impl LabelProduct {
    /// The product's name as the list shows it.
    pub fn label(&self) -> String {
        format!("{} {} ({})", self.vendor, self.number, self.description)
    }

    /// The space between labels across: the gap column's width.
    pub fn gap_across_mm(&self) -> f32 {
        (self.pitch_across_mm - self.label_width_mm).max(0.0)
    }

    /// Whether the labels fit the page.
    pub fn fits(&self) -> bool {
        let across = self.side_mm
            + self.pitch_across_mm * (self.columns.saturating_sub(1)) as f32
            + self.label_width_mm;
        let down = self.top_mm
            + self.pitch_down_mm * (self.rows.saturating_sub(1)) as f32
            + self.label_height_mm;
        across <= self.page_width_mm + 0.5 && down <= self.page_height_mm + 0.5
    }
}

/// The body of a sheet of labels: each label `text`, or for a merge the
/// address block, after Next Record in every label but the first.
pub fn labels_body(
    product: &LabelProduct,
    text: Option<&str>,
    merge: bool,
    japanese: bool,
) -> String {
    let gap = product.gap_across_mm();
    let label_width = twips(product.label_width_mm);
    let gap_width = twips(gap);
    let height = twips(product.pitch_down_mm.max(product.label_height_mm));
    let mut grid = String::new();
    for column in 0..product.columns {
        if column > 0 && gap_width > 0 {
            grid.push_str(&format!("<w:gridCol w:w=\"{gap_width}\"/>"));
        }
        grid.push_str(&format!("<w:gridCol w:w=\"{label_width}\"/>"));
    }
    let tight = "<w:spacing w:before=\"0\" w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/>";
    let label_content = |first: bool| -> String {
        if merge {
            let next = if first {
                String::new()
            } else {
                MergeFieldSpec::Rule(MergeRule::Next).xml("")
            };
            format!(
                "<w:p><w:pPr>{tight}</w:pPr>{next}{}</w:p>",
                MergeFieldSpec::AddressBlock(AddressBlockOptions::new(japanese)).xml("")
            )
        } else {
            let text = text.unwrap_or("");
            text.split('\n')
                .map(|line| {
                    format!(
                        "<w:p><w:pPr>{tight}</w:pPr>{}</w:p>",
                        result_runs_xml(line, "")
                    )
                })
                .collect()
        }
    };
    let mut rows = String::new();
    let mut first = true;
    for _ in 0..product.rows {
        rows.push_str(&format!(
            "<w:tr><w:trPr><w:cantSplit/><w:trHeight w:val=\"{height}\" w:hRule=\"exact\"/></w:trPr>"
        ));
        for column in 0..product.columns {
            if column > 0 && gap_width > 0 {
                rows.push_str(&format!(
                    "<w:tc><w:tcPr><w:tcW w:w=\"{gap_width}\" w:type=\"dxa\"/></w:tcPr><w:p><w:pPr>{tight}</w:pPr></w:p></w:tc>"
                ));
            }
            rows.push_str(&format!(
                "<w:tc><w:tcPr><w:tcW w:w=\"{label_width}\" w:type=\"dxa\"/><w:vAlign w:val=\"center\"/></w:tcPr>{}</w:tc>",
                label_content(first)
            ));
            first = false;
        }
        rows.push_str("</w:tr>");
    }
    let right = twips(
        (product.page_width_mm
            - product.side_mm
            - product.pitch_across_mm * (product.columns.saturating_sub(1)) as f32
            - product.label_width_mm)
            .max(0.0),
    );
    format!(
        "<w:tbl><w:tblPr><w:tblW w:w=\"0\" w:type=\"auto\"/><w:tblLayout w:type=\"fixed\"/><w:tblCellMar><w:top w:w=\"0\" w:type=\"dxa\"/><w:left w:w=\"85\" w:type=\"dxa\"/><w:bottom w:w=\"0\" w:type=\"dxa\"/><w:right w:w=\"85\" w:type=\"dxa\"/></w:tblCellMar><w:tblLook w:val=\"04A0\" w:firstRow=\"1\" w:lastRow=\"0\" w:firstColumn=\"1\" w:lastColumn=\"0\" w:noHBand=\"0\" w:noVBand=\"1\"/></w:tblPr><w:tblGrid>{grid}</w:tblGrid>{rows}</w:tbl><w:p><w:pPr><w:spacing w:before=\"0\" w:after=\"0\" w:line=\"20\" w:lineRule=\"exact\"/><w:rPr><w:sz w:val=\"2\"/></w:rPr></w:pPr></w:p><w:sectPr><w:pgSz w:w=\"{}\" w:h=\"{}\"/><w:pgMar w:top=\"{}\" w:right=\"{right}\" w:bottom=\"0\" w:left=\"{}\" w:header=\"0\" w:footer=\"0\" w:gutter=\"0\"/><w:docGrid w:linePitch=\"360\"/></w:sectPr>",
        twips(product.page_width_mm),
        twips(product.page_height_mm),
        twips(product.top_mm),
        twips(product.side_mm),
    )
}

/// The first table's cells that hold labels: a gap column's cells are the
/// narrow ones.
fn label_cells(table: &XmlElement) -> Vec<(usize, usize)> {
    let mut cells = Vec::new();
    let rows = table.children.iter().filter_map(|node| match node {
        XmlNode::Element(row) if row.name == "w:tr" => Some(row),
        _ => None,
    });
    for (row_index, row) in rows.enumerate() {
        let row_cells = row.children.iter().filter_map(|node| match node {
            XmlNode::Element(cell) if cell.name == "w:tc" => Some(cell),
            _ => None,
        });
        for (cell_index, cell) in row_cells.enumerate() {
            let width = cell
                .children
                .iter()
                .find_map(|node| match node {
                    XmlNode::Element(properties) if properties.name == "w:tcPr" => {
                        properties.children.iter().find_map(|node| match node {
                            XmlNode::Element(width) if width.name == "w:tcW" => width
                                .attributes
                                .iter()
                                .find(|attribute| attribute.name == "w:w")
                                .and_then(|attribute| attribute.value.parse::<i64>().ok()),
                            _ => None,
                        })
                    }
                    _ => None,
                })
                .unwrap_or(i64::MAX);
            if width > twips(10.0) {
                cells.push((row_index, cell_index));
            }
        }
    }
    cells
}

/// ラベルの更新: the first label's paragraphs into every other label, after
/// a Next Record, as Word fills a merge's sheet of labels.
pub fn propagate_labels(xml: &mut DocxXmlDocument) -> Result<(), String> {
    fn find_table(element: &XmlElement) -> Option<crate::docx_xml::XmlNodeId> {
        for node in &element.children {
            if let XmlNode::Element(child) = node {
                if child.name == "w:tbl" {
                    return Some(child.id);
                }
                if child.name == "w:body" {
                    return find_table(child);
                }
            }
        }
        None
    }
    let table_id = find_table(xml.root()).ok_or("the document has no labels")?;
    let table = xml
        .element(table_id)
        .ok_or("the document has no labels")?
        .clone();
    let cells = label_cells(&table);
    let Some(&(first_row, first_cell)) = cells.first() else {
        return Err("the document has no labels".to_string());
    };
    let cell_at = |table: &XmlElement, row: usize, cell: usize| -> Option<XmlElement> {
        table
            .children
            .iter()
            .filter_map(|node| match node {
                XmlNode::Element(row) if row.name == "w:tr" => Some(row),
                _ => None,
            })
            .nth(row)?
            .children
            .iter()
            .filter_map(|node| match node {
                XmlNode::Element(cell) if cell.name == "w:tc" => Some(cell.clone()),
                _ => None,
            })
            .nth(cell)
    };
    let first = cell_at(&table, first_row, first_cell).ok_or("the first label is missing")?;
    let content: Vec<XmlNode> = first
        .children
        .iter()
        .filter(|node| !matches!(node, XmlNode::Element(element) if element.name == "w:tcPr"))
        .cloned()
        .collect();
    let next = xml_fragment(xml, &MergeFieldSpec::Rule(MergeRule::Next).xml(""));
    let mut table = table;
    let mut row_index = 0;
    for node in &mut table.children {
        let XmlNode::Element(row) = node else {
            continue;
        };
        if row.name != "w:tr" {
            continue;
        }
        let mut cell_index = 0;
        for node in &mut row.children {
            let XmlNode::Element(cell) = node else {
                continue;
            };
            if cell.name != "w:tc" {
                continue;
            }
            let position = (row_index, cell_index);
            cell_index += 1;
            if position == (first_row, first_cell) || !cells.contains(&position) {
                continue;
            }
            let mut copy = content.clone();
            // Next Record leads the label's first paragraph.
            if let Some(XmlNode::Element(paragraph)) = copy
                .iter_mut()
                .find(|node| matches!(node, XmlNode::Element(element) if element.name == "w:p"))
            {
                let at = paragraph
                    .children
                    .iter()
                    .position(|node| !matches!(node, XmlNode::Element(element) if element.name == "w:pPr"))
                    .unwrap_or(paragraph.children.len());
                for (offset, node) in next.iter().cloned().enumerate() {
                    paragraph.children.insert(at + offset, node);
                }
            }
            cell.children.retain(
                |node| matches!(node, XmlNode::Element(element) if element.name == "w:tcPr"),
            );
            cell.children.extend(copy);
        }
        row_index += 1;
    }
    let renewed = xml.create_element(table.name, table.attributes, table.children);
    let parent = xml.parent_id(table_id).ok_or("the labels have no parent")?;
    let parent_element = xml.element_mut(parent).ok_or("the labels have no parent")?;
    if let Some(node) = parent_element
        .children
        .iter_mut()
        .find(|node| matches!(node, XmlNode::Element(element) if element.id == table_id))
    {
        *node = XmlNode::Element(renewed);
    }
    xml.rebuild_index();
    let _ = xml_escape;
    Ok(())
}
