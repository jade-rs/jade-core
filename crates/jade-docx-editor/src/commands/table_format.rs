//! A table's own formatting carried to another table of the same shape, as
//! Word's paste carries it with the table: the table's properties
//! (`w:tblPr` — alignment, indent, borders, cell margins, spacing, layout,
//! direction, position), each row's (`w:trPr`) and each cell's (`w:tcPr` —
//! borders, shading, vertical alignment, text direction, margins, fit text).
//! What the cells hold, how they are merged and the table style are the
//! target's own.

use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{Table, TableCell, TableRow};
use crate::docx_xml::{XmlElement, XmlNode, XmlNodeId};
use crate::word_document::WordDocument;
use jade_docx_layout::selection::SelectionRange;

/// `w:tblPr` children a copied format leaves to the target: the table style
/// (pasted on its own) and the record of a tracked change.
const TABLE_KEPT: [&str; 2] = ["w:tblStyle", "w:tblPrChange"];
/// `w:trPr` children that belong to the row's place in the grid or its
/// revisions, not its formatting.
const ROW_KEPT: [&str; 7] = [
    "w:gridBefore",
    "w:wBefore",
    "w:gridAfter",
    "w:wAfter",
    "w:ins",
    "w:del",
    "w:trPrChange",
];
/// `w:tcPr` children that describe how the cell is merged, or its
/// revisions, not its formatting.
const CELL_KEPT: [&str; 7] = [
    "w:gridSpan",
    "w:vMerge",
    "w:hMerge",
    "w:cellIns",
    "w:cellDel",
    "w:cellMerge",
    "w:tcPrChange",
];

/// A table's own formatting as a copy takes it, to give a table of the same
/// shape with [`SetTableFormatCommand`].
#[derive(Clone, Debug)]
pub struct TableFormat {
    table: Table,
    xml: Option<XmlElement>,
    rows: Vec<RowFormat>,
}

#[derive(Clone, Debug)]
struct RowFormat {
    row: TableRow,
    xml: Option<XmlElement>,
    cells: Vec<(TableCell, Option<XmlElement>)>,
}

impl PartialEq for TableFormat {
    fn eq(&self, other: &Self) -> bool {
        format!("{self:?}") == format!("{other:?}")
    }
}

impl TableFormat {
    /// How many rows it formats, and how many cells each of them has.
    pub fn shape(&self) -> Vec<usize> {
        self.rows.iter().map(|row| row.cells.len()).collect()
    }

    /// The format of `count` of its rows from `first`, as a copy of those
    /// rows alone takes it.
    pub fn rows(&self, first: usize, count: usize) -> TableFormat {
        TableFormat {
            rows: self.rows.iter().skip(first).take(count).cloned().collect(),
            ..self.clone()
        }
    }
}

/// The first child of `parent` named `name`.
fn child<'a>(parent: &'a XmlElement, name: &str) -> Option<&'a XmlElement> {
    parent.children.iter().find_map(|child| match child {
        XmlNode::Element(element) if element.name == name => Some(element),
        _ => None,
    })
}

/// The children of `parent` named `name`, in order.
fn children<'a>(parent: &'a XmlElement, name: &'a str) -> impl Iterator<Item = &'a XmlElement> {
    parent.children.iter().filter_map(move |child| match child {
        XmlNode::Element(element) if element.name == name => Some(element),
        _ => None,
    })
}

fn children_mut<'a>(
    parent: &'a mut XmlElement,
    name: &'a str,
) -> impl Iterator<Item = &'a mut XmlElement> {
    parent
        .children
        .iter_mut()
        .filter_map(move |child| match child {
            XmlNode::Element(element) if element.name == name => Some(element),
            _ => None,
        })
}

/// `element` without the children named in `kept`, which the target keeps.
fn without(element: &XmlElement, kept: &[&str]) -> XmlElement {
    let mut element = element.clone();
    element.children.retain(
        |child| !matches!(child, XmlNode::Element(child) if kept.contains(&child.name.as_str())),
    );
    element
}

/// `element` with placeholder ids, so the document it goes into numbers it
/// afresh instead of taking ids that may be another node's there.
pub(crate) fn with_fresh_ids(mut element: XmlElement) -> XmlElement {
    fn clear(element: &mut XmlElement) {
        element.id = XmlNodeId::new(0);
        for child in &mut element.children {
            if let XmlNode::Element(child) = child {
                clear(child);
            }
        }
    }
    clear(&mut element);
    element
}

/// Puts `copied` in place of `parent`'s `name` child, keeping the target's
/// own children named in `kept`. The properties element goes first among
/// the parent's children (`w:tblPr` follows nothing; `w:trPr` follows only
/// `w:tblPrEx`; `w:tcPr` comes first).
fn replace_properties(
    parent: &mut XmlElement,
    name: &str,
    copied: Option<&XmlElement>,
    kept: &[&str],
) {
    let own = child(parent, name).cloned();
    let at = parent
        .children
        .iter()
        .position(|child| matches!(child, XmlNode::Element(element) if element.name == name))
        .unwrap_or_else(|| {
            parent
                .children
                .iter()
                .position(|child| matches!(child, XmlNode::Element(element) if element.name != "w:tblPrEx"))
                .unwrap_or(parent.children.len())
        });
    parent
        .children
        .retain(|child| !matches!(child, XmlNode::Element(element) if element.name == name));
    let mut properties = copied
        .map(|copied| with_fresh_ids(without(copied, kept)))
        .unwrap_or_else(|| XmlElement {
            id: XmlNodeId::new(0),
            name: name.to_string(),
            attributes: Vec::new(),
            children: Vec::new(),
        });
    // The target's own children named in `kept` stay, first, as the schema
    // orders `w:tblStyle`, `w:gridBefore` and `w:gridSpan` before the rest.
    if let Some(own) = own {
        let own_kept: Vec<XmlNode> = own
            .children
            .into_iter()
            .filter(|child| matches!(child, XmlNode::Element(element) if kept.contains(&element.name.as_str())))
            .collect();
        for (offset, child) in own_kept.into_iter().enumerate() {
            properties.children.insert(offset, child);
        }
    }
    if !properties.children.is_empty() || !properties.attributes.is_empty() {
        parent
            .children
            .insert(at.min(parent.children.len()), XmlNode::Element(properties));
    }
}

impl WordDocument {
    /// Table `order`'s own formatting, to give a table of the same shape with
    /// [`SetTableFormatCommand`].
    pub fn table_format(&self, order: usize) -> Option<TableFormat> {
        let table = self.table_by_order(order)?;
        let element = table
            .node_id
            .and_then(|id| self.raw_document.as_ref()?.element(id));
        let xml_rows: Vec<&XmlElement> = element
            .map(|element| children(element, "w:tr").collect())
            .unwrap_or_default();
        let rows = table
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let xml_row = xml_rows.get(index).copied();
                let xml_cells: Vec<&XmlElement> = xml_row
                    .map(|row| children(row, "w:tc").collect())
                    .unwrap_or_default();
                RowFormat {
                    row: TableRow {
                        cells: Vec::new(),
                        ..row.clone()
                    },
                    xml: xml_row.and_then(|row| child(row, "w:trPr")).cloned(),
                    cells: row
                        .cells
                        .iter()
                        .enumerate()
                        .map(|(cell_index, cell)| {
                            (
                                TableCell {
                                    blocks: Vec::new(),
                                    ..cell.clone()
                                },
                                xml_cells
                                    .get(cell_index)
                                    .and_then(|cell| child(cell, "w:tcPr"))
                                    .cloned(),
                            )
                        })
                        .collect(),
                }
            })
            .collect();
        Some(TableFormat {
            table: Table {
                rows: Vec::new(),
                ..table.clone()
            },
            xml: element
                .and_then(|element| child(element, "w:tblPr"))
                .cloned(),
            rows,
        })
    }
}

/// Gives table `order` a copied [`TableFormat`] in place of its own: its
/// properties, its rows' and its cells', keeping what the cells hold, how
/// they are merged, the table style and revision marks. The table must have
/// the copied one's shape: as many rows, each with as many cells.
pub struct SetTableFormatCommand {
    order: usize,
    format: TableFormat,
    previous: Option<(Table, Option<XmlElement>)>,
    previous_selection: Option<SelectionRange>,
}

impl SetTableFormatCommand {
    pub fn new(order: usize, format: TableFormat) -> Self {
        Self {
            order,
            format,
            previous: None,
            previous_selection: None,
        }
    }
}

impl Command for SetTableFormatCommand {
    fn name(&self) -> &'static str {
        "set_table_format"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let format = &self.format;
        let table = ctx
            .document
            .table_by_order(self.order)
            .ok_or_else(|| CommandError::Custom("the table is gone".into()))?
            .clone();
        let shape: Vec<usize> = table.rows.iter().map(|row| row.cells.len()).collect();
        if shape != format.shape() {
            return Err(CommandError::Custom(
                "the table is not the shape of the copied one".into(),
            ));
        }
        let element_id = table.node_id;
        let previous_xml = element_id
            .and_then(|id| ctx.document.raw_document.as_ref()?.element(id))
            .cloned();

        // The model: the copy's properties, the target's content and place.
        let mut updated = Table {
            rows: Vec::new(),
            columns: table.columns.clone(),
            style_name: table.style_name.clone(),
            resolved_style: table.resolved_style.clone(),
            node_id: table.node_id,
            ..format.table.clone()
        };
        for (own, copied) in table.rows.iter().zip(&format.rows) {
            let cells = own
                .cells
                .iter()
                .zip(&copied.cells)
                .map(|(own, (copied, _))| TableCell {
                    blocks: own.blocks.clone(),
                    span: own.span,
                    vertical_merge: own.vertical_merge.clone(),
                    node_id: own.node_id,
                    ..copied.clone()
                })
                .collect();
            updated.rows.push(TableRow {
                cells,
                grid_before: own.grid_before,
                grid_after: own.grid_after,
                width_before: own.width_before.clone(),
                width_after: own.width_after.clone(),
                node_id: own.node_id,
                revision: own.revision.clone(),
                ..copied.row.clone()
            });
        }

        // The XML: each properties element replaced, the target's grid,
        // merge, style and revision children kept.
        if let (Some(id), Some(raw)) = (element_id, ctx.document.raw_document.as_mut()) {
            if let Some(element) = raw.element_mut(id) {
                replace_properties(element, "w:tblPr", format.xml.as_ref(), &TABLE_KEPT);
                for (row, copied) in children_mut(element, "w:tr").zip(&format.rows) {
                    replace_properties(row, "w:trPr", copied.xml.as_ref(), &ROW_KEPT);
                    for (cell, (_, copied)) in children_mut(row, "w:tc").zip(&copied.cells) {
                        replace_properties(cell, "w:tcPr", copied.as_ref(), &CELL_KEPT);
                    }
                }
                raw.rebuild_index_for_subtree(id);
            }
        }
        *ctx.document
            .table_mut_by_order(self.order)
            .ok_or_else(|| CommandError::Custom("the table is gone".into()))? = updated;
        self.previous = Some((table, previous_xml));
        self.previous_selection = *ctx.selection;
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let (table, xml) = self
            .previous
            .take()
            .ok_or_else(|| CommandError::Custom("table format snapshot is missing".into()))?;
        if let (Some(id), Some(xml), Some(raw)) =
            (table.node_id, xml, ctx.document.raw_document.as_mut())
        {
            if let Some(element) = raw.element_mut(id) {
                *element = xml;
                raw.rebuild_index_for_subtree(id);
            }
        }
        *ctx.document
            .table_mut_by_order(self.order)
            .ok_or_else(|| CommandError::Custom("the table is gone".into()))? = table;
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandExecutor;
    use crate::Options;
    use futures::executor;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn docx(body: &str) -> Vec<u8> {
        let document = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}<w:sectPr/></w:body></w:document>"
        );
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options = SimpleFileOptions::default();
            for (path, value) in [
                (
                    "[Content_Types].xml",
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>",
                ),
                (
                    "_rels/.rels",
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>",
                ),
                ("word/document.xml", document.as_str()),
            ] {
                zip.start_file(path, options).unwrap();
                zip.write_all(value.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        cursor.into_inner()
    }

    fn open(body: &str) -> WordDocument {
        executor::block_on(WordDocument::load(&docx(body), &Options::default())).unwrap()
    }

    fn saved(document: &WordDocument) -> String {
        let bytes = document.export_docx().unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut xml = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml)
            .unwrap();
        xml
    }

    fn table(jc: &str, cells: [&str; 2], cell_props: &str) -> String {
        format!(
            "<w:tbl><w:tblPr><w:tblW w:w=\"0\" w:type=\"auto\"/>{jc}</w:tblPr><w:tblGrid><w:gridCol w:w=\"2000\"/><w:gridCol w:w=\"2000\"/></w:tblGrid>\
             <w:tr><w:tc><w:tcPr>{cell_props}</w:tcPr><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>",
            cells[0], cells[1]
        )
    }

    #[test]
    fn a_copied_table_format_replaces_the_tables_own_and_undoes() {
        let body = format!(
            "{}{}",
            table(
                "<w:jc w:val=\"center\"/>",
                ["a", "b"],
                "<w:tcBorders><w:tl2br w:val=\"single\" w:sz=\"4\" w:color=\"FF0000\"/></w:tcBorders><w:vAlign w:val=\"center\"/>"
            ),
            table("<w:tblInd w:w=\"720\" w:type=\"dxa\"/>", ["c", "d"], "<w:vAlign w:val=\"bottom\"/>")
        );
        let mut document = open(&body);
        let format = document.table_format(0).unwrap();
        let before = saved(&document);
        let mut executor = CommandExecutor::new();
        let mut selection = None;
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor
            .execute(Box::new(SetTableFormatCommand::new(1, format)), &mut ctx)
            .unwrap();
        let table = document.table_by_order(1).unwrap();
        assert_eq!(
            table.rows[0].cells[0].vertical_align.as_deref(),
            Some("center")
        );
        let xml = saved(&document);
        let second = &xml[xml
            .find(">c<")
            .map(|at| xml[..at].rfind("<w:tbl>").unwrap())
            .unwrap()..];
        assert!(second.contains("w:jc w:val=\"center\""), "{xml}");
        assert!(!second.contains("w:tblInd"), "{xml}");
        assert!(second.contains("w:tl2br"), "{xml}");
        assert!(!second.contains("bottom"), "{xml}");
        assert!(
            second.contains(">c<") && second.contains(">d<"),
            "the cells keep their text"
        );
        assert_eq!(xml.matches("w:tl2br").count(), 2, "{xml}");

        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor.undo(&mut ctx).unwrap();
        assert_eq!(saved(&document), before);
    }

    #[test]
    fn a_merged_cell_stays_merged_and_a_table_of_another_shape_is_refused() {
        let merged = "<w:tbl><w:tblPr><w:tblW w:w=\"0\" w:type=\"auto\"/></w:tblPr><w:tblGrid><w:gridCol w:w=\"2000\"/><w:gridCol w:w=\"2000\"/></w:tblGrid>\
             <w:tr><w:tc><w:tcPr><w:gridSpan w:val=\"2\"/></w:tcPr><w:p><w:r><w:t>wide</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>";
        let other = "<w:tbl><w:tblPr><w:tblW w:w=\"0\" w:type=\"auto\"/></w:tblPr><w:tblGrid><w:gridCol w:w=\"4000\"/></w:tblGrid>\
             <w:tr><w:tc><w:tcPr><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"FFFF00\"/></w:tcPr><w:p><w:r><w:t>one</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>";
        let mut document = open(&format!("{}{merged}{other}", table("", ["a", "b"], "")));
        let mut executor = CommandExecutor::new();
        let mut selection = None;
        // One cell to a row: the shaded single cell's format goes on the
        // merged cell, which stays two columns wide.
        let format = document.table_format(2).unwrap();
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor
            .execute(Box::new(SetTableFormatCommand::new(1, format)), &mut ctx)
            .unwrap();
        let xml = saved(&document);
        let wide = &xml[..xml.find("wide").unwrap()];
        let wide = &wide[wide.rfind("<w:tc>").unwrap()..];
        assert!(
            wide.contains("w:gridSpan w:val=\"2\"") && wide.contains("FFFF00"),
            "{xml}"
        );
        assert_eq!(
            document.table_by_order(1).unwrap().rows[0].cells[0].span,
            Some(2)
        );
        // Two cells to a row do not fit one.
        let format = document.table_format(0).unwrap();
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        assert!(executor
            .execute(Box::new(SetTableFormatCommand::new(1, format)), &mut ctx)
            .is_err());
    }
}
