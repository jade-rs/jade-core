use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::BodyElement;
use crate::docx_xml::{XmlNode, XmlNodeId};
use crate::word_document::WordDocumentSnapshot;
use jade_docx_layout::selection::SelectionRange;

/// Removes a whole table (Word's 表の削除) with undo support.
///
/// A top-level table is detached from the body: undo re-inserts the backed-up
/// body element and the raw `<w:tbl>` XML node at their original positions.
/// `insert_node` reassigns the node ids of the re-inserted subtree, so after
/// restoring we point the model table at the fresh root id and run
/// `rewrite_table_xml_from_model` to bring the nested model node ids back in
/// sync with the raw tree (the same proven path the row/column mutations use).
///
/// A table nested in a cell or a text box is removed from its container
/// instead, and undo restores a package snapshot: the container's raw XML
/// changes shape (a paragraph may have to be appended so the cell stays
/// valid), which the element-level backup cannot express.
pub struct DeleteTableCommand {
    table_order_index: usize,
    backup: Option<DeleteTableBackup>,
    previous_selection: Option<SelectionRange>,
}

enum DeleteTableBackup {
    TopLevel {
        body_index: usize,
        element: BodyElement,
        raw: Option<RawTableBackup>,
    },
    Nested(Box<WordDocumentSnapshot>),
}

struct RawTableBackup {
    parent_id: XmlNodeId,
    insert_index: usize,
    node: XmlNode,
}

impl DeleteTableCommand {
    pub fn new(table_order_index: usize) -> Self {
        Self {
            table_order_index,
            backup: None,
            previous_selection: None,
        }
    }
}

impl DeleteTableCommand {
    /// Physical removal used after a revision decision as well as untracked edits.
    pub(crate) fn apply_untracked(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous_selection = *ctx.selection;
        let Some(body_index) = ctx
            .document
            .body_index_of_table_order(self.table_order_index)
        else {
            let snapshot = ctx
                .document
                .snapshot_including_package()
                .ok_or(CommandError::DocumentUnavailable)?;
            if let Err(error) = ctx
                .document
                .remove_nested_table_by_order(self.table_order_index)
            {
                ctx.document.restore_snapshot(snapshot);
                return Err(CommandError::Custom(error));
            }
            self.backup = Some(DeleteTableBackup::Nested(Box::new(snapshot)));
            self.previous_selection = previous_selection;
            return Ok(());
        };

        // Capture the raw XML node id before the model element is detached.
        let table_node_id = ctx
            .document
            .table_by_order(self.table_order_index)
            .and_then(|table| table.node_id);

        // Remove raw XML first. If this fails, the model remains untouched and
        // the command is not recorded in history.
        let raw = match (ctx.document.raw_document.as_mut(), table_node_id) {
            (Some(raw), Some(node_id)) => {
                let parent_id = raw.parent_id(node_id);
                let path = raw.path(node_id);
                match (parent_id, path) {
                    (Some(parent_id), Some(path)) => {
                        let insert_index = path.as_slice().last().copied().unwrap_or(0);
                        let node = raw
                            .remove_node(&path)
                            .map_err(|err| CommandError::Custom(err.to_string()))?;
                        Some(RawTableBackup {
                            parent_id,
                            insert_index,
                            node,
                        })
                    }
                    _ => {
                        return Err(CommandError::Custom(
                            "table raw XML node is not indexed".to_string(),
                        ))
                    }
                }
            }
            _ => None,
        };

        let element = {
            let document = ctx
                .document
                .document_mut()
                .ok_or(CommandError::DocumentUnavailable)?;
            let body = document
                .body
                .as_mut()
                .ok_or(CommandError::DocumentUnavailable)?;
            if body_index >= body.elements.len() {
                return Err(CommandError::Custom(
                    "table body index is out of range".to_string(),
                ));
            }
            let removed = body.elements.remove(body_index);
            crate::word_document::WordDocument::rebuild_section_ranges(document);
            removed
        };

        ctx.document.rebuild_paragraph_surfaces();
        self.backup = Some(DeleteTableBackup::TopLevel {
            body_index,
            element,
            raw,
        });
        self.previous_selection = previous_selection;
        Ok(())
    }
}

impl Command for DeleteTableCommand {
    fn name(&self) -> &'static str {
        "delete_table"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let Some(session) = ctx.document.revision_session().cloned() else {
            return self.apply_untracked(ctx);
        };
        let rows: Vec<_> = ctx
            .document
            .table_by_order(self.table_order_index)
            .ok_or_else(|| {
                CommandError::Custom(format!("table {} was not found", self.table_order_index))
            })?
            .rows
            .iter()
            .map(|row| row.revision.as_deref().cloned())
            .collect();
        if rows
            .iter()
            .flatten()
            .any(|mark| mark.author != session.author)
        {
            return Err(CommandError::Custom(
                "decide the existing row revision before deleting this table".into(),
            ));
        }
        for (row, mark) in rows.iter().enumerate() {
            if mark.as_ref().is_some_and(|mark| !mark.is_deletion())
                && ctx.document.row_has_foreign_revisions(
                    self.table_order_index,
                    row,
                    session.author.as_deref(),
                )
            {
                return Err(CommandError::Custom(
                    "decide the other author's revisions before cancelling this inserted row"
                        .into(),
                ));
            }
        }
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or(CommandError::DocumentUnavailable)?;
        let previous_selection = *ctx.selection;
        let mark = rows.iter().any(Option::is_none).then(|| {
            ctx.document
                .new_row_revision(jade_docx_reader::RevisionKind::Deleted)
                .expect("revision session is open")
        });
        let result = (|| {
            for (row, existing) in rows.into_iter().enumerate().rev() {
                match existing {
                    Some(existing) if existing.is_deletion() => {}
                    Some(_) => {
                        if ctx
                            .document
                            .table_by_order(self.table_order_index)
                            .is_some_and(|table| table.rows.len() == 1)
                        {
                            self.apply_untracked(ctx)?;
                        } else {
                            ctx.document
                                .delete_table_row(self.table_order_index, row)
                                .map_err(CommandError::Custom)?;
                        }
                    }
                    None => ctx
                        .document
                        .set_table_row_revision(self.table_order_index, row, mark.clone())
                        .map_err(CommandError::Custom)?,
                }
            }
            Ok(())
        })();
        if result.is_err() {
            ctx.document.restore_snapshot(snapshot);
            return result;
        }
        self.backup = Some(DeleteTableBackup::Nested(Box::new(snapshot)));
        self.previous_selection = previous_selection;
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let backup = self
            .backup
            .take()
            .ok_or_else(|| CommandError::Custom("delete_table backup is missing".into()))?;
        let (body_index, element, raw) = match backup {
            DeleteTableBackup::Nested(snapshot) => {
                ctx.document.restore_snapshot(*snapshot);
                *ctx.selection = self.previous_selection;
                return Ok(());
            }
            DeleteTableBackup::TopLevel {
                body_index,
                element,
                raw,
            } => (body_index, element, raw),
        };

        {
            let document = ctx
                .document
                .document_mut()
                .ok_or(CommandError::DocumentUnavailable)?;
            let body = document
                .body
                .as_mut()
                .ok_or(CommandError::DocumentUnavailable)?;
            let insert_at = body_index.min(body.elements.len());
            body.elements.insert(insert_at, element);
            crate::word_document::WordDocument::rebuild_section_ranges(document);
        }

        if let Some(raw_backup) = raw {
            let inserted_id = {
                let raw =
                    ctx.document.raw_document.as_mut().ok_or_else(|| {
                        CommandError::Custom("raw document is unavailable".into())
                    })?;
                raw.insert_node(
                    raw_backup.parent_id,
                    raw_backup.insert_index,
                    raw_backup.node,
                )
                .map_err(|err| CommandError::Custom(err.to_string()))?
            };
            if let (Some(inserted_id), Some(table)) = (
                inserted_id,
                ctx.document.table_mut_by_order(self.table_order_index),
            ) {
                table.node_id = Some(inserted_id);
            }
            // `insert_node` reassigned the subtree's node ids; regenerate the
            // raw `<w:tbl>` from the restored model so nested model node ids
            // (rows / cells / paragraphs) line up with the raw tree again.
            ctx.document
                .rewrite_table_xml_from_model(self.table_order_index)
                .map_err(CommandError::Custom)?;
        }

        ctx.document.rebuild_paragraph_surfaces();
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::{
        Body, BodyElement, Document, Paragraph, Run, SectionRange, StyleMap, Table, TableCell,
        TableRow,
    };
    use crate::word_document::WordDocument;
    use jade_docx_reader::docx_xml::{DocxXmlDocument, XmlNodeId};

    fn make_paragraph(text: &str) -> BodyElement {
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

    fn make_cell(text: &str) -> TableCell {
        TableCell {
            blocks: vec![make_paragraph(text)],
            style: StyleMap::default(),
            class_name: None,
            span: None,
            vertical_merge: None,
            text_direction: None,
            margins: None,
            vertical_align: None,
            node_id: None,
        }
    }

    fn make_table(texts: &[&str]) -> Table {
        Table {
            rows: vec![TableRow {
                revision: None,
                cells: texts.iter().map(|text| make_cell(text)).collect(),
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
        }
    }

    fn document_with_table_between_paragraphs() -> WordDocument {
        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![
                    make_paragraph("before"),
                    BodyElement::Table(Box::new(make_table(&["X1", "X2"]))),
                    make_paragraph("after"),
                ],
            }),
            sections: vec![SectionRange {
                start: 0,
                end: 3,
                properties: None,
            }],
            ..Document::default()
        })
    }

    fn body_element_count(doc: &WordDocument) -> usize {
        doc.document
            .as_ref()
            .and_then(|d| d.body.as_ref())
            .map(|body| body.elements.len())
            .unwrap_or(0)
    }

    #[test]
    fn tracked_table_delete_and_insert_cancellation_support_undo_redo() {
        for inserted in [false, true] {
            let mut document = document_with_table_between_paragraphs();
            document.set_revision_session(Some(crate::word_document::RevisionSession {
                author: Some("LAWGUE".into()),
                ..Default::default()
            }));
            if inserted {
                let mark = document.new_row_revision(jade_docx_reader::RevisionKind::Inserted);
                document.set_table_row_revision(0, 0, mark).unwrap();
            }
            let mut selection = None;
            let mut executor = CommandExecutor::new();
            let mut context = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(Box::new(DeleteTableCommand::new(0)), &mut context)
                .unwrap();
            assert_eq!(context.document.table_by_order(0).is_none(), inserted);
            if !inserted {
                assert!(context.document.table_by_order(0).unwrap().rows[0]
                    .revision
                    .as_ref()
                    .unwrap()
                    .is_deletion());
            }
            assert!(executor.undo(&mut context).unwrap());
            let row = &context.document.table_by_order(0).unwrap().rows[0];
            assert_eq!(row.revision.is_some(), inserted);
            assert_eq!(row.cells.len(), 2);
            assert!(executor.redo(&mut context).unwrap());
            assert_eq!(context.document.table_by_order(0).is_none(), inserted);
            assert!(executor.undo(&mut context).unwrap());
            assert_eq!(
                context.document.table_by_order(0).unwrap().rows[0]
                    .cells
                    .len(),
                2
            );
        }
    }

    #[test]
    fn delete_table_supports_undo_and_redo() {
        let mut word_doc = document_with_table_between_paragraphs();
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        assert_eq!(body_element_count(&word_doc), 3);
        assert!(word_doc.table_by_order(0).is_some());

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(Box::new(DeleteTableCommand::new(0)), &mut ctx)
                .expect("delete table");
        }
        assert_eq!(body_element_count(&word_doc), 2);
        assert!(word_doc.table_by_order(0).is_none());
        assert_eq!(word_doc.document.as_ref().unwrap().sections[0].end, 2);

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo delete table"));
        }
        assert_eq!(body_element_count(&word_doc), 3);
        let table = word_doc.table_by_order(0).expect("table restored");
        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0].cells.len(), 2);
        assert_eq!(word_doc.document.as_ref().unwrap().sections[0].end, 3);

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.redo(&mut ctx).expect("redo delete table"));
        }
        assert_eq!(body_element_count(&word_doc), 2);
        assert!(word_doc.table_by_order(0).is_none());
        assert_eq!(word_doc.document.as_ref().unwrap().sections[0].end, 2);
    }

    fn fixture_path(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../public/fixtures")
            .join(name)
    }

    fn total_table_count(doc: &WordDocument) -> usize {
        (0..)
            .take_while(|order| doc.table_by_order(*order).is_some())
            .count()
    }

    #[test]
    fn delete_table_removes_a_nested_table_and_keeps_the_host_cell_valid() {
        // A table whose first cell holds a nested table: order 0 = outer,
        // order 1 = nested. Word's 表の削除 on the nested table removes only
        // that table; the host cell keeps its paragraph.
        let mut outer = make_table(&["host"]);
        outer.rows[0].cells[0]
            .blocks
            .push(BodyElement::Table(Box::new(make_table(&["inner"]))));
        let mut word_doc = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::Table(Box::new(outer))],
            }),
            ..Document::default()
        });
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(Box::new(DeleteTableCommand::new(1)), &mut ctx)
                .expect("delete nested table");
        }
        assert_eq!(body_element_count(&word_doc), 1);
        assert!(word_doc.table_by_order(1).is_none(), "nested table removed");
        let host = word_doc.table_by_order(0).expect("outer table intact");
        assert_eq!(host.rows[0].cells[0].blocks.len(), 1);
        assert!(matches!(
            host.rows[0].cells[0].blocks[0],
            BodyElement::Paragraph(_)
        ));

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo nested delete"));
        }
        let host = word_doc.table_by_order(0).expect("outer table intact");
        assert_eq!(host.rows[0].cells[0].blocks.len(), 2);
        assert!(
            word_doc.table_by_order(1).is_some(),
            "nested table restored"
        );
    }

    #[test]
    fn delete_table_removes_a_nested_table_from_a_real_package() {
        let data =
            std::fs::read(fixture_path("table-nested.docx")).expect("read nested table fixture");
        let mut word_doc =
            futures::executor::block_on(WordDocument::load(&data, &Default::default()))
                .expect("load nested table fixture");
        let before_tables = total_table_count(&word_doc);
        assert!(before_tables >= 2, "fixture nests at least one table");
        // The last table in depth-first order is nested (the fixture has a
        // single top-level table).
        let nested_order = before_tables - 1;
        assert!(word_doc.body_index_of_table_order(nested_order).is_none());
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(Box::new(DeleteTableCommand::new(nested_order)), &mut ctx)
                .expect("delete nested table");
        }
        assert_eq!(total_table_count(&word_doc), before_tables - 1);
        let exported = word_doc.export_docx().expect("export after nested delete");
        let reopened =
            futures::executor::block_on(WordDocument::load(&exported, &Default::default()))
                .expect("reopen after nested delete");
        assert_eq!(total_table_count(&reopened), before_tables - 1);

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo nested delete"));
        }
        assert_eq!(total_table_count(&word_doc), before_tables);
        let exported = word_doc.export_docx().expect("export after undo");
        let reopened =
            futures::executor::block_on(WordDocument::load(&exported, &Default::default()))
                .expect("reopen after undo");
        assert_eq!(total_table_count(&reopened), before_tables);
    }

    #[test]
    fn delete_table_raw_failure_keeps_model_table() {
        let mut word_doc = document_with_table_between_paragraphs();
        word_doc.table_mut_by_order(0).expect("table").node_id = Some(XmlNodeId::new(9999));
        word_doc.raw_document = Some(
            DocxXmlDocument::parse(
                r#"<w:document xmlns:w="urn:w"><w:body><w:p/></w:body></w:document>"#,
            )
            .expect("raw document"),
        );
        let mut selection = None;
        let mut executor = CommandExecutor::new();

        let result = {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor.execute(Box::new(DeleteTableCommand::new(0)), &mut ctx)
        };

        assert!(result.is_err());
        assert_eq!(body_element_count(&word_doc), 3);
        assert!(word_doc.table_by_order(0).is_some());
        assert!(!executor.can_undo());
    }
}
