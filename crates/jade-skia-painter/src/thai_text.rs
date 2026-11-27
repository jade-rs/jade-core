use super::{Font, Paint, Path, Rect, SkiaPainter};
use skia_safe::{Canvas, Point, TextBlobBuilder};

pub(super) struct GlyphRun {
    glyphs: rustybuzz::GlyphBuffer,
    scale: f32,
}

pub(super) fn shape(font: &Font, text: &str) -> Option<GlyphRun> {
    if !text.starts_with(jade_painter::is_thai) {
        return None;
    }
    let data = SkiaPainter::shaping_data(&font.typeface())?;
    let face = rustybuzz::Face::from_slice(&data.0, data.1)?;
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.set_flags(rustybuzz::BufferFlags::DO_NOT_INSERT_DOTTED_CIRCLE);
    buffer.guess_segment_properties();
    let glyphs = rustybuzz::shape(&face, &[], buffer);
    if glyphs.glyph_infos().iter().any(|info| info.glyph_id == 0) {
        return None;
    }
    Some(GlyphRun {
        glyphs,
        scale: font.size() / face.units_per_em() as f32,
    })
}

impl GlyphRun {
    pub(super) fn advance(&self, font: &Font) -> f32 {
        self.glyphs
            .glyph_positions()
            .iter()
            .map(|pos| pos.x_advance as f32 * self.scale * font.scale_x())
            .sum()
    }

    fn positioned<'a>(&'a self, font: &'a Font) -> impl Iterator<Item = (u16, Point)> + 'a {
        let mut pen = 0.0;
        let x_scale = self.scale * font.scale_x();
        self.glyphs
            .glyph_infos()
            .iter()
            .zip(self.glyphs.glyph_positions())
            .map(move |(info, pos)| {
                let y = -(pos.y_offset as f32) * self.scale;
                let x = pen + pos.x_offset as f32 * x_scale + y * font.skew_x();
                pen += pos.x_advance as f32 * x_scale;
                (info.glyph_id as u16, Point::new(x, y))
            })
    }

    pub(super) fn bounds(&self, font: &Font, paint: &Paint) -> Rect {
        let mut ink = Rect::default();
        for (glyph, position) in self.positioned(font) {
            let mut bounds = [Rect::default()];
            font.get_bounds(&[glyph], &mut bounds, Some(paint));
            if !bounds[0].is_empty() {
                ink.join(bounds[0].with_offset(position));
            }
        }
        ink
    }

    pub(super) fn scalar_advances(&self, font: &Font, text: &str) -> Vec<f32> {
        let mut glyphs = self
            .glyphs
            .glyph_infos()
            .iter()
            .zip(self.glyphs.glyph_positions())
            .peekable();
        let x_scale = self.scale * font.scale_x();
        text.char_indices()
            .map(|(offset, _)| {
                let mut width = 0.0;
                while let Some((_, position)) =
                    glyphs.next_if(|(info, _)| info.cluster as usize == offset)
                {
                    width += position.x_advance as f32 * x_scale;
                }
                width
            })
            .collect()
    }

    pub(super) fn draw_outline(
        &self,
        canvas: &Canvas,
        font: &Font,
        origin: (f32, f32),
        paint: &Paint,
    ) {
        let mut path = Path::new();
        for (glyph, point) in self.positioned(font) {
            if let Some(outline) = font.get_path(glyph) {
                path.add_path(&outline, point, None);
            }
        }
        canvas.save();
        canvas.translate(origin);
        canvas.draw_path(&path, paint);
        canvas.restore();
    }

    pub(super) fn draw(&self, canvas: &Canvas, font: &Font, origin: (f32, f32), paint: &Paint) {
        let mut builder = TextBlobBuilder::new();
        let (glyphs, points) = builder.alloc_run_pos(font, self.glyphs.len(), None);
        for (i, (glyph, point)) in self.positioned(font).enumerate() {
            glyphs[i] = glyph;
            points[i] = point;
        }
        if let Some(blob) = builder.make() {
            canvas.draw_text_blob(&blob, origin, paint);
        }
    }
}
