use super::{Command, CommandContext, CommandError, CommandResult};
use crate::word_document::WordDocumentSnapshot;
use jade_docx_layout::selection::{SelectionRange, TextPosition};

/// Word's 表の分割 (Ctrl+Shift+Enter): the rows from `row_index` on become
/// a second table with an empty paragraph between the two tables. At row 0
/// the empty paragraph goes in front of the table instead, as in Word.
///
/// The body gains elements and the raw table XML is split in two, so undo
/// restores a package snapshot rather than patching the pieces back.
pub struct SplitTableCommand {
    table_order_index: usize,
    row_index: usize,
    previous: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
}

impl SplitTableCommand {
    pub fn new(table_order_index: usize, row_index: usize) -> Self {
        Self {
            table_order_index,
            row_index,
            previous: None,
            previous_selection: None,
        }
    }
}

impl Command for SplitTableCommand {
    fn name(&self) -> &'static str {
        "split_table"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or(CommandError::DocumentUnavailable)?;
        let previous_selection = *ctx.selection;
        match ctx
            .document
            .split_table(self.table_order_index, self.row_index)
        {
            Ok(break_paragraph_index) => {
                self.previous = Some(snapshot);
                self.previous_selection = previous_selection;
                // The empty paragraph is inserted before the caret's row, so
                // the caret keeps its cell (Word leaves it in the split row).
                let shift = |position: TextPosition| {
                    if position.paragraph >= break_paragraph_index {
                        TextPosition {
                            paragraph: position.paragraph + 1,
                            ..position
                        }
                    } else {
                        position
                    }
                };
                if let Some(selection) = ctx.selection.as_mut() {
                    selection.anchor = shift(selection.anchor);
                    selection.focus = shift(selection.focus);
                }
                Ok(())
            }
            Err(error) => {
                ctx.document.restore_snapshot(snapshot);
                Err(CommandError::Custom(error))
            }
        }
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self
            .previous
            .take()
            .ok_or_else(|| CommandError::Custom("split table snapshot is missing".to_string()))?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandExecutor;
    use crate::word_document::WordDocument;
    use crate::Options;

    fn fixture() -> WordDocument {
        let data = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../public/fixtures/table-cell-spacing.docx"
        ))
        .expect("read table fixture");
        futures::executor::block_on(WordDocument::load(&data, &Options::default()))
            .expect("load table fixture")
    }

    fn table_count(doc: &WordDocument) -> usize {
        (0..)
            .take_while(|index| doc.table_by_order(*index).is_some())
            .count()
    }

    #[test]
    fn splitting_a_table_leaves_an_empty_paragraph_between_the_halves() {
        let mut doc = fixture();
        let rows = doc.table_by_order(0).expect("table").rows.len();
        assert!(rows >= 2, "fixture table has {rows} rows");
        let tables_before = table_count(&doc);
        let paragraphs_before = doc.paragraph_count();
        // The caret sits in the first cell of row 1, the row that starts the
        // second table.
        let row_1_text = doc.table_by_order(0).expect("table").rows[1].cells[0]
            .blocks
            .iter()
            .find_map(|block| match block {
                crate::document::BodyElement::Paragraph(paragraph) => Some(
                    paragraph
                        .runs
                        .iter()
                        .map(|run| run.text.as_str())
                        .collect::<String>(),
                ),
                _ => None,
            })
            .expect("row 1 has a paragraph");
        let row_1_paragraph = (0..doc.paragraph_count())
            .find(|index| {
                doc.paragraph(*index).is_some_and(|paragraph| {
                    paragraph
                        .runs
                        .iter()
                        .map(|run| run.text.as_str())
                        .collect::<String>()
                        == row_1_text
                })
            })
            .expect("row 1 paragraph index");
        let mut selection = Some(SelectionRange::collapsed(TextPosition {
            paragraph: row_1_paragraph,
            offset: 0,
        }));
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor
                .execute(Box::new(SplitTableCommand::new(0, 1)), &mut ctx)
                .expect("split table");
        }
        assert_eq!(
            selection.map(|range| range.focus.paragraph),
            Some(row_1_paragraph + 1),
            "the caret follows its cell into the second table"
        );
        assert_eq!(table_count(&doc), tables_before + 1);
        assert_eq!(doc.paragraph_count(), paragraphs_before + 1);
        assert_eq!(doc.table_by_order(0).expect("head").rows.len(), 1);
        assert_eq!(doc.table_by_order(1).expect("tail").rows.len(), rows - 1);
        let body_index = doc
            .body_index_of_table_order(0)
            .expect("head table in body");
        let document = doc.document.as_ref().expect("document");
        let body = document.body.as_ref().expect("body");
        assert!(
            matches!(
                body.elements.get(body_index + 1),
                Some(crate::document::BodyElement::Paragraph(paragraph)) if paragraph.runs.is_empty()
            ),
            "an empty paragraph follows the head table"
        );
        assert!(
            matches!(
                body.elements.get(body_index + 2),
                Some(crate::document::BodyElement::Table(_))
            ),
            "the tail table follows the paragraph"
        );

        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor.undo(&mut ctx).expect("undo split");
        }
        assert_eq!(table_count(&doc), tables_before);
        assert_eq!(doc.paragraph_count(), paragraphs_before);
        assert_eq!(doc.table_by_order(0).expect("table").rows.len(), rows);
    }

    #[test]
    fn splitting_at_the_first_row_puts_a_paragraph_in_front() {
        let mut doc = fixture();
        let tables_before = table_count(&doc);
        let paragraphs_before = doc.paragraph_count();
        let mut selection = None;
        let mut ctx = CommandContext::new(&mut doc, &mut selection);
        SplitTableCommand::new(0, 0)
            .apply(&mut ctx)
            .expect("split at row 0");
        assert_eq!(table_count(&doc), tables_before);
        assert_eq!(doc.paragraph_count(), paragraphs_before + 1);
    }
}
