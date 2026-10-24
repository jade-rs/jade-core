//! Dependency ordering uses observed cell/range reads from the same calculation
//! that supplies source values, including names and dynamic references.
use crate::Result;
use jade_xlsx_reader::{CellRange, CellRef, Workbook};
use std::collections::{BTreeMap, BTreeSet};
pub(super) type Graph = BTreeMap<u32, BTreeSet<u32>>;
pub(super) fn refusal(message: &str) -> crate::HeadlessError {
    jade_xlsx_writer::WriteError::Pivot(message.into()).into()
}
fn overlaps(a: CellRange, b: CellRange) -> bool {
    a.start.row <= b.end.row
        && b.start.row <= a.end.row
        && a.start.column <= b.end.column
        && b.start.column <= a.end.column
}
fn consume(work: &mut usize) -> Result<()> {
    *work += 1;
    if *work > 20_000_000 {
        return Err(refusal("pivot dependency expansion exceeded its budget"));
    }
    Ok(())
}
pub(super) fn graph(
    book: &Workbook,
    reads: &jade_xlsx_calc::ValueDependencies,
    work: &mut usize,
) -> Result<Graph> {
    if reads.incomplete {
        return Err(refusal(
            "pivot source dependency trace is incomplete or exceeded its budget",
        ));
    }
    let mut cells =
        BTreeMap::<usize, BTreeMap<u32, Vec<(CellRef, &Vec<(usize, CellRange)>)>>>::new();
    for ((sheet, at), ranges) in &reads.reads {
        cells
            .entry(*sheet)
            .or_default()
            .entry(at.row)
            .or_default()
            .push((*at, ranges));
    }
    let mut graph = Graph::new();
    for table in &book.pivots.tables {
        if graph.contains_key(&table.cache_id) {
            continue;
        }
        let (part, range) = jade_xlsx_writer::pivot_refresh::source_range(book, table.cache_id)?;
        let sheet = book
            .sheets
            .iter()
            .position(|s| s.part_name == part)
            .ok_or_else(|| refusal("missing pivot source worksheet"))?;
        let mut stack = vec![(sheet, range)];
        let mut visited = BTreeSet::new();
        let mut visited_cells = BTreeSet::new();
        let mut deps = BTreeSet::new();
        while let Some((sheet, area)) = stack.pop() {
            if !visited.insert((sheet, area.start, area.end)) {
                continue;
            }
            consume(work)?;
            if area.start.row > area.end.row || area.start.column > area.end.column {
                return Err(refusal("inverted pivot source dependency range"));
            }
            let owner = book
                .sheets
                .get(sheet)
                .ok_or_else(|| refusal("invalid dependency worksheet"))?;
            for report in &book.pivots.tables {
                consume(work)?;
                if report.sheet_part == owner.part_name && overlaps(area, report.location.reference)
                {
                    deps.insert(report.cache_id);
                }
            }
            if let Some(rows) = cells.get(&sheet) {
                for (_, row) in rows.range(area.start.row..=area.end.row) {
                    for (at, ranges) in row {
                        consume(work)?;
                        if area.contains(*at) && visited_cells.insert((sheet, *at)) {
                            stack.extend(ranges.iter().copied());
                        }
                    }
                }
            }
        }
        graph.insert(table.cache_id, deps);
    }
    // Check the whole graph, including already completed caches: a changing
    // conditional/dynamic reference can introduce a cycle later in the batch.
    let mut remaining = graph.clone();
    while !remaining.is_empty() {
        let ready = remaining
            .iter()
            .filter(|(_, deps)| deps.is_empty())
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        if ready.is_empty() {
            return Err(refusal("circular dependency between pivot cache sources"));
        }
        for id in &ready {
            remaining.remove(id);
        }
        for deps in remaining.values_mut() {
            for id in &ready {
                deps.remove(id);
            }
        }
    }
    Ok(graph)
}
/// Invalidate a consumed source when an observed predecessor was refreshed
/// later, or is itself pending. Dependencies changing during the batch cannot
/// make an earlier completion appear fresh just because its id is in a set.
pub(super) fn invalidate(graph: &Graph, completed: &mut BTreeMap<u32, usize>) {
    loop {
        let stale = completed
            .iter()
            .filter(|(id, stamp)| {
                graph[*id]
                    .iter()
                    .any(|dep| completed.get(dep).is_none_or(|time| time >= *stamp))
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        if stale.is_empty() {
            return;
        }
        for id in stale {
            completed.remove(&id);
        }
    }
}
