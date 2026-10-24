//! A text selection covering every paragraph of a table deletes its structure,
//! rather than leaving an empty grid. Partial-cell selections keep their cells.

use super::delete_table::DeleteTableCommand;
use super::insert_text::InsertTextCommand;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::docx_xml::XmlNodeId;
use crate::word_document::{WordDocument, WordDocumentSnapshot};
use jade_docx_layout::selection::{SelectionRange, TextPosition};
use std::collections::{HashMap, HashSet};

fn len(document: &WordDocument, paragraph: usize) -> usize {
    document
        .paragraph(paragraph)
        .map_or(0, |p| p.runs.iter().map(|run| run.text.len()).sum())
}

fn index_of(document: &WordDocument, id: XmlNodeId) -> Option<usize> {
    (0..document.paragraph_count())
        .find(|index| document.paragraph(*index).and_then(|p| p.node_id) == Some(id))
}

// The accepted-view coordinates used by server clients stop before trailing
// deletions and start after leading deletions. Those invisible bytes do not
// make a fully selected cell partial. Insertions and moved-to text still do.
fn only_deleted_text(document: &WordDocument, paragraph: usize, from: usize, to: usize) -> bool {
    let Some(paragraph) = document.paragraph(paragraph) else {
        return false;
    };
    let mut offset = 0;
    for run in &paragraph.runs {
        let end = offset + run.text.len();
        if offset < to
            && end > from
            && !run.revision.as_ref().is_some_and(|mark| mark.is_deletion())
        {
            return false;
        }
        offset = end;
    }
    to <= offset
}

pub(super) fn delete_range_with_tables(
    ctx: &mut CommandContext<'_>,
    start: TextPosition,
    end: TextPosition,
) -> CommandResult<Option<WordDocumentSnapshot>> {
    if start.paragraph == end.paragraph && start.offset == end.offset {
        return Ok(None);
    }
    let mut tables = Vec::new();
    let mut covered = HashSet::new();
    for order in 0.. {
        let Some(table) = ctx.document.table_by_order(order) else {
            break;
        };
        let Some(id) = table.node_id else { continue };
        let paragraphs = ctx.document.table_paragraphs(order);
        if paragraphs.is_empty() || paragraphs.iter().any(|p| covered.contains(p)) {
            continue;
        }
        if paragraphs.iter().all(|p| {
            *p >= start.paragraph
                && *p <= end.paragraph
                && (*p != start.paragraph || only_deleted_text(ctx.document, *p, 0, start.offset))
                // The range end is exclusive. Reaching an empty cell at offset
                // zero does not select that cell or its enclosing table.
                && (*p != end.paragraph
                    || (end.offset > 0
                        && only_deleted_text(ctx.document, *p, end.offset, len(ctx.document, *p))))
        }) {
            covered.extend(paragraphs);
            tables.push(id);
        }
    }
    if tables.is_empty() {
        return Ok(None);
    }
    // Stable raw IDs survive removal of preceding tables and paragraphs.
    let outside: Vec<_> = (start.paragraph..=end.paragraph)
        .filter(|p| !covered.contains(p))
        .filter_map(|p| {
            Some((
                ctx.document.paragraph(p)?.node_id?,
                if p == start.paragraph {
                    start.offset
                } else {
                    0
                },
                if p == end.paragraph {
                    end.offset
                } else {
                    len(ctx.document, p)
                },
            ))
        })
        .collect();
    let following = covered
        .contains(&end.paragraph)
        .then(|| {
            ctx.document
                .paragraph(end.paragraph + 1)
                .and_then(|p| p.node_id)
        })
        .flatten();
    let tracking = ctx.document.revision_session().is_some();
    let selected_tables = tables.iter().copied().collect();
    let table_boundaries: HashMap<_, _> = outside
        .iter()
        .filter_map(|(id, _, to)| {
            let index = index_of(ctx.document, *id)?;
            if *to < len(ctx.document, index) {
                return None;
            }
            let next = ctx
                .document
                .paragraph_after_removed_tables(index, &selected_tables)?;
            Some((*id, ctx.document.paragraph(next)?.node_id?))
        })
        .collect();
    let snapshot = ctx
        .document
        .snapshot_including_package()
        .ok_or(CommandError::DocumentUnavailable)?;
    let previous_selection = *ctx.selection;
    let result = (|| -> CommandResult<()> {
        for id in tables.into_iter().rev() {
            let order = (0..)
                .take_while(|order| ctx.document.table_by_order(*order).is_some())
                .find(|order| {
                    ctx.document.table_by_order(*order).and_then(|t| t.node_id) == Some(id)
                })
                .ok_or(CommandError::DocumentUnavailable)?;
            DeleteTableCommand::new(order).apply(ctx)?;
        }
        let mut groups: Vec<Vec<(XmlNodeId, usize, usize)>> = Vec::new();
        for item in outside {
            let index = index_of(ctx.document, item.0).ok_or(CommandError::ParagraphUnavailable)?;
            let joins = groups
                .last()
                .and_then(|group| group.last())
                .and_then(|last| index_of(ctx.document, last.0))
                .is_some_and(|last| {
                    (last + 1..=index).all(|p| ctx.document.paragraphs_share_a_container(last, p))
                });
            if joins {
                groups.last_mut().unwrap().push(item);
            } else {
                groups.push(vec![item]);
            }
        }
        for group in groups.into_iter().rev() {
            let first = group.first().unwrap();
            let last = group.last().unwrap();
            let from = TextPosition::new(index_of(ctx.document, first.0).unwrap(), first.1);
            let mut to = TextPosition::new(index_of(ctx.document, last.0).unwrap(), last.2);
            // After physical table removal the selected heading's paragraph
            // mark can join the following body paragraph in the normal path.
            if !tracking {
                if let Some(next) = following.and_then(|id| index_of(ctx.document, id)) {
                    if to.paragraph + 1 == next
                        && ctx
                            .document
                            .paragraphs_share_a_container(to.paragraph, next)
                    {
                        to = TextPosition::new(next, 0);
                    }
                }
            }
            let range = SelectionRange {
                anchor: ctx.document.model_to_rendered_text_position(from),
                focus: ctx.document.model_to_rendered_text_position(to),
            };
            InsertTextCommand::new(range, String::new(), None).apply(ctx)?;
            if tracking {
                if let Some(index) = group
                    .iter()
                    .rev()
                    .find_map(|item| index_of(ctx.document, item.0))
                {
                    // This group's boundary is selected only when the range
                    // continues into one of the tables covered above.
                    if let Some(next) = table_boundaries
                        .get(&last.0)
                        .and_then(|id| index_of(ctx.document, *id))
                    {
                        if next == index + 1
                            && ctx.document.paragraphs_share_a_container(index, next)
                        {
                            // Cancelling an inserted table removes it now;
                            // its heading boundary is still a tracked edit.
                            ctx.document.mark_deleted_paragraph_break(index)
                        } else {
                            ctx.document
                                .mark_deleted_paragraph_break_before_tables(index, &selected_tables)
                        }
                        .map_err(CommandError::Custom)?;
                    }
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = previous_selection;
        return Err(error);
    }
    Ok(Some(snapshot))
}
