//! Word's table tools that reshape or recompute a table rather than format
//! one cell: 表スタイルのオプション, セルの間隔, 文字列の方向, 左端からのインデント,
//! 表の位置, 文字列を表にする / 表の解除, 並べ替え, 計算式, and the built-in
//! table styles Word adds to `styles.xml` the first time one is applied.

use super::*;
use crate::commands::insert_text::InsertTextCommand;
use crate::commands::Command;
use crate::document::TextDirection;

/// Word's 表スタイルのオプション. Each switch that is `Some` is set; the others
/// keep their effective value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TableLookUpdate {
    pub first_row: Option<bool>,
    pub last_row: Option<bool>,
    pub first_column: Option<bool>,
    pub last_column: Option<bool>,
    pub banded_rows: Option<bool>,
    pub banded_columns: Option<bool>,
}

/// Word's 表の位置.
#[derive(Clone, Debug, PartialEq)]
pub struct TablePositionSpec {
    /// `left`, `center`, `right`, `inside`, `outside`; wins over the offset.
    pub horizontal_align: Option<String>,
    /// Points from the anchor.
    pub horizontal_offset: Option<f32>,
    /// `margin`, `page` or `text` (the column).
    pub horizontal_anchor: String,
    /// `top`, `center`, `bottom`, `inside`, `outside`; wins over the offset.
    pub vertical_align: Option<String>,
    pub vertical_offset: Option<f32>,
    /// `margin`, `page` or `text` (the paragraph: 文字列と一緒に移動する).
    pub vertical_anchor: String,
    pub distance_top: f32,
    pub distance_bottom: f32,
    pub distance_left: f32,
    pub distance_right: f32,
    /// 重ねて配置する (`w:tblOverlap`).
    pub allow_overlap: bool,
}

/// How 並べ替え compares one column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TableSortKind {
    #[default]
    Text,
    Number,
    Date,
}

/// One 並べ替え key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableSortKey {
    pub column: usize,
    pub kind: TableSortKind,
    pub descending: bool,
}

/// What splits a paragraph into cells, or joins cells into a paragraph.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Separator {
    Paragraph,
    Text(String),
}

impl Separator {
    fn parse(value: &str) -> Result<Self, String> {
        Ok(match value {
            "tab" | "\t" => Separator::Text("\t".to_string()),
            "comma" | "," => Separator::Text(",".to_string()),
            "paragraph" | "\n" => Separator::Paragraph,
            other if other.chars().count() == 1 => Separator::Text(other.to_string()),
            other => {
                return Err(format!(
                    "the separator must be tab, comma, paragraph or one character, not {other:?}"
                ))
            }
        })
    }
}

fn twips(points: f32) -> String {
    ((points * 20.0).round() as i64).to_string()
}

fn attr(name: &str, value: impl Into<String>) -> XmlAttribute {
    XmlAttribute {
        name: name.to_string(),
        value: value.into(),
    }
}

fn paragraph_plain_text(paragraph: &Paragraph) -> String {
    paragraph.runs.iter().map(|run| run.text.as_str()).collect()
}

/// What a paragraph shows: a field read from a file keeps its result beside
/// the run rather than in it.
fn paragraph_display_text(paragraph: &Paragraph) -> String {
    paragraph
        .runs
        .iter()
        .map(|run| match run.field.as_ref() {
            Some(field) if run.text.is_empty() => field
                .resolved_value
                .clone()
                .or_else(|| field.cached_text.clone())
                .unwrap_or_default(),
            _ => run.text.clone(),
        })
        .collect()
}

/// The text of a cell's paragraphs, one line each, nested tables included.
fn cell_plain_text(cell: &TableCell) -> String {
    let mut lines = Vec::new();
    for block in &cell.blocks {
        match block {
            BodyElement::Paragraph(paragraph) => lines.push(paragraph_display_text(paragraph)),
            BodyElement::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        lines.push(cell_plain_text(cell));
                    }
                }
            }
            BodyElement::Math(_) => {}
        }
    }
    lines.join("\n")
}

impl WordDocument {
    // ---- table and cell properties --------------------------------------------

    /// セルの間隔 (`w:tblCellSpacing`), in points; `None` removes it. Rows that
    /// carry their own spacing follow the table, as Word's dialog does.
    pub fn set_table_cell_spacing(
        &mut self,
        table_order_index: usize,
        spacing: Option<f32>,
    ) -> Result<(), String> {
        if let Some(value) = spacing {
            if !value.is_finite() || !(0.0..=792.0).contains(&value) {
                return Err("cell spacing must be between 0 and 792 points".to_string());
            }
        }
        let spacing = spacing.filter(|value| *value > 0.0);
        let (table_id, row_ids) = {
            let table = self
                .table_mut_by_order(table_order_index)
                .ok_or_else(|| format!("table {table_order_index} was not found"))?;
            table.cell_spacing = spacing;
            let mut row_ids = Vec::new();
            for row in &mut table.rows {
                if row.cell_spacing.take().is_some() {
                    row_ids.push(row.node_id);
                }
            }
            (table.node_id, row_ids)
        };
        for row_id in row_ids.into_iter().flatten() {
            if let Some(raw) = self.raw_document.as_mut() {
                let tr_pr = raw.element(row_id).and_then(|row| {
                    row.children.iter().find_map(|child| match child {
                        XmlNode::Element(element) if element.name == "w:trPr" => Some(element.id),
                        _ => None,
                    })
                });
                if let Some(tr_pr) = tr_pr.and_then(|id| raw.element_mut(id)) {
                    tr_pr.children.retain(|node| {
                        !matches!(node, XmlNode::Element(element) if element.name == "w:tblCellSpacing")
                    });
                }
                raw.rebuild_index_for_subtree(row_id);
            }
        }
        match spacing {
            Some(points) => self.set_table_property_element(
                table_id,
                "w:tblCellSpacing",
                vec![attr("w:w", twips(points)), attr("w:type", "dxa")],
            ),
            None => self.remove_table_property_element(table_id, "w:tblCellSpacing"),
        }
    }

    /// 文字列の方向 for one cell (`w:textDirection`). `None` or `lrTb` is
    /// horizontal text.
    pub fn set_table_cell_text_direction(
        &mut self,
        table_order_index: usize,
        row: usize,
        column: usize,
        direction: Option<&str>,
    ) -> Result<(), String> {
        let parsed = match direction {
            None => None,
            Some(value) => Some(TextDirection::try_from_str(value).ok_or_else(|| {
                format!(
                    "text direction must be lrTb, tbRl, btLr, lrTbV, tbRlV or btLrV, not {value:?}"
                )
            })?),
        };
        let parsed = parsed.filter(|direction| *direction != TextDirection::LrTb);
        let cell_id = {
            let cell = self.table_cell_mut_at(table_order_index, row, column)?;
            cell.text_direction = parsed;
            cell.node_id
        };
        match direction.filter(|_| parsed.is_some()) {
            Some(value) => self.set_cell_property_element(
                cell_id,
                "w:textDirection",
                vec![attr("w:val", value)],
            ),
            None => self.remove_cell_property_element(cell_id, "w:textDirection"),
        }
    }

    /// 左端からのインデント (`w:tblInd`), in points.
    pub fn set_table_indent(
        &mut self,
        table_order_index: usize,
        indent: f32,
    ) -> Result<(), String> {
        if !indent.is_finite() || indent.abs() > 1584.0 {
            return Err("the table indent must be within ±1584 points".to_string());
        }
        let table_id = {
            let table = self
                .table_mut_by_order(table_order_index)
                .ok_or_else(|| format!("table {table_order_index} was not found"))?;
            table
                .style
                .insert("margin-left".to_string(), format!("{indent:.2}pt"));
            table.style.remove("table-indent-width-type");
            table.node_id
        };
        self.set_table_property_element(
            table_id,
            "w:tblInd",
            vec![attr("w:w", twips(indent)), attr("w:type", "dxa")],
        )
    }

    /// 表の位置: the table floats (文字列の折り返し: する) where `spec` puts it.
    pub fn set_table_position(
        &mut self,
        table_order_index: usize,
        spec: &TablePositionSpec,
    ) -> Result<(), String> {
        let anchor = |value: &str| -> Result<String, String> {
            match value {
                "margin" | "page" | "text" => Ok(value.to_string()),
                other => Err(format!("an anchor is margin, page or text, not {other:?}")),
            }
        };
        let horizontal_anchor = anchor(&spec.horizontal_anchor)?;
        let vertical_anchor = anchor(&spec.vertical_anchor)?;
        if let Some(align) = spec.horizontal_align.as_deref() {
            if !matches!(align, "left" | "center" | "right" | "inside" | "outside") {
                return Err(format!(
                    "horizontal position {align:?} is not one Word knows"
                ));
            }
        }
        if let Some(align) = spec.vertical_align.as_deref() {
            if !matches!(align, "top" | "center" | "bottom" | "inside" | "outside") {
                return Err(format!("vertical position {align:?} is not one Word knows"));
            }
        }
        for value in [
            spec.distance_top,
            spec.distance_bottom,
            spec.distance_left,
            spec.distance_right,
        ] {
            if !value.is_finite() || !(0.0..=1584.0).contains(&value) {
                return Err("a distance from text must be between 0 and 1584 points".to_string());
            }
        }
        let table_id = {
            let table = self
                .table_mut_by_order(table_order_index)
                .ok_or_else(|| format!("table {table_order_index} was not found"))?;
            table.style.retain(|key, _| {
                key != "table-position"
                    && key != "table-overlap"
                    && !key.starts_with("table-position-")
                    && !key.starts_with("table-distance-")
            });
            let mut set = |key: &str, value: String| {
                table.style.insert(key.to_string(), value);
            };
            set("table-position", "floating".to_string());
            set(
                "table-position-horizontal-anchor",
                horizontal_anchor.clone(),
            );
            set("table-position-vertical-anchor", vertical_anchor.clone());
            match spec.horizontal_align.as_ref() {
                Some(align) => set("table-position-x-align", align.clone()),
                None => set(
                    "table-position-x",
                    format!("{:.2}pt", spec.horizontal_offset.unwrap_or(0.0)),
                ),
            }
            match spec.vertical_align.as_ref() {
                Some(align) => set("table-position-y-align", align.clone()),
                None => set(
                    "table-position-y",
                    format!("{:.2}pt", spec.vertical_offset.unwrap_or(0.0)),
                ),
            }
            for (key, value) in [
                ("table-distance-top", spec.distance_top),
                ("table-distance-bottom", spec.distance_bottom),
                ("table-distance-left", spec.distance_left),
                ("table-distance-right", spec.distance_right),
            ] {
                set(key, format!("{value:.2}pt"));
            }
            if !spec.allow_overlap {
                set("table-overlap", "never".to_string());
            }
            table.node_id
        };
        let mut attributes = vec![
            attr("w:leftFromText", twips(spec.distance_left)),
            attr("w:rightFromText", twips(spec.distance_right)),
            attr("w:topFromText", twips(spec.distance_top)),
            attr("w:bottomFromText", twips(spec.distance_bottom)),
            attr("w:vertAnchor", vertical_anchor),
            attr("w:horzAnchor", horizontal_anchor),
        ];
        match spec.horizontal_align.as_ref() {
            Some(align) => attributes.push(attr("w:tblpXSpec", align.clone())),
            None => attributes.push(attr(
                "w:tblpX",
                twips(spec.horizontal_offset.unwrap_or(0.0)),
            )),
        }
        match spec.vertical_align.as_ref() {
            Some(align) => attributes.push(attr("w:tblpYSpec", align.clone())),
            None => attributes.push(attr("w:tblpY", twips(spec.vertical_offset.unwrap_or(0.0)))),
        }
        self.set_table_property_element(table_id, "w:tblpPr", attributes)?;
        if spec.allow_overlap {
            self.remove_table_property_element(table_id, "w:tblOverlap")
        } else {
            self.set_table_property_element(table_id, "w:tblOverlap", vec![attr("w:val", "never")])
        }
    }

    // ---- built-in table styles ---------------------------------------------

    /// Whether the package has a `styles.xml` to hold table styles.
    pub fn has_styles_part(&self) -> bool {
        self.styles_part.is_some() && self.styles_part_path().is_some()
    }

    /// Resolves a built-in table style (`Table Grid`, `Grid Table 4 - Accent
    /// 1`, …) by id or name, adding Word's definition to `styles.xml` when the
    /// package does not define it yet — as Word does the first time one is
    /// applied. `None` for other names and for packages without a styles part.
    pub(crate) fn ensure_builtin_table_style(&mut self, requested: &str) -> Option<String> {
        let builtin = builtin_table_style(requested)?;
        if let Some(id) = self
            .styles_part
            .as_ref()
            .and_then(|styles| styles.resolve_table_style_id(builtin.name))
            .map(str::to_string)
        {
            return Some(id);
        }
        let styles_path = self.styles_part_path()?;
        let content = self.read_xml_part_content(&styles_path)?;
        let taken = |id: &str| {
            self.styles_part
                .as_ref()
                .is_some_and(|styles| styles.get_style(id).is_some())
        };
        let style_id = if taken(builtin.id) {
            (1u32..)
                .map(|suffix| format!("{}{suffix}", builtin.id))
                .find(|candidate| !taken(candidate))?
        } else {
            builtin.id.to_string()
        };
        let based_on = self
            .styles_part
            .as_ref()
            .and_then(|styles| styles.default_table_style_id())
            .map(|id| format!("<w:basedOn w:val=\"{}\"/>", xml_escape_text(id)))
            .unwrap_or_default();
        let style = format!(
            "<w:style w:type=\"table\" w:styleId=\"{id}\"><w:name w:val=\"{name}\"/>{based_on}\
             <w:uiPriority w:val=\"{priority}\"/>{body}</w:style>",
            id = xml_escape_text(&style_id),
            name = xml_escape_text(builtin.name),
            priority = builtin.priority,
            body = builtin.body(),
        );
        let close = content.rfind("</w:styles>")?;
        let mut serialized = content.clone();
        serialized.insert_str(close, &style);
        // Through the XML document, so the part stays well formed text.
        let serialized = DocxXmlDocument::parse(&serialized).ok()?.to_string().ok()?;
        self.upsert_xml_part_content(&styles_path, serialized.clone());
        match self.styles_part.as_mut() {
            Some(styles) => styles.parse(&serialized).ok()?,
            None => self.styles_part = StylesPart::from_xml(&serialized).ok(),
        }
        Some(style_id)
    }

    // ---- converting ----------------------------------------------------------

    /// 文字列を表にする. Returns the new table's order index.
    pub fn convert_text_to_table(
        &mut self,
        start_paragraph: usize,
        end_paragraph: usize,
        separator: &str,
        columns: Option<usize>,
    ) -> Result<usize, String> {
        let separator = Separator::parse(separator)?;
        let (first, last) = (
            start_paragraph.min(end_paragraph),
            start_paragraph.max(end_paragraph),
        );
        for index in [first, last] {
            if !self.is_top_level_body_paragraph_surface(index) {
                return Err(format!(
                    "paragraph {index} is not a body paragraph outside a table"
                ));
            }
        }
        let first_body = self
            .body_index_for_surface(first)
            .ok_or("the first paragraph has no body position")?;
        let last_body = self
            .body_index_for_surface(last)
            .ok_or("the last paragraph has no body position")?;
        {
            let body = self
                .document
                .as_ref()
                .and_then(|document| document.body.as_ref())
                .ok_or("the document has no body")?;
            for index in first_body..=last_body {
                match body.elements.get(index) {
                    Some(BodyElement::Paragraph(paragraph)) => {
                        if paragraph
                            .properties
                            .as_ref()
                            .is_some_and(|properties| properties.section.is_some())
                        {
                            return Err(
                                "the text holds a section break; convert each section's text on its own"
                                    .to_string(),
                            );
                        }
                    }
                    _ => {
                        return Err(
                            "the text to convert must be paragraphs only, without a table"
                                .to_string(),
                        )
                    }
                }
            }
        }
        let body_len = self
            .document
            .as_ref()
            .and_then(|document| document.body.as_ref())
            .map_or(0, |body| body.elements.len());
        // A table cannot end the body: the paragraph after it holds the body's
        // last mark. Split one off the end when the text runs to the end.
        if last_body + 1 >= body_len {
            let surface = self
                .surface_index_for_body(last_body)
                .ok_or("the last paragraph has no surface")?;
            let length = self
                .paragraph(surface)
                .map(paragraph_plain_text)
                .map_or(0, |text| text.len());
            if !self.split_paragraph_at(surface, length) {
                return Err("the last paragraph could not be split".to_string());
            }
        }

        // Each paragraph's pieces, right to left so earlier positions hold.
        let mut pieces_per_paragraph = Vec::new();
        for body_index in (first_body..=last_body).rev() {
            let surface = self
                .surface_index_for_body(body_index)
                .ok_or("a paragraph has no surface")?;
            let text = self
                .paragraph(surface)
                .map(paragraph_plain_text)
                .unwrap_or_default();
            let Separator::Text(separator) = &separator else {
                pieces_per_paragraph.push(1usize);
                continue;
            };
            let offsets: Vec<usize> = text
                .match_indices(separator.as_str())
                .map(|(at, _)| at)
                .collect();
            for &offset in offsets.iter().rev() {
                self.delete_model_text(surface, offset, offset + separator.len())?;
                if !self.split_paragraph_at(surface, offset) {
                    return Err(format!("paragraph {surface} could not be split"));
                }
            }
            pieces_per_paragraph.push(offsets.len() + 1);
        }
        pieces_per_paragraph.reverse();
        let total: usize = pieces_per_paragraph.iter().sum();

        // The cells each piece goes to, row by row.
        let mut grid: Vec<Vec<usize>> = Vec::new();
        match &separator {
            Separator::Paragraph => {
                let width = columns.unwrap_or(1).max(1);
                let mut row = Vec::new();
                for piece in 0..total {
                    row.push(piece);
                    if row.len() == width {
                        grid.push(std::mem::take(&mut row));
                    }
                }
                if !row.is_empty() {
                    grid.push(row);
                }
            }
            Separator::Text(_) => {
                let widest = pieces_per_paragraph.iter().copied().max().unwrap_or(1);
                let width = columns.unwrap_or(widest).max(1);
                let mut piece = 0;
                for count in &pieces_per_paragraph {
                    let pieces: Vec<usize> = (piece..piece + count).collect();
                    piece += count;
                    for chunk in pieces.chunks(width) {
                        grid.push(chunk.to_vec());
                    }
                }
            }
        }
        let column_count = grid.iter().map(Vec::len).max().unwrap_or(1).max(1);
        let column_count = columns.unwrap_or(column_count).max(column_count);
        let row_count = grid.len().max(1);

        let table = self.build_table_for_insert(row_count, column_count, &[], None);
        let table_order = self.insert_top_level_table(first_body + total, table)?;

        // The pieces leave the body for their cells, XML first.
        let piece_ids: Vec<Option<XmlNodeId>> = {
            let body = self
                .document
                .as_ref()
                .and_then(|document| document.body.as_ref())
                .ok_or("the document has no body")?;
            (first_body..first_body + total)
                .map(|index| {
                    body.elements
                        .get(index)
                        .and_then(BodyElement::as_paragraph)
                        .and_then(|paragraph| paragraph.node_id)
                })
                .collect()
        };
        let cell_ids: Vec<Vec<Option<XmlNodeId>>> = self
            .table_by_order(table_order)
            .map(|table| {
                table
                    .rows
                    .iter()
                    .map(|row| row.cells.iter().map(|cell| cell.node_id).collect())
                    .collect()
            })
            .unwrap_or_default();
        if let Some(mut raw) = self.raw_document.take() {
            let result = (|| -> Result<(), String> {
                let mut nodes: Vec<Option<XmlNode>> = vec![None; total];
                for (piece, id) in piece_ids.iter().enumerate().rev() {
                    let Some(id) = id else { continue };
                    let Some(path) = raw.path(*id) else { continue };
                    nodes[piece] = Some(raw.remove_node(&path).map_err(|error| error.to_string())?);
                    raw.rebuild_index();
                }
                for (row, pieces) in grid.iter().enumerate() {
                    for (column, piece) in pieces.iter().enumerate() {
                        let Some(Some(cell_id)) = cell_ids.get(row).and_then(|row| row.get(column))
                        else {
                            continue;
                        };
                        let Some(node) = nodes[*piece].take() else {
                            continue;
                        };
                        let cell = raw.element_mut(*cell_id).ok_or("a new cell has no XML")?;
                        cell.children.retain(
                            |child| !matches!(child, XmlNode::Element(element) if element.name == "w:p"),
                        );
                        let at = cell.children.len();
                        raw.insert_node_preserving_ids(*cell_id, at, node)
                            .map_err(|error| error.to_string())?;
                    }
                }
                Ok(())
            })();
            raw.rebuild_index();
            self.raw_document = Some(raw);
            result?;
        }
        {
            let document = self.document_mut().ok_or("the document is unavailable")?;
            let body = document.body.as_mut().ok_or("the document has no body")?;
            let mut pieces: Vec<Option<BodyElement>> = body
                .elements
                .drain(first_body..first_body + total)
                .map(Some)
                .collect();
            let BodyElement::Table(table) = body
                .elements
                .get_mut(first_body)
                .ok_or("the new table went missing")?
            else {
                return Err("the new table went missing".to_string());
            };
            for (row, cells) in grid.iter().enumerate() {
                for (column, piece) in cells.iter().enumerate() {
                    let Some(cell) = table
                        .rows
                        .get_mut(row)
                        .and_then(|row| row.cells.get_mut(column))
                    else {
                        continue;
                    };
                    if let Some(element) = pieces[*piece].take() {
                        cell.blocks = vec![element];
                    }
                }
            }
            Self::rebuild_section_ranges(document);
        }
        self.rebuild_paragraph_surfaces();
        Ok(table_order)
    }

    /// 表の解除: the table becomes paragraphs, one per row, its cells joined
    /// by `separator` — or one per cell paragraph for `paragraph`.
    pub fn convert_table_to_text(
        &mut self,
        table_order_index: usize,
        separator: &str,
    ) -> Result<(), String> {
        let separator = Separator::parse(separator)?;
        let body_index = self
            .body_index_of_table_order(table_order_index)
            .ok_or_else(|| {
                if self.table_by_order(table_order_index).is_some() {
                    "a table inside a cell is converted from its own cell; select the outer table"
                        .to_string()
                } else {
                    format!("table {table_order_index} was not found")
                }
            })?;
        // The paragraphs each row gives, cell by cell.
        // Per row, per cell kept, the cell's paragraphs' XML ids.
        type RowParagraphs = Vec<Vec<Option<XmlNodeId>>>;
        let (table_id, rows): (Option<XmlNodeId>, Vec<RowParagraphs>) = {
            let table = self
                .table_by_order(table_order_index)
                .ok_or_else(|| format!("table {table_order_index} was not found"))?;
            let mut rows = Vec::new();
            for row in &table.rows {
                let mut cells = Vec::new();
                for cell in &row.cells {
                    if cell.vertical_merge.as_deref() == Some("continue") {
                        continue;
                    }
                    let mut paragraphs = Vec::new();
                    for block in &cell.blocks {
                        match block {
                            BodyElement::Paragraph(paragraph) => paragraphs.push(paragraph.node_id),
                            BodyElement::Table(_) => {
                                return Err(
                                    "a cell holds a table; convert that table to text first"
                                        .to_string(),
                                )
                            }
                            BodyElement::Math(_) => {
                                return Err("a cell holds an equation block".to_string())
                            }
                        }
                    }
                    cells.push(paragraphs);
                }
                rows.push(cells);
            }
            (table.node_id, rows)
        };

        // XML: the cell paragraphs go where the table was.
        if let (Some(mut raw), Some(table_id)) = (self.raw_document.take(), table_id) {
            let result = (|| -> Result<(), String> {
                let parent = raw.parent_id(table_id).ok_or("the table has no parent")?;
                let path = raw.path(table_id).ok_or("the table has no path")?;
                let mut at = *path.as_slice().last().ok_or("the table has no index")?;
                let mut moved = Vec::new();
                for cells in &rows {
                    for paragraphs in cells {
                        for id in paragraphs.iter().flatten() {
                            let path = raw.path(*id).ok_or("a cell paragraph has no path")?;
                            moved.push(raw.remove_node(&path).map_err(|error| error.to_string())?);
                            raw.rebuild_index();
                        }
                    }
                }
                let path = raw.path(table_id).ok_or("the table has no path")?;
                raw.remove_node(&path).map_err(|error| error.to_string())?;
                for node in moved {
                    raw.insert_node_preserving_ids(parent, at, node)
                        .map_err(|error| error.to_string())?;
                    at += 1;
                }
                Ok(())
            })();
            raw.rebuild_index();
            self.raw_document = Some(raw);
            result?;
        }
        // The model, the same way.
        let mut counts: Vec<Vec<usize>> = Vec::new();
        {
            let document = self.document_mut().ok_or("the document is unavailable")?;
            let body = document.body.as_mut().ok_or("the document has no body")?;
            let BodyElement::Table(table) = body.elements.remove(body_index) else {
                return Err("the table went missing".to_string());
            };
            let mut paragraphs = Vec::new();
            for row in table.rows {
                let mut row_counts = Vec::new();
                for cell in row.cells {
                    if cell.vertical_merge.as_deref() == Some("continue") {
                        continue;
                    }
                    let mut count = 0;
                    for block in cell.blocks {
                        if let BodyElement::Paragraph(_) = block {
                            paragraphs.push(block);
                            count += 1;
                        }
                    }
                    row_counts.push(count);
                }
                counts.push(row_counts);
            }
            for (offset, paragraph) in paragraphs.into_iter().enumerate() {
                body.elements.insert(body_index + offset, paragraph);
            }
            Self::rebuild_section_ranges(document);
        }
        self.rebuild_paragraph_surfaces();

        // Each row's cells become one paragraph, last row first so the body
        // positions before it hold.
        let Separator::Text(separator) = separator else {
            return Ok(());
        };
        let mut row_start = body_index + counts.iter().flatten().sum::<usize>();
        for row in counts.iter().rev() {
            row_start -= row.iter().sum::<usize>();
            let mut boundary = row_start;
            let mut boundaries = Vec::new();
            for count in &row[..row.len().saturating_sub(1)] {
                boundary += count;
                // The last paragraph of this cell, joined to the next one's
                // first. An empty cell has one empty paragraph.
                boundaries.push(boundary - 1);
            }
            for &last in boundaries.iter().rev() {
                self.join_body_paragraphs_with(last, &separator)?;
            }
        }
        Ok(())
    }

    /// Appends `text` to the body paragraph at `body_index` and joins the next
    /// one onto it.
    fn join_body_paragraphs_with(&mut self, body_index: usize, text: &str) -> Result<(), String> {
        let surface = self
            .surface_index_for_body(body_index)
            .ok_or("a paragraph to join has no surface")?;
        let next = self
            .surface_index_for_body(body_index + 1)
            .ok_or("the paragraph after it has no surface")?;
        let length = self
            .paragraph(surface)
            .map(paragraph_plain_text)
            .map_or(0, |text| text.len());
        let end = self.model_to_rendered_text_position(TextPosition::new(surface, length));
        let mut selection = Some(SelectionRange::collapsed(end));
        {
            let mut ctx = CommandContext::new(self, &mut selection);
            InsertTextCommand::new(SelectionRange::collapsed(end), text.to_string(), None)
                .apply(&mut ctx)
                .map_err(|error| format!("{error:?}"))?;
        }
        let joined_end = TextPosition::new(end.paragraph, end.offset + text.len());
        let range = SelectionRange::new(joined_end, TextPosition::new(next, 0));
        let mut selection = Some(range);
        let mut ctx = CommandContext::new(self, &mut selection);
        DeleteTextCommand::new(range, DeleteDirection::Forward)
            .apply(&mut ctx)
            .map_err(|error| format!("{error:?}"))
    }

    /// Deletes model text in one paragraph, through the editor's own delete.
    fn delete_model_text(&mut self, surface: usize, from: usize, to: usize) -> Result<(), String> {
        let start = self.model_to_rendered_text_position(TextPosition::new(surface, from));
        let end = self.model_to_rendered_text_position(TextPosition::new(surface, to));
        let range = SelectionRange::new(start, end);
        let mut selection = Some(range);
        let mut ctx = CommandContext::new(self, &mut selection);
        DeleteTextCommand::new(range, DeleteDirection::Forward)
            .apply(&mut ctx)
            .map_err(|error| format!("{error:?}"))
    }

    // ---- sorting -------------------------------------------------------------

    /// 並べ替え: the rows after `header_rows` ordered by `keys`.
    pub fn sort_table(
        &mut self,
        table_order_index: usize,
        keys: &[TableSortKey],
        header_rows: usize,
    ) -> Result<(), String> {
        if keys.is_empty() {
            return Err("sort needs at least one key".to_string());
        }
        {
            let table = self
                .table_mut_by_order(table_order_index)
                .ok_or_else(|| format!("table {table_order_index} was not found"))?;
            let header_rows = header_rows.min(table.rows.len());
            let sortable = &table.rows[header_rows..];
            if sortable
                .iter()
                .any(|row| row.cells.iter().any(|cell| cell.vertical_merge.is_some()))
            {
                return Err("rows with cells merged down cannot be sorted".to_string());
            }
            let text_at = |row: &TableRow, column: usize| -> String {
                Self::find_row_boundary(row, column)
                    .and_then(|boundary| row.cells.get(boundary.cell_index))
                    .map(cell_plain_text)
                    .unwrap_or_default()
            };
            let mut rows: Vec<(Vec<SortValue>, TableRow)> = table
                .rows
                .drain(header_rows..)
                .map(|row| {
                    let values = keys
                        .iter()
                        .map(|key| SortValue::of(&text_at(&row, key.column), key.kind))
                        .collect();
                    (values, row)
                })
                .collect();
            rows.sort_by(|(a, _), (b, _)| {
                for (index, key) in keys.iter().enumerate() {
                    let order = a[index].compare(&b[index]);
                    let order = if key.descending {
                        order.reverse()
                    } else {
                        order
                    };
                    if order != std::cmp::Ordering::Equal {
                        return order;
                    }
                }
                std::cmp::Ordering::Equal
            });
            table.rows.extend(rows.into_iter().map(|(_, row)| row));
        }
        self.rewrite_table_xml_from_model(table_order_index)?;
        self.rebuild_paragraph_surfaces();
        Ok(())
    }

    // ---- formulas ------------------------------------------------------------

    /// The table cell paragraph `surface` is in, innermost table first: the
    /// table, the row, and the cell's first grid column.
    fn table_cell_of_surface(&self, surface: usize) -> Option<(&Table, usize, usize)> {
        let Some(ParagraphSurface::Body { path }) = self.paragraph_surfaces.get(surface) else {
            return None;
        };
        let body = self.document.as_ref()?.body.as_ref()?;
        let mut elements: &[BodyElement] = &body.elements;
        let mut found = None;
        let mut index = 0;
        while index < path.len() {
            let PathEntry::Element(element_index) = path[index] else {
                return None;
            };
            match elements.get(element_index)? {
                BodyElement::Table(table) => {
                    let PathEntry::TableCell { row, cell } = *path.get(index + 1)? else {
                        return None;
                    };
                    let row_model = table.rows.get(row)?;
                    let column = Self::row_cell_boundaries(row_model).get(cell)?.start;
                    found = Some((table.as_ref(), row, column));
                    elements = &row_model.cells.get(cell)?.blocks;
                    index += 2;
                }
                BodyElement::Paragraph(_) | BodyElement::Math(_) => break,
            }
        }
        found
    }

    /// Computes a `=` field's result for the cell paragraph `surface` is in.
    pub fn evaluate_table_formula(
        &self,
        surface: usize,
        formula: &str,
        number_format: Option<&str>,
    ) -> Result<String, String> {
        let (table, row, column) = self
            .table_cell_of_surface(surface)
            .ok_or("a formula goes in a table cell")?;
        let grid = TableGridValues::of(table);
        let value = formula_eval::evaluate(formula, &grid, row, column)?;
        Ok(formula_eval::format_number(value, number_format))
    }

    /// 計算式: the `=` field at a caret in a cell, its result computed now.
    pub fn insert_table_formula(
        &mut self,
        paragraph: usize,
        offset: usize,
        formula: &str,
        number_format: Option<&str>,
    ) -> Result<(), String> {
        let formula = formula.trim().trim_start_matches('=').trim();
        if formula.is_empty() {
            return Err("the formula is empty".to_string());
        }
        let text = self.evaluate_table_formula(paragraph, formula, number_format)?;
        let instruction = formula_instruction(formula, number_format);
        self.insert_formula_field_at(TextPosition::new(paragraph, offset), &text, instruction)
    }

    fn insert_formula_field_at(
        &mut self,
        at: TextPosition,
        text: &str,
        instruction: String,
    ) -> Result<(), String> {
        let range = SelectionRange::collapsed(at);
        let mut selection = Some(range);
        {
            let mut ctx = CommandContext::new(self, &mut selection);
            InsertTextCommand::new(range, text.to_string(), None)
                .apply(&mut ctx)
                .map_err(|error| format!("{error:?}"))?;
        }
        let end = TextPosition::new(at.paragraph, at.offset + text.len());
        self.promote_inserted_text_to_field(end, text, RunFieldKind::Unsupported { instruction })
    }

    /// Word's フィールド更新 over every `=` field in a table: each result is
    /// computed again from the cells as they are now. Returns how many changed.
    pub fn update_table_formulas(&mut self) -> Result<usize, String> {
        let mut changed = 0;
        for surface in 0..self.paragraph_surfaces.len() {
            if self.table_cell_of_surface(surface).is_none() {
                continue;
            }
            let fields: Vec<(usize, String)> = match self.paragraph(surface) {
                Some(paragraph) => paragraph
                    .runs
                    .iter()
                    .enumerate()
                    .filter_map(
                        |(index, run)| match run.field.as_ref().map(|field| &field.kind) {
                            Some(RunFieldKind::Unsupported { instruction })
                                if instruction.trim_start().starts_with('=') =>
                            {
                                Some((index, instruction.clone()))
                            }
                            _ => None,
                        },
                    )
                    .collect(),
                None => continue,
            };
            let mut touched = false;
            for (run_index, instruction) in fields {
                let (formula, number_format) = parse_formula_instruction(&instruction);
                let Ok(value) =
                    self.evaluate_table_formula(surface, &formula, number_format.as_deref())
                else {
                    continue;
                };
                let Some(run) = self
                    .paragraph_mut(surface)
                    .and_then(|paragraph| paragraph.runs.get_mut(run_index))
                else {
                    continue;
                };
                let Some(field) = run.field.as_mut() else {
                    continue;
                };
                if run.text == value && field.cached_text.as_deref() == Some(value.as_str()) {
                    continue;
                }
                // As the reference fields are brought up to date: the run
                // shows the result and the saved result follows it.
                run.text = value.clone();
                field.resolved_value = Some(value.clone());
                field.cached_text = Some(value);
                touched = true;
                changed += 1;
            }
            if touched {
                self.sync_paragraph_text(surface);
            }
        }
        Ok(changed)
    }
}

/// `=SUM(ABOVE) \# "#,##0"`, as the field code is written.
fn formula_instruction(formula: &str, number_format: Option<&str>) -> String {
    match number_format
        .map(str::trim)
        .filter(|format| !format.is_empty())
    {
        Some(format) => format!("={formula} \\# \"{format}\""),
        None => format!("={formula}"),
    }
}

/// The formula and its `\#` picture, from a field code.
fn parse_formula_instruction(instruction: &str) -> (String, Option<String>) {
    let body = instruction.trim().trim_start_matches('=');
    match body.find("\\#") {
        Some(at) => {
            let format = body[at + 2..].trim().trim_matches('"').to_string();
            (body[..at].trim().to_string(), Some(format))
        }
        None => (body.trim().to_string(), None),
    }
}

/// A cell's value for sorting.
#[derive(Clone, Debug)]
enum SortValue {
    Missing,
    Number(f64),
    Text(String),
}

impl SortValue {
    fn of(text: &str, kind: TableSortKind) -> Self {
        match kind {
            TableSortKind::Text => {
                let text = text.trim();
                if text.is_empty() {
                    SortValue::Missing
                } else {
                    SortValue::Text(text.to_lowercase())
                }
            }
            TableSortKind::Number => formula_eval::first_number(text)
                .map(SortValue::Number)
                .unwrap_or(SortValue::Missing),
            TableSortKind::Date => parse_date(text)
                .map(SortValue::Number)
                .unwrap_or(SortValue::Missing),
        }
    }

    /// Ascending: what has no value first, as Word sorts blanks.
    fn compare(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        match (self, other) {
            (SortValue::Missing, SortValue::Missing) => Ordering::Equal,
            (SortValue::Missing, _) => Ordering::Less,
            (_, SortValue::Missing) => Ordering::Greater,
            (SortValue::Number(a), SortValue::Number(b)) => {
                a.partial_cmp(b).unwrap_or(Ordering::Equal)
            }
            (SortValue::Text(a), SortValue::Text(b)) => a.cmp(b),
            (SortValue::Number(_), SortValue::Text(_)) => Ordering::Less,
            (SortValue::Text(_), SortValue::Number(_)) => Ordering::Greater,
        }
    }
}

/// Days since 0000-01-01-ish for `yyyy/mm/dd`, `yyyy-mm-dd`, `yyyy.mm.dd`,
/// `yyyy年m月d日`, `m/d/yyyy`.
fn parse_date(text: &str) -> Option<f64> {
    let normalized: String = text
        .trim()
        .chars()
        .map(|ch| match ch {
            '０'..='９' => char::from_u32(ch as u32 - '０' as u32 + '0' as u32).unwrap_or(ch),
            '年' | '月' | '-' | '.' | '／' => '/',
            '日' => ' ',
            other => other,
        })
        .collect();
    let parts: Vec<i64> = normalized
        .split(|ch: char| ch == '/' || ch.is_whitespace())
        .filter(|part| !part.is_empty())
        .take(3)
        .map(|part| part.parse::<i64>().ok())
        .collect::<Option<Vec<_>>>()?;
    let [a, b, c] = parts[..] else {
        return None;
    };
    let (year, month, day) = if a > 31 { (a, b, c) } else { (c, a, b) };
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year * 372 + (month - 1) * 31 + (day - 1)) as f64)
}

/// The numbers of a table's cells by grid position, for formulas.
pub(crate) struct TableGridValues {
    /// Per row, per grid column: the cell's text, or `None` where no cell
    /// starts (inside a span, or past the row's end).
    cells: Vec<Vec<Option<String>>>,
}

impl TableGridValues {
    fn of(table: &Table) -> Self {
        let cells = table
            .rows
            .iter()
            .map(|row| {
                let mut columns = Vec::new();
                for boundary in WordDocument::row_cell_boundaries(row) {
                    while columns.len() < boundary.start {
                        columns.push(None);
                    }
                    let cell = &row.cells[boundary.cell_index];
                    let text = if cell.vertical_merge.as_deref() == Some("continue") {
                        None
                    } else {
                        Some(cell_plain_text(cell))
                    };
                    columns.push(text);
                    while columns.len() < boundary.end {
                        columns.push(None);
                    }
                }
                columns
            })
            .collect();
        Self { cells }
    }
}

impl formula_eval::Grid for TableGridValues {
    fn rows(&self) -> usize {
        self.cells.len()
    }

    fn columns(&self, row: usize) -> usize {
        self.cells.get(row).map_or(0, Vec::len)
    }

    fn text(&self, row: usize, column: usize) -> Option<&str> {
        self.cells.get(row)?.get(column)?.as_deref()
    }
}

/// Word's `=` field expressions: arithmetic, comparisons, cell references
/// (`A1`, `B2:C4`), the directions `ABOVE`, `BELOW`, `LEFT`, `RIGHT`, and the
/// functions Word's 計算式 dialog lists.
pub(crate) mod formula_eval {
    pub trait Grid {
        fn rows(&self) -> usize;
        fn columns(&self, row: usize) -> usize;
        fn text(&self, row: usize, column: usize) -> Option<&str>;
    }

    /// The first number in a cell's text, as Word reads one: grouping commas,
    /// a currency sign and full-width digits allowed; `(5)` is −5.
    pub fn first_number(text: &str) -> Option<f64> {
        let text: String = text
            .chars()
            .map(|ch| match ch {
                '０'..='９' => char::from_u32(ch as u32 - '０' as u32 + '0' as u32).unwrap_or(ch),
                '．' => '.',
                '，' => ',',
                '－' | '−' => '-',
                other => other,
            })
            .collect();
        let chars: Vec<char> = text.chars().collect();
        let start = chars
            .iter()
            .position(|ch| ch.is_ascii_digit() || *ch == '.')?;
        let negative = chars[..start]
            .iter()
            .rev()
            .find(|ch| !ch.is_whitespace() && !matches!(ch, '$' | '¥' | '￥' | '€' | '£'))
            .is_some_and(|ch| *ch == '-' || *ch == '(');
        let mut number = String::new();
        let mut seen_point = false;
        for ch in &chars[start..] {
            match ch {
                '0'..='9' => number.push(*ch),
                ',' if !seen_point => {}
                '.' if !seen_point => {
                    seen_point = true;
                    number.push('.');
                }
                _ => break,
            }
        }
        let value: f64 = number.trim_end_matches('.').parse().ok()?;
        let percent = chars[start..]
            .iter()
            .find(|ch| !(ch.is_ascii_digit() || matches!(ch, ',' | '.')))
            .is_some_and(|ch| *ch == '%');
        let value = if percent { value / 100.0 } else { value };
        Some(if negative { -value } else { value })
    }

    #[derive(Clone, Debug, PartialEq)]
    enum Token {
        Number(f64),
        Name(String),
        Operator(char),
        Compare(&'static str),
        Open,
        Close,
        Separator,
        Colon,
    }

    fn tokens(source: &str) -> Result<Vec<Token>, String> {
        let chars: Vec<char> = source.chars().collect();
        let mut out = Vec::new();
        let mut index = 0;
        while index < chars.len() {
            let ch = chars[index];
            match ch {
                ' ' | '\t' | '\u{3000}' => index += 1,
                '0'..='9' | '.' => {
                    let start = index;
                    while index < chars.len()
                        && (chars[index].is_ascii_digit() || chars[index] == '.')
                    {
                        index += 1;
                    }
                    let text: String = chars[start..index].iter().collect();
                    out.push(Token::Number(
                        text.parse()
                            .map_err(|_| format!("{text} is not a number"))?,
                    ));
                }
                'a'..='z' | 'A'..='Z' => {
                    let start = index;
                    while index < chars.len() && chars[index].is_ascii_alphanumeric() {
                        index += 1;
                    }
                    out.push(Token::Name(
                        chars[start..index]
                            .iter()
                            .collect::<String>()
                            .to_uppercase(),
                    ));
                }
                '+' | '-' | '*' | '/' | '^' | '%' => {
                    out.push(Token::Operator(ch));
                    index += 1;
                }
                '<' | '>' | '=' => {
                    let next = chars.get(index + 1).copied();
                    let (op, width) = match (ch, next) {
                        ('<', Some('=')) => ("<=", 2),
                        ('>', Some('=')) => (">=", 2),
                        ('<', Some('>')) => ("<>", 2),
                        ('<', _) => ("<", 1),
                        ('>', _) => (">", 1),
                        _ => ("=", 1),
                    };
                    out.push(Token::Compare(op));
                    index += width;
                }
                '(' => {
                    out.push(Token::Open);
                    index += 1;
                }
                ')' => {
                    out.push(Token::Close);
                    index += 1;
                }
                ',' | ';' => {
                    out.push(Token::Separator);
                    index += 1;
                }
                ':' => {
                    out.push(Token::Colon);
                    index += 1;
                }
                other => return Err(format!("{other:?} is not part of a formula")),
            }
        }
        Ok(out)
    }

    struct Parser<'a, G: Grid> {
        tokens: Vec<Token>,
        at: usize,
        grid: &'a G,
        row: usize,
        column: usize,
    }

    /// `A1` as (row, column), zero-based.
    fn cell_reference(name: &str) -> Option<(usize, usize)> {
        let letters: String = name
            .chars()
            .take_while(|ch| ch.is_ascii_alphabetic())
            .collect();
        let digits = &name[letters.len()..];
        if letters.is_empty() || digits.is_empty() || !digits.chars().all(|ch| ch.is_ascii_digit())
        {
            return None;
        }
        let column = letters.chars().fold(0usize, |total, ch| {
            total * 26 + (ch as usize - 'A' as usize + 1)
        });
        let row: usize = digits.parse().ok()?;
        (row >= 1 && column >= 1).then_some((row - 1, column - 1))
    }

    impl<'a, G: Grid> Parser<'a, G> {
        fn peek(&self) -> Option<&Token> {
            self.tokens.get(self.at)
        }

        fn next(&mut self) -> Option<Token> {
            let token = self.tokens.get(self.at).cloned();
            self.at += 1;
            token
        }

        fn expect(&mut self, token: Token) -> Result<(), String> {
            match self.next() {
                Some(found) if found == token => Ok(()),
                _ => Err("the formula is missing a bracket or a separator".to_string()),
            }
        }

        fn comparison(&mut self) -> Result<f64, String> {
            let left = self.additive()?;
            if let Some(Token::Compare(op)) = self.peek().cloned() {
                self.at += 1;
                let right = self.additive()?;
                let truth = match op {
                    "<" => left < right,
                    ">" => left > right,
                    "<=" => left <= right,
                    ">=" => left >= right,
                    "<>" => left != right,
                    _ => left == right,
                };
                return Ok(if truth { 1.0 } else { 0.0 });
            }
            Ok(left)
        }

        fn additive(&mut self) -> Result<f64, String> {
            let mut value = self.multiplicative()?;
            while let Some(Token::Operator(op @ ('+' | '-'))) = self.peek().cloned() {
                self.at += 1;
                let right = self.multiplicative()?;
                value = if op == '+' {
                    value + right
                } else {
                    value - right
                };
            }
            Ok(value)
        }

        fn multiplicative(&mut self) -> Result<f64, String> {
            let mut value = self.power()?;
            while let Some(Token::Operator(op @ ('*' | '/'))) = self.peek().cloned() {
                self.at += 1;
                let right = self.power()?;
                value = if op == '*' {
                    value * right
                } else if right == 0.0 {
                    return Err("!ゼロ除算です".to_string());
                } else {
                    value / right
                };
            }
            Ok(value)
        }

        fn power(&mut self) -> Result<f64, String> {
            let base = self.unary()?;
            if let Some(Token::Operator('^')) = self.peek() {
                self.at += 1;
                let exponent = self.power()?;
                return Ok(base.powf(exponent));
            }
            Ok(base)
        }

        fn unary(&mut self) -> Result<f64, String> {
            match self.peek() {
                Some(Token::Operator('-')) => {
                    self.at += 1;
                    Ok(-self.unary()?)
                }
                Some(Token::Operator('+')) => {
                    self.at += 1;
                    self.unary()
                }
                _ => {
                    let value = self.primary()?;
                    if let Some(Token::Operator('%')) = self.peek() {
                        self.at += 1;
                        return Ok(value / 100.0);
                    }
                    Ok(value)
                }
            }
        }

        fn primary(&mut self) -> Result<f64, String> {
            match self.next() {
                Some(Token::Number(value)) => Ok(value),
                Some(Token::Open) => {
                    let value = self.comparison()?;
                    self.expect(Token::Close)?;
                    Ok(value)
                }
                Some(Token::Name(name)) => {
                    if let Some(Token::Open) = self.peek() {
                        self.at += 1;
                        return self.function(&name);
                    }
                    match name.as_str() {
                        "TRUE" => Ok(1.0),
                        "FALSE" => Ok(0.0),
                        _ => {
                            let values = self.reference_values(&name)?;
                            Ok(values.first().copied().unwrap_or(0.0))
                        }
                    }
                }
                _ => Err("!構文エラーです".to_string()),
            }
        }

        /// The numbers a direction, a cell or a range covers.
        fn reference_values(&mut self, name: &str) -> Result<Vec<f64>, String> {
            let grid = self.grid;
            let number_at =
                |row: usize, column: usize| grid.text(row, column).and_then(first_number);
            let walk = |cells: Vec<(usize, usize)>| -> Vec<f64> {
                // Word's direction ranges run from the next cell out until a
                // cell without a number, skipping blank cells before the first.
                let mut values = Vec::new();
                for (row, column) in cells {
                    match number_at(row, column) {
                        Some(value) => values.push(value),
                        None if values.is_empty() => {
                            if grid
                                .text(row, column)
                                .is_some_and(|text| !text.trim().is_empty())
                            {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                values
            };
            let (row, column) = (self.row, self.column);
            Ok(match name {
                "ABOVE" => walk((0..row).rev().map(|r| (r, column)).collect()),
                "BELOW" => walk((row + 1..grid.rows()).map(|r| (r, column)).collect()),
                "LEFT" => walk((0..column).rev().map(|c| (row, c)).collect()),
                "RIGHT" => walk((column + 1..grid.columns(row)).map(|c| (row, c)).collect()),
                _ => {
                    let start = cell_reference(name)
                        .ok_or_else(|| format!("!定義されていないブックマークです ({name})"))?;
                    let end = if let Some(Token::Colon) = self.peek() {
                        self.at += 1;
                        match self.next() {
                            Some(Token::Name(other)) => cell_reference(&other)
                                .ok_or_else(|| format!("{other} is not a cell"))?,
                            _ => return Err("a range needs a cell after the colon".to_string()),
                        }
                    } else {
                        start
                    };
                    let mut values = Vec::new();
                    for r in start.0.min(end.0)..=start.0.max(end.0) {
                        for c in start.1.min(end.1)..=start.1.max(end.1) {
                            if let Some(value) = number_at(r, c) {
                                values.push(value);
                            }
                        }
                    }
                    values
                }
            })
        }

        fn arguments(&mut self) -> Result<Vec<f64>, String> {
            let mut values = Vec::new();
            if let Some(Token::Close) = self.peek() {
                self.at += 1;
                return Ok(values);
            }
            loop {
                // A bare direction or range stands for all its numbers.
                let is_reference = match (self.peek(), self.tokens.get(self.at + 1)) {
                    (Some(Token::Name(name)), next) => {
                        !matches!(next, Some(Token::Open))
                            && (matches!(name.as_str(), "ABOVE" | "BELOW" | "LEFT" | "RIGHT")
                                || matches!(next, Some(Token::Colon)))
                    }
                    _ => false,
                };
                if is_reference {
                    let Some(Token::Name(name)) = self.next() else {
                        unreachable!()
                    };
                    values.extend(self.reference_values(&name)?);
                } else {
                    values.push(self.comparison()?);
                }
                match self.next() {
                    Some(Token::Separator) => continue,
                    Some(Token::Close) => return Ok(values),
                    _ => return Err("!構文エラーです".to_string()),
                }
            }
        }

        fn function(&mut self, name: &str) -> Result<f64, String> {
            let values = self.arguments()?;
            let first = || values.first().copied().unwrap_or(0.0);
            Ok(match name {
                "SUM" => values.iter().sum(),
                "AVERAGE" => {
                    if values.is_empty() {
                        0.0
                    } else {
                        values.iter().sum::<f64>() / values.len() as f64
                    }
                }
                "COUNT" => values.len() as f64,
                "MAX" => values.iter().copied().reduce(f64::max).unwrap_or(0.0),
                "MIN" => values.iter().copied().reduce(f64::min).unwrap_or(0.0),
                "PRODUCT" => values.iter().product(),
                "ABS" => first().abs(),
                "INT" => first().floor(),
                "SIGN" => {
                    let value = first();
                    if value > 0.0 {
                        1.0
                    } else if value < 0.0 {
                        -1.0
                    } else {
                        0.0
                    }
                }
                "MOD" => {
                    let (a, b) = (first(), values.get(1).copied().unwrap_or(0.0));
                    if b == 0.0 {
                        return Err("!ゼロ除算です".to_string());
                    }
                    a % b
                }
                "ROUND" => {
                    let places = values.get(1).copied().unwrap_or(0.0) as i32;
                    let factor = 10f64.powi(places);
                    (first() * factor).round() / factor
                }
                "AND" => {
                    (values.iter().all(|value| *value != 0.0) && !values.is_empty()) as i32 as f64
                }
                "OR" => values.iter().any(|value| *value != 0.0) as i32 as f64,
                "NOT" => (first() == 0.0) as i32 as f64,
                "IF" => {
                    if first() != 0.0 {
                        values.get(1).copied().unwrap_or(0.0)
                    } else {
                        values.get(2).copied().unwrap_or(0.0)
                    }
                }
                "DEFINED" => 1.0,
                "TRUE" => 1.0,
                "FALSE" => 0.0,
                other => return Err(format!("!{other} は関数ではありません")),
            })
        }
    }

    /// Evaluates `formula` (without its `=`) for the cell at `(row, column)`.
    pub fn evaluate<G: Grid>(
        formula: &str,
        grid: &G,
        row: usize,
        column: usize,
    ) -> Result<f64, String> {
        let formula = formula.trim().trim_start_matches('=');
        let mut parser = Parser {
            tokens: tokens(formula)?,
            at: 0,
            grid,
            row,
            column,
        };
        let value = parser.comparison()?;
        if parser.at != parser.tokens.len() {
            return Err("!構文エラーです".to_string());
        }
        if !value.is_finite() {
            return Err("!ゼロ除算です".to_string());
        }
        Ok(value)
    }

    /// Formats a result with a `\#` picture — `0`, `#,##0`, `0.00`, `0%`,
    /// `¥#,##0`, `$#,##0.00;($#,##0.00)` — or Word's general form without one.
    pub fn format_number(value: f64, picture: Option<&str>) -> String {
        let Some(picture) = picture.map(str::trim).filter(|picture| !picture.is_empty()) else {
            let rounded = (value * 1e10).round() / 1e10;
            if rounded.fract() == 0.0 && rounded.abs() < 1e15 {
                return format!("{}", rounded as i64);
            }
            let text = format!("{rounded:.10}");
            return text.trim_end_matches('0').trim_end_matches('.').to_string();
        };
        let sections: Vec<&str> = picture.split(';').collect();
        let (section, value, sign) = match sections.as_slice() {
            [positive, negative, ..] if value < 0.0 => (*negative, -value, ""),
            [positive, ..] => (*positive, value.abs(), if value < 0.0 { "-" } else { "" }),
            [] => (picture, value.abs(), if value < 0.0 { "-" } else { "" }),
        };
        let is_digit = |ch: char| matches!(ch, '0' | '#' | 'x');
        let chars: Vec<char> = section.chars().collect();
        let Some(first) = chars.iter().position(|ch| is_digit(*ch)) else {
            return format!("{sign}{section}");
        };
        let last = chars
            .iter()
            .rposition(|ch| is_digit(*ch) || *ch == '.' || *ch == ',')
            .unwrap_or(first);
        let prefix: String = chars[..first].iter().filter(|ch| **ch != '\'').collect();
        let suffix: String = chars[last + 1..].iter().filter(|ch| **ch != '\'').collect();
        let digits: String = chars[first..=last].iter().collect();
        let percent = suffix.contains('%');
        let value = if percent { value * 100.0 } else { value };
        let (integer_picture, fraction_picture) = match digits.split_once('.') {
            Some((integer, fraction)) => (integer, fraction),
            None => (digits.as_str(), ""),
        };
        let places = fraction_picture.chars().filter(|ch| is_digit(*ch)).count();
        let grouping = integer_picture.contains(',');
        let minimum_integer = integer_picture.chars().filter(|ch| *ch == '0').count();
        // Half away from zero, as Word rounds, not to even.
        let factor = 10f64.powi(places as i32);
        let rounded = format!("{:.*}", places, (value * factor).round() / factor);
        let (integer, fraction) = match rounded.split_once('.') {
            Some((integer, fraction)) => (integer.to_string(), fraction.to_string()),
            None => (rounded, String::new()),
        };
        let mut integer = integer.trim_start_matches('0').to_string();
        while integer.len() < minimum_integer {
            integer.insert(0, '0');
        }
        if grouping {
            let digits: Vec<char> = integer.chars().collect();
            let mut grouped = String::new();
            for (index, ch) in digits.iter().enumerate() {
                if index > 0 && (digits.len() - index).is_multiple_of(3) {
                    grouped.push(',');
                }
                grouped.push(*ch);
            }
            integer = grouped;
        }
        // Optional fraction digits (`#`) drop their trailing zeros.
        let mut fraction: Vec<char> = fraction.chars().collect();
        let optional: Vec<bool> = fraction_picture
            .chars()
            .filter(|ch| is_digit(*ch))
            .map(|ch| ch == '#')
            .collect();
        while fraction.last() == Some(&'0') && optional.get(fraction.len() - 1) == Some(&true) {
            fraction.pop();
        }
        let number = if fraction.is_empty() {
            if integer.is_empty() {
                "0".to_string()
            } else {
                integer
            }
        } else {
            format!("{integer}.{}", fraction.iter().collect::<String>())
        };
        format!("{sign}{prefix}{number}{suffix}")
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        struct Cells(Vec<Vec<&'static str>>);

        impl Grid for Cells {
            fn rows(&self) -> usize {
                self.0.len()
            }
            fn columns(&self, row: usize) -> usize {
                self.0[row].len()
            }
            fn text(&self, row: usize, column: usize) -> Option<&str> {
                self.0.get(row)?.get(column).copied()
            }
        }

        fn sheet() -> Cells {
            Cells(vec![
                vec!["品目", "数量", "単価"],
                vec!["りんご", "3", "¥1,200"],
                vec!["みかん", "5", "300"],
                vec!["合計", "", ""],
            ])
        }

        #[test]
        fn directions_sum_the_numbers_next_to_the_cell() {
            let grid = sheet();
            assert_eq!(evaluate("SUM(ABOVE)", &grid, 3, 1).unwrap(), 8.0);
            assert_eq!(evaluate("SUM(ABOVE)", &grid, 3, 2).unwrap(), 1500.0);
            assert_eq!(evaluate("SUM(LEFT)", &grid, 1, 2).unwrap(), 3.0);
            assert_eq!(evaluate("AVERAGE(BELOW)", &grid, 0, 1).unwrap(), 4.0);
            assert_eq!(evaluate("MAX(B2:C3)", &grid, 3, 1).unwrap(), 1200.0);
            assert_eq!(evaluate("B2*C2+B3*C3", &grid, 3, 2).unwrap(), 5100.0);
            assert_eq!(evaluate("ROUND(10/3,2)", &grid, 0, 0).unwrap(), 3.33);
            assert_eq!(evaluate("IF(B2>B3,1,2)", &grid, 0, 0).unwrap(), 2.0);
            assert!(evaluate("SUM(", &grid, 0, 0).is_err());
            assert!(evaluate("1/0", &grid, 0, 0).is_err());
        }

        #[test]
        fn pictures_format_as_word_does() {
            assert_eq!(format_number(1234.5, None), "1234.5");
            assert_eq!(format_number(8.0, None), "8");
            assert_eq!(format_number(1234.5, Some("#,##0")), "1,235");
            assert_eq!(format_number(1234.5, Some("#,##0.00")), "1,234.50");
            assert_eq!(format_number(0.256, Some("0%")), "26%");
            assert_eq!(format_number(1500.0, Some("¥#,##0")), "¥1,500");
            assert_eq!(
                format_number(-5.0, Some("$#,##0.00;($#,##0.00)")),
                "($5.00)"
            );
            assert_eq!(format_number(2.5, Some("0.0#")), "2.5");
        }

        #[test]
        fn cell_numbers_read_as_word_reads_them() {
            assert_eq!(first_number("¥1,200"), Some(1200.0));
            assert_eq!(first_number("(5)"), Some(-5.0));
            assert_eq!(first_number("約 ３５ 個"), Some(35.0));
            assert_eq!(first_number("50%"), Some(0.5));
            assert_eq!(first_number("なし"), None);
        }
    }
}

/// A table style Word ships, written the way Word writes it into
/// `styles.xml` when it is first applied.
struct BuiltinTableStyle {
    id: &'static str,
    name: &'static str,
    priority: u32,
    /// The border colour of the whole table, if any.
    border: Option<&'static str>,
    /// Header row: fill, text colour.
    header: Option<(&'static str, &'static str)>,
    /// Band fill for odd rows and columns.
    band: Option<&'static str>,
    /// Whether the first and last columns are bold.
    bold_columns: bool,
    /// Whether the inside lines are drawn.
    inside_lines: bool,
}

/// Word's own style names, ids and colours (Office theme accents).
pub const BUILTIN_TABLE_STYLES: &[(&str, &str)] = &[
    ("TableGrid", "Table Grid"),
    ("PlainTable1", "Plain Table 1"),
    ("PlainTable3", "Plain Table 3"),
    ("GridTable1Light", "Grid Table 1 Light"),
    ("GridTable4", "Grid Table 4"),
    ("GridTable4-Accent1", "Grid Table 4 - Accent 1"),
    ("GridTable4-Accent2", "Grid Table 4 - Accent 2"),
    ("GridTable4-Accent6", "Grid Table 4 - Accent 6"),
    ("GridTable5Dark-Accent1", "Grid Table 5 Dark - Accent 1"),
    ("ListTable3-Accent1", "List Table 3 - Accent 1"),
    ("ListTable4-Accent1", "List Table 4 - Accent 1"),
    ("ListTable4-Accent2", "List Table 4 - Accent 2"),
];

fn builtin_table_style(requested: &str) -> Option<BuiltinTableStyle> {
    let key: String = requested
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    let (id, name) = BUILTIN_TABLE_STYLES.iter().copied().find(|(id, name)| {
        let normalize = |value: &str| -> String {
            value
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect()
        };
        normalize(id) == key || normalize(name) == key
    })?;
    let style = |priority, border, header, band, bold_columns, inside_lines| BuiltinTableStyle {
        id,
        name,
        priority,
        border,
        header,
        band,
        bold_columns,
        inside_lines,
    };
    Some(match id {
        "TableGrid" => style(39, Some("auto"), None, None, false, true),
        "PlainTable1" => style(41, Some("BFBFBF"), None, Some("F2F2F2"), true, true),
        "PlainTable3" => style(43, None, None, Some("F2F2F2"), true, false),
        "GridTable1Light" => style(46, Some("999999"), None, None, true, true),
        "GridTable4" => style(
            49,
            Some("666666"),
            Some(("000000", "FFFFFF")),
            Some("CCCCCC"),
            true,
            true,
        ),
        "GridTable4-Accent1" => style(
            49,
            Some("8EAADB"),
            Some(("4472C4", "FFFFFF")),
            Some("D9E2F3"),
            true,
            true,
        ),
        "GridTable4-Accent2" => style(
            49,
            Some("F4B083"),
            Some(("ED7D31", "FFFFFF")),
            Some("FBE4D5"),
            true,
            true,
        ),
        "GridTable4-Accent6" => style(
            49,
            Some("A8D08D"),
            Some(("70AD47", "FFFFFF")),
            Some("E2EFD9"),
            true,
            true,
        ),
        "GridTable5Dark-Accent1" => style(
            50,
            Some("FFFFFF"),
            Some(("4472C4", "FFFFFF")),
            Some("B4C6E7"),
            true,
            true,
        ),
        "ListTable3-Accent1" => style(
            48,
            Some("4472C4"),
            Some(("4472C4", "FFFFFF")),
            None,
            true,
            false,
        ),
        "ListTable4-Accent1" => style(
            49,
            Some("8EAADB"),
            Some(("4472C4", "FFFFFF")),
            Some("D9E2F3"),
            true,
            false,
        ),
        "ListTable4-Accent2" => style(
            49,
            Some("F4B083"),
            Some(("ED7D31", "FFFFFF")),
            Some("FBE4D5"),
            true,
            false,
        ),
        _ => return None,
    })
}

impl BuiltinTableStyle {
    /// The style's `w:pPr`, `w:tblPr` and `w:tblStylePr` elements.
    fn body(&self) -> String {
        let edge = |name: &str, color: &str, size: u32, value: &str| {
            format!(
                "<w:{name} w:val=\"{value}\" w:sz=\"{size}\" w:space=\"0\" w:color=\"{color}\"/>"
            )
        };
        let mut out = String::from(
            "<w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr>",
        );
        out.push_str("<w:tblPr>");
        if self.band.is_some() {
            out.push_str(
                "<w:tblStyleRowBandSize w:val=\"1\"/><w:tblStyleColBandSize w:val=\"1\"/>",
            );
        }
        if let Some(color) = self.border {
            out.push_str("<w:tblBorders>");
            for side in ["top", "left", "bottom", "right"] {
                out.push_str(&edge(side, color, 4, "single"));
            }
            if self.inside_lines {
                out.push_str(&edge("insideH", color, 4, "single"));
                out.push_str(&edge("insideV", color, 4, "single"));
            }
            out.push_str("</w:tblBorders>");
        }
        out.push_str(
            "<w:tblCellMar><w:top w:w=\"0\" w:type=\"dxa\"/><w:left w:w=\"108\" w:type=\"dxa\"/>\
             <w:bottom w:w=\"0\" w:type=\"dxa\"/><w:right w:w=\"108\" w:type=\"dxa\"/></w:tblCellMar>",
        );
        out.push_str("</w:tblPr>");
        if self.id == "TableGrid" {
            return out;
        }
        let rule = self.border.unwrap_or("7F7F7F");
        // 見出し行: bold, on the accent where the style has one.
        out.push_str("<w:tblStylePr w:type=\"firstRow\"><w:rPr><w:b/><w:bCs/>");
        if let Some((_, text)) = self.header {
            out.push_str(&format!("<w:color w:val=\"{text}\"/>"));
        }
        out.push_str("</w:rPr><w:tblPr/><w:tcPr>");
        match self.header {
            Some((fill, _)) => {
                out.push_str("<w:tcBorders>");
                for side in ["top", "left", "bottom", "right"] {
                    out.push_str(&edge(side, fill, 4, "single"));
                }
                out.push_str("<w:insideH w:val=\"nil\"/><w:insideV w:val=\"nil\"/></w:tcBorders>");
                out.push_str(&format!(
                    "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{fill}\"/>"
                ));
            }
            None => {
                out.push_str("<w:tcBorders>");
                out.push_str(&edge("bottom", rule, 12, "single"));
                out.push_str("</w:tcBorders>");
            }
        }
        out.push_str("</w:tcPr></w:tblStylePr>");
        // 集計行: bold over a double rule.
        out.push_str(&format!(
            "<w:tblStylePr w:type=\"lastRow\"><w:rPr><w:b/><w:bCs/></w:rPr><w:tblPr/><w:tcPr>\
             <w:tcBorders>{}</w:tcBorders></w:tcPr></w:tblStylePr>",
            edge(
                "top",
                self.header.map_or(rule, |(fill, _)| fill),
                4,
                "double"
            )
        ));
        if self.bold_columns {
            for kind in ["firstCol", "lastCol"] {
                out.push_str(&format!(
                    "<w:tblStylePr w:type=\"{kind}\"><w:rPr><w:b/><w:bCs/></w:rPr></w:tblStylePr>"
                ));
            }
        }
        if let Some(band) = self.band {
            for kind in ["band1Vert", "band1Horz"] {
                out.push_str(&format!(
                    "<w:tblStylePr w:type=\"{kind}\"><w:tblPr/><w:tcPr>\
                     <w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{band}\"/></w:tcPr></w:tblStylePr>"
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::headless_api::{
        apply_headless_commands_to_document, apply_headless_commands_with_executor_to_document,
    };
    use futures::executor;
    use jade_docx_headless_api::{
        HeadlessCommand, HeadlessEditRequest, HeadlessTableSortKey, HeadlessTableSortKind,
    };
    use std::io::{Read, Write};
    use zip::write::SimpleFileOptions;

    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

    fn package(body: &str, styles: Option<&str>) -> Vec<u8> {
        let document = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:document xmlns:w=\"{W}\"><w:body>{body}<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\"/></w:sectPr></w:body></w:document>"
        );
        let mut types = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>",
        );
        if styles.is_some() {
            types.push_str("<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>");
        }
        types.push_str("</Types>");
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options = SimpleFileOptions::default();
            let mut parts = vec![
                ("[Content_Types].xml".to_string(), types),
                ("_rels/.rels".to_string(), "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>".to_string()),
                ("word/document.xml".to_string(), document),
            ];
            if let Some(styles) = styles {
                parts.push(("word/_rels/document.xml.rels".to_string(), "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId9\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/></Relationships>".to_string()));
                parts.push((
                    "word/styles.xml".to_string(),
                    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:styles xmlns:w=\"{W}\">{styles}</w:styles>"),
                ));
            }
            for (path, value) in parts {
                zip.start_file(path, options).unwrap();
                zip.write_all(value.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        cursor.into_inner()
    }

    fn load(bytes: &[u8]) -> WordDocument {
        executor::block_on(WordDocument::load(bytes, &crate::Options::default())).expect("load")
    }

    fn part(bytes: &[u8], name: &str) -> String {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut text = String::new();
        archive
            .by_name(name)
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        text
    }

    fn apply(document: &mut WordDocument, command: HeadlessCommand) {
        apply_headless_commands_to_document(
            document,
            &HeadlessEditRequest {
                commands: vec![command],
            },
        )
        .expect("command applies");
    }

    fn texts(document: &WordDocument) -> Vec<String> {
        (0..document.paragraph_count())
            .map(|index| {
                document
                    .paragraph(index)
                    .map(paragraph_plain_text)
                    .unwrap_or_default()
            })
            .collect()
    }

    fn cell(text: &str) -> String {
        format!("<w:tc><w:tcPr><w:tcW w:w=\"2000\" w:type=\"dxa\"/></w:tcPr><w:p><w:r><w:t xml:space=\"preserve\">{text}</w:t></w:r></w:p></w:tc>")
    }

    fn table(rows: &[&[&str]]) -> String {
        let columns = rows.iter().map(|row| row.len()).max().unwrap_or(1);
        let grid: String = (0..columns).map(|_| "<w:gridCol w:w=\"2000\"/>").collect();
        let body: String = rows
            .iter()
            .map(|row| {
                format!(
                    "<w:tr>{}</w:tr>",
                    row.iter().map(|text| cell(text)).collect::<String>()
                )
            })
            .collect();
        format!("<w:tbl><w:tblPr><w:tblW w:w=\"0\" w:type=\"auto\"/></w:tblPr><w:tblGrid>{grid}</w:tblGrid>{body}</w:tbl>")
    }

    fn cell_texts(document: &WordDocument, table_order: usize) -> Vec<Vec<String>> {
        document
            .table_by_order(table_order)
            .expect("table")
            .rows
            .iter()
            .map(|row| row.cells.iter().map(cell_plain_text).collect())
            .collect()
    }

    const NORMAL_TABLE: &str = "<w:style w:type=\"table\" w:default=\"1\" w:styleId=\"TableNormal\"><w:name w:val=\"Normal Table\"/><w:tblPr><w:tblInd w:w=\"0\" w:type=\"dxa\"/><w:tblCellMar><w:top w:w=\"0\" w:type=\"dxa\"/><w:left w:w=\"108\" w:type=\"dxa\"/><w:bottom w:w=\"0\" w:type=\"dxa\"/><w:right w:w=\"108\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr></w:style>";

    #[test]
    fn style_options_write_every_switch_and_read_back() {
        let bytes = package(
            &format!("{}<w:p/>", table(&[&["a", "b"], &["c", "d"]])),
            Some(NORMAL_TABLE),
        );
        let mut document = load(&bytes);
        apply(
            &mut document,
            HeadlessCommand::SetTableLook {
                table_order_index: 0,
                first_row: Some(false),
                banded_rows: Some(true),
                last_row: Some(true),
                first_column: Some(false),
                last_column: Some(true),
                banded_columns: Some(true),
            },
        );
        let exported = document.export_docx().unwrap();
        let xml = part(&exported, "word/document.xml");
        for expected in [
            "w:firstRow=\"0\"",
            "w:lastRow=\"1\"",
            "w:firstColumn=\"0\"",
            "w:lastColumn=\"1\"",
            "w:noHBand=\"0\"",
            "w:noVBand=\"0\"",
        ] {
            assert!(xml.contains(expected), "{expected} in {xml}");
        }
        let look = load(&exported).table_by_order(0).unwrap().look;
        assert!(!look.first_row_enabled() && look.last_row_enabled());
        assert!(!look.first_column_enabled() && look.last_column_enabled());
        assert!(look.horizontal_banding_enabled() && look.vertical_banding_enabled());
    }

    #[test]
    fn a_built_in_style_is_added_on_first_use_and_drawn_from() {
        let bytes = package(
            &format!("{}<w:p/>", table(&[&["a", "b"], &["c", "d"]])),
            Some(NORMAL_TABLE),
        );
        let mut document = load(&bytes);
        for _ in 0..2 {
            apply(
                &mut document,
                HeadlessCommand::ApplyTableNamedStyle {
                    table_order_index: 0,
                    style_id: Some("Grid Table 4 - Accent 1".to_string()),
                },
            );
        }
        let table = document.table_by_order(0).unwrap();
        assert_eq!(table.style_name.as_deref(), Some("GridTable4-Accent1"));
        let header = table.table_style_cell_format(crate::document::TableStyleCellPosition {
            row: 0,
            row_count: 2,
            cell: 0,
            cell_count: 2,
        });
        assert_eq!(
            header
                .fill
                .as_deref()
                .map(|fill| fill.trim_start_matches('#').to_ascii_uppercase())
                .as_deref(),
            Some("4472C4")
        );
        let exported = document.export_docx().unwrap();
        let styles = part(&exported, "word/styles.xml");
        assert_eq!(
            styles.matches("w:styleId=\"GridTable4-Accent1\"").count(),
            1,
            "{styles}"
        );
        assert!(styles.contains("<w:basedOn w:val=\"TableNormal\"/>"));
        assert!(part(&exported, "word/document.xml")
            .contains("<w:tblStyle w:val=\"GridTable4-Accent1\"/>"));
        let reloaded = load(&exported);
        assert!(reloaded.table_by_order(0).unwrap().resolved_style.is_some());
        // Table Grid by its id too.
        apply(
            &mut document,
            HeadlessCommand::ApplyTableNamedStyle {
                table_order_index: 0,
                style_id: Some("TableGrid".to_string()),
            },
        );
        assert!(document
            .table_by_order(0)
            .unwrap()
            .resolved_style
            .as_ref()
            .unwrap()
            .borders
            .is_some());
    }

    #[test]
    fn an_inserted_table_takes_table_grid_as_words_insert_does() {
        use crate::commands::insert_table::InsertTableCommand;
        let mut document = load(&package(
            "<w:p><w:r><w:t>本文</w:t></w:r></w:p>",
            Some(NORMAL_TABLE),
        ));
        let before = document.export_docx().unwrap();
        let mut executor = CommandExecutor::new();
        let mut selection = None;
        executor
            .execute(
                Box::new(
                    InsertTableCommand::at_caret(0, 6, 2, 2, Vec::new()).with_style("Table Grid"),
                ),
                &mut CommandContext::new(&mut document, &mut selection),
            )
            .expect("insert");
        let table = document.table_by_order(0).unwrap();
        assert_eq!(table.style_name.as_deref(), Some("TableGrid"));
        assert!(table.resolved_style.as_ref().unwrap().borders.is_some());
        let exported = document.export_docx().unwrap();
        assert!(part(&exported, "word/document.xml").contains("<w:tblStyle w:val=\"TableGrid\"/>"));
        executor
            .undo(&mut CommandContext::new(&mut document, &mut selection))
            .expect("undo");
        assert_eq!(
            part(&document.export_docx().unwrap(), "word/styles.xml"),
            part(&before, "word/styles.xml")
        );
        assert!(document.table_by_order(0).is_none());
    }

    #[test]
    fn spacing_direction_indent_and_position_round_trip() {
        let bytes = package(
            &format!(
                "<w:p><w:r><w:t>before</w:t></w:r></w:p>{}<w:p/>",
                table(&[&["縦書き", "b"], &["c", "d"]])
            ),
            None,
        );
        let mut document = load(&bytes);
        for command in [
            HeadlessCommand::SetTableCellSpacing {
                table_order_index: 0,
                spacing: Some(2.0),
            },
            HeadlessCommand::SetTableCellTextDirection {
                table_order_index: 0,
                row: 0,
                column: 0,
                direction: Some("tbRl".to_string()),
            },
            HeadlessCommand::SetTableIndent {
                table_order_index: 0,
                indent: 18.0,
            },
            HeadlessCommand::SetTablePosition {
                table_order_index: 0,
                horizontal_align: Some("center".to_string()),
                horizontal_offset: None,
                horizontal_anchor: "margin".to_string(),
                vertical_align: None,
                vertical_offset: Some(36.0),
                vertical_anchor: "text".to_string(),
                distance_top: 0.0,
                distance_bottom: 0.0,
                distance_left: 9.0,
                distance_right: 9.0,
                allow_overlap: false,
            },
        ] {
            apply(&mut document, command);
        }
        let exported = document.export_docx().unwrap();
        let xml = part(&exported, "word/document.xml");
        assert!(
            xml.contains("<w:tblCellSpacing w:w=\"40\" w:type=\"dxa\"/>"),
            "{xml}"
        );
        assert!(xml.contains("<w:textDirection w:val=\"tbRl\"/>"), "{xml}");
        assert!(
            xml.contains("<w:tblInd w:w=\"360\" w:type=\"dxa\"/>"),
            "{xml}"
        );
        assert!(
            xml.contains("w:tblpXSpec=\"center\"") && xml.contains("w:tblpY=\"720\""),
            "{xml}"
        );
        assert!(xml.contains("<w:tblOverlap w:val=\"never\"/>"), "{xml}");
        let reloaded = load(&exported);
        let table = reloaded.table_by_order(0).unwrap();
        assert_eq!(table.cell_spacing, Some(2.0));
        assert_eq!(
            table.rows[0].cells[0].text_direction,
            Some(TextDirection::TbRl)
        );
        assert_eq!(
            table.style.get("margin-left").map(String::as_str),
            Some("18.00pt")
        );
        assert_eq!(
            table.style.get("table-position").map(String::as_str),
            Some("floating")
        );
        assert_eq!(
            table
                .style
                .get("table-position-vertical-anchor")
                .map(String::as_str),
            Some("text")
        );
        // Back to horizontal text and no spacing.
        let mut document = reloaded;
        apply(
            &mut document,
            HeadlessCommand::SetTableCellSpacing {
                table_order_index: 0,
                spacing: None,
            },
        );
        apply(
            &mut document,
            HeadlessCommand::SetTableCellTextDirection {
                table_order_index: 0,
                row: 0,
                column: 0,
                direction: None,
            },
        );
        let xml = part(&document.export_docx().unwrap(), "word/document.xml");
        assert!(
            !xml.contains("tblCellSpacing") && !xml.contains("textDirection"),
            "{xml}"
        );
    }

    #[test]
    fn text_becomes_a_table_keeping_its_formatting_and_undoes() {
        let body = "<w:p><w:r><w:t>title</w:t></w:r></w:p>\
            <w:p><w:r><w:rPr><w:b/></w:rPr><w:t xml:space=\"preserve\">名前</w:t></w:r><w:r><w:tab/><w:t>数量</w:t></w:r></w:p>\
            <w:p><w:r><w:t xml:space=\"preserve\">りんご</w:t></w:r><w:r><w:tab/><w:t>3</w:t></w:r><w:r><w:tab/><w:t>赤</w:t></w:r></w:p>\
            <w:p><w:r><w:t>after</w:t></w:r></w:p>";
        let mut document = load(&package(body, None));
        let before = document.export_docx().unwrap();
        let mut executor = CommandExecutor::new();
        apply_headless_commands_with_executor_to_document(
            &mut document,
            &mut executor,
            None,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::ConvertTextToTable {
                    start_paragraph: 1,
                    end_paragraph: 2,
                    separator: "tab".to_string(),
                    columns: None,
                }],
            },
        )
        .expect("converts");
        assert_eq!(
            cell_texts(&document, 0),
            vec![vec!["名前", "数量", ""], vec!["りんご", "3", "赤"]]
        );
        assert_eq!(texts(&document).first().map(String::as_str), Some("title"));
        assert_eq!(texts(&document).last().map(String::as_str), Some("after"));
        let exported = document.export_docx().unwrap();
        let xml = part(&exported, "word/document.xml");
        assert!(xml.contains("<w:b/>"), "bold kept: {xml}");
        assert!(!xml.contains("<w:tab/>"), "separators gone: {xml}");
        let reloaded = load(&exported);
        assert_eq!(cell_texts(&reloaded, 0)[1], vec!["りんご", "3", "赤"]);

        let mut selection = None;
        executor
            .undo(&mut CommandContext::new(&mut document, &mut selection))
            .expect("undo");
        assert_eq!(
            part(&document.export_docx().unwrap(), "word/document.xml"),
            part(&before, "word/document.xml")
        );
    }

    #[test]
    fn the_last_paragraphs_become_a_table_with_a_paragraph_after_it() {
        let body = "<w:p><w:r><w:t>a,b</w:t></w:r></w:p><w:p><w:r><w:t>c,d</w:t></w:r></w:p>";
        let mut document = load(&package(body, None));
        apply(
            &mut document,
            HeadlessCommand::ConvertTextToTable {
                start_paragraph: 0,
                end_paragraph: 1,
                separator: "comma".to_string(),
                columns: None,
            },
        );
        assert_eq!(
            cell_texts(&document, 0),
            vec![vec!["a", "b"], vec!["c", "d"]]
        );
        let reloaded = load(&document.export_docx().unwrap());
        assert_eq!(
            cell_texts(&reloaded, 0),
            vec![vec!["a", "b"], vec!["c", "d"]]
        );
        // One cell per paragraph, two to a row.
        let body = "<w:p><w:r><w:t>1</w:t></w:r></w:p><w:p><w:r><w:t>2</w:t></w:r></w:p><w:p><w:r><w:t>3</w:t></w:r></w:p><w:p/>";
        let mut document = load(&package(body, None));
        apply(
            &mut document,
            HeadlessCommand::ConvertTextToTable {
                start_paragraph: 0,
                end_paragraph: 2,
                separator: "paragraph".to_string(),
                columns: Some(2),
            },
        );
        assert_eq!(
            cell_texts(&document, 0),
            vec![vec!["1", "2"], vec!["3", ""]]
        );
    }

    #[test]
    fn a_table_becomes_text_one_paragraph_per_row() {
        let body = format!(
            "<w:p><w:r><w:t>title</w:t></w:r></w:p>{}<w:p><w:r><w:t>after</w:t></w:r></w:p>",
            table(&[&["a", "b", "c"], &["d", "", "f"]])
        );
        let mut document = load(&package(&body, None));
        apply(
            &mut document,
            HeadlessCommand::ConvertTableToText {
                table_order_index: 0,
                separator: "tab".to_string(),
            },
        );
        assert_eq!(
            texts(&document),
            vec!["title", "a\tb\tc", "d\t\tf", "after"]
        );
        assert!(document.table_by_order(0).is_none());
        let reloaded = load(&document.export_docx().unwrap());
        assert_eq!(
            texts(&reloaded),
            vec!["title", "a\tb\tc", "d\t\tf", "after"]
        );

        let mut document = load(&package(&body, None));
        apply(
            &mut document,
            HeadlessCommand::ConvertTableToText {
                table_order_index: 0,
                separator: "paragraph".to_string(),
            },
        );
        assert_eq!(
            texts(&document),
            vec!["title", "a", "b", "c", "d", "", "f", "after"]
        );
    }

    #[test]
    fn rows_sort_by_their_keys_below_the_header() {
        let body = format!(
            "{}<w:p/>",
            table(&[
                &["名前", "点"],
                &["b", "10"],
                &["a", "9"],
                &["c", "10"],
                &["d", ""]
            ])
        );
        let mut document = load(&package(&body, None));
        apply(
            &mut document,
            HeadlessCommand::SortTable {
                table_order_index: 0,
                keys: vec![
                    HeadlessTableSortKey {
                        column: 1,
                        kind: HeadlessTableSortKind::Number,
                        descending: true,
                    },
                    HeadlessTableSortKey {
                        column: 0,
                        kind: HeadlessTableSortKind::Text,
                        descending: false,
                    },
                ],
                header_rows: 1,
            },
        );
        let expected = vec![
            vec!["名前", "点"],
            vec!["b", "10"],
            vec!["c", "10"],
            vec!["a", "9"],
            vec!["d", ""],
        ];
        assert_eq!(cell_texts(&document, 0), expected);
        assert_eq!(
            cell_texts(&load(&document.export_docx().unwrap()), 0),
            expected
        );
        // Text ascending, dates.
        let body = format!(
            "{}<w:p/>",
            table(&[&["2024/3/1"], &["2023年12月5日"], &["2024-01-31"]])
        );
        let mut document = load(&package(&body, None));
        apply(
            &mut document,
            HeadlessCommand::SortTable {
                table_order_index: 0,
                keys: vec![HeadlessTableSortKey {
                    column: 0,
                    kind: HeadlessTableSortKind::Date,
                    descending: false,
                }],
                header_rows: 0,
            },
        );
        assert_eq!(
            cell_texts(&document, 0),
            vec![vec!["2023年12月5日"], vec!["2024-01-31"], vec!["2024/3/1"]]
        );
    }

    #[test]
    fn a_formula_is_computed_saved_and_updated() {
        let body = format!(
            "{}<w:p/>",
            table(&[
                &["品目", "数量"],
                &["りんご", "3"],
                &["みかん", "5"],
                &["合計", ""]
            ])
        );
        let mut document = load(&package(&body, None));
        // The paragraph of the last row's second cell.
        let paragraph = (0..document.paragraph_count())
            .find(|index| {
                document
                    .table_cell_of_surface(*index)
                    .is_some_and(|(_, row, column)| (row, column) == (3, 1))
            })
            .expect("total cell");
        apply(
            &mut document,
            HeadlessCommand::InsertTableFormula {
                paragraph,
                offset: 0,
                formula: "=SUM(ABOVE)".to_string(),
                number_format: Some("#,##0".to_string()),
            },
        );
        assert_eq!(cell_texts(&document, 0)[3][1], "8");
        let exported = document.export_docx().unwrap();
        let xml = part(&exported, "word/document.xml");
        assert!(
            xml.contains("=SUM(ABOVE) \\# &quot;#,##0&quot;")
                || xml.contains("=SUM(ABOVE) \\# \"#,##0\""),
            "{xml}"
        );

        let mut document = load(&exported);
        assert_eq!(cell_texts(&document, 0)[3][1], "8");
        apply(
            &mut document,
            HeadlessCommand::SetTableCellText {
                table_order_index: 0,
                row: 1,
                column: 1,
                text: "1200".to_string(),
            },
        );
        apply(&mut document, HeadlessCommand::UpdateTableFormulas);
        assert_eq!(cell_texts(&document, 0)[3][1], "1,205");
        let exported = document.export_docx().unwrap();
        assert!(part(&exported, "word/document.xml").contains(">1,205<"));
        let reloaded = load(&exported);
        assert_eq!(cell_texts(&reloaded, 0)[3][1], "1,205");
        // A formula outside a table is refused.
        let mut plain = load(&package("<w:p><w:r><w:t>x</w:t></w:r></w:p>", None));
        assert!(apply_headless_commands_to_document(
            &mut plain,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::InsertTableFormula {
                    paragraph: 0,
                    offset: 0,
                    formula: "SUM(ABOVE)".to_string(),
                    number_format: None,
                }],
            },
        )
        .is_err());
    }
}
