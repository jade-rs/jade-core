use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::RunChart;
use crate::word_document::{FloatingObjectTarget, WordDocumentSnapshot};

pub struct UpdateChartCommand {
    target: FloatingObjectTarget,
    chart: RunChart,
    snapshot: Option<WordDocumentSnapshot>,
}

impl UpdateChartCommand {
    pub fn new(target: FloatingObjectTarget, chart: RunChart) -> Self {
        Self {
            target,
            chart,
            snapshot: None,
        }
    }
}

impl Command for UpdateChartCommand {
    fn name(&self) -> &'static str {
        "update_chart"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or_else(|| CommandError::Custom("document is unavailable".into()))?;
        if let Err(error) = ctx
            .document
            .update_chart_at(self.target, self.chart.clone())
        {
            ctx.document.restore_snapshot(snapshot);
            return Err(CommandError::Custom(error));
        }
        self.snapshot = Some(snapshot);
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self
            .snapshot
            .take()
            .ok_or_else(|| CommandError::Custom("missing chart backup for undo".into()))?;
        ctx.document.restore_snapshot(snapshot);
        Ok(())
    }
}
