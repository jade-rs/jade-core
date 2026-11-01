use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use crate::{
    calc_result::CalcResult,
    expressions::{
        parser::{ArrayNode, Node},
        token::Error,
        types::CellReferenceIndex,
    },
    model::Model,
};

use super::util::compare_values;

/// Compare two sort keys following Excel's rules:
///   Numbers < Strings < Booleans < Errors < Empty cells
/// Empty cells always sort last regardless of ascending/descending.
fn sort_key_cmp(
    a: &ArrayNode,
    b: &ArrayNode,
    ascending: bool,
    cell: CellReferenceIndex,
) -> Ordering {
    let a_empty = matches!(a, ArrayNode::Empty);
    let b_empty = matches!(b, ArrayNode::Empty);
    match (a_empty, b_empty) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => {
            let key_a = array_node_to_calc_result(a, cell);
            let key_b = array_node_to_calc_result(b, cell);
            let cmp = compare_values(&key_a, &key_b);
            if ascending {
                cmp.cmp(&0)
            } else {
                0.cmp(&cmp)
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn array_node_to_calc_result(node: &ArrayNode, cell: CellReferenceIndex) -> CalcResult {
    match node {
        ArrayNode::Number(n) => CalcResult::Number(*n),
        ArrayNode::Boolean(b) => CalcResult::Boolean(*b),
        ArrayNode::String(s) => CalcResult::String(s.clone()),
        ArrayNode::Error(e) => CalcResult::Error {
            error: e.clone(),
            origin: cell,
            message: "".to_string(),
        },
        ArrayNode::Empty => CalcResult::EmptyCell,
    }
}

fn calc_result_to_array_node(result: CalcResult) -> ArrayNode {
    match result {
        CalcResult::Number(n) => ArrayNode::Number(n),
        CalcResult::Boolean(b) => ArrayNode::Boolean(b),
        CalcResult::String(s) => ArrayNode::String(s),
        CalcResult::Error { error, .. } => ArrayNode::Error(error),
        CalcResult::EmptyCell | CalcResult::EmptyArg => ArrayNode::Empty,
        _ => ArrayNode::Error(Error::VALUE),
    }
}

fn array_node_is_truthy(node: &ArrayNode) -> bool {
    match node {
        ArrayNode::Boolean(b) => *b,
        ArrayNode::Number(n) => *n != 0.0,
        _ => false,
    }
}

/// A hashable, equality-normalised representation of a single cell value.
/// Strings are uppercased (matching case-insensitive array_nodes_equal).
/// Numbers use bit-level identity (f64::to_bits), which is exact for all
/// values that actually appear in spreadsheet cells.
#[derive(Hash, Eq, PartialEq)]
enum CellKey {
    Number(u64),
    Boolean(bool),
    Str(String),
    Error(u8),
    Empty,
}

fn cell_key(node: &ArrayNode) -> CellKey {
    match node {
        ArrayNode::Number(n) => CellKey::Number(n.to_bits()),
        ArrayNode::Boolean(b) => CellKey::Boolean(*b),
        ArrayNode::String(s) => CellKey::Str(s.to_uppercase()),
        ArrayNode::Error(e) => CellKey::Error(error_discriminant(e)),
        ArrayNode::Empty => CellKey::Empty,
    }
}

fn error_discriminant(e: &crate::expressions::token::Error) -> u8 {
    use crate::expressions::token::Error;
    match e {
        Error::REF => 0,
        Error::NAME => 1,
        Error::VALUE => 2,
        Error::DIV => 3,
        Error::NA => 4,
        Error::NUM => 5,
        Error::ERROR => 6,
        Error::NIMPL => 7,
        Error::SPILL => 8,
        Error::CALC => 9,
        Error::CIRC => 10,
        Error::NULL => 11,
    }
}

fn row_key(row: &[ArrayNode]) -> Vec<CellKey> {
    row.iter().map(cell_key).collect()
}

fn col_key(data: &[Vec<ArrayNode>], j: usize) -> Vec<CellKey> {
    data.iter().map(|row| cell_key(&row[j])).collect()
}

// Extract a 1-D column key from a 2-D by_array.
// Accepts column vectors (N×1), row vectors (1×N), or the first column of a multi-column array.
fn extract_key_column(data: &[Vec<ArrayNode>], expected_len: usize) -> Option<Vec<ArrayNode>> {
    if data.is_empty() {
        return None;
    }
    if data.len() == expected_len && data[0].len() == 1 {
        Some(data.iter().map(|row| row[0].clone()).collect())
    } else if data.len() == 1 && data[0].len() == expected_len {
        Some(data[0].clone())
    } else if data.len() == expected_len {
        Some(data.iter().map(|row| row[0].clone()).collect())
    } else {
        None
    }
}

impl<'a> Model<'a> {
    /// Evaluate a node and convert the result to a 2-D array of ArrayNodes.
    /// Handles Range references, inline Arrays, and scalar values.
    pub(crate) fn eval_to_array(
        &mut self,
        node: &Node,
        cell: CellReferenceIndex,
    ) -> Result<Vec<Vec<ArrayNode>>, CalcResult> {
        let result = self.evaluate_node_in_context(node, cell);
        match result {
            CalcResult::Range { left, right } => Ok(self.evaluate_range(left, right)),
            CalcResult::Array(arr) => Ok(arr),
            CalcResult::Number(n) => Ok(vec![vec![ArrayNode::Number(n)]]),
            CalcResult::Boolean(b) => Ok(vec![vec![ArrayNode::Boolean(b)]]),
            CalcResult::String(s) => Ok(vec![vec![ArrayNode::String(s)]]),
            CalcResult::EmptyCell | CalcResult::EmptyArg => Ok(vec![vec![ArrayNode::Number(0.0)]]),
            err @ CalcResult::Error { .. } => Err(err),
            CalcResult::Lambda(_) => Err(CalcResult::new_error(
                Error::VALUE,
                cell,
                "LAMBDA cannot be used as an array".to_string(),
            )),
        }
    }

    // ── SORT ──────────────────────────────────────────────────────────────────

    /// `=SORT(array, [sort_index], [sort_order], [by_col])`
    ///
    /// Returns the array sorted by the given column (or row when by_col=TRUE).
    ///   * sort_index – 1-based column/row index to sort by (default 1)
    ///   * sort_order – 1 = ascending, -1 = descending (default 1)
    ///   * by_col     – FALSE = sort rows, TRUE = sort columns (default FALSE)
    pub(crate) fn fn_sort(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.is_empty() || args.len() > 4 {
            return CalcResult::new_args_number_error(cell);
        }

        let data = match self.eval_to_array(&args[0], cell) {
            Ok(d) => d,
            Err(e) => return e,
        };

        if data.is_empty() || data[0].is_empty() {
            return CalcResult::Array(vec![]);
        }

        let num_rows = data.len();
        let num_cols = data[0].len();

        let sort_index: usize = if args.len() >= 2 {
            match self.get_number(&args[1], cell) {
                Ok(n) => {
                    let n = n as i64;
                    if n < 1 {
                        return CalcResult::new_error(
                            Error::VALUE,
                            cell,
                            "sort_index must be >= 1".to_string(),
                        );
                    }
                    n as usize
                }
                Err(e) => return e,
            }
        } else {
            1
        };

        let ascending: bool = if args.len() >= 3 {
            match self.get_number(&args[2], cell) {
                Ok(n) => match n as i32 {
                    1 => true,
                    -1 => false,
                    _ => {
                        return CalcResult::new_error(
                            Error::VALUE,
                            cell,
                            "sort_order must be 1 or -1".to_string(),
                        )
                    }
                },
                Err(e) => return e,
            }
        } else {
            true
        };

        let by_col: bool = if args.len() >= 4 {
            match self.get_boolean(&args[3], cell) {
                Ok(b) => b,
                Err(e) => return e,
            }
        } else {
            false
        };

        if !by_col {
            // Sort rows by column sort_index
            if sort_index > num_cols {
                return CalcResult::new_error(
                    Error::VALUE,
                    cell,
                    "sort_index exceeds array width".to_string(),
                );
            }
            let key_col = sort_index - 1;

            let mut row_indices: Vec<usize> = (0..num_rows).collect();
            row_indices.sort_by(|&a, &b| {
                sort_key_cmp(&data[a][key_col], &data[b][key_col], ascending, cell)
            });

            CalcResult::Array(row_indices.iter().map(|&r| data[r].clone()).collect())
        } else {
            // Sort columns by row sort_index
            if sort_index > num_rows {
                return CalcResult::new_error(
                    Error::VALUE,
                    cell,
                    "sort_index exceeds array height".to_string(),
                );
            }
            let key_row = sort_index - 1;

            let mut col_indices: Vec<usize> = (0..num_cols).collect();
            col_indices.sort_by(|&a, &b| {
                sort_key_cmp(&data[key_row][a], &data[key_row][b], ascending, cell)
            });

            CalcResult::Array(
                data.iter()
                    .map(|row| col_indices.iter().map(|&c| row[c].clone()).collect())
                    .collect(),
            )
        }
    }

    // ── SORTBY ────────────────────────────────────────────────────────────────

    /// `=SORTBY(array, by_array1, [sort_order1], [by_array2, sort_order2], ...)`
    ///
    /// Sorts array rows using one or more external key arrays.
    ///   * sort_order values: 1 = ascending (default), -1 = descending
    ///   * by_array must have the same number of rows as array
    ///   * Valid argument counts: 2, 3, 5, 7, … (additional keys in pairs)
    pub(crate) fn fn_sortby(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let n = args.len();
        // Valid: 2, 3, 5, 7, ... (n==2 || n==3 || n>=5 with odd count)
        if n < 2 || (n > 3 && n.is_multiple_of(2)) {
            return CalcResult::new_args_number_error(cell);
        }

        let data = match self.eval_to_array(&args[0], cell) {
            Ok(d) => d,
            Err(e) => return e,
        };

        if data.is_empty() || data[0].is_empty() {
            return CalcResult::Array(vec![]);
        }

        let num_rows = data.len();

        // Collect sort keys: (flat key column, ascending)
        let mut sort_keys: Vec<(Vec<ArrayNode>, bool)> = Vec::new();

        // First key at args[1], optional order at args[2]
        let key1 = match self.eval_to_array(&args[1], cell) {
            Ok(d) => d,
            Err(e) => return e,
        };
        let key1_flat = match extract_key_column(&key1, num_rows) {
            Some(k) => k,
            None => {
                return CalcResult::new_error(
                    Error::VALUE,
                    cell,
                    "by_array1 must have the same number of rows as array".to_string(),
                )
            }
        };
        let order1 = if n >= 3 {
            match self.get_number(&args[2], cell) {
                Ok(v) => match v as i32 {
                    1 => true,
                    -1 => false,
                    _ => {
                        return CalcResult::new_error(
                            Error::VALUE,
                            cell,
                            "sort_order must be 1 or -1".to_string(),
                        )
                    }
                },
                Err(e) => return e,
            }
        } else {
            true
        };
        sort_keys.push((key1_flat, order1));

        // Additional key/order pairs starting at args[3]
        let mut i = 3;
        while i < n {
            let key = match self.eval_to_array(&args[i], cell) {
                Ok(d) => d,
                Err(e) => return e,
            };
            let key_flat = match extract_key_column(&key, num_rows) {
                Some(k) => k,
                None => {
                    return CalcResult::new_error(
                        Error::VALUE,
                        cell,
                        "by_array must have the same number of rows as array".to_string(),
                    )
                }
            };
            let ascending = if i + 1 < n {
                match self.get_number(&args[i + 1], cell) {
                    Ok(v) => match v as i32 {
                        1 => true,
                        -1 => false,
                        _ => {
                            return CalcResult::new_error(
                                Error::VALUE,
                                cell,
                                "sort_order must be 1 or -1".to_string(),
                            )
                        }
                    },
                    Err(e) => return e,
                }
            } else {
                true
            };
            sort_keys.push((key_flat, ascending));
            i += 2;
        }

        let mut row_indices: Vec<usize> = (0..num_rows).collect();
        row_indices.sort_by(|&a, &b| {
            for (keys, ascending) in &sort_keys {
                let ord = sort_key_cmp(&keys[a], &keys[b], *ascending, cell);
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            Ordering::Equal
        });

        CalcResult::Array(row_indices.iter().map(|&r| data[r].clone()).collect())
    }

    // ── UNIQUE ────────────────────────────────────────────────────────────────

    /// `=UNIQUE(array, [by_col], [exactly_once])`
    ///
    /// Returns the unique rows (or columns) from array.
    ///   * by_col       – FALSE = deduplicate rows, TRUE = deduplicate columns (default FALSE)
    ///   * exactly_once – FALSE = return one copy of every distinct value (default)
    ///     TRUE  = return only rows/columns that appear exactly once
    pub(crate) fn fn_unique(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.is_empty() || args.len() > 3 {
            return CalcResult::new_args_number_error(cell);
        }

        let data = match self.eval_to_array(&args[0], cell) {
            Ok(d) => d,
            Err(e) => return e,
        };

        if data.is_empty() || data[0].is_empty() {
            return CalcResult::Array(vec![]);
        }

        let by_col: bool = if args.len() >= 2 {
            match self.get_boolean(&args[1], cell) {
                Ok(b) => b,
                Err(e) => return e,
            }
        } else {
            false
        };

        let exactly_once: bool = if args.len() >= 3 {
            match self.get_boolean(&args[2], cell) {
                Ok(b) => b,
                Err(e) => return e,
            }
        } else {
            false
        };

        if !by_col {
            let num_rows = data.len();

            // First pass: count how many times each row key appears.
            let mut counts: HashMap<Vec<CellKey>, usize> = HashMap::with_capacity(num_rows);
            for row in &data {
                *counts.entry(row_key(row)).or_insert(0) += 1;
            }

            // Second pass: collect rows in original order.
            let result: Vec<Vec<ArrayNode>> = if exactly_once {
                data.into_iter()
                    .filter(|row| counts[&row_key(row)] == 1)
                    .collect()
            } else {
                let mut seen: HashSet<Vec<CellKey>> = HashSet::with_capacity(num_rows);
                data.into_iter()
                    .filter(|row| seen.insert(row_key(row)))
                    .collect()
            };

            if result.is_empty() {
                return CalcResult::new_error(
                    Error::CALC,
                    cell,
                    "UNIQUE found no matching rows".to_string(),
                );
            }

            CalcResult::Array(result)
        } else {
            let num_cols = data[0].len();

            // First pass: count how many times each column key appears.
            let mut counts: HashMap<Vec<CellKey>, usize> = HashMap::with_capacity(num_cols);
            for j in 0..num_cols {
                *counts.entry(col_key(&data, j)).or_insert(0) += 1;
            }

            // Second pass: collect column indices in original order.
            let result_cols: Vec<usize> = if exactly_once {
                (0..num_cols)
                    .filter(|&j| counts[&col_key(&data, j)] == 1)
                    .collect()
            } else {
                let mut seen: HashSet<Vec<CellKey>> = HashSet::with_capacity(num_cols);
                (0..num_cols)
                    .filter(|&j| seen.insert(col_key(&data, j)))
                    .collect()
            };

            if result_cols.is_empty() {
                return CalcResult::new_error(
                    Error::CALC,
                    cell,
                    "UNIQUE found no matching columns".to_string(),
                );
            }

            CalcResult::Array(
                data.iter()
                    .map(|row| result_cols.iter().map(|&c| row[c].clone()).collect())
                    .collect(),
            )
        }
    }

    // ── FILTER ────────────────────────────────────────────────────────────────

    /// `=FILTER(array, include, [if_empty])`
    ///
    /// Returns only the rows of array where the corresponding include value is truthy.
    ///   * include  – a column vector of booleans/numbers with the same height as array
    ///   * if_empty – value returned when no rows pass the filter (default: #CALC! error)
    ///
    /// Note: include must be a range reference or inline array containing boolean/numeric
    /// values. For comparison-based filtering (e.g. A1:A5>3) store the comparison result
    /// in a helper column first.
    pub(crate) fn fn_filter(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() < 2 || args.len() > 3 {
            return CalcResult::new_args_number_error(cell);
        }

        let data = match self.eval_to_array(&args[0], cell) {
            Ok(d) => d,
            Err(e) => return e,
        };

        if data.is_empty() || data[0].is_empty() {
            return CalcResult::Array(vec![]);
        }

        let num_rows = data.len();
        let num_cols = data[0].len();

        let include = match self.eval_to_array(&args[1], cell) {
            Ok(d) => d,
            Err(e) => return e,
        };

        // Strings and errors are invalid as include mask elements.
        // Strings → #VALUE!; errors → propagate the error.
        for row in &include {
            for node in row {
                match node {
                    ArrayNode::Boolean(_) | ArrayNode::Number(_) | ArrayNode::Empty => {}
                    ArrayNode::String(_) => {
                        return CalcResult::new_error(
                            Error::VALUE,
                            cell,
                            "include array must contain numbers or booleans".to_string(),
                        );
                    }
                    ArrayNode::Error(e) => {
                        return CalcResult::new_error(
                            e.clone(),
                            cell,
                            "include array contains an error".to_string(),
                        );
                    }
                }
            }
        }

        // Determine whether we are filtering rows or columns
        let include_flat: Vec<bool>;
        let filter_rows: bool;

        if include.len() == num_rows && include[0].len() == 1 {
            // Column vector — filter rows
            include_flat = include
                .iter()
                .map(|row| array_node_is_truthy(&row[0]))
                .collect();
            filter_rows = true;
        } else if include.len() == 1 && include[0].len() == num_rows {
            // Row vector with length == num_rows — also filter rows
            include_flat = include[0].iter().map(array_node_is_truthy).collect();
            filter_rows = true;
        } else if include.len() == 1 && include[0].len() == num_cols {
            // Row vector — filter columns
            include_flat = include[0].iter().map(array_node_is_truthy).collect();
            filter_rows = false;
        } else if include.len() == num_cols && include[0].len() == 1 {
            // Column vector with length == num_cols — filter columns
            include_flat = include
                .iter()
                .map(|row| array_node_is_truthy(&row[0]))
                .collect();
            filter_rows = false;
        } else {
            return CalcResult::new_error(
                Error::VALUE,
                cell,
                "include array dimensions must match array rows or columns".to_string(),
            );
        }

        let filtered: Vec<Vec<ArrayNode>> = if filter_rows {
            data.into_iter()
                .enumerate()
                .filter_map(|(i, row)| if include_flat[i] { Some(row) } else { None })
                .collect()
        } else {
            let col_indices: Vec<usize> = include_flat
                .iter()
                .enumerate()
                .filter_map(|(i, &keep)| if keep { Some(i) } else { None })
                .collect();
            data.iter()
                .map(|row| col_indices.iter().map(|&c| row[c].clone()).collect())
                .collect()
        };

        let is_empty_result = filtered.is_empty() || filtered.first().is_none_or(|r| r.is_empty());

        if is_empty_result {
            if args.len() >= 3 {
                let empty_val = self.evaluate_node_in_context(&args[2], cell);
                match empty_val {
                    CalcResult::EmptyArg => CalcResult::new_error(
                        Error::CALC,
                        cell,
                        "No data returned by FILTER".to_string(),
                    ),
                    v => CalcResult::Array(vec![vec![calc_result_to_array_node(v)]]),
                }
            } else {
                CalcResult::new_error(Error::CALC, cell, "No data returned by FILTER".to_string())
            }
        } else {
            CalcResult::Array(filtered)
        }
    }
}

// ── GROUPBY ───────────────────────────────────────────────────────────────────

/// A blank in a report GROUPBY or PIVOTBY lays out: an empty string, as a
/// spilled empty value would read as 0.
fn blank() -> ArrayNode {
    ArrayNode::String(String::new())
}

impl<'a> Model<'a> {
    /// `=GROUPBY(row_fields, values, function, [field_headers], [total_depth],
    /// [sort_order], [filter_array])`
    ///
    /// One row per distinct value of `row_fields`, each with `function`
    /// applied to that group's `values`, column by column; a grand total.
    ///   * function      – a LAMBDA, or a function by name, as `SUM`
    ///   * field_headers – automatic by default, 0 none, 1 headers not shown, 3 shown
    ///   * total_depth   – 1 a grand total at the bottom (default), 0 none,
    ///     -1 at the top
    ///   * sort_order    – the column sorted by, 1 the first (default);
    ///     negative descending
    ///   * filter_array  – TRUE for the rows to group
    pub(crate) fn fn_groupby(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() < 3 || args.len() > 8 {
            return CalcResult::new_args_number_error(cell);
        }
        let fields = match self.eval_to_array(&args[0], cell) {
            Ok(array) => array,
            Err(error) => return error,
        };
        let values = match self.eval_to_array(&args[1], cell) {
            Ok(array) => array,
            Err(error) => return error,
        };
        if fields.is_empty() || fields.len() != values.len() {
            return CalcResult::new_error(
                Error::VALUE,
                cell,
                "row_fields and values must have as many rows".to_string(),
            );
        }
        // Excel detects a header only when the first value is text and the
        // second value is numeric. Explicit field_headers still takes priority.
        let automatic_headers = if matches!(
            values.first().and_then(|row| row.first()),
            Some(ArrayNode::String(_))
        ) && matches!(
            values.get(1).and_then(|row| row.first()),
            Some(ArrayNode::Number(_))
        ) {
            if fields[0].len() > 1 || values[0].len() > 1 {
                3
            } else {
                1
            }
        } else {
            0
        };
        let mut option = |index: usize, default: i64| -> Result<i64, CalcResult> {
            if index >= args.len() {
                return Ok(default);
            }
            match self.evaluate_node_in_context(&args[index], cell) {
                CalcResult::EmptyArg | CalcResult::EmptyCell => Ok(default),
                _ => self
                    .get_number(&args[index], cell)
                    .map(|n| n.trunc() as i64),
            }
        };
        let headers = match option(3, automatic_headers) {
            Ok(n) => n,
            Err(error) => return error,
        };
        let total = match option(4, 1) {
            Ok(n) => n,
            Err(error) => return error,
        };
        let sort = match option(5, 1) {
            Ok(n) => n,
            Err(error) => return error,
        };
        let filter = if args.len() > 6 {
            match self.evaluate_node_in_context(&args[6], cell) {
                CalcResult::EmptyArg => None,
                _ => match self.eval_to_array(&args[6], cell) {
                    Ok(array) => {
                        let flat: Vec<bool> =
                            array.iter().flatten().map(array_node_is_truthy).collect();
                        if flat.len() != fields.len() {
                            return CalcResult::new_error(
                                Error::VALUE,
                                cell,
                                "filter_array must have a value for each row".to_string(),
                            );
                        }
                        Some(flat)
                    }
                    Err(error) => return error,
                },
            }
        } else {
            None
        };
        let has_headers = headers == 1 || headers == 3;
        let first = usize::from(has_headers);
        let key_width = fields[0].len();
        let value_width = values[0].len();

        // The groups, in the order they first come.
        let mut index: HashMap<Vec<CellKey>, usize> = HashMap::new();
        let mut groups: Vec<(Vec<ArrayNode>, Vec<usize>)> = Vec::new();
        let mut all = Vec::new();
        for row in first..fields.len() {
            if filter.as_ref().is_some_and(|keep| !keep[row]) {
                continue;
            }
            all.push(row);
            let key = row_key(&fields[row]);
            match index.get(&key) {
                Some(&at) => groups[at].1.push(row),
                None => {
                    index.insert(key, groups.len());
                    groups.push((fields[row].clone(), vec![row]));
                }
            }
        }
        if groups.is_empty() {
            return CalcResult::new_error(Error::CALC, cell, "nothing to group".to_string());
        }
        let aggregate = |model: &mut Self, rows: &[usize]| -> Vec<ArrayNode> {
            (0..value_width)
                .map(|column| {
                    let picked = rows
                        .iter()
                        .map(|row| {
                            values[*row]
                                .get(column)
                                .cloned()
                                .unwrap_or(ArrayNode::Empty)
                        })
                        .collect();
                    model.groupby_apply(&args[2], picked, cell)
                })
                .collect()
        };
        let mut lines: Vec<(Vec<ArrayNode>, Vec<ArrayNode>)> = Vec::with_capacity(groups.len());
        for (key, rows) in &groups {
            let summary = aggregate(self, rows);
            lines.push((key.clone(), summary));
        }
        let by = sort.unsigned_abs() as usize;
        if by >= 1 && by <= key_width + value_width {
            let ascending = sort > 0;
            lines.sort_by(|a, b| {
                let (x, y) = if by <= key_width {
                    (&a.0[by - 1], &b.0[by - 1])
                } else {
                    (&a.1[by - 1 - key_width], &b.1[by - 1 - key_width])
                };
                sort_key_cmp(x, y, ascending, cell)
            });
        }

        let mut out: Vec<Vec<ArrayNode>> = Vec::new();
        if headers == 3 {
            let mut heading = fields[0].clone();
            heading.extend(values[0].iter().cloned());
            out.push(heading);
        }
        let total_row = if total != 0 {
            let mut row = vec![ArrayNode::String("Total".to_string())];
            row.extend(std::iter::repeat_n(blank(), key_width.saturating_sub(1)));
            row.extend(aggregate(self, &all));
            Some(row)
        } else {
            None
        };
        if total < 0 {
            out.extend(total_row.clone());
        }
        for (mut key, summary) in lines {
            key.extend(summary);
            out.push(key);
        }
        if total > 0 {
            out.extend(total_row);
        }
        CalcResult::Array(out)
    }

    /// `function` applied to one group's values: a LAMBDA called with them,
    /// or a function named bare (`SUM`, as Excel's eta-reduced form) given
    /// them as its one argument.
    fn groupby_apply(
        &mut self,
        function: &Node,
        column: Vec<ArrayNode>,
        cell: CellReferenceIndex,
    ) -> ArrayNode {
        let argument = Node::ArrayKind(column.into_iter().map(|value| vec![value]).collect());
        // Not a LAMBDA's or LET's own name (those have an id), nor a defined
        // name: a function's.
        let bare = match function {
            Node::NamedVariableKind { name, id: None } => Some(name),
            Node::DefinedNameKind((name, _, _)) => Some(name),
            _ => None,
        };
        if let Some(name) = bare {
            if let Some(kind) = self.language.functions.lookup(name) {
                let call = Node::FunctionKind {
                    kind,
                    args: vec![argument],
                };
                return calc_result_to_array_node(self.evaluate_node_in_context(&call, cell));
            }
        }
        let lambda = self.evaluate_node_in_context(function, cell);
        calc_result_to_array_node(self.call_lambda(lambda, &[argument], cell))
    }
}

// ── PIVOTBY ───────────────────────────────────────────────────────────────────

/// The distinct keys of `rows` of `fields`, in first-seen order, each with
/// the rows that have it.
fn distinct_keys(fields: &[Vec<ArrayNode>], rows: &[usize]) -> Vec<(Vec<ArrayNode>, Vec<usize>)> {
    let mut index: HashMap<Vec<CellKey>, usize> = HashMap::new();
    let mut keys: Vec<(Vec<ArrayNode>, Vec<usize>)> = Vec::new();
    for &row in rows {
        let key = row_key(&fields[row]);
        match index.get(&key) {
            Some(&at) => keys[at].1.push(row),
            None => {
                index.insert(key, keys.len());
                keys.push((fields[row].clone(), vec![row]));
            }
        }
    }
    keys
}

fn sort_keys(keys: &mut [(Vec<ArrayNode>, Vec<usize>)], order: i64, cell: CellReferenceIndex) {
    if order == 0 {
        return;
    }
    let ascending = order > 0;
    keys.sort_by(|a, b| {
        a.0.iter()
            .zip(&b.0)
            .map(|(x, y)| sort_key_cmp(x, y, ascending, cell))
            .find(|ordering| *ordering != Ordering::Equal)
            .unwrap_or(Ordering::Equal)
    });
}

impl<'a> Model<'a> {
    /// `=PIVOTBY(row_fields, col_fields, values, function, [field_headers],
    /// [row_total_depth], [row_sort_order], [col_total_depth],
    /// [col_sort_order], [filter_array])`
    ///
    /// The distinct `row_fields` down the side and `col_fields` across the
    /// top, `function` applied to the values each pair has; totals for each
    /// row and column and in all.
    ///   * function             – a LAMBDA, or a function by name, as `SUM`
    ///   * field_headers        – 0 none (default), 1 headers not shown,
    ///     3 shown
    ///   * row/col_total_depth  – 1 totals after (default), 0 none, -1 before
    ///   * row/col_sort_order   – 1 ascending (default), -1 descending
    ///   * filter_array         – TRUE for the rows to take
    pub(crate) fn fn_pivotby(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() < 4 || args.len() > 11 {
            return CalcResult::new_args_number_error(cell);
        }
        let mut arrays = Vec::with_capacity(3);
        for node in &args[..3] {
            match self.eval_to_array(node, cell) {
                Ok(array) => arrays.push(array),
                Err(error) => return error,
            }
        }
        let values = arrays.pop().unwrap_or_default();
        let columns = arrays.pop().unwrap_or_default();
        let fields = arrays.pop().unwrap_or_default();
        if fields.is_empty() || fields.len() != values.len() || columns.len() != values.len() {
            return CalcResult::new_error(
                Error::VALUE,
                cell,
                "row_fields, col_fields and values must have as many rows".to_string(),
            );
        }
        let mut option = |index: usize, default: i64| -> Result<i64, CalcResult> {
            if index >= args.len() {
                return Ok(default);
            }
            match self.evaluate_node_in_context(&args[index], cell) {
                CalcResult::EmptyArg | CalcResult::EmptyCell => Ok(default),
                _ => self
                    .get_number(&args[index], cell)
                    .map(|n| n.trunc() as i64),
            }
        };
        let mut read = |index: usize, default: i64| option(index, default);
        let (headers, row_total, row_sort, col_total, col_sort) =
            match (read(4, 0), read(5, 1), read(6, 1), read(7, 1), read(8, 1)) {
                (Ok(a), Ok(b), Ok(c), Ok(d), Ok(e)) => (a, b, c, d, e),
                (Err(error), ..)
                | (_, Err(error), ..)
                | (_, _, Err(error), ..)
                | (_, _, _, Err(error), _)
                | (_, _, _, _, Err(error)) => return error,
            };
        let filter = if args.len() > 9 {
            match self.evaluate_node_in_context(&args[9], cell) {
                CalcResult::EmptyArg => None,
                _ => match self.eval_to_array(&args[9], cell) {
                    Ok(array) => {
                        let flat: Vec<bool> =
                            array.iter().flatten().map(array_node_is_truthy).collect();
                        if flat.len() != fields.len() {
                            return CalcResult::new_error(
                                Error::VALUE,
                                cell,
                                "filter_array must have a value for each row".to_string(),
                            );
                        }
                        Some(flat)
                    }
                    Err(error) => return error,
                },
            }
        } else {
            None
        };
        let has_headers = headers == 1 || headers == 3;
        let first = usize::from(has_headers);
        let taken: Vec<usize> = (first..fields.len())
            .filter(|row| filter.as_ref().is_none_or(|keep| keep[*row]))
            .collect();
        if taken.is_empty() {
            return CalcResult::new_error(Error::CALC, cell, "nothing to pivot".to_string());
        }
        let mut row_keys = distinct_keys(&fields, &taken);
        let mut col_keys = distinct_keys(&columns, &taken);
        sort_keys(&mut row_keys, row_sort, cell);
        sort_keys(&mut col_keys, col_sort, cell);
        let key_width = fields[0].len();
        let depth = columns[0].len();
        let value_width = values[0].len();

        let aggregate = |model: &mut Self, rows: &[usize]| -> Vec<ArrayNode> {
            (0..value_width)
                .map(|column| {
                    if rows.is_empty() {
                        return blank();
                    }
                    let picked = rows
                        .iter()
                        .map(|row| {
                            values[*row]
                                .get(column)
                                .cloned()
                                .unwrap_or(ArrayNode::Empty)
                        })
                        .collect();
                    model.groupby_apply(&args[3], picked, cell)
                })
                .collect()
        };
        let both = |a: &[usize], b: &[usize]| -> Vec<usize> {
            a.iter().copied().filter(|row| b.contains(row)).collect()
        };
        // The columns across: each column key (and the total) times each
        // value column.
        let mut across: Vec<Option<&[usize]>> =
            col_keys.iter().map(|(_, rows)| Some(&rows[..])).collect();
        if col_total < 0 {
            across.insert(0, None);
        } else if col_total > 0 {
            across.push(None);
        }

        let mut out: Vec<Vec<ArrayNode>> = Vec::new();
        // Header rows: one per column field, then the value names when there
        // are several value columns.
        for level in 0..depth {
            let mut row = vec![blank(); key_width];
            if headers == 3 && level + 1 == depth {
                row = fields[0].clone();
            }
            for column in &across {
                let label = match column {
                    Some(rows) => columns[rows[0]][level].clone(),
                    None if level == 0 => ArrayNode::String("Total".to_string()),
                    None => blank(),
                };
                row.push(label);
                row.extend(std::iter::repeat_n(blank(), value_width - 1));
            }
            out.push(row);
        }
        if value_width > 1 {
            let mut row = vec![blank(); key_width];
            for _ in &across {
                for column in 0..value_width {
                    row.push(if has_headers {
                        values[0][column].clone()
                    } else {
                        blank()
                    });
                }
            }
            out.push(row);
        }
        let mut body: Vec<Vec<ArrayNode>> = Vec::new();
        let line = |model: &mut Self, label: Vec<ArrayNode>, rows: &[usize]| -> Vec<ArrayNode> {
            let mut row = label;
            for column in &across {
                let cells = match column {
                    Some(column_rows) => both(rows, column_rows),
                    None => rows.to_vec(),
                };
                row.extend(aggregate(model, &cells));
            }
            row
        };
        for (key, rows) in &row_keys {
            body.push(line(self, key.clone(), rows));
        }
        if row_total != 0 {
            let mut label = vec![ArrayNode::String("Total".to_string())];
            label.extend(std::iter::repeat_n(blank(), key_width - 1));
            let total = line(self, label, &taken);
            if row_total < 0 {
                body.insert(0, total);
            } else {
                body.push(total);
            }
        }
        out.extend(body);
        CalcResult::Array(out)
    }
}
