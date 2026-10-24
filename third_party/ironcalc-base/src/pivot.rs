//! Visible pivot summaries supplied by an OOXML host to one calculation model.
//! Values are read from normal engine cells; cache source rows are never summed
//! here. Other models have independent, initially empty pivot metadata.
use crate::{
    calc_result::CalcResult,
    cast::{array_node_to_string, calc_result_to_array_node},
    expressions::{
        parser::{ArrayNode, Node},
        token::Error,
        types::CellReferenceIndex,
    },
    model::Model,
};
use std::sync::Arc;

/// A typed member value, independent of any workbook reader implementation.
#[derive(Clone, Debug, PartialEq)]
pub enum ItemValue {
    Text(String),
    Number(f64),
    Boolean(bool),
    Missing,
    Error(String),
    Date(String),
    Unknown,
}
/// A pivot field item, in its original item-index position.
#[derive(Clone, Debug)]
pub struct Member {
    pub aliases: Vec<ItemValue>,
    pub hidden: bool,
}
/// A visible dimension or an unused source field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldRole {
    Row,
    Column,
    Page,
    Unused,
}
/// Field aliases and members, separate from data-measure aliases.
#[derive(Clone, Debug)]
pub struct Field {
    pub names: Vec<String>,
    pub members: Vec<Member>,
    pub role: FieldRole,
    pub page_item: Option<u32>,
}
/// A measure; its aggregation disambiguates explicit custom subtotal rows.
#[derive(Clone, Debug)]
pub struct Measure {
    pub names: Vec<String>,
    pub aggregation: String,
}
/// One physical visible row or column, after expanding repeated item prefixes.
#[derive(Clone, Debug)]
pub struct Line {
    pub members: std::collections::BTreeMap<usize, u32>,
    pub measure: Option<u32>,
    pub kind: String,
}
/// A pivot's saved data cells and their visible coordinate descriptors.
#[derive(Clone, Debug)]
pub struct Region {
    pub sheet: u32,
    pub first_row: i32,
    pub first_column: i32,
    pub last_row: i32,
    pub last_column: i32,
    pub data_row: i32,
    pub data_column: i32,
    pub fields: Vec<Field>,
    pub measures: Vec<Measure>,
    pub rows: Vec<Line>,
    pub columns: Vec<Line>,
    /// A matched but uninterpreted layout produces an engine artifact so the
    /// OOXML adapter can retain its imported cached answer honestly.
    pub unsupported: Option<String>,
}
fn selected(
    regions: &[Arc<Region>],
    left: CellReferenceIndex,
    right: CellReferenceIndex,
) -> Option<Arc<Region>> {
    if left.sheet != right.sheet {
        return None;
    }
    regions
        .iter()
        .rev()
        .find(|r| {
            r.sheet == left.sheet
                && left.row <= r.last_row
                && right.row >= r.first_row
                && left.column <= r.last_column
                && right.column >= r.first_column
        })
        .cloned()
}
fn error(code: Error, cell: CellReferenceIndex, message: &str) -> CalcResult {
    CalcResult::new_error(code, cell, message.into())
}
fn iso_serial(value: &str) -> Option<f64> {
    use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike};
    let value = value.strip_suffix('Z').unwrap_or(value);
    let (date, time) = if let Ok(dt) = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f")
    {
        (dt.date(), Some(dt.time()))
    } else {
        (NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?, None)
    };
    let serial =
        crate::formatter::dates::date_to_serial_number(date.day(), date.month(), date.year())
            .ok()? as f64;
    Some(
        serial
            + time.map_or(0., |t| {
                (f64::from(t.num_seconds_from_midnight()) + f64::from(t.nanosecond()) / 1e9)
                    / 86400.
            }),
    )
}
fn line_priority(kind: &str, aggregation: &str) -> Option<u8> {
    match kind {
        "data" => Some(1),
        "default" | "grand" => Some(2),
        "blank" => None,
        other if other == aggregation => Some(3),
        _ => None,
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ItemKey {
    Text(String),
    Number(u64),
    Boolean(bool),
    Missing,
}
fn number_key(value: f64) -> Option<ItemKey> {
    if !value.is_finite() {
        return None;
    }
    let value = crate::number_format::to_excel_precision(value, 15);
    Some(ItemKey::Number(if value == 0. {
        0
    } else {
        value.to_bits()
    }))
}
fn argument_key(value: &ArrayNode) -> Option<ItemKey> {
    match value {
        ArrayNode::String(s) => Some(ItemKey::Text(s.to_uppercase())),
        ArrayNode::Number(n) => number_key(*n),
        ArrayNode::Boolean(b) => Some(ItemKey::Boolean(*b)),
        ArrayNode::Empty => Some(ItemKey::Missing),
        _ => None,
    }
}
fn alias_key(value: &ItemValue) -> Option<ItemKey> {
    match value {
        ItemValue::Text(s) | ItemValue::Error(s) => Some(ItemKey::Text(s.to_uppercase())),
        ItemValue::Number(n) => number_key(*n),
        ItemValue::Boolean(b) => Some(ItemKey::Boolean(*b)),
        ItemValue::Missing => Some(ItemKey::Missing),
        ItemValue::Date(s) => number_key(iso_serial(s)?),
        ItemValue::Unknown => None,
    }
}
type NameIndex = std::collections::BTreeMap<String, Option<usize>>;
fn add_name(index: &mut NameIndex, name: &str, item: usize) {
    let key = name.to_uppercase();
    index
        .entry(key)
        .and_modify(|old| {
            if *old != Some(item) {
                *old = None;
            }
        })
        .or_insert(Some(item));
}
fn names_index<'a>(names: impl Iterator<Item = (usize, &'a [String])>) -> NameIndex {
    let all = names.collect::<Vec<_>>();
    let mut primary = NameIndex::new();
    let mut aliases = NameIndex::new();
    for (i, names) in &all {
        if let Some(name) = names.first() {
            add_name(&mut primary, name, *i);
        }
        for name in names.iter().skip(1) {
            add_name(&mut aliases, name, *i);
        }
    }
    for (name, item) in aliases {
        primary.entry(name).or_insert(item);
    }
    primary
}
struct MemberIndex {
    values: std::collections::BTreeMap<ItemKey, Vec<u32>>,
    unknown: bool,
}
impl MemberIndex {
    fn new(field: &Field) -> Self {
        let mut index = Self {
            values: std::collections::BTreeMap::new(),
            unknown: false,
        };
        for (i, member) in field.members.iter().enumerate().filter(|(_, m)| !m.hidden) {
            for alias in &member.aliases {
                if let Some(key) = alias_key(alias) {
                    let keys = if key == ItemKey::Missing {
                        vec![
                            key,
                            ItemKey::Text(String::new()),
                            ItemKey::Text("(BLANK)".into()),
                        ]
                    } else {
                        vec![key]
                    };
                    for key in keys {
                        let matches = index.values.entry(key).or_default();
                        if !matches.contains(&(i as u32)) {
                            matches.push(i as u32);
                        }
                    }
                } else {
                    index.unknown = true;
                }
            }
        }
        index
    }
}
type AxisKey = (Option<u32>, Vec<(usize, u32)>);
type AxisLines<'a> = std::collections::BTreeMap<AxisKey, Vec<(usize, &'a str)>>;
struct AxisIndex<'a> {
    role: FieldRole,
    lines: AxisLines<'a>,
}
impl<'a> AxisIndex<'a> {
    fn new(region: &Region, lines: &'a [Line], role: FieldRole) -> Self {
        let mut index = Self {
            role,
            lines: std::collections::BTreeMap::new(),
        };
        for (i, line) in lines.iter().enumerate() {
            let key = line
                .members
                .iter()
                .filter_map(|(field, item)| {
                    (region.fields.get(*field).is_some_and(|f| f.role == role))
                        .then_some((*field, *item))
                })
                .collect();
            index
                .lines
                .entry((line.measure, key))
                .or_default()
                .push((i, line.kind.as_str()));
        }
        index
    }
    fn find(
        &self,
        region: &Region,
        measure: usize,
        criteria: &[(usize, u32)],
    ) -> Result<usize, bool> {
        let mut key = criteria
            .iter()
            .copied()
            .filter(|(field, _)| region.fields[*field].role == self.role)
            .collect::<Vec<_>>();
        key.sort_unstable();
        let aggregation = &region.measures[measure].aggregation;
        let mut found = None;
        let mut priority = 0;
        let mut ambiguous = false;
        for mode in [None, Some(measure as u32)] {
            if let Some(lines) = self.lines.get(&(mode, key.clone())) {
                for (index, kind) in lines {
                    let Some(rank) = line_priority(kind, aggregation) else {
                        continue;
                    };
                    if rank > priority {
                        found = Some(*index);
                        priority = rank;
                        ambiguous = false;
                    } else if rank == priority {
                        ambiguous = true;
                    }
                }
            }
        }
        if ambiguous {
            Err(true)
        } else {
            found.ok_or(false)
        }
    }
}
struct Resolver<'a> {
    region: &'a Region,
    fields: NameIndex,
    measures: NameIndex,
    members: Vec<MemberIndex>,
    rows: AxisIndex<'a>,
    columns: AxisIndex<'a>,
}
impl<'a> Resolver<'a> {
    fn new(region: &'a Region) -> Self {
        Self {
            region,
            fields: names_index(
                region
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(i, f)| (i, f.names.as_slice())),
            ),
            measures: names_index(
                region
                    .measures
                    .iter()
                    .enumerate()
                    .map(|(i, m)| (i, m.names.as_slice())),
            ),
            members: region.fields.iter().map(MemberIndex::new).collect(),
            rows: AxisIndex::new(region, &region.rows, FieldRole::Row),
            columns: AxisIndex::new(region, &region.columns, FieldRole::Column),
        }
    }
}
impl Model<'_> {
    /// Replace the host-supplied visible pivot descriptors for this model.
    /// Evaluation reads ordinary result cells and never refreshes source caches.
    pub fn set_pivot_regions(&mut self, regions: Vec<Arc<Region>>) {
        self.pivot_regions = regions;
    }

    pub(crate) fn fn_getpivotdata(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        if !(2..=254).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        if !args.len().is_multiple_of(2) {
            return error(Error::REF, cell, "Expected field/item pairs");
        }
        // Keep references for the anchor, but evaluate value arguments once in
        // source order. A data-field error precedes an invalid pivot anchor.
        let first = self.evaluate_node_with_reference(&args[0], cell);
        if matches!(first, CalcResult::Error { .. }) {
            return first;
        }
        let reference = self.evaluate_node_with_reference(&args[1], cell);
        let (left, right) = match reference {
            CalcResult::Range { left, right } => (left, right),
            e @ CalcResult::Error { .. } => return e,
            _ => return error(Error::REF, cell, "Expected a pivot reference"),
        };
        let Some(region) = selected(&self.pivot_regions, left, right) else {
            return error(Error::REF, cell, "No pivot in reference");
        };
        if let Some(reason) = &region.unsupported {
            return error(Error::ERROR, cell, reason);
        }
        let resolver = Resolver::new(&region);
        let mut values = Vec::new();
        let (mut height, mut width) = (1usize, 1usize);
        let mut array = false;
        let mut input_cells = 0usize;
        for (index, arg) in args.iter().enumerate().filter(|(i, _)| *i != 1) {
            let evaluated = if index == 0 {
                first.clone()
            } else {
                self.evaluate_node_in_context(arg, cell)
            };
            let value = match evaluated {
                e @ CalcResult::Error { .. } => return e,
                CalcResult::Range { left, right } => {
                    if left.sheet != right.sheet
                        || left.row > right.row
                        || left.column > right.column
                        || (i64::from(right.row) - i64::from(left.row) + 1)
                            .saturating_mul(i64::from(right.column) - i64::from(left.column) + 1)
                            > 1_000_000
                    {
                        return error(Error::NUM, cell, "Pivot argument range too large");
                    }
                    let a = self.evaluate_range(left, right);
                    array |= a.len() > 1 || a.first().is_some_and(|r| r.len() > 1);
                    a
                }
                CalcResult::Array(a) => {
                    array = true;
                    a
                }
                v => vec![vec![calc_result_to_array_node(v)]],
            };
            let h = value.len();
            let w = value.first().map_or(0, Vec::len);
            if h == 0 || w == 0 || value.iter().any(|r| r.len() != w) {
                return error(Error::VALUE, cell, "Invalid argument array");
            }
            input_cells = input_cells.saturating_add(h.saturating_mul(w));
            if input_cells > 1_000_000 {
                return error(Error::NUM, cell, "Pivot arguments exceed the cell budget");
            }
            height = height.max(h);
            width = width.max(w);
            values.push((index, value));
        }
        if height.saturating_mul(width) > 1_000_000 {
            return error(Error::NUM, cell, "Pivot result array too large");
        }
        if values.iter().any(|(_, v)| {
            v.len() != 1 && v.len() != height || v[0].len() != 1 && v[0].len() != width
        }) {
            return error(Error::VALUE, cell, "Incompatible argument arrays");
        }
        let mut result = Vec::with_capacity(height);
        for row in 0..height {
            let mut line = Vec::with_capacity(width);
            for column in 0..width {
                let args = values
                    .iter()
                    .map(|(_, v)| {
                        v[if v.len() == 1 { 0 } else { row }]
                            [if v[0].len() == 1 { 0 } else { column }]
                        .clone()
                    })
                    .collect::<Vec<_>>();
                let answer = self.pivot_scalar(&resolver, &args, cell);
                if !array && height == 1 && width == 1 {
                    return answer;
                }
                line.push(calc_result_to_array_node(answer));
            }
            result.push(line);
        }
        CalcResult::Array(result)
    }
    fn pivot_scalar(
        &mut self,
        resolver: &Resolver<'_>,
        args: &[ArrayNode],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        let region = resolver.region;
        if let Some(e) = args.iter().find_map(|v| {
            if let ArrayNode::Error(e) = v {
                Some(e.clone())
            } else {
                None
            }
        }) {
            return error(e, cell, "Pivot argument error");
        }
        let name = match array_node_to_string(&args[0]) {
            Ok(s) => s,
            Err(e) => return error(e, cell, "Invalid data field"),
        };
        let Some(measure) = resolver
            .measures
            .get(&name.to_uppercase())
            .copied()
            .flatten()
        else {
            return error(Error::REF, cell, "Unknown or ambiguous data field");
        };
        let mut criteria = Vec::new();
        for pair in args[1..].chunks_exact(2) {
            let name = match array_node_to_string(&pair[0]) {
                Ok(s) => s,
                Err(e) => return error(e, cell, "Invalid field name"),
            };
            let Some(field) = resolver.fields.get(&name.to_uppercase()).copied().flatten() else {
                return error(Error::REF, cell, "Unknown or ambiguous field");
            };
            let f = &region.fields[field];
            if f.role == FieldRole::Unused {
                return error(Error::REF, cell, "Field is not visible");
            }
            let index = &resolver.members[field];
            let matched = argument_key(&pair[1]).and_then(|key| index.values.get(&key));
            let Some(matches) = matched.filter(|m| m.len() == 1) else {
                return error(
                    if index.unknown {
                        Error::ERROR
                    } else {
                        Error::REF
                    },
                    cell,
                    "Unknown or ambiguous pivot item",
                );
            };
            let item = matches[0];
            if f.role == FieldRole::Page && f.page_item != Some(item) {
                return error(Error::REF, cell, "Report filter item is not displayed");
            }
            if criteria.iter().any(|(i, v)| *i == field && *v != item) {
                return error(Error::REF, cell, "Conflicting pivot criteria");
            }
            if !criteria.iter().any(|(i, _)| *i == field) {
                criteria.push((field, item));
            }
        }
        let row = match resolver.rows.find(region, measure, &criteria) {
            Ok(i) => i,
            Err(ambiguous) => {
                return error(
                    if ambiguous { Error::ERROR } else { Error::REF },
                    cell,
                    "No unique visible pivot row",
                )
            }
        };
        let column = match resolver.columns.find(region, measure, &criteria) {
            Ok(i) => i,
            Err(ambiguous) => {
                return error(
                    if ambiguous { Error::ERROR } else { Error::REF },
                    cell,
                    "No unique visible pivot column",
                )
            }
        };
        let row = i32::try_from(row)
            .ok()
            .and_then(|r| region.data_row.checked_add(r));
        let column = i32::try_from(column)
            .ok()
            .and_then(|c| region.data_column.checked_add(c));
        match (row, column) {
            (Some(row), Some(column))
                if (1..=1_048_576).contains(&row) && (1..=16_384).contains(&column) =>
            {
                self.evaluate_cell(CellReferenceIndex {
                    sheet: region.sheet,
                    row,
                    column,
                })
            }
            _ => error(Error::ERROR, cell, "Pivot result outside worksheet bounds"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test::util::new_empty_model;
    fn region() -> Arc<Region> {
        Arc::new(Region {
            sheet: 0,
            first_row: 3,
            first_column: 5,
            last_row: 6,
            last_column: 6,
            data_row: 6,
            data_column: 6,
            fields: vec![],
            measures: vec![Measure {
                names: vec!["Amount".into()],
                aggregation: "sum".into(),
            }],
            rows: vec![Line {
                members: Default::default(),
                measure: None,
                kind: "grand".into(),
            }],
            columns: vec![Line {
                members: Default::default(),
                measure: None,
                kind: "grand".into(),
            }],
            unsupported: None,
        })
    }
    #[test]
    fn pivot_descriptors_belong_to_one_model_and_can_be_replaced_or_cleared() {
        let mut a = new_empty_model();
        let mut b = new_empty_model();
        for model in [&mut a, &mut b] {
            model._set("F6", "42");
            model._set("H1", "=GETPIVOTDATA(\"Amount\",E3)");
        }
        a.set_pivot_regions(vec![region()]);
        a.evaluate();
        b.evaluate();
        assert_eq!(a._get_text("H1"), "42");
        assert_eq!(b._get_text("H1"), "#REF!");
        b.set_pivot_regions(vec![region()]);
        b._set("F6", "27");
        a.set_pivot_regions(vec![]);
        b.evaluate();
        a.evaluate();
        assert_eq!(a._get_text("H1"), "#REF!");
        assert_eq!(b._get_text("H1"), "27");
    }
    #[test]
    fn pivot_result_coordinate_overflow_is_an_artifact_instead_of_a_panic() {
        let mut model = new_empty_model();
        let mut invalid = (*region()).clone();
        invalid.data_row = i32::MAX;
        invalid.rows.insert(
            0,
            Line {
                members: Default::default(),
                measure: None,
                kind: "blank".into(),
            },
        );
        model.set_pivot_regions(vec![Arc::new(invalid)]);
        model._set("H1", "=GETPIVOTDATA(\"Amount\",E3)");
        model.evaluate();
        assert_eq!(model._get_text("H1"), "#ERROR!");
    }
    #[test]
    fn pivot_result_cycles_remain_calculation_cycles() {
        let mut model = new_empty_model();
        model.set_pivot_regions(vec![region()]);
        model._set("F6", "=H1");
        model._set("H1", "=GETPIVOTDATA(\"Amount\",E3)");
        model.evaluate();
        assert_eq!(model._get_text("H1"), "#CIRC!");
    }
}
