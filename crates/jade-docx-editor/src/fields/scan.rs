//! Finding the fields of one story (`document.xml`, a header, the notes) in
//! document order, with what evaluating them needs: their instructions —
//! nested fields included as pieces — their current results, the paragraphs
//! around them, the bookmarks, the tables and the sections.

use crate::docx_xml::{XmlElement, XmlNode, XmlNodeId};
use std::collections::HashMap;

/// Where a `w:fldChar` sits: its run and its index among the run's children.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Marker {
    pub run: XmlNodeId,
    pub child: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Piece {
    Text(String),
    /// A field nested in the instruction, by index.
    Field(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    /// Nested in the parent's instruction: its result is an operand.
    Operand,
    /// Inside the parent's result (a PAGEREF in a TOC).
    Result,
}

/// A table cell a paragraph sits in: table, row and grid column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CellRef {
    pub table: usize,
    pub row: usize,
    pub column: usize,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ScannedField {
    pub begin: Option<Marker>,
    pub separate: Option<Marker>,
    pub end: Option<Marker>,
    /// The `w:fldSimple` element of a simple field.
    pub simple: Option<XmlNodeId>,
    /// The instruction of a simple field (`w:instr`).
    pub simple_instruction: Option<String>,
    pub pieces: Vec<Piece>,
    /// The visible text of the current result.
    pub result_text: String,
    /// The runs directly in the result, in order (not those of fields nested
    /// in it).
    pub result_runs: Vec<XmlNodeId>,
    /// The runs holding instruction text.
    pub instruction_runs: Vec<XmlNodeId>,
    /// Every run from the begin marker to the end marker, nested ones too.
    pub span_runs: Vec<XmlNodeId>,
    pub parent: Option<(usize, Role)>,
    pub locked: bool,
    /// The paragraph (index into [`Story::paragraphs`]) the field begins in.
    pub paragraph: usize,
    /// The paragraph it ends in.
    pub end_paragraph: usize,
    pub cell: Option<CellRef>,
    /// Whether the field still reads its instruction.
    pub(crate) in_instruction: bool,
}

impl ScannedField {
    /// The instruction with nested fields given by `nested`.
    pub fn instruction_with(&self, mut nested: impl FnMut(usize) -> String) -> String {
        if let Some(instruction) = &self.simple_instruction {
            return instruction.clone();
        }
        let mut out = String::new();
        for piece in &self.pieces {
            match piece {
                Piece::Text(text) => out.push_str(text),
                Piece::Field(index) => {
                    let value = nested(*index);
                    // A nested result is one operand: quote it unless it
                    // already sits in quotes or is one bare word.
                    let inside_quotes = out.chars().filter(|&c| c == '"').count() % 2 == 1;
                    if inside_quotes
                        || (!value.is_empty()
                            && !value.chars().any(char::is_whitespace)
                            && !value.contains('"'))
                    {
                        out.push_str(&value);
                    } else {
                        out.push('"');
                        out.push_str(&value.replace('"', "\\\""));
                        out.push('"');
                    }
                }
            }
        }
        out
    }

    /// The instruction as Word shows it with field codes on: nested fields
    /// in braces.
    pub fn display_code(&self, fields: &[ScannedField]) -> String {
        if let Some(instruction) = &self.simple_instruction {
            return instruction.clone();
        }
        let mut out = String::new();
        for piece in &self.pieces {
            match piece {
                Piece::Text(text) => out.push_str(text),
                Piece::Field(index) => {
                    out.push_str("{ ");
                    out.push_str(fields[*index].display_code(fields).trim());
                    out.push_str(" }");
                }
            }
        }
        out
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ScannedParagraph {
    pub id: XmlNodeId,
    pub style_id: Option<String>,
    pub outline_level: Option<u8>,
    pub text: String,
    pub section: usize,
    pub cell: Option<CellRef>,
    /// `w:numPr`: the list and level.
    pub numbering: Option<(String, u32)>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ScannedBookmark {
    pub text: String,
    pub paragraph: usize,
    /// The first footnote (`true`) or endnote reference inside it.
    pub note: Option<(bool, String)>,
    pub(crate) open: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ScannedTable {
    /// Rows of cells: grid column, span and text.
    pub rows: Vec<Vec<ScannedCell>>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ScannedCell {
    pub column: usize,
    pub span: usize,
    pub text: String,
}

/// A section's `w:pgNumType`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SectionNumbering {
    pub format: Option<String>,
    pub start: Option<u32>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Story {
    pub paragraphs: Vec<ScannedParagraph>,
    pub fields: Vec<ScannedField>,
    /// By lowercased name.
    pub bookmarks: HashMap<String, ScannedBookmark>,
    pub tables: Vec<ScannedTable>,
    pub sections: Vec<SectionNumbering>,
    /// Footnote and endnote references in order: (is footnote, id).
    pub note_references: Vec<(bool, String)>,
}

impl Story {
    /// Whether field `index` sits inside field `ancestor`'s result.
    pub fn in_result_of(&self, index: usize, ancestor: usize) -> bool {
        let mut current = index;
        while let Some((parent, role)) = self.fields[current].parent {
            if parent == ancestor {
                return role == Role::Result;
            }
            current = parent;
        }
        false
    }
}

struct TableBuilder {
    index: usize,
    row: usize,
    column: usize,
    cell: Option<usize>,
}

#[derive(Default)]
struct Scanner {
    story: Story,
    open_fields: Vec<usize>,
    /// Bookmarks open now, by `w:id`, with their names.
    open_bookmarks: HashMap<String, String>,
    tables: Vec<TableBuilder>,
    paragraph: Option<usize>,
    section: usize,
}

/// Scans a story's root element.
pub(crate) fn scan_story(root: &XmlElement) -> Story {
    let mut scanner = Scanner::default();
    scanner.story.sections.push(SectionNumbering::default());
    scanner.walk(root);
    // A field left open by a malformed story ends with it.
    scanner.story
}

fn attribute<'a>(element: &'a XmlElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .map(|attribute| attribute.value.as_str())
}

pub(crate) fn child_element<'a>(element: &'a XmlElement, name: &str) -> Option<&'a XmlElement> {
    element.children.iter().find_map(|child| match child {
        XmlNode::Element(child) if child.name == name => Some(child),
        _ => None,
    })
}

fn element_text(element: &XmlElement) -> String {
    element
        .children
        .iter()
        .map(|child| match child {
            XmlNode::Text(text) | XmlNode::CData(text) => text.as_str(),
            _ => "",
        })
        .collect()
}

pub(crate) fn on_off(element: &XmlElement, name: &str) -> bool {
    attribute(element, name).is_some_and(|value| matches!(value, "1" | "true" | "on"))
}

fn section_numbering(sect_pr: &XmlElement) -> SectionNumbering {
    child_element(sect_pr, "w:pgNumType")
        .map(|numbering| SectionNumbering {
            format: attribute(numbering, "w:fmt").map(str::to_string),
            start: attribute(numbering, "w:start").and_then(|value| value.parse().ok()),
        })
        .unwrap_or_default()
}

impl Scanner {
    fn walk(&mut self, element: &XmlElement) {
        for child in &element.children {
            if let XmlNode::Element(child) = child {
                self.element(child);
            }
        }
    }

    fn element(&mut self, element: &XmlElement) {
        match element.name.as_str() {
            "w:p" => self.paragraph(element),
            "w:tbl" => {
                let index = self.story.tables.len();
                self.story.tables.push(ScannedTable::default());
                self.tables.push(TableBuilder {
                    index,
                    row: 0,
                    column: 0,
                    cell: None,
                });
                self.walk(element);
                self.tables.pop();
            }
            "w:tr" => {
                if let Some(table) = self.tables.last_mut() {
                    self.story.tables[table.index].rows.push(Vec::new());
                    table.row = self.story.tables[table.index].rows.len() - 1;
                    table.column = 0;
                }
                self.walk(element);
            }
            "w:tc" => {
                let span = child_element(element, "w:tcPr")
                    .and_then(|pr| child_element(pr, "w:gridSpan"))
                    .and_then(|span| attribute(span, "w:val"))
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(1)
                    .max(1);
                if let Some(table) = self.tables.last_mut() {
                    let row = &mut self.story.tables[table.index].rows[table.row];
                    row.push(ScannedCell {
                        column: table.column,
                        span,
                        text: String::new(),
                    });
                    table.cell = Some(row.len() - 1);
                }
                self.walk(element);
                if let Some(table) = self.tables.last_mut() {
                    table.column += span;
                    table.cell = None;
                }
            }
            "w:r" => self.run(element),
            "w:fldSimple" => self.simple_field(element),
            "w:bookmarkStart" => {
                let (Some(id), Some(name)) =
                    (attribute(element, "w:id"), attribute(element, "w:name"))
                else {
                    return;
                };
                self.open_bookmarks.insert(id.to_string(), name.to_string());
                self.story.bookmarks.insert(
                    name.to_lowercase(),
                    ScannedBookmark {
                        text: String::new(),
                        paragraph: self.paragraph.unwrap_or(0),
                        note: None,
                        open: true,
                    },
                );
            }
            "w:bookmarkEnd" => {
                if let Some(name) =
                    attribute(element, "w:id").and_then(|id| self.open_bookmarks.remove(id))
                {
                    if let Some(bookmark) = self.story.bookmarks.get_mut(&name.to_lowercase()) {
                        bookmark.open = false;
                    }
                }
            }
            // Deleted content is not in the document Word shows or updates.
            "w:del" | "w:moveFrom" => {}
            "w:pPr" | "w:rPr" | "w:sectPr" | "w:tblPr" | "w:tcPr" | "w:trPr" | "w:tblGrid"
            | "w:tblPrEx" => {}
            // Word reads the choice; the fallback repeats it for old readers.
            "mc:AlternateContent" => {
                if let Some(choice) = child_element(element, "mc:Choice") {
                    self.walk(choice);
                }
            }
            _ => self.walk(element),
        }
    }

    fn paragraph(&mut self, element: &XmlElement) {
        let properties = child_element(element, "w:pPr");
        let style_id = properties
            .and_then(|pr| child_element(pr, "w:pStyle"))
            .and_then(|style| attribute(style, "w:val"))
            .map(str::to_string);
        let outline_level = properties
            .and_then(|pr| child_element(pr, "w:outlineLvl"))
            .and_then(|level| attribute(level, "w:val"))
            .and_then(|value| value.parse().ok());
        let numbering = properties
            .and_then(|pr| child_element(pr, "w:numPr"))
            .and_then(|num| {
                let id = child_element(num, "w:numId").and_then(|id| attribute(id, "w:val"))?;
                let level = child_element(num, "w:ilvl")
                    .and_then(|level| attribute(level, "w:val"))
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0);
                Some((id.to_string(), level))
            });
        let cell = self.tables.last().and_then(|table| {
            table.cell.map(|cell| CellRef {
                table: table.index,
                row: table.row,
                column: self.story.tables[table.index].rows[table.row][cell].column,
            })
        });
        let outer = self.paragraph.take();
        self.paragraph = Some(self.story.paragraphs.len());
        self.story.paragraphs.push(ScannedParagraph {
            id: element.id,
            style_id,
            outline_level,
            text: String::new(),
            section: self.section,
            cell,
            numbering,
        });
        self.walk(element);
        // A paragraph inside a table cell adds a break between paragraphs
        // of the cell's text.
        if let Some(table) = self.tables.last() {
            if let Some(cell) = table.cell {
                let text = &mut self.story.tables[table.index].rows[table.row][cell].text;
                if !text.is_empty() {
                    text.push('\n');
                }
            }
        }
        if let Some(sect_pr) = properties.and_then(|pr| child_element(pr, "w:sectPr")) {
            if let Some(current) = self.story.sections.last_mut() {
                *current = section_numbering(sect_pr);
            }
            self.section += 1;
            self.story.sections.push(SectionNumbering::default());
        }
        self.paragraph = outer;
    }

    /// The final `w:sectPr` (a child of the body) describes the last section.
    pub(crate) fn finish_body(&mut self, body: &XmlElement) {
        if let Some(sect_pr) = child_element(body, "w:sectPr") {
            if let Some(last) = self.story.sections.last_mut() {
                *last = section_numbering(sect_pr);
            }
        }
    }

    fn current_paragraph(&self) -> usize {
        self.paragraph.unwrap_or(0)
    }

    fn open_field(&mut self, field: ScannedField) -> usize {
        let index = self.story.fields.len();
        let parent = self.open_fields.last().map(|&parent| {
            let role = if self.story.fields[parent].in_instruction {
                Role::Operand
            } else {
                Role::Result
            };
            (parent, role)
        });
        if let Some((parent, Role::Operand)) = parent {
            self.story.fields[parent].pieces.push(Piece::Field(index));
        }
        self.story.fields.push(ScannedField {
            parent,
            paragraph: self.current_paragraph(),
            end_paragraph: self.current_paragraph(),
            cell: self.tables.last().and_then(|table| {
                table.cell.map(|cell| CellRef {
                    table: table.index,
                    row: table.row,
                    column: self.story.tables[table.index].rows[table.row][cell].column,
                })
            }),
            ..field
        });
        self.open_fields.push(index);
        index
    }

    fn close_field(&mut self) {
        if let Some(index) = self.open_fields.pop() {
            self.story.fields[index].end_paragraph = self.current_paragraph();
            self.story.fields[index].in_instruction = false;
        }
    }

    /// Visible text: into the paragraph, the open bookmarks, the table cell
    /// and the result of every open field.
    fn visible(&mut self, text: &str) {
        // A result inside an instruction (a nested operand's) is not on the
        // page: it only belongs to the fields opened after that instruction.
        let reading = self
            .open_fields
            .iter()
            .rposition(|&field| self.story.fields[field].in_instruction);
        let from = reading.map_or(0, |at| at + 1);
        for &field in &self.open_fields[from..] {
            self.story.fields[field].result_text.push_str(text);
        }
        if reading.is_some() {
            return;
        }
        if let Some(paragraph) = self.paragraph {
            self.story.paragraphs[paragraph].text.push_str(text);
        }
        for name in self.open_bookmarks.values() {
            if let Some(bookmark) = self.story.bookmarks.get_mut(&name.to_lowercase()) {
                bookmark.text.push_str(text);
            }
        }
        if let Some(table) = self.tables.last() {
            if let Some(cell) = table.cell {
                self.story.tables[table.index].rows[table.row][cell]
                    .text
                    .push_str(text);
            }
        }
    }

    fn instruction(&mut self, text: &str, run: XmlNodeId) {
        let Some(&field) = self.open_fields.last() else {
            return;
        };
        let field = &mut self.story.fields[field];
        if !field.instruction_runs.contains(&run) {
            field.instruction_runs.push(run);
        }
        match field.pieces.last_mut() {
            Some(Piece::Text(existing)) => existing.push_str(text),
            _ => field.pieces.push(Piece::Text(text.to_string())),
        }
    }

    fn reading_instruction(&self) -> bool {
        self.open_fields
            .last()
            .is_some_and(|&field| self.story.fields[field].in_instruction)
    }

    fn run(&mut self, element: &XmlElement) {
        for &field in &self.open_fields {
            self.story.fields[field].span_runs.push(element.id);
        }
        let mut result_run_recorded = false;
        for (child_index, child) in element.children.iter().enumerate() {
            let XmlNode::Element(child) = child else {
                continue;
            };
            match child.name.as_str() {
                "w:fldChar" => match attribute(child, "w:fldCharType") {
                    Some("begin") => {
                        let index = self.open_field(ScannedField {
                            begin: Some(Marker {
                                run: element.id,
                                child: child_index,
                            }),
                            locked: on_off(child, "w:fldLock"),
                            in_instruction: true,
                            ..ScannedField::default()
                        });
                        self.story.fields[index].span_runs.push(element.id);
                    }
                    Some("separate") => {
                        if let Some(&field) = self.open_fields.last() {
                            let field = &mut self.story.fields[field];
                            if field.in_instruction {
                                field.in_instruction = false;
                                field.separate = Some(Marker {
                                    run: element.id,
                                    child: child_index,
                                });
                            }
                        }
                    }
                    Some("end") => {
                        if let Some(&field) = self.open_fields.last() {
                            self.story.fields[field].end = Some(Marker {
                                run: element.id,
                                child: child_index,
                            });
                        }
                        self.close_field();
                    }
                    _ => {}
                },
                "w:instrText" => {
                    if self.reading_instruction() {
                        self.instruction(&element_text(child), element.id);
                    }
                }
                "w:t" => {
                    let text = element_text(child);
                    if self.reading_instruction() {
                        self.instruction(&text, element.id);
                    } else {
                        self.record_result_run(element.id, &mut result_run_recorded);
                        self.visible(&text);
                    }
                }
                "w:tab" | "w:ptab" => {
                    if !self.reading_instruction() {
                        self.record_result_run(element.id, &mut result_run_recorded);
                        self.visible("\t");
                    }
                }
                "w:br" | "w:cr" => {
                    if !self.reading_instruction() {
                        self.record_result_run(element.id, &mut result_run_recorded);
                        self.visible("\n");
                    }
                }
                "w:noBreakHyphen" => {
                    if !self.reading_instruction() {
                        self.record_result_run(element.id, &mut result_run_recorded);
                        self.visible("\u{2011}");
                    }
                }
                "w:sym" => {
                    if !self.reading_instruction() {
                        let character = attribute(child, "w:char")
                            .and_then(|code| u32::from_str_radix(code, 16).ok())
                            .map(|code| {
                                if (0xF000..=0xF0FF).contains(&code) {
                                    code - 0xF000
                                } else {
                                    code
                                }
                            })
                            .and_then(char::from_u32);
                        if let Some(character) = character {
                            self.record_result_run(element.id, &mut result_run_recorded);
                            self.visible(&character.to_string());
                        }
                    }
                }
                "w:footnoteReference" | "w:endnoteReference" => {
                    let footnote = child.name == "w:footnoteReference";
                    if let Some(id) = attribute(child, "w:id") {
                        self.story.note_references.push((footnote, id.to_string()));
                        for name in self.open_bookmarks.values() {
                            if let Some(bookmark) =
                                self.story.bookmarks.get_mut(&name.to_lowercase())
                            {
                                bookmark.note.get_or_insert((footnote, id.to_string()));
                            }
                        }
                    }
                    self.record_result_run(element.id, &mut result_run_recorded);
                }
                "w:drawing" | "w:pict" | "w:object" => {
                    self.record_result_run(element.id, &mut result_run_recorded);
                    // Text boxes carry paragraphs of their own.
                    self.walk(child);
                }
                "mc:AlternateContent" => {
                    self.record_result_run(element.id, &mut result_run_recorded);
                    if let Some(choice) = child_element(child, "mc:Choice") {
                        self.walk(choice);
                    }
                }
                _ => {}
            }
        }
    }

    fn record_result_run(&mut self, run: XmlNodeId, recorded: &mut bool) {
        if *recorded {
            return;
        }
        if let Some(&field) = self.open_fields.last() {
            let field = &mut self.story.fields[field];
            if !field.in_instruction {
                field.result_runs.push(run);
                *recorded = true;
            }
        }
    }

    fn simple_field(&mut self, element: &XmlElement) {
        let instruction = attribute(element, "w:instr")
            .unwrap_or_default()
            .to_string();
        let index = self.open_field(ScannedField {
            simple: Some(element.id),
            simple_instruction: Some(instruction),
            locked: on_off(element, "w:fldLock"),
            in_instruction: false,
            ..ScannedField::default()
        });
        // A simple field nested in an instruction is an operand like a
        // complex one.
        let _ = index;
        self.walk(element);
        self.close_field();
    }
}

/// Scans `document.xml`'s root: the body, with its final section.
pub(crate) fn scan_document(root: &XmlElement) -> Story {
    let mut scanner = Scanner::default();
    scanner.story.sections.push(SectionNumbering::default());
    scanner.walk(root);
    if let Some(body) = child_element(root, "w:body") {
        scanner.finish_body(body);
    }
    scanner.story
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docx_xml::DocxXmlDocument;

    fn scan(body: &str) -> Story {
        let xml = format!(
            "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}</w:body></w:document>"
        );
        let doc = DocxXmlDocument::parse(&xml).unwrap();
        scan_document(doc.root())
    }

    #[test]
    fn finds_nested_fields_with_their_instructions_and_results() {
        let story = scan(concat!(
            "<w:p><w:r><w:t>A </w:t></w:r>",
            "<w:r><w:fldChar w:fldCharType=\"begin\"/></w:r>",
            "<w:r><w:instrText xml:space=\"preserve\"> IF </w:instrText></w:r>",
            "<w:r><w:fldChar w:fldCharType=\"begin\" w:fldLock=\"1\"/></w:r>",
            "<w:r><w:instrText> PAGE </w:instrText></w:r>",
            "<w:r><w:fldChar w:fldCharType=\"separate\"/></w:r>",
            "<w:r><w:t>3</w:t></w:r>",
            "<w:r><w:fldChar w:fldCharType=\"end\"/></w:r>",
            "<w:r><w:instrText xml:space=\"preserve\"> = 1 \"one\" \"other\" </w:instrText></w:r>",
            "<w:r><w:fldChar w:fldCharType=\"separate\"/></w:r>",
            "<w:r><w:t>other</w:t></w:r>",
            "<w:r><w:fldChar w:fldCharType=\"end\"/></w:r>",
            "<w:fldSimple w:instr=\" AUTHOR \"><w:r><w:t>Ann</w:t></w:r></w:fldSimple>",
            "</w:p>"
        ));
        assert_eq!(story.fields.len(), 3);
        let outer = &story.fields[0];
        assert_eq!(outer.pieces.len(), 3);
        assert_eq!(outer.result_text, "other");
        assert_eq!(
            outer.display_code(&story.fields),
            " IF { PAGE } = 1 \"one\" \"other\" "
        );
        assert_eq!(
            outer.instruction_with(|_| "3".to_string()),
            " IF 3 = 1 \"one\" \"other\" "
        );
        assert_eq!(story.fields[1].parent, Some((0, Role::Operand)));
        assert!(story.fields[1].locked);
        assert_eq!(
            story.fields[2].simple_instruction.as_deref(),
            Some(" AUTHOR ")
        );
        assert_eq!(story.fields[2].result_text, "Ann");
        // The nested result is no visible text of its own.
        assert_eq!(story.paragraphs[0].text, "A otherAnn");
    }

    #[test]
    fn records_bookmarks_tables_and_sections() {
        let story = scan(concat!(
            "<w:p><w:bookmarkStart w:id=\"0\" w:name=\"Total\"/><w:r><w:t>42</w:t></w:r><w:bookmarkEnd w:id=\"0\"/>",
            "<w:pPr><w:sectPr><w:pgNumType w:fmt=\"lowerRoman\" w:start=\"3\"/></w:sectPr></w:pPr></w:p>",
            "<w:tbl><w:tr><w:tc><w:p><w:r><w:t>1</w:t></w:r></w:p></w:tc>",
            "<w:tc><w:tcPr><w:gridSpan w:val=\"2\"/></w:tcPr><w:p><w:r><w:t>2</w:t></w:r></w:p></w:tc></w:tr>",
            "<w:tr><w:tc><w:p><w:fldSimple w:instr=\"=SUM(ABOVE)\"><w:r><w:t>0</w:t></w:r></w:fldSimple></w:p></w:tc></w:tr></w:tbl>",
            "<w:sectPr><w:pgNumType w:fmt=\"decimal\"/></w:sectPr>"
        ));
        assert_eq!(story.bookmarks["total"].text, "42");
        assert_eq!(story.sections.len(), 2);
        assert_eq!(story.sections[0].format.as_deref(), Some("lowerRoman"));
        assert_eq!(story.sections[0].start, Some(3));
        assert_eq!(story.sections[1].format.as_deref(), Some("decimal"));
        assert_eq!(story.tables[0].rows[0][1].column, 1);
        assert_eq!(story.tables[0].rows[0][1].span, 2);
        assert_eq!(
            story.fields[0].cell,
            Some(CellRef {
                table: 0,
                row: 1,
                column: 0
            })
        );
        assert_eq!(story.paragraphs[1].section, 1);
    }
}
