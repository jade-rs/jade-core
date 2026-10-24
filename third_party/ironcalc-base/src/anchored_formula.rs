//! Formulas evaluated at many cells from one parse, as conditional formats
//! evaluate theirs (jade addition).

use crate::{
    calc_result::{CalcResult, Range},
    expressions::{
        parser::{ArrayNode, Node},
        types::{CellReferenceIndex, CellReferenceRC},
    },
    implicit_intersection::implicit_intersection,
    model::Model,
};

/// A formula parsed against one anchor cell, as a conditional-formatting
/// rule's formula is: evaluated at another cell, its relative references
/// move along by the same offset.
pub struct AnchoredFormula(Node);

impl Model<'_> {
    /// Parses `formula` (English, with or without `=`) relative to the cell at
    /// (`sheet`, `row`, `column`), 1-based. `None` when it does not parse.
    pub fn parse_anchored_formula(
        &mut self,
        formula: &str,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Option<AnchoredFormula> {
        let body = formula.trim();
        let body = body.strip_prefix('=').unwrap_or(body);
        if body.is_empty() {
            return None;
        }
        let sheet_name = self.workbook.worksheets.get(sheet as usize)?.get_name();
        let context = CellReferenceRC {
            sheet: sheet_name,
            row,
            column,
        };
        let node = self.parse_internal_formula(body, &context);
        if matches!(node, Node::ParseErrorKind { .. }) {
            return None;
        }
        Some(AnchoredFormula(node))
    }

    /// Whether `formula` holds at (`sheet`, `row`, `column`) as Excel decides
    /// a conditional format's formula: a non-zero number or TRUE. Text,
    /// blanks and errors do not.
    pub fn anchored_formula_holds(
        &mut self,
        formula: &AnchoredFormula,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> bool {
        let cell = CellReferenceIndex { sheet, row, column };
        let mut result = self.evaluate_node_in_context(&formula.0, cell);
        if let CalcResult::Range { left, right } = &result {
            let at = if left == right {
                Some(*left)
            } else {
                implicit_intersection(
                    &cell,
                    &Range {
                        left: *left,
                        right: *right,
                    },
                )
            };
            result = match at {
                Some(at) => self.evaluate_cell(at),
                None => return false,
            };
        }
        match result {
            CalcResult::Number(number) => number != 0.0 && number.is_finite(),
            CalcResult::Boolean(flag) => flag,
            CalcResult::Array(rows) => match rows.first().and_then(|row| row.first()) {
                Some(ArrayNode::Boolean(flag)) => *flag,
                Some(ArrayNode::Number(number)) => *number != 0.0,
                _ => false,
            },
            _ => false,
        }
    }
}
