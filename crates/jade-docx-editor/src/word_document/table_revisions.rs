//! Table rows inserted or deleted with changes tracked (`w:trPr/w:ins`,
//! `w:trPr/w:del`, ECMA-376 Part 1 §17.13.5): listed with the other
//! changes, marked when a row is inserted or deleted while changes are
//! tracked, and decided — an accepted deletion or a rejected insertion
//! takes the row out, the other way the mark goes.

use super::review_tools::{child, local};
use super::*;

/// A table row inserted or deleted with changes tracked.
#[derive(Clone, Debug, PartialEq)]
pub struct RowRevision {
    /// The table, by its order in the document.
    pub table: usize,
    pub row: usize,
    pub mark: RevisionMark,
    /// The row's paragraphs, in document order.
    pub paragraphs: Vec<usize>,
}

impl RevisionTargets {
    /// Whether a target names a row revision `id` on one of `paragraphs`.
    fn names_row(&self, paragraphs: &[usize], id: Option<&str>) -> bool {
        paragraphs.iter().any(|paragraph| {
            self.runs
                .get(paragraph)
                .is_some_and(|spans| spans.iter().any(|(_, _, span_id)| span_id.as_deref() == id))
        })
    }
}

impl WordDocument {
    /// Every row inserted or deleted with changes tracked, in document
    /// order.
    pub fn table_row_revisions(&self) -> Vec<RowRevision> {
        let index_of: HashMap<XmlNodeId, usize> = (0..self.paragraph_count())
            .filter_map(|index| Some((self.paragraph(index)?.node_id?, index)))
            .collect();
        let mut out = Vec::new();
        let mut order = 0;
        while let Some(table) = self.table_by_order(order) {
            for (row_index, row) in table.rows.iter().enumerate() {
                let Some(mark) = row.revision.as_deref() else {
                    continue;
                };
                let paragraphs = row
                    .cells
                    .iter()
                    .flat_map(|cell| cell.blocks.iter())
                    .filter_map(|block| match block {
                        BodyElement::Paragraph(paragraph) => {
                            index_of.get(&paragraph.node_id?).copied()
                        }
                        _ => None,
                    })
                    .collect();
                out.push(RowRevision {
                    table: order,
                    row: row_index,
                    mark: mark.clone(),
                    paragraphs,
                });
            }
            order += 1;
        }
        out
    }

    /// Marks row `row` of table `table` inserted or deleted (`Some`), or
    /// takes its mark away (`None`), in the model and the XML.
    pub fn set_table_row_revision(
        &mut self,
        table: usize,
        row: usize,
        mark: Option<RevisionMark>,
    ) -> Result<(), String> {
        let node = {
            let row = self
                .table_mut_by_order(table)
                .ok_or_else(|| format!("table {table} was not found"))?
                .rows
                .get_mut(row)
                .ok_or_else(|| format!("row {row} is out of range"))?;
            row.revision = mark.clone().map(Box::new);
            row.node_id
        };
        let (Some(raw), Some(node)) = (self.raw_document.as_mut(), node) else {
            return Ok(());
        };
        let has_tr_pr = raw
            .element(node)
            .is_some_and(|row| child(row, "trPr").is_some());
        if !has_tr_pr {
            if mark.is_none() {
                return Ok(());
            }
            let tr_pr = raw.create_element("w:trPr", Vec::new(), Vec::new());
            let row = raw.element_mut(node).ok_or("the row went missing")?;
            // After `w:tblPrEx`, which comes first in a row.
            let at = row
                .children
                .iter()
                .position(|node| matches!(node, XmlNode::Element(child) if local(&child.name) == "tblPrEx"))
                .map_or(0, |at| at + 1);
            row.children.insert(at, XmlNode::Element(tr_pr));
        }
        let element = mark.as_ref().map(|mark| {
            let mut attributes = Vec::new();
            for (name, value) in [
                ("w:id", mark.id.as_deref()),
                ("w:author", mark.author.as_deref()),
                ("w:date", mark.date.as_deref()),
            ] {
                if let Some(value) = value {
                    attributes.push(XmlAttribute {
                        name: name.into(),
                        value: value.into(),
                    });
                }
            }
            let name = match mark.kind {
                RevisionKind::Deleted | RevisionKind::MovedFrom => "w:del",
                _ => "w:ins",
            };
            raw.create_element(name, attributes, Vec::new())
        });
        let row = raw.element_mut(node).ok_or("the row went missing")?;
        let Some(XmlNode::Element(tr_pr)) = row
            .children
            .iter_mut()
            .find(|node| matches!(node, XmlNode::Element(child) if local(&child.name) == "trPr"))
        else {
            return Ok(());
        };
        tr_pr.children.retain(
            |node| !matches!(node, XmlNode::Element(child) if matches!(local(&child.name), "ins" | "del")),
        );
        if let Some(element) = element {
            // `w:ins` and `w:del` follow the row's other properties and come
            // before `w:trPrChange`.
            let at = tr_pr
                .children
                .iter()
                .position(|node| matches!(node, XmlNode::Element(child) if local(&child.name) == "trPrChange"))
                .unwrap_or(tr_pr.children.len());
            tr_pr.children.insert(at, XmlNode::Element(element));
        }
        Ok(())
    }

    /// A new tracked mark of `kind` from the open revision session.
    pub(crate) fn new_row_revision(&mut self, kind: RevisionKind) -> Option<RevisionMark> {
        let session = self.revision_session()?.clone();
        Some(RevisionMark {
            kind,
            id: Some(self.allocate_revision_id().to_string()),
            author: session.author.clone(),
            date: session.date.clone(),
            date_utc: None,
            insertion: None,
        })
    }

    /// The row revisions a decision with these filters reaches: the row's
    /// XML node, and whether deciding it removes the row.
    pub(crate) fn rows_to_decide(
        &self,
        accept: bool,
        kind_filter: Option<&RevisionKind>,
        id_filter: Option<RevisionIdFilter<'_>>,
        range: Option<(usize, usize)>,
        targets: Option<&RevisionTargets>,
    ) -> Vec<(usize, usize, bool)> {
        self.table_row_revisions()
            .into_iter()
            .filter(|revision| {
                kind_filter.is_none_or(|kind| kind == &revision.mark.kind)
                    && id_filter.is_none_or(|filter| filter.matches(revision.mark.id.as_deref()))
                    && range.is_none_or(|(start, end)| {
                        revision
                            .paragraphs
                            .iter()
                            .any(|paragraph| (start..=end).contains(paragraph))
                    })
                    && targets.is_none_or(|targets| {
                        targets.names_row(&revision.paragraphs, revision.mark.id.as_deref())
                    })
            })
            .map(|revision| {
                let removes = revision.mark.is_deletion() == accept;
                (revision.table, revision.row, removes)
            })
            .collect()
    }

    /// Decides the row revisions `rows_to_decide` found, last row first so
    /// the others keep their places.
    pub(crate) fn decide_rows(
        &mut self,
        mut rows: Vec<(usize, usize, bool)>,
    ) -> Result<usize, String> {
        rows.sort();
        let count = rows.len();
        for (table, row, removes) in rows.into_iter().rev() {
            if removes {
                self.delete_table_row(table, row)?;
            } else {
                self.set_table_row_revision(table, row, None)?;
            }
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::word_document::review_tools::tests::docx_with_body;
    use futures::executor;

    fn table_document() -> WordDocument {
        let body = r#"<w:p><w:r><w:t>Before</w:t></w:r></w:p>
<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid>
<w:tr><w:tc><w:p><w:r><w:t>one</w:t></w:r></w:p></w:tc></w:tr>
<w:tr><w:trPr><w:ins w:id="5" w:author="Ann" w:date="2026-01-01T00:00:00Z"/></w:trPr><w:tc><w:p><w:r><w:t>two</w:t></w:r></w:p></w:tc></w:tr>
<w:tr><w:trPr><w:del w:id="6" w:author="Bob"/></w:trPr><w:tc><w:p><w:r><w:t>three</w:t></w:r></w:p></w:tc></w:tr>
</w:tbl><w:p/>"#;
        executor::block_on(WordDocument::load(
            &docx_with_body(body),
            &Options::default(),
        ))
        .unwrap()
    }

    fn row_texts(document: &WordDocument) -> Vec<String> {
        document
            .table_by_order(0)
            .unwrap()
            .rows
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .flat_map(|cell| cell.blocks.iter())
                    .filter_map(|block| match block {
                        BodyElement::Paragraph(paragraph) => Some(
                            paragraph
                                .runs
                                .iter()
                                .map(|run| run.text.as_str())
                                .collect::<String>(),
                        ),
                        _ => None,
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn row_revisions_are_listed_with_their_paragraphs() {
        let document = table_document();
        let rows = document.table_row_revisions();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            (rows[0].row, rows[0].mark.kind.clone()),
            (1, RevisionKind::Inserted)
        );
        assert_eq!(rows[0].mark.author.as_deref(), Some("Ann"));
        assert_eq!(
            document.paragraph(rows[0].paragraphs[0]).unwrap().runs[0].text,
            "two"
        );
        assert_eq!(
            (rows[1].row, rows[1].mark.kind.clone()),
            (2, RevisionKind::Deleted)
        );
    }

    #[test]
    fn accepting_all_keeps_the_insertion_and_drops_the_deletion() {
        let mut document = table_document();
        document
            .decide_revisions(true, None, None, None, None, None)
            .unwrap();
        assert_eq!(row_texts(&document), ["one", "two"]);
        assert!(document.table_row_revisions().is_empty());
        let saved = document.export_docx().unwrap();
        let reloaded = executor::block_on(WordDocument::load(&saved, &Options::default())).unwrap();
        assert_eq!(row_texts(&reloaded), ["one", "two"]);
        assert!(reloaded.table_row_revisions().is_empty());
    }

    #[test]
    fn rejecting_one_row_by_its_id_takes_only_that_row_out() {
        let mut document = table_document();
        document
            .decide_revisions(
                false,
                None,
                None,
                None,
                Some(RevisionIdFilter::One("5")),
                None,
            )
            .unwrap();
        assert_eq!(row_texts(&document), ["one", "three"]);
        let left = document.table_row_revisions();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].mark.kind, RevisionKind::Deleted);
        // Rejecting the deletion keeps the row and takes its mark away.
        document
            .decide_revisions(false, None, None, None, None, None)
            .unwrap();
        assert_eq!(row_texts(&document), ["one", "three"]);
        let saved = document.export_docx().unwrap();
        let reloaded = executor::block_on(WordDocument::load(&saved, &Options::default())).unwrap();
        assert!(reloaded.table_row_revisions().is_empty());
        assert_eq!(row_texts(&reloaded), ["one", "three"]);
    }

    #[test]
    fn rows_inserted_or_deleted_with_changes_tracked_are_marked() {
        use crate::commands::table_structure::{TableStructureCommand, TableStructureOp};
        use crate::commands::{CommandContext, CommandExecutor};
        let mut document = table_document();
        document.set_revision_session(Some(RevisionSession {
            author: Some("Dee".into()),
            date: Some("2026-02-03T00:00:00Z".into()),
            date_utc: None,
        }));
        let mut executor = CommandExecutor::new();
        let mut selection = None;
        let mut run = |document: &mut WordDocument, op| {
            let mut ctx = CommandContext::new(document, &mut selection);
            executor
                .execute(Box::new(TableStructureCommand::new(0, op)), &mut ctx)
                .unwrap();
        };
        // Below "one": a new row, an insertion by Dee.
        run(
            &mut document,
            TableStructureOp::InsertRow {
                row_index: 1,
                template_row: Some(0),
            },
        );
        let rows = document.table_row_revisions();
        let new = rows.iter().find(|row| row.row == 1).unwrap();
        assert_eq!(new.mark.kind, RevisionKind::Inserted);
        assert_eq!(new.mark.author.as_deref(), Some("Dee"));
        // "one" deleted: it stays, marked.
        run(&mut document, TableStructureOp::DeleteRow { row_index: 0 });
        assert_eq!(row_texts(&document).len(), 4);
        assert_eq!(
            document.table_row_revisions()[0].mark.kind,
            RevisionKind::Deleted
        );
        // The inserted row deleted again simply goes.
        run(&mut document, TableStructureOp::DeleteRow { row_index: 1 });
        assert_eq!(row_texts(&document), ["one", "two", "three"]);
        // Accepting everything leaves the rows a reader sees.
        document
            .decide_revisions(true, None, None, None, None, None)
            .unwrap();
        assert_eq!(row_texts(&document), ["two"]);
    }

    #[test]
    fn a_row_mark_is_written_into_the_row_properties() {
        let mut document = table_document();
        document
            .set_table_row_revision(
                0,
                0,
                Some(RevisionMark {
                    kind: RevisionKind::Inserted,
                    id: Some("9".into()),
                    author: Some("Cy".into()),
                    date: None,
                    date_utc: None,
                    insertion: None,
                }),
            )
            .unwrap();
        let saved = document.export_docx().unwrap();
        let reloaded = executor::block_on(WordDocument::load(&saved, &Options::default())).unwrap();
        let rows = reloaded.table_row_revisions();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].mark.author.as_deref(), Some("Cy"));
    }
}
