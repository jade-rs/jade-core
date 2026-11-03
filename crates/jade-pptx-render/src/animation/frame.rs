//! Paints a slide layout as a [`SlideFrame`] has it: each shape, its body,
//! paragraphs and letters moved, scaled, turned, faded, masked and
//! recoloured.

use jade_painter::{ImageRect, ImageSource, Painter, TextMetrics};
use jade_pptx_layout::{PlacedLine, PlacedShape, Rect, SlideLayout};
use unicode_segmentation::UnicodeSegmentation;

use crate::animation::evaluate::{KeyPart, Reveal, SlideFrame, TargetBase, TargetKey, TargetState};
use crate::animation::player::SlideText;
use crate::animation::reveal::reveal_path;
use crate::{font_shorthand, render_background, render_shape_clipped, DrawImage, RenderOptions};

/// A target's untouched geometry and colours on `layout`, for the
/// player's formulas.
pub fn target_bounds(layout: &SlideLayout, text: &SlideText, key: &TargetKey) -> TargetBase {
    let (width, height) = (layout.width_px.max(1.0), layout.height_px.max(1.0));
    let members: Vec<&PlacedShape> = layout
        .shapes
        .iter()
        .filter(|shape| belongs(shape, key.shape_id))
        .collect();
    let Some(first) = members.first() else {
        return TargetBase::default();
    };
    let rect = match key.part {
        KeyPart::Paragraph(paragraph) if members.len() == 1 => {
            paragraph_rect(first, text.get(&key.shape_id), paragraph).map(|local| {
                let paint = first.paint_rect();
                Rect {
                    x: local.x - paint.x + first.rect.x,
                    y: local.y - paint.y + first.rect.y,
                    ..local
                }
            })
        }
        _ => None,
    }
    .unwrap_or_else(|| union(members.iter().map(|shape| shape.rect)));
    let text_color = first
        .lines
        .iter()
        .flat_map(|line| &line.runs)
        .map(|run| run.color.clone())
        .next();
    TargetBase {
        x: (rect.x + rect.width / 2.0) / width,
        y: (rect.y + rect.height / 2.0) / height,
        w: rect.width / width,
        h: rect.height / height,
        r: if members.len() == 1 {
            first.rotation_rad.to_degrees()
        } else {
            0.0
        },
        fill: first.fill.clone(),
        line: first.outline.as_ref().map(|outline| outline.color.clone()),
        text: text_color,
    }
}

/// Paints `layout` in the state `frame` gives each target.
pub fn render_slide_frame<P: Painter>(
    painter: &P,
    layout: &SlideLayout,
    options: &RenderOptions,
    draw_image: &mut DrawImage<'_>,
    frame: &SlideFrame,
    text: &SlideText,
) -> Result<(), P::Error> {
    render_background(painter, layout, options)?;
    painter.set_text_baseline("alphabetic");
    let size = (layout.width_px, layout.height_px);
    for shape in &layout.shapes {
        let own = shape.source_id;
        let touched = |id: u32| frame.states.keys().any(|key| key.shape_id == id);
        let ancestors_touched = shape.group_ids.iter().any(|&id| touched(id));
        if !ancestors_touched && !own.is_some_and(touched) {
            render_shape_clipped(painter, shape, options, draw_image, None)?;
            continue;
        }
        // Groups outermost first, each moved about its own bounds.
        let mut outer: Vec<(TargetState, Rect)> = Vec::new();
        for &group in &shape.group_ids {
            let key = TargetKey::shape(group);
            if let Some(state) = frame.states.get(&key) {
                outer.push((state.clone(), group_rect(layout, group)));
            }
        }
        if outer.iter().any(|(state, _)| !state.shows()) {
            continue;
        }
        painter.save();
        let mut alpha = 1.0;
        for (state, rect) in &outer {
            alpha *= state.opacity;
            apply_transform(painter, state, *rect, size)?;
            if let Some(reveal) = &state.reveal {
                reveal_path(painter, *rect, &reveal.filter, reveal.progress);
                painter.clip();
            }
        }
        let shape_state = own
            .and_then(|id| frame.states.get(&TargetKey::shape(id)))
            .cloned()
            .unwrap_or_default();
        let result = draw_shape(
            painter,
            shape,
            options,
            draw_image,
            frame,
            text,
            &shape_state,
            alpha,
            size,
        );
        painter.restore();
        result?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_shape<P: Painter>(
    painter: &P,
    shape: &PlacedShape,
    options: &RenderOptions,
    draw_image: &mut DrawImage<'_>,
    frame: &SlideFrame,
    text: &SlideText,
    state: &TargetState,
    alpha: f64,
    size: (f64, f64),
) -> Result<(), P::Error> {
    if !state.shows() {
        return Ok(());
    }
    let id = shape.source_id.unwrap_or(u32::MAX);
    let paragraphs: Vec<u32> = frame
        .states
        .keys()
        .filter(|key| key.shape_id == id)
        .filter_map(|key| match key.part {
            KeyPart::Paragraph(paragraph) => Some(paragraph),
            _ => None,
        })
        .collect();
    let body_state = frame.states.get(&TargetKey {
        shape_id: id,
        part: KeyPart::Body,
    });
    let units = frame.has_units(id);
    let rect = shape.rect;
    let local = shape.paint_rect();
    painter.save();
    let alpha = alpha * state.opacity;
    apply_transform(painter, state, rect, size)?;
    let result = (|| {
        if paragraphs.is_empty() && body_state.is_none() && !units {
            let piece = recolor(shape, state);
            return draw_piece(
                painter,
                &piece,
                options,
                draw_image,
                alpha,
                state.reveal.as_ref().map(|r| (r, local)),
                &[],
            );
        }
        // The body, then the text paragraph by paragraph.
        let mut body = recolor(shape, state);
        body.lines.clear();
        let body_state = body_state.cloned().unwrap_or_default();
        if body_state.shows() {
            painter.save();
            apply_transform(painter, &body_state, rect, size)?;
            let body = recolor(&body, &body_state);
            let reveal = body_state
                .reveal
                .as_ref()
                .or(state.reveal.as_ref())
                .map(|r| (r, local));
            draw_piece(
                painter,
                &body,
                options,
                draw_image,
                alpha * body_state.opacity,
                reveal,
                &[],
            )?;
            painter.restore();
        }
        let starts = paragraph_starts(text.get(&id));
        let mut text_only = recolor(shape, state);
        strip_body(&mut text_only);
        let count = paragraph_count(shape, &starts);
        for paragraph in 0..count {
            let key = TargetKey {
                shape_id: id,
                part: KeyPart::Paragraph(paragraph as u32),
            };
            let paragraph_state = frame.state(&key);
            if !paragraph_state.shows() {
                continue;
            }
            let mut piece = recolor(&text_only, &paragraph_state);
            piece
                .lines
                .retain(|line| paragraph_of(line, &starts) == paragraph);
            if piece.lines.is_empty() {
                continue;
            }
            let box_local = lines_rect(&piece, &piece.lines);
            let box_slide = Rect {
                x: box_local.x - local.x + rect.x,
                y: box_local.y - local.y + rect.y,
                ..box_local
            };
            painter.save();
            apply_transform(painter, &paragraph_state, box_slide, size)?;
            let reveal = paragraph_state
                .reveal
                .as_ref()
                .map(|r| (r, box_local))
                .or(state.reveal.as_ref().map(|r| (r, local)));
            let alpha = alpha * paragraph_state.opacity;
            let outcome = if units {
                draw_units(
                    painter, &piece, options, draw_image, frame, text, alpha, reveal, size,
                )
            } else {
                draw_piece(painter, &piece, options, draw_image, alpha, reveal, &[])
            };
            painter.restore();
            outcome?;
        }
        Ok(())
    })();
    painter.restore();
    result
}

/// Letters or words, each in its own state, clipped to its own box.
#[allow(clippy::too_many_arguments)]
fn draw_units<P: Painter>(
    painter: &P,
    shape: &PlacedShape,
    options: &RenderOptions,
    draw_image: &mut DrawImage<'_>,
    frame: &SlideFrame,
    text: &SlideText,
    alpha: f64,
    reveal: Option<(&Reveal, Rect)>,
    size: (f64, f64),
) -> Result<(), P::Error> {
    let id = shape.source_id.unwrap_or(u32::MAX);
    let words = frame
        .states
        .keys()
        .any(|key| key.shape_id == id && matches!(key.part, KeyPart::Word(_)));
    let table = UnitTable::new(text.get(&id).map(Vec::as_slice).unwrap_or(&[]), words);
    let local = shape.paint_rect();
    // Each unit's boxes in the shape's local frame, with the runs it is in.
    let mut pieces: Vec<UnitPiece> = Vec::new();
    for (line_index, line) in shape.lines.iter().enumerate() {
        for (run_index, run) in line.runs.iter().enumerate() {
            let graphemes: Vec<&str> = run.text.graphemes(true).collect();
            if graphemes.is_empty() {
                continue;
            }
            painter.set_font(&font_shorthand(run, options));
            let measured = painter
                .measure_text_width(&run.text)
                .unwrap_or(run.width)
                .max(1e-6);
            let stretch = if run.width > 0.0 {
                run.width / measured
            } else {
                1.0
            };
            let mut prefix = String::new();
            let mut x0 = 0.0;
            for (index, grapheme) in graphemes.iter().enumerate() {
                prefix.push_str(grapheme);
                let x1 = painter.measure_text_width(&prefix).unwrap_or(0.0) * stretch;
                let offset = run.source_offsets.get(index).copied();
                let unit = offset.and_then(|offset| table.unit_at(offset));
                if let Some(unit) = unit {
                    let rect = Rect {
                        x: local.x + run.x + x0,
                        y: local.y + line.top,
                        width: (x1 - x0).max(0.0),
                        height: line.height.max(1.0),
                    };
                    match pieces.iter_mut().find(|(u, _, _)| *u == unit) {
                        Some((_, rects, runs)) => {
                            rects.push(rect);
                            if !runs.contains(&(line_index, run_index)) {
                                runs.push((line_index, run_index));
                            }
                        }
                        None => pieces.push((unit, vec![rect], vec![(line_index, run_index)])),
                    }
                }
                x0 = x1;
            }
        }
    }
    for (unit, rects, runs) in pieces {
        let key = TargetKey {
            shape_id: id,
            part: if words {
                KeyPart::Word(unit)
            } else {
                KeyPart::Letter(unit)
            },
        };
        let state = frame.state(&key);
        if !state.shows() {
            continue;
        }
        let mut piece = recolor(shape, &state);
        piece.lines = piece
            .lines
            .iter()
            .enumerate()
            .filter_map(|(line_index, line)| {
                let kept: Vec<_> = line
                    .runs
                    .iter()
                    .enumerate()
                    .filter(|(run_index, _)| runs.contains(&(line_index, *run_index)))
                    .map(|(_, run)| run.clone())
                    .collect();
                (!kept.is_empty()).then(|| PlacedLine {
                    runs: kept,
                    ..line.clone()
                })
            })
            .collect();
        let bounds = union(rects.iter().copied());
        let slide_bounds = Rect {
            x: bounds.x - local.x + shape.rect.x,
            y: bounds.y - local.y + shape.rect.y,
            ..bounds
        };
        painter.save();
        apply_transform(painter, &state, slide_bounds, size)?;
        let unit_reveal = state.reveal.as_ref().map(|r| (r, bounds)).or(reveal);
        let result = draw_piece(
            painter,
            &piece,
            options,
            draw_image,
            alpha * state.opacity,
            unit_reveal,
            &rects,
        );
        painter.restore();
        result?;
    }
    Ok(())
}

/// One piece drawn through the shape's own transform, faded by `alpha`,
/// clipped to its reveal mask and to `boxes` when there are any.
fn draw_piece<P: Painter>(
    painter: &P,
    shape: &PlacedShape,
    options: &RenderOptions,
    draw_image: &mut DrawImage<'_>,
    alpha: f64,
    reveal: Option<(&Reveal, Rect)>,
    boxes: &[Rect],
) -> Result<(), P::Error> {
    if alpha <= 0.0 {
        return Ok(());
    }
    let clip = |painter: &P| {
        if let Some((reveal, rect)) = reveal {
            reveal_path(painter, rect, &reveal.filter, reveal.progress);
            painter.clip();
        }
        if !boxes.is_empty() {
            painter.begin_path();
            for rect in boxes {
                painter.rect(rect.x, rect.y, rect.width, rect.height);
            }
            painter.clip();
        }
    };
    let clipped = reveal.is_some() || !boxes.is_empty();
    if alpha >= 1.0 {
        return render_shape_clipped(
            painter,
            shape,
            options,
            draw_image,
            clipped.then_some(&clip),
        );
    }
    let faded = Faded {
        inner: painter,
        alpha,
    };
    painter.save();
    painter.set_global_alpha(alpha);
    let faded_clip = |faded: &Faded<'_, P>| clip(faded.inner);
    let result = render_shape_clipped(
        &faded,
        shape,
        options,
        draw_image,
        clipped.then_some(&faded_clip),
    );
    painter.restore();
    result
}

/// Runs `draw` through a painter faded to `alpha`.
pub(crate) fn draw_faded<P: Painter>(
    painter: &P,
    alpha: f64,
    draw: impl FnOnce(&Faded<'_, P>) -> Result<(), P::Error>,
) -> Result<(), P::Error> {
    if alpha <= 0.0 {
        return Ok(());
    }
    let faded = Faded {
        inner: painter,
        alpha: alpha.min(1.0),
    };
    painter.save();
    painter.set_global_alpha(alpha.min(1.0));
    let result = draw(&faded);
    painter.restore();
    result
}

/// Moves, turns and scales about `rect`'s centre (slide px).
fn apply_transform<P: Painter>(
    painter: &P,
    state: &TargetState,
    rect: Rect,
    size: (f64, f64),
) -> Result<(), P::Error> {
    let (dx, dy) = (state.offset.0 * size.0, state.offset.1 * size.1);
    if dx != 0.0 || dy != 0.0 {
        painter.translate(dx, dy)?;
    }
    if state.rotation_deg != 0.0 || state.scale != (1.0, 1.0) {
        let (cx, cy) = (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
        painter.translate(cx, cy)?;
        if state.rotation_deg != 0.0 {
            painter.rotate(state.rotation_deg.to_radians())?;
        }
        if state.scale != (1.0, 1.0) {
            // A zero scale would make the transform singular.
            painter.scale(state.scale.0.max(1e-4), state.scale.1.max(1e-4))?;
        }
        painter.translate(-cx, -cy)?;
    }
    Ok(())
}

/// The shape with the state's colour overrides.
fn recolor(shape: &PlacedShape, state: &TargetState) -> PlacedShape {
    let mut shape = shape.clone();
    if state.fill_on == Some(false) {
        shape.fill = None;
        shape.gradient = None;
    }
    if let Some(fill) = &state.fill {
        if state.fill_on != Some(false) {
            shape.fill = Some(fill.clone());
            shape.gradient = None;
        }
    }
    if state.line_on == Some(false) {
        shape.outline = None;
    }
    if let (Some(color), Some(outline)) = (&state.line, shape.outline.as_mut()) {
        outline.color = color.clone();
    }
    for line in &mut shape.lines {
        for run in &mut line.runs {
            if let Some(color) = &state.text_color {
                run.color = color.clone();
            }
            if let Some(bold) = state.bold {
                run.bold = bold;
            }
            if let Some(underline) = state.underline {
                run.underline = underline;
            }
        }
    }
    shape
}

fn strip_body(shape: &mut PlacedShape) {
    shape.fill = None;
    shape.gradient = None;
    shape.outline = None;
    shape.image_part = None;
    shape.shadow = None;
    shape.pattern = None;
    shape.effects.reflection = None;
    shape.effects.inner_shadow = None;
    shape.effects.soft_edge_px = None;
}

fn belongs(shape: &PlacedShape, id: u32) -> bool {
    shape.source_id == Some(id) || shape.group_ids.contains(&id)
}

fn group_rect(layout: &SlideLayout, id: u32) -> Rect {
    union(
        layout
            .shapes
            .iter()
            .filter(|shape| belongs(shape, id))
            .map(|shape| shape.rect),
    )
}

fn union(rects: impl Iterator<Item = Rect>) -> Rect {
    let mut bounds: Option<(f64, f64, f64, f64)> = None;
    for rect in rects {
        let next = (rect.x, rect.y, rect.x + rect.width, rect.y + rect.height);
        bounds = Some(match bounds {
            None => next,
            Some(b) => (
                b.0.min(next.0),
                b.1.min(next.1),
                b.2.max(next.2),
                b.3.max(next.3),
            ),
        });
    }
    let (x0, y0, x1, y1) = bounds.unwrap_or_default();
    Rect {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    }
}

/// Each paragraph's first UTF-16 offset in the shape's text, the way the
/// layout counts it (one unit between paragraphs).
fn paragraph_starts(paragraphs: Option<&Vec<String>>) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut offset = 0;
    for paragraph in paragraphs.map(Vec::as_slice).unwrap_or(&[]) {
        starts.push(offset);
        offset += paragraph.encode_utf16().count() + 1;
    }
    starts
}

fn paragraph_of(line: &PlacedLine, starts: &[usize]) -> usize {
    let Some((start, _)) = line.source_range else {
        return 0;
    };
    starts
        .partition_point(|&first| first <= start)
        .saturating_sub(1)
}

fn paragraph_count(shape: &PlacedShape, starts: &[usize]) -> usize {
    let from_lines = shape
        .lines
        .iter()
        .map(|line| paragraph_of(line, starts) + 1)
        .max()
        .unwrap_or(0);
    from_lines.max(starts.len())
}

/// A paragraph's box in the shape's local frame.
fn paragraph_rect(shape: &PlacedShape, text: Option<&Vec<String>>, paragraph: u32) -> Option<Rect> {
    let starts = paragraph_starts(text);
    let lines: Vec<PlacedLine> = shape
        .lines
        .iter()
        .filter(|line| paragraph_of(line, &starts) == paragraph as usize)
        .cloned()
        .collect();
    (!lines.is_empty()).then(|| lines_rect(shape, &lines))
}

fn lines_rect(shape: &PlacedShape, lines: &[PlacedLine]) -> Rect {
    let local = shape.paint_rect();
    union(lines.iter().map(|line| {
        let left = line
            .runs
            .iter()
            .map(|run| run.x)
            .fold(f64::INFINITY, f64::min);
        let right = line
            .runs
            .iter()
            .map(|run| run.x + run.width)
            .fold(f64::NEG_INFINITY, f64::max);
        let (left, right) = if left.is_finite() {
            (left, right)
        } else {
            (line.text_left, line.text_left)
        };
        Rect {
            x: local.x + left,
            y: local.y + line.top,
            width: (right - left).max(0.0),
            height: line.height,
        }
    }))
}

/// A unit number, its boxes and the (line, run) pairs it is drawn in.
type UnitPiece = (usize, Vec<Rect>, Vec<(usize, usize)>);

/// Maps UTF-16 offsets in a shape's text to letter or word numbers.
struct UnitTable {
    /// (start offset, end offset, unit) spans.
    spans: Vec<(usize, usize, usize)>,
}

impl UnitTable {
    fn new(paragraphs: &[String], words: bool) -> Self {
        let mut spans = Vec::new();
        let mut offset = 0;
        let mut unit = 0;
        for paragraph in paragraphs {
            let mut in_word = false;
            for ch in paragraph.chars() {
                let width = ch.len_utf16();
                if words {
                    if ch.is_whitespace() {
                        if in_word {
                            unit += 1;
                        }
                        in_word = false;
                    } else {
                        in_word = true;
                        spans.push((offset, offset + width, unit));
                    }
                } else {
                    spans.push((offset, offset + width, unit));
                    unit += 1;
                }
                offset += width;
            }
            if words && in_word {
                unit += 1;
            }
            offset += 1;
        }
        Self { spans }
    }

    fn unit_at(&self, offset: usize) -> Option<usize> {
        let index = self.spans.partition_point(|span| span.1 <= offset);
        self.spans
            .get(index)
            .filter(|span| span.0 <= offset)
            .map(|span| span.2)
    }
}

/// A painter whose global alpha is scaled by a shape's opacity, so a
/// fading shape's own transparencies compound with it.
pub(crate) struct Faded<'a, P: Painter> {
    inner: &'a P,
    alpha: f64,
}

impl<P: Painter> Painter for Faded<'_, P> {
    type Error = P::Error;

    fn draw_image(
        &self,
        image: ImageSource<'_>,
        source: Option<[f64; 4]>,
        destination: ImageRect,
    ) -> Result<bool, Self::Error> {
        self.inner.draw_image(image, source, destination)
    }
    fn save(&self) {
        self.inner.save()
    }
    fn restore(&self) {
        self.inner.restore()
    }
    fn translate(&self, x: f64, y: f64) -> Result<(), Self::Error> {
        self.inner.translate(x, y)
    }
    fn scale(&self, x: f64, y: f64) -> Result<(), Self::Error> {
        self.inner.scale(x, y)
    }
    fn rotate(&self, angle: f64) -> Result<(), Self::Error> {
        self.inner.rotate(angle)
    }
    fn set_fill_style_str(&self, value: &str) {
        self.inner.set_fill_style_str(value)
    }
    fn set_stroke_style_str(&self, value: &str) {
        self.inner.set_stroke_style_str(value)
    }
    fn set_line_width(&self, width: f64) {
        self.inner.set_line_width(width)
    }
    fn set_global_alpha(&self, alpha: f64) {
        self.inner.set_global_alpha(alpha * self.alpha)
    }
    fn set_line_dash(&self, segments: &[f64]) {
        self.inner.set_line_dash(segments)
    }
    fn set_shadow_color(&self, value: &str) {
        self.inner.set_shadow_color(value)
    }
    fn set_shadow_blur(&self, value: f64) {
        self.inner.set_shadow_blur(value)
    }
    fn set_shadow_offset_x(&self, value: f64) {
        self.inner.set_shadow_offset_x(value)
    }
    fn set_shadow_offset_y(&self, value: f64) {
        self.inner.set_shadow_offset_y(value)
    }
    fn set_blur_filter(&self, radius_px: f64) {
        self.inner.set_blur_filter(radius_px)
    }
    fn set_font(&self, value: &str) {
        self.inner.set_font(value)
    }
    fn set_text_baseline(&self, value: &str) {
        self.inner.set_text_baseline(value)
    }
    fn fill_text(&self, text: &str, x: f64, y: f64) -> Result<(), Self::Error> {
        self.inner.fill_text(text, x, y)
    }
    fn stroke_text(&self, text: &str, x: f64, y: f64) -> Result<(), Self::Error> {
        self.inner.stroke_text(text, x, y)
    }
    fn measure_text_width(&self, text: &str) -> Result<f64, Self::Error> {
        self.inner.measure_text_width(text)
    }
    fn measure_text_metrics(&self, text: &str) -> Result<TextMetrics, Self::Error> {
        self.inner.measure_text_metrics(text)
    }
    fn fill_rect(&self, x: f64, y: f64, width: f64, height: f64) {
        self.inner.fill_rect(x, y, width, height)
    }
    fn fill_horizontal_gradient_rect(
        &self,
        rect: ImageRect,
        start: [u8; 4],
        end: [u8; 4],
    ) -> Result<(), Self::Error> {
        self.inner.fill_horizontal_gradient_rect(rect, start, end)
    }
    fn clear_rect(&self, x: f64, y: f64, width: f64, height: f64) {
        self.inner.clear_rect(x, y, width, height)
    }
    fn stroke_rect(&self, x: f64, y: f64, width: f64, height: f64) {
        self.inner.stroke_rect(x, y, width, height)
    }
    fn begin_path(&self) {
        self.inner.begin_path()
    }
    fn close_path(&self) {
        self.inner.close_path()
    }
    fn move_to(&self, x: f64, y: f64) {
        self.inner.move_to(x, y)
    }
    fn line_to(&self, x: f64, y: f64) {
        self.inner.line_to(x, y)
    }
    fn rect(&self, x: f64, y: f64, width: f64, height: f64) {
        self.inner.rect(x, y, width, height)
    }
    fn arc(
        &self,
        x: f64,
        y: f64,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
    ) -> Result<(), Self::Error> {
        self.inner.arc(x, y, radius, start_angle, end_angle)
    }
    fn quadratic_curve_to(&self, cp_x: f64, cp_y: f64, x: f64, y: f64) {
        self.inner.quadratic_curve_to(cp_x, cp_y, x, y)
    }
    fn bezier_curve_to(&self, cp1_x: f64, cp1_y: f64, cp2_x: f64, cp2_y: f64, x: f64, y: f64) {
        self.inner.bezier_curve_to(cp1_x, cp1_y, cp2_x, cp2_y, x, y)
    }
    fn ellipse(
        &self,
        x: f64,
        y: f64,
        radius_x: f64,
        radius_y: f64,
        rotation: f64,
        start_angle: f64,
        end_angle: f64,
    ) -> Result<(), Self::Error> {
        self.inner
            .ellipse(x, y, radius_x, radius_y, rotation, start_angle, end_angle)
    }
    fn ellipse_with_anticlockwise(
        &self,
        x: f64,
        y: f64,
        radius_x: f64,
        radius_y: f64,
        rotation: f64,
        start_angle: f64,
        end_angle: f64,
        anticlockwise: bool,
    ) -> Result<(), Self::Error> {
        self.inner.ellipse_with_anticlockwise(
            x,
            y,
            radius_x,
            radius_y,
            rotation,
            start_angle,
            end_angle,
            anticlockwise,
        )
    }
    fn fill(&self) {
        self.inner.fill()
    }
    fn stroke(&self) {
        self.inner.stroke()
    }
    fn clip(&self) {
        self.inner.clip()
    }
    fn use_fill_style_for_stroke(&self) {
        self.inner.use_fill_style_for_stroke()
    }
    fn try_fill_glyph_run_batch(
        &self,
        text: &str,
        x_positions: &[f64],
        y: f64,
    ) -> Result<bool, Self::Error> {
        self.inner.try_fill_glyph_run_batch(text, x_positions, y)
    }
    fn try_fill_shaped_text(
        &self,
        text: &str,
        clusters: &[(usize, f64)],
        y: f64,
    ) -> Result<bool, Self::Error> {
        self.inner.try_fill_shaped_text(text, clusters, y)
    }
    fn try_fill_glyph_run_batch_multi_font(
        &self,
        text: &str,
        x_positions: &[f64],
        font_indexes: &[u16],
        fonts: &[std::rc::Rc<str>],
        y: f64,
    ) -> Result<bool, Self::Error> {
        self.inner
            .try_fill_glyph_run_batch_multi_font(text, x_positions, font_indexes, fonts, y)
    }
    fn set_structure_node(&self, node: i32) {
        self.inner.set_structure_node(node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::evaluate::TargetState;
    use crate::animation::test_support::{layout, shape};
    use jade_painter::recording::{Op, RecordingPainter};

    #[test]
    fn frames_fade_move_and_hide_shapes() {
        let slide = layout(vec![
            shape(2, "Box", 100.0, "#FF0000"),
            shape(3, "Other", 300.0, "#00FF00"),
        ]);
        let mut frame = SlideFrame::default();
        frame.states.insert(
            TargetKey::shape(2),
            TargetState {
                opacity: 0.5,
                offset: (0.1, 0.0),
                ..TargetState::default()
            },
        );
        frame.states.insert(
            TargetKey::shape(3),
            TargetState {
                visible: false,
                ..TargetState::default()
            },
        );
        let painter = RecordingPainter::new();
        render_slide_frame(
            &painter,
            &slide,
            &RenderOptions::default(),
            &mut |_| false,
            &frame,
            &SlideText::new(),
        )
        .expect("paints");
        let ops = painter.ops();
        assert!(ops.contains(&Op::SetGlobalAlpha(0.5)));
        assert!(ops.contains(&Op::Translate(100.0, 0.0)));
        assert!(ops.contains(&Op::SetFillStyle("#FF0000".to_string())));
        assert!(
            !ops.contains(&Op::SetFillStyle("#00FF00".to_string())),
            "the hidden shape is not drawn"
        );

        // A wipe clips the shape to the shown part.
        frame.states.insert(
            TargetKey::shape(2),
            TargetState {
                reveal: Some(Reveal {
                    filter: "wipe(down)".to_string(),
                    progress: 0.5,
                }),
                ..TargetState::default()
            },
        );
        painter.clear();
        render_slide_frame(
            &painter,
            &slide,
            &RenderOptions::default(),
            &mut |_| false,
            &frame,
            &SlideText::new(),
        )
        .expect("paints");
        assert!(painter.ops().contains(&Op::LineTo(200.0, 25.0)));
        assert!(painter.count(|op| *op == Op::Clip) >= 1);

        let base = target_bounds(&slide, &SlideText::new(), &TargetKey::shape(2));
        assert_eq!((base.x, base.y, base.w), (0.15, 0.05, 0.1));
    }
}
