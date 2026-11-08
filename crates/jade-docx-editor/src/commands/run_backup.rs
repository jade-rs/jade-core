use crate::document::Run;
use crate::docx_xml::{DocxXmlDocument, XmlElement, XmlNode, XmlNodeId};

use super::{CommandContext, CommandError, CommandResult};

#[derive(Clone)]
pub(crate) struct ParagraphRunsBackup {
    pub paragraph_index: usize,
    pub runs: Vec<Run>,
    run_properties_xml: Vec<(XmlNodeId, Option<XmlElement>)>,
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
        let run_properties_xml = if ctx.document.is_note_paragraph_surface(paragraph_index) {
            ctx.document
                .note_paragraph_xml(paragraph_index)
                .map(|(_, raw, _)| capture_run_properties_xml(&raw, &paragraph.runs))
                .unwrap_or_default()
        } else {
            ctx.document
                .raw_document
                .as_ref()
                .map(|raw| capture_run_properties_xml(raw, &paragraph.runs))
                .unwrap_or_default()
        };
        backups.push(ParagraphRunsBackup {
            paragraph_index,
            runs: paragraph.runs.clone(),
            run_properties_xml,
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
        // The model normalizes some authored properties (for example, a font
        // name shadowed by its theme). Undo must restore that source XML too.
        if ctx
            .document
            .is_note_paragraph_surface(backup.paragraph_index)
        {
            let _ = ctx
                .document
                .update_note_paragraph_xml(backup.paragraph_index, |raw, _| {
                    restore_run_properties_xml(raw, &backup.run_properties_xml);
                });
        } else if let Some(raw) = ctx.document.raw_document.as_mut() {
            restore_run_properties_xml(raw, &backup.run_properties_xml);
        }
    }
    Ok(())
}

fn capture_run_properties_xml(
    raw: &DocxXmlDocument,
    runs: &[Run],
) -> Vec<(XmlNodeId, Option<XmlElement>)> {
    runs.iter()
        .filter_map(|run| {
            let id = run.node_id?;
            let element = raw.element(id)?;
            let properties = element.children.iter().find_map(|child| match child {
                XmlNode::Element(element) if element.name == "w:rPr" => Some(element.clone()),
                _ => None,
            });
            Some((id, properties))
        })
        .collect()
}

fn restore_run_properties_xml(
    raw: &mut DocxXmlDocument,
    snapshots: &[(XmlNodeId, Option<XmlElement>)],
) {
    for (id, properties) in snapshots {
        let Some(run) = raw.element_mut(*id) else {
            continue;
        };
        run.children
            .retain(|child| !matches!(child, XmlNode::Element(element) if element.name == "w:rPr"));
        if let Some(properties) = properties {
            run.children.insert(0, XmlNode::Element(properties.clone()));
        }
        raw.rebuild_index_for_subtree(*id);
    }
}
