use crate::document::Run;
use crate::docx_xml::XmlNodeId;

use super::{CommandContext, CommandError, CommandResult};

#[derive(Clone)]
pub(crate) struct ParagraphRunsBackup {
    pub paragraph_index: usize,
    pub runs: Vec<Run>,
}

pub(crate) fn capture_paragraph_runs(
    ctx: &mut CommandContext<'_>,
    start: usize,
    end: usize,
) -> CommandResult<Vec<ParagraphRunsBackup>> {
    if start > end {
        return Err(CommandError::InvalidSelection);
    }

    let paragraph_count = ctx.document.paragraph_count();
    if paragraph_count == 0 || start >= paragraph_count || end >= paragraph_count {
        return Err(CommandError::ParagraphUnavailable);
    }

    let mut backups = Vec::with_capacity(end.saturating_sub(start) + 1);
    for paragraph_index in start..=end {
        let paragraph = ctx
            .document
            .paragraph(paragraph_index)
            .ok_or(CommandError::ParagraphUnavailable)?;
        backups.push(ParagraphRunsBackup {
            paragraph_index,
            runs: paragraph.runs.clone(),
        });
    }
    Ok(backups)
}

pub(crate) fn restore_paragraph_runs(
    ctx: &mut CommandContext<'_>,
    backups: &[ParagraphRunsBackup],
) -> CommandResult<()> {
    for backup in backups {
        let paragraph = ctx
            .document
            .paragraph_mut(backup.paragraph_index)
            .ok_or(CommandError::ParagraphUnavailable)?;
        // Formatting edits keep run ids, so a restored run can land on an
        // element whose `w:rPr` the edit rewrote; rebuild those. Drawings are
        // not changed by these edits and keep their source XML.
        let restyled: Vec<XmlNodeId> = backup
            .runs
            .iter()
            .filter_map(|restored| {
                let id = restored.node_id?;
                let current = paragraph
                    .runs
                    .iter()
                    .find(|current| current.node_id == Some(id))?;
                (current.properties != restored.properties).then_some(id)
            })
            .collect();
        paragraph.runs = backup.runs.clone();
        ctx.document
            .sync_paragraph_run_properties(backup.paragraph_index, &restyled);
    }
    Ok(())
}
