//! Styled OMML plans: every structure of Office Math laid out the way Word
//! lays it out, from one set of placement rules for measuring, painting and
//! the caret.
//!
//! Placement follows the OpenType MATH model Word's own math engine uses. A
//! host that binds the math face (Cambria Math) supplies its constants, glyph
//! variants and assemblies; without one the same rules run on Cambria Math's
//! published constants and the host's measured glyphs, and stretchy
//! characters are drawn as paths.
//!
//! The plan also records where every element went ([`MathBox`]), which is
//! what an equation editor needs to put a caret inside a fraction.
use super::{is_property_tag, MathMetrics, MathParagraphAlignment};
use jade_docx_reader::document::{MathElement, MathElementKind, MathParagraph};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

pub mod alphabet;
#[cfg(feature = "opentype-math")]
pub mod font;
mod lines;
mod shapes;
mod structures;

pub use alphabet::{AtomClass, MathScript};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathVariant {
    Plain,
    Italic,
    Bold,
    BoldItalic,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MathTextStyle<'a> {
    pub family: &'a str,
    pub size_px: f64,
    pub color: &'a str,
    pub variant: MathVariant,
}

pub trait StyledMathMeasure {
    type Error;
    fn measure_glyph(
        &self,
        text: &str,
        style: MathTextStyle<'_>,
    ) -> Result<MathMetrics, Self::Error>;
    /// The host binds font bytes to the actual authored face. Prepared metrics
    /// and outlines must identify the same glyph; no painter substitution.
    #[cfg(feature = "opentype-math")]
    fn prepare_glyph(
        &self,
        _text: &str,
        _style: MathTextStyle<'_>,
        _display_operator: bool,
    ) -> Result<Option<font::PreparedMathGlyph>, Self::Error> {
        Ok(None)
    }
    /// Explicit math nesting for optical script glyphs. Existing host contexts
    /// preserve their preparation policy through the default implementation.
    #[cfg(feature = "opentype-math")]
    fn prepare_glyph_at_script_level(
        &self,
        text: &str,
        style: MathTextStyle<'_>,
        display_operator: bool,
        _level: u32,
    ) -> Result<Option<font::PreparedMathGlyph>, Self::Error> {
        self.prepare_glyph(text, style, display_operator)
    }
    #[cfg(feature = "opentype-math")]
    fn font_parameters(&self, _style: MathTextStyle<'_>) -> Option<font::MathFontParameters> {
        None
    }
    /// A version of `text` at least `target_px` long along `axis`: a larger
    /// variant from the face, or an assembly of its parts. `None` when the
    /// face has no construction for it (the layout then draws a path).
    #[cfg(feature = "opentype-math")]
    fn stretch_glyph(
        &self,
        _text: &str,
        _style: MathTextStyle<'_>,
        _axis: super::font::MathStretchAxis,
        _target_px: f64,
    ) -> Result<Option<font::StretchedGlyph>, Self::Error> {
        Ok(None)
    }
    /// Where an accent goes over `text`, from its left edge, when the face
    /// records it.
    #[cfg(feature = "opentype-math")]
    fn top_accent_attachment(&self, _text: &str, _style: MathTextStyle<'_>) -> Option<f64> {
        None
    }
    /// Relative to the unscaled run size. This generic fallback uses 80%/60%;
    /// a bound MATH face supplies its actual percentages.
    fn script_scale(&self, _style: MathTextStyle<'_>, level: u32) -> f64 {
        match level {
            0 => 1.0,
            1 => 0.8,
            _ => 0.6,
        }
    }
}

#[derive(Debug)]
pub struct MathGlyph<'a> {
    pub text_range: Range<usize>,
    pub x: f64,
    pub baseline: f64,
    pub width: f64,
    /// Ink above and below the glyph's baseline.
    pub ascent: f64,
    pub descent: f64,
    pub style: MathTextStyle<'a>,
    /// Whether the glyph is painted. A stretched delimiter is drawn by a
    /// shape instead, and a hidden phantom or an alignment mark not at all;
    /// their glyphs stay for the text and the caret.
    pub visible: bool,
    #[cfg(feature = "opentype-math")]
    pub font_glyph: Option<font::PreparedMathGlyph>,
}
#[derive(Debug)]
pub struct MathRule<'a> {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub thickness: f64,
    pub color: &'a str,
}

/// One step of a shape's outline, in pixels from the plan's origin with y
/// growing downwards.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MathPathCommand {
    Move(f64, f64),
    Line(f64, f64),
    Quad(f64, f64, f64, f64),
    Cubic(f64, f64, f64, f64, f64, f64),
    Close,
}

impl MathPathCommand {
    fn translate(&mut self, dx: f64, dy: f64) {
        match self {
            Self::Move(x, y) | Self::Line(x, y) => {
                *x += dx;
                *y += dy;
            }
            Self::Quad(a, b, c, d) => {
                *a += dx;
                *b += dy;
                *c += dx;
                *d += dy;
            }
            Self::Cubic(a, b, c, d, e, f) => {
                *a += dx;
                *b += dy;
                *c += dx;
                *d += dy;
                *e += dx;
                *f += dy;
            }
            Self::Close => {}
        }
    }
}

/// A drawn shape: a stretched delimiter or radical sign (the face's parts,
/// or a path when there is no face), a wide accent, a box's border.
#[derive(Debug, Clone)]
pub struct MathShape<'a> {
    pub commands: Vec<MathPathCommand>,
    /// `None` fills the outline; `Some(width)` strokes it.
    pub stroke: Option<f64>,
    pub color: &'a str,
}

/// Where an element of the source tree was laid out.
#[derive(Debug, Clone, PartialEq)]
pub struct MathBox {
    /// Child indices from [`MathParagraph::elements`] down to the element,
    /// counting property children.
    pub path: Vec<u16>,
    pub kind: MathElementKind,
    pub x: f64,
    pub baseline: f64,
    pub width: f64,
    pub ascent: f64,
    pub descent: f64,
    /// For an `m:t`, its glyphs, one per grapheme of its text.
    pub glyphs: Range<usize>,
    /// An argument with nothing in it: Word shows a placeholder square.
    pub empty: bool,
    /// The run size the element was laid out at.
    pub size_px: f64,
}

/// One line of a display equation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MathLine {
    /// The line's baseline below the first line's.
    pub baseline: f64,
    pub x: f64,
    pub width: f64,
    pub ascent: f64,
    pub descent: f64,
}

#[derive(Debug)]
pub struct StyledMathLayout<'a> {
    pub metrics: MathMetrics,
    /// Required paragraph box; ink geometry and glyph baselines stay separate.
    pub line_metrics: MathMetrics,
    pub alignment: MathParagraphAlignment,
    pub text: String,
    pub glyphs: Vec<MathGlyph<'a>>,
    pub rules: Vec<MathRule<'a>>,
    pub shapes: Vec<MathShape<'a>>,
    pub boxes: Vec<MathBox>,
    pub lines: Vec<MathLine>,
}

impl StyledMathLayout<'_> {
    /// The innermost box at a point relative to the plan's origin, preferring
    /// the deepest element whose box holds it.
    pub fn box_at(&self, x: f64, y: f64) -> Option<&MathBox> {
        self.boxes
            .iter()
            .filter(|b| {
                x >= b.x - 0.5
                    && x <= b.x + b.width + 0.5
                    && y >= b.baseline - b.ascent - 0.5
                    && y <= b.baseline + b.descent + 0.5
            })
            .max_by_key(|b| b.path.len())
    }

    /// The box laid out for the element at `path`.
    pub fn box_for(&self, path: &[u16]) -> Option<&MathBox> {
        self.boxes.iter().find(|b| b.path == path)
    }
}

/// Document-wide math settings (`m:mathPr` in the settings part).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MathSettings {
    /// `m:intLim`: integrals take their limits at the side (the default).
    pub integral_limits_side: bool,
    /// `m:naryLim`: other n-ary operators take theirs over and under.
    pub nary_limits_over: bool,
    /// `m:smallFrac`: fractions in display math stay at text size... reduced.
    pub small_fractions: bool,
    /// `m:wrapIndent`, in pixels: how far a broken line's continuation is
    /// indented. Word's default is one inch.
    pub wrap_indent_px: f64,
    /// `m:wrapRight`: continuation lines are set flush right instead.
    pub wrap_right: bool,
    /// `m:brkBin`: where a line breaks at a binary operator.
    pub break_binary: BreakBinary,
    /// `m:defJc`: the default justification of display math.
    pub default_justification: MathJustification,
}

impl Default for MathSettings {
    fn default() -> Self {
        Self {
            integral_limits_side: true,
            nary_limits_over: true,
            small_fractions: false,
            wrap_indent_px: 96.0,
            wrap_right: false,
            break_binary: BreakBinary::Before,
            default_justification: MathJustification::CenterGroup,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakBinary {
    Before,
    After,
    Repeat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathJustification {
    Left,
    Right,
    Center,
    CenterGroup,
}

impl MathJustification {
    pub fn from_omml(value: &str) -> Option<Self> {
        Some(match value {
            "left" => Self::Left,
            "right" => Self::Right,
            "center" => Self::Center,
            "centerGroup" => Self::CenterGroup,
            _ => return None,
        })
    }
}

/// How a plan is laid out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MathLayoutOptions<'a> {
    /// Display math (an equation on its own line): large operators, limits
    /// over and under, full-size fractions. Inline math shrinks them.
    pub display: bool,
    /// The width a display equation breaks its lines to fit.
    pub max_width: Option<f64>,
    /// The face `m:nor` (normal text) runs are drawn in.
    pub text_family: Option<&'a str>,
    pub settings: MathSettings,
    /// An EQ field: one size throughout, upright, in the run's own face.
    pub uniform: bool,
    /// Script and script-script sizes relative to the run, overriding the
    /// face's MATH percentages. Word sets scripts at 70% of the run whatever
    /// the face says (Cambria Math's table says 73%).
    pub script_scales: Option<(f64, f64)>,
}

impl Default for MathLayoutOptions<'_> {
    fn default() -> Self {
        Self {
            display: true,
            max_width: None,
            text_family: None,
            settings: MathSettings::default(),
            uniform: false,
            script_scales: None,
        }
    }
}

/// Word's script sizes: 70% of the run for scripts, as Word for Mac draws
/// them at 8pt and 10.5pt (`math` oracle), and Cambria Math's 60% below that.
pub const WORD_SCRIPT_SCALES: (f64, f64) = (0.70, 0.60);

impl MathLayoutOptions<'_> {
    pub fn inline() -> Self {
        Self {
            display: false,
            ..Self::default()
        }
    }

    /// An EQ field ([`super::eq_field`]): in its run's face and size,
    /// upright, one size throughout.
    pub fn eq_field() -> Self {
        Self {
            display: false,
            uniform: true,
            ..Self::default()
        }
    }
}

/// Font-unit placement constants at one size, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MathConstants {
    pub size_px: f64,
    pub axis_height: f64,
    pub accent_base_height: f64,
    pub subscript_shift_down: f64,
    pub subscript_top_max: f64,
    pub subscript_baseline_drop_min: f64,
    pub superscript_shift_up: f64,
    pub superscript_shift_up_cramped: f64,
    pub superscript_bottom_min: f64,
    pub superscript_baseline_drop_max: f64,
    pub sub_superscript_gap_min: f64,
    pub superscript_bottom_max_with_subscript: f64,
    pub space_after_script: f64,
    pub upper_limit_gap_min: f64,
    pub upper_limit_baseline_rise_min: f64,
    pub lower_limit_gap_min: f64,
    pub lower_limit_baseline_drop_min: f64,
    pub stack_top_shift_up: f64,
    pub stack_top_display_shift_up: f64,
    pub stack_bottom_shift_down: f64,
    pub stack_bottom_display_shift_down: f64,
    pub stack_gap_min: f64,
    pub stack_display_gap_min: f64,
    pub stretch_stack_gap_above_min: f64,
    pub stretch_stack_gap_below_min: f64,
    pub fraction_numerator_shift_up: f64,
    pub fraction_numerator_display_shift_up: f64,
    pub fraction_denominator_shift_down: f64,
    pub fraction_denominator_display_shift_down: f64,
    pub fraction_numerator_gap_min: f64,
    pub fraction_numerator_display_gap_min: f64,
    pub fraction_rule_thickness: f64,
    pub fraction_denominator_gap_min: f64,
    pub fraction_denominator_display_gap_min: f64,
    pub skewed_fraction_horizontal_gap: f64,
    pub skewed_fraction_vertical_gap: f64,
    pub overbar_vertical_gap: f64,
    pub overbar_rule_thickness: f64,
    pub overbar_extra_ascender: f64,
    pub underbar_vertical_gap: f64,
    pub underbar_rule_thickness: f64,
    pub underbar_extra_descender: f64,
    pub radical_vertical_gap: f64,
    pub radical_display_vertical_gap: f64,
    pub radical_rule_thickness: f64,
    pub radical_extra_ascender: f64,
    pub radical_kern_before_degree: f64,
    pub radical_kern_after_degree: f64,
    pub radical_degree_bottom_raise: f64,
    pub delimited_min_height: f64,
    pub display_operator_min_height: f64,
    pub leading: f64,
}

impl MathConstants {
    /// Cambria Math's MATH constants (2048 units per em), what Word lays
    /// math out with, for a host that has not bound the face itself.
    pub fn cambria(size_px: f64) -> Self {
        let u = |v: f64| v * size_px / 2048.0;
        Self {
            size_px,
            axis_height: u(585.0),
            accent_base_height: u(976.0),
            subscript_shift_down: u(418.0),
            subscript_top_max: u(760.0),
            subscript_baseline_drop_min: u(320.0),
            superscript_shift_up: u(750.0),
            superscript_shift_up_cramped: u(615.0),
            superscript_bottom_min: u(239.0),
            superscript_baseline_drop_max: u(460.0),
            sub_superscript_gap_min: u(300.0),
            superscript_bottom_max_with_subscript: u(765.0),
            space_after_script: u(85.0),
            upper_limit_gap_min: u(133.0),
            upper_limit_baseline_rise_min: u(500.0),
            lower_limit_gap_min: u(133.0),
            lower_limit_baseline_drop_min: u(1240.0),
            stack_top_shift_up: u(940.0),
            stack_top_display_shift_up: u(1550.0),
            stack_bottom_shift_down: u(770.0),
            stack_bottom_display_shift_down: u(1370.0),
            stack_gap_min: u(400.0),
            stack_display_gap_min: u(660.0),
            stretch_stack_gap_above_min: u(133.0),
            stretch_stack_gap_below_min: u(133.0),
            fraction_numerator_shift_up: u(1200.0),
            fraction_numerator_display_shift_up: u(1550.0),
            fraction_denominator_shift_down: u(1030.0),
            fraction_denominator_display_shift_down: u(1370.0),
            fraction_numerator_gap_min: u(133.0),
            fraction_numerator_display_gap_min: u(260.0),
            fraction_rule_thickness: u(133.0),
            fraction_denominator_gap_min: u(133.0),
            fraction_denominator_display_gap_min: u(260.0),
            skewed_fraction_horizontal_gap: u(800.0),
            skewed_fraction_vertical_gap: u(133.0),
            overbar_vertical_gap: u(345.0),
            overbar_rule_thickness: u(133.0),
            overbar_extra_ascender: u(133.0),
            underbar_vertical_gap: u(345.0),
            underbar_rule_thickness: u(133.0),
            underbar_extra_descender: u(133.0),
            radical_vertical_gap: u(166.0),
            radical_display_vertical_gap: u(345.0),
            radical_rule_thickness: u(133.0),
            radical_extra_ascender: u(133.0),
            radical_kern_before_degree: u(133.0),
            radical_kern_after_degree: u(-640.0),
            radical_degree_bottom_raise: 0.65,
            delimited_min_height: u(3000.0),
            display_operator_min_height: u(2500.0),
            leading: u(300.0),
        }
    }

    #[cfg(feature = "opentype-math")]
    pub fn from_font(c: &super::font::MathFontConstants, scale: f64, size_px: f64) -> Self {
        let u = |v: i16| f64::from(v) * scale;
        Self {
            size_px,
            axis_height: u(c.axis_height),
            accent_base_height: u(c.accent_base_height),
            subscript_shift_down: u(c.subscript_shift_down),
            subscript_top_max: u(c.subscript_top_max),
            subscript_baseline_drop_min: u(c.subscript_baseline_drop_min),
            superscript_shift_up: u(c.superscript_shift_up),
            superscript_shift_up_cramped: u(c.superscript_shift_up_cramped),
            superscript_bottom_min: u(c.superscript_bottom_min),
            superscript_baseline_drop_max: u(c.superscript_baseline_drop_max),
            sub_superscript_gap_min: u(c.sub_superscript_gap_min),
            superscript_bottom_max_with_subscript: u(c.superscript_bottom_max_with_subscript),
            space_after_script: u(c.space_after_script),
            upper_limit_gap_min: u(c.upper_limit_gap_min),
            upper_limit_baseline_rise_min: u(c.upper_limit_baseline_rise_min),
            lower_limit_gap_min: u(c.lower_limit_gap_min),
            lower_limit_baseline_drop_min: u(c.lower_limit_baseline_drop_min),
            stack_top_shift_up: u(c.stack_top_shift_up),
            stack_top_display_shift_up: u(c.stack_top_display_style_shift_up),
            stack_bottom_shift_down: u(c.stack_bottom_shift_down),
            stack_bottom_display_shift_down: u(c.stack_bottom_display_style_shift_down),
            stack_gap_min: u(c.stack_gap_min),
            stack_display_gap_min: u(c.stack_display_style_gap_min),
            stretch_stack_gap_above_min: u(c.stretch_stack_gap_above_min),
            stretch_stack_gap_below_min: u(c.stretch_stack_gap_below_min),
            fraction_numerator_shift_up: u(c.fraction_numerator_shift_up),
            fraction_numerator_display_shift_up: u(c.fraction_numerator_display_style_shift_up),
            fraction_denominator_shift_down: u(c.fraction_denominator_shift_down),
            fraction_denominator_display_shift_down: u(
                c.fraction_denominator_display_style_shift_down
            ),
            fraction_numerator_gap_min: u(c.fraction_numerator_gap_min),
            fraction_numerator_display_gap_min: u(c.fraction_num_display_style_gap_min),
            fraction_rule_thickness: u(c.fraction_rule_thickness),
            fraction_denominator_gap_min: u(c.fraction_denominator_gap_min),
            fraction_denominator_display_gap_min: u(c.fraction_denom_display_style_gap_min),
            skewed_fraction_horizontal_gap: u(c.skewed_fraction_horizontal_gap),
            skewed_fraction_vertical_gap: u(c.skewed_fraction_vertical_gap),
            overbar_vertical_gap: u(c.overbar_vertical_gap),
            overbar_rule_thickness: u(c.overbar_rule_thickness),
            overbar_extra_ascender: u(c.overbar_extra_ascender),
            underbar_vertical_gap: u(c.underbar_vertical_gap),
            underbar_rule_thickness: u(c.underbar_rule_thickness),
            underbar_extra_descender: u(c.underbar_extra_descender),
            radical_vertical_gap: u(c.radical_vertical_gap),
            radical_display_vertical_gap: u(c.radical_display_style_vertical_gap),
            radical_rule_thickness: u(c.radical_rule_thickness),
            radical_extra_ascender: u(c.radical_extra_ascender),
            radical_kern_before_degree: u(c.radical_kern_before_degree),
            radical_kern_after_degree: u(c.radical_kern_after_degree),
            radical_degree_bottom_raise: f64::from(c.radical_degree_bottom_raise_percent) / 100.0,
            delimited_min_height: f64::from(c.delimited_min_height) * scale,
            display_operator_min_height: f64::from(c.display_operator_min_height) * scale,
            leading: u(c.leading),
        }
    }
}

pub fn layout_styled_math<'a, C: StyledMathMeasure>(
    context: &C,
    math: &'a MathParagraph,
    style: MathTextStyle<'a>,
) -> Result<StyledMathLayout<'a>, C::Error> {
    layout_styled_math_with_options(context, math, style, &MathLayoutOptions::default())
}

/// Lays `math` out as `options` says: inline or display, broken to a width.
pub fn layout_styled_math_with_options<'a, C: StyledMathMeasure>(
    context: &C,
    math: &'a MathParagraph,
    style: MathTextStyle<'a>,
    options: &MathLayoutOptions<'a>,
) -> Result<StyledMathLayout<'a>, C::Error> {
    layout_styled_elements(context, &math.elements, style, options)
}

/// Lays out a list of math elements (an `m:oMath`, an `m:oMathPara`, or the
/// content of one) as one plan.
pub fn layout_styled_elements<'a, C: StyledMathMeasure>(
    context: &C,
    elements: &'a [MathElement],
    style: MathTextStyle<'a>,
    options: &MathLayoutOptions<'a>,
) -> Result<StyledMathLayout<'a>, C::Error> {
    let alignment = elements
        .iter()
        .find_map(MathParagraphAlignment::from_element)
        .unwrap_or(match options.settings.default_justification {
            MathJustification::Left => MathParagraphAlignment::Left,
            MathJustification::Right => MathParagraphAlignment::Right,
            _ => MathParagraphAlignment::Center,
        });
    let justify = elements
        .iter()
        .find_map(paragraph_justification)
        .unwrap_or(options.settings.default_justification);
    let mut builder = Builder {
        context,
        justify,
        style,
        base_size: style.size_px,
        level: 0,
        display: options.display,
        cramped: false,
        options: *options,
        prev: None,
        first: None,
        pending_italic: 0.0,
        path: Vec::new(),
        script: MathScript::Roman,
        normal_text: false,
        amps: None,
        depth: 0,
        breaks: None,
        #[cfg(feature = "opentype-math")]
        display_operator: false,
        plan: StyledMathLayout {
            metrics: MathMetrics::default(),
            line_metrics: MathMetrics::default(),
            alignment,
            text: String::new(),
            glyphs: Vec::new(),
            rules: Vec::new(),
            shapes: Vec::new(),
            boxes: Vec::new(),
            lines: Vec::new(),
        },
    };
    builder.plan.metrics = builder.root(elements)?;
    builder.plan.line_metrics = builder.line_metrics(builder.plan.metrics);
    Ok(builder.plan)
}

pub(crate) struct Builder<'c, 'a, C> {
    pub(crate) context: &'c C,
    /// `m:oMathParaPr/m:jc`, or the document's default.
    pub(crate) justify: MathJustification,
    pub(crate) style: MathTextStyle<'a>,
    /// The size scripts are scaled from: the run size at the top level.
    pub(crate) base_size: f64,
    pub(crate) level: u32,
    pub(crate) display: bool,
    pub(crate) cramped: bool,
    pub(crate) options: MathLayoutOptions<'a>,
    /// The class of the atom before the next one in the current sequence.
    pub(crate) prev: Option<AtomClass>,
    /// The class the element being laid out shows to the atom before it.
    pub(crate) first: Option<AtomClass>,
    /// The italic correction of the italic letter just placed, owed to the
    /// next glyph if it is upright (Word's `P(` and `f(`).
    pub(crate) pending_italic: f64,
    pub(crate) path: Vec<u16>,
    pub(crate) script: MathScript,
    pub(crate) normal_text: bool,
    /// Inside an equation-array row: where its `&` marks are.
    pub(crate) amps: Option<(u32, Vec<Mark>)>,
    /// Structure nesting, for the marks that belong to a row or a line.
    pub(crate) depth: u32,
    /// Where the display line being laid out may break.
    pub(crate) breaks: Option<Vec<lines::BreakCandidate>>,
    #[cfg(feature = "opentype-math")]
    pub(crate) display_operator: bool,
    pub(crate) plan: StyledMathLayout<'a>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Mark {
    pub(crate) glyphs: usize,
    pub(crate) rules: usize,
    pub(crate) shapes: usize,
    pub(crate) boxes: usize,
}

/// Where a glyph's ink and advance came from.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GlyphInfo {
    pub(crate) italic: f64,
}

impl<'a, C: StyledMathMeasure> Builder<'_, 'a, C> {
    pub(crate) fn mark(&self) -> Mark {
        Mark {
            glyphs: self.plan.glyphs.len(),
            rules: self.plan.rules.len(),
            shapes: self.plan.shapes.len(),
            boxes: self.plan.boxes.len(),
        }
    }

    pub(crate) fn shift(&mut self, start: Mark, x: f64, y: f64) {
        let end = self.mark();
        self.shift_range(start, end, x, y);
    }

    pub(crate) fn shift_range(&mut self, start: Mark, end: Mark, x: f64, y: f64) {
        if x == 0.0 && y == 0.0 {
            return;
        }
        for g in &mut self.plan.glyphs[start.glyphs..end.glyphs] {
            g.x += x;
            g.baseline += y;
        }
        for r in &mut self.plan.rules[start.rules..end.rules] {
            r.x += x;
            r.y += y;
        }
        for s in &mut self.plan.shapes[start.shapes..end.shapes] {
            for c in &mut s.commands {
                c.translate(x, y);
            }
        }
        for b in &mut self.plan.boxes[start.boxes..end.boxes] {
            b.x += x;
            b.baseline += y;
        }
    }

    /// The run size at the current script level.
    pub(crate) fn size(&self) -> f64 {
        if self.options.uniform {
            return self.style.size_px;
        }
        if let Some((script, script_script)) = self.options.script_scales {
            return self.style.size_px
                * match self.level {
                    0 => 1.0,
                    1 => script,
                    _ => script_script,
                };
        }
        self.style.size_px * self.context.script_scale(self.style, self.level)
    }

    /// The scaled style glyphs at the current level are drawn in.
    pub(crate) fn sized_style(&self) -> MathTextStyle<'a> {
        let mut style = self.style;
        style.size_px = self.size();
        style
    }

    pub(crate) fn constants(&self) -> MathConstants {
        let size = self.size();
        #[cfg(feature = "opentype-math")]
        {
            let mut style = self.style;
            style.size_px = size;
            if let Some(p) = self.context.font_parameters(style) {
                return MathConstants::from_font(&p.constants(), p.scale(), size);
            }
        }
        MathConstants::cambria(size)
    }

    fn line_metrics(&self, ink: MathMetrics) -> MathMetrics {
        #[cfg(feature = "opentype-math")]
        if let Some(font) = self.context.font_parameters(self.style) {
            return font.line_metrics(ink);
        }
        ink
    }

    /// The plan's top level: an `m:oMathPara`'s equations each on their own
    /// line, an `m:oMath` broken to the width, or plain content.
    fn root(&mut self, elements: &'a [MathElement]) -> Result<MathMetrics, C::Error> {
        let mut placed = Vec::new();
        let mut group = 0usize;
        let mut index = 0usize;
        while index < elements.len() {
            let element = &elements[index];
            if is_skipped(element) {
                index += 1;
                continue;
            }
            match element.kind {
                MathElementKind::MathParagraph => {
                    self.path.push(index as u16);
                    for (child_index, child) in element.children.iter().enumerate() {
                        if child.kind != MathElementKind::Math {
                            continue;
                        }
                        self.path.push(child_index as u16);
                        placed.extend(self.math_lines(child, group)?);
                        group += 1;
                        self.path.pop();
                    }
                    self.path.pop();
                    index += 1;
                }
                MathElementKind::Math => {
                    self.path.push(index as u16);
                    placed.extend(self.math_lines(element, group)?);
                    group += 1;
                    self.path.pop();
                    index += 1;
                }
                _ => {
                    // Bare content (a field's equation, a test's run) is one
                    // sequence on one line.
                    let start = self.mark();
                    self.prev = None;
                    let mut total = MathMetrics::default();
                    while index < elements.len()
                        && !matches!(
                            elements[index].kind,
                            MathElementKind::Math | MathElementKind::MathParagraph
                        )
                    {
                        let element = &elements[index];
                        if !is_skipped(element) {
                            self.path.push(index as u16);
                            let at = self.mark();
                            let metrics = self.element(element)?;
                            self.path.pop();
                            let x = total.width;
                            self.shift(at, x, 0.0);
                            total.include(metrics, x, 0.0);
                            total.width = total.width.max(x + metrics.width);
                        }
                        index += 1;
                    }
                    placed.push(lines::PlacedLine {
                        start,
                        end: self.mark(),
                        metrics: total,
                        group,
                        x: 0.0,
                        align: None,
                    });
                    group += 1;
                }
            }
        }
        Ok(lines::stack(self, placed))
    }

    /// Records the start of an element's box; [`Self::close_box`] fills it.
    pub(crate) fn open_box(&mut self, element: &MathElement, _start: Mark) -> usize {
        let index = self.plan.boxes.len();
        self.plan.boxes.push(MathBox {
            path: self.path.clone(),
            kind: element.kind.clone(),
            x: 0.0,
            baseline: 0.0,
            width: 0.0,
            ascent: 0.0,
            descent: 0.0,
            glyphs: self.plan.glyphs.len()..self.plan.glyphs.len(),
            empty: false,
            size_px: self.size(),
        });
        index
    }

    pub(crate) fn close_box(&mut self, index: usize, metrics: MathMetrics) {
        let glyphs_end = self.plan.glyphs.len();
        let b = &mut self.plan.boxes[index];
        b.width = metrics.width;
        b.ascent = metrics.ascent;
        b.descent = metrics.descent;
        b.glyphs.end = glyphs_end;
    }

    /// Lays out `text` grapheme by grapheme, spacing each by its class.
    pub(crate) fn text(&mut self, text: &str) -> Result<MathMetrics, C::Error> {
        let mut total = MathMetrics::default();
        let style = self.sized_style();
        let uniform = self.options.uniform;
        for grapheme in text.graphemes(true) {
            let start = self.plan.text.len();
            let first_char = grapheme.chars().next().unwrap_or(' ');
            // Alignment marks of an equation array take no room.
            if grapheme == "&" && self.amps.as_ref().is_some_and(|(d, _)| *d == self.depth) {
                self.plan.text.push('&');
                let mark = self.mark();
                self.plan.glyphs.push(MathGlyph {
                    text_range: start..self.plan.text.len(),
                    x: total.width,
                    baseline: 0.0,
                    width: 0.0,
                    ascent: 0.0,
                    descent: 0.0,
                    style,
                    visible: false,
                    #[cfg(feature = "opentype-math")]
                    font_glyph: None,
                });
                if let Some((_, amps)) = self.amps.as_mut() {
                    amps.push(mark);
                }
                continue;
            }
            let plain = self.normal_text || uniform;
            for ch in grapheme.chars() {
                let shown = if plain { ch } else { display_char(ch) };
                let shown = if plain {
                    shown
                } else {
                    alphabet::math_alphanumeric(shown, style.variant, self.script)
                };
                self.plan.text.push(shown);
            }
            let range = start..self.plan.text.len();
            // Spacing by class: none in normal text, in an EQ field, or
            // around a space typed into the run.
            let mut space = 0.0;
            if !plain && !first_char.is_whitespace() {
                let raw = if grapheme.chars().count() == 1 {
                    alphabet::char_class(first_char)
                } else {
                    AtomClass::Ord
                };
                let class = alphabet::resolve_binary(self.prev, raw);
                if let Some(prev) = self.prev {
                    space = alphabet::atom_space(prev, class, self.level > 0) * self.size() / 18.0;
                }
                if self.first.is_none() {
                    self.first = Some(class);
                }
                self.prev = Some(class);
                if matches!(class, AtomClass::Bin | AtomClass::Rel) {
                    self.break_candidate(class, false, None);
                }
            } else if plain {
                self.prev = Some(AtomClass::Ord);
                if self.first.is_none() {
                    self.first = Some(AtomClass::Ord);
                }
            }
            let (metrics, info, font_glyph) = self.measure(&range, style)?;
            #[cfg(not(feature = "opentype-math"))]
            let _ = &font_glyph;
            let italic_letter = !plain
                && matches!(style.variant, MathVariant::Italic | MathVariant::BoldItalic)
                && first_char.is_alphabetic();
            if !italic_letter {
                space += self.pending_italic;
            }
            self.pending_italic = if italic_letter {
                info.italic.max(0.0)
            } else {
                0.0
            };
            let x = total.width + space;
            self.plan.glyphs.push(MathGlyph {
                text_range: range,
                x,
                baseline: 0.0,
                width: metrics.width,
                ascent: metrics.ascent,
                descent: metrics.descent,
                style,
                visible: true,
                #[cfg(feature = "opentype-math")]
                font_glyph,
            });
            total.include(metrics, x, 0.0);
            total.width = total.width.max(x + metrics.width);
        }
        Ok(total)
    }

    /// Notes that the display line may break before the next glyph.
    pub(crate) fn break_candidate(
        &mut self,
        class: AtomClass,
        manual: bool,
        align_at: Option<u32>,
    ) {
        if self.depth != 0 {
            return;
        }
        let mark = self.mark();
        if let Some(breaks) = self.breaks.as_mut() {
            breaks.push(lines::BreakCandidate {
                mark,
                class,
                manual,
                align_at,
                align_point: false,
            });
        }
    }

    /// Notes an alignment point (`m:aln`) before the next glyph.
    pub(crate) fn align_point(&mut self) {
        if self.depth != 0 {
            return;
        }
        let mark = self.mark();
        if let Some(breaks) = self.breaks.as_mut() {
            breaks.push(lines::BreakCandidate {
                mark,
                class: AtomClass::Ord,
                manual: false,
                align_at: None,
                align_point: true,
            });
        }
    }

    /// Measures (and, with a bound face, prepares) the glyph for the text
    /// in `range` of the plan's text.
    #[allow(clippy::type_complexity)]
    pub(crate) fn measure(
        &mut self,
        range: &Range<usize>,
        style: MathTextStyle<'a>,
    ) -> Result<(MathMetrics, GlyphInfo, GlyphHandle), C::Error> {
        let text = &self.plan.text[range.clone()];
        #[cfg(feature = "opentype-math")]
        {
            let prepared = self.context.prepare_glyph_at_script_level(
                text,
                style,
                self.display_operator,
                self.level,
            )?;
            if let Some(glyph) = prepared {
                let info = GlyphInfo {
                    italic: glyph.italic_correction_px,
                };
                return Ok((glyph.metrics(), info, Some(glyph)));
            }
        }
        let metrics = self.context.measure_glyph(text, style)?;
        Ok((metrics, GlyphInfo::default(), None))
    }

    /// Lays out a sequence of siblings left to right with TeX's spacing.
    pub(crate) fn sequence(
        &mut self,
        children: &'a [MathElement],
    ) -> Result<MathMetrics, C::Error> {
        let mut total = MathMetrics::default();
        for (index, child) in children.iter().enumerate() {
            if is_skipped(child) {
                continue;
            }
            self.path.push(index as u16);
            let start = self.mark();
            let metrics = self.element(child)?;
            self.path.pop();
            let x = total.width;
            self.shift(start, x, 0.0);
            total.include(metrics, x, 0.0);
            total.width = total.width.max(x + metrics.width);
        }
        Ok(total)
    }

    /// Lays out one argument (`m:e`, `m:num`, `m:sup` …) as its own
    /// sequence at `level`, recording its box. An empty argument keeps a
    /// placeholder's room, as Word shows one.
    pub(crate) fn slot(
        &mut self,
        found: Option<(usize, &'a MathElement)>,
        level_up: u32,
        display: bool,
        cramped: bool,
    ) -> Result<MathMetrics, C::Error> {
        let Some((index, element)) = found else {
            return Ok(MathMetrics::default());
        };
        let saved = (
            self.level,
            self.display,
            self.cramped,
            self.prev.take(),
            self.first,
            std::mem::take(&mut self.pending_italic),
        );
        if !self.options.uniform {
            self.level += level_up;
        }
        self.display = display;
        self.cramped = cramped;
        self.depth += 1;
        self.path.push(index as u16);
        let start = self.mark();
        let b = self.open_box(element, start);
        self.first = None;
        let mut metrics = self.sequence(&element.children)?;
        let empty = !element
            .children
            .iter()
            .any(|c| !is_skipped(c) && has_content(c));
        if empty && !self.options.uniform {
            let size = self.size();
            metrics = MathMetrics {
                width: size * 0.6,
                ascent: size * 0.7,
                descent: 0.0,
            };
            self.plan.boxes[b].empty = true;
        }
        self.close_box(b, metrics);
        self.path.pop();
        self.depth -= 1;
        self.level = saved.0;
        self.display = saved.1;
        self.cramped = saved.2;
        self.prev = saved.3;
        self.first = saved.4;
        self.pending_italic = saved.5;
        Ok(metrics)
    }

    /// Lays out one element of a sequence at the origin.
    pub(crate) fn element(&mut self, e: &'a MathElement) -> Result<MathMetrics, C::Error> {
        use MathElementKind::*;
        let start = self.mark();
        let b = self.open_box(e, start);
        let class = structure_class(e);
        // Structures space as one atom against their neighbours.
        let mut lead = 0.0;
        let structure = !matches!(e.kind, Run | Text | Math | MathParagraph | Unknown)
            || e.local_name().starts_with("eq");
        if structure {
            if let Some(prev) = self.prev {
                let left = class.0;
                let left = alphabet::resolve_binary(Some(prev), left);
                lead = alphabet::atom_space(prev, left, self.level > 0) * self.size() / 18.0;
            }
            lead += std::mem::take(&mut self.pending_italic);
        }
        self.first = None;
        if structure && matches!(class.0, AtomClass::Bin | AtomClass::Rel) {
            self.break_candidate(class.0, false, None);
        }
        let metrics = match e.kind {
            Run => self.run(e)?,
            Text => {
                let text = e.text.as_deref().unwrap_or_default();
                self.text(text)?
            }
            Math | MathParagraph => {
                self.depth += 1;
                let m = self.sequence(&e.children)?;
                self.depth -= 1;
                m
            }
            Fraction => self.fraction(e)?,
            Superscript | Subscript | SubSup => self.scripts(e)?,
            PreSubSuper => self.prescripts(e)?,
            Nary => self.nary(e)?,
            Radical => self.radical(e)?,
            Delimiter => self.delimiter(e)?,
            Function => self.function(e)?,
            Accent => self.accent(e)?,
            Bar => self.bar(e)?,
            GroupChar => self.group_char(e)?,
            LimitLower | LimitUpper => self.limit(e)?,
            Box => self.boxed(e)?,
            BorderBox => self.border_box(e)?,
            Phantom => self.phantom(e)?,
            EquationArray => self.equation_array(e)?,
            Matrix => self.matrix(e)?,
            _ => match e.local_name() {
                "eqShift" | "eqArray" | "eqOverstrike" | "eqDisplace" | "eqList" => {
                    self.eq_structure(e)?
                }
                // Containers that are not structures (`w:ins`, `m:e` read on
                // its own) keep their content in line.
                _ => {
                    self.depth += 1;
                    let m = self.sequence(&e.children)?;
                    self.depth -= 1;
                    m
                }
            },
        };
        let mut metrics = metrics;
        if structure {
            if lead > 0.0 {
                self.shift(start, lead, 0.0);
                metrics.width += lead;
            }
            let left = alphabet::resolve_binary(self.prev, class.0);
            self.prev = Some(class.1);
            self.first = Some(left);
            self.pending_italic = 0.0;
        }
        self.close_box(b, metrics);
        if structure && lead > 0.0 {
            // The box starts where the structure's ink does.
            self.plan.boxes[b].x += lead;
            self.plan.boxes[b].width -= lead;
        }
        Ok(metrics)
    }

    /// An `m:r`: its properties set the style of its text.
    fn run(&mut self, e: &'a MathElement) -> Result<MathMetrics, C::Error> {
        let saved = (self.style, self.script, self.normal_text);
        let run = run_properties(e, self.style, self.options.text_family);
        if let Some(align_at) = run.manual_break {
            let class = first_char(e).map_or(AtomClass::Ord, alphabet::char_class);
            self.break_candidate(class, true, align_at);
        }
        if run.align_point {
            self.align_point();
        }
        self.style = run.style;
        self.script = run.script;
        self.normal_text = run.normal;
        let mut total = MathMetrics::default();
        for (index, child) in e.children.iter().enumerate() {
            if is_skipped(child) || child.kind != MathElementKind::Text {
                continue;
            }
            self.path.push(index as u16);
            let start = self.mark();
            let b = self.open_box(child, start);
            let metrics = self.text(child.text.as_deref().unwrap_or_default())?;
            self.close_box(b, metrics);
            self.path.pop();
            let x = total.width;
            self.shift(start, x, 0.0);
            total.include(metrics, x, 0.0);
            total.width = total.width.max(x + metrics.width);
        }
        self.style = saved.0;
        self.script = saved.1;
        self.normal_text = saved.2;
        Ok(total)
    }

    /// Lays out an `m:oMath`, broken into display lines.
    fn math_lines(
        &mut self,
        math: &'a MathElement,
        group: usize,
    ) -> Result<Vec<lines::PlacedLine>, C::Error> {
        lines::math_lines(self, math, group)
    }
}

/// The justification an `m:oMathPara` asks for.
fn paragraph_justification(element: &MathElement) -> Option<MathJustification> {
    if element.local_name() == "oMathParaPr" {
        return element
            .children
            .iter()
            .find(|c| c.local_name() == "jc")
            .and_then(|jc| jc.attributes.get("val"))
            .and_then(|v| MathJustification::from_omml(v));
    }
    if element.kind == MathElementKind::MathParagraph {
        return element.children.iter().find_map(paragraph_justification);
    }
    None
}

/// A glyph prepared from a bound face, when there is one.
#[cfg(feature = "opentype-math")]
pub(crate) type GlyphHandle = Option<font::PreparedMathGlyph>;
#[cfg(not(feature = "opentype-math"))]
pub(crate) type GlyphHandle = Option<()>;

/// What Word shows for a character typed into a math run: a hyphen is a
/// minus sign, an asterisk the asterisk operator, an apostrophe a prime.
fn display_char(ch: char) -> char {
    match ch {
        '-' => '−',
        '*' => '∗',
        '\'' => '′',
        _ => ch,
    }
}

/// Property and non-content children a layout passes over.
pub(crate) fn is_skipped(element: &MathElement) -> bool {
    if element.tag.as_deref().is_some_and(is_property_tag) {
        return true;
    }
    match element.local_name() {
        "mcs" | "ctrlPr" => true,
        "bookmarkStart" | "bookmarkEnd" | "proofErr" | "commentRangeStart" | "commentRangeEnd"
        | "permStart" | "permEnd" | "del" | "moveFrom" => element.namespace.is_some(),
        _ => false,
    }
}

/// Whether an element shows anything.
pub(crate) fn has_content(element: &MathElement) -> bool {
    match element.kind {
        MathElementKind::Text => element.text.as_deref().is_some_and(|t| !t.is_empty()),
        MathElementKind::Run => element
            .children
            .iter()
            .any(|c| c.kind == MathElementKind::Text && has_content(c)),
        MathElementKind::Unknown => element
            .children
            .iter()
            .any(|c| !is_skipped(c) && has_content(c)),
        _ => true,
    }
}

/// The classes a structure shows to the atom before it and after it.
fn structure_class(e: &MathElement) -> (AtomClass, AtomClass) {
    use AtomClass::*;
    use MathElementKind::*;
    match e.kind {
        Nary => (Op, Op),
        Function => (Op, Ord),
        Delimiter => (Open, Close),
        Fraction => (Inner, Inner),
        Box => {
            if on(property_element(e, "boxPr"), "opEmu") {
                let class = first_char(e).map_or(Bin, alphabet::char_class);
                (class, class)
            } else {
                (Ord, Ord)
            }
        }
        LimitLower | LimitUpper | GroupChar | Accent | Bar => {
            let class = single_char(e).map_or(Ord, alphabet::char_class);
            match class {
                Rel | Bin | Op => (class, class),
                _ => (Ord, Ord),
            }
        }
        _ => (Ord, Ord),
    }
}

fn first_char(e: &MathElement) -> Option<char> {
    if let Some(text) = &e.text {
        return text.chars().next();
    }
    e.children
        .iter()
        .filter(|c| !is_skipped(c))
        .find_map(first_char)
}

/// The one character a structure's base holds, when it holds just one.
fn single_char(e: &MathElement) -> Option<char> {
    let base = child(e, MathElementKind::Base)?.1;
    let mut text = String::new();
    collect_text(base, &mut text);
    let mut chars = text.chars();
    let first = chars.next()?;
    chars.next().is_none().then_some(first)
}

pub(crate) fn collect_text(e: &MathElement, out: &mut String) {
    if is_skipped(e) {
        return;
    }
    if let Some(text) = &e.text {
        out.push_str(text);
    }
    for c in &e.children {
        collect_text(c, out);
    }
}

/// The first child of `kind`, with its index.
pub(crate) fn child(e: &MathElement, kind: MathElementKind) -> Option<(usize, &MathElement)> {
    e.children.iter().enumerate().find(|(_, c)| c.kind == kind)
}

/// Every child of `kind`, with their indices.
pub(crate) fn children_of(
    e: &MathElement,
    kind: MathElementKind,
) -> impl Iterator<Item = (usize, &MathElement)> {
    e.children
        .iter()
        .enumerate()
        .filter(move |(_, c)| c.kind == kind)
}

pub(crate) fn property_element<'e>(e: &'e MathElement, name: &str) -> Option<&'e MathElement> {
    e.children.iter().find(|c| c.tag.as_deref() == Some(name))
}

pub(crate) fn property<'a>(element: Option<&'a MathElement>, name: &str) -> Option<&'a str> {
    element?
        .children
        .iter()
        .find(|c| c.tag.as_deref() == Some(name))?
        .attributes
        .get("val")
        .map(String::as_str)
}

/// An on/off property: present without a value, or with 1/true/on.
pub(crate) fn on(element: Option<&MathElement>, name: &str) -> bool {
    element
        .and_then(|pr| pr.children.iter().find(|c| c.tag.as_deref() == Some(name)))
        .is_some_and(|p| {
            p.attributes
                .get("val")
                .is_none_or(|v| matches!(v.as_str(), "1" | "true" | "on"))
        })
}

/// An on/off property that is on unless it says off.
pub(crate) fn on_unless_off(element: Option<&MathElement>, name: &str) -> bool {
    match element.and_then(|pr| pr.children.iter().find(|c| c.tag.as_deref() == Some(name))) {
        None => true,
        Some(p) => p
            .attributes
            .get("val")
            .is_none_or(|v| matches!(v.as_str(), "1" | "true" | "on")),
    }
}

pub(crate) struct RunStyle<'a> {
    pub(crate) style: MathTextStyle<'a>,
    pub(crate) script: MathScript,
    pub(crate) normal: bool,
    /// `m:brk`: a manual line break before the run, aligned at the n-th
    /// operator of the line above when it says so.
    pub(crate) manual_break: Option<Option<u32>>,
    /// `m:aln`: the run starts an alignment point.
    pub(crate) align_point: bool,
}

fn run_properties<'a>(
    run: &'a MathElement,
    mut style: MathTextStyle<'a>,
    text_family: Option<&'a str>,
) -> RunStyle<'a> {
    let mut script = MathScript::Roman;
    let mut normal = false;
    let mut authored_family = false;
    let mut manual_break = None;
    let mut align_point = false;
    // Math and Word run properties share the local name rPr; their children
    // identify them without dropping either source subtree.
    for pr in run
        .children
        .iter()
        .filter(|c| c.tag.as_deref() == Some("rPr"))
    {
        for p in &pr.children {
            match p.tag.as_deref() {
                Some("rFonts") => {
                    if let Some(family) = p
                        .attributes
                        .get("ascii")
                        .or_else(|| p.attributes.get("hAnsi"))
                    {
                        style.family = family;
                        authored_family = true;
                    }
                }
                Some("sz") => {
                    if let Some(size) = p
                        .attributes
                        .get("val")
                        .and_then(|s| s.parse::<f64>().ok())
                        .filter(|v| v.is_finite() && *v > 0.0)
                    {
                        style.size_px = size * 96.0 / 144.0;
                    }
                }
                Some("color") => {
                    if let Some(color) = p
                        .attributes
                        .get("val")
                        .filter(|c| c.len() == 6 && c.bytes().all(|b| b.is_ascii_hexdigit()))
                    {
                        style.color = color;
                    }
                }
                Some("sty") => {
                    style.variant = match p.attributes.get("val").map(String::as_str) {
                        Some("p") => MathVariant::Plain,
                        Some("b") => MathVariant::Bold,
                        Some("bi") => MathVariant::BoldItalic,
                        _ => MathVariant::Italic,
                    }
                }
                Some("brk") if p.namespace.is_none() => {
                    manual_break = Some(
                        p.attributes
                            .get("alnAt")
                            .and_then(|v| v.parse::<u32>().ok()),
                    );
                }
                Some("aln") if p.namespace.is_none() => {
                    align_point = !matches!(
                        p.attributes.get("val").map(String::as_str),
                        Some("0" | "false" | "off")
                    );
                }
                Some("scr") => {
                    script = MathScript::from_omml(
                        p.attributes.get("val").map(String::as_str).unwrap_or(""),
                    );
                }
                Some("nor")
                    if !matches!(
                        p.attributes.get("val").map(String::as_str),
                        Some("0" | "false" | "off")
                    ) =>
                {
                    style.variant = MathVariant::Plain;
                    normal = true;
                }
                _ => {}
            }
        }
    }
    // Normal text is set in the document's text face unless the run names
    // its own.
    if normal && !authored_family {
        if let Some(family) = text_family {
            style.family = family;
        }
    }
    RunStyle {
        style,
        script,
        normal,
        manual_break,
        align_point,
    }
}

#[cfg(test)]
mod tests;
