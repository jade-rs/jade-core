use crate::{PlacedLine, PlacedRun, PlacedShape, RunStyle, TextMeasure};
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextHit {
    pub offset: usize,
    pub upstream: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextCaretMove {
    #[serde(flatten)]
    pub hit: TextHit,
    /// Preferred advance in the text frame, independent of slide rotation/zoom.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_x: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextSelectionGeometry {
    pub caret_offset: usize,
    pub caret: [Point; 2],
    pub highlights: Vec<[Point; 4]>,
}

struct Boundary<'a> {
    offset: usize,
    byte: usize,
    run: Option<&'a PlacedRun>,
    x: f64,
}
impl Boundary<'_> {
    fn x(&self, measure: &dyn TextMeasure) -> f64 {
        let Some(run) = self.run else {
            return self.x;
        };
        if self.byte == 0 {
            return run.x;
        }
        if self.byte == run.text.len() {
            return run.x + run.width;
        }
        let style = RunStyle {
            size_pt: run.size_pt,
            bold: run.bold,
            italic: run.italic,
            font: run.font.clone(),
            east_asian_font: run.east_asian_font.clone(),
        };
        run.x + measure.caret_advance(&run.text, self.byte, &style)
    }
}

fn boundaries(line: &PlacedLine) -> Vec<Boundary<'_>> {
    let mut out = Vec::new();
    for run in &line.runs {
        let bytes: Vec<_> = run
            .text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(run.text.len()))
            .collect();
        if bytes.len() != run.source_offsets.len() {
            continue;
        }
        let mut previous = None;
        for (&byte, &offset) in bytes.iter().zip(&run.source_offsets) {
            if previous == Some(offset) {
                continue;
            }
            out.push(Boundary {
                offset,
                byte,
                run: Some(run),
                x: run.x,
            });
            previous = Some(offset);
        }
    }
    if out.is_empty() {
        if let Some((offset, _)) = line.source_range {
            out.push(Boundary {
                offset,
                byte: 0,
                run: None,
                x: line.text_left,
            });
        }
    }
    out
}

fn caret<'a>(
    lines: &'a [PlacedLine],
    offset: usize,
    upstream: bool,
) -> Option<(usize, Boundary<'a>)> {
    let mut best: Option<(usize, Boundary<'a>)> = None;
    for (index, line) in lines.iter().enumerate() {
        for b in boundaries(line) {
            let distance = b.offset.abs_diff(offset);
            let replace = best.as_ref().is_none_or(|(old_line, old)| {
                let no_source_advance = lines[*old_line]
                    .runs
                    .iter()
                    .flat_map(|run| &run.source_offsets)
                    .all(|source| *source == old.offset);
                distance < old.offset.abs_diff(offset)
                    || (distance == old.offset.abs_diff(offset)
                        && if upstream {
                            index < *old_line
                        } else {
                            index == *old_line || (index > *old_line && !no_source_advance)
                        })
            });
            if replace {
                best = Some((index, b));
            }
        }
    }
    best
}

fn hit_line(line: &PlacedLine, x: f64, measure: &dyn TextMeasure) -> Option<usize> {
    let points = boundaries(line);
    if points.is_empty() {
        return None;
    }
    // Prefix advances are queried lazily: a long unwrapped line needs only
    // logarithmically many backend measurements for a pointer hit.
    let (mut lo, mut hi) = (0, points.len());
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if points[mid].x(measure) < x {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    let right = lo.min(points.len() - 1);
    let left = right.saturating_sub(1);
    let selected = if (points[left].x(measure) - x).abs() <= (points[right].x(measure) - x).abs() {
        left
    } else {
        right
    };
    Some(points[selected].offset)
}

impl PlacedShape {
    fn text_to_slide(&self, x: f64, y: f64) -> Point {
        let r = self.paint_rect();
        let (x, y) = if self.text_rotation_deg != 0.0 {
            let (sin, cos) = self.text_rotation_deg.to_radians().sin_cos();
            let (dx, dy) = (x - r.height / 2.0, y - r.width / 2.0);
            (cos * dx - sin * dy, sin * dx + cos * dy)
        } else {
            (x - r.width / 2.0, y - r.height / 2.0)
        };
        let (x, y) = self.paint_to_slide((r.x + r.width / 2.0 + x, r.y + r.height / 2.0 + y));
        Point { x, y }
    }

    fn slide_to_text(&self, x: f64, y: f64) -> (f64, f64) {
        let (x, y) = self.slide_to_paint((x, y)).unwrap_or((f64::NAN, f64::NAN));
        let r = self.paint_rect();
        let (dx, dy) = (x - r.x - r.width / 2.0, y - r.y - r.height / 2.0);
        let (x, y) = (dx, dy);
        if self.text_rotation_deg != 0.0 {
            let (sin, cos) = self.text_rotation_deg.to_radians().sin_cos();
            (
                cos * x + sin * y + r.height / 2.0,
                -sin * x + cos * y + r.width / 2.0,
            )
        } else {
            (x + r.width / 2.0, y + r.height / 2.0)
        }
    }

    pub fn hit_text(&self, x: f64, y: f64, measure: &dyn TextMeasure) -> Option<TextHit> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let (x, y) = self.slide_to_text(x, y);
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let (index, line) = self
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.source_range.is_some())
            .min_by(|(_, a), (_, b)| {
                let distance = |line: &PlacedLine| {
                    if y < line.top {
                        line.top - y
                    } else if y > line.top + line.height {
                        y - line.top - line.height
                    } else {
                        0.0
                    }
                };
                distance(a).total_cmp(&distance(b))
            })?;
        let offset = hit_line(line, x, measure)?;
        let upstream = self
            .lines
            .get(index + 1)
            .and_then(|line| line.source_range)
            .is_some_and(|(start, _)| start == offset);
        Some(TextHit { offset, upstream })
    }

    pub fn selection_geometry(
        &self,
        start: usize,
        end: usize,
        focus: usize,
        upstream: bool,
        measure: &dyn TextMeasure,
    ) -> Option<TextSelectionGeometry> {
        let (index, point) = caret(&self.lines, focus, upstream)?;
        let line = &self.lines[index];
        let x = point.x(measure);
        let mut highlights = Vec::new();
        for line in &self.lines {
            if start >= end {
                break;
            }
            let Some((from, to)) = line.source_range else {
                continue;
            };
            let selected_tail = line
                .selection_tail
                .is_some_and(|(a, b)| start < b && end > a);
            if (start >= to || end <= from) && !selected_tail {
                continue;
            }
            let all = boundaries(line);
            let left = all
                .iter()
                .min_by_key(|p| p.offset.abs_diff(start.max(from)))?;
            let right = all.iter().min_by_key(|p| p.offset.abs_diff(end.min(to)))?;
            let x1 = left.x(measure);
            let x2 = if end >= to {
                line.runs
                    .iter()
                    .rfind(|run| !run.source_offsets.is_empty())
                    .map(|run| run.x + run.width)
                    .unwrap_or_else(|| right.x(measure))
            } else {
                right.x(measure)
            } + if selected_tail {
                line.height * 0.25
            } else {
                0.0
            };
            highlights.push([
                self.text_to_slide(x1, line.top),
                self.text_to_slide(x2, line.top),
                self.text_to_slide(x2, line.top + line.height),
                self.text_to_slide(x1, line.top + line.height),
            ]);
        }
        Some(TextSelectionGeometry {
            caret_offset: point.offset,
            caret: [
                self.text_to_slide(x, line.top),
                self.text_to_slide(x, line.top + line.height),
            ],
            highlights,
        })
    }

    pub fn move_text_caret(
        &self,
        offset: usize,
        direction: &str,
        upstream: bool,
        measure: &dyn TextMeasure,
    ) -> Option<TextHit> {
        self.move_text_caret_with_x(offset, direction, upstream, None, measure)
            .map(|movement| movement.hit)
    }

    pub fn move_text_caret_with_x(
        &self,
        offset: usize,
        direction: &str,
        upstream: bool,
        preferred_x: Option<f64>,
        measure: &dyn TextMeasure,
    ) -> Option<TextCaretMove> {
        if preferred_x.is_some_and(|x| !x.is_finite()) {
            return None;
        }
        let (index, point) = caret(&self.lines, offset, upstream)?;
        let preferred_x = matches!(direction, "up" | "down")
            .then(|| preferred_x.unwrap_or_else(|| point.x(measure)));
        let target = match direction {
            "up" => index.saturating_sub(1),
            "down" => (index + 1).min(self.lines.len().saturating_sub(1)),
            _ => index,
        };
        let line = self.lines.get(target)?;
        let (from, to) = line.source_range?;
        let offset = match direction {
            "line_start" => from,
            "line_end" => to,
            "up" | "down" => hit_line(line, preferred_x?, measure)?,
            _ => return None,
        };
        let upstream = direction == "line_end"
            || self
                .lines
                .get(target + 1)
                .and_then(|line| line.source_range)
                .is_some_and(|(start, _)| start == offset);
        Some(TextCaretMove {
            hit: TextHit { offset, upstream },
            preferred_x,
        })
    }
}
