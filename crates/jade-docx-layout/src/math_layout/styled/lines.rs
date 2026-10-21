//! Display lines: an equation broken where the author put `m:brk` or where it
//! runs out of width, its continuation lines aligned as Word aligns them, and
//! the lines of an `m:oMathPara` stacked and justified.
use super::{
    shapes, AtomClass, BreakBinary, Builder, Mark, MathJustification, MathLine, StyledMathMeasure,
};
use crate::math_layout::MathMetrics;
use jade_docx_reader::document::MathElement;

/// A place a display line may break: before an operator at the top level of
/// the equation, or where a run asks for a manual break.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BreakCandidate {
    pub(crate) mark: Mark,
    pub(crate) class: AtomClass,
    pub(crate) manual: bool,
    /// `m:brk m:alnAt`: align under the n-th operator of the line above.
    pub(crate) align_at: Option<u32>,
    /// `m:aln`: an alignment point rather than a break.
    pub(crate) align_point: bool,
}

/// A laid-out line before stacking: the plan items in `start..end`, on a
/// baseline at 0 and starting at x 0.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PlacedLine {
    pub(crate) start: Mark,
    pub(crate) end: Mark,
    pub(crate) metrics: MathMetrics,
    /// Lines of one equation share a group and are justified together.
    pub(crate) group: usize,
    /// Where the line starts within its group.
    pub(crate) x: f64,
    /// The x of its alignment point, if it has one.
    pub(crate) align: Option<f64>,
}

/// Lays out an `m:oMath` on one line, then breaks it.
pub(crate) fn math_lines<'a, C: StyledMathMeasure>(
    b: &mut Builder<'_, 'a, C>,
    math: &'a MathElement,
    group: usize,
) -> Result<Vec<PlacedLine>, C::Error> {
    let start = b.mark();
    let index = b.open_box(math, start);
    let saved = (b.depth, b.prev.take(), b.breaks.take());
    b.depth = 0;
    b.breaks = Some(Vec::new());
    let metrics = b.sequence(&math.children)?;
    let breaks = b.breaks.take().unwrap_or_default();
    b.depth = saved.0;
    b.prev = saved.1;
    b.breaks = saved.2;
    b.close_box(index, metrics);
    let end = b.mark();
    if !b.display || b.options.uniform {
        return Ok(vec![PlacedLine {
            start,
            end,
            metrics,
            group,
            x: 0.0,
            align: None,
        }]);
    }
    Ok(break_lines(b, start, end, metrics, breaks, group))
}

/// Whether a run asks for a manual break before it.
#[allow(dead_code)]
pub(crate) fn run_break(element: &MathElement) -> bool {
    element
        .children
        .iter()
        .filter(|c| c.tag.as_deref() == Some("rPr") && c.namespace.is_none())
        .any(|pr| pr.children.iter().any(|p| p.tag.as_deref() == Some("brk")))
}

fn break_lines<C: StyledMathMeasure>(
    b: &mut Builder<'_, '_, C>,
    start: Mark,
    end: Mark,
    metrics: MathMetrics,
    breaks: Vec<BreakCandidate>,
    group: usize,
) -> Vec<PlacedLine> {
    // The x of the glyph a candidate stands before, once laid out.
    let x_of = |b: &Builder<'_, '_, C>, mark: Mark| -> Option<f64> {
        b.plan.glyphs.get(mark.glyphs).map(|g| g.x)
    };
    // One candidate per glyph; a manual break wins.
    let mut candidates: Vec<BreakCandidate> = Vec::new();
    let mut align_points: Vec<Mark> = Vec::new();
    for c in breaks {
        if c.align_point {
            align_points.push(c.mark);
            continue;
        }
        if c.mark.glyphs >= end.glyphs || c.mark.glyphs <= start.glyphs {
            continue;
        }
        match candidates.last_mut() {
            Some(last) if last.mark.glyphs == c.mark.glyphs => {
                if c.manual {
                    *last = c;
                }
            }
            _ => candidates.push(c),
        }
    }
    let max_width = b.options.max_width.filter(|w| *w > 0.0);
    let settings = b.options.settings;
    // Choose the breaks: every manual one, then automatic ones where a line
    // would run past the width.
    let mut chosen: Vec<(BreakCandidate, f64)> = Vec::new();
    let mut line_start_x = 0.0;
    let mut indent = 0.0;
    let mut last_fit: Option<(BreakCandidate, f64)> = None;
    for c in &candidates {
        let Some(x) = x_of(b, c.mark) else {
            continue;
        };
        if c.manual {
            chosen.push((*c, x));
            line_start_x = x;
            indent = 0.0;
            last_fit = None;
            continue;
        }
        if let Some(width) = max_width {
            if x - line_start_x + indent > width {
                if let Some((fit, fit_x)) = last_fit.take() {
                    chosen.push((fit, fit_x));
                    line_start_x = fit_x;
                    indent = if settings.wrap_right {
                        0.0
                    } else {
                        settings.wrap_indent_px
                    };
                }
            }
        }
        last_fit = Some((*c, x));
    }
    // The end of the equation may still overflow the last line.
    if let (Some(width), Some((fit, fit_x))) = (max_width, last_fit) {
        if metrics.width - line_start_x + indent > width
            && chosen
                .last()
                .is_none_or(|(c, _)| c.mark.glyphs < fit.mark.glyphs)
        {
            chosen.push((fit, fit_x));
        }
    }
    if chosen.is_empty() && align_points.is_empty() {
        return vec![PlacedLine {
            start,
            end,
            metrics,
            group,
            x: 0.0,
            align: None,
        }];
    }
    // brkBin="after" keeps the operator at the end of the line above.
    let after = settings.break_binary == BreakBinary::After;
    let mut bounds: Vec<(Mark, f64, Option<BreakCandidate>)> = vec![(start, 0.0, None)];
    for (c, x) in &chosen {
        let mut mark = c.mark;
        let mut at = *x;
        if after && !c.manual && c.class == AtomClass::Bin {
            if let Some(g) = b.plan.glyphs.get(mark.glyphs) {
                at = g.x + g.width;
                mark.glyphs += 1;
            }
        }
        bounds.push((mark, at, Some(*c)));
    }
    let mut lines = Vec::new();
    // The x of every operator on the first line, for alnAt and the default
    // alignment under its first relation.
    let first_end = bounds.get(1).map_or(end, |b| b.0);
    let first_operators: Vec<f64> = candidates
        .iter()
        .filter(|c| c.mark.glyphs < first_end.glyphs)
        .filter_map(|c| x_of(b, c.mark))
        .collect();
    let first_relation = candidates
        .iter()
        .filter(|c| c.mark.glyphs < first_end.glyphs && c.class == AtomClass::Rel)
        .find_map(|c| x_of(b, c.mark));
    for i in 0..bounds.len() {
        let (line_start, origin_x, candidate) = bounds[i];
        let line_end = bounds.get(i + 1).map_or(end, |b| b.0);
        let extent = extent(b, line_start, line_end);
        b.shift_range(line_start, line_end, -origin_x, 0.0);
        let width = (extent.2 - origin_x).max(0.0);
        let x = match candidate {
            None => 0.0,
            Some(c) if c.manual => match c.align_at {
                Some(n) if n >= 1 => first_operators.get(n as usize - 1).copied().unwrap_or(0.0),
                _ => first_relation.unwrap_or(settings.wrap_indent_px.min(96.0)),
            },
            Some(_) => {
                if settings.wrap_right {
                    max_width.map_or(0.0, |w| (w - width).max(0.0))
                } else {
                    settings.wrap_indent_px
                }
            }
        };
        let align = align_points
            .iter()
            .find(|m| m.glyphs >= line_start.glyphs && m.glyphs < line_end.glyphs)
            .and_then(|m| b.plan.glyphs.get(m.glyphs))
            .map(|g| g.x);
        lines.push(PlacedLine {
            start: line_start,
            end: line_end,
            metrics: MathMetrics {
                width,
                ascent: extent.1,
                descent: extent.3,
            },
            group,
            x,
            align,
        });
    }
    lines
}

/// The ink of the plan items in a range: left, ascent, right, descent,
/// relative to baseline 0.
pub(crate) fn extent<C: StyledMathMeasure>(
    b: &Builder<'_, '_, C>,
    start: Mark,
    end: Mark,
) -> (f64, f64, f64, f64) {
    let mut left = f64::INFINITY;
    let mut right = f64::NEG_INFINITY;
    let mut ascent: f64 = 0.0;
    let mut descent: f64 = 0.0;
    for g in &b.plan.glyphs[start.glyphs..end.glyphs] {
        left = left.min(g.x);
        right = right.max(g.x + g.width);
        ascent = ascent.max(g.ascent - g.baseline);
        descent = descent.max(g.descent + g.baseline);
    }
    for r in &b.plan.rules[start.rules..end.rules] {
        left = left.min(r.x);
        right = right.max(r.x + r.width);
        ascent = ascent.max(-(r.y - r.thickness / 2.0));
        descent = descent.max(r.y + r.thickness / 2.0);
    }
    for s in &b.plan.shapes[start.shapes..end.shapes] {
        if let Some((l, t, r, btm)) = shapes::bounds(&s.commands) {
            left = left.min(l);
            right = right.max(r);
            ascent = ascent.max(-t);
            descent = descent.max(btm);
        }
    }
    for x in &b.plan.boxes[start.boxes..end.boxes] {
        if x.empty {
            left = left.min(x.x);
            right = right.max(x.x + x.width);
            ascent = ascent.max(x.ascent - x.baseline);
            descent = descent.max(x.descent + x.baseline);
        }
    }
    if !left.is_finite() {
        left = 0.0;
        right = 0.0;
    }
    (left, ascent, right, descent)
}

/// Stacks the lines, aligns their alignment points, justifies each group,
/// and records the lines in the plan. Returns the plan's metrics with the
/// first line's baseline at 0.
pub(crate) fn stack<C: StyledMathMeasure>(
    b: &mut Builder<'_, '_, C>,
    mut lines: Vec<PlacedLine>,
) -> MathMetrics {
    if lines.is_empty() {
        return MathMetrics::default();
    }
    if lines.len() == 1 && lines[0].x == 0.0 {
        let line = lines[0];
        b.plan.lines.push(MathLine {
            baseline: 0.0,
            x: 0.0,
            width: line.metrics.width,
            ascent: line.metrics.ascent,
            descent: line.metrics.descent,
        });
        return line.metrics;
    }
    // Alignment points line up within a group.
    let groups = lines.iter().map(|l| l.group).max().unwrap_or(0) + 1;
    for group in 0..groups {
        let target = lines
            .iter()
            .filter(|l| l.group == group)
            .filter_map(|l| l.align.map(|a| a + l.x))
            .fold(f64::NEG_INFINITY, f64::max);
        if target.is_finite() {
            for line in lines.iter_mut().filter(|l| l.group == group) {
                if let Some(a) = line.align {
                    line.x = target - a;
                }
            }
        }
    }
    let group_width = |lines: &[PlacedLine], group: usize| {
        lines
            .iter()
            .filter(|l| l.group == group)
            .map(|l| l.x + l.metrics.width)
            .fold(0.0, f64::max)
    };
    let width = (0..groups)
        .map(|g| group_width(&lines, g))
        .fold(0.0, f64::max);
    let justification = b.justification();
    let size = b.base_size;
    let gap = super::MathConstants::cambria(size).leading;
    let mut baseline = 0.0;
    let mut previous: Option<MathMetrics> = None;
    let mut total = MathMetrics::default();
    for line in &lines {
        if let Some(prev) = previous {
            baseline += (prev.descent + line.metrics.ascent + gap).max(size * 1.2);
        }
        let group_w = group_width(&lines, line.group);
        let group_x = match justification {
            MathJustification::Left => 0.0,
            MathJustification::Right => width - group_w,
            _ => (width - group_w) / 2.0,
        };
        let x = match justification {
            MathJustification::Center => group_x + (group_w - line.metrics.width) / 2.0,
            _ => group_x + line.x,
        };
        b.shift_range(line.start, line.end, x, baseline);
        b.plan.lines.push(MathLine {
            baseline,
            x,
            width: line.metrics.width,
            ascent: line.metrics.ascent,
            descent: line.metrics.descent,
        });
        total.ascent = total.ascent.max(line.metrics.ascent - baseline);
        total.descent = total.descent.max(line.metrics.descent + baseline);
        previous = Some(line.metrics);
    }
    total.width = width;
    total
}

impl<C: StyledMathMeasure> Builder<'_, '_, C> {
    /// How the lines of display math are justified (`m:oMathParaPr/m:jc`).
    pub(crate) fn justification(&self) -> MathJustification {
        self.justify
    }
}
