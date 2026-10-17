use super::{Command, CommandContext, CommandError, CommandResult};
use crate::word_document::WordDocumentSnapshot;
use jade_docx_layout::selection::{SelectionRange, TextPosition};

/// Inserts a fresh empty table after a top-level paragraph, or appends it as a
/// nested table when the paragraph is inside a table cell. A package snapshot
/// keeps both forms atomic across undo/redo.
///
/// [`InsertTableCommand::at_caret`] places the table the way Word does for a
/// caret position (above the paragraph at its start, splitting it in the
/// middle, after it at the end) and leaves the caret in the first cell.
pub struct InsertTableCommand {
    after_paragraph: usize,
    /// Rendered byte offset of the caret in `after_paragraph`; `None` keeps
    /// the plain "after this paragraph" placement.
    caret_offset: Option<usize>,
    rows: usize,
    columns: usize,
    column_widths: Vec<f32>,
    backup: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
}

impl InsertTableCommand {
    pub fn new(
        after_paragraph: usize,
        rows: usize,
        columns: usize,
        column_widths: Vec<f32>,
    ) -> Self {
        Self {
            after_paragraph,
            caret_offset: None,
            rows,
            columns,
            column_widths,
            backup: None,
            previous_selection: None,
        }
    }

    /// Word's placement for a caret at rendered byte `offset` of `paragraph`.
    pub fn at_caret(
        paragraph: usize,
        offset: usize,
        rows: usize,
        columns: usize,
        column_widths: Vec<f32>,
    ) -> Self {
        Self {
            caret_offset: Some(offset),
            ..Self::new(paragraph, rows, columns, column_widths)
        }
    }
}

impl Command for InsertTableCommand {
    fn name(&self) -> &'static str {
        "insert_table"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous_selection = *ctx.selection;
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or(CommandError::DocumentUnavailable)?;
        let result = (|| {
            if ctx.document.is_note_paragraph_surface(self.after_paragraph) {
                let model = match self.caret_offset {
                    Some(offset) => {
                        ctx.document
                            .rendered_to_model_text_position(TextPosition::new(
                                self.after_paragraph,
                                offset,
                            ))
                    }
                    None => TextPosition::new(
                        self.after_paragraph,
                        ctx.document
                            .paragraph_text_len_at(self.after_paragraph)
                            .unwrap_or(0),
                    ),
                };
                let first = ctx.document.insert_table_at_note_caret(
                    model.paragraph,
                    model.offset,
                    self.rows,
                    self.columns,
                    &self.column_widths,
                )?;
                return Ok(self.caret_offset.map(|_| first));
            }
            let table_order = match self.caret_offset {
                Some(offset) => {
                    // Rendered reference marks widen paragraphs; split at the model offset.
                    let model = ctx
                        .document
                        .rendered_to_model_text_position(TextPosition::new(
                            self.after_paragraph,
                            offset,
                        ));
                    ctx.document.insert_table_at_caret(
                        model.paragraph,
                        model.offset,
                        self.rows,
                        self.columns,
                        &self.column_widths,
                    )?
                }
                None => ctx.document.insert_table_after_paragraph(
                    self.after_paragraph,
                    self.rows,
                    self.columns,
                    &self.column_widths,
                )?,
            };
            Ok::<_, String>(self.caret_offset.and_then(|_| {
                ctx.document
                    .body_index_of_table_order(table_order)
                    .and_then(|index| ctx.document.first_paragraph_index_in_body_element(index))
            }))
        })();
        match result {
            Ok(Some(first_cell)) => {
                *ctx.selection = Some(SelectionRange::collapsed(TextPosition::new(first_cell, 0)));
            }
            Ok(None) => {}
            Err(error) => {
                // Structural edits stay atomic if model or XML validation fails.
                ctx.document.restore_snapshot(snapshot);
                return Err(CommandError::Custom(error));
            }
        }

        self.backup = Some(snapshot);
        self.previous_selection = previous_selection;
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self
            .backup
            .take()
            .ok_or_else(|| CommandError::Custom("insert_table backup is missing".into()))?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::resize_table_row::{visual_height_for_resize, ResizeTableRowCommand};
    use crate::commands::snapshot_edit::SnapshotEditCommand;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::{
        Body, BodyElement, Document, Paragraph, Run, SectionRange, StyleMap, Table, TableCell,
        TableCellMargins, TableRow,
    };
    use crate::word_document::{BorderEdgeSpec, TableBorderSpec, WordDocument};
    use jade_docx_layout::selection::TextPosition;
    use proptest::prelude::*;
    use std::path::PathBuf;

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../public/fixtures")
            .join(name)
    }

    #[test]
    fn table_property_height_modes_round_trip_without_changing_neighbors() {
        use crate::document::TableRowHeightRule;
        let data = std::fs::read(fixture_path("table2x2.docx")).unwrap();
        let mut doc =
            futures::executor::block_on(WordDocument::load(&data, &Default::default())).unwrap();
        let neighbor = doc.table_by_order(0).unwrap().rows[1].clone();
        for (mode, rule) in [
            ("atLeast", TableRowHeightRule::AtLeast),
            ("exact", TableRowHeightRule::Exact),
        ] {
            doc.set_table_row_height_properties(0, 0, mode, Some(31.25))
                .unwrap();
            let bytes = doc.export_docx().unwrap();
            doc = futures::executor::block_on(WordDocument::load(&bytes, &Default::default()))
                .unwrap();
            let table = doc.table_by_order(0).unwrap();
            assert_eq!(table.rows[0].height, Some(31.25));
            assert_eq!(table.rows[0].height_rule, Some(rule));
            assert!(table.rows[0].is_header);
            assert_eq!(table.rows[1].height, neighbor.height);
            assert_eq!(table.rows[1].height_rule, neighbor.height_rule);
        }
        for (mode, height) in [
            ("bad", Some(12.0)),
            ("auto", Some(12.0)),
            ("exact", None),
            ("exact", Some(f32::NAN)),
            ("atLeast", Some(-1.0)),
            ("exact", Some(f32::MAX)),
        ] {
            assert!(doc
                .set_table_row_height_properties(0, 0, mode, height)
                .is_err());
            assert_eq!(doc.table_by_order(0).unwrap().rows[0].height, Some(31.25));
        }
        doc.set_table_row_height_properties(0, 0, "auto", None)
            .unwrap();
        let bytes = doc.export_docx().unwrap();
        let reopened =
            futures::executor::block_on(WordDocument::load(&bytes, &Default::default())).unwrap();
        let row = &reopened.table_by_order(0).unwrap().rows[0];
        assert_eq!(row.height, None);
        assert_eq!(row.height_rule, None);
        assert!(row.is_header);
    }

    #[test]
    fn table_reset_cell_margins_restores_defaults_and_preserves_neighbor() {
        let data = std::fs::read(fixture_path("table2x2.docx")).unwrap();
        let mut doc =
            futures::executor::block_on(WordDocument::load(&data, &Default::default())).unwrap();
        let defaults = doc.table_by_order(0).unwrap().cell_margins.clone();
        let neighbor = doc.table_by_order(0).unwrap().rows[0].cells[1]
            .margins
            .clone();
        assert!(doc.table_by_order(0).unwrap().rows[0].cells[0]
            .margins
            .is_some());
        doc.reset_table_cell_margins(0, 0, 0).unwrap();
        let bytes = doc.export_docx().unwrap();
        let reopened =
            futures::executor::block_on(WordDocument::load(&bytes, &Default::default())).unwrap();
        let table = reopened.table_by_order(0).unwrap();
        assert_eq!(table.rows[0].cells[0].margins, None);
        assert_eq!(table.rows[0].cells[1].margins, neighbor);
        assert_eq!(table.cell_margins, defaults);
        assert!(doc.reset_table_cell_margins(0, 99, 0).is_err());
    }

    fn paragraph_with_text(text: &str) -> BodyElement {
        BodyElement::from_paragraph(Paragraph {
            runs: vec![Run {
                text: text.to_string(),
                properties: None,
                image: None,
                text_box: None,
                math: None,
                field: None,
                hyperlink: None,
                node_id: None,
                text_node_id: None,
                revision_node_id: None,
                comment_reference_ids: Vec::new(),
                footnote_reference_ids: Vec::new(),
                endnote_reference_ids: Vec::new(),
                note_separator: None,
                has_footnote_ref: false,
                has_endnote_ref: false,
                revision: None,
            }],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        })
    }

    fn single_paragraph_document() -> WordDocument {
        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![paragraph_with_text("")],
            }),
            sections: vec![SectionRange {
                start: 0,
                end: 1,
                properties: None,
            }],
            ..Document::default()
        })
    }

    fn body_table_count(doc: &WordDocument) -> usize {
        doc.document
            .as_ref()
            .and_then(|d| d.body.as_ref())
            .map(|body| {
                body.elements
                    .iter()
                    .filter(|e| matches!(e, BodyElement::Table(_)))
                    .count()
            })
            .unwrap_or(0)
    }

    fn total_table_count(doc: &WordDocument) -> usize {
        (0..)
            .take_while(|&index| doc.table_by_order(index).is_some())
            .count()
    }

    fn max_table_depth_in_elements(elements: &[BodyElement], depth: usize) -> usize {
        elements
            .iter()
            .fold(depth, |maximum, element| match element {
                BodyElement::Table(table) => {
                    table.rows.iter().fold(maximum.max(depth + 1), |max, row| {
                        row.cells.iter().fold(max, |max, cell| {
                            max.max(max_table_depth_in_elements(&cell.blocks, depth + 1))
                        })
                    })
                }
                BodyElement::Paragraph(_) | BodyElement::Math(_) => maximum,
            })
    }

    fn max_table_depth(doc: &WordDocument) -> usize {
        doc.document
            .as_ref()
            .and_then(|document| document.body.as_ref())
            .map(|body| max_table_depth_in_elements(&body.elements, 0))
            .unwrap_or(0)
    }

    fn table_with_blocks(blocks: Vec<BodyElement>) -> BodyElement {
        BodyElement::Table(Box::new(Table {
            rows: vec![TableRow {
                cells: vec![TableCell {
                    blocks,
                    style: StyleMap::default(),
                    class_name: None,
                    span: None,
                    vertical_merge: None,
                    text_direction: None,
                    margins: None,
                    vertical_align: None,
                    node_id: None,
                }],
                style: StyleMap::default(),
                class_name: None,
                is_header: false,
                cant_split: false,
                grid_before: None,
                grid_after: None,
                width_before: None,
                width_after: None,
                cell_spacing: None,
                cell_spacing_exception: None,
                cell_margins: None,
                borders: None,
                height: None,
                height_rule: None,
                node_id: None,
            }],
            columns: Vec::new(),
            style: StyleMap::default(),
            cell_style: StyleMap::default(),
            style_name: None,
            class_name: None,
            col_band_size: None,
            row_band_size: None,
            look: Default::default(),
            resolved_style: None,
            borders: None,
            cell_margins: None,
            cell_spacing: None,
            table_layout: None,
            bidi_visual: false,
            node_id: None,
        }))
    }

    #[test]
    fn insert_table_supports_undo_and_redo() {
        let mut word_doc = single_paragraph_document();
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        assert_eq!(body_table_count(&word_doc), 0);

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTableCommand::new(0, 2, 2, Vec::new())),
                    &mut ctx,
                )
                .expect("insert table");
        }

        assert_eq!(body_table_count(&word_doc), 1);
        assert_eq!(word_doc.document.as_ref().unwrap().sections[0].end, 2);
        let table = word_doc.table_by_order(0).expect("inserted table exists");
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0].cells.len(), 2);

        // Undo should remove the table from the body.
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo insert table"));
        }
        assert_eq!(body_table_count(&word_doc), 0);
        assert_eq!(word_doc.document.as_ref().unwrap().sections[0].end, 1);
        assert!(word_doc.table_by_order(0).is_none());
        // The original paragraph must remain intact.
        assert_eq!(word_doc.paragraph_count(), 1);

        // Redo should reinsert the table.
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.redo(&mut ctx).expect("redo insert table"));
        }
        assert_eq!(body_table_count(&word_doc), 1);
        assert_eq!(word_doc.document.as_ref().unwrap().sections[0].end, 2);
        let table = word_doc.table_by_order(0).expect("redo restored table");
        assert_eq!(table.rows.len(), 2);
    }

    #[test]
    fn inserted_table_preserves_raw_node_bindings_for_followup_edits_and_export() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut word_doc =
            futures::executor::block_on(WordDocument::load(&data, &Default::default()))
                .expect("load fixture");
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTableCommand::new(0, 2, 2, vec![100.0, 100.0])),
                    &mut ctx,
                )
                .expect("insert table");
        }

        word_doc
            .set_table_column_widths(0, &[120.0, 240.0])
            .expect("format inserted table");
        let exported = word_doc.export_docx().expect("export inserted table");
        let reparsed =
            futures::executor::block_on(WordDocument::load(&exported, &Default::default()))
                .expect("reopen exported document");
        let table = reparsed
            .table_by_order(0)
            .expect("table survives round-trip");

        assert_eq!(table.columns[0].width, Some(120.0));
        assert_eq!(table.columns[1].width, Some(240.0));
    }

    fn model_texts(doc: &WordDocument) -> Vec<String> {
        (0..doc.paragraph_count())
            .map(|index| {
                doc.paragraph(index)
                    .map(|paragraph| {
                        paragraph
                            .runs
                            .iter()
                            .map(|run| run.text.as_str())
                            .collect::<String>()
                    })
                    .unwrap_or_default()
            })
            .collect()
    }

    fn body_kinds(doc: &WordDocument) -> Vec<&'static str> {
        doc.document
            .as_ref()
            .and_then(|document| document.body.as_ref())
            .map(|body| {
                body.elements
                    .iter()
                    .map(|element| match element {
                        BodyElement::Paragraph(_) => "p",
                        BodyElement::Table(_) => "tbl",
                        _ => "other",
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn hello_document() -> WordDocument {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read hello fixture");
        futures::executor::block_on(WordDocument::load(&data, &Default::default()))
            .expect("load hello fixture")
    }

    fn insert_at_caret(
        doc: &mut WordDocument,
        executor: &mut CommandExecutor,
        paragraph: usize,
        offset: usize,
    ) -> Option<SelectionRange> {
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(
            paragraph, offset,
        )));
        let mut ctx = CommandContext::new(doc, &mut selection);
        executor
            .execute(
                Box::new(InsertTableCommand::at_caret(
                    paragraph,
                    offset,
                    2,
                    2,
                    Vec::new(),
                )),
                &mut ctx,
            )
            .expect("insert table at caret");
        selection
    }

    /// Word puts a table inserted with the caret at the start of a paragraph
    /// above that paragraph, and leaves the caret in the first cell.
    #[test]
    fn caret_at_paragraph_start_puts_the_table_above_and_enters_the_first_cell() {
        let mut doc = hello_document();
        // "Hello" ends in a page break character, which stays in the run text.
        assert_eq!(model_texts(&doc), vec!["Hello\u{c}", "World"]);
        let mut executor = CommandExecutor::new();

        let selection = insert_at_caret(&mut doc, &mut executor, 0, 0);

        assert_eq!(body_kinds(&doc), vec!["tbl", "p", "p"]);
        assert_eq!(
            model_texts(&doc),
            vec!["", "", "", "", "Hello\u{c}", "World"]
        );
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(0, 0))),
            "the caret lands in the first cell"
        );

        let mut selection = selection;
        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor.undo(&mut ctx).expect("undo");
        }
        assert_eq!(body_kinds(&doc), vec!["p", "p"]);
        assert_eq!(model_texts(&doc), vec!["Hello\u{c}", "World"]);
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(0, 0)))
        );
    }

    /// A caret in the middle of a paragraph splits it around the table.
    #[test]
    fn caret_inside_a_paragraph_splits_it_around_the_table() {
        let mut doc = hello_document();
        let mut executor = CommandExecutor::new();

        let selection = insert_at_caret(&mut doc, &mut executor, 0, 2);

        assert_eq!(body_kinds(&doc), vec!["p", "tbl", "p", "p"]);
        assert_eq!(
            model_texts(&doc),
            vec!["He", "", "", "", "", "llo\u{c}", "World"]
        );
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(1, 0)))
        );

        let exported = doc.export_docx().expect("export");
        let reopened =
            futures::executor::block_on(WordDocument::load(&exported, &Default::default()))
                .expect("reopen");
        assert_eq!(body_kinds(&reopened), vec!["p", "tbl", "p", "p"]);
        assert_eq!(
            model_texts(&reopened),
            vec!["He", "", "", "", "", "llo\u{c}", "World"]
        );
    }

    /// At the end of the document's last paragraph the table follows it, and
    /// an empty paragraph keeps the body ending in a paragraph as Word does;
    /// from an empty paragraph the table goes above it.
    #[test]
    fn caret_at_the_end_of_the_last_paragraph_keeps_a_paragraph_after_the_table() {
        let mut doc = hello_document();
        let mut executor = CommandExecutor::new();

        let selection = insert_at_caret(&mut doc, &mut executor, 1, 5);

        assert_eq!(body_kinds(&doc), vec!["p", "p", "tbl", "p"]);
        assert_eq!(
            model_texts(&doc),
            vec!["Hello\u{c}", "World", "", "", "", "", ""]
        );
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(2, 0)))
        );

        // The trailing empty paragraph is now paragraph 6.
        let selection = insert_at_caret(&mut doc, &mut executor, 6, 0);
        assert_eq!(body_kinds(&doc), vec!["p", "p", "tbl", "tbl", "p"]);
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(6, 0)))
        );
        assert_eq!(model_texts(&doc).len(), 11);
    }

    /// At the end of a section's last paragraph the paragraph mark carrying
    /// the `w:sectPr` stays after the table, so the table remains in its
    /// section.
    #[test]
    fn caret_at_the_end_of_a_section_keeps_the_section_break_after_the_table() {
        let mut doc = hello_document();
        let hello_len = model_texts(&doc)[0].len();
        doc.insert_section_break_at(
            0,
            hello_len,
            crate::word_document::SectionBreakType::NextPage,
        )
        .expect("section break after Hello");
        assert_eq!(model_texts(&doc), vec!["Hello\u{c}", "", "World"]);
        let section_count = doc.document.as_ref().expect("document").sections.len();
        assert_eq!(section_count, 2);
        let mut executor = CommandExecutor::new();

        let selection = insert_at_caret(&mut doc, &mut executor, 0, hello_len);

        assert_eq!(body_kinds(&doc), vec!["p", "tbl", "p", "p", "p"]);
        assert_eq!(
            model_texts(&doc),
            vec!["Hello\u{c}", "", "", "", "", "", "", "World"]
        );
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(TextPosition::new(1, 0)))
        );
        let sections = &doc.document.as_ref().expect("document").sections;
        assert_eq!(sections.len(), 2, "no section was added or lost");
        assert_eq!(
            sections[0].end, 3,
            "the first section ends after the table and its empty terminator"
        );
    }

    #[test]
    fn inserting_from_a_cell_appends_a_nested_table_instead_of_splitting_the_outer_table() {
        let data = std::fs::read(fixture_path("table2x2.docx")).expect("read table fixture");
        let mut word_doc =
            futures::executor::block_on(WordDocument::load(&data, &Default::default()))
                .expect("load table fixture");
        let anchor = (0..word_doc.paragraph_count())
            .find(|&index| word_doc.paragraph_is_in_table(index))
            .expect("cell paragraph");
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(anchor, 0)));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTableCommand::new(anchor, 3, 2, Vec::new())),
                    &mut ctx,
                )
                .expect("insert nested table");
        }

        assert_eq!(body_table_count(&word_doc), 1, "outer table was split");
        assert_eq!(total_table_count(&word_doc), 2);
        let nested = word_doc.table_by_order(1).expect("nested table");
        assert_eq!(nested.rows.len(), 3);
        assert_eq!(nested.rows[0].cells.len(), 2);
        let outer = word_doc.table_by_order(0).expect("outer table");
        assert!(outer.rows[0].cells[0]
            .blocks
            .iter()
            .any(|block| matches!(block, BodyElement::Table(_))));

        let exported = word_doc.export_docx().expect("export nested table");
        let reopened =
            futures::executor::block_on(WordDocument::load(&exported, &Default::default()))
                .expect("reopen nested table");
        assert_eq!(body_table_count(&reopened), 1);
        assert_eq!(total_table_count(&reopened), 2);
    }

    #[test]
    fn inserting_from_a_text_box_paragraph_places_the_table_inside_the_box() {
        let data = std::fs::read(fixture_path("textbox.docx")).expect("read textbox fixture");
        let mut word_doc =
            futures::executor::block_on(WordDocument::load(&data, &Default::default()))
                .expect("load textbox fixture");
        let anchor = (0..word_doc.paragraph_count())
            .find(|&index| !word_doc.is_body_paragraph_surface(index))
            .expect("text box paragraph surface");
        let before_tables = total_table_count(&word_doc);
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(anchor, 0)));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTableCommand::new(anchor, 2, 2, Vec::new())),
                    &mut ctx,
                )
                .expect("insert table inside the text box");
        }
        assert_eq!(body_table_count(&word_doc), 0, "the body gets no table");
        assert_eq!(total_table_count(&word_doc), before_tables + 1);
        let text_box = (0..word_doc.paragraph_count())
            .filter_map(|index| word_doc.paragraph(index))
            .flat_map(|paragraph| paragraph.runs.iter())
            .find_map(|run| run.text_box.clone())
            .expect("text box");
        assert!(text_box
            .blocks
            .iter()
            .any(|block| matches!(block, BodyElement::Table(_))));

        let exported = word_doc.export_docx().expect("export text box table");
        let reopened =
            futures::executor::block_on(WordDocument::load(&exported, &Default::default()))
                .expect("reopen text box table");
        assert_eq!(body_table_count(&reopened), 0);
        assert_eq!(total_table_count(&reopened), before_tables + 1);

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo"));
        }
        assert_eq!(total_table_count(&word_doc), before_tables);
    }

    #[test]
    fn inserting_into_a_box_drawn_in_the_editor_survives_a_round_trip() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
        let mut word_doc =
            futures::executor::block_on(WordDocument::load(&data, &Default::default()))
                .expect("load fixture");
        word_doc
            .insert_text_box_at(1, 0)
            .expect("insert a text box in the editor");
        let anchor = (0..word_doc.paragraph_count())
            .find(|&index| !word_doc.is_body_paragraph_surface(index))
            .expect("text box paragraph surface");
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(anchor, 0)));
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTableCommand::new(anchor, 1, 3, Vec::new())),
                    &mut ctx,
                )
                .expect("insert table inside the drawn box");
        }
        assert_eq!(body_table_count(&word_doc), 0);
        assert_eq!(total_table_count(&word_doc), 1);
        let exported = word_doc.export_docx().expect("export");
        let reopened =
            futures::executor::block_on(WordDocument::load(&exported, &Default::default()))
                .expect("reopen");
        assert_eq!(body_table_count(&reopened), 0);
        assert_eq!(total_table_count(&reopened), 1);
        let reopened_table = reopened.table_by_order(0).expect("table in the box");
        assert_eq!(reopened_table.rows[0].cells.len(), 3);
    }

    #[test]
    fn inserting_from_an_existing_nested_cell_targets_the_innermost_cell() {
        let data =
            std::fs::read(fixture_path("table-nested.docx")).expect("read nested table fixture");
        let mut word_doc =
            futures::executor::block_on(WordDocument::load(&data, &Default::default()))
                .expect("load nested table fixture");
        let anchor = (0..word_doc.paragraph_count())
            .find(|&index| {
                word_doc.paragraph(index).is_some_and(|paragraph| {
                    paragraph.runs.iter().any(|run| run.text.contains("World"))
                })
            })
            .expect("paragraph in the existing nested table");
        let before_tables = total_table_count(&word_doc);
        let before_depth = max_table_depth(&word_doc);
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(anchor, 0)));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTableCommand::new(anchor, 4, 3, Vec::new())),
                    &mut ctx,
                )
                .expect("insert third-level table");
        }

        assert_eq!(body_table_count(&word_doc), 1);
        assert_eq!(total_table_count(&word_doc), before_tables + 1);
        assert_eq!(max_table_depth(&word_doc), before_depth + 1);
        assert!((0..total_table_count(&word_doc)).any(|order| {
            word_doc.table_by_order(order).is_some_and(|table| {
                table.rows.len() == 4 && table.rows.iter().all(|row| row.cells.len() == 3)
            })
        }));

        let exported = word_doc.export_docx().expect("export third-level table");
        let reopened =
            futures::executor::block_on(WordDocument::load(&exported, &Default::default()))
                .expect("reopen third-level table");
        assert_eq!(body_table_count(&reopened), 1);
        assert_eq!(total_table_count(&reopened), before_tables + 1);
        assert_eq!(max_table_depth(&reopened), before_depth + 1);
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 64,
            max_shrink_iters: 2_000,
            ..ProptestConfig::default()
        })]

        /// A table created inside a real DOCX must keep every generated raw XML
        /// binding valid across arbitrary dimensions and explicit grid widths.
        /// Formatting it immediately and reopening the export must reproduce
        /// the same grid instead of failing on a stale generated node id.
        #[test]
        fn inserted_table_grid_round_trips_for_generated_dimensions_and_widths(
            rows in 1usize..6,
            widths in prop::collection::vec(12u16..480, 1..8),
        ) {
            let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
            let mut word_doc = futures::executor::block_on(WordDocument::load(
                &data,
                &Default::default(),
            ))
            .expect("load fixture");
            let columns = widths.len();
            let initial_widths = vec![36.0; columns];
            let expected: Vec<f32> = widths.into_iter().map(f32::from).collect();
            let mut selection = None;
            let mut executor = CommandExecutor::new();

            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                executor
                    .execute(
                        Box::new(InsertTableCommand::new(
                            0,
                            rows,
                            columns,
                            initial_widths,
                        )),
                        &mut ctx,
                    )
                    .expect("insert table");
            }

            word_doc
                .set_table_column_widths(0, &expected)
                .expect("format inserted table");
            let exported = word_doc.export_docx().expect("export inserted table");
            let reparsed = futures::executor::block_on(WordDocument::load(
                &exported,
                &Default::default(),
            ))
            .expect("reopen exported document");
            let table = reparsed.table_by_order(0).expect("table survives round-trip");
            let actual: Vec<f32> = table
                .columns
                .iter()
                .map(|column| column.width.expect("explicit width"))
                .collect();

            prop_assert_eq!(table.rows.len(), rows);
            prop_assert_eq!(actual, expected);
        }

        /// Cell-local insertion must remain nested for arbitrary generated
        /// dimensions, restore the original package exactly on undo, and
        /// recreate the same package on redo. This exercises current-state
        /// paragraph lookup instead of caching the outer table's pre-edit
        /// coordinates.
        #[test]
        fn nested_table_insert_round_trips_generated_dimensions_and_history(
            rows in 1usize..7,
            columns in 1usize..7,
        ) {
            let data = std::fs::read(fixture_path("table2x2.docx"))
                .expect("read table fixture");
            let mut word_doc = futures::executor::block_on(WordDocument::load(
                &data,
                &Default::default(),
            ))
            .expect("load table fixture");
            let before = word_doc.export_docx().expect("export before");
            let anchor = (0..word_doc.paragraph_count())
                .find(|&index| word_doc.paragraph_is_in_table(index))
                .expect("cell paragraph");
            let mut selection = Some(SelectionRange::collapsed(TextPosition::new(anchor, 0)));
            let mut executor = CommandExecutor::new();

            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                executor
                    .execute(
                        Box::new(InsertTableCommand::new(
                            anchor,
                            rows,
                            columns,
                            Vec::new(),
                        )),
                        &mut ctx,
                    )
                    .expect("insert nested table");
            }
            prop_assert_eq!(body_table_count(&word_doc), 1);
            prop_assert_eq!(total_table_count(&word_doc), 2);
            let nested = word_doc.table_by_order(1).expect("nested table");
            prop_assert_eq!(nested.rows.len(), rows);
            prop_assert!(nested.rows.iter().all(|row| row.cells.len() == columns));
            let after = word_doc.export_docx().expect("export after insert");

            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                prop_assert!(executor.undo(&mut ctx).expect("undo nested insert"));
            }
            prop_assert_eq!(word_doc.export_docx().expect("export after undo"), before);
            prop_assert_eq!(body_table_count(&word_doc), 1);
            prop_assert_eq!(total_table_count(&word_doc), 1);

            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                prop_assert!(executor.redo(&mut ctx).expect("redo nested insert"));
            }
            prop_assert_eq!(word_doc.export_docx().expect("export after redo"), after.clone());
            let reopened = futures::executor::block_on(WordDocument::load(
                &after,
                &Default::default(),
            ))
            .expect("reopen nested table");
            prop_assert_eq!(body_table_count(&reopened), 1);
            prop_assert_eq!(total_table_count(&reopened), 2);
            let nested = reopened.table_by_order(1).expect("reopened nested table");
            prop_assert_eq!(nested.rows.len(), rows);
            prop_assert!(nested.rows.iter().all(|row| row.cells.len() == columns));
        }

        /// Top-level structural edits must keep section ranges covering every
        /// body element. The one-empty-paragraph case is especially important:
        /// a stale `end == 1` makes the renderer stop before the inserted table,
        /// while Enter happens to repair the range and makes it appear later.
        #[test]
        fn inserted_table_keeps_generated_section_ranges_renderable_through_history(
            paragraph_count in 1usize..12,
            raw_anchor in any::<usize>(),
            rows in 1usize..7,
            columns in 1usize..7,
        ) {
            let anchor = raw_anchor % paragraph_count;
            let elements = (0..paragraph_count)
                .map(|index| paragraph_with_text(if index == 0 { "" } else { "body" }))
                .collect();
            let mut word_doc = WordDocument::from_document(Document {
                body: Some(Body { elements }),
                sections: vec![SectionRange {
                    start: 0,
                    end: paragraph_count,
                    properties: None,
                }],
                ..Document::default()
            });
            let mut selection = None;
            let mut executor = CommandExecutor::new();
            let assert_cover = |document: &WordDocument, expected_body_len: usize| {
                let model = document.document.as_ref().expect("document");
                prop_assert_eq!(
                    model.body.as_ref().map(|body| body.elements.len()),
                    Some(expected_body_len),
                );
                prop_assert_eq!(model.sections.first().map(|section| section.start), Some(0));
                prop_assert_eq!(
                    model.sections.last().map(|section| section.end),
                    Some(expected_body_len),
                );
                prop_assert!(model
                    .sections
                    .windows(2)
                    .all(|pair| pair[0].end == pair[1].start));
                Ok(())
            };

            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                executor
                    .execute(
                        Box::new(InsertTableCommand::new(anchor, rows, columns, Vec::new())),
                        &mut ctx,
                    )
                    .expect("insert table");
            }
            assert_cover(&word_doc, paragraph_count + 1)?;

            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                prop_assert!(executor.undo(&mut ctx).expect("undo insert table"));
            }
            assert_cover(&word_doc, paragraph_count)?;

            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                prop_assert!(executor.redo(&mut ctx).expect("redo insert table"));
            }
            assert_cover(&word_doc, paragraph_count + 1)?;
        }

        /// Table- and cell-level border state must survive a package round trip
        /// for every supported common line style, eighth-point width and RGB
        /// colour. This also exercises formatting immediately after insertion.
        #[test]
        fn inserted_table_borders_round_trip_for_generated_styles(
            rows in 1usize..5,
            columns in 1usize..6,
            table_style in prop::sample::select(vec!["single", "double", "dashed", "dotted", "thick", "none"]),
            cell_style in prop::sample::select(vec!["single", "double", "dashed", "dotted", "thick", "none"]),
            table_width_eighths in 1u8..64,
            cell_width_eighths in 1u8..64,
            table_rgb in any::<[u8; 3]>(),
            cell_rgb in any::<[u8; 3]>(),
        ) {
            let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
            let mut word_doc = futures::executor::block_on(WordDocument::load(
                &data,
                &Default::default(),
            ))
            .expect("load fixture");
            word_doc
                .insert_table_after_paragraph(0, rows, columns, &vec![72.0; columns])
                .expect("insert table");

            let table_color = format!(
                "#{:02X}{:02X}{:02X}",
                table_rgb[0], table_rgb[1], table_rgb[2]
            );
            let cell_color = format!(
                "#{:02X}{:02X}{:02X}",
                cell_rgb[0], cell_rgb[1], cell_rgb[2]
            );
            let table_width = f32::from(table_width_eighths) / 8.0;
            let cell_width = f32::from(cell_width_eighths) / 8.0;
            word_doc
                .set_table_borders(
                    0,
                    &TableBorderSpec {
                        top: Some(BorderEdgeSpec {
                            style: Some(table_style.to_string()),
                            width: Some(table_width),
                            color: Some(table_color.clone()),
                        }),
                        inside_h: Some(BorderEdgeSpec {
                            style: Some("none".to_string()),
                            ..BorderEdgeSpec::default()
                        }),
                        ..TableBorderSpec::default()
                    },
                )
                .expect("set table borders");
            word_doc
                .set_table_cell_borders(
                    0,
                    0,
                    0,
                    &TableBorderSpec {
                        bottom: Some(BorderEdgeSpec {
                            style: Some(cell_style.to_string()),
                            width: Some(cell_width),
                            color: Some(cell_color.clone()),
                        }),
                        ..TableBorderSpec::default()
                    },
                )
                .expect("set cell borders");

            let exported = word_doc.export_docx().expect("export bordered table");
            let reparsed = futures::executor::block_on(WordDocument::load(
                &exported,
                &Default::default(),
            ))
            .expect("reopen bordered table");
            let table = reparsed.table_by_order(0).expect("table survives round-trip");
            let top = table
                .borders
                .as_ref()
                .and_then(|borders| borders.top.as_ref())
                .expect("top border");
            let inside_h = table
                .borders
                .as_ref()
                .and_then(|borders| borders.inside_h.as_ref())
                .expect("inside horizontal border");
            let cell_borders = reparsed
                .cell_borders_from_style(0, 0, 0)
                .expect("cell borders");
            let bottom = cell_borders.bottom.as_ref().expect("bottom cell border");

            prop_assert_eq!(top.val.as_deref(), Some(table_style));
            prop_assert_eq!(top.size_eighths, Some(u32::from(table_width_eighths)));
            prop_assert_eq!(top.color.as_deref(), Some(table_color.as_str()));
            prop_assert_eq!(inside_h.val.as_deref(), Some("none"));
            prop_assert_eq!(bottom.val.as_deref(), Some(cell_style));
            prop_assert_eq!(bottom.size_eighths, Some(u32::from(cell_width_eighths)));
            prop_assert_eq!(bottom.color.as_deref(), Some(cell_color.as_str()));
        }

        /// Table defaults and direct cell overrides must retain every generated
        /// quarter-point edge through OOXML export and reopen. The two levels are
        /// asserted independently so direct-cell precedence remains observable.
        #[test]
        fn inserted_table_cell_margins_round_trip_for_generated_edges(
            rows in 1usize..5,
            columns in 1usize..6,
            table_edges in any::<[u8; 4]>(),
            cell_edges in any::<[u8; 4]>(),
        ) {
            let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
            let mut word_doc = futures::executor::block_on(WordDocument::load(
                &data,
                &Default::default(),
            ))
            .expect("load fixture");
            word_doc
                .insert_table_after_paragraph(0, rows, columns, &vec![72.0; columns])
                .expect("insert table");

            let points = |edges: [u8; 4]| edges.map(|edge| f32::from(edge) / 4.0);
            let table_points = points(table_edges);
            let cell_points = points(cell_edges);
            let margins = |edges: [f32; 4]| TableCellMargins {
                top: Some(edges[0]),
                bottom: Some(edges[1]),
                left: Some(edges[2]),
                right: Some(edges[3]),
                ..TableCellMargins::default()
            };
            word_doc
                .set_table_cell_margins(0, None, margins(table_points))
                .expect("set table default cell margins");
            word_doc
                .set_table_cell_margins(0, Some((0, 0)), margins(cell_points))
                .expect("set direct cell margins");

            let exported = word_doc.export_docx().expect("export padded table");
            let reparsed = futures::executor::block_on(WordDocument::load(
                &exported,
                &Default::default(),
            ))
            .expect("reopen padded table");
            let table = reparsed.table_by_order(0).expect("table survives round-trip");
            let actual_table = table.cell_margins.as_ref().expect("table cell margins");
            let actual_cell = table.rows[0].cells[0]
                .margins
                .as_ref()
                .expect("direct cell margins");

            prop_assert_eq!(
                [actual_table.top, actual_table.bottom, actual_table.left, actual_table.right],
                table_points.map(Some),
            );
            prop_assert_eq!(
                [actual_cell.top, actual_cell.bottom, actual_cell.left, actual_cell.right],
                cell_points.map(Some),
            );
        }

        /// Setting a generated visual row height must remain stable across
        /// undo/redo and DOCX reopen even when table-default bottom padding is
        /// non-zero. The persisted exact height alone is intentionally smaller.
        #[test]
        fn inserted_table_row_height_round_trips_with_generated_bottom_margin(
            rows in 1usize..5,
            columns in 1usize..6,
            raw_row in any::<usize>(),
            visual_quarter_points in 80u16..800,
            bottom_quarter_points in 0u8..64,
        ) {
            let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
            let mut word_doc = futures::executor::block_on(WordDocument::load(
                &data,
                &Default::default(),
            ))
            .expect("load fixture");
            word_doc
                .insert_table_after_paragraph(0, rows, columns, &vec![72.0; columns])
                .expect("insert table");
            let row = raw_row % rows;
            let visual_height = f32::from(visual_quarter_points) / 4.0;
            let bottom_margin = f32::from(bottom_quarter_points) / 4.0;
            word_doc
                .set_table_cell_margins(
                    0,
                    None,
                    TableCellMargins {
                        bottom: Some(bottom_margin),
                        ..TableCellMargins::default()
                    },
                )
                .expect("set bottom margin");

            let mut selection = None;
            let mut executor = CommandExecutor::new();
            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                executor
                    .execute(
                        Box::new(ResizeTableRowCommand::table_bottom_edge(
                            0,
                            row,
                            visual_height,
                        )),
                        &mut ctx,
                    )
                    .expect("set row height");
            }
            prop_assert_eq!(
                word_doc.table_by_order(0).and_then(|table| visual_height_for_resize(table, row)),
                Some(visual_height),
            );
            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                prop_assert!(executor.undo(&mut ctx).expect("undo row height"));
            }
            prop_assert_eq!(word_doc.table_by_order(0).unwrap().rows[row].height, None);
            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                prop_assert!(executor.redo(&mut ctx).expect("redo row height"));
            }

            let exported = word_doc.export_docx().expect("export resized table");
            let reparsed = futures::executor::block_on(WordDocument::load(
                &exported,
                &Default::default(),
            ))
            .expect("reopen resized table");
            let table = reparsed.table_by_order(0).expect("table survives round-trip");
            prop_assert_eq!(table.rows[row].height_rule, Some(crate::document::TableRowHeightRule::Exact));
            prop_assert_eq!(visual_height_for_resize(table, row), Some(visual_height));
        }

        /// Gallery presets must format every physical cell, undo atomically,
        /// and survive export/reopen for generated table dimensions.
        #[test]
        fn inserted_table_style_presets_round_trip_for_generated_dimensions(
            rows in 1usize..7,
            columns in 1usize..7,
            preset_index in 0usize..5,
        ) {
            let presets = ["plain", "grid", "lightBlue", "blueBands", "grayBands"];
            let preset = presets[preset_index];
            let data = std::fs::read(fixture_path("hello.docx")).expect("read fixture");
            let mut word_doc = futures::executor::block_on(WordDocument::load(
                &data,
                &Default::default(),
            ))
            .expect("load fixture");
            word_doc
                .insert_table_after_paragraph(0, rows, columns, &vec![72.0; columns])
                .expect("insert table");

            let mut selection = None;
            let mut executor = CommandExecutor::new();
            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                executor
                    .execute(
                        Box::new(SnapshotEditCommand::new(
                            "apply_table_style_preset",
                            move |document| document.apply_table_style_preset(0, preset),
                        )),
                        &mut ctx,
                    )
                    .expect("apply preset");
            }
            {
                let table = word_doc.table_by_order(0).expect("formatted table");
                prop_assert_eq!(table.style_name.as_deref(), None);
                prop_assert_eq!(table.rows.len(), rows);
                prop_assert!(table.rows.iter().all(|row| row.cells.len() == columns));
            }
            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                prop_assert!(executor.undo(&mut ctx).expect("undo preset"));
            }
            prop_assert!(word_doc.table_by_order(0).unwrap().rows.iter().all(|row| row
                .cells
                .iter()
                .all(|cell| !cell.style.contains_key("background-color"))));
            {
                let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
                prop_assert!(executor.redo(&mut ctx).expect("redo preset"));
            }

            let exported = word_doc.export_docx().expect("export styled table");
            let reparsed = futures::executor::block_on(WordDocument::load(
                &exported,
                &Default::default(),
            ))
            .expect("reopen styled table");
            let table = reparsed.table_by_order(0).expect("table survives round-trip");
            let border_style = if preset == "plain" { "none" } else { "single" };
            for (row_index, row) in table.rows.iter().enumerate() {
                let fill = match preset {
                    "lightBlue" if row_index == 0 => "#D9EAF7",
                    "blueBands" if row_index == 0 => "#BDD7EE",
                    "blueBands" if row_index % 2 == 0 => "#DDEBF7",
                    "grayBands" if row_index == 0 => "#D9E1F2",
                    "grayBands" if row_index % 2 == 0 => "#E7E6E6",
                    _ => "#FFFFFF",
                };
                for cell in &row.cells {
                    prop_assert_eq!(cell.style.get("background-color").map(String::as_str), Some(fill));
                    prop_assert_eq!(cell.style.get("border-top-style").map(String::as_str), Some(border_style));
                }
            }
        }
    }

    #[test]
    fn insert_table_after_nested_tables_records_the_inserted_top_level_position() {
        let nested = table_with_blocks(vec![paragraph_with_text("nested")]);
        let outer = table_with_blocks(vec![paragraph_with_text("host"), nested]);
        let mut word_doc = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![outer, paragraph_with_text("anchor")],
            }),
            ..Document::default()
        });
        let anchor_paragraph = word_doc.paragraph_count() - 1;
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertTableCommand::new(anchor_paragraph, 1, 1, Vec::new())),
                    &mut ctx,
                )
                .expect("insert after nested tables");
        }

        assert_eq!(body_table_count(&word_doc), 2);
        assert!(executor.can_undo());
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        assert!(executor.undo(&mut ctx).expect("undo inserted table"));
        assert_eq!(body_table_count(&word_doc), 1);
    }
}
