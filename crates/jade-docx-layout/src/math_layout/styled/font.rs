//! Explicit host-bound static MATH face for the existing styled layout builder.
//! The cache belongs to the host context and is reused across plans and sizes.
use super::{MathLayoutOptions, MathMetrics, MathPathCommand, MathTextStyle, StyledMathMeasure};
use crate::math_layout::font::{
    MathFontGlyph, MathGlyphOutline, MathOutlineCommand, MathOutlineError, MathStretchAxis,
    MathStretchPlan, OpenTypeMathFont,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct PreparedMathGlyph {
    pub geometry: MathFontGlyph,
    pub scale: f64,
    pub outline: Arc<MathGlyphOutline>,
    pub italic_correction_px: f64,
}
impl PreparedMathGlyph {
    pub fn metrics(&self) -> MathMetrics {
        MathMetrics {
            width: self.geometry.advance_px,
            ascent: self.geometry.ink_px.y_max.max(0.0),
            descent: (-self.geometry.ink_px.y_min).max(0.0),
        }
    }
}

/// One glyph of a stretched character, offset from its origin.
#[derive(Debug, Clone)]
pub struct StretchedPart {
    pub glyph: PreparedMathGlyph,
    pub dx: f64,
    pub dy: f64,
}

/// A character grown along an axis: one larger variant, or an assembly of
/// parts. Ink is measured from the origin's baseline, y up.
#[derive(Debug, Clone)]
pub struct StretchedGlyph {
    pub parts: Vec<StretchedPart>,
    pub advance: f64,
    pub ascent: f64,
    pub descent: f64,
}

/// A prepared glyph's outline as plan path commands, its origin at
/// (`x`, `baseline`).
pub fn outline_commands(glyph: &PreparedMathGlyph, x: f64, baseline: f64) -> Vec<MathPathCommand> {
    let px = |v: f32| x + f64::from(v) * glyph.scale;
    let py = |v: f32| baseline - f64::from(v) * glyph.scale;
    glyph
        .outline
        .commands
        .iter()
        .map(|c| match *c {
            MathOutlineCommand::Move(a, b) => MathPathCommand::Move(px(a), py(b)),
            MathOutlineCommand::Line(a, b) => MathPathCommand::Line(px(a), py(b)),
            MathOutlineCommand::Quad(a, b, c, d) => {
                MathPathCommand::Quad(px(a), py(b), px(c), py(d))
            }
            MathOutlineCommand::Cubic(a, b, c, d, e, f) => {
                MathPathCommand::Cubic(px(a), py(b), px(c), py(d), px(e), py(f))
            }
            MathOutlineCommand::Close => MathPathCommand::Close,
        })
        .collect()
}

#[derive(Debug)]
pub enum MathFontLayoutError<E> {
    Font(crate::math_layout::font::MathFontError),
    Fallback(E),
    InvalidSize,
    InvalidScriptScale,
    NoAdequateVariant,
    AssemblyRequired,
    MissingOutline,
    Outline(MathOutlineError),
    GlyphCacheBudgetExceeded,
}
impl<E> MathFontLayoutError<E> {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Font(_) => "font-parse",
            Self::Fallback(_) => "fallback-measurement",
            Self::InvalidSize => "invalid-size",
            Self::InvalidScriptScale => "invalid-script-scale",
            Self::NoAdequateVariant => "no-adequate-variant",
            Self::AssemblyRequired => "assembly-required",
            Self::MissingOutline => "missing-outline",
            Self::Outline(_) => "outline-budget",
            Self::GlyphCacheBudgetExceeded => "glyph-cache-budget",
        }
    }
}

struct CachedGlyph {
    advance_units: u16,
    italic_correction_units: i16,
    outline: Arc<MathGlyphOutline>,
}

/// The family is the host's binding, not a guessed name read from a font.
/// Other authored families and multi-codepoint graphemes use `fallback`.
/// Construction/connector planning remains separate; shared layout places scripts.
pub struct OpenTypeMathMeasure<'a, C> {
    font: &'a OpenTypeMathFont<'a>,
    family: &'a str,
    fallback: C,
    cache: Rc<RefCell<HashMap<u16, CachedGlyph>>>,
    max_cached_glyphs: usize,
    max_outline_commands: usize,
    display_style: bool,
}
impl<'a, C> OpenTypeMathMeasure<'a, C> {
    pub fn new(
        font: &'a OpenTypeMathFont<'a>,
        family: &'a str,
        fallback: C,
        max_cached_glyphs: usize,
        max_outline_commands: usize,
    ) -> Self {
        Self {
            font,
            family,
            fallback,
            cache: Rc::new(RefCell::new(HashMap::new())),
            max_cached_glyphs,
            max_outline_commands,
            display_style: false,
        }
    }

    pub fn cached_glyphs(&self) -> usize {
        self.cache.borrow().len()
    }
    /// The host chooses display versus inline math; a paragraph container alone
    /// does not establish Word's operator-size policy.
    pub fn with_display_style(mut self, display_style: bool) -> Self {
        self.display_style = display_style;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathFontDataError {
    InvalidFamily,
    TooLarge,
    CollectionNotSupported,
    AlreadyRegistered,
    RegistryFull,
    InvalidConstants,
    Font(crate::math_layout::font::MathFontError),
}

/// Immutable owned font bytes and a bounded per-face outline cache. A parsed
/// face is borrowed for each plan, never stored as a self-referential pointer.
pub struct MathFontData {
    family: String,
    bytes: Box<[u8]>,
    face_index: u32,
    cache: Rc<RefCell<HashMap<u16, CachedGlyph>>>,
    max_cached_glyphs: usize,
    max_outline_commands: usize,
    layouts: std::cell::Cell<u64>,
}
impl MathFontData {
    pub const MAX_BYTES: usize = 16 * 1024 * 1024;

    pub fn validate(bytes: &[u8]) -> Result<(), MathFontDataError> {
        if bytes.len() > Self::MAX_BYTES {
            return Err(MathFontDataError::TooLarge);
        }
        // FontFace(buffer) and this parser must select the same standalone face.
        if bytes.starts_with(b"ttcf") {
            return Err(MathFontDataError::CollectionNotSupported);
        }
        let font = OpenTypeMathFont::parse(bytes, 0).map_err(MathFontDataError::Font)?;
        if font.constants().script_scale(1).is_none() || font.constants().script_scale(2).is_none()
        {
            return Err(MathFontDataError::InvalidConstants);
        }
        Ok(())
    }

    pub fn new(
        family: String,
        bytes: Vec<u8>,
        max_cached_glyphs: usize,
        max_outline_commands: usize,
    ) -> Result<Self, MathFontDataError> {
        let family = family.trim().to_owned();
        if family.is_empty() {
            return Err(MathFontDataError::InvalidFamily);
        }
        Self::validate(&bytes)?;
        Ok(Self {
            family,
            bytes: bytes.into_boxed_slice(),
            face_index: 0,
            cache: Rc::new(RefCell::new(HashMap::new())),
            max_cached_glyphs,
            max_outline_commands,
            layouts: std::cell::Cell::new(0),
        })
    }
    /// A face of a collection (`.ttc`) a native host read from disk, such as
    /// Cambria Math in Office's `Cambria.ttc`. A browser registers standalone
    /// faces through [`Self::new`].
    pub fn from_collection(
        family: String,
        bytes: Vec<u8>,
        face_index: u32,
        max_cached_glyphs: usize,
        max_outline_commands: usize,
    ) -> Result<Self, MathFontDataError> {
        let family = family.trim().to_owned();
        if family.is_empty() {
            return Err(MathFontDataError::InvalidFamily);
        }
        if bytes.len() > Self::MAX_BYTES * 4 {
            return Err(MathFontDataError::TooLarge);
        }
        let font = OpenTypeMathFont::parse(&bytes, face_index).map_err(MathFontDataError::Font)?;
        if font.constants().script_scale(1).is_none() || font.constants().script_scale(2).is_none()
        {
            return Err(MathFontDataError::InvalidConstants);
        }
        Ok(Self {
            family,
            bytes: bytes.into_boxed_slice(),
            face_index,
            cache: Rc::new(RefCell::new(HashMap::new())),
            max_cached_glyphs,
            max_outline_commands,
            layouts: std::cell::Cell::new(0),
        })
    }
    pub fn family(&self) -> &str {
        &self.family
    }
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }
    pub fn cached_glyphs(&self) -> usize {
        self.cache.borrow().len()
    }
    pub fn layouts(&self) -> u64 {
        self.layouts.get()
    }

    pub fn layout<'m, C: StyledMathMeasure>(
        &self,
        fallback: C,
        math: &'m jade_docx_reader::document::MathParagraph,
        style: MathTextStyle<'m>,
        display_style: bool,
    ) -> Result<super::StyledMathLayout<'m>, MathFontLayoutError<C::Error>> {
        let options = MathLayoutOptions {
            display: true,
            ..MathLayoutOptions::default()
        };
        self.layout_elements(fallback, &math.elements, style, display_style, &options)
    }

    /// Lays out with explicit options; the face's display operators follow
    /// `options.display`.
    pub fn layout_with_options<'m, C: StyledMathMeasure>(
        &self,
        fallback: C,
        elements: &'m [jade_docx_reader::document::MathElement],
        style: MathTextStyle<'m>,
        options: &MathLayoutOptions<'m>,
    ) -> Result<super::StyledMathLayout<'m>, MathFontLayoutError<C::Error>> {
        self.layout_elements(fallback, elements, style, options.display, options)
    }

    fn layout_elements<'m, C: StyledMathMeasure>(
        &self,
        fallback: C,
        elements: &'m [jade_docx_reader::document::MathElement],
        style: MathTextStyle<'m>,
        display_style: bool,
        options: &MathLayoutOptions<'m>,
    ) -> Result<super::StyledMathLayout<'m>, MathFontLayoutError<C::Error>> {
        let face = OpenTypeMathFont::parse(&self.bytes, self.face_index)
            .map_err(MathFontLayoutError::Font)?;
        let context = OpenTypeMathMeasure {
            font: &face,
            family: &self.family,
            fallback,
            cache: Rc::clone(&self.cache),
            max_cached_glyphs: self.max_cached_glyphs,
            max_outline_commands: self.max_outline_commands,
            display_style,
        };
        self.layouts.set(self.layouts.get().saturating_add(1));
        super::layout_styled_elements(&context, elements, style, options)
    }
}
impl<C: StyledMathMeasure> OpenTypeMathMeasure<'_, C> {
    /// A glyph by id, from the cache or decoded into it.
    fn prepare_id(
        &self,
        glyph: u16,
        scale: f64,
    ) -> Result<PreparedMathGlyph, MathFontLayoutError<C::Error>> {
        let mut cache = self.cache.borrow_mut();
        if !cache.contains_key(&glyph) {
            if cache.len() >= self.max_cached_glyphs {
                return Err(MathFontLayoutError::GlyphCacheBudgetExceeded);
            }
            let outline = self
                .font
                .glyph_outline(glyph, self.max_outline_commands)
                .map_err(MathFontLayoutError::Outline)?
                .unwrap_or(MathGlyphOutline {
                    commands: Vec::new(),
                    bounds: crate::math_layout::font::MathGlyphBounds {
                        x_min: 0.0,
                        y_min: 0.0,
                        x_max: 0.0,
                        y_max: 0.0,
                    },
                });
            let advance_units = self
                .font
                .glyph_advance_units(glyph)
                .ok_or(MathFontLayoutError::MissingOutline)?;
            cache.insert(
                glyph,
                CachedGlyph {
                    advance_units,
                    italic_correction_units: self.font.italic_correction(glyph),
                    outline: Arc::new(outline),
                },
            );
        }
        let cached = &cache[&glyph];
        let bounds = cached.outline.bounds;
        Ok(PreparedMathGlyph {
            geometry: MathFontGlyph {
                glyph_id: glyph,
                advance_px: f64::from(cached.advance_units) * scale,
                ink_px: crate::math_layout::font::MathGlyphBounds {
                    x_min: bounds.x_min * scale,
                    y_min: bounds.y_min * scale,
                    x_max: bounds.x_max * scale,
                    y_max: bounds.y_max * scale,
                },
            },
            scale,
            italic_correction_px: f64::from(cached.italic_correction_units) * scale,
            outline: Arc::clone(&cached.outline),
        })
    }
}

impl<C: StyledMathMeasure> StyledMathMeasure for OpenTypeMathMeasure<'_, C> {
    type Error = MathFontLayoutError<C::Error>;

    fn stretch_glyph(
        &self,
        text: &str,
        style: MathTextStyle<'_>,
        axis: MathStretchAxis,
        target_px: f64,
    ) -> Result<Option<StretchedGlyph>, Self::Error> {
        if style.family != self.family {
            return Ok(None);
        }
        let Some(scale) = self.font.scale(style.size_px) else {
            return Ok(None);
        };
        let mut chars = text.chars();
        let Some(character) = chars.next().filter(|_| chars.next().is_none()) else {
            return Ok(None);
        };
        let Some(glyph) = self.font.glyph_id(character) else {
            return Ok(None);
        };
        let target_units = (target_px / scale).ceil().max(0.0) as u32;
        let Some(choice) = self.font.stretch_choice(glyph, axis, target_units) else {
            return Ok(None);
        };
        match choice {
            crate::math_layout::font::MathStretchChoice::Variant(id) => {
                if id == glyph {
                    return Ok(None);
                }
                let g = self.prepare_id(id, scale)?;
                let ink = g.geometry.ink_px;
                Ok(Some(StretchedGlyph {
                    advance: g.geometry.advance_px,
                    ascent: ink.y_max.max(0.0),
                    descent: (-ink.y_min).max(0.0),
                    parts: vec![StretchedPart {
                        glyph: g,
                        dx: 0.0,
                        dy: 0.0,
                    }],
                }))
            }
            crate::math_layout::font::MathStretchChoice::Assembly(placed) => {
                let mut parts = Vec::new();
                let mut advance: f64 = 0.0;
                let mut ascent: f64 = 0.0;
                let mut descent: f64 = 0.0;
                for (id, offset_units) in placed {
                    let g = self.prepare_id(id, scale)?;
                    let ink = g.geometry.ink_px;
                    let offset = f64::from(offset_units) * scale;
                    let (dx, dy) = match axis {
                        // Bottom to top: the part's ink bottom at its offset.
                        MathStretchAxis::Vertical => (0.0, -(offset - ink.y_min)),
                        MathStretchAxis::Horizontal => (offset, 0.0),
                    };
                    match axis {
                        MathStretchAxis::Vertical => {
                            advance = advance.max(g.geometry.advance_px);
                            ascent = ascent.max(-dy + ink.y_max);
                            descent = descent.max(dy - ink.y_min);
                        }
                        MathStretchAxis::Horizontal => {
                            advance = advance.max(dx + g.geometry.advance_px);
                            ascent = ascent.max(ink.y_max);
                            descent = descent.max(-ink.y_min);
                        }
                    }
                    parts.push(StretchedPart { glyph: g, dx, dy });
                }
                Ok(Some(StretchedGlyph {
                    parts,
                    advance,
                    ascent,
                    descent,
                }))
            }
        }
    }

    fn top_accent_attachment(&self, text: &str, style: MathTextStyle<'_>) -> Option<f64> {
        if style.family != self.family {
            return None;
        }
        let scale = self.font.scale(style.size_px)?;
        let mut chars = text.chars();
        let character = chars.next().filter(|_| chars.next().is_none())?;
        let glyph = self.font.glyph_id(character)?;
        self.font
            .top_accent_attachment(glyph)
            .map(|v| f64::from(v) * scale)
    }

    fn measure_glyph(
        &self,
        text: &str,
        style: MathTextStyle<'_>,
    ) -> Result<MathMetrics, Self::Error> {
        self.fallback
            .measure_glyph(text, style)
            .map_err(MathFontLayoutError::Fallback)
    }

    fn script_scale(&self, style: MathTextStyle<'_>, level: u32) -> f64 {
        if style.family == self.family {
            // Invalid font constants are rejected by prepare_glyph, not replaced
            // by an unrelated fallback face's guessed script percentage.
            self.font
                .constants()
                .script_scale(level)
                .unwrap_or(f64::NAN)
        } else {
            self.fallback.script_scale(style, level)
        }
    }

    fn font_parameters(&self, style: MathTextStyle<'_>) -> Option<MathFontParameters> {
        (style.family == self.family)
            .then(|| MathFontParameters::from_font(self.font, style.size_px))
            .flatten()
    }

    fn prepare_glyph(
        &self,
        text: &str,
        style: MathTextStyle<'_>,
        display_operator: bool,
    ) -> Result<Option<PreparedMathGlyph>, Self::Error> {
        self.prepare_glyph_at_script_level(text, style, display_operator, 0)
    }

    fn prepare_glyph_at_script_level(
        &self,
        text: &str,
        style: MathTextStyle<'_>,
        display_operator: bool,
        level: u32,
    ) -> Result<Option<PreparedMathGlyph>, Self::Error> {
        if style.family != self.family {
            return Ok(None);
        }
        if self.font.constants().script_scale(1).is_none()
            || self.font.constants().script_scale(2).is_none()
        {
            return Err(MathFontLayoutError::InvalidScriptScale);
        }
        let scale = self
            .font
            .scale(style.size_px)
            .ok_or(MathFontLayoutError::InvalidSize)?;
        let mut chars = text.chars();
        let Some(character) = chars.next().filter(|_| chars.next().is_none()) else {
            return Ok(None);
        };
        let Some(mut glyph) = self.font.glyph_id(character) else {
            return Ok(None);
        };
        glyph = self.font.script_glyph_id(glyph, level);
        // Script styles never use the enclosing display-style operator height.
        if display_operator && self.display_style && level == 0 {
            // Word draws a display integral about two ems tall (Cambria
            // Math's second variant), taller than the MATH minimum asks.
            let minimum = u32::from(self.font.constants().display_operator_min_height);
            let target = if "∫∬∭∮∯∰∱∲∳⨌".contains(character) {
                minimum.max(2 * u32::from(self.font.units_per_em()))
            } else {
                minimum
            };
            glyph = match self
                .font
                .stretch_plan(glyph, MathStretchAxis::Vertical, target)
            {
                Some(MathStretchPlan::Glyph(v)) => v.glyph_id,
                Some(MathStretchPlan::Assembly(_)) => {
                    return Err(MathFontLayoutError::AssemblyRequired)
                }
                None => return Err(MathFontLayoutError::NoAdequateVariant),
            };
        }
        let mut cache = self.cache.borrow_mut();
        if !cache.contains_key(&glyph) {
            if cache.len() >= self.max_cached_glyphs {
                return Err(MathFontLayoutError::GlyphCacheBudgetExceeded);
            }
            let Some(outline) = self
                .font
                .glyph_outline(glyph, self.max_outline_commands)
                .map_err(MathFontLayoutError::Outline)?
            else {
                if display_operator {
                    return Err(MathFontLayoutError::MissingOutline);
                }
                return Ok(None);
            };
            let advance_units = self
                .font
                .glyph_advance_units(glyph)
                .ok_or(MathFontLayoutError::MissingOutline)?;
            cache.insert(
                glyph,
                CachedGlyph {
                    advance_units,
                    italic_correction_units: self.font.italic_correction(glyph),
                    outline: Arc::new(outline),
                },
            );
        }
        let cached = &cache[&glyph];
        let bounds = cached.outline.bounds;
        Ok(Some(PreparedMathGlyph {
            geometry: MathFontGlyph {
                glyph_id: glyph,
                advance_px: f64::from(cached.advance_units) * scale,
                ink_px: crate::math_layout::font::MathGlyphBounds {
                    x_min: bounds.x_min * scale,
                    y_min: bounds.y_min * scale,
                    x_max: bounds.x_max * scale,
                    y_max: bounds.y_max * scale,
                },
            },
            scale,
            italic_correction_px: f64::from(cached.italic_correction_units) * scale,
            outline: Arc::clone(&cached.outline),
        }))
    }
}

/// MATH constants scaled using the actual base face and size, without device
/// corrections. Placement follows font ink/gap constraints, not an em ratio.
#[derive(Debug, Clone, Copy)]
pub struct MathFontParameters {
    constants: crate::math_layout::font::MathFontConstants,
    scale: f64,
    vertical: MathMetrics,
}
#[derive(Debug, Clone, Copy, Default)]
pub struct MathScriptShifts {
    pub up: f64,
    pub down: f64,
}
impl MathFontParameters {
    pub fn from_font(font: &OpenTypeMathFont<'_>, size_px: f64) -> Option<Self> {
        Some(Self {
            constants: font.constants(),
            scale: font.scale(size_px)?,
            vertical: font.line_metrics(size_px)?,
        })
    }
    /// Keep ink unchanged and reserve MathLeading above it, in addition to
    /// the face's typographic ascent/descender floor.
    pub fn line_metrics(self, ink: MathMetrics) -> MathMetrics {
        MathMetrics {
            width: ink.width,
            ascent: self
                .vertical
                .ascent
                .max(ink.ascent)
                .max(ink.ascent + f64::from(self.constants.leading) * self.scale),
            descent: self.vertical.descent.max(ink.descent),
        }
    }
    pub fn constants(&self) -> crate::math_layout::font::MathFontConstants {
        self.constants
    }
    pub fn scale(&self) -> f64 {
        self.scale
    }
    pub fn space_after_script(self) -> f64 {
        f64::from(self.constants.space_after_script) * self.scale
    }
    pub fn script_shifts(
        self,
        base: MathMetrics,
        upper: Option<MathMetrics>,
        lower: Option<MathMetrics>,
    ) -> MathScriptShifts {
        let c = self.constants;
        let value = |v: i16| f64::from(v) * self.scale;
        let mut shifts = MathScriptShifts::default();
        if let Some(upper) = upper {
            shifts.up = value(c.superscript_shift_up)
                .max(value(c.superscript_bottom_min) + upper.descent)
                .max(base.ascent - value(c.superscript_baseline_drop_max));
        }
        if let Some(lower) = lower {
            shifts.down = value(c.subscript_shift_down)
                .max(lower.ascent - value(c.subscript_top_max))
                .max(base.descent + value(c.subscript_baseline_drop_min));
        }
        if let (Some(upper), Some(lower)) = (upper, lower) {
            let gap = shifts.up - upper.descent + shifts.down - lower.ascent;
            let missing = (value(c.sub_superscript_gap_min) - gap).max(0.0);
            let raise = (value(c.superscript_bottom_max_with_subscript)
                - (shifts.up - upper.descent))
                .max(0.0)
                .min(missing);
            shifts.up += raise;
            shifts.down += missing - raise;
        }
        shifts
    }
    pub fn limit_shifts(
        self,
        base: MathMetrics,
        upper: Option<MathMetrics>,
        lower: Option<MathMetrics>,
    ) -> MathScriptShifts {
        let c = self.constants;
        let value = |v: i16| f64::from(v) * self.scale;
        MathScriptShifts {
            up: upper.map_or(0.0, |m| {
                base.ascent
                    + value(c.upper_limit_baseline_rise_min)
                        .max(value(c.upper_limit_gap_min) + m.descent)
            }),
            down: lower.map_or(0.0, |m| {
                base.descent
                    + value(c.lower_limit_baseline_drop_min)
                        .max(value(c.lower_limit_gap_min) + m.ascent)
            }),
        }
    }
}
