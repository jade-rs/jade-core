use crate::{
    calc_result::CalcResult,
    expressions::{parser::Node, types::CellReferenceIndex},
    model::Model,
};

impl Model<'_> {
    pub(crate) fn fn_compat_operator(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        spill: bool,
    ) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        let child = Box::new(args[0].clone());
        let node = if spill {
            Node::SpillRangeOperator { child }
        } else {
            Node::ImplicitIntersection {
                automatic: false,
                child,
            }
        };
        self.evaluate_node_in_context(&node, cell)
    }
}

#[cfg(test)]
mod tests {
    use crate::test::util::new_empty_model;

    #[test]
    fn a_spill_reference_evaluates_a_later_anchor_before_looking_up_its_extent() {
        let mut model = new_empty_model();
        model._set("A1", "=SUM(_xlfn.ANCHORARRAY(Z10))");
        model._set("Z10", "=SEQUENCE(3)");
        model.evaluate();
        assert_eq!(model._get_text("A1"), "6");
    }
}
