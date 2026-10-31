//! Data ▸ Group: rows and columns grouped into outline levels, and a group's
//! detail hidden and shown again.
//!
//! Excel keeps an outline in three places: each grouped row's or column's
//! `outlineLevel` (1 to 7), `collapsed` on the summary row or column of a
//! group whose detail is hidden (with the detail itself `hidden`), and the
//! deepest level in `sheetFormatPr/@outlineLevelRow` and `outlineLevelCol`,
//! which sizes the outline gutter. A group is a run of neighbours at its
//! level or deeper; its summary is the row after it (`summaryBelow`, the
//! default) or the one before, and the column after it or before
//! (`summaryRight`).

use std::collections::BTreeMap;

use quick_xml::events::{BytesStart, Event};
use quick_xml::name::QName;
use quick_xml::Reader;

use jade_xlsx_reader::model::{MAX_COLUMN_COUNT, MAX_ROW_COUNT};
use jade_xlsx_reader::{Axis, Sheet};

use crate::patch::{apply, attribute, escape, local, namespace_prefix, Splice};
use crate::{Result, WriteError};

/// Excel's deepest outline level.
pub const MAX_OUTLINE_LEVEL: u8 = 7;

/// What Data ▸ Outline does to the rows or columns given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutlineChange {
    /// Group: one level deeper.
    Group,
    /// Ungroup: one level shallower, for those grouped at all.
    Ungroup,
    /// Hide Detail: the group at the first one, or the group it is the
    /// summary of, collapsed.
    HideDetail,
    /// Show Detail: that group expanded, groups inside it that are
    /// collapsed staying so.
    ShowDetail,
    /// One of the outline's level buttons: the rows (or columns) above that
    /// level shown and every group at it or deeper collapsed — `1` collapses
    /// everything, one past the deepest level shows everything. The rows or
    /// columns given are not looked at.
    ShowLevel(u8),
}

/// What one row or column of the outline becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutlineMark {
    pub level: u8,
    pub collapsed: bool,
    pub hidden: bool,
}

/// One axis of a sheet's outline, as the plan reads it.
struct AxisView<'a> {
    sheet: &'a Sheet,
    axis: Axis,
    limit: u32,
    summary_after: bool,
}

impl AxisView<'_> {
    fn level(&self, index: u32) -> u8 {
        match self.axis {
            Axis::Rows => self.sheet.outline.row_level(index),
            Axis::Columns => self.sheet.outline.column_level(index),
        }
    }

    fn collapsed(&self, index: u32) -> bool {
        match self.axis {
            Axis::Rows => self.sheet.outline.collapsed_rows.contains(&index),
            Axis::Columns => self.sheet.outline.collapsed_columns.contains(&index),
        }
    }

    fn hidden(&self, index: u32) -> bool {
        match self.axis {
            Axis::Rows => self
                .sheet
                .row(index)
                .map_or(self.sheet.default_rows_hidden, |row| row.hidden),
            Axis::Columns => self
                .sheet
                .columns
                .iter()
                .any(|span| span.hidden && (span.first..=span.last).contains(&index)),
        }
    }

    fn mark(&self, index: u32) -> OutlineMark {
        OutlineMark {
            level: self.level(index),
            collapsed: self.collapsed(index),
            hidden: self.hidden(index),
        }
    }

    /// The run of neighbours of `index` at `level` or deeper.
    fn run(&self, index: u32, level: u8) -> (u32, u32) {
        let mut first = index;
        while first > 0 && self.level(first - 1) >= level {
            first -= 1;
        }
        let mut last = index;
        while last + 1 < self.limit && self.level(last + 1) >= level {
            last += 1;
        }
        (first, last)
    }

    /// Where the summary of the group `first..=last` sits, if anywhere.
    fn summary(&self, (first, last): (u32, u32)) -> Option<u32> {
        if self.summary_after {
            (last + 1 < self.limit).then_some(last + 1)
        } else {
            first.checked_sub(1)
        }
    }

    /// The group Hide or Show Detail at `index` acts on, and its level:
    /// the one `index` summarises when there is one, else the innermost one
    /// `index` is in.
    fn group_at(&self, index: u32) -> Option<((u32, u32), u8)> {
        let own = self.level(index);
        let detail = if self.summary_after {
            index.checked_sub(1)
        } else {
            (index + 1 < self.limit).then_some(index + 1)
        };
        if let Some(detail) = detail.filter(|&detail| self.level(detail) > own) {
            let level = own + 1;
            return Some((self.run(detail, level), level));
        }
        (own > 0).then(|| (self.run(index, own), own))
    }
}

/// What `change` makes of the rows or columns `first..=last` of `sheet`:
/// each one it changes, with all it becomes. Empty when it changes nothing
/// — ungrouping what is not grouped, hiding the detail of no group.
pub fn plan_outline(
    sheet: &Sheet,
    axis: Axis,
    first: u32,
    last: u32,
    change: OutlineChange,
) -> Result<BTreeMap<u32, OutlineMark>> {
    let limit = match axis {
        Axis::Rows => MAX_ROW_COUNT,
        Axis::Columns => MAX_COLUMN_COUNT,
    };
    let (first, last) = (first.min(last), first.max(last));
    if last >= limit {
        return Err(WriteError::StructureOutOfBounds { at: last, count: 1 });
    }
    let view = AxisView {
        sheet,
        axis,
        limit,
        summary_after: match axis {
            Axis::Rows => sheet.outline.summary_below,
            Axis::Columns => sheet.outline.summary_right,
        },
    };
    let mut marks = BTreeMap::new();
    match change {
        OutlineChange::Group => {
            for index in first..=last {
                let mut mark = view.mark(index);
                if mark.level >= MAX_OUTLINE_LEVEL {
                    return Err(WriteError::Outline("an outline has at most seven levels"));
                }
                mark.level += 1;
                marks.insert(index, mark);
            }
        }
        OutlineChange::Ungroup => {
            for index in first..=last {
                let mut mark = view.mark(index);
                if mark.level > 0 {
                    mark.level -= 1;
                    marks.insert(index, mark);
                }
            }
        }
        OutlineChange::HideDetail => {
            let Some((group, _)) = view.group_at(first) else {
                return Ok(marks);
            };
            for index in group.0..=group.1 {
                let mut mark = view.mark(index);
                mark.hidden = true;
                marks.insert(index, mark);
            }
            if let Some(summary) = view.summary(group) {
                let mut mark = view.mark(summary);
                mark.collapsed = true;
                marks.insert(summary, mark);
            }
        }
        OutlineChange::ShowLevel(shown) => {
            let levels = match axis {
                Axis::Rows => &sheet.outline.row_levels,
                Axis::Columns => &sheet.outline.column_levels,
            };
            let deepest = levels.values().copied().max().unwrap_or(0);
            for (&index, &level) in levels {
                let mut mark = view.mark(index);
                mark.hidden = level >= shown;
                marks.insert(index, mark);
            }
            // Each group's summary says whether the group is collapsed.
            for level in 1..=deepest {
                let mut runs: Vec<(u32, u32)> = Vec::new();
                for (&index, _) in levels.iter().filter(|(_, l)| **l >= level) {
                    match runs.last_mut() {
                        Some(run) if run.1 + 1 == index => run.1 = index,
                        _ => runs.push((index, index)),
                    }
                }
                for run in runs {
                    if let Some(summary) = view.summary(run) {
                        let mut mark = marks
                            .get(&summary)
                            .copied()
                            .unwrap_or_else(|| view.mark(summary));
                        mark.collapsed = level >= shown;
                        marks.insert(summary, mark);
                    }
                }
            }
        }
        OutlineChange::ShowDetail => {
            let Some((group, level)) = view.group_at(first) else {
                return Ok(marks);
            };
            show(&view, group, level, &mut marks);
            if let Some(summary) = view.summary(group) {
                let mut mark = marks
                    .get(&summary)
                    .copied()
                    .unwrap_or_else(|| view.mark(summary));
                mark.collapsed = false;
                marks.insert(summary, mark);
            }
        }
    }
    // Only what changes.
    marks.retain(|&index, mark| *mark != view.mark(index));
    Ok(marks)
}

/// Shows the rows or columns of `group` at `level`, and those of each group
/// inside it whose summary is not collapsed, level by level.
fn show(view: &AxisView<'_>, group: (u32, u32), level: u8, marks: &mut BTreeMap<u32, OutlineMark>) {
    let mut index = group.0;
    while index <= group.1 {
        if view.level(index) <= level {
            let mut mark = marks
                .get(&index)
                .copied()
                .unwrap_or_else(|| view.mark(index));
            mark.hidden = false;
            marks.insert(index, mark);
            index += 1;
            continue;
        }
        let inner = view.run(index, level + 1);
        let inner = (inner.0.max(group.0), inner.1.min(group.1));
        let collapsed = view
            .summary(inner)
            .is_some_and(|summary| view.collapsed(summary));
        if !collapsed {
            show(view, inner, level + 1, marks);
        }
        index = inner.1 + 1;
    }
}

/// The deepest level `marks` leave on the axis of `sheet`.
pub fn deepest_level(sheet: &Sheet, axis: Axis, marks: &BTreeMap<u32, OutlineMark>) -> u8 {
    let levels = match axis {
        Axis::Rows => &sheet.outline.row_levels,
        Axis::Columns => &sheet.outline.column_levels,
    };
    levels
        .iter()
        .filter(|(index, _)| !marks.contains_key(index))
        .map(|(_, level)| *level)
        .chain(marks.values().map(|mark| mark.level))
        .max()
        .unwrap_or(0)
}

/// The sheet part with `marks` written on its rows or columns and the
/// deepest level `deepest` in `sheetFormatPr`. `default_width` is the width
/// a column gets when it has no `<col>` yet.
pub fn write_outline(
    xml: &str,
    axis: Axis,
    marks: &BTreeMap<u32, OutlineMark>,
    deepest: u8,
    default_rows_hidden: bool,
    default_width: f64,
) -> Result<String> {
    let mut splices = match axis {
        Axis::Rows => row_splices(xml, marks, default_rows_hidden)?,
        Axis::Columns => column_splices(xml, marks, default_width)?,
    };
    splices.extend(format_splice(xml, axis, deepest)?);
    Ok(apply(xml, splices))
}

/// `element`'s start tag with each of `set` given its value, or taken off
/// for `None`, the other attributes kept.
fn rewrite(
    element: &BytesStart<'_>,
    set: &[(&str, Option<String>)],
    empty: bool,
) -> Result<String> {
    let mut out = format!("<{}", String::from_utf8_lossy(element.name().as_ref()));
    for attr in element.attributes() {
        let attr = attr?;
        let key = String::from_utf8_lossy(attr.key.as_ref());
        if set.iter().any(|(name, _)| *name == key) {
            continue;
        }
        out.push_str(&format!(" {key}=\"{}\"", escape(&attr.unescape_value()?)));
    }
    for (key, value) in set {
        if let Some(value) = value {
            out.push_str(&format!(" {key}=\"{}\"", escape(value)));
        }
    }
    out.push_str(if empty { "/>" } else { ">" });
    Ok(out)
}

fn mark_attributes(
    mark: &OutlineMark,
    default_hidden: bool,
) -> [(&'static str, Option<String>); 3] {
    [
        (
            "hidden",
            if mark.hidden {
                Some("1".into())
            } else if default_hidden {
                Some("0".into())
            } else {
                None
            },
        ),
        (
            "outlineLevel",
            (mark.level > 0).then(|| mark.level.to_string()),
        ),
        ("collapsed", mark.collapsed.then(|| "1".into())),
    ]
}

/// Whether a mark says anything a missing row or column would not.
fn worth_writing(mark: &OutlineMark, default_hidden: bool) -> bool {
    mark.level > 0 || mark.collapsed || mark.hidden != default_hidden
}

fn new_rows(
    marks: &mut std::iter::Peekable<std::collections::btree_map::Iter<'_, u32, OutlineMark>>,
    before: u32,
    prefix: &str,
    default_hidden: bool,
) -> String {
    let mut out = String::new();
    while let Some((&row, mark)) = marks.next_if(|(&row, _)| row < before) {
        if !worth_writing(mark, default_hidden) {
            continue;
        }
        let mut text = format!("<{prefix}row r=\"{}\"", row + 1);
        for (key, value) in mark_attributes(mark, default_hidden) {
            if let Some(value) = value {
                text.push_str(&format!(" {key}=\"{value}\""));
            }
        }
        text.push_str("/>");
        out.push_str(&text);
    }
    out
}

fn row_splices(
    xml: &str,
    marks: &BTreeMap<u32, OutlineMark>,
    default_hidden: bool,
) -> Result<Vec<Splice>> {
    let mut reader = Reader::from_str(xml);
    let mut pending = marks.iter().peekable();
    let mut splices = Vec::new();
    let mut prefix = String::new();
    let mut in_data = false;
    let mut next_row = 0;
    let mut buf = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        let end = reader.buffer_position() as usize;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let name = element.name().as_ref().to_vec();
                match local(&name) {
                    b"sheetData" => {
                        prefix = namespace_prefix(&name);
                        if empty {
                            let rows =
                                new_rows(&mut pending, MAX_ROW_COUNT, &prefix, default_hidden);
                            splices.push(Splice {
                                start,
                                end,
                                text: format!("<{prefix}sheetData>{rows}</{prefix}sheetData>"),
                            });
                        } else {
                            in_data = true;
                        }
                    }
                    b"row" if in_data => {
                        let row = attribute(element, b"r")?
                            .and_then(|row| row.parse::<u32>().ok())
                            .and_then(|row| row.checked_sub(1))
                            .unwrap_or(next_row);
                        next_row = row + 1;
                        let inserted = new_rows(&mut pending, row, &prefix, default_hidden);
                        if !inserted.is_empty() {
                            splices.push(Splice {
                                start,
                                end: start,
                                text: inserted,
                            });
                        }
                        if let Some((_, mark)) = pending.next_if(|(&marked, _)| marked == row) {
                            splices.push(Splice {
                                start,
                                end,
                                text: rewrite(
                                    element,
                                    &mark_attributes(mark, default_hidden),
                                    empty,
                                )?,
                            });
                        }
                        if !empty {
                            reader.read_to_end_into(QName(&name), &mut Vec::new())?;
                        }
                    }
                    _ => {}
                }
            }
            Event::End(ref element)
                if in_data && local(element.name().as_ref()) == b"sheetData" =>
            {
                in_data = false;
                let inserted = new_rows(&mut pending, MAX_ROW_COUNT, &prefix, default_hidden);
                if !inserted.is_empty() {
                    splices.push(Splice {
                        start,
                        end: start,
                        text: inserted,
                    });
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    if pending.peek().is_some() {
        return Err(WriteError::MissingPart("sheetData".into()));
    }
    Ok(splices)
}

fn column_splices(
    xml: &str,
    marks: &BTreeMap<u32, OutlineMark>,
    default_width: f64,
) -> Result<Vec<Splice>> {
    let mut reader = Reader::from_str(xml);
    let mut prefix = String::new();
    // The `<cols>` element's span and start tag, and its `<col>`s by first
    // column, rewritten where marked.
    let mut columns: Option<(usize, usize, String)> = None;
    let mut entries: Vec<(u32, String)> = Vec::new();
    let mut covered = std::collections::BTreeSet::new();
    let mut insert_at = None;
    let mut in_columns = false;
    let mut buf = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        let end = reader.buffer_position() as usize;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let name = element.name().as_ref().to_vec();
                match local(&name) {
                    b"worksheet" => prefix = namespace_prefix(&name),
                    b"cols" => {
                        columns = Some((start, end, rewrite(element, &[], false)?));
                        in_columns = !empty;
                    }
                    b"col" if in_columns => {
                        let first = attribute(element, b"min")?
                            .and_then(|value| value.parse::<u32>().ok())
                            .unwrap_or(1)
                            .saturating_sub(1);
                        let last = attribute(element, b"max")?
                            .and_then(|value| value.parse::<u32>().ok())
                            .unwrap_or(first + 1)
                            .saturating_sub(1)
                            .min(MAX_COLUMN_COUNT - 1)
                            .max(first);
                        if !empty {
                            reader.read_to_end_into(QName(&name), &mut Vec::new())?;
                        }
                        let finish = reader.buffer_position() as usize;
                        if marks.range(first..=last).next().is_none() {
                            entries.push((first, xml[start..finish].to_string()));
                            continue;
                        }
                        covered.extend(first..=last);
                        // Split the span where the marks differ.
                        let mut at = first;
                        while at <= last {
                            let mark = marks.get(&at);
                            let mut to = at;
                            while to < last && marks.get(&(to + 1)) == mark {
                                to += 1;
                            }
                            let mut set = vec![
                                ("min", Some((at + 1).to_string())),
                                ("max", Some((to + 1).to_string())),
                            ];
                            if let Some(mark) = mark {
                                set.extend(mark_attributes(mark, false));
                            }
                            entries.push((at, rewrite(element, &set, true)?));
                            at = to + 1;
                        }
                    }
                    b"sheetData" => {
                        insert_at = Some(start);
                        break;
                    }
                    _ => {}
                }
            }
            Event::End(ref element) if in_columns && local(element.name().as_ref()) == b"cols" => {
                in_columns = false;
                if let Some((_, close, _)) = columns.as_mut() {
                    *close = end;
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    // Marked columns no `<col>` covered get their own, runs of the same mark
    // as one.
    let mut fresh = marks
        .iter()
        .filter(|(column, mark)| !covered.contains(*column) && worth_writing(mark, false))
        .peekable();
    while let Some((&first, mark)) = fresh.next() {
        let mut last = first;
        while fresh
            .next_if(|(&column, next)| column == last + 1 && *next == mark)
            .is_some()
        {
            last += 1;
        }
        let mut text = format!(
            "<{prefix}col min=\"{}\" max=\"{}\" width=\"{default_width}\"",
            first + 1,
            last + 1
        );
        for (key, value) in mark_attributes(mark, false) {
            if let Some(value) = value {
                text.push_str(&format!(" {key}=\"{value}\""));
            }
        }
        text.push_str("/>");
        entries.push((first, text));
    }
    entries.sort_by_key(|(first, _)| *first);
    let content: String = entries.into_iter().map(|(_, text)| text).collect();
    Ok(vec![match columns {
        Some((start, end, head)) => Splice {
            start,
            end,
            text: format!("{head}{content}</{prefix}cols>"),
        },
        None => {
            let at = insert_at.ok_or_else(|| WriteError::MissingPart("sheetData".into()))?;
            Splice {
                start: at,
                end: at,
                text: format!("<{prefix}cols>{content}</{prefix}cols>"),
            }
        }
    }])
}

/// `sheetFormatPr` saying the deepest level, which Excel sizes the outline
/// gutter by; one is added before `<cols>` or `<sheetData>` when the sheet
/// has none.
fn format_splice(xml: &str, axis: Axis, deepest: u8) -> Result<Option<Splice>> {
    let key = match axis {
        Axis::Rows => "outlineLevelRow",
        Axis::Columns => "outlineLevelCol",
    };
    let value = (deepest > 0).then(|| deepest.to_string());
    let mut reader = Reader::from_str(xml);
    let mut prefix = String::new();
    let mut buf = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        let end = reader.buffer_position() as usize;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let name = element.name().as_ref().to_vec();
                match local(&name) {
                    b"worksheet" => prefix = namespace_prefix(&name),
                    b"sheetFormatPr" => {
                        if attribute(element, key.as_bytes())? == value {
                            return Ok(None);
                        }
                        return Ok(Some(Splice {
                            start,
                            end,
                            text: rewrite(element, &[(key, value)], empty)?,
                        }));
                    }
                    b"cols" | b"sheetData" => {
                        let Some(value) = value else {
                            return Ok(None);
                        };
                        return Ok(Some(Splice {
                            start,
                            end: start,
                            text: format!(
                                "<{prefix}sheetFormatPr defaultRowHeight=\"15\" {key}=\"{value}\"/>"
                            ),
                        }));
                    }
                    _ => {}
                }
            }
            Event::Eof => return Ok(None),
            _ => {}
        }
        buf.clear();
    }
}

/// Data ▸ Group ▸ Auto Outline: the outline the sheet's summary formulas
/// imply, for rows and for columns. A formula whose range runs down its own
/// column and ends right above it (`=SUM(B2:B5)` in B6) groups those rows
/// under it; one whose range runs along its own row and ends right left of
/// it groups those columns. A group inside another goes a level deeper.
/// What the sheet had is replaced; hidden rows and columns stay hidden.
pub fn plan_auto_outline(
    sheet: &Sheet,
) -> Result<(BTreeMap<u32, OutlineMark>, BTreeMap<u32, OutlineMark>)> {
    let mut row_groups = std::collections::BTreeSet::new();
    let mut column_groups = std::collections::BTreeSet::new();
    for row in &sheet.rows {
        for cell in &row.cells {
            let Some(formula) = &cell.formula else {
                continue;
            };
            let at = cell.reference;
            for (start, end) in local_ranges(formula) {
                if start.column == at.column
                    && end.column == at.column
                    && end.row + 1 == at.row
                    && start.row <= end.row
                {
                    row_groups.insert((start.row, end.row));
                } else if start.row == at.row
                    && end.row == at.row
                    && end.column + 1 == at.column
                    && start.column <= end.column
                {
                    column_groups.insert((start.column, end.column));
                }
            }
        }
    }
    Ok((
        replaced_outline(sheet, Axis::Rows, &row_groups)?,
        replaced_outline(sheet, Axis::Columns, &column_groups)?,
    ))
}

/// Data ▸ Ungroup ▸ Clear Outline: every level and collapsed mark on the
/// axis taken off, hidden rows or columns left as they are.
pub fn plan_clear_outline(sheet: &Sheet, axis: Axis) -> Result<BTreeMap<u32, OutlineMark>> {
    replaced_outline(sheet, axis, &std::collections::BTreeSet::new())
}

/// The marks that replace the axis's outline with `groups`, each a range of
/// detail; only those that change.
fn replaced_outline(
    sheet: &Sheet,
    axis: Axis,
    groups: &std::collections::BTreeSet<(u32, u32)>,
) -> Result<BTreeMap<u32, OutlineMark>> {
    let view = AxisView {
        sheet,
        axis,
        limit: match axis {
            Axis::Rows => MAX_ROW_COUNT,
            Axis::Columns => MAX_COLUMN_COUNT,
        },
        summary_after: true,
    };
    let mut levels: BTreeMap<u32, u8> = BTreeMap::new();
    for &(first, last) in groups {
        for index in first..=last {
            let level = levels.entry(index).or_default();
            if *level >= MAX_OUTLINE_LEVEL {
                return Err(WriteError::Outline("an outline has at most seven levels"));
            }
            *level += 1;
        }
    }
    let (old_levels, old_collapsed) = match axis {
        Axis::Rows => (&sheet.outline.row_levels, &sheet.outline.collapsed_rows),
        Axis::Columns => (&sheet.outline.column_levels, &sheet.outline.collapsed_columns),
    };
    let mut marks = BTreeMap::new();
    let touched = old_levels
        .keys()
        .chain(old_collapsed.iter())
        .chain(levels.keys())
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    for index in touched {
        let mut mark = view.mark(index);
        mark.level = levels.get(&index).copied().unwrap_or(0);
        mark.collapsed = false;
        if mark != view.mark(index) {
            marks.insert(index, mark);
        }
    }
    Ok(marks)
}

/// The ranges a formula names on its own sheet, as their corners: `B2:B5`
/// and `$B$2:$B$5`, not `Other!B2:B5` nor text in quotes.
fn local_ranges(formula: &str) -> Vec<(jade_xlsx_reader::CellRef, jade_xlsx_reader::CellRef)> {
    let bytes = formula.as_bytes();
    let mut ranges = Vec::new();
    let mut at = 0;
    let mut in_text = false;
    while at < bytes.len() {
        let byte = bytes[at];
        if byte == b'"' {
            in_text = !in_text;
            at += 1;
            continue;
        }
        let starts_token = at == 0
            || !(bytes[at - 1].is_ascii_alphanumeric()
                || matches!(bytes[at - 1], b'!' | b'_' | b'.' | b'$' | b'\''));
        if in_text || !starts_token || !(byte == b'$' || byte.is_ascii_uppercase()) {
            at += 1;
            continue;
        }
        let Some((start, after_start)) = reference_at(bytes, at) else {
            at += 1;
            continue;
        };
        if bytes.get(after_start) == Some(&b':') {
            if let Some((end, after_end)) = reference_at(bytes, after_start + 1) {
                let named_after = bytes
                    .get(after_end)
                    .is_some_and(|next| next.is_ascii_alphanumeric() || *next == b'(');
                if !named_after {
                    ranges.push((start, end));
                }
                at = after_end;
                continue;
            }
        }
        at = after_start;
    }
    ranges
}

/// A cell reference (`B2`, `$B$2`) starting at `at`, and where it ends.
fn reference_at(bytes: &[u8], at: usize) -> Option<(jade_xlsx_reader::CellRef, usize)> {
    let mut end = at;
    let skip_dollar = |end: &mut usize| {
        if bytes.get(*end) == Some(&b'$') {
            *end += 1;
        }
    };
    skip_dollar(&mut end);
    let letters = end;
    while bytes.get(end).is_some_and(u8::is_ascii_uppercase) && end - letters < 3 {
        end += 1;
    }
    if end == letters {
        return None;
    }
    skip_dollar(&mut end);
    let digits = end;
    while bytes.get(end).is_some_and(u8::is_ascii_digit) {
        end += 1;
    }
    if end == digits {
        return None;
    }
    let text: String = std::str::from_utf8(&bytes[at..end])
        .ok()?
        .chars()
        .filter(|character| *character != '$')
        .collect();
    let reference = jade_xlsx_reader::CellRef::parse(&text).ok()?;
    Some((reference, end))
}
