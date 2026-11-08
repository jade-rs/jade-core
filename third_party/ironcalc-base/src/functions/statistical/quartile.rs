use crate::expressions::types::CellReferenceIndex;
use crate::{calc_result::CalcResult, expressions::parser::Node, model::Model};

impl<'a> Model<'a> {
    pub(crate) fn fn_quartile_inc(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        self.percentile_arguments(args, cell, false, true)
    }

    pub(crate) fn fn_quartile_exc(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        self.percentile_arguments(args, cell, true, true)
    }
}
