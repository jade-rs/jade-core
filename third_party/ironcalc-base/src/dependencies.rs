//! Optional model-owned value-read tracing. Areas are retained as rectangles,
//! including empty cells, so consumers need not expand whole-axis references.
use crate::{calc_result::CalcResult, expressions::types::CellReferenceIndex, model::Model};
use std::collections::{HashMap, HashSet};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReadArea {
    pub sheet: u32,
    pub first_row: i32,
    pub first_column: i32,
    pub last_row: i32,
    pub last_column: i32,
}
impl ReadArea {
    fn covers(self, other: Self) -> bool {
        self.sheet == other.sheet
            && self.first_row <= other.first_row
            && self.first_column <= other.first_column
            && self.last_row >= other.last_row
            && self.last_column >= other.last_column
    }
    fn is_cell(self) -> bool {
        self.first_row == self.last_row && self.first_column == self.last_column
    }
    fn between(left: CellReferenceIndex, right: CellReferenceIndex) -> Option<Self> {
        (left.sheet == right.sheet).then_some(Self {
            sheet: left.sheet,
            first_row: left.row.min(right.row),
            first_column: left.column.min(right.column),
            last_row: left.row.max(right.row),
            last_column: left.column.max(right.column),
        })
    }
}
#[derive(Clone, Debug, Default)]
pub struct ReadDependencies {
    pub cells: HashMap<CellReferenceIndex, HashSet<ReadArea>>,
    /// A trace that reached its caller's edge budget must never be treated as
    /// complete dependency evidence.
    pub incomplete: bool,
}
pub(crate) struct ReadTrace {
    pub stack: Vec<CellReferenceIndex>,
    pub result: ReadDependencies,
    broad: HashMap<CellReferenceIndex, Vec<ReadArea>>,
    count: usize,
    limit: usize,
}
impl ReadTrace {
    fn new(limit: usize) -> Self {
        Self {
            stack: Vec::new(),
            result: ReadDependencies::default(),
            broad: HashMap::new(),
            count: 0,
            limit,
        }
    }
    pub fn reset(&mut self) {
        self.result = Default::default();
        self.stack.clear();
        self.count = 0;
        self.broad.clear();
    }
    pub fn record(
        &mut self,
        cell: CellReferenceIndex,
        left: CellReferenceIndex,
        right: CellReferenceIndex,
    ) {
        let Some(area) = ReadArea::between(left, right) else {
            self.result.incomplete = true;
            return;
        };
        if self
            .result
            .cells
            .get(&cell)
            .is_some_and(|reads| reads.contains(&area))
        {
            return;
        }
        if self
            .broad
            .get(&cell)
            .is_some_and(|areas| areas.iter().any(|a| a.covers(area)))
        {
            return;
        }
        if !area.is_cell() {
            if let Some(reads) = self.result.cells.get_mut(&cell) {
                let old = reads.len();
                reads.retain(|old| !area.covers(*old));
                self.count -= old - reads.len();
            }
            if let Some(areas) = self.broad.get_mut(&cell) {
                areas.retain(|old| !area.covers(*old));
            }
        }
        if self.count >= self.limit {
            self.result.incomplete = true;
            return;
        }
        if !area.is_cell() {
            self.broad.entry(cell).or_default().push(area);
        }
        self.result.cells.entry(cell).or_default().insert(area);
        self.count += 1;
    }
}
impl Model<'_> {
    /// Enable tracing before evaluation. Ordinary models allocate no trace.
    pub fn trace_value_reads(&mut self, edge_limit: usize) {
        self.read_trace = Some(ReadTrace::new(edge_limit));
    }
    pub fn take_value_reads(&mut self) -> ReadDependencies {
        self.read_trace.take().map(|t| t.result).unwrap_or_default()
    }
    pub(crate) fn trace_range_result(&mut self, cell: CellReferenceIndex, result: &CalcResult) {
        if let (Some(trace), CalcResult::Range { left, right }) = (&mut self.read_trace, result) {
            trace.record(cell, *left, *right);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn at(row: i32, column: i32) -> CellReferenceIndex {
        CellReferenceIndex {
            sheet: 0,
            row,
            column,
        }
    }
    #[test]
    fn records_dynamic_reads_empty_cells_ranges_and_transitive_formula_edges() {
        let mut m = Model::new_empty("trace", "en", "UTC", "en").unwrap();
        m.update_cell_with_number(0, 1, 1, 5.).unwrap();
        m.update_cell_with_formula(0, 1, 2, "=INDIRECT(\"A1\")+SUM(A2:A100)".into())
            .unwrap();
        m.update_cell_with_formula(0, 1, 3, "=B1*2".into()).unwrap();
        m.trace_value_reads(1000);
        m.evaluate();
        let trace = m.take_value_reads();
        assert!(!trace.incomplete);
        assert!(trace.cells[&at(1, 2)].contains(&ReadArea::between(at(1, 1), at(1, 1)).unwrap()));
        assert!(trace.cells[&at(1, 2)].contains(&ReadArea::between(at(2, 1), at(100, 1)).unwrap()));
        assert!(trace.cells[&at(1, 3)].contains(&ReadArea::between(at(1, 2), at(1, 2)).unwrap()));
    }
    #[test]
    fn tracing_is_optional_model_owned_and_bounded_without_changing_values() {
        let mut m = Model::new_empty("trace", "en", "UTC", "en").unwrap();
        m.update_cell_with_formula(0, 1, 1, "=B1+C1".into())
            .unwrap();
        m.trace_value_reads(1);
        m.evaluate();
        let trace = m.take_value_reads();
        assert!(trace.incomplete);
        assert_eq!(trace.cells.values().map(HashSet::len).sum::<usize>(), 1);
        let mut other = Model::new_empty("other", "en", "UTC", "en").unwrap();
        other.evaluate();
        assert!(other.take_value_reads().cells.is_empty());
        m.evaluate();
        assert!(m.take_value_reads().cells.is_empty());
        assert_eq!(
            m.get_cell_value_by_index(0, 1, 1).unwrap(),
            crate::cell::CellValue::Number(0.)
        );
    }
}

#[cfg(test)]
mod compression_tests {
    use super::*;
    fn at(row: i32, column: i32) -> CellReferenceIndex {
        CellReferenceIndex {
            sheet: 0,
            row,
            column,
        }
    }
    #[test]
    fn contained_cell_reads_do_not_expand_a_whole_axis_dependency() {
        let mut trace = ReadTrace::new(2);
        trace.record(at(1, 2), at(1, 1), at(1_048_576, 1));
        for row in 1..=1000 {
            trace.record(at(1, 2), at(row, 1), at(row, 1));
        }
        assert!(!trace.result.incomplete);
        assert_eq!(trace.result.cells[&at(1, 2)].len(), 1);
    }
    #[test]
    fn repeated_evaluation_resets_the_trace_budget_and_ignores_unread_branches() {
        let mut m = Model::new_empty("trace", "en", "UTC", "en").unwrap();
        m.update_cell_with_formula(0, 1, 1, "=IF(TRUE,B1,C1)".into())
            .unwrap();
        m.trace_value_reads(1);
        m.evaluate();
        m.evaluate();
        let reads = m.take_value_reads();
        assert!(!reads.incomplete);
        assert!(reads.cells[&at(1, 1)].contains(&ReadArea::between(at(1, 2), at(1, 2)).unwrap()));
        assert!(!reads.cells[&at(1, 1)].contains(&ReadArea::between(at(1, 3), at(1, 3)).unwrap()));
    }
}
