//! Conditional formats made, deleted and cleared, as Excel's Conditional
//! Formatting menu does: a new rule goes first (priority 1, the others one
//! lower), in its own `<conditionalFormatting>` before the sheet's others;
//! its look is a differential style appended to the styles part's `dxfs`.
//!
//! Existing rules are edited in place, byte for byte — a rule this does not
//! write (an `x14` extension, a formula rule) keeps everything it had.

use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

use jade_xlsx_reader::{CellRange, CellRef};

use crate::patch::{apply, attribute, escape, local, namespace_prefix, Splice};
use crate::shift::range_text;
use crate::{Result, WriteError};

/// A rule from Excel's Conditional Formatting menu.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NewConditionalRule {
    /// Highlight Cells Rules: a value or formula each, as typed.
    GreaterThan(String),
    LessThan(String),
    Between(String, String),
    EqualTo(String),
    TextContains(String),
    DuplicateValues,
    /// Top/Bottom Rules: N items, or N percent of them.
    Top {
        rank: u32,
        percent: bool,
    },
    Bottom {
        rank: u32,
        percent: bool,
    },
    AboveAverage,
    BelowAverage,
    /// A data bar in `RRGGBB`.
    DataBar(u32),
    /// Two or three colours, `RRGGBB`, from the lowest value to the highest.
    ColorScale(Vec<u32>),
}

/// The looks Excel's highlight rules offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConditionalStyle {
    LightRedFillDarkRedText,
    YellowFillDarkYellowText,
    GreenFillDarkGreenText,
    LightRedFill,
    RedText,
    /// For data bars and colour scales, which draw themselves.
    None,
}

impl ConditionalStyle {
    /// The `<dxf>` it is written as, when it has one.
    fn dxf(self, prefix: &str) -> Option<String> {
        let (fill, text): (Option<&str>, Option<&str>) = match self {
            ConditionalStyle::LightRedFillDarkRedText => (Some("FFC7CE"), Some("9C0006")),
            ConditionalStyle::YellowFillDarkYellowText => (Some("FFEB9C"), Some("9C5700")),
            ConditionalStyle::GreenFillDarkGreenText => (Some("C6EFCE"), Some("006100")),
            ConditionalStyle::LightRedFill => (Some("FFC7CE"), None),
            ConditionalStyle::RedText => (None, Some("9C0006")),
            ConditionalStyle::None => return None,
        };
        let mut dxf = format!("<{prefix}dxf>");
        if let Some(text) = text {
            dxf.push_str(&format!(
                "<{prefix}font><{prefix}color rgb=\"FF{text}\"/></{prefix}font>"
            ));
        }
        if let Some(fill) = fill {
            dxf.push_str(&format!(
                "<{prefix}fill><{prefix}patternFill><{prefix}bgColor rgb=\"FF{fill}\"/></{prefix}patternFill></{prefix}fill>"
            ));
        }
        dxf.push_str(&format!("</{prefix}dxf>"));
        Some(dxf)
    }
}

/// Worksheet children that come after `conditionalFormatting`, in schema
/// order: a sheet without one gets it before the first of these.
const FOLLOWING: &[&[u8]] = &[
    b"dataValidations",
    b"hyperlinks",
    b"printOptions",
    b"pageMargins",
    b"pageSetup",
    b"headerFooter",
    b"rowBreaks",
    b"colBreaks",
    b"customProperties",
    b"cellWatches",
    b"ignoredErrors",
    b"smartTags",
    b"drawing",
    b"legacyDrawing",
    b"legacyDrawingHF",
    b"drawingHF",
    b"picture",
    b"oleObjects",
    b"controls",
    b"webPublishItems",
    b"tableParts",
    b"extLst",
];

/// One `<conditionalFormatting>` as the file has it: a worksheet-level
/// one, or Excel 2010's `x14:conditionalFormatting` in the sheet's `extLst`.
struct Block {
    span: (usize, usize),
    /// The opening tag's span.
    head: (usize, usize),
    /// Its ranges, and for an x14 block where its `xm:sqref` text is; a
    /// worksheet block writes them in its `sqref`.
    sqref: String,
    sqref_text: Option<(usize, usize)>,
    rules: Vec<Rule>,
    /// The `<ext>` an x14 block is in, by its place in the `extLst`.
    ext: Option<usize>,
}

struct Rule {
    span: (usize, usize),
    /// The opening tag's span.
    head: (usize, usize),
    priority: Option<u32>,
    /// An x14 rule's `id`, and a worksheet rule's `x14:id` naming the x14
    /// rule that is its other half.
    id: Option<String>,
    link: Option<String>,
}

struct SheetLayout {
    prefix: String,
    blocks: Vec<Block>,
    x14: Vec<Block>,
    /// The worksheet's `extLst` and its `ext` children.
    ext_list: Option<(usize, usize)>,
    exts: Vec<(usize, usize)>,
    /// Where a first block goes in a sheet without any.
    insert_at: usize,
}

/// Which rules a change takes out: `(x14, block, rule)`.
type RuleKey = (bool, usize, usize);

impl SheetLayout {
    fn block(&self, x14: bool, index: usize) -> &Block {
        if x14 {
            &self.x14[index]
        } else {
            &self.blocks[index]
        }
    }

    /// Every rule in the order Excel's Rules Manager lists them — the
    /// order the reader gives them in, by priority: the worksheet's own,
    /// then the x14 rules standing alone. An x14 rule a worksheet rule
    /// links to is that rule's other half, not a rule of its own.
    fn listed(&self) -> Vec<RuleKey> {
        let linked: Vec<&str> = self
            .blocks
            .iter()
            .flat_map(|block| &block.rules)
            .filter_map(|rule| rule.link.as_deref())
            .collect();
        let mut keys: Vec<RuleKey> = Vec::new();
        for (x14, blocks) in [(false, &self.blocks), (true, &self.x14)] {
            for (index, block) in blocks.iter().enumerate() {
                for (rule_index, rule) in block.rules.iter().enumerate() {
                    if x14 && rule.id.as_deref().is_some_and(|id| linked.contains(&id)) {
                        continue;
                    }
                    keys.push((x14, index, rule_index));
                }
            }
        }
        keys.sort_by_key(|(x14, block, rule)| {
            self.block(*x14, *block).rules[*rule]
                .priority
                .unwrap_or(u32::MAX)
        });
        keys
    }

    /// The x14 rule a worksheet rule links to.
    fn other_half(&self, rule: &Rule) -> Option<RuleKey> {
        let link = rule.link.as_deref()?;
        self.x14.iter().enumerate().find_map(|(index, block)| {
            block
                .rules
                .iter()
                .position(|rule| rule.id.as_deref() == Some(link))
                .map(|rule| (true, index, rule))
        })
    }

    /// Splices taking out `rules`, and `blocks` whole: a block goes with
    /// its last rule, an `ext` with its last x14 block, and the `extLst`
    /// with its last `ext`. Nested removals are covered by the outer one.
    fn removals(&self, rules: &[RuleKey], blocks: &[(bool, usize)]) -> Vec<Splice> {
        let cut = |span: (usize, usize)| Splice {
            start: span.0,
            end: span.1,
            text: String::new(),
        };
        let mut splices = Vec::new();
        let gone = |x14: bool, index: usize| {
            blocks.contains(&(x14, index))
                || (0..self.block(x14, index).rules.len())
                    .all(|rule| rules.contains(&(x14, index, rule)))
        };
        let mut x14_gone = vec![false; self.x14.len()];
        for (x14, blocks) in [(false, &self.blocks), (true, &self.x14)] {
            for (index, block) in blocks.iter().enumerate() {
                if gone(x14, index) {
                    splices.push(cut(block.span));
                    if x14 {
                        x14_gone[index] = true;
                    }
                } else {
                    for (rule_index, rule) in block.rules.iter().enumerate() {
                        if rules.contains(&(x14, index, rule_index)) {
                            splices.push(cut(rule.span));
                        }
                    }
                }
            }
        }
        let mut exts_gone = 0;
        for (index, ext) in self.exts.iter().enumerate() {
            let mut held = self
                .x14
                .iter()
                .enumerate()
                .filter(|(_, block)| block.ext == Some(index))
                .peekable();
            if held.peek().is_some() && held.all(|(block, _)| x14_gone[block]) {
                splices.push(cut(*ext));
                exts_gone += 1;
            }
        }
        if let Some(list) = self.ext_list {
            if exts_gone > 0 && exts_gone == self.exts.len() {
                splices.push(cut(list));
            }
        }
        splices
    }
}

fn scan(xml: &str) -> Result<SheetLayout> {
    enum Capture {
        Nothing,
        Link,
        Sqref,
    }
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    // Each open element's local name and where it starts.
    let mut stack: Vec<(Vec<u8>, usize)> = Vec::new();
    let mut layout = SheetLayout {
        prefix: String::new(),
        blocks: Vec::new(),
        x14: Vec::new(),
        ext_list: None,
        exts: Vec::new(),
        insert_at: 0,
    };
    let mut insert_before: Option<usize> = None;
    let mut root_end: Option<usize> = None;
    let mut block: Option<Block> = None;
    let mut rule: Option<Rule> = None;
    let mut capture = Capture::Nothing;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        let end = reader.buffer_position() as usize;
        let (name, opened, empty) = match &event {
            Event::Start(element) | Event::Empty(element) => {
                let empty = matches!(event, Event::Empty(_));
                let qualified = element.name().as_ref().to_vec();
                let name = local(&qualified).to_vec();
                let depth = stack.len();
                let parent = stack.last().map(|(name, _)| name.as_slice());
                if depth == 0 {
                    layout.prefix = namespace_prefix(&qualified);
                }
                match name.as_slice() {
                    b"conditionalFormatting"
                        if depth == 1 || parent == Some(b"conditionalFormattings") =>
                    {
                        block = Some(Block {
                            span: (start, end),
                            head: (start, end),
                            sqref: attribute(element, b"sqref")?.unwrap_or_default(),
                            sqref_text: None,
                            rules: Vec::new(),
                            ext: (depth > 1).then_some(layout.exts.len()),
                        });
                    }
                    b"cfRule" if parent == Some(b"conditionalFormatting") && block.is_some() => {
                        rule = Some(Rule {
                            span: (start, end),
                            head: (start, end),
                            priority: attribute(element, b"priority")?
                                .and_then(|value| value.parse().ok()),
                            id: attribute(element, b"id")?,
                            link: None,
                        });
                    }
                    b"id" if rule.is_some() => capture = Capture::Link,
                    b"sqref" if parent == Some(b"conditionalFormatting") && block.is_some() => {
                        capture = Capture::Sqref
                    }
                    other
                        if depth == 1 && insert_before.is_none() && FOLLOWING.contains(&other) =>
                    {
                        insert_before = Some(start);
                    }
                    _ => {}
                }
                (name, true, empty)
            }
            Event::End(element) => (local(element.name().as_ref()).to_vec(), false, false),
            Event::Text(text) => {
                let text = text.decode()?.trim().to_string();
                match capture {
                    Capture::Link => {
                        if let Some(rule) = rule.as_mut() {
                            rule.link = Some(text);
                        }
                    }
                    Capture::Sqref => {
                        if let Some(block) = block.as_mut() {
                            block.sqref = text;
                            block.sqref_text = Some((start, end));
                        }
                    }
                    Capture::Nothing => {}
                }
                buf.clear();
                continue;
            }
            Event::Eof => break,
            _ => {
                buf.clear();
                continue;
            }
        };
        if opened && !empty {
            stack.push((name, start));
            buf.clear();
            continue;
        }
        // An element closes: an empty one here, or the one on the stack.
        let (begin, depth) = if empty {
            (start, stack.len())
        } else {
            let (_, begin) = stack.pop().unwrap_or_default();
            (begin, stack.len())
        };
        let parent = stack.last().map(|(name, _)| name.as_slice());
        capture = Capture::Nothing;
        match name.as_slice() {
            b"cfRule" if parent == Some(b"conditionalFormatting") => {
                if let (Some(mut done), Some(block)) = (rule.take(), block.as_mut()) {
                    done.span = (begin, end);
                    block.rules.push(done);
                }
            }
            b"conditionalFormatting" if depth == 1 || parent == Some(b"conditionalFormattings") => {
                if let Some(mut done) = block.take() {
                    done.span = (begin, end);
                    if depth == 1 {
                        layout.blocks.push(done);
                    } else {
                        layout.x14.push(done);
                    }
                }
            }
            b"ext" if depth == 2 && parent == Some(b"extLst") => {
                layout.exts.push((begin, end));
            }
            b"extLst" if depth == 1 => layout.ext_list = Some((begin, end)),
            _ if depth == 0 => root_end = Some(start),
            _ => {}
        }
        buf.clear();
    }
    layout.insert_at = layout
        .blocks
        .first()
        .map(|block| block.span.0)
        .or(insert_before)
        .or(root_end)
        .ok_or(WriteError::NoRoot("worksheet"))?;
    Ok(layout)
}

/// The `<cfRule>` for `rule` over `range`, at `priority`.
fn rule_xml(
    prefix: &str,
    rule: &NewConditionalRule,
    range: CellRange,
    dxf: Option<usize>,
    priority: u32,
) -> String {
    let dxf = dxf
        .map(|index| format!(" dxfId=\"{index}\""))
        .unwrap_or_default();
    let formula = |value: &str| {
        format!(
            "<{prefix}formula>{}</{prefix}formula>",
            escape(value.trim().strip_prefix('=').unwrap_or(value.trim()))
        )
    };
    let cell_is = |operator: &str, values: &[&str]| {
        format!(
            "<{prefix}cfRule type=\"cellIs\"{dxf} priority=\"{priority}\" operator=\"{operator}\">{}</{prefix}cfRule>",
            values.iter().map(|value| formula(value)).collect::<String>()
        )
    };
    let color = |rgb: u32| format!("<{prefix}color rgb=\"FF{rgb:06X}\"/>");
    match rule {
        NewConditionalRule::GreaterThan(value) => cell_is("greaterThan", &[value]),
        NewConditionalRule::LessThan(value) => cell_is("lessThan", &[value]),
        NewConditionalRule::Between(low, high) => cell_is("between", &[low, high]),
        NewConditionalRule::EqualTo(value) => cell_is("equal", &[value]),
        NewConditionalRule::TextContains(text) => {
            let at = crate::shift::reference_text(range.start);
            let quoted = text.replace('"', "\"\"");
            format!(
                "<{prefix}cfRule type=\"containsText\"{dxf} priority=\"{priority}\" operator=\"containsText\" text=\"{}\">{}</{prefix}cfRule>",
                escape(text),
                formula(&format!("NOT(ISERROR(SEARCH(\"{quoted}\",{at})))"))
            )
        }
        NewConditionalRule::DuplicateValues => {
            format!("<{prefix}cfRule type=\"duplicateValues\"{dxf} priority=\"{priority}\"/>")
        }
        NewConditionalRule::Top { rank, percent } | NewConditionalRule::Bottom { rank, percent } => {
            format!(
                "<{prefix}cfRule type=\"top10\"{dxf} priority=\"{priority}\"{}{} rank=\"{rank}\"/>",
                if *percent { " percent=\"1\"" } else { "" },
                if matches!(rule, NewConditionalRule::Bottom { .. }) {
                    " bottom=\"1\""
                } else {
                    ""
                }
            )
        }
        NewConditionalRule::AboveAverage => {
            format!("<{prefix}cfRule type=\"aboveAverage\"{dxf} priority=\"{priority}\"/>")
        }
        NewConditionalRule::BelowAverage => format!(
            "<{prefix}cfRule type=\"aboveAverage\"{dxf} priority=\"{priority}\" aboveAverage=\"0\"/>"
        ),
        NewConditionalRule::DataBar(rgb) => format!(
            "<{prefix}cfRule type=\"dataBar\" priority=\"{priority}\"><{prefix}dataBar><{prefix}cfvo type=\"min\"/><{prefix}cfvo type=\"max\"/>{}</{prefix}dataBar></{prefix}cfRule>",
            color(*rgb)
        ),
        NewConditionalRule::ColorScale(colors) => {
            let thresholds = if colors.len() >= 3 {
                format!(
                    "<{prefix}cfvo type=\"min\"/><{prefix}cfvo type=\"percentile\" val=\"50\"/><{prefix}cfvo type=\"max\"/>"
                )
            } else {
                format!("<{prefix}cfvo type=\"min\"/><{prefix}cfvo type=\"max\"/>")
            };
            format!(
                "<{prefix}cfRule type=\"colorScale\" priority=\"{priority}\"><{prefix}colorScale>{thresholds}{}</{prefix}colorScale></{prefix}cfRule>",
                colors.iter().take(3).map(|rgb| color(*rgb)).collect::<String>()
            )
        }
    }
}

/// The opening tag at `span` with `key` set to `value`.
fn with_attribute(xml: &str, span: (usize, usize), key: &[u8], value: &str) -> Result<String> {
    let mut reader = Reader::from_str(&xml[span.0..span.1]);
    let mut buf = Vec::new();
    match reader.read_event_into(&mut buf)? {
        Event::Start(element) => {
            crate::patch::element_head_with_attribute(&element, key, value, false)
        }
        Event::Empty(element) => {
            crate::patch::element_head_with_attribute(&element, key, value, true)
        }
        _ => Ok(xml[span.0..span.1].to_string()),
    }
}

/// The styles part with `dxf` appended to its `dxfs`, and the new index.
pub fn append_dxf(
    styles: &str,
    dxf_for: impl Fn(&str) -> Option<String>,
) -> Result<(String, Option<usize>)> {
    let mut reader = Reader::from_str(styles);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut depth = 0usize;
    let mut prefix = String::new();
    let mut dxfs: Option<(usize, usize, bool)> = None;
    let mut dxfs_end: Option<usize> = None;
    let mut count = 0usize;
    let mut insert_before: Option<usize> = None;
    let mut root_end: Option<usize> = None;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        let end = reader.buffer_position() as usize;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let qualified = element.name().as_ref().to_vec();
                let name = local(&qualified);
                if depth == 0 {
                    prefix = namespace_prefix(&qualified);
                } else if depth == 1 {
                    if name == b"dxfs" {
                        dxfs = Some((start, end, empty));
                    } else if insert_before.is_none()
                        && matches!(name, b"tableStyles" | b"colors" | b"extLst")
                    {
                        insert_before = Some(start);
                    }
                } else if depth == 2 && name == b"dxf" && dxfs.is_some() && dxfs_end.is_none() {
                    count += 1;
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(ref element) => {
                depth = depth.saturating_sub(1);
                if depth == 1 && local(element.name().as_ref()) == b"dxfs" {
                    dxfs_end = Some(start);
                }
                if depth == 0 {
                    root_end = Some(start);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    let Some(dxf) = dxf_for(&prefix) else {
        return Ok((styles.to_string(), None));
    };
    let mut splices = Vec::new();
    match (dxfs, dxfs_end) {
        (Some((start, end, false)), Some(close)) => {
            splices.push(Splice {
                start,
                end,
                text: with_attribute(styles, (start, end), b"count", &(count + 1).to_string())?,
            });
            splices.push(Splice {
                start: close,
                end: close,
                text: dxf,
            });
        }
        (Some((start, end, true)), _) => splices.push(Splice {
            start,
            end,
            text: format!("<{prefix}dxfs count=\"1\">{dxf}</{prefix}dxfs>"),
        }),
        _ => {
            let at = insert_before
                .or(root_end)
                .ok_or(WriteError::NoRoot("styles"))?;
            splices.push(Splice {
                start: at,
                end: at,
                text: format!("<{prefix}dxfs count=\"1\">{dxf}</{prefix}dxfs>"),
            });
        }
    }
    Ok((apply(styles, splices), Some(count)))
}

/// The worksheet and styles parts with `rule` over `range` added first, in
/// `style`: `(sheet, styles)`. Every other rule — Excel 2010's x14 ones
/// too, which share the numbering — moves one lower.
pub fn add_conditional_format(
    sheet: &str,
    styles: &str,
    range: CellRange,
    rule: &NewConditionalRule,
    style: ConditionalStyle,
) -> Result<(String, String)> {
    let (styles, dxf) = match rule {
        NewConditionalRule::DataBar(_) | NewConditionalRule::ColorScale(_) => {
            (styles.to_string(), None)
        }
        _ => append_dxf(styles, |prefix| style.dxf(prefix))?,
    };
    let layout = scan(sheet)?;
    let prefix = &layout.prefix;
    let mut splices = Vec::new();
    for rule in layout
        .blocks
        .iter()
        .chain(&layout.x14)
        .flat_map(|block| &block.rules)
    {
        if let Some(priority) = rule.priority {
            splices.push(Splice {
                start: rule.head.0,
                end: rule.head.1,
                text: with_attribute(sheet, rule.head, b"priority", &(priority + 1).to_string())?,
            });
        }
    }
    splices.push(Splice {
        start: layout.insert_at,
        end: layout.insert_at,
        text: format!(
            "<{prefix}conditionalFormatting sqref=\"{}\">{}</{prefix}conditionalFormatting>",
            escape(&range_text(range)),
            rule_xml(prefix, rule, range, dxf, 1)
        ),
    });
    Ok((apply(sheet, splices), styles))
}

/// The worksheet with the rule `index` — in priority order, as Excel's
/// Rules Manager lists them — deleted, with its x14 half: a block goes with
/// its last rule.
pub fn delete_conditional_format(sheet: &str, index: usize) -> Result<String> {
    let layout = scan(sheet)?;
    let listed = layout.listed();
    let Some(&(x14, block, rule)) = listed.get(index) else {
        return Err(WriteError::NoSuchConditionalFormat(index));
    };
    let mut rules = vec![(x14, block, rule)];
    if !x14 {
        rules.extend(layout.other_half(&layout.blocks[block].rules[rule]));
    }
    Ok(apply(sheet, layout.removals(&rules, &[])))
}

/// What is left of `range` once `cut` is taken out: up to four ranges.
fn subtract(range: CellRange, cut: CellRange) -> Vec<CellRange> {
    let overlaps = range.start.column <= cut.end.column
        && cut.start.column <= range.end.column
        && range.start.row <= cut.end.row
        && cut.start.row <= range.end.row;
    if !overlaps {
        return vec![range];
    }
    let mut out = Vec::new();
    let (top, bottom) = (
        cut.start.row.max(range.start.row),
        cut.end.row.min(range.end.row),
    );
    if range.start.row < top {
        out.push(CellRange {
            start: range.start,
            end: CellRef::new(range.end.column, top - 1),
        });
    }
    if bottom < range.end.row {
        out.push(CellRange {
            start: CellRef::new(range.start.column, bottom + 1),
            end: range.end,
        });
    }
    if range.start.column < cut.start.column {
        out.push(CellRange {
            start: CellRef::new(range.start.column, top),
            end: CellRef::new(cut.start.column - 1, bottom),
        });
    }
    if cut.end.column < range.end.column {
        out.push(CellRange {
            start: CellRef::new(cut.end.column + 1, top),
            end: CellRef::new(range.end.column, bottom),
        });
    }
    out
}

/// The worksheet with its conditional formats cleared: all of them, or —
/// as Clear Rules from Selected Cells does — only from `range`, a rule
/// keeping the rest of the cells it applied to. Excel 2010's x14 halves
/// are cleared alike.
pub fn clear_conditional_formats(sheet: &str, range: Option<CellRange>) -> Result<String> {
    let layout = scan(sheet)?;
    let mut splices = Vec::new();
    let mut emptied = Vec::new();
    for (x14, blocks) in [(false, &layout.blocks), (true, &layout.x14)] {
        for (index, block) in blocks.iter().enumerate() {
            let Some(cut) = range else {
                emptied.push((x14, index));
                continue;
            };
            let ranges: Vec<CellRange> = block
                .sqref
                .split_whitespace()
                .filter_map(|text| CellRange::parse(text).ok())
                .collect();
            let left: Vec<CellRange> = ranges
                .iter()
                .flat_map(|range| subtract(*range, cut))
                .collect();
            if left == ranges {
                continue;
            }
            if left.is_empty() {
                emptied.push((x14, index));
                continue;
            }
            let sqref = left
                .iter()
                .map(|range| range_text(*range))
                .collect::<Vec<_>>()
                .join(" ");
            match block.sqref_text {
                Some((start, end)) => splices.push(Splice {
                    start,
                    end,
                    text: escape(&sqref),
                }),
                None => splices.push(Splice {
                    start: block.head.0,
                    end: block.head.1,
                    text: with_attribute(sheet, block.head, b"sqref", &sqref)?,
                }),
            }
        }
    }
    splices.extend(layout.removals(&[], &emptied));
    Ok(apply(sheet, splices))
}
