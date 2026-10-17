//! Explicit host-bound static MATH face for the existing styled layout builder.
//! The cache belongs to the host context and is reused across plans and sizes.
use super::{MathMetrics, MathTextStyle, StyledMathMeasure};
use crate::math_layout::font::{
    MathFontGlyph, MathGlyphOutline, MathOutlineError, MathStretchAxis, MathStretchPlan,
    OpenTypeMathFont,
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
        let face = OpenTypeMathFont::parse(&self.bytes, 0).map_err(MathFontLayoutError::Font)?;
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
        super::layout_styled_math(&context, math, style)
    }
}
impl<C: StyledMathMeasure> StyledMathMeasure for OpenTypeMathMeasure<'_, C> {
    type Error = MathFontLayoutError<C::Error>;

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
            glyph = match self.font.stretch_plan(
                glyph,
                MathStretchAxis::Vertical,
                u32::from(self.font.constants().display_operator_min_height),
            ) {
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
