//! Rebase decoded formula references without flattening compatibility XML.
use std::{collections::BTreeMap, ops::Range};

use jade_xlsx_reader::{
    cell_content::WorksheetSource,
    formula::{shifted_formula_references, FormulaReferenceEdit},
    shift_formula, StructureChange,
};
use quick_xml::{events::Event, Reader};

use crate::{
    patch::{apply, escape, Splice},
    Result, WriteError,
};

enum Kind {
    Sequence,
    Alternatives { fallback: bool },
    Text { span: Range<usize>, value: String },
}
struct Node {
    kind: Kind,
    children: Vec<usize>,
    depth: usize,
}

fn tree(raw: &str, base: usize, source: &WorksheetSource) -> Result<Vec<Node>> {
    let framed = format!("<f>{raw}</f>");
    let mut reader = Reader::from_str(&framed);
    let mut nodes = vec![Node {
        kind: Kind::Sequence,
        children: vec![],
        depth: 0,
    }];
    let mut stack = vec![0];
    let mut root = true;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(_) if root => {
                root = false;
            }
            Event::Start(e) | Event::Empty(e) => {
                let offset = base + start - 3;
                let parent = *stack.last().unwrap();
                let kind = if source.has_alternate_content_at(offset) {
                    Kind::Alternatives { fallback: false }
                } else if let Some(fallback) = source.compatible_branch_at(offset) {
                    if let Kind::Alternatives { fallback: has } = &mut nodes[parent].kind {
                        *has |= fallback;
                    } else {
                        return Err(WriteError::FormulaMarkup("misplaced formula alternative"));
                    }
                    Kind::Sequence
                } else if source.processes_content_at(offset) {
                    Kind::Sequence
                } else {
                    if !empty {
                        reader.read_to_end(e.name())?;
                    }
                    continue;
                };
                let depth = nodes[parent].depth
                    + usize::from(matches!(nodes[parent].kind, Kind::Alternatives { .. }));
                let index = nodes.len();
                nodes.push(Node {
                    kind,
                    children: vec![],
                    depth,
                });
                nodes[parent].children.push(index);
                if !empty {
                    stack.push(index);
                }
            }
            Event::End(_) => {
                if stack.len() == 1 {
                    break;
                }
                stack.pop();
            }
            Event::Text(_) | Event::CData(_) | Event::GeneralRef(_) => {
                let parent = *stack.last().unwrap();
                // Whitespace directly under AlternateContent is not formula text.
                if matches!(nodes[parent].kind, Kind::Alternatives { .. }) {
                    continue;
                }
                let value = match event {
                    Event::Text(e) => e.decode()?.into_owned(),
                    Event::CData(e) => e.decode()?.into_owned(),
                    Event::GeneralRef(e) => {
                        quick_xml::escape::unescape(&format!("&{};", e.decode()?))
                            .map_err(quick_xml::Error::from)?
                            .into_owned()
                    }
                    _ => unreachable!(),
                };
                let index = nodes.len();
                nodes.push(Node {
                    kind: Kind::Text {
                        span: start - 3..reader.buffer_position() as usize - 3,
                        value,
                    },
                    children: vec![],
                    depth: nodes[parent].depth,
                });
                nodes[parent].children.push(index);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(nodes)
}

fn flatten(
    nodes: &[Node],
    choices: &BTreeMap<usize, Option<usize>>,
) -> std::result::Result<Vec<usize>, usize> {
    let mut pending = vec![0];
    let mut parts = Vec::new();
    while let Some(index) = pending.pop() {
        match &nodes[index].kind {
            Kind::Text { .. } => parts.push(index),
            Kind::Sequence => pending.extend(nodes[index].children.iter().rev().copied()),
            Kind::Alternatives { .. } => {
                let Some(branch) = choices.get(&index) else {
                    return Err(index);
                };
                if let Some(branch) = branch {
                    pending.push(*branch);
                }
            }
        }
    }
    Ok(parts)
}

fn layouts(nodes: &[Node]) -> Result<Vec<Vec<usize>>> {
    let budget = nodes.len().saturating_mul(8).max(65536);
    let mut pending = vec![BTreeMap::new()];
    let mut stored = 0usize;
    let mut references = 0usize;
    let mut result = Vec::new();
    for _ in 0..1024 {
        let Some(choices) = pending.pop() else {
            return Ok(result);
        };
        stored -= choices.len();
        match flatten(nodes, &choices) {
            Ok(parts) => {
                references = references
                    .checked_add(parts.len())
                    .filter(|&n| n <= budget)
                    .ok_or(WriteError::FormulaMarkup(
                        "formula alternative reference limit",
                    ))?;
                result.push(parts);
            }
            Err(index) => {
                let Kind::Alternatives { fallback } = nodes[index].kind else {
                    unreachable!()
                };
                let branches = nodes[index]
                    .children
                    .iter()
                    .copied()
                    .map(Some)
                    .chain((!fallback).then_some(None));
                for branch in branches {
                    stored = stored
                        .checked_add(choices.len() + 1)
                        .filter(|&n| n <= budget)
                        .ok_or(WriteError::FormulaMarkup("formula alternative state limit"))?;
                    let mut selected = choices.clone();
                    selected.insert(index, branch);
                    pending.push(selected);
                }
            }
        }
    }
    Err(WriteError::FormulaMarkup("too many formula alternatives"))
}

// Reference tokens are short. LCS isolates independent coordinate changes in
// ranges and in references split across more than one compatibility branch.
fn deltas(old: &str, new: &str) -> Vec<(Range<usize>, String)> {
    if old == new {
        return vec![];
    }
    let a: Vec<_> = old.chars().collect();
    let b: Vec<_> = new.chars().collect();
    if (a.len() + 1).saturating_mul(b.len() + 1) > 4096 {
        return vec![(0..old.len(), new.into())];
    }
    let width = b.len() + 1;
    let mut lengths = vec![0usize; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lengths[i * width + j] = if a[i] == b[j] {
                1 + lengths[(i + 1) * width + j + 1]
            } else {
                lengths[(i + 1) * width + j].max(lengths[i * width + j + 1])
            };
        }
    }
    let offsets: Vec<_> = old
        .char_indices()
        .map(|(at, _)| at)
        .chain([old.len()])
        .collect();
    let (mut i, mut j) = (0, 0);
    let mut result = Vec::new();
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            i += 1;
            j += 1;
            continue;
        }
        let start = i;
        let mut text = String::new();
        while i < a.len() || j < b.len() {
            if i < a.len() && j < b.len() && a[i] == b[j] {
                break;
            }
            if j < b.len()
                && (i == a.len() || lengths[i * width + j + 1] > lengths[(i + 1) * width + j])
            {
                text.push(b[j]);
                j += 1;
            } else {
                i += 1;
            }
        }
        result.push((offsets[start]..offsets[i], text));
    }
    result
}

fn value(node: &Node) -> &str {
    let Kind::Text { value, .. } = &node.kind else {
        unreachable!()
    };
    value
}

fn plan(
    nodes: &[Node],
    layouts: &[Vec<usize>],
    sheet: &str,
    change: &StructureChange,
    whole: bool,
) -> Option<BTreeMap<usize, String>> {
    let mut combined = BTreeMap::new();
    for parts in layouts {
        let mut text = String::new();
        let positions: Vec<_> = parts
            .iter()
            .map(|&id| {
                let start = text.len();
                text.push_str(value(&nodes[id]));
                (start, text.len(), id)
            })
            .collect();
        let mut edits: BTreeMap<usize, Vec<(Range<usize>, String)>> = BTreeMap::new();
        for FormulaReferenceEdit { range, text: next } in
            shifted_formula_references(&text, sheet, change)
        {
            let changes = if whole {
                vec![(0..range.len(), next)]
            } else {
                deltas(&text[range.clone()], &next)
            };
            for (delta, replacement) in changes {
                let range = range.start + delta.start..range.start + delta.end;
                let hits: Vec<_> = positions
                    .iter()
                    .copied()
                    .filter(|&(start, end, _)| {
                        if range.is_empty() {
                            start <= range.start && range.start <= end
                        } else {
                            start < range.end && range.start < end
                        }
                    })
                    .collect();
                let &(_, _, anchor) = hits.iter().max_by_key(|&&(_, _, id)| nodes[id].depth)?;
                for (start, end, id) in hits {
                    if range.is_empty() && id != anchor {
                        continue;
                    }
                    let local = range.start.max(start) - start..range.end.min(end) - start;
                    edits.entry(id).or_default().push((
                        local,
                        if id == anchor {
                            replacement.clone()
                        } else {
                            String::new()
                        },
                    ));
                }
            }
        }
        let mut verified = String::new();
        for &id in parts {
            let mut next = value(&nodes[id]).to_string();
            if let Some(edits) = edits.get_mut(&id) {
                edits.sort_by_key(|(range, _)| (range.start, range.end));
                if edits.windows(2).any(|w| w[0].0.end > w[1].0.start) {
                    return None;
                }
                for (range, text) in edits.iter().rev() {
                    next.replace_range(range.clone(), text);
                }
            }
            verified.push_str(&next);
            if let Some(previous) = combined.insert(id, next.clone()) {
                if previous != next {
                    return None;
                }
            }
        }
        if verified != shift_formula(&text, sheet, change) {
            return None;
        }
    }
    Some(combined)
}

pub(crate) fn rewrite(
    raw: &str,
    base: usize,
    source: &WorksheetSource,
    sheet: &str,
    change: &StructureChange,
) -> Result<String> {
    let nodes = tree(raw, base, source)?;
    if !jade_xlsx_reader::formula::same_sheet_name(sheet, &change.sheet)
        && !nodes
            .iter()
            .any(|node| matches!(&node.kind, Kind::Text { value, .. } if value.contains('!')))
    {
        return Ok(raw.into());
    }
    let layouts = layouts(&nodes)?;
    if layouts.iter().all(|parts| {
        let text: String = parts.iter().map(|&id| value(&nodes[id])).collect();
        shift_formula(&text, sheet, change) == text
    }) {
        return Ok(raw.into());
    }
    // Fine coordinate edits preserve shared text. Whole-reference placement
    // can also represent deletions whose #REF! must live in one branch.
    for whole in [false, true] {
        let Some(plan) = plan(&nodes, &layouts, sheet, change, whole) else {
            continue;
        };
        let budget = raw.len().saturating_mul(8).max(65536);
        let mut size = 0usize;
        let mut splices = Vec::new();
        for (id, text) in plan {
            let Kind::Text { span, value } = &nodes[id].kind else {
                unreachable!()
            };
            if text == *value {
                continue;
            }
            let text = escape(&text);
            size = size
                .checked_add(text.len())
                .filter(|&n| n <= budget)
                .ok_or(WriteError::FormulaMarkup("formula text expansion limit"))?;
            splices.push(Splice {
                start: span.start,
                end: span.end,
                text,
            });
        }
        return Ok(apply(raw, splices));
    }
    Err(WriteError::FormulaMarkup(
        "shared formula text cannot be rebased consistently",
    ))
}
