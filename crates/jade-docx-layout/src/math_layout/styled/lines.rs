//! Display lines: an equation broken where the author put `m:brk` or where it
//! runs out of width, its continuation lines aligned as Word aligns them, and
//! the lines of an `m:oMathPara` stacked and justified.
use super::{
    is_skipped, shapes, AtomClass, BreakBinary, BreakBinarySubtraction, Builder, Mark, MathGlyph,
    MathJustification, MathLine, StyledMathMeasure,
};
use crate::math_layout::MathMetrics;
use jade_docx_reader::document::{MathElement, MathElementKind};
use std::collections::HashMap;

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
    /// A subtraction character in a run, rather than a whole operator box.
    pub(crate) subtraction: bool,
    /// Space before this operator, omitted at the preceding line end.
    pub(crate) preceding_space: f64,
    /// Space before the next atom, omitted at the continuation's start.
    pub(crate) following_space: f64,
}

/// A laid-out line before stacking: the plan items in `start..end`, on a
/// baseline at 0 and starting at x 0.
#[derive(Debug)]
pub(crate) struct PlacedLine<'a> {
    pub(crate) start: Mark,
    pub(crate) end: Mark,
    pub(crate) metrics: MathMetrics,
    /// Lines of one equation share a group and are justified together.
    pub(crate) group: usize,
    /// Where the line starts within its group.
    pub(crate) x: f64,
    /// The x of its alignment point, if it has one.
    pub(crate) align: Option<f64>,
    /// A paint copy of the repeated operator at the line end, in local coordinates.
    pub(crate) trailing_operator: Option<MathGlyph<'a>>,
}

/// Lays out an `m:oMath` on one line, then breaks it.
pub(crate) fn math_lines<'a, C: StyledMathMeasure>(
    b: &mut Builder<'_, 'a, C>,
    math: &'a MathElement,
    group: usize,
) -> Result<Vec<PlacedLine<'a>>, C::Error> {
    let start = b.mark();
    let index = b.open_box(math, start);
    let saved = (b.depth, b.prev.take(), b.breaks.take());
    b.depth = 0;
    b.breaks = Some(Vec::new());
    // A standalone display array can use the formatting width. Arrays in
    // arguments or alongside other atoms do not own that width.
    let max_array_width = b.options.max_width.filter(|_| {
        if !b.display || b.options.uniform {
            return false;
        }
        let mut content = math.children.iter().filter(|child| !is_skipped(child));
        content
            .next()
            .is_some_and(|child| child.kind == MathElementKind::EquationArray)
            && content.next().is_none()
    });
    let metrics = b.sequence_with_array_width(&math.children, max_array_width)?;
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
            trailing_operator: None,
        }]);
    }
    break_lines(b, start, end, metrics, breaks, group)
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

fn break_lines<'a, C: StyledMathMeasure>(
    b: &mut Builder<'_, 'a, C>,
    start: Mark,
    end: Mark,
    metrics: MathMetrics,
    breaks: Vec<BreakCandidate>,
    group: usize,
) -> Result<Vec<PlacedLine<'a>>, C::Error> {
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
    let word_mac_rtl = b.options.word_mac_rtl();
    let prefers_relation = !word_mac_rtl
        && crate::line_metrics::word_layout_platform()
            == crate::line_metrics::WordLayoutPlatform::Mac;
    let breaks_after_operator =
        |class| class == AtomClass::Bin || prefers_relation && class == AtomClass::Rel;
    let substitute_start = settings.break_binary_subtraction == BreakBinarySubtraction::MinusPlus;
    let substitute_end = settings.break_binary_subtraction == BreakBinarySubtraction::PlusMinus;
    let repeats = |c: BreakCandidate| {
        settings.break_binary == BreakBinary::Repeat && !c.manual && breaks_after_operator(c.class)
    };
    let mut plus_signs = HashMap::new();
    if max_width.is_some_and(|width| metrics.width > width) && (substitute_start || substitute_end)
    {
        for c in candidates
            .iter()
            .copied()
            .filter(|c| c.subtraction && repeats(*c))
        {
            let source = &b.plan.glyphs[c.mark.glyphs];
            let (metrics, _, font_glyph) = b.measure_text("+", source.style)?;
            let mut plus = source.clone();
            plus.rendered_text = Some("+");
            plus.width = metrics.width;
            plus.ascent = metrics.ascent;
            plus.descent = metrics.descent;
            #[cfg(feature = "opentype-math")]
            {
                plus.font_glyph = font_glyph;
            }
            #[cfg(not(feature = "opentype-math"))]
            let _ = font_glyph;
            plus_signs.insert(c.mark.glyphs, plus);
        }
    }
    let start_delta = |c: BreakCandidate, b: &Builder<'_, '_, C>| {
        if substitute_start {
            plus_signs
                .get(&c.mark.glyphs)
                .map_or(0.0, |plus| plus.width - b.plan.glyphs[c.mark.glyphs].width)
        } else {
            0.0
        }
    };
    // Choose the breaks: every manual one, then automatic ones where a line
    // would run past the width.
    let mut chosen: Vec<(BreakCandidate, f64)> = Vec::new();
    let mut line_start_x = 0.0;
    let mut leading_delta = 0.0;
    let mut indent = 0.0;
    let mut last_fit: Option<(BreakCandidate, f64)> = None;
    let mut candidates_to_scan = candidates.as_slice();
    if let Some(width) = max_width.filter(|width| word_mac_rtl && metrics.width > *width) {
        let first_segment_width = candidates
            .iter()
            .find(|c| c.manual)
            .and_then(|c| x_of(b, c.mark).map(|x| x - c.preceding_space))
            .unwrap_or(metrics.width);
        // Word starts the continuation at the first relation when an RTL
        // equation inherits the text paragraph's display settings.
        if let Some((index, relation, x)) = candidates
            .iter()
            .enumerate()
            .take_while(|(_, c)| !c.manual)
            .filter(|(_, c)| c.class == AtomClass::Rel)
            .filter_map(|(i, c)| x_of(b, c.mark).map(|x| (i, *c, x)))
            .find(|(_, c, x)| first_segment_width > width && x - c.preceding_space <= width)
        {
            chosen.push((relation, x));
            line_start_x = x;
            candidates_to_scan = &candidates[index + 1..];
        }
    }
    let after = settings.break_binary == BreakBinary::After;
    let mut preferred_relation = None;
    let mut saw_binary = false;
    for c in candidates_to_scan {
        let Some(operator_x) = x_of(b, c.mark) else {
            continue;
        };
        // Both `after` and `repeat` leave an operator on the preceding line.
        // With `repeat`, the continuation still starts at the operator.
        let keeps_operator = !c.manual
            && settings.break_binary != BreakBinary::Before
            && breaks_after_operator(c.class);
        let fit_x = if keeps_operator {
            let glyph = &b.plan.glyphs[c.mark.glyphs];
            let width = if substitute_end {
                plus_signs
                    .get(&c.mark.glyphs)
                    .map_or(glyph.width, |plus| plus.width)
            } else {
                glyph.width
            };
            operator_x + width
        } else {
            operator_x - c.preceding_space
        };
        let x = if settings.break_binary == BreakBinary::After && keeps_operator {
            fit_x + c.following_space
        } else {
            operator_x
        };
        if c.manual {
            chosen.push((*c, x));
            line_start_x = x;
            leading_delta = 0.0;
            indent = 0.0;
            last_fit = None;
            preferred_relation = None;
            saw_binary = false;
            continue;
        }
        if let Some(width) = max_width {
            if fit_x - line_start_x + leading_delta + indent > width {
                if let Some((fit, fit_x)) = preferred_relation.take().or(last_fit.take()) {
                    chosen.push((fit, fit_x));
                    line_start_x = fit_x;
                    leading_delta = start_delta(fit, b);
                    indent = if settings.wrap_right {
                        0.0
                    } else {
                        settings.wrap_indent_px
                    };
                }
            }
        }
        // Word prefers a relation within the expression over a later binary
        // operator. An initial assignment such as x= stays with its right side.
        if prefers_relation && c.class == AtomClass::Rel && saw_binary {
            preferred_relation = Some((*c, x));
        }
        saw_binary |= c.class == AtomClass::Bin;
        last_fit = Some((*c, x));
    }
    // The end of the equation may still overflow the last line.
    if let (Some(width), Some((fit, fit_x))) = (max_width, preferred_relation.or(last_fit)) {
        if metrics.width - line_start_x + leading_delta + indent > width
            && chosen
                .last()
                .is_none_or(|(c, _)| c.mark.glyphs < fit.mark.glyphs)
        {
            chosen.push((fit, fit_x));
        }
    }
    if chosen.is_empty() && align_points.is_empty() {
        return Ok(vec![PlacedLine {
            start,
            end,
            metrics,
            group,
            x: 0.0,
            align: None,
            trailing_operator: None,
        }]);
    }
    // brkBin="after" keeps the operator at the end of the line above.
    let mut bounds: Vec<(Mark, f64, Option<BreakCandidate>)> = vec![(start, 0.0, None)];
    for (c, x) in &chosen {
        let mut mark = c.mark;
        if after && !c.manual && breaks_after_operator(c.class) {
            mark.glyphs += 1;
        }
        bounds.push((mark, *x, Some(*c)));
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
        let leading_delta = candidate.map_or(0.0, |c| start_delta(c, b));
        if let Some(plus) = candidate
            .filter(|_| substitute_start)
            .and_then(|c| plus_signs.get(&c.mark.glyphs))
        {
            b.plan.glyphs[line_start.glyphs] = plus.clone();
            let mut rest = line_start;
            rest.glyphs += 1;
            b.shift_range(rest, line_end, leading_delta, 0.0);
        }
        let (_, mut ascent, mut right, mut descent) = extent(b, line_start, line_end);
        let trailing_operator = bounds
            .get(i + 1)
            .and_then(|bound| bound.2)
            .filter(|c| repeats(*c))
            .map(|c| {
                let mut glyph = if substitute_end {
                    plus_signs.get(&c.mark.glyphs)
                } else {
                    None
                }
                .unwrap_or(&b.plan.glyphs[c.mark.glyphs])
                .clone();
                glyph.x += leading_delta;
                ascent = ascent.max(glyph.ascent - glyph.baseline);
                descent = descent.max(glyph.descent + glyph.baseline);
                right = right.max(glyph.x + glyph.width);
                glyph.x -= origin_x;
                glyph
            });
        b.shift_range(line_start, line_end, -origin_x, 0.0);
        let width = (right - origin_x).max(0.0);
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
                ascent,
                descent,
            },
            group,
            x,
            align,
            trailing_operator,
        });
    }
    Ok(lines)
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
pub(crate) fn stack<'a, C: StyledMathMeasure>(
    b: &mut Builder<'_, 'a, C>,
    mut lines: Vec<PlacedLine<'a>>,
) -> MathMetrics {
    if lines.is_empty() {
        return MathMetrics::default();
    }
    let size = b.base_size;
    let word_mac_cambria = b.style.family.eq_ignore_ascii_case("Cambria Math")
        && crate::line_metrics::word_layout_platform()
            == crate::line_metrics::WordLayoutPlatform::Mac;
    // A standalone tall display line reserves Cambria Math’s OS/2
    // typographic line gap. Wrapped displays use MATH leading.
    let top_leading = size * (353.0 / 2048.0);
    let gap = super::MathConstants::cambria(size).leading;
    let tall_ascent = size * (1946.0 / 2048.0);
    let needs_top_leading = word_mac_cambria && (b.options.display || lines.len() > 1) && {
        #[cfg(feature = "opentype-math")]
        {
            b.context.font_parameters(b.style).is_none()
        }
        #[cfg(not(feature = "opentype-math"))]
        {
            true
        }
    };
    if lines.len() == 1 && lines[0].x == 0.0 {
        let line = &lines[0];
        b.plan.lines.push(MathLine {
            baseline: 0.0,
            x: 0.0,
            width: line.metrics.width,
            ascent: line.metrics.ascent,
            descent: line.metrics.descent,
        });
        let mut metrics = line.metrics;
        if needs_top_leading {
            let optical_ascent = b.plan.glyphs[line.start.glyphs..line.end.glyphs]
                .iter()
                .filter(|glyph| glyph.visible && glyph.optical_script && glyph.baseline < 0.0)
                .map(|glyph| glyph.ascent - glyph.baseline)
                .fold(0.0, f64::max);
            // Optical scripts get MATH leading. A taller structure such as
            // a fraction still reserves the standalone typographic gap.
            if metrics.ascent > tall_ascent && metrics.ascent > optical_ascent {
                metrics.ascent += top_leading;
            }
            if optical_ascent > 0.0 {
                metrics.ascent = metrics.ascent.max(optical_ascent + gap);
            }
        }
        return metrics;
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
    let mut group_widths = vec![0.0_f64; groups];
    for line in &lines {
        group_widths[line.group] = group_widths[line.group].max(line.x + line.metrics.width);
    }
    let width = group_widths.iter().copied().fold(0.0, f64::max);
    let justification = b.justification();
    // Repeated wrapped Cambria Math rows in Word for Mac use about 14.10 pt
    // between baselines at 12 pt, including across multiple display equations.
    let min_gap = size * if word_mac_cambria { 1.175 } else { 1.2 };
    let mac_wrapped = word_mac_cambria && lines.len() > 1;
    let mut baseline = 0.0;
    let mut previous: Option<MathMetrics> = None;
    let mut total = MathMetrics::default();
    for line in lines {
        if let Some(prev) = previous {
            let mut ascent = line.metrics.ascent;
            let mut descent = prev.descent;
            // Ink alone underestimates the adjoining text row when a tall
            // formula expands the baseline interval beyond the usual pitch.
            if mac_wrapped && prev.ascent > tall_ascent {
                ascent = ascent.max(size * 0.8);
            }
            if mac_wrapped && ascent > tall_ascent {
                descent = descent.max(size * 0.25);
            }
            baseline += (descent + ascent + gap).max(min_gap);
        } else if needs_top_leading && line.metrics.ascent > tall_ascent {
            total.ascent = line.metrics.ascent + gap;
        }
        let group_w = group_widths[line.group];
        let group_x = match justification {
            MathJustification::Left => 0.0,
            MathJustification::Right => width - group_w,
            _ => (width - group_w) / 2.0,
        };
        let x = match justification {
            MathJustification::Right if b.options.word_mac_rtl() => width - line.metrics.width,
            MathJustification::Center => group_x + (group_w - line.metrics.width) / 2.0,
            _ => group_x + line.x,
        };
        b.shift_range(line.start, line.end, x, baseline);
        if let Some(mut glyph) = line.trailing_operator {
            // A display copy shares the original text range. Appending it
            // leaves the source glyph indices used by equation carets intact.
            glyph.x += x;
            glyph.baseline += baseline;
            b.plan.glyphs.push(glyph);
        }
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
