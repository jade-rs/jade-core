//! The 描画 tab over any [`WordSession`]: pens, the pencil and the
//! highlighter with their colours, thicknesses and effects; the eraser (a
//! stroke at a time, or only the part it passes over); なげなわ選択 and
//! moving or resizing what it caught; インクを図形に変換; and the ink
//! editor's gestures. Strokes are Word ink (InkML behind a `w14:contentPart`,
//! with a picture fallback), written by the engine's
//! `SetInkDrawingsCommand`.
//!
//! Ink lives here as strokes on pages, in page px: the layer the shells draw
//! over and hit-test, rebuilt from the document after every edit.

use crate::commands::insert_shape::InsertShapeCommand;
use crate::commands::set_ink_drawings::SetInkDrawingsCommand;
use crate::commands::transform_object::TransformObjectCommand;
use crate::commands::{Command, CommandContext, CommandError, CommandResult};
use crate::document::TextBoxShape;
pub use crate::inkml::{InkBrush, InkTool, InkTrace};
use crate::word_document::ink_drawing::InkDrawingSpec;
use crate::word_document::{FloatingObjectTransform, ShapeInsertSpec, WordDocumentSnapshot};
use jade_selection::SelectionRange;

use super::WordSession;

/// Pixels in a millimetre at 96 dpi.
const PX_PER_MM: f64 = 96.0 / 25.4;

/// The pens' thicknesses in mm (ペンの太さ).
pub const PEN_THICKNESSES_MM: [f64; 7] = [0.25, 0.35, 0.5, 0.7, 1.0, 2.0, 3.5];
/// The highlighter's.
pub const HIGHLIGHTER_THICKNESSES_MM: [f64; 5] = [2.0, 4.0, 6.0, 8.0, 10.0];
/// The pens' colours.
pub const PEN_COLORS: [&str; 16] = [
    "#000000", "#FFFFFF", "#7F7F7F", "#E71224", "#F7630C", "#FFC114", "#16C60C", "#00A0A0",
    "#0078D7", "#004E8C", "#886CE4", "#AE198D", "#E3008C", "#8C5A2B", "#5B2A86", "#3A6E1F",
];
/// The highlighter's colours.
pub const HIGHLIGHTER_COLORS: [&str; 8] = [
    "#FFFF00", "#00FF00", "#00FFFF", "#FF00FF", "#0000FF", "#FF0000", "#FFC000", "#C0C0C0",
];
/// Office's ink effects, with their string keys.
pub const INK_EFFECTS: [(&str, &str); 8] = [
    ("rainbow", "draw.effect.rainbow"),
    ("galaxy", "draw.effect.galaxy"),
    ("lava", "draw.effect.lava"),
    ("ocean", "draw.effect.ocean"),
    ("gold", "draw.effect.gold"),
    ("silver", "draw.effect.silver"),
    ("rosegold", "draw.effect.rosegold"),
    ("bronze", "draw.effect.bronze"),
];

/// A pen in the 描画 tab's pen gallery.
#[derive(Clone, Debug, PartialEq)]
pub struct Pen {
    pub tool: InkTool,
    /// `#RRGGBB`.
    pub color: String,
    pub thickness_mm: f64,
    /// One of [`INK_EFFECTS`]' names: the colour is then the effect's.
    pub effect: Option<String>,
}

impl Pen {
    pub fn new(tool: InkTool, color: &str, thickness_mm: f64) -> Self {
        Self {
            tool,
            color: color.to_string(),
            thickness_mm,
            effect: None,
        }
    }

    /// The brush strokes drawn with it get.
    pub fn brush(&self) -> InkBrush {
        let thickness = self.thickness_mm * PX_PER_MM;
        let (width, height) = match self.tool {
            // A tall chisel tip.
            InkTool::Highlighter => (thickness / 6.0, thickness),
            _ => (thickness, thickness),
        };
        InkBrush {
            tool: self.tool,
            color: self.color.clone(),
            width_px: width,
            height_px: height,
            effect: self.effect.clone().filter(|_| self.tool == InkTool::Pen),
            transparency: 0,
        }
    }

    /// The thicknesses it offers.
    pub fn thicknesses(&self) -> &'static [f64] {
        if self.tool == InkTool::Highlighter {
            &HIGHLIGHTER_THICKNESSES_MM
        } else {
            &PEN_THICKNESSES_MM
        }
    }

    /// The colours it offers.
    pub fn colors(&self) -> &'static [&'static str] {
        if self.tool == InkTool::Highlighter {
            &HIGHLIGHTER_COLORS
        } else {
            &PEN_COLORS
        }
    }

    /// The string key naming its kind: ペン, 鉛筆, 蛍光ペン.
    pub fn kind_key(&self) -> &'static str {
        match self.tool {
            InkTool::Pen => "draw.pen",
            InkTool::Pencil => "draw.pencil",
            InkTool::Highlighter => "draw.highlighter",
        }
    }
}

/// Word's starting pens: black and red pens, a pencil, a yellow highlighter.
pub fn default_pens() -> Vec<Pen> {
    vec![
        Pen::new(InkTool::Pen, "#000000", 0.5),
        Pen::new(InkTool::Pen, "#E71224", 0.5),
        Pen::new(InkTool::Pencil, "#7F7F7F", 1.0),
        Pen::new(InkTool::Highlighter, "#FFFF00", 6.0),
    ]
}

/// What the eraser takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EraserKind {
    /// ストローク消しゴム: whole strokes it touches.
    Stroke,
    /// ポイント消しゴム: only the parts it passes over.
    Point,
}

/// The tool in hand on the 描画 tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawTool {
    /// 選択: the mouse selects text and objects again.
    Select,
    /// なげなわ選択.
    Lasso,
    /// One of the pens, by index.
    Pen(usize),
    Eraser(EraserKind),
}

/// The 描画 tab's state, kept by the shell.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawState {
    pub tool: DrawTool,
    pub pens: Vec<Pen>,
    /// タッチで描画 (描画モード): whether the mouse or a finger draws with the
    /// tool in hand. Off, the pens wait and the mouse selects text.
    pub draw_with_touch: bool,
    /// インクを図形に変換: strokes that look like shapes become shapes.
    pub ink_to_shape: bool,
    /// インク エディター: strokes over text are gestures on it.
    pub ink_editor: bool,
    /// The strokes なげなわ選択 caught, by index into the session's strokes.
    pub selection: Vec<usize>,
}

impl Default for DrawState {
    fn default() -> Self {
        Self {
            tool: DrawTool::Select,
            pens: default_pens(),
            draw_with_touch: true,
            ink_to_shape: false,
            ink_editor: false,
            selection: Vec::new(),
        }
    }
}

impl DrawState {
    /// Whether a drag on the page draws, erases or lassoes rather than
    /// selecting text.
    pub fn captures_pointer(&self) -> bool {
        self.draw_with_touch && self.tool != DrawTool::Select
    }

    /// The pen in hand, if one is.
    pub fn pen(&self) -> Option<&Pen> {
        match self.tool {
            DrawTool::Pen(index) => self.pens.get(index),
            _ => None,
        }
    }

    /// Picks up a pen; again on the pen in hand puts it down.
    pub fn choose_pen(&mut self, index: usize) {
        self.tool = if self.tool == DrawTool::Pen(index) {
            DrawTool::Select
        } else {
            DrawTool::Pen(index)
        };
        self.selection.clear();
    }

    pub fn set_pen_color(&mut self, index: usize, color: &str) {
        if let Some(pen) = self.pens.get_mut(index) {
            pen.color = color.to_string();
            pen.effect = None;
        }
    }

    pub fn set_pen_thickness(&mut self, index: usize, thickness_mm: f64) {
        if let Some(pen) = self.pens.get_mut(index) {
            pen.thickness_mm = thickness_mm;
        }
    }

    pub fn set_pen_effect(&mut self, index: usize, effect: Option<&str>) {
        if let Some(pen) = self
            .pens
            .get_mut(index)
            .filter(|pen| pen.tool == InkTool::Pen)
        {
            pen.effect = effect.map(str::to_string);
        }
    }

    /// ペンの追加: another pen like the one given.
    pub fn add_pen(&mut self, tool: InkTool) -> usize {
        let pen = match tool {
            InkTool::Highlighter => Pen::new(tool, "#00FF00", 6.0),
            InkTool::Pencil => Pen::new(tool, "#000000", 1.0),
            InkTool::Pen => Pen::new(tool, "#0078D7", 0.5),
        };
        self.pens.push(pen);
        self.pens.len() - 1
    }
}

/// A stroke on a page, in page px.
#[derive(Clone, Debug, PartialEq)]
pub struct PageStroke {
    /// 0-based.
    pub page: usize,
    pub trace: InkTrace,
    /// The ink drawing (by its paragraph and run) it came from, or `None`
    /// for one not yet in the document.
    pub source: Option<(usize, usize)>,
}

impl PageStroke {
    /// Its extent with the brush: `(left, top, right, bottom)`.
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        crate::inkml::InkContent::bounds(std::slice::from_ref(&self.trace))
            .unwrap_or((0.0, 0.0, 0.0, 0.0))
    }

    /// Whether it passes within `radius` of `point`.
    pub fn touches(&self, point: (f64, f64), radius: f64) -> bool {
        let reach = radius + self.trace.brush.width_px.max(self.trace.brush.height_px) / 2.0;
        let points = &self.trace.points;
        if points.len() == 1 {
            return distance(points[0], point) <= reach;
        }
        points
            .windows(2)
            .any(|pair| segment_distance(point, pair[0], pair[1]) <= reach)
    }
}

/// Strokes rewritten together: their page and the drawing they came from.
type InkGroupKey = (usize, Option<(usize, usize)>);

/// A box: `(left, top, right, bottom)`.
pub type Box4 = (f64, f64, f64, f64);

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn segment_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    let t = if length > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    distance(p, (a.0 + t * dx, a.1 + t * dy))
}

/// Whether `point` is inside the closed polygon `polygon` (even-odd).
pub fn point_in_polygon(point: (f64, f64), polygon: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let mut j = polygon.len().wrapping_sub(1);
    for i in 0..polygon.len() {
        let (a, b) = (polygon[i], polygon[j]);
        if (a.1 > point.1) != (b.1 > point.1)
            && point.0 < (b.0 - a.0) * (point.1 - a.1) / (b.1 - a.1) + a.0
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Erases what a point eraser passing along `path` covers from `stroke`:
/// the pieces of it left, each a stroke of its own (none if it is all gone).
pub fn erase_points(stroke: &PageStroke, path: &[(f64, f64)], radius: f64) -> Vec<PageStroke> {
    let reach = radius + stroke.trace.brush.width_px / 2.0;
    let erased = |point: (f64, f64)| {
        if path.len() == 1 {
            return distance(path[0], point) <= reach;
        }
        path.windows(2)
            .any(|pair| segment_distance(point, pair[0], pair[1]) <= reach)
    };
    // Resample long segments so a short eraser pass can cut one.
    let mut points = Vec::new();
    for pair in stroke.trace.points.windows(2) {
        let steps = (distance(pair[0], pair[1]) / (reach.max(1.0) / 2.0)).ceil() as usize;
        for step in 0..steps.max(1) {
            let t = step as f64 / steps.max(1) as f64;
            points.push((
                pair[0].0 + (pair[1].0 - pair[0].0) * t,
                pair[0].1 + (pair[1].1 - pair[0].1) * t,
            ));
        }
    }
    if let Some(last) = stroke.trace.points.last() {
        points.push(*last);
    }
    let mut pieces: Vec<Vec<(f64, f64)>> = vec![Vec::new()];
    for point in points {
        if erased(point) {
            if !pieces.last().is_some_and(Vec::is_empty) {
                pieces.push(Vec::new());
            }
        } else if let Some(piece) = pieces.last_mut() {
            piece.push(point);
        }
    }
    pieces
        .into_iter()
        .filter(|piece| !piece.is_empty())
        .map(|points| PageStroke {
            page: stroke.page,
            trace: InkTrace {
                points,
                brush: stroke.trace.brush.clone(),
            },
            source: None,
        })
        .collect()
}

/// Moves and scales `stroke` from box `from` onto box `to`.
pub fn transform_stroke(stroke: &PageStroke, from: Box4, to: Box4) -> PageStroke {
    let sx = (to.2 - to.0) / (from.2 - from.0).max(1e-6);
    let sy = (to.3 - to.1) / (from.3 - from.1).max(1e-6);
    let mut moved = stroke.clone();
    for point in &mut moved.trace.points {
        *point = (
            to.0 + (point.0 - from.0) * sx,
            to.1 + (point.1 - from.1) * sy,
        );
    }
    moved.source = None;
    moved
}

/// The box round several strokes.
pub fn strokes_bounds<'a>(strokes: impl IntoIterator<Item = &'a PageStroke>) -> Option<Box4> {
    strokes
        .into_iter()
        .map(PageStroke::bounds)
        .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)))
}

// ---- recognising shapes and gestures ---------------------------------------

/// A shape インクを図形に変換 makes of a stroke.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InkShape {
    /// A line from the first point to the second; with an arrowhead at the
    /// second when `arrow`.
    Line {
        from: (f64, f64),
        to: (f64, f64),
        arrow: bool,
    },
    Rectangle(Box4),
    Ellipse(Box4),
    Triangle(Box4),
    Diamond(Box4),
}

fn path_length(points: &[(f64, f64)]) -> f64 {
    points
        .windows(2)
        .map(|pair| distance(pair[0], pair[1]))
        .sum()
}

fn bounds_of(points: &[(f64, f64)]) -> Box4 {
    points.iter().fold(
        (f64::MAX, f64::MAX, f64::MIN, f64::MIN),
        |(l, t, r, b), &(x, y)| (l.min(x), t.min(y), r.max(x), b.max(y)),
    )
}

/// Ramer–Douglas–Peucker: the corners of a polyline.
fn simplify(points: &[(f64, f64)], epsilon: f64) -> Vec<(f64, f64)> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let (first, last) = (points[0], points[points.len() - 1]);
    let (index, farthest) = points[1..points.len() - 1]
        .iter()
        .enumerate()
        .map(|(index, &point)| (index + 1, segment_distance(point, first, last)))
        .fold(
            (0, 0.0),
            |best, item| if item.1 > best.1 { item } else { best },
        );
    if farthest > epsilon {
        let mut left = simplify(&points[..=index], epsilon);
        let right = simplify(&points[index..], epsilon);
        left.pop();
        left.extend(right);
        left
    } else {
        vec![first, last]
    }
}

/// Recognises a shape in a stroke, as インクを図形に変換 does: a line, an
/// arrow, a triangle, a rectangle, a diamond or an ellipse. `None` for a
/// stroke that is none of them (handwriting stays ink).
pub fn recognize_shape(points: &[(f64, f64)]) -> Option<InkShape> {
    if points.len() < 2 {
        return None;
    }
    let length = path_length(points);
    let (l, t, r, b) = bounds_of(points);
    let (w, h) = (r - l, b - t);
    let diagonal = (w * w + h * h).sqrt();
    if length < 12.0 || diagonal < 8.0 {
        return None;
    }
    let first = points[0];
    let last = points[points.len() - 1];
    let closed = distance(first, last) < 0.2 * diagonal.max(1.0);

    if !closed {
        // A line: nearly as long as the way between its ends.
        if distance(first, last) / length > 0.92 {
            return Some(InkShape::Line {
                from: first,
                to: last,
                arrow: false,
            });
        }
        // An arrow: a straight shaft to its farthest point, then a short
        // head drawn back from the tip.
        let (tip, _) = points
            .iter()
            .enumerate()
            .map(|(index, &point)| (index, distance(first, point)))
            .fold(
                (0, 0.0),
                |best, item| if item.1 > best.1 { item } else { best },
            );
        let shaft = &points[..=tip];
        let head = &points[tip..];
        let shaft_length = path_length(shaft);
        let head_length = path_length(head);
        if tip > 0
            && distance(first, points[tip]) / shaft_length.max(1e-6) > 0.9
            && head_length > 0.1 * shaft_length
            && head_length < 0.8 * shaft_length
        {
            return Some(InkShape::Line {
                from: first,
                to: points[tip],
                arrow: true,
            });
        }
        return None;
    }

    let mut closed_points = points.to_vec();
    closed_points.push(first);
    let corners = simplify(&closed_points, 0.08 * diagonal);
    // The polygon's corners, without the repeated start.
    let mut polygon = corners.clone();
    if polygon.len() > 1 && distance(polygon[0], polygon[polygon.len() - 1]) < 0.2 * diagonal {
        polygon.pop();
    }
    let rect = (l, t, r, b);
    match polygon.len() {
        3 => return Some(InkShape::Triangle(rect)),
        4 => {
            // Corners near the box's corners: a rectangle; near its sides'
            // middles: a diamond.
            let near =
                |point: (f64, f64), target: (f64, f64)| distance(point, target) < 0.25 * diagonal;
            let box_corners = [(l, t), (r, t), (r, b), (l, b)];
            let mids = [
                ((l + r) / 2.0, t),
                (r, (t + b) / 2.0),
                ((l + r) / 2.0, b),
                (l, (t + b) / 2.0),
            ];
            if polygon
                .iter()
                .all(|&corner| box_corners.iter().any(|&target| near(corner, target)))
            {
                return Some(InkShape::Rectangle(rect));
            }
            if polygon
                .iter()
                .all(|&corner| mids.iter().any(|&target| near(corner, target)))
            {
                return Some(InkShape::Diamond(rect));
            }
        }
        _ => {}
    }
    // An ellipse: every point about as far round the box's ellipse.
    let (cx, cy, rx, ry) = (
        (l + r) / 2.0,
        (t + b) / 2.0,
        (w / 2.0).max(1.0),
        (h / 2.0).max(1.0),
    );
    let radii: Vec<f64> = points
        .iter()
        .map(|&(x, y)| (((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2)).sqrt())
        .collect();
    let mean = radii.iter().sum::<f64>() / radii.len() as f64;
    let spread = (radii
        .iter()
        .map(|radius| (radius - mean).powi(2))
        .sum::<f64>()
        / radii.len() as f64)
        .sqrt();
    if polygon.len() >= 5 && spread < 0.12 {
        return Some(InkShape::Ellipse(rect));
    }
    if polygon.len() == 4 {
        return Some(InkShape::Rectangle(rect));
    }
    None
}

/// What a stroke over text means to the インク エディター.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InkGesture {
    /// A line or scribble through words: delete them. The strike's ends.
    Delete { from: (f64, f64), to: (f64, f64) },
    /// A loop round words: select them. The loop's box.
    Select(Box4),
    /// A caret (^) between words: put the insertion point there.
    Insert((f64, f64)),
}

/// Recognises an ink editor gesture.
pub fn recognize_gesture(points: &[(f64, f64)]) -> Option<InkGesture> {
    if points.len() < 2 {
        return None;
    }
    let (l, t, r, b) = bounds_of(points);
    let (w, h) = (r - l, b - t);
    let length = path_length(points);
    let first = points[0];
    let last = points[points.len() - 1];
    let diagonal = (w * w + h * h).sqrt();
    if diagonal < 6.0 {
        return None;
    }
    // A loop closes on itself.
    if distance(first, last) < 0.25 * diagonal && length > 2.5 * diagonal.max(1.0) * 0.8 {
        return Some(InkGesture::Select((l, t, r, b)));
    }
    // A caret: up to a peak and down again, narrow and pointed.
    let (peak_index, peak) = points
        .iter()
        .enumerate()
        .fold((0, first), |best, (index, &point)| {
            if point.1 < best.1 .1 {
                (index, point)
            } else {
                best
            }
        });
    if peak_index > 0
        && peak_index < points.len() - 1
        && h > 0.5 * w
        && (first.1 - peak.1) > 0.6 * h
        && (last.1 - peak.1) > 0.6 * h
        && w < 60.0
        && (first.0 - peak.0) * (last.0 - peak.0) < 0.0
    {
        return Some(InkGesture::Insert((peak.0, b)));
    }
    // A strike: wide and flat, straight or back and forth.
    if w > 2.0 * h.max(1.0) && w > 12.0 {
        let (left, right) = if first.0 <= last.0 {
            (first, last)
        } else {
            (last, first)
        };
        let middle = (t + b) / 2.0;
        return Some(InkGesture::Delete {
            from: (l.min(left.0), middle),
            to: (r.max(right.0), middle),
        });
    }
    None
}

// ---- the session -----------------------------------------------------------

/// What the 描画 tab does to a document.
pub trait WordInkTools: WordSession {
    /// The document's ink, as strokes on pages.
    fn ink_strokes(&self) -> Vec<PageStroke> {
        let mut strokes = Vec::new();
        for drawing in self.word_document().ink_drawings() {
            // The strokes fill the box they were laid in, which a resize since
            // has stretched.
            let (natural_w, natural_h) = crate::inkml::InkContent::bounds(&drawing.traces)
                .map(|(_, _, right, bottom)| (right, bottom))
                .unwrap_or((drawing.width_px, drawing.height_px));
            let region = self.object_region(drawing.paragraph_index, drawing.run_index);
            let (page, x, y, width, height) = match region {
                Some(region) => region,
                None if drawing.page_relative => (
                    self.paragraph_page(drawing.paragraph_index)
                        .map_or(0, |page| page.saturating_sub(1)),
                    drawing.x_px,
                    drawing.y_px,
                    drawing.width_px,
                    drawing.height_px,
                ),
                None => continue,
            };
            let (sx, sy) = (width / natural_w.max(1e-6), height / natural_h.max(1e-6));
            for trace in drawing.traces {
                strokes.push(PageStroke {
                    page,
                    trace: InkTrace {
                        points: trace
                            .points
                            .iter()
                            .map(|&(px, py)| (x + px * sx, y + py * sy))
                            .collect(),
                        brush: trace.brush,
                    },
                    source: Some((drawing.paragraph_index, drawing.run_index)),
                });
            }
        }
        strokes
    }

    /// The body paragraph new ink on `page` (0-based) is anchored in: the
    /// first that starts there, else the last that starts before it.
    fn ink_anchor_paragraph(&self, page: usize) -> usize {
        let document = self.word_document();
        let mut before = None;
        for paragraph in 0..document.paragraph_count() {
            if !document.is_body_paragraph_surface(paragraph) {
                continue;
            }
            match self
                .paragraph_page(paragraph)
                .map(|page| page.saturating_sub(1))
            {
                Some(start) if start == page => return paragraph,
                Some(start) if start < page => before = Some(paragraph),
                Some(_) => break,
                None => {
                    before.get_or_insert(paragraph);
                }
            }
        }
        before.unwrap_or(0)
    }

    /// Writes `strokes` as the document's ink, as one undo step: drawings
    /// whose strokes are all still there, unchanged, stay; the rest are
    /// written again, a drawing per page and source.
    fn commit_ink(&mut self, strokes: &[PageStroke]) -> Result<(), String> {
        let before = self.ink_strokes();
        let mut specs = Vec::new();
        // Drawings kept whole.
        let mut sources: Vec<(usize, usize)> =
            before.iter().filter_map(|stroke| stroke.source).collect();
        sources.dedup();
        let mut kept = Vec::new();
        for source in sources {
            let old: Vec<&PageStroke> = before
                .iter()
                .filter(|stroke| stroke.source == Some(source))
                .collect();
            let new: Vec<&PageStroke> = strokes
                .iter()
                .filter(|stroke| stroke.source == Some(source))
                .collect();
            if old.len() == new.len() && old.iter().zip(&new).all(|(a, b)| a.trace == b.trace) {
                specs.push(InkDrawingSpec::Keep {
                    paragraph_index: source.0,
                    run_index: source.1,
                });
                kept.push(source);
            }
        }
        // Everything else, grouped by page and source.
        let mut groups: Vec<(InkGroupKey, Vec<InkTrace>)> = Vec::new();
        for stroke in strokes {
            if stroke.source.is_some_and(|source| kept.contains(&source)) {
                continue;
            }
            let key = (stroke.page, stroke.source);
            match groups.iter_mut().find(|(group, _)| *group == key) {
                Some((_, traces)) => traces.push(stroke.trace.clone()),
                None => groups.push((key, vec![stroke.trace.clone()])),
            }
        }
        for ((page, source), traces) in groups {
            let paragraph_index = source
                .map(|(paragraph, _)| paragraph)
                .unwrap_or_else(|| self.ink_anchor_paragraph(page));
            specs.push(InkDrawingSpec::New {
                paragraph_index,
                traces,
            });
        }
        self.execute_command(Box::new(SetInkDrawingsCommand::new(specs)), true)
    }

    /// Draws a stroke with `pen`: as ink, or as a shape when
    /// インクを図形に変換 recognises one. Returns the shape made, if any.
    fn draw_stroke(
        &mut self,
        page: usize,
        points: Vec<(f64, f64)>,
        pen: &Pen,
        ink_to_shape: bool,
    ) -> Result<Option<InkShape>, String> {
        if points.is_empty() {
            return Ok(None);
        }
        if ink_to_shape && pen.tool != InkTool::Highlighter {
            if let Some(shape) = recognize_shape(&points) {
                self.insert_ink_shape(page, shape, pen)?;
                return Ok(Some(shape));
            }
        }
        let mut strokes = self.ink_strokes();
        strokes.push(PageStroke {
            page,
            trace: InkTrace {
                points,
                brush: pen.brush(),
            },
            source: None,
        });
        self.commit_ink(&strokes)?;
        Ok(None)
    }

    /// ストローク消しゴム at `point`: the strokes it touches go. Whether any did.
    fn erase_strokes_at(
        &mut self,
        page: usize,
        point: (f64, f64),
        radius: f64,
    ) -> Result<bool, String> {
        let strokes = self.ink_strokes();
        let left: Vec<PageStroke> = strokes
            .iter()
            .filter(|stroke| !(stroke.page == page && stroke.touches(point, radius)))
            .cloned()
            .collect();
        if left.len() == strokes.len() {
            return Ok(false);
        }
        self.commit_ink(&left)?;
        Ok(true)
    }

    /// ポイント消しゴム along `path`: only what it passes over goes, splitting
    /// strokes it crosses. Whether anything did.
    fn erase_points_along(
        &mut self,
        page: usize,
        path: &[(f64, f64)],
        radius: f64,
    ) -> Result<bool, String> {
        let strokes = self.ink_strokes();
        let mut changed = false;
        let mut left = Vec::new();
        for stroke in &strokes {
            if stroke.page != page || !path.iter().any(|&point| stroke.touches(point, radius)) {
                left.push(stroke.clone());
                continue;
            }
            changed = true;
            left.extend(erase_points(stroke, path, radius));
        }
        if !changed {
            return Ok(false);
        }
        self.commit_ink(&left)?;
        Ok(true)
    }

    /// なげなわ選択: the strokes on `page` mostly inside `polygon`.
    fn lasso_strokes(&self, page: usize, polygon: &[(f64, f64)]) -> Vec<usize> {
        if polygon.len() < 3 {
            return Vec::new();
        }
        self.ink_strokes()
            .iter()
            .enumerate()
            .filter(|(_, stroke)| stroke.page == page)
            .filter(|(_, stroke)| {
                let inside = stroke
                    .trace
                    .points
                    .iter()
                    .filter(|&&point| point_in_polygon(point, polygon))
                    .count();
                inside * 2 > stroke.trace.points.len()
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// Moves and resizes the selected strokes from their box onto `to`.
    fn transform_strokes(&mut self, selection: &[usize], to: Box4) -> Result<(), String> {
        let mut strokes = self.ink_strokes();
        let Some(from) = strokes_bounds(selection.iter().filter_map(|&index| strokes.get(index)))
        else {
            return Ok(());
        };
        for &index in selection {
            if let Some(stroke) = strokes.get_mut(index) {
                *stroke = transform_stroke(stroke, from, to);
            }
        }
        self.commit_ink(&strokes)
    }

    /// Deletes the selected strokes.
    fn delete_strokes(&mut self, selection: &[usize]) -> Result<(), String> {
        let strokes: Vec<PageStroke> = self
            .ink_strokes()
            .into_iter()
            .enumerate()
            .filter(|(index, _)| !selection.contains(index))
            .map(|(_, stroke)| stroke)
            .collect();
        self.commit_ink(&strokes)
    }

    /// Gives the selected strokes `pen`'s colour, thickness and effect.
    fn restyle_strokes(&mut self, selection: &[usize], pen: &Pen) -> Result<(), String> {
        let mut strokes = self.ink_strokes();
        for &index in selection {
            if let Some(stroke) = strokes.get_mut(index) {
                stroke.trace.brush = pen.brush();
                stroke.source = None;
            }
        }
        self.commit_ink(&strokes)
    }

    /// インクを図形に変換 for strokes already drawn: each the recogniser
    /// knows becomes a shape. How many did.
    fn convert_strokes_to_shapes(&mut self, selection: &[usize]) -> Result<usize, String> {
        let strokes = self.ink_strokes();
        let mut converted = 0;
        let mut left = Vec::new();
        let mut shapes = Vec::new();
        for (index, stroke) in strokes.iter().enumerate() {
            let shape = selection
                .contains(&index)
                .then(|| recognize_shape(&stroke.trace.points))
                .flatten();
            match shape {
                Some(shape) => {
                    converted += 1;
                    shapes.push((stroke.page, shape, stroke.trace.brush.clone()));
                }
                None => left.push(stroke.clone()),
            }
        }
        if converted == 0 {
            return Ok(0);
        }
        self.commit_ink(&left)?;
        for (page, shape, brush) in shapes {
            let pen = Pen {
                tool: brush.tool,
                color: brush.color.clone(),
                thickness_mm: brush.width_px / PX_PER_MM,
                effect: None,
            };
            self.insert_ink_shape(page, shape, &pen)?;
        }
        Ok(converted)
    }

    /// Puts a shape where a stroke drew it, outlined in the pen's colour.
    fn insert_ink_shape(&mut self, page: usize, shape: InkShape, pen: &Pen) -> Result<(), String> {
        let paragraph = self.ink_anchor_paragraph(page);
        let offset = self
            .word_document()
            .paragraph(paragraph)
            .map(|paragraph| paragraph.runs.iter().map(|run| run.text.len()).sum())
            .unwrap_or(0);
        let (kind, (l, t, r, b), flip) = match shape {
            InkShape::Line { from, to, arrow } => (
                if arrow {
                    TextBoxShape::LineArrow
                } else {
                    TextBoxShape::Line
                },
                (
                    from.0.min(to.0),
                    from.1.min(to.1),
                    from.0.max(to.0),
                    from.1.max(to.1),
                ),
                // A line runs top-left to bottom-right unless flipped.
                ((to.0 < from.0), (to.1 < from.1)),
            ),
            InkShape::Rectangle(rect) => (TextBoxShape::Rectangle, rect, (false, false)),
            InkShape::Ellipse(rect) => (TextBoxShape::Ellipse, rect, (false, false)),
            InkShape::Triangle(rect) => (TextBoxShape::Triangle, rect, (false, false)),
            InkShape::Diamond(rect) => (TextBoxShape::Diamond, rect, (false, false)),
        };
        let spec = ShapeInsertSpec {
            shape: kind,
            width_px: (r - l).max(1.0),
            height_px: (b - t).max(1.0),
            fill_color: None,
            border_color: Some(pen.color.trim_start_matches('#').to_string()),
            border_width_px: (pen.thickness_mm * PX_PER_MM).max(0.75),
            text: String::new(),
            text_color: None,
            text_font_size_pt: None,
            text_bold: None,
            text_italic: None,
            text_alignment: None,
        };
        let transform = FloatingObjectTransform {
            x: l,
            y: t,
            width: (r - l).max(1.0),
            height: (b - t).max(1.0),
            rotation_deg: 0.0,
        };
        self.execute_command(
            Box::new(InkShapeCommand {
                paragraph,
                offset,
                spec: Some(spec),
                transform,
                flip,
                snapshot: None,
                previous_selection: None,
            }),
            true,
        )
    }
}

impl<T: WordSession + ?Sized> WordInkTools for T {}

/// インクを図形に変換's edit: a shape inserted and placed where the stroke
/// was, as one undo step.
struct InkShapeCommand {
    paragraph: usize,
    offset: usize,
    spec: Option<ShapeInsertSpec>,
    transform: FloatingObjectTransform,
    flip: (bool, bool),
    snapshot: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
}

impl Command for InkShapeCommand {
    fn name(&self) -> &'static str {
        "ink_to_shape"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or_else(|| CommandError::Custom("document is unavailable".to_string()))?;
        let previous_selection = *ctx.selection;
        let spec = self
            .spec
            .clone()
            .ok_or_else(|| CommandError::Custom("no shape to insert".to_string()))?;
        let result = (|| -> CommandResult<()> {
            let mut insert = InsertShapeCommand::new(self.paragraph, self.offset, spec);
            insert.apply(ctx)?;
            let target = insert
                .inserted_target()
                .ok_or_else(|| CommandError::Custom("the shape went missing".to_string()))?;
            TransformObjectCommand::new(target, self.transform).apply(ctx)?;
            if self.flip != (false, false) {
                if let Some(text_box) = ctx
                    .document
                    .paragraph_mut(target.paragraph_index)
                    .and_then(|paragraph| paragraph.runs.get_mut(target.run_index))
                    .and_then(|run| run.text_box.as_mut())
                {
                    text_box.layout.flip_horizontal = self.flip.0;
                    text_box.layout.flip_vertical = self.flip.1;
                }
                ctx.document.sync_paragraph_text(target.paragraph_index);
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.snapshot = Some(snapshot);
                self.previous_selection = previous_selection;
                Ok(())
            }
            Err(error) => {
                ctx.document.restore_snapshot(snapshot);
                *ctx.selection = previous_selection;
                Err(error)
            }
        }
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self
            .snapshot
            .take()
            .ok_or_else(|| CommandError::Custom("ink_to_shape snapshot is missing".to_string()))?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

// ---- the pointer ------------------------------------------------------------

/// The eraser's reach, in page px.
pub const ERASER_RADIUS: f64 = 6.0;
/// How near the selection's corner a press resizes it.
pub const SELECTION_HANDLE: f64 = 10.0;

/// Moving or resizing the strokes なげなわ選択 caught.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectionDrag {
    pub from_box: Box4,
    pub to_box: Box4,
    pub grab: (f64, f64),
    pub resizing: bool,
}

/// What a release did.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawRelease {
    /// Nothing changed.
    Nothing,
    /// The document's ink (or a shape) changed.
    Edited,
    /// The lasso caught strokes (or none).
    Selected,
    /// With the インク エディター on, the stroke is a gesture: the shell
    /// tries it on the text, and draws `points` as ink with the pen if it
    /// does not apply ([`DrawController::ink_instead`]).
    Gesture {
        page: usize,
        gesture: InkGesture,
        points: Vec<(f64, f64)>,
    },
}

/// A line the overlay draws: points, colour, width in px, opacity, and
/// whether it closes (a lasso).
#[derive(Clone, Debug, PartialEq)]
pub struct OverlayStroke {
    pub points: Vec<(f64, f64)>,
    pub color: String,
    pub width: f64,
    pub opacity: f64,
    pub closed: bool,
}

/// What the overlay over a page shows while drawing.
#[derive(Clone, Debug, PartialEq)]
pub struct InkOverlay {
    /// The stroke, eraser pass or lasso being drawn.
    pub gesture: Option<OverlayStroke>,
    /// The selection's box.
    pub selection: Option<Box4>,
    /// The selected strokes where a drag would put them.
    pub preview: Vec<PageStroke>,
}

/// The 描画 tab and the pointer over the pages: the tools, the stroke under
/// way and the selection, for either shell.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawController {
    pub state: DrawState,
    /// The stroke, eraser pass or lasso being drawn: its page and points.
    pub gesture: Option<(usize, Vec<(f64, f64)>)>,
    /// The page the selection is on and the selected strokes as they were.
    pub selected: Option<(usize, Vec<PageStroke>)>,
    pub drag: Option<SelectionDrag>,
}

impl DrawController {
    /// Picks up a tool.
    pub fn set_tool(&mut self, tool: DrawTool) {
        self.state.tool = tool;
        if tool != DrawTool::Lasso {
            self.selected = None;
        }
    }

    /// Picks up a pen, or puts it down.
    pub fn choose_pen(&mut self, index: usize) {
        self.state.choose_pen(index);
        self.selected = None;
    }

    /// The selection's box, as it is being dragged.
    pub fn selection_box(&self) -> Option<(usize, Box4)> {
        let (page, strokes) = self.selected.as_ref()?;
        if let Some(drag) = self.drag.as_ref() {
            return Some((*page, drag.to_box));
        }
        strokes_bounds(strokes.iter()).map(|bounds| (*page, bounds))
    }

    /// A press at page px `at`: whether drawing takes it (a stroke, an
    /// eraser pass, a lasso or a grab on the selection starts).
    pub fn press(&mut self, page: usize, at: (f64, f64)) -> bool {
        if !self.state.captures_pointer() {
            return false;
        }
        if self.state.tool == DrawTool::Lasso {
            if let Some((selected_page, (l, t, r, b))) = self.selection_box() {
                let reach = SELECTION_HANDLE;
                if selected_page == page
                    && at.0 >= l - reach
                    && at.0 <= r + reach
                    && at.1 >= t - reach
                    && at.1 <= b + reach
                {
                    let resizing = (at.0 - r).abs() <= reach && (at.1 - b).abs() <= reach;
                    self.drag = Some(SelectionDrag {
                        from_box: (l, t, r, b),
                        to_box: (l, t, r, b),
                        grab: at,
                        resizing,
                    });
                    return true;
                }
            }
            self.selected = None;
        }
        self.gesture = Some((page, vec![at]));
        true
    }

    /// Whether a stroke or a selection drag is under way.
    pub fn is_drawing(&self) -> bool {
        self.gesture.is_some() || self.drag.is_some()
    }

    /// The pointer moved, with the button down, to page px `at`.
    pub fn drag_to(&mut self, page: usize, at: (f64, f64)) {
        if let Some(drag) = self.drag.as_mut() {
            let (dx, dy) = (at.0 - drag.grab.0, at.1 - drag.grab.1);
            let (l, t, r, b) = drag.from_box;
            drag.to_box = if drag.resizing {
                (l, t, (r + dx).max(l + 4.0), (b + dy).max(t + 4.0))
            } else {
                (l + dx, t + dy, r + dx, b + dy)
            };
            return;
        }
        if let Some((gesture_page, points)) = self.gesture.as_mut() {
            if *gesture_page == page
                && points
                    .last()
                    .is_none_or(|last| (last.0 - at.0).abs() + (last.1 - at.1).abs() > 0.5)
            {
                points.push(at);
            }
        }
    }

    /// The button came up: the stroke goes into the document, the eraser
    /// takes what it passed over, the lasso catches strokes, the selection
    /// lands where it was dragged.
    pub fn release<S: WordInkTools + ?Sized>(
        &mut self,
        session: &mut S,
    ) -> Result<DrawRelease, String> {
        if let Some(drag) = self.drag.take() {
            if drag.to_box == drag.from_box {
                return Ok(DrawRelease::Nothing);
            }
            self.move_selection(session, drag.to_box)?;
            return Ok(DrawRelease::Edited);
        }
        let Some((page, points)) = self.gesture.take() else {
            return Ok(DrawRelease::Nothing);
        };
        match self.state.tool {
            DrawTool::Pen(index) => {
                let pen = self.state.pens[index].clone();
                if self.state.ink_editor {
                    if let Some(gesture) = recognize_gesture(&points) {
                        return Ok(DrawRelease::Gesture {
                            page,
                            gesture,
                            points,
                        });
                    }
                }
                session.draw_stroke(page, points, &pen, self.state.ink_to_shape)?;
                Ok(DrawRelease::Edited)
            }
            DrawTool::Eraser(EraserKind::Stroke) => {
                let strokes = session.ink_strokes();
                let left: Vec<PageStroke> = strokes
                    .iter()
                    .filter(|stroke| {
                        !(stroke.page == page
                            && points
                                .iter()
                                .any(|&point| stroke.touches(point, ERASER_RADIUS)))
                    })
                    .cloned()
                    .collect();
                if left.len() == strokes.len() {
                    return Ok(DrawRelease::Nothing);
                }
                session.commit_ink(&left)?;
                Ok(DrawRelease::Edited)
            }
            DrawTool::Eraser(EraserKind::Point) => Ok(
                if session.erase_points_along(page, &points, ERASER_RADIUS)? {
                    DrawRelease::Edited
                } else {
                    DrawRelease::Nothing
                },
            ),
            DrawTool::Lasso => {
                let caught = session.lasso_strokes(page, &points);
                let strokes = session.ink_strokes();
                let selected: Vec<PageStroke> = caught
                    .iter()
                    .filter_map(|&index| strokes.get(index).cloned())
                    .collect();
                self.selected = (!selected.is_empty()).then_some((page, selected));
                Ok(DrawRelease::Selected)
            }
            DrawTool::Select => Ok(DrawRelease::Nothing),
        }
    }

    /// A gesture the text did not take: the stroke goes in as ink.
    pub fn ink_instead<S: WordInkTools + ?Sized>(
        &self,
        session: &mut S,
        page: usize,
        points: Vec<(f64, f64)>,
    ) -> Result<(), String> {
        let Some(pen) = self.state.pen().cloned() else {
            return Ok(());
        };
        session
            .draw_stroke(page, points, &pen, self.state.ink_to_shape)
            .map(|_| ())
    }

    /// Indices of the selected strokes among the session's.
    pub fn selected_indices<S: WordInkTools + ?Sized>(&self, session: &S) -> Vec<usize> {
        let Some((_, selected)) = self.selected.as_ref() else {
            return Vec::new();
        };
        session
            .ink_strokes()
            .iter()
            .enumerate()
            .filter(|(_, stroke)| selected.iter().any(|wanted| wanted.trace == stroke.trace))
            .map(|(index, _)| index)
            .collect()
    }

    /// The selection again after an edit: the strokes now like `wanted`.
    fn reselect<S: WordInkTools + ?Sized>(
        &mut self,
        session: &S,
        page: usize,
        wanted: &[PageStroke],
    ) {
        let close = |a: &PageStroke, b: &PageStroke| {
            a.page == b.page
                && a.trace.brush == b.trace.brush
                && a.trace.points.len() == b.trace.points.len()
                && a.trace
                    .points
                    .iter()
                    .zip(&b.trace.points)
                    .all(|(p, q)| (p.0 - q.0).abs() < 0.05 && (p.1 - q.1).abs() < 0.05)
        };
        let selected: Vec<PageStroke> = session
            .ink_strokes()
            .into_iter()
            .filter(|stroke| wanted.iter().any(|want| close(stroke, want)))
            .collect();
        self.selected = (!selected.is_empty()).then_some((page, selected));
    }

    /// Moves or resizes the selection onto `to`, keeping it selected.
    pub fn move_selection<S: WordInkTools + ?Sized>(
        &mut self,
        session: &mut S,
        to: Box4,
    ) -> Result<(), String> {
        let Some((page, selected)) = self.selected.clone() else {
            return Ok(());
        };
        let Some(from) = strokes_bounds(selected.iter()) else {
            return Ok(());
        };
        let indices = self.selected_indices(session);
        session.transform_strokes(&indices, to)?;
        let moved: Vec<PageStroke> = selected
            .iter()
            .map(|stroke| transform_stroke(stroke, from, to))
            .collect();
        self.reselect(session, page, &moved);
        Ok(())
    }

    /// Deletes the selection.
    pub fn delete_selection<S: WordInkTools + ?Sized>(
        &mut self,
        session: &mut S,
    ) -> Result<(), String> {
        let indices = self.selected_indices(session);
        self.selected = None;
        session.delete_strokes(&indices)
    }

    /// Turns the selected strokes that are shapes into shapes.
    pub fn convert_selection<S: WordInkTools + ?Sized>(
        &mut self,
        session: &mut S,
    ) -> Result<usize, String> {
        let indices = self.selected_indices(session);
        self.selected = None;
        session.convert_strokes_to_shapes(&indices)
    }

    /// Gives the selection pen `index`'s look; with nothing selected, picks
    /// the pen up. Whether the document changed.
    pub fn restyle_selection<S: WordInkTools + ?Sized>(
        &mut self,
        session: &mut S,
        index: usize,
    ) -> Result<bool, String> {
        let Some((page, selected)) = self.selected.clone() else {
            self.state.tool = DrawTool::Pen(index);
            return Ok(false);
        };
        let pen = self.state.pens[index].clone();
        let indices = self.selected_indices(session);
        session.restyle_strokes(&indices, &pen)?;
        let restyled: Vec<PageStroke> = selected
            .into_iter()
            .map(|mut stroke| {
                stroke.trace.brush = pen.brush();
                stroke
            })
            .collect();
        self.reselect(session, page, &restyled);
        Ok(true)
    }

    /// What to show over page `page`, if anything.
    pub fn overlay(&self, page: usize) -> Option<InkOverlay> {
        let gesture = self
            .gesture
            .as_ref()
            .filter(|(gesture_page, _)| *gesture_page == page)
            .map(|(_, points)| match (self.state.tool, self.state.pen()) {
                (DrawTool::Pen(_), Some(pen)) => {
                    let brush = pen.brush();
                    OverlayStroke {
                        points: points.clone(),
                        color: pen.color.clone(),
                        width: brush.width_px.max(brush.height_px),
                        opacity: match pen.tool {
                            InkTool::Highlighter => 0.45,
                            InkTool::Pencil => 0.75,
                            InkTool::Pen => 1.0,
                        },
                        closed: false,
                    }
                }
                (DrawTool::Lasso, _) => OverlayStroke {
                    points: points.clone(),
                    color: "#2B579A".to_string(),
                    width: 1.0,
                    opacity: 1.0,
                    closed: true,
                },
                _ => OverlayStroke {
                    points: points.clone(),
                    color: "#808080".to_string(),
                    width: ERASER_RADIUS * 2.0,
                    opacity: 0.35,
                    closed: false,
                },
            });
        let selection = self
            .selection_box()
            .filter(|(selected_page, _)| *selected_page == page)
            .map(|(_, bounds)| bounds);
        let preview = match (self.selected.as_ref(), self.drag.as_ref()) {
            (Some((selected_page, strokes)), Some(drag)) if *selected_page == page => strokes
                .iter()
                .map(|stroke| transform_stroke(stroke, drag.from_box, drag.to_box))
                .collect(),
            _ => Vec::new(),
        };
        (gesture.is_some() || selection.is_some()).then_some(InkOverlay {
            gesture,
            selection,
            preview,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn circle(cx: f64, cy: f64, rx: f64, ry: f64, n: usize) -> Vec<(f64, f64)> {
        (0..=n)
            .map(|i| {
                let a = i as f64 / n as f64 * std::f64::consts::TAU;
                (cx + rx * a.cos(), cy + ry * a.sin())
            })
            .collect()
    }

    fn polyline(corners: &[(f64, f64)], per_side: usize) -> Vec<(f64, f64)> {
        let mut points = Vec::new();
        for pair in corners.windows(2) {
            for step in 0..per_side {
                let t = step as f64 / per_side as f64;
                // A little hand wobble.
                let wobble = ((step * 7) % 3) as f64 * 0.6 - 0.6;
                points.push((
                    pair[0].0 + (pair[1].0 - pair[0].0) * t + wobble,
                    pair[0].1 + (pair[1].1 - pair[0].1) * t - wobble,
                ));
            }
        }
        points.push(*corners.last().unwrap());
        points
    }

    #[test]
    fn shapes_are_recognised() {
        let rect = polyline(
            &[
                (10.0, 10.0),
                (110.0, 10.0),
                (110.0, 70.0),
                (10.0, 70.0),
                (12.0, 12.0),
            ],
            20,
        );
        assert!(matches!(
            recognize_shape(&rect),
            Some(InkShape::Rectangle(_))
        ));
        let triangle = polyline(
            &[(60.0, 10.0), (110.0, 90.0), (10.0, 90.0), (59.0, 12.0)],
            20,
        );
        assert!(matches!(
            recognize_shape(&triangle),
            Some(InkShape::Triangle(_))
        ));
        let diamond = polyline(
            &[
                (60.0, 10.0),
                (110.0, 50.0),
                (60.0, 90.0),
                (10.0, 50.0),
                (59.0, 11.0),
            ],
            20,
        );
        assert!(matches!(
            recognize_shape(&diamond),
            Some(InkShape::Diamond(_))
        ));
        let ellipse = circle(100.0, 100.0, 60.0, 35.0, 48);
        match recognize_shape(&ellipse) {
            Some(InkShape::Ellipse((l, t, r, b))) => {
                assert!((l - 40.0).abs() < 1.0 && (r - 160.0).abs() < 1.0);
                assert!((t - 65.0).abs() < 1.0 && (b - 135.0).abs() < 1.0);
            }
            other => panic!("{other:?}"),
        }
        let line = polyline(&[(10.0, 10.0), (200.0, 80.0)], 30);
        assert!(matches!(
            recognize_shape(&line),
            Some(InkShape::Line { arrow: false, .. })
        ));
        let mut arrow = polyline(&[(10.0, 100.0), (200.0, 100.0)], 30);
        arrow.extend(polyline(&[(200.0, 100.0), (180.0, 85.0)], 5));
        assert!(matches!(
            recognize_shape(&arrow),
            Some(InkShape::Line { arrow: true, .. })
        ));
        // Handwriting is not a shape.
        let scribble: Vec<(f64, f64)> = (0..60)
            .map(|i| (i as f64 * 3.0, if i % 2 == 0 { 0.0 } else { 25.0 }))
            .collect();
        assert_eq!(recognize_shape(&scribble), None);
    }

    #[test]
    fn gestures_are_recognised() {
        let strike = polyline(&[(10.0, 50.0), (150.0, 52.0)], 20);
        assert!(matches!(
            recognize_gesture(&strike),
            Some(InkGesture::Delete { .. })
        ));
        let loop_ = circle(80.0, 50.0, 60.0, 20.0, 40);
        assert!(matches!(
            recognize_gesture(&loop_),
            Some(InkGesture::Select(_))
        ));
        let caret = polyline(&[(50.0, 80.0), (60.0, 55.0), (70.0, 80.0)], 10);
        match recognize_gesture(&caret) {
            Some(InkGesture::Insert((x, y))) => {
                assert!((x - 60.0).abs() < 2.0, "{x}");
                assert!(y >= 79.0);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_point_eraser_cuts_a_stroke_in_two() {
        let stroke = PageStroke {
            page: 0,
            trace: InkTrace {
                points: vec![(0.0, 0.0), (100.0, 0.0)],
                brush: Pen::new(InkTool::Pen, "#000000", 0.5).brush(),
            },
            source: Some((0, 1)),
        };
        let pieces = erase_points(&stroke, &[(50.0, -10.0), (50.0, 10.0)], 4.0);
        assert_eq!(pieces.len(), 2);
        assert!(pieces[0].trace.points.iter().all(|point| point.0 < 46.0));
        assert!(pieces[1].trace.points.iter().all(|point| point.0 > 54.0));
        assert!(pieces.iter().all(|piece| piece.source.is_none()));
        assert!(erase_points(&stroke, &[(-10.0, 0.0), (110.0, 0.0)], 4.0).is_empty());
    }

    #[test]
    fn native_brush_bounds_preserve_exact_translation() {
        let stroke = PageStroke {
            page: 0,
            trace: InkTrace {
                points: vec![(10.0, 10.0), (20.0, 20.0)],
                brush: InkBrush {
                    width_px: 2.0,
                    height_px: 12.0,
                    ..InkBrush::default()
                },
            },
            source: None,
        };
        let from = stroke.bounds();
        assert!(from.0 < 10.0 && from.1 < 10.0 && from.2 > 20.0 && from.3 > 20.0);
        let moved = transform_stroke(
            &stroke,
            from,
            (from.0 + 1.0, from.1 + 10.0, from.2 + 1.0, from.3 + 10.0),
        );
        assert_eq!(moved.trace.points, vec![(11.0, 20.0), (21.0, 30.0)]);
    }

    #[test]
    fn lasso_and_transform_math() {
        let square = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        assert!(point_in_polygon((5.0, 5.0), &square));
        assert!(!point_in_polygon((15.0, 5.0), &square));
        let stroke = PageStroke {
            page: 0,
            trace: InkTrace {
                points: vec![(10.0, 10.0), (20.0, 20.0)],
                brush: InkBrush::default(),
            },
            source: None,
        };
        let moved = transform_stroke(
            &stroke,
            (10.0, 10.0, 20.0, 20.0),
            (100.0, 50.0, 120.0, 60.0),
        );
        assert_eq!(moved.trace.points, vec![(100.0, 50.0), (120.0, 60.0)]);
    }
}
